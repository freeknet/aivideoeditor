// ============================================================================
// Module:       people
// Description:  Who is in which footage: a hand-editable people.toml mapping
//               clips or clip time ranges to performer names, and a draft of
//               it clustered from the vision captions.
//
// Dependencies: crate::index, crate::plan, serde, toml
// ============================================================================

//! The people map behind the "no repeated performers" rule.
//!
//! `<work>/people.toml`:
//!
//! ```toml
//! [names]
//! "MVI_7987" = "sleeveless-fans"       # the whole clip
//! "MVI_7965@26-45" = "white-mask"      # shots starting 26..45 s into it
//! ```
//!
//! A range entry beats a whole-clip entry. Footage with no entry counts as
//! one person per clip, which is right for most single-performer clips and
//! never lets a clip repeat its subject.
//!
//! `ave people --work <dir>` drafts the file from the captions: it groups
//! clips whose main performer descriptions share enough words (clothing,
//! colours, prop) and writes each group under one placeholder name, with the
//! descriptions as comments so the grouping can be checked and corrected
//! as text.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use crate::index::Index;
use crate::plan::Shot;

/// The parsed people map.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct People {
    /// Key (`clip` or `clip@from-to`) to name.
    #[serde(default)]
    pub(crate) names: BTreeMap<String, String>,
}

impl People {
    /// Loads `<work>/people.toml`, or an empty map if there is none.
    pub(crate) fn load(work: &Path) -> Result<Self> {
        let path = work.join("people.toml");
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// The name for footage of `clip` at `t` seconds.
    pub(crate) fn name_at(&self, clip: &str, t: f32) -> String {
        self.entry_at(clip, t)
            .map_or_else(|| clip.to_owned(), str::to_owned)
    }

    /// The name the map gives footage of `clip` at `t` seconds, or `None`
    /// when no entry covers it (a range entry beats a whole-clip entry).
    pub(crate) fn entry_at(&self, clip: &str, t: f32) -> Option<&str> {
        for (key, name) in &self.names {
            if let (c, Some((a, b))) = parse_key(key)
                && c == clip
                && (a..b).contains(&t)
            {
                return Some(name);
            }
        }
        self.names.get(clip).map(String::as_str)
    }

    /// Who a shot shows: its own `who` when set, else the map.
    pub(crate) fn who(&self, shot: &Shot) -> String {
        if shot.who.is_empty() {
            self.name_at(&shot.clip, shot.t_in)
        } else {
            shot.who.clone()
        }
    }
}

/// The clip and, for a range entry, the range (`from`, `to`) of a key.
///
/// A key whose range does not parse comes back with `None`, like a whole
/// clip, but under the full key text, so it never matches a real clip.
pub(crate) fn parse_key(key: &str) -> (&str, Option<(f32, f32)>) {
    let Some((clip, range)) = key.split_once('@') else {
        return (key, None);
    };
    let parsed = range.split_once('-').and_then(|(a, b)| {
        let a = a.trim().parse::<f32>().ok()?;
        let b = b.trim().parse::<f32>().ok()?;
        Some((a, b))
    });
    match parsed {
        Some(r) => (clip, Some(r)),
        None => (key, None),
    }
}

/// Seconds as a key writes them: at most two decimals, rounded down so a
/// range never reaches into the next segment, and no trailing zeros.
fn key_time(t: f32) -> String {
    let floored = (t.max(0.0) * 100.0 + 1e-3).floor() / 100.0;
    let text = format!("{floored:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The key for `from..to` seconds of `clip`: `clip@from-to`.
pub(crate) fn range_key(clip: &str, from: f32, to: f32) -> String {
    format!("{clip}@{}-{}", key_time(from), key_time(to))
}

/// A range entry of one clip: start, end, name, and the key it sits under.
type RangeEntry = (f32, f32, String, String);

impl People {
    /// Every name in the map, sorted.
    pub(crate) fn all_names(&self) -> BTreeSet<&str> {
        self.names.values().map(String::as_str).collect()
    }

    /// The keys mapped to `name`, in key order.
    pub(crate) fn keys_of(&self, name: &str) -> Vec<&str> {
        self.names
            .iter()
            .filter(|(_, n)| *n == name)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// Renames `from` to `to` everywhere; returns how many entries changed.
    pub(crate) fn rename(&mut self, from: &str, to: &str) -> usize {
        let to = to.trim();
        if to.is_empty() || from == to {
            return 0;
        }
        let mut n = 0;
        for name in self.names.values_mut() {
            if name == from {
                to.clone_into(name);
                n += 1;
            }
        }
        n
    }

    /// Merges every name in `from` into `into`; returns how many entries
    /// changed.
    pub(crate) fn merge(&mut self, from: &[String], into: &str) -> usize {
        from.iter()
            .filter(|f| f.as_str() != into)
            .map(|f| self.rename(f, into))
            .sum()
    }

    /// Removes every entry for `name`; returns how many there were.
    pub(crate) fn forget(&mut self, name: &str) -> usize {
        let before = self.names.len();
        self.names.retain(|_, n| n != name);
        before - self.names.len()
    }

    /// The range entries of `clip`, in time order.
    fn ranges_of(&self, clip: &str) -> Vec<RangeEntry> {
        let mut ranges: Vec<RangeEntry> = self
            .names
            .iter()
            .filter_map(|(key, name)| match parse_key(key) {
                (c, Some((a, b))) if c == clip => Some((a, b, name.clone(), key.clone())),
                _ => None,
            })
            .collect();
        ranges.sort_by(|x, y| x.0.total_cmp(&y.0));
        ranges
    }

    /// Cuts `from..to` out of every range entry of `clip`, keeping the parts
    /// of each range outside it under the range's old name.
    pub(crate) fn carve(&mut self, clip: &str, from: f32, to: f32) {
        for (a, b, name, key) in self.ranges_of(clip) {
            if !(a < to && from < b) {
                continue;
            }
            self.names.remove(&key);
            if a < from {
                self.names.insert(range_key(clip, a, from), name.clone());
            }
            if b > to {
                self.names.insert(range_key(clip, to, b), name);
            }
        }
    }

    /// Maps `from..to` seconds of `clip` to `name`, replacing whatever range
    /// entries covered that span before.
    pub(crate) fn assign_range(&mut self, clip: &str, from: f32, to: f32, name: &str) {
        let key = range_key(clip, from, to);
        if let (_, Some((a, b))) = parse_key(&key) {
            self.carve(clip, a, b);
        }
        self.names.insert(key, name.to_owned());
    }

    /// Re-maps some of `clip`'s segments to `name` (`None`: to nobody).
    ///
    /// `segments` are all of the clip's segments, `(start, end)` in time
    /// order; `chosen` indexes into them. Choosing every segment rewrites
    /// the clip as one whole-clip entry; otherwise each run of adjacent
    /// chosen segments becomes one `clip@from-to` range. The clip is tidied
    /// afterwards (see [`Self::tidy`]).
    pub(crate) fn assign_segments(
        &mut self,
        clip: &str,
        segments: &[(f32, f32)],
        chosen: &[usize],
        name: Option<&str>,
    ) {
        let mut chosen: Vec<usize> = chosen
            .iter()
            .copied()
            .filter(|&i| i < segments.len())
            .collect();
        chosen.sort_unstable();
        chosen.dedup();
        if chosen.is_empty() {
            return;
        }
        if chosen.len() == segments.len() {
            self.names.retain(|key, _| parse_key(key).0 != clip);
            if let Some(name) = name {
                self.names.insert(clip.to_owned(), name.to_owned());
            }
            self.tidy(clip, segments);
            return;
        }
        let mut runs: Vec<(usize, usize)> = Vec::new();
        for &i in &chosen {
            match runs.last_mut() {
                Some((_, last)) if *last + 1 == i => *last = i,
                _ => runs.push((i, i)),
            }
        }
        for (first, last) in runs {
            let (Some(a), Some(b)) = (segments.get(first), segments.get(last)) else {
                continue;
            };
            match name {
                Some(name) => self.assign_range(clip, a.0, b.1, name),
                // Under a whole-clip entry, "nobody" is the clip as its own
                // person, which is what unlisted footage counts as.
                None if self.names.contains_key(clip) => {
                    self.assign_range(clip, a.0, b.1, clip);
                }
                None => {
                    let key = range_key(clip, a.0, b.1);
                    if let (_, Some((x, y))) = parse_key(&key) {
                        self.carve(clip, x, y);
                    }
                }
            }
        }
        self.tidy(clip, segments);
    }

    /// Simplifies `clip`'s entries without changing who any segment shows:
    /// joins touching ranges of one name, drops a whole-clip entry no
    /// segment falls under any more, and collapses the clip to one
    /// whole-clip entry when every segment shows the same person.
    pub(crate) fn tidy(&mut self, clip: &str, segments: &[(f32, f32)]) {
        loop {
            let ranges = self.ranges_of(clip);
            let join = ranges.windows(2).find_map(|w| match w {
                [x, y] if (x.1 - y.0).abs() < 0.011 && x.2 == y.2 => Some((x.clone(), y.clone())),
                _ => None,
            });
            let Some((x, y)) = join else {
                break;
            };
            self.names.remove(&x.3);
            self.names.remove(&y.3);
            self.names.insert(range_key(clip, x.0, y.1), x.2);
        }
        if segments.is_empty() {
            return;
        }
        let who: Vec<Option<String>> = segments
            .iter()
            .map(|(start, _)| self.entry_at(clip, *start).map(str::to_owned))
            .collect();
        let all_same = who.windows(2).all(|w| matches!(w, [a, b] if a == b));
        if all_same && let Some(Some(name)) = who.first() {
            let name = name.clone();
            self.names.retain(|key, _| parse_key(key).0 != clip);
            if name != clip {
                self.names.insert(clip.to_owned(), name);
            }
            return;
        }
        let ranges = self.ranges_of(clip);
        if self.names.contains_key(clip) {
            let whole_used = segments
                .iter()
                .any(|(start, _)| !ranges.iter().any(|r| (r.0..r.1).contains(start)));
            if !whole_used {
                self.names.remove(clip);
            }
        } else {
            // Without a whole-clip entry, a range naming the clip itself
            // says what unlisted footage already means.
            for (_, _, name, key) in ranges {
                if name == clip {
                    self.names.remove(&key);
                }
            }
        }
    }

    /// The map as `people.toml` text: one block per person, each entry
    /// under a comment describing the clip from its captions, so the file
    /// stays reviewable by hand.
    pub(crate) fn to_toml(&self, index: Option<&Index>) -> String {
        let profiles = index.map(profiles).unwrap_or_default();
        let describe = |clip: &str| profiles.iter().find(|p| p.clip == clip);
        let start_of = |name: &str| {
            self.keys_of(name)
                .iter()
                .filter_map(|k| describe(parse_key(k).0).map(|p| p.start))
                .fold(f64::INFINITY, f64::min)
        };
        let mut names: Vec<&str> = self.all_names().into_iter().collect();
        names.sort_by(|a, b| start_of(a).total_cmp(&start_of(b)).then(a.cmp(b)));

        let mut s = String::new();
        let _ = writeln!(
            s,
            "# Who is in which footage. Rename people, split wrong groups, add `clip@from-to` ranges\n\
             # where one clip shows several performers (a range beats a whole-clip entry). Unlisted\n\
             # footage counts as one person per clip. The comments are regenerated from the captions\n\
             # whenever the app saves this file.\n\n[names]"
        );
        for name in names {
            let _ = writeln!(s);
            for key in self.keys_of(name) {
                if let Some(p) = describe(parse_key(key).0) {
                    let words = p.words.iter().cloned().collect::<Vec<_>>().join(" ");
                    let sample = p.sample.replace(['\n', '\r'], " ");
                    let _ = writeln!(s, "# {key}: {sample}  ({words})");
                }
                let _ = writeln!(
                    s,
                    "{} = {}",
                    toml::Value::String(key.to_owned()),
                    toml::Value::String(name.to_owned())
                );
            }
        }
        s
    }

    /// Writes `<work>/people.toml` through a temporary file and a rename,
    /// so a crash mid-write cannot leave half a file.
    pub(crate) fn save(&self, work: &Path, index: Option<&Index>) -> Result<()> {
        let path = work.join("people.toml");
        let tmp = work.join("people.toml.tmp");
        std::fs::write(&tmp, self.to_toml(index))
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))
    }

    /// The people `clip` most plausibly shows, best first, scored 0..1 by
    /// how its caption descriptions, prop and recording time compare with
    /// the clips already under each name (the clip itself excluded).
    pub(crate) fn guesses(&self, profiles: &[ClipPerson], clip: &str) -> Vec<(String, f32)> {
        let Some(me) = profiles.iter().find(|p| p.clip == clip) else {
            return Vec::new();
        };
        let mut best: BTreeMap<&str, f32> = BTreeMap::new();
        for (key, name) in &self.names {
            let member = parse_key(key).0;
            if member == clip || name == member {
                continue;
            }
            let Some(other) = profiles.iter().find(|p| p.clip == member) else {
                continue;
            };
            let score = likeness(me, other);
            let slot = best.entry(name).or_insert(0.0);
            *slot = slot.max(score);
        }
        let mut out: Vec<(String, f32)> =
            best.into_iter().map(|(n, s)| (n.to_owned(), s)).collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }
}

/// How alike two clips' main performers are, 0..1: shared identifying
/// words count most, then the prop, then being recorded within minutes of
/// each other.
pub(crate) fn likeness(a: &ClipPerson, b: &ClipPerson) -> f32 {
    let prop = if a.prop == "none" || b.prop == "none" {
        0.0
    } else if a.prop == b.prop {
        1.0
    } else if family(a.prop) == family(b.prop) {
        0.6
    } else {
        0.0
    };
    let gap = if a.start > b.end {
        a.start - b.end
    } else if b.start > a.end {
        b.start - a.end
    } else {
        0.0
    };
    let near = if gap <= 120.0 {
        1.0
    } else if gap <= 600.0 {
        0.5
    } else {
        0.0
    };
    0.6 * similarity(&a.words, &b.words) + 0.25 * prop + 0.15 * near
}

/// Words that say nothing about who someone is at night: everyone wears
/// "dark clothing" and has a "medium build" in firelight.
const STOP: [&str; 62] = [
    "a",
    "an",
    "the",
    "and",
    "with",
    "in",
    "on",
    "of",
    "person",
    "man",
    "woman",
    "performer",
    "wearing",
    "dark",
    "black",
    "light",
    "colored",
    "coloured",
    "clothing",
    "clothes",
    "outfit",
    "attire",
    "top",
    "is",
    "their",
    "his",
    "her",
    "hair",
    "who",
    "at",
    "to",
    "from",
    "some",
    "while",
    "holding",
    "visible",
    "figure",
    "build",
    "medium",
    "slender",
    "slim",
    "short",
    "tall",
    "athletic",
    "pants",
    "shirt",
    "none",
    "fire",
    "fan",
    "fans",
    "staff",
    "poi",
    "led",
    "long",
    "prop",
    "standing",
    "accessories",
    "silhouette",
    "no",
    "face",
    "spinner",
    "both",
];

/// The identifying words of a description.
fn words(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2 && !STOP.contains(w))
        .map(str::to_owned)
        .collect()
}

/// Jaccard similarity of two word sets.
fn similarity(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    let inter = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        0.0
    } else {
        inter as f32 / union as f32
    }
}

/// A coarse prop category from a caption's prop text.
pub(crate) fn prop_kind(prop: &str) -> &'static str {
    let p = prop.to_lowercase();
    let fire = p.contains("fire") || p.contains("torch") || p.contains("flame");
    if p.contains("silk") || p.contains("aerial") {
        "silks"
    } else if p.contains("dj") || p.contains("deck") || p.contains("turntable") {
        "dj"
    } else if p.contains("hoop") || p.contains("wheel") {
        if fire { "fire-hoop" } else { "led-hoop" }
    } else if p.contains("staff") || p.contains("stick") {
        "staff"
    } else if p.contains("fan") {
        "fans"
    } else if (p.contains("poi") || p.contains("club") || p.contains("juggl"))
        && !fire
        && p.contains("led")
    {
        "led"
    } else if p.contains("poi") || p.contains("club") || p.contains("juggl") {
        "poi"
    } else if p.contains("led") || p.contains("light") {
        "led"
    } else if fire {
        "fire"
    } else {
        "none"
    }
}

/// What the captions say about one clip's main performer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClipPerson {
    /// Clip id.
    pub(crate) clip: String,
    /// When recording started, seconds since the epoch.
    pub(crate) start: f64,
    /// When recording stopped.
    pub(crate) end: f64,
    /// Identifying words seen in at least a third of its captions.
    pub(crate) words: BTreeSet<String>,
    /// The prop category most captions give.
    pub(crate) prop: &'static str,
    /// The first description, `appearance [prop]`.
    pub(crate) sample: String,
}

/// The family of a prop category: the vision model confuses fans, staffs and
/// poi in firelight, but not fire with LED or silks.
fn family(prop: &str) -> &str {
    match prop {
        "fans" | "staff" | "poi" | "fire" | "fire-hoop" => "fire",
        other => other,
    }
}

/// Whether two clips plausibly show the same performer.
fn same_person(a: &ClipPerson, b: &ClipPerson, adjacent: bool) -> bool {
    if a.prop == "none" || b.prop == "none" {
        return false;
    }
    if adjacent {
        if family(a.prop) != family(b.prop) {
            return false;
        }
    } else if a.prop != b.prop {
        return false;
    }
    let shared = a.words.intersection(&b.words).count();
    if adjacent {
        // Shot within two minutes with the same prop: the same act unless
        // the descriptions disagree outright.
        a.words.is_empty() || b.words.is_empty() || shared > 0
    } else {
        shared >= 2 && similarity(&a.words, &b.words) >= 0.5
    }
}

/// What the captions say about each clip with a performer in it, in clip
/// order. Venue and crowd shots are left out, and so are clips nobody is in.
pub(crate) fn profiles(index: &Index) -> Vec<ClipPerson> {
    let mut people: Vec<ClipPerson> = Vec::new();
    for clip in &index.clips {
        let caps: Vec<&crate::caption::Caption> = index
            .segments
            .iter()
            .filter(|s| s.clip == clip.id)
            .filter_map(|s| s.caption.as_ref())
            .filter(|c| c.moment != "venue" && c.moment != "crowd")
            .collect();
        let mains: Vec<&crate::caption::Person> =
            caps.iter().filter_map(|c| c.people.first()).collect();
        if mains.is_empty() {
            continue;
        }
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut props: BTreeMap<&'static str, usize> = BTreeMap::new();
        for p in &mains {
            for w in words(&p.appearance) {
                *counts.entry(w).or_default() += 1;
            }
            *props.entry(prop_kind(&p.prop)).or_default() += 1;
        }
        let need = mains.len().div_ceil(3).max(1);
        let prop = props
            .iter()
            .max_by_key(|(_, n)| **n)
            .map_or("none", |(k, _)| *k);
        // The file's write time is when recording stopped.
        let end = clip.stamp.1 as f64;
        people.push(ClipPerson {
            clip: clip.id.clone(),
            start: end - clip.probe.duration,
            end,
            words: counts
                .into_iter()
                .filter(|(_, n)| *n >= need)
                .map(|(w, _)| w)
                .collect(),
            prop,
            sample: mains
                .first()
                .map(|p| format!("{} [{}]", p.appearance, p.prop))
                .unwrap_or_default(),
        });
    }
    people
}

/// Drafts `people.toml` from the captions and the clips' recording times.
pub(crate) fn draft(index: &Index) -> String {
    let mut people = profiles(index);
    people.sort_by(|a, b| a.start.total_cmp(&b.start));

    // Union-find over clips; complete-link for non-adjacent merges.
    let mut group: Vec<usize> = (0..people.len()).collect();
    let find = |g: &[usize], mut i: usize| {
        while g[i] != i {
            i = g[i];
        }
        i
    };
    for i in 1..people.len() {
        let gap = people[i].start - people[i - 1].end;
        if (0.0..=120.0).contains(&gap) && same_person(&people[i - 1], &people[i], true) {
            let (a, b) = (find(&group, i - 1), find(&group, i));
            group[b] = a;
        }
    }
    for i in 0..people.len() {
        for j in 0..i {
            let (gi, gj) = (find(&group, i), find(&group, j));
            if gi == gj {
                continue;
            }
            let all = (0..people.len())
                .filter(|&k| find(&group, k) == gi)
                .all(|k| {
                    (0..people.len())
                        .filter(|&m| find(&group, m) == gj)
                        .all(|m| same_person(&people[k], &people[m], false))
                });
            if all {
                group[gi] = gj;
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..people.len() {
        groups.entry(find(&group, i)).or_default().push(i);
    }

    let mut s = String::new();
    let _ = writeln!(
        s,
        "# Draft from the vision captions and recording times. Rename people, split wrong groups, add\n\
         # `clip@from-to` ranges where one clip shows several performers. Unlisted footage counts as one\n\
         # person per clip.\n\n[names]"
    );
    let mut ordered: Vec<&Vec<usize>> = groups.values().collect();
    ordered.sort_by(|a, b| people[a[0]].start.total_cmp(&people[b[0]].start));
    for (n, members) in ordered.iter().enumerate() {
        let name = format!("{}-{:02}", people[members[0]].prop, n + 1);
        let _ = writeln!(s);
        for &i in members.iter() {
            let p = &people[i];
            let key = p.words.iter().cloned().collect::<Vec<_>>().join(" ");
            let _ = writeln!(s, "# {}: {}  ({key})", p.clip, p.sample);
            let _ = writeln!(s, "\"{}\" = \"{name}\"", p.clip);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{ClipPerson, People, parse_key, range_key, similarity, words};

    /// A map from `(key, name)` pairs.
    fn map(entries: &[(&str, &str)]) -> People {
        People {
            names: entries
                .iter()
                .map(|(k, n)| ((*k).to_owned(), (*n).to_owned()))
                .collect(),
        }
    }

    /// Who each segment start of `clip` shows.
    fn who(people: &People, clip: &str, segments: &[(f32, f32)]) -> Vec<String> {
        segments
            .iter()
            .map(|(a, _)| people.name_at(clip, *a))
            .collect()
    }

    const SEGS: [(f32, f32); 4] = [(0.0, 5.0), (5.0, 12.5), (12.5, 20.0), (20.0, 31.25)];

    #[test]
    fn keys_parse_and_print() {
        assert_eq!(parse_key("MVI_1"), ("MVI_1", None), "whole clip");
        assert_eq!(
            parse_key("MVI_1@26-45.5"),
            ("MVI_1", Some((26.0, 45.5))),
            "range"
        );
        assert_eq!(parse_key("MVI_1@x"), ("MVI_1@x", None), "bad range");
        assert_eq!(range_key("MVI_1", 26.0, 45.5), "MVI_1@26-45.5", "trimmed");
        assert_eq!(
            range_key("MVI_1", 12.339, 20.0),
            "MVI_1@12.33-20",
            "rounded down"
        );
    }

    #[test]
    fn rename_merge_and_forget() {
        let mut p = map(&[("A", "x"), ("B", "y"), ("C", "z"), ("D", "x")]);
        assert_eq!(p.rename("x", "ava"), 2, "both entries renamed");
        assert_eq!(p.rename("ava", "  "), 0, "blank names refused");
        assert_eq!(
            p.merge(&["y".to_owned(), "z".to_owned()], "ava"),
            2,
            "two merged"
        );
        assert_eq!(p.all_names().len(), 1, "one person left: {p:?}");
        assert_eq!(p.keys_of("ava").len(), 4, "all four keys");
        assert_eq!(p.forget("ava"), 4, "forgotten");
        assert!(p.names.is_empty(), "nothing left");
    }

    #[test]
    fn carving_keeps_the_outside_of_a_range() {
        let mut p = map(&[("C@0-30", "a")]);
        p.carve("C", 10.0, 20.0);
        assert_eq!(p.name_at("C", 5.0), "a", "left part kept");
        assert_eq!(p.name_at("C", 15.0), "C", "middle cut out");
        assert_eq!(p.name_at("C", 25.0), "a", "right part kept");
        p.assign_range("C", 8.0, 22.0, "b");
        assert_eq!(p.name_at("C", 5.0), "a", "before the new range");
        assert_eq!(p.name_at("C", 9.0), "b", "inside the new range");
        assert_eq!(p.name_at("C", 23.0), "a", "after the new range");
        assert_eq!(p.names.len(), 3, "no overlapping leftovers: {p:?}");
    }

    #[test]
    fn moving_some_segments_writes_ranges_and_all_writes_the_clip() {
        let mut p = map(&[("C", "a")]);
        p.assign_segments("C", &SEGS, &[1, 2], Some("b"));
        assert_eq!(who(&p, "C", &SEGS), ["a", "b", "b", "a"], "{p:?}");
        assert!(p.names.contains_key("C@5-20"), "one joined range: {p:?}");
        p.assign_segments("C", &SEGS, &[0, 3], Some("b"));
        assert_eq!(who(&p, "C", &SEGS), ["b", "b", "b", "b"], "{p:?}");
        assert_eq!(p, map(&[("C", "b")]), "collapsed to the whole clip");
    }

    #[test]
    fn unassigning_segments_makes_them_their_own_person() {
        let mut p = map(&[("C", "a")]);
        p.assign_segments("C", &SEGS, &[3], None);
        assert_eq!(who(&p, "C", &SEGS), ["a", "a", "a", "C"], "{p:?}");
        p.assign_segments("C", &SEGS, &[0, 1, 2, 3], None);
        assert!(p.names.is_empty(), "nothing listed: {p:?}");
        let mut q = map(&[("C@5-12.5", "a")]);
        q.assign_segments("C", &SEGS, &[1], None);
        assert!(q.names.is_empty(), "range removed: {q:?}");
    }

    #[test]
    fn a_whole_clip_entry_nobody_falls_under_is_dropped() {
        let mut p = map(&[("C", "a")]);
        p.assign_segments("C", &SEGS, &[0, 1], Some("b"));
        p.assign_segments("C", &SEGS, &[2, 3], Some("c"));
        assert_eq!(who(&p, "C", &SEGS), ["b", "b", "c", "c"], "{p:?}");
        assert!(!p.names.contains_key("C"), "stale whole-clip entry: {p:?}");
    }

    #[test]
    fn the_saved_file_reads_back_the_same() -> anyhow::Result<()> {
        let p = map(&[
            ("MVI_1", "white-mask"),
            ("MVI_2@3.5-9", "ava \"the\" spinner"),
            ("MVI_3", "white-mask"),
        ]);
        let text = p.to_toml(None);
        let back: People = toml::from_str(&text)?;
        assert_eq!(back, p, "round trip:\n{text}");
        let dir = tempfile::tempdir()?;
        p.save(dir.path(), None)?;
        assert_eq!(People::load(dir.path())?, p, "saved and loaded");
        assert!(
            !dir.path().join("people.toml.tmp").exists(),
            "temporary file renamed away"
        );
        Ok(())
    }

    #[test]
    fn guesses_rank_the_closest_person_first() {
        let person = |clip: &str, start: f64, w: &str, prop: &'static str| ClipPerson {
            clip: clip.to_owned(),
            start,
            end: start + 60.0,
            words: words(w),
            prop,
            sample: String::new(),
        };
        let profiles = [
            person("A", 0.0, "green hoodie hood", "led"),
            person("B", 5000.0, "tie-dye shirt jeans", "fans"),
            person("Q", 3000.0, "green hoodie", "led"),
        ];
        let p = map(&[("A", "hoodie"), ("B", "tiedye")]);
        let g = p.guesses(&profiles, "Q");
        assert_eq!(
            g.first().map(|(n, _)| n.as_str()),
            Some("hoodie"),
            "closest first: {g:?}"
        );
        assert!(
            g.windows(2).all(|w| w[0].1 >= w[1].1),
            "sorted by score: {g:?}"
        );
        assert!(
            p.guesses(&profiles, "nobody").is_empty(),
            "an unknown clip has no guesses"
        );
    }

    #[test]
    fn ranges_beat_whole_clips() -> anyhow::Result<()> {
        let people: People =
            toml::from_str("[names]\n\"MVI_1\" = \"alice\"\n\"MVI_1@10-20\" = \"bob\"\n")?;
        assert_eq!(people.name_at("MVI_1", 5.0), "alice", "whole clip");
        assert_eq!(people.name_at("MVI_1", 12.0), "bob", "range wins");
        assert_eq!(
            people.name_at("MVI_2", 1.0),
            "MVI_2",
            "unlisted is its own person"
        );
        Ok(())
    }

    #[test]
    fn descriptions_compare_by_identifying_words() -> anyhow::Result<()> {
        let a = words("woman in a black sleeveless top, hair tied back [fire fans]");
        let b = words("person with black sleeveless top and tied hair [fire fans]");
        let c = words("man in tie-dye shirt and jeans [fire staff]");
        assert!(
            similarity(&a, &b) > similarity(&a, &c),
            "same outfit is closer"
        );
        Ok(())
    }
}

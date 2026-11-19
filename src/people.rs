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
        for (key, name) in &self.names {
            let Some((c, range)) = key.split_once('@') else {
                continue;
            };
            if c != clip {
                continue;
            }
            let Some((a, b)) = range.split_once('-') else {
                continue;
            };
            if let (Ok(a), Ok(b)) = (a.trim().parse::<f32>(), b.trim().parse::<f32>())
                && (a..b).contains(&t)
            {
                return name.clone();
            }
        }
        self.names
            .get(clip)
            .cloned()
            .unwrap_or_else(|| clip.to_owned())
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
fn prop_kind(prop: &str) -> &'static str {
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

/// What the draft knows about one clip.
struct ClipPerson {
    clip: String,
    start: f64,
    end: f64,
    words: BTreeSet<String>,
    prop: &'static str,
    sample: String,
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

/// Drafts `people.toml` from the captions and the clips' recording times.
pub(crate) fn draft(index: &Index) -> String {
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
    use super::{People, similarity, words};

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

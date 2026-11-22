// ============================================================================
// Module:       analyze::sets
// Description:  Groups a night's clips into sets (its acts): runs split by
//               gaps in recording time and by lasting changes of setting
//               in the captions, each with a short derived title.
//
// Dependencies: crate::index, crate::media, serde
// ============================================================================

//! Sets: the acts of the night.
//!
//! A flow night is a sequence of acts: a jam in the parking lot, a stage
//! set inside, a fire circle after midnight. The camera follows them, so
//! the clips, put in recording order, fall into runs. [`group`] splits
//! them where
//!
//! - recording pauses for longer than [`GAP_SECS`], or
//! - the captions' setting moves between indoors and outdoors and stays
//!   there for at least two captioned clips (one clip of the other place
//!   is a look through a door, not a new act).
//!
//! Each set gets a title from its captions and segments: the place
//! (`Indoor` / `Outdoor`), the most named location (`parking lot`,
//! `stage`, `projections`...), the props that carry it (`fire`, `LED`)
//! and the moment when it is not mostly performing. Nothing here judges
//! the footage; it only orders what the index already measured.
//!
//! Clips without a recording time go last, in a set of their own; a night
//! where no clip has one has no sets (the shot log then lists clips by
//! name, as before).

use crate::index::Index;

/// A pause in recording longer than this starts a new set. Within an act
/// the camera stops for a card swap, a battery, the next spinner stepping
/// in: a few minutes (on 2026-11-11 at most 9 minutes inside a run). Acts
/// are separated by changeovers of a quarter hour or more (26 to 57
/// minutes that night), so 15 minutes splits the one from the other.
pub(crate) const GAP_SECS: i64 = 15 * 60;

/// One set of the night.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Set {
    /// Number, from 1, in recording order.
    pub(crate) n: usize,
    /// Short derived title (`Outdoor parking lot · fire + LED`).
    pub(crate) title: String,
    /// `indoor`, `outdoor`, or empty when the captions do not say.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) place: String,
    /// Start of the first clip, RFC 3339 on the camera's clock when known
    /// (empty for the set of clips with no time).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) start: String,
    /// End of the last clip, as `start`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) end: String,
    /// Seconds of footage in the set.
    pub(crate) footage: f32,
    /// Clip ids, in recording order.
    pub(crate) clips: Vec<String>,
    /// Why the set starts here: `start of the night`, `26 min gap`,
    /// `indoor -> outdoor`, `no recording time`.
    pub(crate) why: String,
}

impl Set {
    /// `20:33-20:48`, or empty when the set has no times.
    pub(crate) fn span(&self) -> String {
        let hm = |t: &str| t.get(11..16).unwrap_or("").to_owned();
        if self.start.is_empty() {
            String::new()
        } else {
            format!("{}-{}", hm(&self.start), hm(&self.end))
        }
    }

    /// `Set 2: Indoor stage · LED · 20:33-20:48 · 7 clips, 1.6 min (26 min gap)`.
    pub(crate) fn heading(&self) -> String {
        let span = self.span();
        let span = if span.is_empty() {
            String::new()
        } else {
            format!(" · {span}")
        };
        format!(
            "Set {}: {}{span} · {} clips, {:.1} min ({})",
            self.n,
            self.title,
            self.clips.len(),
            self.footage / 60.0,
            self.why
        )
    }
}

/// Indoors or out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    /// Inside a venue.
    Indoor,
    /// Outside.
    Outdoor,
}

impl Place {
    /// `indoor` / `outdoor`.
    fn word(self) -> &'static str {
        match self {
            Self::Indoor => "indoor",
            Self::Outdoor => "outdoor",
        }
    }
}

/// Where a caption's free-text setting is, when it says. Outdoor words
/// are checked first: "outdoor stage" is outside.
fn place_of(setting: &str) -> Option<Place> {
    const OUTDOOR: [&str; 11] = [
        "outdoor",
        "outside",
        "parking",
        "alley",
        "street",
        "courtyard",
        "paved",
        "market",
        "beach",
        "garden",
        "sky",
    ];
    const INDOOR: [&str; 10] = [
        "indoor",
        "inside",
        "stage",
        "dj booth",
        "room",
        "ceiling",
        "lantern",
        "projected",
        "projection",
        "club",
    ];
    let s = setting.to_lowercase();
    if OUTDOOR.iter().any(|w| s.contains(w)) {
        Some(Place::Outdoor)
    } else if INDOOR.iter().any(|w| s.contains(w)) {
        Some(Place::Indoor)
    } else {
        None
    }
}

/// The place most of `settings` name, when one has a strict majority of
/// those that name one.
fn majority<'a>(settings: impl Iterator<Item = &'a str>) -> Option<Place> {
    let (mut indoor, mut outdoor) = (0_usize, 0_usize);
    for s in settings {
        match place_of(s) {
            Some(Place::Indoor) => indoor += 1,
            Some(Place::Outdoor) => outdoor += 1,
            None => {}
        }
    }
    match indoor.cmp(&outdoor) {
        std::cmp::Ordering::Greater => Some(Place::Indoor),
        std::cmp::Ordering::Less => Some(Place::Outdoor),
        std::cmp::Ordering::Equal => None,
    }
}

/// One clip, placed in time.
struct Item {
    /// Clip id.
    id: String,
    /// Start of recording, Unix seconds.
    start: i64,
    /// End of recording, Unix seconds.
    end: i64,
    /// The camera clock's offset, minutes, when known.
    offset: Option<i32>,
    /// Seconds of footage.
    secs: f32,
    /// Where its captions say it is.
    place: Option<Place>,
}

/// A set being built.
struct Building {
    /// Indices into the items.
    items: Vec<usize>,
    /// The place it is in so far.
    place: Option<Place>,
    /// Captioned clips seen in that place.
    known: usize,
    /// Why it starts.
    why: String,
}

impl Building {
    /// A set starting at item `k`.
    fn new(k: usize, place: Option<Place>, why: String) -> Self {
        Self {
            items: vec![k],
            place,
            known: usize::from(place.is_some()),
            why,
        }
    }
}

/// Groups the clips of `index` into sets (see the module docs). Empty
/// when no clip has a recording time.
pub(crate) fn group(index: &Index) -> Vec<Set> {
    let mut items = Vec::new();
    let mut untimed = Vec::new();
    for clip in &index.clips {
        let settings = index
            .segments
            .iter()
            .filter(|g| g.clip == clip.id)
            .filter_map(|g| g.caption.as_ref())
            .map(|c| c.setting.as_str());
        let place = majority(settings);
        let secs = clip.probe.duration as f32;
        let rec = clip.probe.recorded_at.as_ref();
        match rec.and_then(crate::media::RecordedAt::unix) {
            Some(start) => items.push(Item {
                id: clip.id.clone(),
                start,
                end: start + clip.probe.duration.max(0.0).round() as i64,
                offset: rec.and_then(crate::media::RecordedAt::offset),
                secs,
                place,
            }),
            None => untimed.push((clip.id.clone(), secs)),
        }
    }
    if items.is_empty() {
        return Vec::new();
    }
    items.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.id.cmp(&b.id)));
    // Which time run each item is in, for the setting look-ahead.
    let mut run = vec![0_usize; items.len()];
    let mut gaps = vec![None; items.len()];
    let mut latest = i64::MIN;
    for (k, item) in items.iter().enumerate() {
        if k > 0 && item.start - latest > GAP_SECS {
            gaps[k] = Some(item.start - latest);
            run[k] = run[k - 1] + 1;
        } else if k > 0 {
            run[k] = run[k - 1];
        }
        latest = latest.max(item.end);
    }
    let mut building: Vec<Building> = Vec::new();
    for (k, item) in items.iter().enumerate() {
        let next_known = || {
            items
                .iter()
                .enumerate()
                .skip(k + 1)
                .take_while(|(j, _)| run[*j] == run[k])
                .find_map(|(_, it)| it.place)
        };
        let Some(cur) = building.last_mut().filter(|_| k > 0 && gaps[k].is_none()) else {
            let why = match gaps[k] {
                Some(gap) => format!("{} min gap", (gap as f64 / 60.0).round()),
                None => "start of the night".to_owned(),
            };
            building.push(Building::new(k, item.place, why));
            continue;
        };
        match (cur.place, item.place) {
            (_, None) => cur.items.push(k),
            (None, Some(p)) => {
                cur.place = Some(p);
                cur.known = 1;
                cur.items.push(k);
            }
            (Some(c), Some(p)) if c == p => {
                cur.known += 1;
                cur.items.push(k);
            }
            (Some(c), Some(p)) => {
                let lasting = next_known() == Some(p);
                if lasting && cur.known >= 2 {
                    let why = format!("{} -> {}", c.word(), p.word());
                    building.push(Building::new(k, Some(p), why));
                } else if lasting {
                    // The set's one clip of `c` was the odd one out.
                    cur.place = Some(p);
                    cur.known = 1;
                    cur.items.push(k);
                } else {
                    cur.items.push(k);
                }
            }
        }
    }
    let mut sets: Vec<Set> = building
        .into_iter()
        .enumerate()
        .map(|(n, b)| {
            let members: Vec<&Item> = b.items.iter().filter_map(|&k| items.get(k)).collect();
            let start = members.iter().map(|i| i.start).min().unwrap_or(0);
            let end = members.iter().map(|i| i.end).max().unwrap_or(start);
            let offset = members.iter().find_map(|i| i.offset);
            let ids: Vec<String> = members.iter().map(|i| i.id.clone()).collect();
            let place = majority_items(&members);
            Set {
                n: n + 1,
                title: title(index, &ids, place),
                place: place.map(Place::word).unwrap_or_default().to_owned(),
                start: crate::media::format_time(start, offset),
                end: crate::media::format_time(end, offset),
                footage: members.iter().map(|i| i.secs).sum(),
                clips: ids,
                why: b.why,
            }
        })
        .collect();
    if !untimed.is_empty() {
        let ids: Vec<String> = untimed.iter().map(|(id, _)| id.clone()).collect();
        sets.push(Set {
            n: sets.len() + 1,
            title: title(index, &ids, None),
            place: String::new(),
            start: String::new(),
            end: String::new(),
            footage: untimed.iter().map(|(_, s)| s).sum(),
            clips: ids,
            why: "no recording time".to_owned(),
        });
    }
    sets
}

/// The place most clips of a set are in.
fn majority_items(members: &[&Item]) -> Option<Place> {
    let indoor = members
        .iter()
        .filter(|i| i.place == Some(Place::Indoor))
        .count();
    let outdoor = members
        .iter()
        .filter(|i| i.place == Some(Place::Outdoor))
        .count();
    match indoor.cmp(&outdoor) {
        std::cmp::Ordering::Greater => Some(Place::Indoor),
        std::cmp::Ordering::Less => Some(Place::Outdoor),
        std::cmp::Ordering::Equal => None,
    }
}

/// A short title for the set of `clips`: place and location, props, and
/// the moment when it is not mostly performing.
fn title(index: &Index, clips: &[String], place: Option<Place>) -> String {
    // Location words, with the name they get in a title. A brick wall
    // stands behind half the night's outdoor shots, so it names a set
    // only when nothing more particular does.
    const NOUNS: [(&str, &str); 11] = [
        ("parking", "parking lot"),
        ("alley", "alley"),
        ("street", "street"),
        ("courtyard", "courtyard"),
        ("market", "market"),
        ("stage", "stage"),
        ("dj booth", "DJ booth"),
        ("project", "projections"),
        ("lantern", "lanterns"),
        ("neon", "neon"),
        ("string lights", "string lights"),
    ];
    let segs: Vec<&crate::index::Scored> = index
        .segments
        .iter()
        .filter(|g| clips.contains(&g.clip))
        .collect();
    let settings: Vec<String> = segs
        .iter()
        .filter_map(|g| g.caption.as_ref())
        .map(|c| c.setting.to_lowercase())
        .collect();
    let count = |word: &str| settings.iter().filter(|s| s.contains(word)).count();
    let noun = NOUNS
        .iter()
        .map(|(word, name)| (count(word), *name))
        .filter(|(n, _)| *n > 0)
        .max_by_key(|(n, _)| *n)
        .map(|(_, name)| name)
        .or_else(|| (count("brick wall") > 0).then_some("brick wall"));
    let head = match (place, noun) {
        (Some(Place::Indoor), Some(n)) => format!("Indoor {n}"),
        (Some(Place::Outdoor), Some(n)) => format!("Outdoor {n}"),
        (Some(Place::Indoor), None) => "Indoor venue".to_owned(),
        (Some(Place::Outdoor), None) => "Outdoors".to_owned(),
        (None, Some(n)) => {
            let mut c = n.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect())
                .unwrap_or_default()
        }
        (None, None) if settings.is_empty() => "Uncaptioned".to_owned(),
        (None, None) => "Venue".to_owned(),
    };
    let mut parts = vec![head];
    let lit = segs.iter().filter(|g| g.subject != "none").count().max(1);
    let fire = segs.iter().filter(|g| g.subject == "fire").count();
    let led = segs.iter().filter(|g| g.subject == "led").count();
    // A prop carries the set when it carries at least a quarter of its
    // lit segments.
    let carries = |n: usize| n * 4 >= lit && n > 0;
    match (carries(fire), carries(led)) {
        (true, true) => parts.push("fire + LED".to_owned()),
        (true, false) => parts.push("fire".to_owned()),
        (false, true) => parts.push("LED".to_owned()),
        (false, false) => {}
    }
    let mut moments: Vec<(&str, usize)> = Vec::new();
    for c in segs.iter().filter_map(|g| g.caption.as_ref()) {
        let m = c.effective_moment();
        match moments.iter_mut().find(|(k, _)| *k == m) {
            Some(slot) => slot.1 += 1,
            None => moments.push((m, 1)),
        }
    }
    if let Some((m, _)) = moments.iter().max_by_key(|(_, n)| *n)
        && !m.is_empty()
        && *m != "performing"
    {
        parts.push(format!("mostly {m}"));
    }
    parts.join(" · ")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{GAP_SECS, group, place_of};
    use crate::analyze::ClipIndex;
    use crate::caption::Caption;
    use crate::index::{Index, Scored};
    use crate::media::{Probe, RecordedAt, format_time};

    /// 2026-11-12T03:00:00Z, 19:00 at -08:00.
    const T0: i64 = 1_794_452_400;

    /// A clip recorded `at` seconds after [`T0`] for `secs`, its one
    /// segment captioned with `setting` (none when empty).
    pub(crate) fn clip(
        id: &str,
        at: i64,
        secs: f64,
        setting: &str,
        subject: &str,
    ) -> (ClipIndex, Scored) {
        let probe = Probe {
            duration: secs,
            width: 1920,
            height: 1080,
            fps: [24, 1],
            vcodec: String::new(),
            has_audio: false,
            recorded_at: Some(RecordedAt {
                utc: format_time(T0 + at, None),
                local: Some(format_time(T0 + at, Some(-480))),
                source: "tag:creation_time+mtime".into(),
            }),
            camera: Some("Canon EOS".into()),
        };
        let clip = ClipIndex {
            id: id.into(),
            path: String::new(),
            stamp: (0, 0),
            probe,
            samples: Vec::new(),
            segments: Vec::new(),
            hist: Vec::new(),
            version: crate::analyze::ANALYSIS_VERSION,
        };
        let seg = Scored {
            id: format!("{id}/1"),
            clip: id.into(),
            subject: subject.into(),
            caption: (!setting.is_empty()).then(|| Caption {
                summary: "a spinner".into(),
                moment: "performing".into(),
                setting: setting.into(),
                ..Caption::default()
            }),
            ..Scored::default()
        };
        (clip, seg)
    }

    pub(crate) fn night(list: Vec<(ClipIndex, Scored)>) -> Index {
        let (clips, segments) = list.into_iter().unzip();
        Index {
            clips,
            segments,
            ..Index::default()
        }
    }

    #[test]
    fn places_read_from_free_text() -> anyhow::Result<()> {
        use super::Place::{Indoor, Outdoor};
        assert_eq!(
            place_of("outdoor parking lot at night"),
            Some(Outdoor),
            "parking"
        );
        assert_eq!(
            place_of("night, outdoor stage"),
            Some(Outdoor),
            "outdoor wins over stage"
        );
        assert_eq!(
            place_of("indoor venue with projected graphics"),
            Some(Indoor),
            "indoor"
        );
        assert_eq!(
            place_of("dark stage with LED screen"),
            Some(Indoor),
            "stage"
        );
        assert_eq!(place_of("night event"), None, "says nothing");
        Ok(())
    }

    #[test]
    fn a_long_gap_and_a_lasting_move_inside_start_sets() -> anyhow::Result<()> {
        let index = night(vec![
            // Set 1: outside, recorded out of name order.
            clip("B", 60, 10.0, "outdoor parking lot at night", "fire"),
            clip("A", 0, 20.0, "night parking lot, brick wall", "fire"),
            // One indoor clip in the middle: a look through the door.
            clip("C", 120, 10.0, "indoor venue", "led"),
            clip("D", 180, 10.0, "night event", "fire"),
            clip("E", 240, 10.0, "outdoor alley at night", "fire"),
            // Set 2: inside for two clips, a few minutes later.
            clip(
                "F",
                300,
                10.0,
                "indoor venue with projected graphics",
                "led",
            ),
            clip("G", 360, 10.0, "dark stage with projected visuals", "led"),
            // Set 3: after a long pause.
            clip("H", 360 + GAP_SECS + 60, 30.0, "", "fire"),
        ]);
        let sets = group(&index);
        let ids: Vec<Vec<&str>> = sets
            .iter()
            .map(|s| s.clips.iter().map(String::as_str).collect())
            .collect();
        assert_eq!(
            ids,
            [vec!["A", "B", "C", "D", "E"], vec!["F", "G"], vec!["H"]],
            "split by setting and by gap, clips in recording order: {sets:#?}"
        );
        let whys: Vec<&str> = sets.iter().map(|s| s.why.as_str()).collect();
        assert_eq!(
            whys,
            ["start of the night", "outdoor -> indoor", "16 min gap"],
            "{sets:#?}"
        );
        assert_eq!(
            sets[0].title, "Outdoor parking lot · fire",
            "place, location and props"
        );
        assert_eq!(sets[1].title, "Indoor projections · LED", "{sets:#?}");
        assert_eq!(sets[2].title, "Uncaptioned · fire", "{sets:#?}");
        assert_eq!(
            (sets[0].start.as_str(), sets[0].span().as_str()),
            ("2026-11-11T19:00:00-08:00", "19:00-19:04"),
            "times on the camera clock"
        );
        assert!(
            (sets[0].footage - 60.0).abs() < 1e-3,
            "footage adds up: {}",
            sets[0].footage
        );
        assert!(
            sets[1]
                .heading()
                .starts_with("Set 2: Indoor projections · LED · 19:05-19:06 · 2 clips"),
            "{}",
            sets[1].heading()
        );
        Ok(())
    }

    #[test]
    fn clips_without_times_go_last_and_no_times_means_no_sets() -> anyhow::Result<()> {
        let mut list = vec![
            clip("A", 0, 10.0, "indoor venue", "led"),
            clip("B", 30, 10.0, "indoor venue", "led"),
        ];
        list[0].0.probe.recorded_at = None;
        let sets = group(&night(list.clone()));
        assert_eq!(sets.len(), 2, "{sets:#?}");
        assert_eq!(
            (sets[1].clips.as_slice(), sets[1].why.as_str()),
            (["A".to_owned()].as_slice(), "no recording time"),
            "{sets:#?}"
        );
        list[1].0.probe.recorded_at = None;
        assert!(group(&night(list)).is_empty(), "an old index has no sets");
        Ok(())
    }

    #[test]
    fn sets_reach_the_shot_log_and_survive_a_save() -> anyhow::Result<()> {
        let index = night(vec![
            clip("Z", 0, 10.0, "indoor venue", "led"),
            clip("Y", 30, 10.0, "indoor venue", "led"),
        ]);
        let dir = tempfile::tempdir()?;
        index.save(dir.path())?;
        let back = Index::load(dir.path())?;
        assert_eq!(back.sets.len(), 1, "saved with its sets: {:?}", back.sets);
        let log = std::fs::read_to_string(dir.path().join("shotlog.md"))?;
        let set = log
            .find("## Set 1: Indoor venue · LED")
            .unwrap_or(usize::MAX);
        let z = log.find("### Z").unwrap_or(usize::MAX);
        let y = log.find("### Y").unwrap_or(usize::MAX);
        assert!(
            set < z && z < y,
            "the set heads its clips in recording order: {log}"
        );
        assert!(log.contains("19:00:00"), "clip times are printed: {log}");
        Ok(())
    }
}

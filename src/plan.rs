// ============================================================================
// Module:       plan
// Description:  The edit plan (a TOML storyboard: music window + shots on the
//               beat grid) and the rule check every plan must pass.
//
// Dependencies: crate::index, crate::beats, crate::people, serde, toml
// ============================================================================

//! Edit plans.
//!
//! A plan is the storyboard the research calls for, as a file a person or
//! an LLM edits by hand: which bars of which track, and one `[[shot]]` per
//! cut with its source clip, in-point, length in beats and crop. `ave plan`
//! drafts one; `ave check` holds any plan to the rules; `ave render` and
//! `ave otio` turn it into a video or a timeline.
//!
//! The rules are the ones the JaJa runbook sets (see [`Rules`]); each is a
//! number in the plan's `[rules]` table so another project can loosen or
//! tighten them without code changes.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

use crate::beats::MusicMap;
use crate::index::Index;
use crate::people::People;

/// Output frame shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Aspect {
    /// 1080x1920 vertical.
    #[serde(rename = "9x16")]
    Vertical,
    /// 1080x1350 feed.
    #[serde(rename = "4x5")]
    Feed,
    /// 1920x1080 landscape.
    #[serde(rename = "16x9")]
    Wide,
}

impl Aspect {
    /// Output size in pixels.
    pub(crate) fn size(self) -> (u32, u32) {
        match self {
            Self::Vertical => (1080, 1920),
            Self::Feed => (1080, 1350),
            Self::Wide => (1920, 1080),
        }
    }

    /// Width of the crop as a fraction of a 16:9 source's width.
    pub(crate) fn crop_frac(self) -> f32 {
        let (w, h) = self.size();
        ((w as f32 / h as f32) / (16.0 / 9.0)).min(1.0)
    }
}

/// Limits a plan must respect.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Rules {
    /// Two shots from one clip must start at least this many seconds apart.
    pub(crate) same_clip_gap: f32,
    /// The same performer may not appear again within this many shots.
    pub(crate) person_gap: usize,
    /// Two shots from different clips whose segment signatures (colour
    /// histogram, emitter layout, action centre; see
    /// [`crate::analyze::segments::Signature`]) are closer than this read
    /// as the same shot when they also show the same performer (or the
    /// people map does not name either). 0 turns the rule off. Calibrated
    /// on 2026-11-11: see [`LOOKALIKE`].
    pub(crate) lookalike: f32,
    /// Whether a shot reusing a moment from another night's plan fails the
    /// check (see [`Memory`]). The drafter avoids them either way.
    pub(crate) reject_reused: bool,
    /// Shots whose source range carries any of these flags fail the check.
    pub(crate) reject_flags: Vec<String>,
    /// The shot on the drop must have this subject (empty: no requirement).
    pub(crate) drop_subject: String,
    /// Caption moments that fail the check (e.g. `transition`).
    pub(crate) reject_moments: Vec<String>,
    /// Whether a caption saying the view is blocked fails the check.
    pub(crate) reject_blocked: bool,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            same_clip_gap: 30.0,
            person_gap: 8,
            lookalike: LOOKALIKE,
            reject_reused: true,
            reject_flags: vec!["dark".into(), "whip".into(), "soft".into()],
            drop_subject: "fire".into(),
            reject_moments: vec!["transition".into(), "idle".into()],
            reject_blocked: true,
        }
    }
}

/// Default look-alike threshold on segment-signature distance.
///
/// Calibrated on 2026-11-11 over the usable segments' signatures: adjacent
/// segments of one clip (same spot and performer, mostly the same action)
/// sit at a median distance of 0.153 with a third under 0.12; pairs of
/// clips showing different performers start at 0.158 (p1), only 0.1 %
/// under 0.12; other clips of the same performer, 4.1 % under 0.12.
pub(crate) const LOOKALIKE: f32 = 0.12;

/// How the render finishes the cut. Every field optional in the plan's
/// `[finish]` table.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Finish {
    /// Cut this many frames ahead of the beat (editors' anticipation: the
    /// picture changes a hair before the music does). 0 cuts on the beat.
    pub(crate) cut_lead_frames: u32,
    /// Match each shot's mid-tones (not black, not highlights, not
    /// emitters) to the drop shot's, so colour does not jump between cuts.
    pub(crate) match_colour: bool,
}

impl Default for Finish {
    fn default() -> Self {
        Self {
            cut_lead_frames: 0,
            match_colour: true,
        }
    }
}

/// Moments used by other nights' plans: the cross-week memory.
///
/// **How nights are found.** A night's work folder is `<night>/_ave`. The
/// memory reads every *other* folder beside `<night>` (its siblings under
/// the same parent, e.g. `Flow Night/2026-11-04` next to
/// `Flow Night/2026-11-11`) and takes each `<sibling>/_ave/plans/*.toml`.
/// Every shot of those plans becomes a used moment: clip id, the source
/// file's size (from `<sibling>/_ave/clips/<id>.json` when present), and
/// the time range it covered (length from `<sibling>/_ave/music.json`'s
/// tempo, or 140 BPM). A shot here reuses a moment when its clip has the
/// same id, the same file size where both are known (Canon numbers repeat
/// every 9999 clips; size tells two files apart), and its range overlaps.
/// The old pipeline's published edits are not read.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Memory {
    /// Every used moment.
    pub(crate) used: Vec<Used>,
}

/// One moment another night's plan used.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Used {
    /// The night folder's name.
    pub(crate) night: String,
    /// The plan's name.
    pub(crate) plan: String,
    /// Clip id.
    pub(crate) clip: String,
    /// Source file size in bytes, when that night's clip cache says.
    pub(crate) size: Option<u64>,
    /// Start in the clip, seconds.
    pub(crate) t0: f32,
    /// End in the clip, seconds.
    pub(crate) t1: f32,
}

/// The footage folder a night's index was built from, normalised for
/// comparison (case and slash style), or `None` when it has no index.
fn footage_of(work: &Path) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Source {
        source: String,
    }
    let text = std::fs::read_to_string(work.join("index.json")).ok()?;
    let s: Source = serde_json::from_str(&text).ok()?;
    Some(
        s.source
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase(),
    )
}

impl Memory {
    /// Reads the plans of every sibling night of the work folder `work`.
    pub(crate) fn load(work: &Path) -> Self {
        #[derive(serde::Deserialize)]
        struct Tempo {
            bpm: f32,
        }
        #[derive(serde::Deserialize)]
        struct Stamp {
            stamp: (u64, u64),
        }
        let mut used = Vec::new();
        let night = if work.file_name().is_some_and(|n| n == "_ave") {
            work.parent()
        } else {
            Some(work)
        };
        let Some(night) = night else {
            return Self { used };
        };
        let Some(root) = night.parent() else {
            return Self { used };
        };
        let Ok(entries) = std::fs::read_dir(root) else {
            return Self { used };
        };
        let me = std::fs::canonicalize(night).unwrap_or_else(|_| night.to_path_buf());
        // Copies of this same night (same footage folder) are not earlier
        // weeks: their plans are this night's own cuts, not moments to avoid.
        let my_footage = footage_of(work);
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() || std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone()) == me {
                continue;
            }
            let other = dir.join("_ave");
            if my_footage.is_some() && footage_of(&other) == my_footage {
                continue;
            }
            let Ok(plans) = std::fs::read_dir(other.join("plans")) else {
                continue;
            };
            let bpm = std::fs::read_to_string(other.join("music.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<Tempo>(&t).ok())
                .map_or(140.0, |t| t.bpm)
                .max(1.0);
            let night_name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut sizes: std::collections::BTreeMap<String, Option<u64>> =
                std::collections::BTreeMap::new();
            for p in plans.flatten().map(|e| e.path()) {
                if p.extension().is_none_or(|e| e != "toml") {
                    continue;
                }
                let Ok(plan) = Plan::load(&p) else {
                    log::warn!("memory: skipping unreadable plan {}", p.display());
                    continue;
                };
                for shot in &plan.shots {
                    let size = *sizes.entry(shot.clip.clone()).or_insert_with(|| {
                        std::fs::read_to_string(
                            other.join("clips").join(format!("{}.json", shot.clip)),
                        )
                        .ok()
                        .and_then(|t| serde_json::from_str::<Stamp>(&t).ok())
                        .map(|s| s.stamp.0)
                    });
                    used.push(Used {
                        night: night_name.clone(),
                        plan: plan.name.clone(),
                        clip: shot.clip.clone(),
                        size,
                        t0: shot.t_in,
                        t1: shot.t_in + shot.beats as f32 * 60.0 / bpm,
                    });
                }
            }
        }
        Self { used }
    }

    /// The memory for an index's night (empty when the index was not
    /// loaded from a work folder).
    pub(crate) fn for_index(index: &Index) -> Self {
        index.work.as_deref().map(Self::load).unwrap_or_default()
    }

    /// The used moment `clip` `t0..t1` repeats, if any.
    pub(crate) fn hit(&self, index: &Index, clip: &str, t0: f32, t1: f32) -> Option<&Used> {
        let size = index.clip(clip).map(|c| c.stamp.0);
        self.used.iter().find(|u| {
            u.clip == clip
                && match (u.size, size) {
                    (Some(a), Some(b)) => a == b,
                    (Some(_) | None, None) | (None, Some(_)) => true,
                }
                && u.t0 < t1
                && t0 < u.t1
        })
    }
}

/// One cut.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Shot {
    /// Source clip id.
    pub(crate) clip: String,
    /// In-point in the source, seconds.
    #[serde(rename = "in")]
    pub(crate) t_in: f32,
    /// Length in beats.
    pub(crate) beats: usize,
    /// Horizontal crop centre, 0..1 of the source width.
    #[serde(default = "half")]
    pub(crate) cx: f32,
    /// Performer / scene name (from the people map), for the repeat rule.
    #[serde(default)]
    pub(crate) who: String,
    /// The index segment the shot came from, informational.
    #[serde(default)]
    pub(crate) seg: String,
    /// Why this shot, for the human reading the plan.
    #[serde(default)]
    pub(crate) note: String,
}

/// Serde default for a centred crop.
fn half() -> f32 {
    0.5
}

/// A whole plan.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Plan {
    /// Output name, e.g. `reel-30`.
    pub(crate) name: String,
    /// Frame shape.
    pub(crate) aspect: Aspect,
    /// Music file.
    pub(crate) music: String,
    /// Bar the edit starts on (index into the music map's bars).
    pub(crate) start_bar: usize,
    /// Bar the drop lands on (the hero shot starts here).
    pub(crate) drop_bar: usize,
    /// Bar the end card starts on.
    pub(crate) card_bar: usize,
    /// Bar the edit ends on (exclusive).
    pub(crate) end_bar: usize,
    /// End card image, shown from `card_bar` to `end_bar`.
    #[serde(default)]
    pub(crate) end_card: String,
    /// Rules this plan is checked against.
    #[serde(default)]
    pub(crate) rules: Rules,
    /// Render finishing options.
    #[serde(default)]
    pub(crate) finish: Finish,
    /// The cuts, in order.
    #[serde(rename = "shot")]
    pub(crate) shots: Vec<Shot>,
    /// Review comments pinned to moments of the cut (`[[comment]]`).
    #[serde(default, rename = "comment", skip_serializing_if = "Vec::is_empty")]
    pub(crate) comments: Vec<Comment>,
}

/// A review comment: a note pinned to a moment of the cut, the way an
/// editor leaves a feedback marker. It belongs to the plan, not to a shot,
/// so it stays at its time when the shots under it change.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Comment {
    /// Seconds from the plan's start bar.
    pub(crate) time: f32,
    /// What to change, in the reviewer's words.
    #[serde(default)]
    pub(crate) note: String,
    /// Marker colour: `green` (default), `blue`, `yellow` or `red`.
    #[serde(default = "green")]
    pub(crate) color: String,
    /// Who left it: `you` or `assistant`.
    #[serde(default = "you")]
    pub(crate) author: String,
    /// Dealt with: kept for the record, not sent to chat.
    #[serde(default)]
    pub(crate) resolved: bool,
}

/// The colours a comment marker may take, default first.
pub(crate) const COMMENT_COLORS: [&str; 4] = ["green", "blue", "yellow", "red"];

/// Serde default for a comment's colour.
fn green() -> String {
    COMMENT_COLORS[0].to_owned()
}

/// Serde default for a comment's author.
fn you() -> String {
    "you".to_owned()
}

impl Plan {
    /// Reads a plan file.
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Writes a plan file.
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Start and length (seconds, relative to the edit start) of each shot.
    pub(crate) fn timing(&self, map: &MusicMap) -> Vec<(f32, f32)> {
        let origin = map.beat_time(self.start_bar, 0);
        let mut beat = 0;
        self.shots
            .iter()
            .map(|s| {
                let a = map.beat_time(self.start_bar, beat) - origin;
                beat += s.beats;
                let b = map.beat_time(self.start_bar, beat) - origin;
                (a, b - a)
            })
            .collect()
    }

    /// Beats of footage before the end card.
    pub(crate) fn footage_beats(&self) -> usize {
        (self.card_bar - self.start_bar) * 4
    }
}

/// One rule violation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Problem {
    /// Shot number (1-based), 0 for whole-plan problems.
    pub(crate) shot: usize,
    /// What is wrong.
    pub(crate) message: String,
}

/// Checks `plan` against its rules, the index and the music map.
pub(crate) fn check(plan: &Plan, index: &Index, map: &MusicMap, people: &People) -> Vec<Problem> {
    check_with(plan, index, map, people, &Memory::for_index(index))
}

/// [`check`] against an explicit cross-week memory.
pub(crate) fn check_with(
    plan: &Plan,
    index: &Index,
    map: &MusicMap,
    people: &People,
    memory: &Memory,
) -> Vec<Problem> {
    let mut problems = Vec::new();
    let mut push = |shot: usize, message: String| problems.push(Problem { shot, message });
    let r = &plan.rules;

    let beats: usize = plan.shots.iter().map(|s| s.beats).sum();
    if beats != plan.footage_beats() {
        push(
            0,
            format!(
                "shots fill {beats} beats but bars {}..{} need {}",
                plan.start_bar,
                plan.card_bar,
                plan.footage_beats()
            ),
        );
    }
    if !(plan.start_bar <= plan.drop_bar
        && plan.drop_bar < plan.card_bar
        && plan.card_bar <= plan.end_bar)
    {
        push(0, "bars must run start <= drop < card <= end".to_owned());
    }
    if plan.end_bar > map.bars.len() + 2 {
        push(
            0,
            format!(
                "end bar {} is past the track's {} bars",
                plan.end_bar,
                map.bars.len()
            ),
        );
    }

    let timing = plan.timing(map);
    let drop_beat = (plan.drop_bar - plan.start_bar) * 4;
    let mut beat = 0;
    let mut hit_drop = false;
    let crop = plan.aspect.crop_frac();
    for (k, (shot, (_, dur))) in plan.shots.iter().zip(&timing).enumerate() {
        let n = k + 1;
        if beat == drop_beat {
            hit_drop = true;
            let subject = index
                .segments
                .iter()
                .find(|g| g.clip == shot.clip && g.seg.start <= shot.t_in && shot.t_in < g.seg.end)
                .map(|g| g.subject.clone())
                .unwrap_or_default();
            if !r.drop_subject.is_empty() && subject != r.drop_subject {
                push(
                    n,
                    format!(
                        "the drop lands on a `{subject}` shot, not `{}`",
                        r.drop_subject
                    ),
                );
            }
        }
        beat += shot.beats;
        let Some(clip) = index.clip(&shot.clip) else {
            push(n, format!("clip {} is not in the index", shot.clip));
            continue;
        };
        let t_out = shot.t_in + dur;
        if shot.t_in < 0.0 || f64::from(t_out) > clip.probe.duration {
            push(
                n,
                format!(
                    "{} {:.2}-{:.2} runs outside the clip ({:.2}s)",
                    shot.clip, shot.t_in, t_out, clip.probe.duration
                ),
            );
        }
        let half_crop = crop / 2.0;
        if crop < 1.0 && (shot.cx < half_crop - 1e-3 || shot.cx > 1.0 - half_crop + 1e-3) {
            push(
                n,
                format!("crop centre {:.2} puts the crop outside the frame", shot.cx),
            );
        }
        // Flags and caption moments of every segment the shot overlaps.
        for g in index.segments.iter().filter(|g| {
            g.clip == shot.clip && g.seg.end > shot.t_in + 0.05 && g.seg.start < t_out - 0.05
        }) {
            let bad: Vec<&String> = g
                .flags
                .iter()
                .filter(|f| r.reject_flags.contains(f))
                .collect();
            if !bad.is_empty() {
                push(
                    n,
                    format!(
                        "overlaps {} which is flagged {}",
                        g.id,
                        bad.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")
                    ),
                );
            }
            if let Some(c) = &g.caption
                && r.reject_moments.contains(&c.effective_moment().to_owned())
            {
                push(
                    n,
                    format!(
                        "overlaps {} captioned as `{}`: {}",
                        g.id, c.moment, c.summary
                    ),
                );
            }
            if crop < 1.0 && g.seg.cx_spread > crop * 1.2 && g.subject != "none" {
                push(
                    n,
                    format!(
                        "the action in {} wanders {:.2} of the frame, wider than the {:.2} crop",
                        g.id, g.seg.cx_spread, crop
                    ),
                );
            }
        }
        // Repeats against every earlier shot.
        for (j, prev) in plan.shots[..k].iter().enumerate() {
            if prev.clip == shot.clip && (prev.t_in - shot.t_in).abs() < r.same_clip_gap {
                push(
                    n,
                    format!(
                        "same stretch of {} as shot {} ({:.1}s apart, need {:.0})",
                        shot.clip,
                        j + 1,
                        (prev.t_in - shot.t_in).abs(),
                        r.same_clip_gap
                    ),
                );
            }
            let (a, b) = (people.who(shot), people.who(prev));
            if !a.is_empty() && a == b && k - j <= r.person_gap {
                push(
                    n,
                    format!(
                        "`{a}` again, {} shots after shot {} (need {})",
                        k - j,
                        j + 1,
                        r.person_gap + 1
                    ),
                );
            }
            if r.lookalike > 0.0
                && prev.clip != shot.clip
                && let Some(d) = lookalike(index, people, (shot, *dur), (prev, timing[j].1))
                && d < r.lookalike
            {
                push(
                    n,
                    format!(
                        "looks like shot {} (signature distance {d:.3} < {:.3}, same spot, performer and action)",
                        j + 1,
                        r.lookalike
                    ),
                );
            }
        }
        if let Some(u) = memory.hit(index, &shot.clip, shot.t_in, t_out)
            && r.reject_reused
        {
            push(
                n,
                format!(
                    "reuses {} {:.1}-{:.1} from {} `{}`",
                    u.clip, u.t0, u.t1, u.night, u.plan
                ),
            );
        }
    }
    if !hit_drop {
        push(
            0,
            format!("no shot starts exactly on the drop (bar {})", plan.drop_bar),
        );
    }
    problems
}

/// Whether a people-map name says who someone is (unlisted footage is
/// named after its clip, which says nothing).
fn known(name: &str, clip: &str) -> bool {
    !name.is_empty() && name != clip
}

/// Signature distance between two shots (each with its length in
/// seconds), or `None` when they cannot be look-alikes: a signature is
/// missing, or the people map names two different performers.
pub(crate) fn lookalike(
    index: &Index,
    people: &People,
    a: (&Shot, f32),
    b: (&Shot, f32),
) -> Option<f32> {
    let (wa, wb) = (people.who(a.0), people.who(b.0));
    if known(&wa, &a.0.clip) && known(&wb, &b.0.clip) && wa != wb {
        return None;
    }
    let sa = &index
        .segment_at(&a.0.clip, a.0.t_in, a.0.t_in + a.1)?
        .seg
        .sig;
    let sb = &index
        .segment_at(&b.0.clip, b.0.t_in, b.0.t_in + b.1)?
        .seg
        .sig;
    if sa.is_empty() || sb.is_empty() {
        return None;
    }
    Some(sa.distance(sb))
}

/// Renders problems as text.
pub(crate) fn report(plan: &Plan, problems: &[Problem]) -> String {
    let mut s = String::new();
    if problems.is_empty() {
        let _ = writeln!(s, "{}: ok ({} shots)", plan.name, plan.shots.len());
    }
    for p in problems {
        if p.shot == 0 {
            let _ = writeln!(s, "{}: {}", plan.name, p.message);
        } else {
            let _ = writeln!(s, "{} shot {}: {}", plan.name, p.shot, p.message);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{Aspect, Plan, Rules};

    #[test]
    fn crop_fractions() -> anyhow::Result<()> {
        assert!(
            (Aspect::Vertical.crop_frac() - 0.316).abs() < 0.01,
            "9:16 from 16:9"
        );
        assert!(
            (Aspect::Wide.crop_frac() - 1.0).abs() < 1e-6,
            "16:9 is the whole frame"
        );
        Ok(())
    }

    #[test]
    fn a_plan_round_trips_through_toml() -> anyhow::Result<()> {
        let text = r#"
name = "reel-15"
aspect = "9x16"
music = "track.wav"
start_bar = 2
drop_bar = 4
card_bar = 6
end_bar = 7

[[shot]]
clip = "MVI_1"
in = 3.5
beats = 4
"#;
        let plan: Plan = toml::from_str(text)?;
        assert_eq!(plan.shots[0].cx, 0.5, "cx defaults to centre");
        assert_eq!(plan.rules, Rules::default(), "rules default");
        let again: Plan = toml::from_str(&toml::to_string_pretty(&plan)?)?;
        assert_eq!(again, plan, "round trip");
        Ok(())
    }

    /// A two-clip index whose clips are long, bright and well focused.
    fn tiny_index() -> crate::index::Index {
        use crate::analyze::ClipIndex;
        use crate::analyze::segments::{Camera, Segment};
        use crate::index::{Index, Scored};
        use crate::media::Probe;
        let seg = |start: f32, end: f32| Segment {
            start,
            end,
            camera: Camera::Static,
            after_cut: false,
            luma: 0.3,
            p95: 0.8,
            clipped: 0.0,
            sharp: 1.0,
            flame: 0.05,
            led: 0.0,
            white: 0.0,
            moving: 0.02,
            cx: 0.5,
            cx_spread: 0.05,
            cy: 0.5,
            zoom_factor: 1.0,
            pan: 0.0,
            jitter: 0.0,
            lit: 0.4,
            lit_frac: 0.1,
            lit_p95: 0.8,
            burst: 0.0,
            peak_t: 0.0,
            moments: Vec::new(),
            sig: Default::default(),
        };
        let probe = Probe {
            duration: 120.0,
            width: 1920,
            height: 1080,
            fps: [24000, 1001],
            vcodec: "h264".into(),
            has_audio: false,
        };
        let clip = |id: &str, hist: f32| ClipIndex {
            id: id.into(),
            path: format!("{id}.mov"),
            stamp: (0, 0),
            probe: probe.clone(),
            samples: Vec::new(),
            segments: vec![seg(0.0, 120.0)],
            hist: vec![hist; 32],
            version: 0,
        };
        let scored = |c: &str| Scored {
            id: format!("{c}/1"),
            clip: c.into(),
            seg: seg(0.0, 120.0),
            subject: "fire".into(),
            q: 0.9,
            flags: Vec::new(),
            why: Vec::new(),
            caption: None,
        };
        Index {
            source: String::new(),
            sharp_ref: 1.0,
            clips: vec![clip("A", 0.0), clip("B", 1.0)],
            segments: vec![scored("A"), scored("B")],
            ..Index::default()
        }
    }

    fn tiny_map() -> crate::beats::MusicMap {
        crate::beats::MusicMap {
            path: "m.wav".into(),
            duration: 60.0,
            bpm: 120.0,
            beats: (0..120).map(|i| i as f32 * 0.5).collect(),
            bars: (0..30)
                .map(|i| crate::beats::Bar {
                    t: i as f32 * 2.0,
                    rms: 1.0,
                    low: 1.0,
                })
                .collect(),
            ..crate::beats::MusicMap::default()
        }
    }

    #[test]
    fn the_check_catches_repeats() -> anyhow::Result<()> {
        let shot = |clip: &str, t: f32| super::Shot {
            clip: clip.into(),
            t_in: t,
            beats: 4,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: String::new(),
        };
        let mut plan = Plan {
            name: "t".into(),
            aspect: Aspect::Vertical,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 0,
            card_bar: 3,
            end_bar: 4,
            end_card: String::new(),
            rules: Rules::default(),
            finish: super::Finish::default(),
            shots: vec![shot("A", 10.0), shot("B", 10.0), shot("A", 60.0)],
            comments: Vec::new(),
        };
        let people = crate::people::People::default();
        let problems = super::check(&plan, &tiny_index(), &tiny_map(), &people);
        assert!(
            problems
                .iter()
                .any(|p| p.shot == 3 && p.message.contains("`A` again")),
            "same person 2 shots apart: {problems:?}"
        );
        assert!(
            !problems.iter().any(|p| p.message.contains("same stretch")),
            "60 s apart is fine: {problems:?}"
        );
        plan.shots[2].t_in = 20.0;
        let problems = super::check(&plan, &tiny_index(), &tiny_map(), &people);
        assert!(
            problems
                .iter()
                .any(|p| p.shot == 3 && p.message.contains("same stretch")),
            "10 s apart repeats: {problems:?}"
        );
        plan.shots[2].beats = 8;
        let problems = super::check(&plan, &tiny_index(), &tiny_map(), &people);
        assert!(
            problems
                .iter()
                .any(|p| p.shot == 0 && p.message.contains("beats")),
            "beat count off: {problems:?}"
        );
        Ok(())
    }

    /// A two-shot plan, A then B, both 4 beats in at 10 s.
    fn two_shots() -> Plan {
        let shot = |clip: &str| super::Shot {
            clip: clip.into(),
            t_in: 10.0,
            beats: 4,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: String::new(),
        };
        Plan {
            name: "t".into(),
            aspect: Aspect::Wide,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 0,
            card_bar: 2,
            end_bar: 2,
            end_card: String::new(),
            rules: Rules::default(),
            finish: super::Finish::default(),
            shots: vec![shot("A"), shot("B")],
            comments: Vec::new(),
        }
    }

    #[test]
    fn look_alikes_need_the_same_spot_and_performer() -> anyhow::Result<()> {
        let mut index = tiny_index();
        let sig = crate::analyze::segments::Signature {
            hist: (0..32).map(|i| if i == 3 { 1.0 } else { 0.0 }).collect(),
            grid: (0..16).map(|i| if i == 5 { 1.0 } else { 0.0 }).collect(),
            cx: 0.5,
        };
        for g in &mut index.segments {
            g.seg.sig = sig.clone();
        }
        let plan = two_shots();
        let unnamed = crate::people::People::default();
        let problems = super::check_with(
            &plan,
            &index,
            &tiny_map(),
            &unnamed,
            &super::Memory::default(),
        );
        assert!(
            problems
                .iter()
                .any(|p| p.shot == 2 && p.message.contains("looks like shot 1")),
            "identical signatures, nobody named: {problems:?}"
        );
        let mut named = crate::people::People::default();
        named.names.insert("A".into(), "ana".into());
        named.names.insert("B".into(), "bo".into());
        let problems = super::check_with(
            &plan,
            &index,
            &tiny_map(),
            &named,
            &super::Memory::default(),
        );
        assert!(
            !problems.iter().any(|p| p.message.contains("looks like")),
            "two different performers at one spot are not a repeat: {problems:?}"
        );
        // A different spot: no look-alike even when unnamed.
        index.segments[1].seg.sig.hist = (0..32).map(|i| if i == 20 { 1.0 } else { 0.0 }).collect();
        let problems = super::check_with(
            &plan,
            &index,
            &tiny_map(),
            &unnamed,
            &super::Memory::default(),
        );
        assert!(
            !problems.iter().any(|p| p.message.contains("looks like")),
            "another spot: {problems:?}"
        );
        Ok(())
    }

    #[test]
    fn moments_from_other_nights_are_remembered() -> anyhow::Result<()> {
        let root = tempfile::tempdir()?;
        let past = root.path().join("2026-11-04").join("_ave");
        std::fs::create_dir_all(past.join("plans"))?;
        std::fs::create_dir_all(past.join("clips"))?;
        let mut old = two_shots();
        old.name = "reel-30".into();
        old.shots.truncate(1);
        old.save(&past.join("plans").join("reel-30.toml"))?;
        std::fs::write(past.join("music.json"), r#"{"bpm": 120.0}"#)?;
        std::fs::write(past.join("clips").join("A.json"), r#"{"stamp": [0, 0]}"#)?;
        let work = root.path().join("2026-11-11").join("_ave");
        std::fs::create_dir_all(&work)?;
        // This night's own plans never count.
        two_shots().save(&work.join("plans").join("mine.toml"))?;

        let memory = super::Memory::load(&work);
        assert_eq!(memory.used.len(), 1, "one used moment: {memory:?}");
        let u = &memory.used[0];
        assert!(
            u.night == "2026-11-04" && (u.t1 - 12.0).abs() < 1e-3 && u.size == Some(0),
            "range from the old tempo, size from the old cache: {u:?}"
        );
        let index = tiny_index();
        let people = crate::people::People::default();
        let problems = super::check_with(&two_shots(), &index, &tiny_map(), &people, &memory);
        assert!(
            problems
                .iter()
                .any(|p| p.shot == 1 && p.message.contains("reuses A 10.0-12.0 from 2026-11-04")),
            "shot 1 reuses last week's moment: {problems:?}"
        );
        assert!(
            !problems
                .iter()
                .any(|p| p.shot == 2 && p.message.contains("reuses")),
            "B was never used: {problems:?}"
        );
        let mut later = two_shots();
        later.shots[0].t_in = 40.0;
        let problems = super::check_with(&later, &index, &tiny_map(), &people, &memory);
        assert!(
            !problems.iter().any(|p| p.message.contains("reuses")),
            "another moment of the same clip is fine: {problems:?}"
        );
        Ok(())
    }
}

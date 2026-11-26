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
//!
//! The check returns [`Finding`]s, each with a stable id ([`IDS`]) and a
//! [`Severity`]: hard when the plan is invalid or would render wrong, soft
//! when a taste rule is bent. Shot findings name a shot; seam findings
//! (jump cuts, exposure and colour jumps) name the cut between two.

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
    /// Whether a shot reusing a moment from another night's plan is reported
    /// (soft `reused`; see [`Memory`]). The drafter avoids them either way.
    pub(crate) reject_reused: bool,
    /// Shots whose source range carries any of these flags are reported
    /// (soft `flagged`); a shot overlapping a bad span (see
    /// [`crate::analyze::spans`]) with one of them is hard `bad-span`.
    pub(crate) reject_flags: Vec<String>,
    /// The shot on the drop must have this subject (empty: no requirement).
    pub(crate) drop_subject: String,
    /// Caption moments that are reported (soft `bad-moment`, e.g.
    /// `transition`).
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
    /// Markers: review comments pinned to moments of the cut (`[[comment]]`,
    /// also read as `[[marker]]`).
    #[serde(
        default,
        rename = "comment",
        alias = "marker",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub(crate) comments: Vec<Comment>,
    /// Soft findings the editor accepted for this cut, by [`Plan::accept_key`]
    /// (`colour-jump@3 MVI_7925@12.30x4 MVI_7990@40.10x4`): they no longer
    /// count. The key names the shots involved, so a finding comes back
    /// once its shots change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) accepted: Vec<String>,
}

/// A marker: a moment of the cut with a colour and, usually, a note. Like
/// Selects, a marker with a note *is* a review comment, so markers and
/// comments are one thing. It belongs to the plan, not to a shot, so it
/// stays at its time when the shots under it change, and version restores
/// keep the live plan's markers.
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

    /// Where the end card sits, seconds from the start bar: `(start, end)`,
    /// i.e. `card_bar` to `end_bar` on the music map. The render holds the
    /// end card image over this span (and stops at `start` without one);
    /// timeline playback holds it for the same span.
    pub(crate) fn card_span(&self, map: &MusicMap) -> (f32, f32) {
        let origin = map.beat_time(self.start_bar, 0);
        (
            map.beat_time(self.card_bar, 0) - origin,
            map.beat_time(self.end_bar, 0) - origin,
        )
    }

    /// Beats of footage before the end card.
    pub(crate) fn footage_beats(&self) -> usize {
        (self.card_bar - self.start_bar) * 4
    }

    /// The key a finding is accepted by: its id, where it is, and the shots
    /// it is about (clip, in-point and beats), so the acceptance lapses when
    /// any of them changes. `colour-jump@3 A@12.30x4 B@40.10x4` for a seam,
    /// `flagged@5 A@12.30x4` for a shot, `off-drop@plan` for the plan.
    pub(crate) fn accept_key(&self, f: &Finding) -> String {
        let shot = |n: usize| {
            self.shots.get(n.wrapping_sub(1)).map_or_else(
                || "?".to_owned(),
                |s| format!("{}@{:.2}x{}", s.clip, s.t_in, s.beats),
            )
        };
        match (f.seam, f.shot) {
            (Some(n), _) => format!("{}@{n} {} {}", f.id, shot(n), shot(n + 1)),
            (None, Some(n)) => format!("{}@{n} {}", f.id, shot(n)),
            (None, None) => format!("{}@plan", f.id),
        }
    }

    /// Whether `f` is a soft finding the editor accepted for this cut.
    /// Hard findings can never be accepted.
    pub(crate) fn is_accepted(&self, f: &Finding) -> bool {
        !f.is_hard() && self.accepted.contains(&self.accept_key(f))
    }

    /// Accepts soft finding `f` (`on`) or takes the acceptance back; whether
    /// anything changed.
    pub(crate) fn set_accepted(&mut self, f: &Finding, on: bool) -> bool {
        if f.is_hard() {
            return false;
        }
        let key = self.accept_key(f);
        let had = self.accepted.contains(&key);
        if on && !had {
            self.accepted.push(key);
        } else if !on && had {
            self.accepted.retain(|k| *k != key);
        }
        on != had
    }
}

/// How much a finding matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Severity {
    /// The plan is invalid or the render would come out wrong (timing off
    /// the grid, a range outside its clip, a shot with no frames). `ave
    /// check` fails and `ave render` refuses without `--force`.
    Hard,
    /// A taste rule bent (a repeat, a flagged range, a jump cut, a colour
    /// jump). Reported, never fatal: whether it matters is the editor's
    /// call.
    Soft,
}

impl Severity {
    /// `hard` or `soft`, as `ave check` prints it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Hard => "hard",
            Self::Soft => "soft",
        }
    }
}

/// One thing the check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    /// Stable kebab-case id of the rule (`bad-span`, `jump-cut`, ...): what
    /// scripts, the assistant and the GUI match on. Every id is in [`IDS`].
    pub(crate) id: &'static str,
    /// Whether it fails the check.
    pub(crate) severity: Severity,
    /// The shot it is about (1-based), `None` for whole-plan findings. A
    /// seam finding names the shot after the cut, so per-shot views see it.
    pub(crate) shot: Option<usize>,
    /// The cut it is about: `Some(n)` is the seam between shot `n` and shot
    /// `n + 1` (1-based). `None` for shot and plan findings.
    pub(crate) seam: Option<usize>,
    /// What is wrong, in words.
    pub(crate) message: String,
}

impl Finding {
    /// Whether this finding fails the check.
    pub(crate) fn is_hard(&self) -> bool {
        self.severity == Severity::Hard
    }

    /// Where it is: `plan`, `shot 3`, or `cut 3|4`.
    pub(crate) fn place(&self) -> String {
        match (self.seam, self.shot) {
            (Some(n), _) => format!("cut {}|{}", n, n + 1),
            (None, Some(n)) => format!("shot {n}"),
            (None, None) => "plan".to_owned(),
        }
    }

    /// `shot 3 [hard bad-span] A 1.00-9.00 runs outside the clip`: one line
    /// for people, logs and the assistant.
    pub(crate) fn text(&self) -> String {
        format!(
            "{} [{} {}] {}",
            self.place(),
            self.severity.as_str(),
            self.id,
            self.message
        )
    }
}

/// Every finding id with its severity and what it means, in check order.
/// `ave check --help` and the MCP instructions point here.
pub(crate) const IDS: [(&str, Severity, &str); 21] = [
    (
        "beat-count",
        Severity::Hard,
        "the shots do not fill the bars to the end card",
    ),
    (
        "bar-order",
        Severity::Hard,
        "bars do not run start <= drop < card <= end",
    ),
    (
        "past-track",
        Severity::Hard,
        "the end bar is past the track",
    ),
    (
        "off-drop",
        Severity::Hard,
        "no shot starts exactly on the drop",
    ),
    (
        "drop-subject",
        Severity::Soft,
        "the drop shot's subject is not the rule's",
    ),
    (
        "missing-clip",
        Severity::Hard,
        "the shot's clip is not in the index",
    ),
    (
        "outside-clip",
        Severity::Hard,
        "the shot runs outside its clip",
    ),
    (
        "bad-span",
        Severity::Hard,
        "the shot overlaps a bad span (a short bad stretch) with a rejected flag",
    ),
    (
        "crop-outside",
        Severity::Hard,
        "the crop centre puts the crop outside the frame",
    ),
    (
        "empty-shot",
        Severity::Hard,
        "the shot gets no frames at the plan's frame rate",
    ),
    (
        "flash-shot",
        Severity::Soft,
        "the shot is shorter than a flash (FLASH_FRAMES)",
    ),
    (
        "flagged",
        Severity::Soft,
        "the range carries a rejected flag (dark, whip, soft)",
    ),
    (
        "bad-moment",
        Severity::Soft,
        "the range is captioned as a rejected moment",
    ),
    (
        "wide-action",
        Severity::Soft,
        "the action wanders wider than the crop",
    ),
    (
        "same-stretch",
        Severity::Soft,
        "two shots start too close together in one clip",
    ),
    (
        "repeat-performer",
        Severity::Soft,
        "the same performer within person_gap shots",
    ),
    (
        "lookalike",
        Severity::Soft,
        "reads as the same shot as an earlier one",
    ),
    (
        "reused",
        Severity::Soft,
        "reuses a moment from another night's plan",
    ),
    (
        "jump-cut",
        Severity::Soft,
        "a cut within one clip that skips a few seconds",
    ),
    (
        "exposure-jump",
        Severity::Soft,
        "brightness jumps across the cut",
    ),
    (
        "colour-jump",
        Severity::Soft,
        "mid-tone colour jumps across the cut",
    ),
];

/// The severity an id is registered with in [`IDS`] (hard for an id that
/// is not there, so a typo cannot hide a failure).
fn severity_of(id: &str) -> Severity {
    IDS.iter()
        .find(|(i, _, _)| *i == id)
        .map_or(Severity::Hard, |(_, s, _)| *s)
}

/// A shot shorter than this many frames at the plan's frame rate reads as
/// a flash, not a shot. Six frames is a quarter second at 24 fps: under it
/// the eye registers that something changed but not what (the editors'
/// "flash frame"). One beat at 180 BPM is still 8 frames, so a plan on the
/// beat grid only trips this with a cut lead or a very fast track.
pub(crate) const FLASH_FRAMES: i64 = 6;

/// Two shots of one clip cut together with a source gap under this many
/// seconds read as a jump cut: the camera and the spot are the same and the
/// performer is mid-routine, so the picture visibly skips. Beyond ten
/// seconds the routine has moved on enough that the cut reads as a new
/// shot (the `same_clip_gap` rule judges those).
pub(crate) const JUMP_CUT_GAP: f32 = 10.0;

/// A cut whose frame brightness changes by more than this many stops
/// (log2 of the mean-luma ratio, edge samples' medians) is an exposure
/// jump. The render's exposure match moves a shot by at most 1.6x gain
/// (about 0.7 stop), so 1.5 stops still leaves most of a stop at the cut.
/// Calibrated on 2026-11-11: random pairs of samples sit at a median 0.63
/// stop and a p90 of 1.55; the five plans' 181 cuts, single edge samples,
/// at a median 0.40 with one above 1.5 (the 0.5 s medians the check takes
/// flag none of them).
pub(crate) const EXPOSURE_JUMP_STOPS: f32 = 1.5;

/// Mean luma below this is treated as this when taking the ratio, so two
/// near-black frames (where noise dominates) never count as a jump.
const LUMA_FLOOR: f32 = 0.02;

/// A cut whose mid-tone chromaticity (r and g of r+g+b) moves farther than
/// this is a colour jump: beyond what the render's ±15 % mid-tone match can
/// close. Calibrated on 2026-11-11: random pairs of samples sit at a median
/// 0.12, p95 0.35, p99 0.45 (LED nights change colour a lot, and that is
/// the look); 0.40 flags the near-complementary jumps only (seven of 181
/// cuts in the five plans).
pub(crate) const COLOUR_JUMP: f32 = 0.40;

/// Seconds of a shot at each side of a cut whose samples are measured
/// (their median), so one flicker of flame at the edge is not a jump.
const EDGE_WINDOW: f32 = 0.5;

/// Brightness and mid-tone chromaticity of `clip` between `t0` and `t1`:
/// medians of the samples there, or the nearest sample when none fall in
/// (samples are about six a second). The chromaticity is `None` when the
/// mid-tones cover under 1 % of the frame (nothing to judge colour on).
fn edge_look(index: &Index, clip: &str, t0: f32, t1: f32) -> Option<(f32, Option<[f32; 2]>)> {
    use crate::analyze::segments::median;
    let c = index.clip(clip)?;
    let mut w: Vec<&crate::analyze::Sample> = c
        .samples
        .iter()
        .filter(|s| s.t >= t0 && s.t <= t1)
        .collect();
    if w.is_empty() {
        let mid = (t0 + t1) / 2.0;
        let near = c.samples.iter().min_by(|a, b| {
            (a.t - mid)
                .abs()
                .partial_cmp(&(b.t - mid).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
        w.push(near);
    }
    let luma = median(&w.iter().map(|s| s.luma).collect::<Vec<_>>());
    let mids: Vec<&&crate::analyze::Sample> = w.iter().filter(|s| s.mid_frac >= 0.01).collect();
    let chroma = (!mids.is_empty()).then(|| {
        let ch = |f: fn(&crate::analyze::Sample) -> f32| {
            median(&mids.iter().map(|s| f(s)).collect::<Vec<_>>())
        };
        let (r, g, b) = (ch(|s| s.mid_r), ch(|s| s.mid_g), ch(|s| s.mid_b));
        let sum = (r + g + b).max(1e-4);
        [r / sum, g / sum]
    });
    Some((luma, chroma))
}

/// The plan's frame rate (the first shot's clip's, as the render uses) and
/// each shot's length in output frames, cut lead included, exactly as
/// [`crate::render`] trims them.
fn frame_counts(plan: &Plan, index: &Index, timing: &[(f32, f32)], card: f32) -> (f64, Vec<i64>) {
    let rate = plan
        .shots
        .first()
        .and_then(|s| index.clip(&s.clip))
        .map_or(24000.0 / 1001.0, |c| {
            f64::from(c.probe.fps[0]) / f64::from(c.probe.fps[1].max(1))
        });
    let frames = |sec: f32| (f64::from(sec) * rate).round() as i64;
    let lead = i64::from(plan.finish.cut_lead_frames);
    let mut bounds: Vec<i64> = vec![0];
    for (s, _) in timing.iter().skip(1) {
        bounds.push(frames(*s) - lead);
    }
    bounds.push(frames(card) - if plan.end_card.is_empty() { 0 } else { lead });
    (rate, bounds.windows(2).map(|w| w[1] - w[0]).collect())
}

/// Checks `plan` against its rules, the index and the music map.
pub(crate) fn check(plan: &Plan, index: &Index, map: &MusicMap, people: &People) -> Vec<Finding> {
    check_with(plan, index, map, people, &Memory::for_index(index))
}

/// [`check`] against an explicit cross-week memory.
pub(crate) fn check_with(
    plan: &Plan,
    index: &Index,
    map: &MusicMap,
    people: &People,
    memory: &Memory,
) -> Vec<Finding> {
    let mut problems = Vec::new();
    let mut push = |id: &'static str, shot: usize, message: String| {
        problems.push(Finding {
            id,
            severity: severity_of(id),
            shot: (shot > 0).then_some(shot),
            seam: None,
            message,
        });
    };
    let r = &plan.rules;

    let beats: usize = plan.shots.iter().map(|s| s.beats).sum();
    if beats != plan.footage_beats() {
        push(
            "beat-count",
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
        push(
            "bar-order",
            0,
            "bars must run start <= drop < card <= end".to_owned(),
        );
    }
    if plan.end_bar > map.bars.len() + 2 {
        push(
            "past-track",
            0,
            format!(
                "end bar {} is past the track's {} bars",
                plan.end_bar,
                map.bars.len()
            ),
        );
    }

    let timing = plan.timing(map);
    let drop_beat = plan.drop_bar.saturating_sub(plan.start_bar) * 4;
    let (rate, frames) = frame_counts(plan, index, &timing, plan.card_span(map).0);
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
            // With no music there is no drop to land a subject on.
            if !map.no_music() && !r.drop_subject.is_empty() && subject != r.drop_subject {
                push(
                    "drop-subject",
                    n,
                    format!(
                        "the drop lands on a `{subject}` shot, not `{}`",
                        r.drop_subject
                    ),
                );
            }
        }
        beat += shot.beats;
        if let Some(&f) = frames.get(k) {
            if f <= 0 {
                push(
                    "empty-shot",
                    n,
                    format!("gets {f} frames at {rate:.3} fps: the render cannot cut it"),
                );
            } else if f < FLASH_FRAMES {
                push(
                    "flash-shot",
                    n,
                    format!(
                        "only {f} frames at {rate:.3} fps (under {FLASH_FRAMES}): reads as a flash"
                    ),
                );
            }
        }
        let Some(clip) = index.clip(&shot.clip) else {
            push(
                "missing-clip",
                n,
                format!("clip {} is not in the index", shot.clip),
            );
            continue;
        };
        let t_out = shot.t_in + dur;
        if shot.t_in < 0.0 || f64::from(t_out) > clip.probe.duration {
            push(
                "outside-clip",
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
                "crop-outside",
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
                    "flagged",
                    n,
                    format!(
                        "overlaps {} which is flagged {}",
                        g.id,
                        bad.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" ")
                    ),
                );
            }
            for b in
                crate::analyze::spans::hits(&g.seg.bad_spans, shot.t_in, t_out, &r.reject_flags)
            {
                push(
                    "bad-span",
                    n,
                    format!("overlaps a bad span in {}: {}", g.id, b.text()),
                );
            }
            if let Some(c) = &g.caption
                && r.reject_moments.contains(&c.effective_moment().to_owned())
            {
                push(
                    "bad-moment",
                    n,
                    format!(
                        "overlaps {} captioned as `{}`: {}",
                        g.id, c.moment, c.summary
                    ),
                );
            }
            if crop < 1.0 && g.seg.cx_spread > crop * 1.2 && g.subject != "none" {
                push(
                    "wide-action",
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
                    "same-stretch",
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
                    "repeat-performer",
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
                    "lookalike",
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
                "reused",
                n,
                format!(
                    "reuses {} {:.1}-{:.1} from {} `{}`",
                    u.clip, u.t0, u.t1, u.night, u.plan
                ),
            );
        }
    }
    if !hit_drop && !map.no_music() {
        push(
            "off-drop",
            0,
            format!("no shot starts exactly on the drop (bar {})", plan.drop_bar),
        );
    }
    problems.extend(seams(plan, index, &timing, drop_beat));
    problems
}

/// The seam checks: what happens across each cut (shot `n` out, shot
/// `n + 1` in). Jump cuts within one clip, then exposure and colour jumps
/// between the edge samples. The cut onto the drop is spared the exposure
/// check: a dark build snapping to a bright hero shot is the point.
fn seams(plan: &Plan, index: &Index, timing: &[(f32, f32)], drop_beat: usize) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |id: &'static str, n: usize, message: String| {
        out.push(Finding {
            id,
            severity: severity_of(id),
            shot: Some(n + 1),
            seam: Some(n),
            message,
        });
    };
    let mut beat = 0;
    for (k, pair) in plan.shots.windows(2).enumerate() {
        let (a, b) = (&pair[0], &pair[1]);
        let n = k + 1;
        beat += a.beats;
        let Some(&(_, dur)) = timing.get(k) else {
            continue;
        };
        let a_out = a.t_in + dur;
        if a.clip == b.clip {
            let gap = b.t_in - a_out;
            // A gap within half a sample step is the same moment carried on.
            if gap.abs() < JUMP_CUT_GAP && gap.abs() > 0.05 {
                push(
                    "jump-cut",
                    n,
                    format!(
                        "{} cuts to itself {} {:.1}s: reads as a jump cut (under {JUMP_CUT_GAP:.0}s)",
                        a.clip,
                        if gap > 0.0 { "skipping" } else { "back" },
                        gap.abs()
                    ),
                );
            }
        }
        let b_dur = timing.get(k + 1).map_or(EDGE_WINDOW, |&(_, d)| d);
        let Some(((la, ca), (lb, cb))) =
            edges(index, (&a.clip, a.t_in, dur), (&b.clip, b.t_in, b_dur))
        else {
            continue;
        };
        let stops = (la.max(LUMA_FLOOR) / lb.max(LUMA_FLOOR)).log2();
        if stops.abs() > EXPOSURE_JUMP_STOPS && beat != drop_beat {
            push(
                "exposure-jump",
                n,
                format!(
                    "brightness {} {:.1} stops across the cut (luma {la:.3} to {lb:.3}, over {EXPOSURE_JUMP_STOPS:.1})",
                    if stops > 0.0 { "drops" } else { "jumps" },
                    stops.abs()
                ),
            );
        }
        if let (Some(x), Some(y)) = (ca, cb) {
            let d = (x[0] - y[0]).hypot(x[1] - y[1]);
            if d > COLOUR_JUMP {
                push(
                    "colour-jump",
                    n,
                    format!(
                        "mid-tone colour moves {d:.2} across the cut (rg {:.2},{:.2} to {:.2},{:.2}; over {COLOUR_JUMP:.2})",
                        x[0], x[1], y[0], y[1]
                    ),
                );
            }
        }
    }
    out
}

/// One side of a cut: clip, in-point and length in seconds.
pub(crate) type Side<'a> = (&'a str, f32, f32);

/// The look at each side of a cut from `a` to `b`: the last
/// [`EDGE_WINDOW`] of `a` and the first of `b`, as the seam check reads
/// them (brightness, mid-tone chromaticity).
#[expect(
    clippy::type_complexity,
    reason = "two looks, each a brightness and an optional chromaticity, read once"
)]
fn edges(
    index: &Index,
    a: Side<'_>,
    b: Side<'_>,
) -> Option<((f32, Option<[f32; 2]>), (f32, Option<[f32; 2]>))> {
    let a_out = a.1 + a.2;
    Some((
        edge_look(index, a.0, (a_out - EDGE_WINDOW).max(a.1), a_out)?,
        edge_look(index, b.0, b.1, b.1 + EDGE_WINDOW.min(b.2))?,
    ))
}

/// How hard a cut from `a` to `b` jumps: stops of brightness change and
/// the mid-tone colour distance (`None` with nothing to judge colour on),
/// measured as the seam check measures them; `None` without samples.
pub(crate) fn seam_jump(index: &Index, a: Side<'_>, b: Side<'_>) -> Option<(f32, Option<f32>)> {
    let ((la, ca), (lb, cb)) = edges(index, a, b)?;
    let stops = (la.max(LUMA_FLOOR) / lb.max(LUMA_FLOOR)).log2().abs();
    let colour = match (ca, cb) {
        (Some(x), Some(y)) => Some((x[0] - y[0]).hypot(x[1] - y[1])),
        (Some(_) | None, _) => None,
    };
    Some((stops, colour))
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

/// How many findings are hard.
pub(crate) fn hard_count(findings: &[Finding]) -> usize {
    findings.iter().filter(|f| f.is_hard()).count()
}

/// Renders findings as text: one line per finding, prefixed with where it
/// is, its severity and its id, then a verdict line (`ok`, `ok with N
/// soft`, or `fails: N hard, M soft`). Only hard findings fail.
pub(crate) fn report(plan: &Plan, problems: &[Finding]) -> String {
    let mut s = String::new();
    for p in problems {
        let accepted = if plan.is_accepted(p) {
            " (accepted)"
        } else {
            ""
        };
        let _ = writeln!(s, "{} {}{accepted}", plan.name, p.text());
    }
    let hard = hard_count(problems);
    let accepted = accepted_count(plan, problems);
    let soft = problems.len() - hard - accepted;
    let shots = plan.shots.len();
    let tail = if accepted > 0 {
        format!(", {accepted} accepted")
    } else {
        String::new()
    };
    let _ = match (hard, soft) {
        (0, 0) => writeln!(s, "{}: ok ({shots} shots){tail}", plan.name),
        (0, _) => writeln!(s, "{}: ok ({shots} shots), {soft} soft{tail}", plan.name),
        (_, _) => writeln!(s, "{}: fails: {hard} hard, {soft} soft{tail}", plan.name),
    };
    s
}

/// How many of `findings` the plan has accepted (soft ones only).
pub(crate) fn accepted_count(plan: &Plan, findings: &[Finding]) -> usize {
    findings.iter().filter(|f| plan.is_accepted(f)).count()
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
            cheers: Vec::new(),
            sig: Default::default(),
            bad_spans: Vec::new(),
        };
        let probe = Probe {
            duration: 120.0,
            width: 1920,
            height: 1080,
            fps: [24000, 1001],
            vcodec: "h264".into(),
            has_audio: false,
            recorded_at: None,
            camera: None,
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
    fn with_no_music_there_is_no_drop_to_miss() -> anyhow::Result<()> {
        let shot = |beats: usize| super::Shot {
            clip: "A".into(),
            t_in: 10.0,
            beats,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: String::new(),
        };
        // Shots start on beats 0, 3 and 8: none on the drop (bar 1, beat 4).
        let mut plan = Plan {
            name: "t".into(),
            aspect: Aspect::Vertical,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 1,
            card_bar: 3,
            end_bar: 4,
            end_card: String::new(),
            rules: Rules::default(),
            finish: super::Finish::default(),
            shots: vec![shot(3), shot(5), shot(4)],
            comments: Vec::new(),
            accepted: Vec::new(),
        };
        let people = crate::people::People::default();
        let problems = super::check(&plan, &tiny_index(), &tiny_map(), &people);
        assert!(
            problems.iter().any(|p| p.id == "off-drop"),
            "with music, a cut that misses the drop is flagged: {problems:?}"
        );
        plan.music = String::new();
        let paced = crate::beats::MusicMap::paced();
        let problems = super::check(&plan, &tiny_index(), &paced, &people);
        assert!(
            !problems
                .iter()
                .any(|p| p.id == "off-drop" || p.id == "drop-subject"),
            "with no music there is no drop: {problems:?}"
        );
        Ok(())
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
            accepted: Vec::new(),
        };
        let people = crate::people::People::default();
        let problems = super::check(&plan, &tiny_index(), &tiny_map(), &people);
        assert!(
            problems
                .iter()
                .any(|p| p.shot == Some(3) && p.message.contains("`A` again")),
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
                .any(|p| p.shot == Some(3) && p.message.contains("same stretch")),
            "10 s apart repeats: {problems:?}"
        );
        plan.shots[2].beats = 8;
        let problems = super::check(&plan, &tiny_index(), &tiny_map(), &people);
        assert!(
            problems
                .iter()
                .any(|p| p.shot.is_none() && p.id == "beat-count"),
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
            accepted: Vec::new(),
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
                .any(|p| p.shot == Some(2) && p.message.contains("looks like shot 1")),
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
                .any(|p| p.shot == Some(1)
                    && p.message.contains("reuses A 10.0-12.0 from 2026-11-04")),
            "shot 1 reuses last week's moment: {problems:?}"
        );
        assert!(
            !problems
                .iter()
                .any(|p| p.shot == Some(2) && p.message.contains("reuses")),
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

    /// The findings of `plan` on the tiny night, no memory, nobody named.
    fn findings(plan: &Plan, index: &crate::index::Index) -> Vec<super::Finding> {
        super::check_with(
            plan,
            index,
            &tiny_map(),
            &crate::people::People::default(),
            &super::Memory::default(),
        )
    }

    #[test]
    fn every_id_is_registered_once_with_its_severity() -> anyhow::Result<()> {
        use super::{IDS, Severity};
        let mut seen = std::collections::BTreeSet::new();
        for (id, _, what) in IDS {
            assert!(seen.insert(id), "{id} registered twice");
            assert!(
                !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{id} is kebab-case"
            );
            assert!(!what.is_empty(), "{id} says what it means");
        }
        let hard: Vec<&str> = IDS
            .iter()
            .filter(|(_, s, _)| *s == Severity::Hard)
            .map(|(id, _, _)| *id)
            .collect();
        assert_eq!(
            hard,
            vec![
                "beat-count",
                "bar-order",
                "past-track",
                "off-drop",
                "missing-clip",
                "outside-clip",
                "bad-span",
                "crop-outside",
                "empty-shot"
            ],
            "only what makes the plan invalid or the render wrong is hard"
        );
        assert_eq!(
            super::severity_of("no-such-rule"),
            Severity::Hard,
            "an unregistered id never hides a failure"
        );
        Ok(())
    }

    #[test]
    fn existing_rules_map_to_ids_and_severities() -> anyhow::Result<()> {
        let mut plan = two_shots();
        plan.shots[1].clip = "Z".into();
        plan.shots[0].t_in = 119.0;
        plan.shots.push(plan.shots[0].clone());
        let got = findings(&plan, &tiny_index());
        let has = |id: &str, shot: Option<usize>, hard: bool| {
            got.iter()
                .any(|f| f.id == id && f.shot == shot && f.is_hard() == hard)
        };
        assert!(has("beat-count", None, true), "12 beats in 8: {got:?}");
        assert!(
            has("missing-clip", Some(2), true),
            "Z is not indexed: {got:?}"
        );
        assert!(
            has("outside-clip", Some(1), true),
            "119 + 2 s > 120: {got:?}"
        );
        assert!(
            has("same-stretch", Some(3), false),
            "A again at 119: {got:?}"
        );
        for f in &got {
            assert!(
                super::IDS
                    .iter()
                    .any(|(id, s, _)| *id == f.id && *s == f.severity),
                "{} is registered with its severity",
                f.id
            );
            assert!(f.seam.is_none(), "no seam findings here: {f:?}");
        }
        let text = super::report(&plan, &got);
        assert!(
            text.contains("t shot 2 [hard missing-clip] clip Z is not in the index")
                && text.contains("t plan [hard beat-count] shots fill 12 beats"),
            "lines carry place, severity and id: {text}"
        );
        assert!(
            text.trim_end().ends_with("t: fails: 5 hard, 2 soft"),
            "verdict last: {text}"
        );
        Ok(())
    }

    #[test]
    fn soft_findings_do_not_fail_the_report() -> anyhow::Result<()> {
        let mut plan = two_shots();
        plan.shots[1].clip = "A".into();
        plan.shots[1].t_in = 14.0;
        let got = findings(&plan, &tiny_index());
        assert!(
            !got.is_empty() && super::hard_count(&got) == 0,
            "only soft findings: {got:?}"
        );
        let text = super::report(&plan, &got);
        assert!(
            text.trim_end()
                .ends_with(&format!("t: ok (2 shots), {} soft", got.len())),
            "passes with the soft count: {text}"
        );
        Ok(())
    }

    #[test]
    fn a_cut_within_one_clip_that_skips_a_little_is_a_jump_cut() -> anyhow::Result<()> {
        let mut plan = two_shots();
        plan.rules.same_clip_gap = 0.0;
        plan.shots[1].clip = "A".into();
        // Shot 1 plays A 10-12; shot 2 picks A up at 14: a 2 s skip.
        plan.shots[1].t_in = 14.0;
        let got = findings(&plan, &tiny_index());
        let jump = got.iter().find(|f| f.id == "jump-cut");
        assert!(
            jump.is_some_and(|f| f.seam == Some(1)
                && f.shot == Some(2)
                && !f.is_hard()
                && f.message.contains("skipping 2.0s")),
            "soft jump cut at cut 1|2: {got:?}"
        );
        assert_eq!(
            jump.map(super::Finding::place),
            Some("cut 1|2".to_owned()),
            "a seam's place"
        );
        // Picked up where it left off: one continuous take, no jump.
        plan.shots[1].t_in = 12.0;
        assert!(
            !findings(&plan, &tiny_index())
                .iter()
                .any(|f| f.id == "jump-cut"),
            "a continuous cut is not a jump"
        );
        // Far enough on, it reads as a new shot.
        plan.shots[1].t_in = 12.0 + super::JUMP_CUT_GAP + 1.0;
        assert!(
            !findings(&plan, &tiny_index())
                .iter()
                .any(|f| f.id == "jump-cut"),
            "beyond the gap"
        );
        // Going back a little is a jump too.
        plan.shots[1].t_in = 9.0;
        assert!(
            findings(&plan, &tiny_index())
                .iter()
                .any(|f| f.id == "jump-cut" && f.message.contains("back 3.0s")),
            "backwards"
        );
        Ok(())
    }

    #[test]
    fn short_shots_are_flashes_and_frameless_ones_are_hard() -> anyhow::Result<()> {
        let mut plan = two_shots();
        // 120 BPM at 23.976 fps: a beat is 12 frames; an 8-frame cut lead
        // leaves shot 1 with 12 - 8 = 4.
        plan.shots[0].beats = 1;
        plan.shots[1].beats = 7;
        plan.finish.cut_lead_frames = 8;
        let got = findings(&plan, &tiny_index());
        assert!(
            got.iter().any(|f| f.id == "flash-shot"
                && f.shot == Some(1)
                && !f.is_hard()
                && f.message.contains("only 4 frames")),
            "4 frames is a flash: {got:?}"
        );
        assert!(
            !got.iter()
                .any(|f| f.id == "flash-shot" && f.shot == Some(2)),
            "shot 2 is long: {got:?}"
        );
        plan.finish.cut_lead_frames = 0;
        assert!(
            !findings(&plan, &tiny_index())
                .iter()
                .any(|f| f.id == "flash-shot"),
            "12 frames is a shot"
        );
        plan.shots[0].beats = 0;
        plan.shots[1].beats = 8;
        let got = findings(&plan, &tiny_index());
        assert!(
            got.iter()
                .any(|f| f.id == "empty-shot" && f.shot == Some(1) && f.is_hard()),
            "no frames cannot render: {got:?}"
        );
        Ok(())
    }

    /// Gives clip `clip` of `index` a sample every 0.25 s with this
    /// brightness and mid-tone colour.
    fn paint(index: &mut crate::index::Index, clip: &str, luma: f32, mid: [f32; 3]) {
        for c in index.clips.iter_mut().filter(|c| c.id == clip) {
            c.samples = (0..480)
                .map(|i| crate::analyze::Sample {
                    t: i as f32 * 0.25,
                    luma,
                    mid_r: mid[0],
                    mid_g: mid[1],
                    mid_b: mid[2],
                    mid_frac: 0.3,
                    ..Default::default()
                })
                .collect();
        }
    }

    #[test]
    fn exposure_jumps_across_a_cut_are_found_except_onto_the_drop() -> anyhow::Result<()> {
        let mut index = tiny_index();
        let grey = [0.3, 0.3, 0.3];
        paint(&mut index, "A", 0.30, grey);
        paint(&mut index, "B", 0.05, grey);
        let mut plan = two_shots();
        let got = findings(&plan, &index);
        assert!(
            got.iter().any(|f| f.id == "exposure-jump"
                && f.seam == Some(1)
                && !f.is_hard()
                && f.message.contains("drops 2.6 stops")),
            "0.30 to 0.05 is 2.6 stops: {got:?}"
        );
        assert!(
            !got.iter().any(|f| f.id == "colour-jump"),
            "same grey: {got:?}"
        );
        paint(&mut index, "B", 0.15, grey);
        assert!(
            !findings(&plan, &index)
                .iter()
                .any(|f| f.id == "exposure-jump"),
            "one stop is within the render's reach"
        );
        // Cutting to black: both under the floor, nothing to see.
        paint(&mut index, "A", 0.01, grey);
        paint(&mut index, "B", 0.005, grey);
        assert!(
            !findings(&plan, &index)
                .iter()
                .any(|f| f.id == "exposure-jump"),
            "near-black noise is not a jump"
        );
        // The cut onto the drop may snap from dark to bright.
        paint(&mut index, "A", 0.05, grey);
        paint(&mut index, "B", 0.30, grey);
        plan.drop_bar = 1;
        assert!(
            !findings(&plan, &index)
                .iter()
                .any(|f| f.id == "exposure-jump"),
            "the drop is spared"
        );
        Ok(())
    }

    #[test]
    fn colour_jumps_across_a_cut_are_found() -> anyhow::Result<()> {
        let mut index = tiny_index();
        paint(&mut index, "A", 0.2, [0.6, 0.2, 0.2]);
        paint(&mut index, "B", 0.2, [0.1, 0.2, 0.7]);
        let plan = two_shots();
        let got = findings(&plan, &index);
        assert!(
            got.iter().any(|f| f.id == "colour-jump"
                && f.seam == Some(1)
                && f.shot == Some(2)
                && !f.is_hard()
                && f.message.contains("moves 0.50")),
            "red to blue mid-tones: {got:?}"
        );
        paint(&mut index, "B", 0.2, [0.5, 0.25, 0.25]);
        assert!(
            !findings(&plan, &index)
                .iter()
                .any(|f| f.id == "colour-jump"),
            "a warmer red is the same look"
        );
        // No mid-tones to judge (all black and emitters): no verdict.
        for c in index.clips.iter_mut() {
            for s in &mut c.samples {
                s.mid_frac = 0.0;
            }
        }
        assert!(
            !findings(&plan, &index)
                .iter()
                .any(|f| f.id == "colour-jump"),
            "nothing to judge colour on"
        );
        Ok(())
    }
}

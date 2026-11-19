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
    /// Clip colour signatures closer than this (0..1 distance) read as the same
    /// shot. Off (0) by default: one location gives every clip the same
    /// palette, and real repeats are caught by the clip and person rules.
    pub(crate) lookalike: f32,
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
            lookalike: 0.0,
            reject_flags: vec!["dark".into(), "whip".into(), "soft".into()],
            drop_subject: "fire".into(),
            reject_moments: vec!["transition".into(), "idle".into()],
            reject_blocked: true,
        }
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
    /// The cuts, in order.
    #[serde(rename = "shot")]
    pub(crate) shots: Vec<Shot>,
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
            if let (Some(ha), Some(hb)) = (shot_hist(index, shot), shot_hist(index, prev)) {
                let d = 0.5 * ha.iter().zip(&hb).map(|(x, y)| (x - y).abs()).sum::<f32>();
                if r.lookalike > 0.0 && d < r.lookalike && prev.clip != shot.clip {
                    push(
                        n,
                        format!("looks like shot {} (colour distance {d:.3})", j + 1),
                    );
                }
            }
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

/// Mean colour histogram of a shot's first second, from the index samples.
fn shot_hist(index: &Index, shot: &Shot) -> Option<Vec<f32>> {
    // Clip-level signature: the samples carry no histogram, and two shots
    // of one clip are already caught by the same-clip rule.
    index
        .clip(&shot.clip)
        .map(|c| c.hist.clone())
        .filter(|h| !h.is_empty())
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
        };
        let scored = |c: &str| Scored {
            id: format!("{c}/1"),
            clip: c.into(),
            seg: seg(0.0, 120.0),
            subject: "fire".into(),
            q: 0.9,
            flags: Vec::new(),
            caption: None,
        };
        Index {
            source: String::new(),
            sharp_ref: 1.0,
            clips: vec![clip("A", 0.0), clip("B", 1.0)],
            segments: vec![scored("A"), scored("B")],
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
            shots: vec![shot("A", 10.0), shot("B", 10.0), shot("A", 60.0)],
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
}

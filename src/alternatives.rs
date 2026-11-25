// ============================================================================
// Module:       alternatives
// Description:  Alternatives for a plan slot: the segments that could
//               replace a shot, ranked, each with the reason it fits and the
//               rules it keeps.
//
// Dependencies: crate::{draft, plan, index, beats, people}
// ============================================================================

//! Alternatives for a plan slot.
//!
//! No editor offers "show me the next-best footage for this slot"; a plan
//! can, because it knows the slot's length, its place in the song and the
//! rules. [`rank`] reuses the drafter's candidate windows (the same
//! flag / caption / crop filters, so an alternative never fails `check` on
//! its own account), keeps only those the plan's repeat rules allow next to
//! every *other* shot, and scores them for what the slot wants: the hero
//! fire on the drop, a calm opener, variety between fire bars. Each comes
//! with a one-line reason a person can read before swapping it in.

use crate::beats::MusicMap;
use crate::draft::{self, Candidate, Want};
use crate::index::Index;
use crate::people::People;
use crate::plan::{Plan, Shot};

/// One way to fill a slot.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Alternative {
    /// Source clip id.
    pub(crate) clip: String,
    /// The segment it comes from.
    pub(crate) seg: String,
    /// In-point, seconds.
    pub(crate) t_in: f32,
    /// Crop centre for the plan's aspect.
    pub(crate) cx: f32,
    /// Performer at the in-point (from the people map), may be empty.
    pub(crate) who: String,
    /// Ranking score (higher is better).
    pub(crate) score: f32,
    /// Why it fits, for a person.
    pub(crate) reason: String,
}

impl Alternative {
    /// The plan shot this alternative becomes in a slot of `beats` beats.
    pub(crate) fn to_shot(&self, beats: usize) -> Shot {
        Shot {
            clip: self.clip.clone(),
            t_in: (self.t_in * 100.0).round() / 100.0,
            beats,
            cx: (self.cx * 100.0).round() / 100.0,
            who: self.who.clone(),
            seg: self.seg.clone(),
            note: format!("swapped: {}", self.reason),
        }
    }
}

/// Beat offset (from the plan start) at which each shot begins.
fn starts(plan: &Plan) -> Vec<usize> {
    let mut beat = 0;
    plan.shots
        .iter()
        .map(|s| {
            let b = beat;
            beat += s.beats;
            b
        })
        .collect()
}

/// The subject of the segment that holds `t` in `clip`.
fn subject_at<'a>(index: &'a Index, clip: &str, t: f32) -> &'a str {
    index
        .segments
        .iter()
        .find(|g| g.clip == clip && g.seg.start <= t && t < g.seg.end)
        .map_or("", |g| g.subject.as_str())
}

/// What slot `k` of `plan` wants: the drafter's word at the start of its
/// note when there is one, otherwise read from where it sits in the song.
pub(crate) fn want_for(plan: &Plan, k: usize, index: &Index) -> Want {
    let Some(shot) = plan.shots.get(k) else {
        return Want::Other;
    };
    let head = shot
        .note
        .trim_start_matches("swapped:")
        .trim()
        .split([' ', '·'])
        .next()
        .unwrap_or_default()
        .to_lowercase();
    if let Some(w) = Want::ALL.into_iter().find(|w| w.label() == head) {
        return w;
    }
    let begin = starts(plan).get(k).copied().unwrap_or(0);
    let drop_beat = plan.drop_bar.saturating_sub(plan.start_bar) * 4;
    if begin == drop_beat {
        Want::Hero
    } else if k + 1 == plan.shots.len() {
        Want::Finale
    } else if begin < drop_beat {
        if k == 0 {
            Want::Hook
        } else if begin + 4 >= drop_beat {
            Want::Build
        } else {
            Want::Calm
        }
    } else if subject_at(index, &shot.clip, shot.t_in) == "fire" {
        Want::Fire
    } else {
        Want::Other
    }
}

/// The part of a reason that says why the slot likes this candidate.
fn fit_phrase(want: Want, c: &Candidate, fit: f32) -> String {
    if fit < 0.6 {
        return format!("weak match for a {} slot", want.label());
    }
    match want {
        Want::Hero => format!("strong fire for the drop (flame {:.2})", c.flame),
        Want::Finale => format!("fire to finish on (flame {:.2})", c.flame),
        Want::Hook => "a striking opener".to_owned(),
        Want::Calm => "calm, fits the breakdown".to_owned(),
        Want::Build => "energy for the build".to_owned(),
        Want::Fire => "fire on a loud bar".to_owned(),
        Want::Other => "variety between the fire".to_owned(),
    }
}

/// Shortens `text` to about `n` characters on a word boundary.
fn clip_text(text: &str, n: usize) -> String {
    if text.chars().count() <= n {
        return text.to_owned();
    }
    let cut: String = text.chars().take(n).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(a, _)| a);
    format!("{cut}…")
}

/// The best `limit` replacements for slot `k` (0-based) of `plan`, best
/// first. Every one fills the slot's length from footage free of rejected
/// flags and captions, keeps the plan's repeat rules against every other
/// shot, and meets the drop-subject rule when the slot starts on the drop.
pub(crate) fn rank(
    plan: &Plan,
    k: usize,
    index: &Index,
    map: &MusicMap,
    people: &People,
    limit: usize,
) -> Vec<Alternative> {
    let Some(current) = plan.shots.get(k) else {
        return Vec::new();
    };
    let timing = plan.timing(map);
    let Some(&(_, dur)) = timing.get(k) else {
        return Vec::new();
    };
    let rules = &plan.rules;
    let want = want_for(plan, k, index);
    let on_drop =
        starts(plan).get(k).copied() == Some(plan.drop_bar.saturating_sub(plan.start_bar) * 4);
    let placed: Vec<Option<Shot>> = plan
        .shots
        .iter()
        .enumerate()
        .map(|(j, s)| (j != k).then(|| s.clone()))
        .collect();
    let pool = draft::slot_candidates(index, rules, plan.aspect, dur);
    let current_kind = pool
        .iter()
        .find(|c| c.clip == current.clip && c.seg_id == current.seg)
        .map(|c| c.kind);
    let neighbours: Vec<&str> = [k.checked_sub(1), Some(k + 1)]
        .into_iter()
        .flatten()
        .filter_map(|j| plan.shots.get(j))
        .map(|s| s.clip.as_str())
        .collect();

    let mut out: Vec<Alternative> = pool
        .into_iter()
        .filter(|c| !(c.clip == current.clip && (c.t_in - current.t_in).abs() < 0.5))
        .filter(|c| draft::plan_allows(c, k, &placed, people, rules))
        .filter(|c| {
            !on_drop
                || rules.drop_subject.is_empty()
                || subject_at(index, &c.clip, c.t_in) == rules.drop_subject
        })
        .filter_map(|c| {
            let fit = draft::fit(want, &c, rules.drop_subject.is_empty());
            let mut score = c.score * fit;
            if current_kind == Some(c.kind) {
                score *= 1.1;
            }
            if neighbours.contains(&c.clip.as_str()) {
                score *= 0.8;
            }
            if score <= 0.0 {
                return None;
            }
            let summary = index
                .segments
                .iter()
                .find(|g| g.id == c.seg_id)
                .and_then(|g| g.caption.as_ref())
                .map(|cap| clip_text(&cap.summary, 70))
                .unwrap_or_default();
            let framing = if c.framing.is_empty() {
                String::new()
            } else {
                format!(" {}", c.framing)
            };
            let mut reason = format!(
                "{} · {}{framing} · q {:.2}",
                fit_phrase(want, &c, fit),
                c.kind,
                c.score
            );
            if !summary.is_empty() {
                reason.push_str(" · ");
                reason.push_str(&summary);
            }
            Some(Alternative {
                who: people.name_at(&c.clip, c.t_in),
                clip: c.clip,
                seg: c.seg_id,
                t_in: c.t_in,
                cx: c.cx,
                score,
                reason,
            })
        })
        .collect();
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::{rank, want_for};
    use crate::analyze::segments::{Camera, Segment};
    use crate::analyze::{ClipIndex, Sample};
    use crate::beats::{Bar, MusicMap};
    use crate::draft::Want;
    use crate::index::{Index, Scored};
    use crate::media::Probe;
    use crate::people::People;
    use crate::plan::{Aspect, Plan, Rules, Shot};

    fn seg(start: f32, end: f32) -> Segment {
        Segment {
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
            ..Segment::default()
        }
    }

    /// Clips A..D, 120 s each, one segment per 10 s; `C` is LED, the rest
    /// fire; `D/3` is flagged dark.
    fn index() -> Index {
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
        let mut clips = Vec::new();
        let mut segments = Vec::new();
        for (id, subject, flame) in [
            ("A", "fire", 0.2),
            ("B", "fire", 0.1),
            ("C", "led", 0.0),
            ("D", "fire", 0.05),
        ] {
            let samples = (0..240)
                .map(|i| Sample {
                    t: i as f32 * 0.5,
                    p95: 0.8,
                    sharp: 1.0,
                    lit: 0.6,
                    lit_frac: 0.05,
                    moving: 0.02,
                    cx: 0.5,
                    flame,
                    ..Sample::default()
                })
                .collect();
            clips.push(ClipIndex {
                id: id.into(),
                path: format!("{id}.mov"),
                stamp: (0, 0),
                probe: probe.clone(),
                samples,
                segments: Vec::new(),
                hist: Vec::new(),
                version: crate::analyze::ANALYSIS_VERSION,
            });
            for n in 0..12 {
                let t = n as f32 * 10.0;
                segments.push(Scored {
                    id: format!("{id}/{}", n + 1),
                    clip: id.into(),
                    seg: seg(t, t + 10.0),
                    subject: subject.into(),
                    q: 0.8,
                    flags: if id == "D" && n == 2 {
                        vec!["dark".into()]
                    } else {
                        Vec::new()
                    },
                    caption: None,
                    ..Default::default()
                });
            }
        }
        Index {
            source: String::new(),
            sharp_ref: 1.0,
            clips,
            segments,
            ..Default::default()
        }
    }

    fn map() -> MusicMap {
        MusicMap {
            path: "m.wav".into(),
            duration: 60.0,
            bpm: 120.0,
            beats: (0..120).map(|i| i as f32 * 0.5).collect(),
            bars: (0..30)
                .map(|i| Bar {
                    t: i as f32 * 2.0,
                    rms: 1.0,
                    low: 1.0,
                })
                .collect(),
            drops: Vec::new(),
            ..Default::default()
        }
    }

    fn shot(clip: &str, t: f32, note: &str) -> Shot {
        Shot {
            clip: clip.into(),
            t_in: t,
            beats: 4,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: note.into(),
        }
    }

    fn plan() -> Plan {
        Plan {
            name: "t".into(),
            aspect: Aspect::Wide,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 1,
            card_bar: 3,
            end_bar: 4,
            end_card: String::new(),
            rules: Rules {
                person_gap: 0,
                ..Rules::default()
            },
            shots: vec![
                shot("C", 5.0, "hook · led"),
                shot("A", 25.0, ""),
                shot("B", 45.0, "fire · fire"),
            ],
            finish: Default::default(),
            comments: Vec::new(),
        }
    }

    #[test]
    fn the_slot_kind_comes_from_the_note_or_the_song() -> anyhow::Result<()> {
        let (p, i) = (plan(), index());
        assert_eq!(want_for(&p, 0, &i), Want::Hook, "from the note");
        assert_eq!(want_for(&p, 1, &i), Want::Hero, "starts on the drop");
        assert_eq!(want_for(&p, 2, &i), Want::Fire, "from the note");
        Ok(())
    }

    #[test]
    fn alternatives_keep_the_rules_and_rank_for_the_slot() -> anyhow::Result<()> {
        let (p, i, m) = (plan(), index(), map());
        let alts = rank(&p, 1, &i, &m, &People::default(), 8);
        assert!(!alts.is_empty(), "something fills a 2 s slot");
        assert!(alts.len() <= 8, "capped at the limit");
        for a in &alts {
            assert_ne!(a.clip, "C", "the drop needs fire, not LED: {a:?}");
            assert!(
                !(a.clip == "B" && (a.t_in - 45.0).abs() < 30.0),
                "too close to shot 3 of the same clip: {a:?}"
            );
            assert!(a.seg != "D/3", "a dark segment is never offered: {a:?}");
            assert!(!a.reason.is_empty(), "every alternative says why");
        }
        assert!(
            alts.windows(2).all(|w| w[0].score >= w[1].score),
            "best first"
        );
        assert_eq!(
            alts.first().map(|a| a.clip.as_str()),
            Some("A"),
            "the strongest flame wins the drop: {alts:?}"
        );
        let swapped = alts
            .first()
            .map(|a| a.to_shot(4))
            .ok_or_else(|| anyhow::anyhow!("no alternative"))?;
        assert!(swapped.note.starts_with("swapped: "), "note says why");
        assert_eq!(swapped.beats, 4, "the slot keeps its length");
        Ok(())
    }
}

// ============================================================================
// Module:       draft
// Description:  Drafts an edit plan from the index and the music map: lays out
//               a beat grid (breakdown, build, drop, phrases, finale, card) and
//               fills each slot with the best footage the rules allow.
//
// Dependencies: crate::index, crate::beats, crate::plan, crate::people
// ============================================================================

//! The automatic first cut.
//!
//! The grid follows the promo structure the JaJa runbook describes:
//!
//! - the edit starts `lead` bars before the drop, in the breakdown: one
//!   shot per bar, calm subjects (LED, silks, DJ, venue), zooms welcome;
//! - the bar before the drop builds: 2 + 1 + 1 beats;
//! - the drop lands on the hero shot (the strongest fire);
//! - loud bars cut every 2 beats, the last bar of each phrase stutters
//!   (1 + 1 + 2); quiet bars hold a shot for the whole bar;
//! - the last bar before the end card is a fire finale.
//!
//! Slots are filled most-important first (drop, finale, opening, then in
//! order). A candidate is a window of a clip: it must lie inside segments
//! free of rejected flags and captions, and it must respect every rule in
//! [`Rules`] against the shots already placed. The draft is a starting
//! point: `ave check` re-verifies it and a person or LLM can edit the TOML.

use anyhow::{Result, bail};

use crate::analyze::Sample;
use crate::analyze::segments::{Camera, median};
use crate::beats::MusicMap;
use crate::index::{Index, Scored};
use crate::people::People;
use crate::plan::{Aspect, Plan, Rules, Shot};

/// What a slot wants to show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Want {
    /// The first shot: a performer, calm but striking.
    Hook,
    /// The breakdown: calm, non-fire if possible.
    Calm,
    /// The build bar: anything with energy, zoom-ins welcome.
    Build,
    /// The drop.
    Hero,
    /// Fire.
    Fire,
    /// Anything that is not fire (LED, silks, DJ) for variety.
    Other,
    /// The finale.
    Finale,
}

impl Want {
    /// Every slot kind.
    pub(crate) const ALL: [Self; 7] = [
        Self::Hook,
        Self::Calm,
        Self::Build,
        Self::Hero,
        Self::Fire,
        Self::Other,
        Self::Finale,
    ];

    /// The lowercase word a draft note starts with (`hero · fire close …`).
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Hook => "hook",
            Self::Calm => "calm",
            Self::Build => "build",
            Self::Hero => "hero",
            Self::Fire => "fire",
            Self::Other => "other",
            Self::Finale => "finale",
        }
    }
}

/// One slot of the grid.
#[derive(Clone, Copy, Debug)]
struct Slot {
    beats: usize,
    want: Want,
    /// Order in which slots are filled (lower first).
    priority: usize,
}

/// How a draft is shaped.
#[derive(Clone, Debug)]
pub(crate) struct Spec {
    /// Output name.
    pub(crate) name: String,
    /// Frame shape.
    pub(crate) aspect: Aspect,
    /// Target length in seconds (the result lands on whole phrases).
    pub(crate) seconds: f32,
    /// End card image.
    pub(crate) end_card: String,
    /// Bars the end card holds.
    pub(crate) card_bars: usize,
}

/// What a segment shows, combining the index subject and the caption.
fn kind(seg: &Scored, caption: Option<&crate::caption::Caption>) -> &'static str {
    if let Some(c) = caption {
        let props = c
            .people
            .iter()
            .map(|p| p.prop.to_lowercase())
            .collect::<Vec<_>>()
            .join(" ");
        let summary = c.summary.to_lowercase();
        if matches!(c.effective_moment(), "venue" | "crowd") {
            return "venue";
        }
        if props.contains("silk") || summary.contains("aerial") || summary.contains("silks") {
            return "silks";
        }
        if props.contains("dj")
            || props.contains("deck")
            || props.contains("turntable")
            || summary.contains(" dj")
        {
            return "dj";
        }
    }
    match seg.subject.as_str() {
        "fire" => "fire",
        "led" => "led",
        _ => "other",
    }
}

/// Quality of one analysis sample, on the same scale as segment `q`.
fn sample_q(s: &Sample, sharp_ref: f32) -> f32 {
    let expo = if s.p95 < 0.5 {
        (s.p95 / 0.5).powf(1.5)
    } else if s.clipped > 0.06 {
        (1.0 - (s.clipped - 0.06) * 4.0).max(0.2)
    } else {
        1.0
    };
    let focus = (s.sharp / sharp_ref).clamp(0.0, 1.4) / 1.4;
    let action = (s.moving / 0.01).sqrt().clamp(0.0, 1.0);
    expo * (0.45 + 0.55 * focus) * (0.35 + 0.65 * action)
}

/// Steadiness weight of a camera behaviour.
fn steady(c: Camera) -> f32 {
    match c {
        Camera::Static | Camera::ZoomIn | Camera::ZoomOut => 1.0,
        Camera::Pan => 0.85,
        Camera::Unknown => 0.7,
        Camera::Shaky => 0.35,
        Camera::Whip => 0.0,
    }
}

/// A placeable window.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    /// Source clip id.
    pub(crate) clip: String,
    /// The segment the window starts in.
    pub(crate) seg_id: String,
    /// In-point, seconds.
    pub(crate) t_in: f32,
    /// Crop centre for the aspect.
    pub(crate) cx: f32,
    /// Quality × steadiness × caption moment (slot-independent).
    pub(crate) score: f32,
    /// What it shows: fire, led, silks, dj, venue, other.
    pub(crate) kind: &'static str,
    /// Caption framing (wide, medium, close), may be empty.
    pub(crate) framing: String,
    /// Mean flame share over the window.
    pub(crate) flame: f32,
}

/// The grid of slots for a short edit.
fn grid(map: &MusicMap, lead: usize, after: usize, drop_bar: usize) -> Vec<Slot> {
    let mut slots = Vec::new();
    for b in 0..lead {
        if b + 1 == lead {
            for beats in [2, 1, 1] {
                slots.push(Slot {
                    beats,
                    want: Want::Build,
                    priority: 50,
                });
            }
        } else {
            slots.push(Slot {
                beats: 4,
                want: if b == 0 { Want::Hook } else { Want::Calm },
                priority: if b == 0 { 2 } else { 40 },
            });
        }
    }
    for k in 0..after {
        let bar = drop_bar + k;
        let loud = map.bars.get(bar).is_none_or(|b| b.low >= 0.35);
        let phrase = k / 4;
        let last_in_phrase = k % 4 == 3;
        if k == 0 {
            slots.push(Slot {
                beats: 4,
                want: Want::Hero,
                priority: 0,
            });
        } else if k + 1 == after {
            slots.push(Slot {
                beats: 4,
                want: Want::Finale,
                priority: 1,
            });
        } else if !loud {
            slots.push(Slot {
                beats: 4,
                want: Want::Other,
                priority: 30,
            });
        } else {
            let want = if phrase % 2 == 1 && k % 4 < 2 {
                Want::Other
            } else {
                Want::Fire
            };
            let pattern: &[usize] = if last_in_phrase { &[1, 1, 2] } else { &[2, 2] };
            for &beats in pattern {
                slots.push(Slot {
                    beats,
                    want,
                    priority: 60 + k,
                });
            }
        }
    }
    slots
}

/// How well a candidate suits a slot (multiplier, 0 = not at all).
pub(crate) fn fit(want: Want, c: &Candidate) -> f32 {
    let fire = c.kind == "fire";
    match want {
        Want::Hook => match c.kind {
            "silks" => 1.6,
            "led" => 1.4,
            "dj" => 0.8,
            "fire" => 0.4,
            "venue" => 0.1,
            _ => 0.5,
        },
        Want::Calm => match c.kind {
            "led" | "silks" => 1.3,
            "dj" => 1.0,
            "venue" => 0.6,
            "fire" => 0.25,
            _ => 0.6,
        },
        Want::Build => {
            if fire {
                0.9
            } else {
                1.0
            }
        }
        Want::Hero => {
            if fire {
                1.0 + 20.0 * c.flame
            } else {
                0.0
            }
        }
        Want::Fire => {
            if fire {
                1.0
            } else {
                0.15
            }
        }
        Want::Other => match c.kind {
            "led" | "silks" | "dj" => 1.3,
            "fire" => 0.5,
            "venue" => 0.2,
            _ => 0.4,
        },
        Want::Finale => {
            if fire {
                1.0 + 10.0 * c.flame
            } else {
                0.0
            }
        }
    }
}

/// All windows of `dur` seconds that avoid rejected segments, one per
/// start segment, scored.
pub(crate) fn candidates(index: &Index, rules: &Rules, aspect: Aspect, dur: f32) -> Vec<Candidate> {
    let crop = aspect.crop_frac();
    let mut out = Vec::new();
    for clip in &index.clips {
        let segs: Vec<&Scored> = index
            .segments
            .iter()
            .filter(|g| g.clip == clip.id)
            .collect();
        // A segment's caption, or its nearest captioned neighbour's when
        // it has none (short segments are not captioned).
        let caption_of = |k: usize| -> Option<&crate::caption::Caption> {
            segs[k].caption.as_ref().or_else(|| {
                (1..segs.len()).find_map(|d| {
                    let before = k.checked_sub(d).and_then(|p| segs[p].caption.as_ref());
                    before.or_else(|| segs.get(k + d).and_then(|n| n.caption.as_ref()))
                })
            })
        };
        let bad = |k: usize| {
            let g = segs[k];
            g.flags.iter().any(|f| rules.reject_flags.contains(f))
                || matches!(g.seg.camera, Camera::Whip)
                || g.caption.as_ref().is_some_and(|c| {
                    rules
                        .reject_moments
                        .contains(&c.effective_moment().to_owned())
                        || (rules.reject_blocked && c.blocked)
                })
        };
        for (i, g) in segs.iter().enumerate() {
            if bad(i) {
                continue;
            }
            // Extend through following good segments if this one is short.
            let mut end = g.seg.end;
            let mut j = i;
            while end - g.seg.start < dur + 0.2 && j + 1 < segs.len() && !bad(j + 1) {
                j += 1;
                end = segs[j].seg.end;
            }
            if end - g.seg.start < dur + 0.1 {
                continue;
            }
            // Centre the window on the segment, inside the good run.
            let mid = 0.5 * (g.seg.start + g.seg.end);
            let t_in = (mid - dur / 2.0).clamp(g.seg.start + 0.05, end - dur - 0.05);
            let t_out = t_in + dur;
            let spanned: Vec<usize> = (i..=j)
                .filter(|&k| segs[k].seg.end > t_in + 0.05 && segs[k].seg.start < t_out - 0.05)
                .collect();
            // The same crop limit `check` applies, so a draft never fails it.
            if crop < 1.0
                && spanned
                    .iter()
                    .any(|&k| segs[k].subject != "none" && segs[k].seg.cx_spread > crop * 1.2)
            {
                continue;
            }
            let captions: Vec<&crate::caption::Caption> =
                spanned.iter().filter_map(|&k| caption_of(k)).collect();
            if captions.iter().any(|c| {
                rules
                    .reject_moments
                    .contains(&c.effective_moment().to_owned())
                    || (rules.reject_blocked && c.blocked)
            }) {
                continue;
            }
            let window: Vec<&Sample> = clip
                .samples
                .iter()
                .filter(|s| s.t >= t_in && s.t < t_out)
                .collect();
            if window.is_empty() {
                continue;
            }
            let q = window
                .iter()
                .map(|s| sample_q(s, index.sharp_ref))
                .sum::<f32>()
                / window.len() as f32;
            let cam = spanned
                .iter()
                .map(|&k| steady(segs[k].seg.camera))
                .fold(1.0_f32, f32::min);
            let cxs: Vec<f32> = window.iter().map(|s| s.cx).collect();
            let half = crop / 2.0;
            let cx = if crop < 1.0 {
                median(&cxs).clamp(half, 1.0 - half)
            } else {
                0.5
            };
            let main = caption_of(i);
            let what = kind(g, main);
            // The weakest caption over the window decides.
            let moment = captions
                .iter()
                .map(|c| match c.effective_moment() {
                    "performing" if c.props_active => 1.2,
                    "performing" => 1.0,
                    "venue" | "crowd" => 1.0,
                    _ => 0.5,
                } * if !c.props_active && (what == "fire" || what == "led") { 0.4 } else { 1.0 })
                .fold(1.2_f32, f32::min);
            let issues = main.map_or(0, |c| c.issues.len()) as f32;
            out.push(Candidate {
                clip: clip.id.clone(),
                seg_id: g.id.clone(),
                t_in,
                cx,
                score: q * cam * moment * (1.0 - 0.08 * issues.min(4.0)),
                kind: what,
                framing: main.map(|c| c.framing.clone()).unwrap_or_default(),
                flame: window.iter().map(|s| s.flame).sum::<f32>() / window.len() as f32,
            });
        }
    }
    out
}

/// Whether `c` may go in slot `k` given what is placed already.
pub(crate) fn allowed(
    c: &Candidate,
    k: usize,
    placed: &[Option<Shot>],
    people: &People,
    rules: &Rules,
) -> bool {
    let who = people.name_at(&c.clip, c.t_in);
    placed.iter().enumerate().all(|(j, p)| {
        let Some(p) = p else { return true };
        if p.clip == c.clip && (p.t_in - c.t_in).abs() < rules.same_clip_gap {
            return false;
        }
        !(people.who(p) == who && k.abs_diff(j) <= rules.person_gap)
    })
}

/// Drafts a plan for `spec`.
pub(crate) fn draft(
    index: &Index,
    map: &MusicMap,
    people: &People,
    spec: &Spec,
    music: &str,
) -> Result<Plan> {
    let Some(drop) = map.drops.first() else {
        bail!("the music map has no drop; pick bars by hand in the plan file");
    };
    let bar_len = 240.0 / map.bpm;
    let lead = if spec.seconds <= 20.0 { 2 } else { 4 };
    // Whole phrases after the drop, as many as fit the target length.
    let mut after = 4;
    while (lead + after + 4 + spec.card_bars) as f32 * bar_len <= spec.seconds + 1.0
        && spec.seconds > 20.0
        && drop.bar + after + 4 + spec.card_bars <= map.bars.len()
    {
        after += 4;
    }
    let start_bar = drop.bar.saturating_sub(lead);
    let lead = drop.bar - start_bar;
    let card_bar = drop.bar + after;
    let rules = Rules::default();
    let slots = grid(map, lead, after, drop.bar);

    let mut placed: Vec<Option<Shot>> = vec![None; slots.len()];
    let mut order: Vec<usize> = (0..slots.len()).collect();
    order.sort_by_key(|&k| (slots[k].priority, k));
    let beat = 60.0 / map.bpm;
    let mut pools: std::collections::BTreeMap<usize, Vec<Candidate>> =
        std::collections::BTreeMap::new();
    for &k in &order {
        let slot = slots[k];
        let dur = slot.beats as f32 * beat;
        let pool = pools
            .entry(slot.beats)
            .or_insert_with(|| candidates(index, &rules, spec.aspect, dur));
        let prev_framing = k
            .checked_sub(1)
            .and_then(|j| placed[j].as_ref())
            .map(|s| s.note.clone());
        let best = pool
            .iter()
            .filter(|c| allowed(c, k, &placed, people, &rules))
            .map(|c| {
                let mut s = c.score * fit(slot.want, c);
                if prev_framing
                    .as_deref()
                    .is_some_and(|n| !c.framing.is_empty() && n.contains(&c.framing))
                {
                    s *= 0.85;
                }
                (s, c)
            })
            .filter(|(s, _)| *s > 0.0)
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let Some((score, c)) = best else {
            bail!(
                "ran out of distinct footage for slot {} ({:?}, {} beats); shorten the edit or loosen the rules",
                k + 1,
                slot.want,
                slot.beats
            );
        };
        placed[k] = Some(Shot {
            clip: c.clip.clone(),
            t_in: (c.t_in * 100.0).round() / 100.0,
            beats: slot.beats,
            cx: (c.cx * 100.0).round() / 100.0,
            who: people.name_at(&c.clip, c.t_in),
            seg: c.seg_id.clone(),
            note: format!(
                "{:?} · {} {} · score {score:.2}",
                slot.want, c.kind, c.framing
            )
            .to_lowercase(),
        });
    }
    Ok(Plan {
        name: spec.name.clone(),
        aspect: spec.aspect,
        music: music.to_owned(),
        start_bar,
        drop_bar: drop.bar,
        card_bar,
        end_bar: card_bar + spec.card_bars,
        end_card: spec.end_card.clone(),
        rules,
        shots: placed.into_iter().flatten().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::{Want, grid};
    use crate::beats::{Bar, MusicMap};

    #[test]
    fn the_grid_fills_whole_bars() -> anyhow::Result<()> {
        let map = MusicMap {
            path: String::new(),
            duration: 100.0,
            bpm: 140.0,
            beats: (0..200).map(|i| i as f32 * 60.0 / 140.0).collect(),
            bars: (0..50)
                .map(|i| Bar {
                    t: i as f32,
                    rms: 1.0,
                    low: 1.0,
                })
                .collect(),
            drops: Vec::new(),
        };
        let slots = grid(&map, 4, 12, 16);
        let beats: usize = slots.iter().map(|s| s.beats).sum();
        assert_eq!(beats, 16 * 4, "4 lead + 12 after bars");
        assert_eq!(
            slots.iter().filter(|s| s.want == Want::Hero).count(),
            1,
            "one hero"
        );
        assert!(
            slots.last().is_some_and(|s| s.want == Want::Finale),
            "ends on the finale"
        );
        Ok(())
    }
}

// ============================================================================
// Module:       draft
// Description:  Drafts an edit plan from the index and the music map: lays out
//               a beat grid (breakdown, build, drop, energy-paced phrases,
//               finale, card) and fills each slot with the best footage the
//               rules allow.
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
//! - the drop lands on the hero shot (the strongest fire), with its biggest
//!   burst (see [`crate::analyze::moments`]) put on the downbeat: the
//!   in-point is the burst's onset, so the flare lands on the cut;
//! - after the drop, shot lengths follow the music's per-bar energy (see
//!   [`pace`]): quiet bars hold a shot for the bar (two quiet bars may share
//!   one 8-beat shot), loud bars cut every 2 beats, the loudest stutter, the
//!   last bar of a phrase prefers the 1 + 1 + 2 stutter, and a pattern that
//!   repeats bar after bar costs more each time, so lengths never go uniform;
//! - the last bar before the end card is a fire finale.
//!
//! Slots are filled most-important first (drop, finale, opening, then in
//! order). A candidate is a window of a clip centred on its segment's best
//! moment (`peak_t`): it must lie inside segments free of rejected flags and
//! captions, shifts clear of any bad span with a rejected flag inside them
//! (see [`crate::analyze::spans`]; the slot fixes its length), must not reuse a moment another night's plan used (see
//! [`Memory`]), must not look like a placed shot, and must respect every
//! rule in [`Rules`] against the shots already placed. If the footage runs
//! out of distinct shots the pacing eases (fewer cuts per bar) and the draft
//! is tried again. A window whose out-point lands just after a crowd cheer
//! (see [`crate::analyze::cheer`]) is also tried, and scores up to a
//! quarter more: the shot shows the trick and cuts on the reaction.
//! The draft is a starting point: `ave check` re-verifies it
//! and a person or LLM can edit the TOML.

use anyhow::{Result, bail};

use crate::analyze::Sample;
use crate::analyze::segments::{Camera, median};
use crate::beats::MusicMap;
use crate::index::{Index, Scored};
use crate::people::People;
use crate::plan::{Aspect, Finish, Memory, Plan, Rules, Shot};

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
    let expo = crate::index::exposure_q(s.lit, s.lit_frac, s.clipped);
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
    /// Burst at the in-point (aligned windows) or the biggest inside.
    pub(crate) burst: f32,
    /// Whether the in-point is a burst onset (the flare lands on the cut).
    pub(crate) aligned: bool,
    /// Strength of a crowd cheer whose onset falls just before the
    /// out-point (the shot ends on the reaction), 0 when none.
    pub(crate) cheer: f32,
}

/// Where a cheer onset should sit before a shot's out-point, seconds: the
/// trick and the first of the reaction are in, the cut follows.
const CHEER_LEAD: (f32, f32) = (0.15, 0.9);

/// A bar pattern after the drop: how a bar (or two) is cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pattern {
    /// One 8-beat shot over two quiet bars.
    Hold8,
    /// One shot for the bar.
    Hold,
    /// 2 + 2.
    Even,
    /// 1 + 1 + 2: the phrase-end stutter.
    StutterEnd,
    /// 2 + 1 + 1.
    StutterStart,
}

impl Pattern {
    /// Every pattern.
    const ALL: [Self; 5] = [
        Self::Hold8,
        Self::Hold,
        Self::Even,
        Self::StutterEnd,
        Self::StutterStart,
    ];

    /// Shot lengths in beats.
    fn beats(self) -> &'static [usize] {
        match self {
            Self::Hold8 => &[8],
            Self::Hold => &[4],
            Self::Even => &[2, 2],
            Self::StutterEnd => &[1, 1, 2],
            Self::StutterStart => &[2, 1, 1],
        }
    }

    /// Bars the pattern covers.
    fn bars(self) -> usize {
        match self {
            Self::Hold8 => 2,
            Self::Hold | Self::Even | Self::StutterEnd | Self::StutterStart => 1,
        }
    }

    /// Cuts per bar.
    fn rate(self) -> f32 {
        self.beats().len() as f32 / self.bars() as f32
    }
}

/// Energy of bar `bar` (low end relative to the loudest bar; loud when
/// unknown).
fn energy(map: &MusicMap, bar: usize) -> f32 {
    map.bars.get(bar).map_or(1.0, |b| b.low)
}

/// Cuts per bar the music asks for at energy `e`: one for quiet bars,
/// rising to 2.5 for the loudest; `ease` takes half a cut off per step.
fn target_rate(e: f32, ease: usize) -> f32 {
    let t = 1.0 + 1.5 * ((e - 0.3) / 0.5).clamp(0.0, 1.0);
    (t - 0.5 * ease as f32).max(0.5)
}

/// Cost of the same pattern `run` bars in a row (uniform lengths read as
/// mechanical; each repeat costs more).
const UNIFORM: f32 = 0.12;

/// Energy-adaptive pacing (after BEAT's bar DP): picks a pattern for every
/// bar `first..first + n` of the track so the cut rate follows the bar's
/// energy, stutters fall on phrase ends (`k % 4 == 3`, `k` counted from
/// the drop), an 8-beat hold never crosses a phrase line, and a pattern
/// repeated bar after bar pays [`UNIFORM`] times its run length.
fn pace(map: &MusicMap, first: usize, k0: usize, n: usize, ease: usize) -> Vec<Pattern> {
    // Bar cost of pattern `p` starting at offset `i`.
    let cost = |p: Pattern, i: usize| -> Option<f32> {
        let k = k0 + i;
        if i + p.bars() > n {
            return None;
        }
        let phrase_end = k % 4 == 3;
        let mut c = 0.0;
        for b in 0..p.bars() {
            let e = energy(map, first + i + b);
            c += (p.rate() - target_rate(e, ease)).powi(2);
        }
        match p {
            Pattern::Hold8 => {
                // Both bars quiet, and inside one phrase.
                if phrase_end || (0..2).any(|b| energy(map, first + i + b) >= 0.35) {
                    return None;
                }
            }
            Pattern::StutterEnd => {
                c += if phrase_end { -0.3 } else { 0.25 };
            }
            Pattern::StutterStart => c += 0.25,
            Pattern::Hold | Pattern::Even => {}
        }
        Some(c)
    };
    // best[i][(p, run)]: cheapest way to cover bars i.. given the pattern
    // ending just before i was p, repeated `run` times (run capped at 3).
    let idx = |p: Pattern| Pattern::ALL.iter().position(|&q| q == p).unwrap_or(0);
    let states = Pattern::ALL.len() * 4 + 1; // +1: nothing before
    let mut best = vec![vec![f32::INFINITY; states]; n];
    best.push(vec![0.0; states]);
    let mut choice: Vec<Vec<Option<Pattern>>> = vec![vec![None; states]; n + 1];
    for i in (0..n).rev() {
        for s in 0..states {
            let (prev, run) = if s + 1 == states {
                (None, 0)
            } else {
                (Some(Pattern::ALL[s / 4]), s % 4)
            };
            for p in Pattern::ALL {
                let Some(c) = cost(p, i) else { continue };
                let (next_run, penalty) = if prev == Some(p) {
                    ((run + 1).min(3), UNIFORM * (run + 1) as f32)
                } else {
                    (1, 0.0)
                };
                let next = idx(p) * 4 + next_run;
                let total = c + penalty + best[i + p.bars()][next];
                if total < best[i][s] {
                    best[i][s] = total;
                    choice[i][s] = Some(p);
                }
            }
        }
    }
    let mut out = Vec::new();
    let (mut i, mut s) = (0, states - 1);
    while i < n {
        let Some(p) = choice[i][s] else { break };
        let run = if s + 1 != states && Pattern::ALL[s / 4] == p {
            (s % 4 + 1).min(3)
        } else {
            1
        };
        out.push(p);
        s = idx(p) * 4 + run;
        i += p.bars();
    }
    out
}

/// The grid of slots for a short edit.
fn grid(map: &MusicMap, lead: usize, after: usize, drop_bar: usize, ease: usize) -> Vec<Slot> {
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
    slots.push(Slot {
        beats: 4,
        want: Want::Hero,
        priority: 0,
    });
    let middle = after.saturating_sub(2);
    let mut k = 1;
    for p in pace(map, drop_bar + 1, 1, middle, ease) {
        let phrase = k / 4;
        let quiet = matches!(p, Pattern::Hold8 | Pattern::Hold);
        let want = if quiet || (phrase % 2 == 1 && k % 4 < 2) {
            Want::Other
        } else {
            Want::Fire
        };
        for &beats in p.beats() {
            slots.push(Slot {
                beats,
                want,
                priority: if quiet { 30 } else { 60 + k },
            });
        }
        k += p.bars();
    }
    if after >= 2 {
        slots.push(Slot {
            beats: 4,
            want: Want::Finale,
            priority: 1,
        });
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
                // The biggest burst landing on the downbeat wins.
                let on_cut = if c.aligned { 1.0 + 40.0 * c.burst } else { 1.0 };
                (1.0 + 20.0 * c.flame) * on_cut
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

/// All windows of `dur` seconds that avoid rejected segments, scored: one
/// per start segment centred on its best moment, plus (when `align`) one
/// starting on each burst onset, so a flare can land on the cut.
fn candidates(
    index: &Index,
    rules: &Rules,
    aspect: Aspect,
    dur: f32,
    align: bool,
    memory: &Memory,
) -> Vec<Candidate> {
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
            // Extend through the following good segments, so a window may
            // run on past this segment's end (a moment near its end can
            // still start a shot).
            let mut end = g.seg.end;
            let mut j = i;
            while j + 1 < segs.len() && !bad(j + 1) && end < g.seg.end + dur + 0.2 {
                j += 1;
                end = segs[j].seg.end;
            }
            // The last segment ends a sample period after its last sample,
            // which can be past the end of the file.
            let end = end.min(clip.probe.duration as f32 - 0.02);
            if end - g.seg.start < dur + 0.1 {
                continue;
            }
            let lo = g.seg.start + 0.05;
            let hi = end - dur - 0.05;
            // Centre the window on the segment's best moment.
            let peak = if g.seg.peak_t > g.seg.start && g.seg.peak_t < g.seg.end {
                g.seg.peak_t
            } else {
                0.5 * (g.seg.start + g.seg.end)
            };
            // Bad spans of a rejected flag in the run: the centred window
            // shifts clear of them, a burst-aligned one is dropped (moving
            // it would lose the burst on the cut).
            let spans: Vec<&crate::analyze::spans::BadSpan> = segs[i..=j]
                .iter()
                .flat_map(|s| &s.seg.bad_spans)
                .filter(|b| rules.reject_flags.contains(&b.flag))
                .collect();
            let centred = (peak - dur / 2.0).clamp(lo, hi);
            let mut starts = Vec::new();
            if let Some(t) = crate::analyze::spans::dodge(&spans, centred, dur, lo, hi) {
                starts.push((t, false));
            }
            if align {
                for &m in &g.seg.moments {
                    if m >= lo && m <= hi && spans.iter().all(|b| !b.overlaps(m, m + dur)) {
                        starts.push((m, true));
                    }
                }
            }
            // A window that ends just after each cheer onset.
            for c in &g.seg.cheers {
                let t = c.t + 0.5 * (CHEER_LEAD.0 + CHEER_LEAD.1) - dur;
                if t >= lo && t <= hi {
                    starts.push((t, false));
                }
            }
            for (t_in, aligned) in starts {
                if memory.hit(index, &clip.id, t_in, t_in + dur).is_some() {
                    continue;
                }
                if let Some(c) = window(
                    index,
                    rules,
                    clip,
                    &segs,
                    (i, j),
                    t_in,
                    dur,
                    crop,
                    aligned,
                    &caption_of,
                ) {
                    out.push(c);
                }
            }
        }
    }
    out
}

/// Scores the window `t_in..t_in + dur` of `clip`, which starts in segment
/// `run.0` and stays inside the good run `run.0..=run.1`.
#[expect(
    clippy::too_many_arguments,
    reason = "the candidate loop's context; a struct would only rename it"
)]
fn window<'c>(
    index: &Index,
    rules: &Rules,
    clip: &crate::analyze::ClipIndex,
    segs: &[&Scored],
    run: (usize, usize),
    t_in: f32,
    dur: f32,
    crop: f32,
    aligned: bool,
    caption_of: &dyn Fn(usize) -> Option<&'c crate::caption::Caption>,
) -> Option<Candidate> {
    let (i, j) = run;
    let g = segs[i];
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
        return None;
    }
    let captions: Vec<&crate::caption::Caption> =
        spanned.iter().filter_map(|&k| caption_of(k)).collect();
    if captions.iter().any(|c| {
        rules
            .reject_moments
            .contains(&c.effective_moment().to_owned())
            || (rules.reject_blocked && c.blocked)
    }) {
        return None;
    }
    let window: Vec<&Sample> = clip
        .samples
        .iter()
        .filter(|s| s.t >= t_in && s.t < t_out)
        .collect();
    if window.is_empty() {
        return None;
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
        // Rounded to the plan's two decimals, inward, so rounding never
        // pushes the crop out of the frame.
        ((median(&cxs) * 100.0).round() / 100.0).clamp(
            (half * 100.0).ceil() / 100.0,
            ((1.0 - half) * 100.0).floor() / 100.0,
        )
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
    // Burst at the cut for aligned windows (the first sample at or after
    // the onset carries the rise), else the biggest inside.
    let burst = if aligned {
        window.iter().take(2).map(|s| s.burst).fold(0.0, f32::max)
    } else {
        window.iter().map(|s| s.burst).fold(0.0, f32::max)
    };
    // A cheer just before the out-point: the shot ends on the reaction.
    let cheer = spanned
        .iter()
        .flat_map(|&k| &segs[k].seg.cheers)
        .filter(|c| c.t >= t_out - CHEER_LEAD.1 && c.t <= t_out - CHEER_LEAD.0)
        .map(|c| c.strength)
        .fold(0.0_f32, f32::max);
    Some(Candidate {
        clip: clip.id.clone(),
        seg_id: g.id.clone(),
        t_in,
        cx,
        score: q * cam * moment * (1.0 - 0.08 * issues.min(4.0)) * (1.0 + 0.25 * cheer),
        kind: what,
        framing: main.map(|c| c.framing.clone()).unwrap_or_default(),
        flame: window.iter().map(|s| s.flame).sum::<f32>() / window.len() as f32,
        burst,
        aligned,
        cheer,
    })
}

/// A candidate as the shot it would become in a slot of `beats`.
fn as_shot(c: &Candidate, beats: usize, people: &People) -> Shot {
    Shot {
        clip: c.clip.clone(),
        t_in: (c.t_in * 100.0).round() / 100.0,
        beats,
        cx: (c.cx * 100.0).round() / 100.0,
        who: people.name_at(&c.clip, c.t_in),
        seg: c.seg_id.clone(),
        note: String::new(),
    }
}

/// Whether `c` may go in slot `k` given what is placed already.
fn allowed(
    c: &Candidate,
    k: usize,
    slots: &[Slot],
    placed: &[Option<Shot>],
    ctx: (&Index, &People, &Rules, f32),
) -> bool {
    let (index, people, rules, beat) = ctx;
    let me = as_shot(c, slots[k].beats, people);
    let who = people.who(&me);
    let dur = slots[k].beats as f32 * beat;
    placed.iter().enumerate().all(|(j, p)| {
        let Some(p) = p else { return true };
        if p.clip == c.clip && (p.t_in - c.t_in).abs() < rules.same_clip_gap {
            return false;
        }
        if people.who(p) == who && k.abs_diff(j) <= rules.person_gap {
            return false;
        }
        let pdur = slots[j].beats as f32 * beat;
        !(rules.lookalike > 0.0
            && p.clip != c.clip
            && crate::plan::lookalike(index, people, (&me, dur), (p, pdur))
                .is_some_and(|d| d < rules.lookalike))
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
    let memory = Memory::for_index(index);
    let mut last = None;
    for ease in 0..3 {
        match draft_paced(index, map, people, spec, music, &memory, ease) {
            Ok(plan) => {
                if ease > 0 {
                    log::info!("eased the cutting pace {ease} step(s) to find distinct footage");
                }
                return Ok(plan);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no draft")))
}

/// One drafting attempt at pacing ease `ease`.
fn draft_paced(
    index: &Index,
    map: &MusicMap,
    people: &People,
    spec: &Spec,
    music: &str,
    memory: &Memory,
    ease: usize,
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
    let slots = grid(map, lead, after, drop.bar, ease);

    let mut placed: Vec<Option<Shot>> = vec![None; slots.len()];
    let mut order: Vec<usize> = (0..slots.len()).collect();
    order.sort_by_key(|&k| (slots[k].priority, k));
    let beat = 60.0 / map.bpm;
    let mut pools: std::collections::BTreeMap<(usize, bool), Vec<Candidate>> =
        std::collections::BTreeMap::new();
    for &k in &order {
        let slot = slots[k];
        let dur = slot.beats as f32 * beat;
        let align = slot.want == Want::Hero;
        let pool = pools
            .entry((slot.beats, align))
            .or_insert_with(|| candidates(index, &rules, spec.aspect, dur, align, memory));
        let prev_framing = k
            .checked_sub(1)
            .and_then(|j| placed[j].as_ref())
            .map(|s| s.note.clone());
        let best = pool
            .iter()
            .filter(|c| allowed(c, k, &slots, &placed, (index, people, &rules, beat)))
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
        let mut shot = as_shot(c, slot.beats, people);
        let mut accent = if c.aligned {
            format!(" · burst {:.1}% on the cut", c.burst * 100.0)
        } else {
            String::new()
        };
        if c.cheer > 0.0 {
            accent.push_str(&format!(" · ends on a cheer {:.2}", c.cheer));
        }
        shot.note = format!(
            "{:?} · {} {} · score {score:.2}{accent}",
            slot.want, c.kind, c.framing
        )
        .to_lowercase();
        placed[k] = Some(shot);
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
        finish: Finish::default(),
        shots: placed.into_iter().flatten().collect(),
        comments: Vec::new(),
    })
}

/// Candidate windows of `dur` seconds for one slot, as the timeline's
/// alternatives strip needs them: burst-aligned windows included and the
/// cross-week memory applied, exactly as the drafter sees them.
pub(crate) fn slot_candidates(
    index: &Index,
    rules: &Rules,
    aspect: Aspect,
    dur: f32,
) -> Vec<Candidate> {
    let memory = Memory::for_index(index);
    candidates(index, rules, aspect, dur, true, &memory)
}

/// Whether `c` may take slot `k` of an existing plan whose other shots are
/// `placed` (`None` at `k`): the same-clip gap and the performer gap.
/// Look-alike and reuse are judged afterwards by `plan::check`.
pub(crate) fn plan_allows(
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

#[cfg(test)]
mod tests {
    use super::{Pattern, Want, grid, pace};
    use crate::beats::{Bar, MusicMap};

    /// A 140 BPM map whose bar energies are `low`.
    fn map(low: &[f32]) -> MusicMap {
        MusicMap {
            path: String::new(),
            duration: 100.0,
            bpm: 140.0,
            beats: (0..low.len() * 4)
                .map(|i| i as f32 * 60.0 / 140.0)
                .collect(),
            bars: low
                .iter()
                .enumerate()
                .map(|(i, &l)| Bar {
                    t: i as f32,
                    rms: 1.0,
                    low: l,
                })
                .collect(),
            drops: Vec::new(),
            ..MusicMap::default()
        }
    }

    #[test]
    fn the_grid_fills_whole_bars() -> anyhow::Result<()> {
        let map = map(&[1.0; 50]);
        let slots = grid(&map, 4, 12, 16, 0);
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

    #[test]
    fn pacing_follows_energy_and_avoids_uniform_lengths() -> anyhow::Result<()> {
        // Four quiet bars, then eight loud ones.
        let mut low = vec![0.1; 4];
        low.extend([1.0; 8]);
        let m = map(&low);
        let p = pace(&m, 0, 0, 12, 0);
        let covered: usize = p.iter().map(|x| x.bars()).sum();
        assert_eq!(covered, 12, "every bar paced: {p:?}");
        let quiet_cuts: usize = p
            .iter()
            .scan(0, |bar, x| {
                let here = *bar;
                *bar += x.bars();
                Some((here, x))
            })
            .filter(|(bar, _)| *bar < 4)
            .map(|(_, x)| x.beats().len())
            .sum();
        assert!(quiet_cuts <= 4, "quiet bars hold: {p:?}");
        let loud: Vec<&Pattern> = p.iter().rev().take(8).collect();
        assert!(
            loud.iter().all(|x| x.rate() >= 2.0),
            "loud bars cut at least every 2 beats: {p:?}"
        );
        assert!(
            loud.windows(3).all(|w| !(w[0] == w[1] && w[1] == w[2])),
            "no pattern three bars running: {p:?}"
        );
        // Bars 7 and 11 end phrases: they stutter 1 + 1 + 2.
        let at = |bar: usize| {
            let mut b = 0;
            p.iter().find(|x| {
                let hit = b == bar;
                b += x.bars();
                hit
            })
        };
        assert_eq!(
            at(7),
            Some(&Pattern::StutterEnd),
            "phrase end stutters: {p:?}"
        );
        assert_eq!(
            at(11),
            Some(&Pattern::StutterEnd),
            "phrase end stutters: {p:?}"
        );
        Ok(())
    }

    /// Fourteen 60 s fire clips, each with one fireball; clip F07's is the
    /// biggest. Returns the index and F07's fireball onset.
    fn fireball_night() -> (crate::index::Index, f32) {
        use crate::analyze::{ClipIndex, Sample, moments, segments};
        let mut clips = Vec::new();
        for c in 0..14 {
            let ball = 20 + 7 * c; // sample index of the fireball
            let size = if c == 7 { 0.15 } else { 0.08 };
            let mut samples: Vec<Sample> = (0..360)
                .map(|i| Sample {
                    t: i as f32 / 6.0,
                    luma: 0.05,
                    p95: 0.6,
                    sharp: 1.0,
                    flame: if (ball..ball + 12).contains(&i) {
                        size
                    } else {
                        0.03
                    },
                    moving: 0.01,
                    cx: 0.5,
                    cy: 0.5,
                    motion_ok: true,
                    lit: 0.4,
                    lit_frac: 0.05,
                    lit_p95: 0.9,
                    ..Sample::default()
                })
                .collect();
            moments::bursts(&mut samples);
            let segs = segments::segment(&samples);
            clips.push(ClipIndex {
                id: format!("F{c:02}"),
                path: String::new(),
                stamp: (0, 0),
                probe: crate::media::Probe {
                    duration: 60.0,
                    width: 1920,
                    height: 1080,
                    fps: [24, 1],
                    vcodec: String::new(),
                    has_audio: false,
                    recorded_at: None,
                    camera: None,
                },
                samples,
                segments: segs,
                hist: Vec::new(),
                version: crate::analyze::ANALYSIS_VERSION,
            });
        }
        let onset = (20 + 7 * 7) as f32 / 6.0 - 1.0 / 12.0;
        let thresholds = crate::index::Thresholds::default();
        let (sharp_ref, segments) = crate::index::score(&clips, &thresholds);
        (
            crate::index::Index {
                sharp_ref,
                clips,
                segments,
                thresholds,
                ..crate::index::Index::default()
            },
            onset,
        )
    }

    #[test]
    fn the_drop_puts_the_biggest_burst_on_the_downbeat() -> anyhow::Result<()> {
        let (index, onset) = fireball_night();
        let mut music = map(&[0.2; 8]
            .iter()
            .chain(&[1.0; 32])
            .copied()
            .collect::<Vec<_>>());
        music.drops.push(crate::beats::Drop {
            bar: 8,
            t: 0.0,
            score: 1.0,
            user: false,
        });
        let spec = super::Spec {
            name: "reel-15".into(),
            aspect: crate::plan::Aspect::Vertical,
            seconds: 15.0,
            end_card: String::new(),
            card_bars: 1,
        };
        let people = crate::people::People::default();
        let plan = super::draft(&index, &music, &people, &spec, "m.wav")?;
        let drop_beat = (plan.drop_bar - plan.start_bar) * 4;
        let mut beat = 0;
        let hero = plan
            .shots
            .iter()
            .find(|s| {
                let hit = beat == drop_beat;
                beat += s.beats;
                hit
            })
            .ok_or_else(|| anyhow::anyhow!("no shot on the drop"))?;
        assert_eq!(hero.clip, "F07", "the biggest fireball: {hero:?}");
        assert!(
            (hero.t_in - onset).abs() <= 1.0 / 24.0,
            "the burst lands within a frame of the cut: in {} vs onset {onset}",
            hero.t_in
        );
        let problems = crate::plan::check(&plan, &index, &music, &people);
        assert!(
            problems.is_empty(),
            "the draft passes its check: {problems:?}"
        );
        Ok(())
    }

    #[test]
    fn easing_cuts_less() -> anyhow::Result<()> {
        let m = map(&[1.0; 12]);
        let cuts = |ease| -> usize {
            pace(&m, 0, 0, 12, ease)
                .iter()
                .map(|x| x.beats().len())
                .sum()
        };
        assert!(cuts(2) < cuts(0), "eased pacing has fewer cuts");
        Ok(())
    }
}

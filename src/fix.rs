// ============================================================================
// Module:       fix
// Description:  Automatic fixes for check findings: candidate edits per
//               finding id, each re-checked, kept only when it clears the
//               finding without adding a hard one; plain-words advice.
//
// Dependencies: crate::{plan, alternatives, analyze::spans, index, beats,
//               people}, crate::gui::assistant::ops
// ============================================================================

//! Fixes for what the check finds.
//!
//! Every finding can be acted on from where it is shown. [`fixes`] proposes
//! edits for one finding, by id: swap a shot for a ranked alternative
//! ([`crate::alternatives::rank`]), move its in-point clear of a bad
//! stretch (the drafter's [`crate::analyze::spans::dodge`] among others),
//! lift a flash or give it a beat, reframe, or move an edit point onto the
//! drop. Each candidate is applied to a copy with the assistant's edit ops
//! ([`ops::apply`], so the same ops can be staged as a chat card) and the
//! copy is checked again. A candidate survives only when the finding is
//! gone from its place and no hard finding was added; the survivors are
//! ranked by how many new soft findings they bring, then by what is left.
//!
//! Some findings have no automatic fix (`beat-count`, `bar-order`,
//! `past-track`): which shot should give or take beats, or which bars the
//! cut should use, is the editor's call. [`advice`] says in plain words
//! what every id means and what to do about it.
//!
//! [`fix_all`] applies the best fix for each finding in turn, re-checking
//! after each, and only keeps a fix that leaves fewer findings than before,
//! so it always ends.

use std::collections::BTreeMap;

use crate::alternatives::Alternative;
use crate::beats::MusicMap;
use crate::gui::assistant::ops::{self, EditOp};
use crate::index::Index;
use crate::people::People;
use crate::plan::{Finding, Memory, Plan, Side};

/// What fixing needs to know about the night.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Night<'a> {
    /// The shot log.
    pub(crate) index: &'a Index,
    /// The music map.
    pub(crate) map: &'a MusicMap,
    /// The people map.
    pub(crate) people: &'a People,
    /// Other nights' plans, loaded once (the check reads them from disk).
    pub(crate) memory: &'a Memory,
}

impl Night<'_> {
    /// The rule check of `plan` on this night.
    pub(crate) fn check(&self, plan: &Plan) -> Vec<Finding> {
        crate::plan::check_with(plan, self.index, self.map, self.people, self.memory)
    }
}

/// One automatic fix for a finding, already applied to a copy and checked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Fix {
    /// What it does, for a button: `Swap shot 4 MVI_7925/3 → MVI_7990/1`.
    pub(crate) label: String,
    /// The edit as the assistant's ops (what `plan_edit` takes).
    pub(crate) ops: Vec<EditOp>,
    /// The plan with the fix applied.
    pub(crate) plan: Plan,
    /// What it clears: `colour jump 0.43`.
    pub(crate) clears: String,
    /// Other findings it clears too.
    pub(crate) more: usize,
    /// Soft findings it adds (never hard ones).
    pub(crate) new: Vec<Finding>,
}

impl Fix {
    /// The label with what the fix does to the check:
    /// `Swap shot 4 A/3 → B/1 (clears colour jump 0.43, no new findings)`.
    pub(crate) fn preview(&self) -> String {
        let more = if self.more > 0 {
            format!(" and {} more", self.more)
        } else {
            String::new()
        };
        let new = if self.new.is_empty() {
            "no new findings".to_owned()
        } else {
            let ids: Vec<&str> = self.new.iter().map(|f| f.id).collect();
            format!("adds {} soft: {}", self.new.len(), ids.join(", "))
        };
        format!("{} (clears {}{more}, {new})", self.label, self.clears)
    }
}

/// Where a finding is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Place {
    /// The whole plan.
    Plan,
    /// Shot `n` (1-based).
    Shot(usize),
    /// The cut between shot `n` and `n + 1`.
    Seam(usize),
}

impl Place {
    /// Where `f` is.
    fn of(f: &Finding) -> Self {
        match (f.seam, f.shot) {
            (Some(n), _) => Self::Seam(n),
            (None, Some(n)) => Self::Shot(n),
            (None, None) => Self::Plan,
        }
    }

    /// Where this place is once shot `removed` (1-based) has been lifted,
    /// `None` when it went with the shot.
    fn after_removing(self, removed: usize) -> Option<Self> {
        match self {
            Self::Plan => Some(self),
            Self::Shot(p) if p < removed => Some(self),
            Self::Shot(p) if p == removed => None,
            Self::Shot(p) => Some(Self::Shot(p - 1)),
            Self::Seam(s) if s + 1 < removed => Some(self),
            Self::Seam(s) if s + 1 == removed => None,
            Self::Seam(s) if s == removed => (s > 1).then(|| Self::Seam(s - 1)),
            Self::Seam(s) => Some(Self::Seam(s - 1)),
        }
    }
}

/// A candidate edit, before it is checked.
#[derive(Clone, Debug)]
struct Candidate {
    /// What it does.
    label: String,
    /// The ops.
    ops: Vec<EditOp>,
    /// The shot it lifts (1-based), which renumbers the ones after it.
    removed: Option<usize>,
    /// How much of the editor's cut it changes, to break ties between fixes
    /// that leave the check alike: moving an in-point or a beat is cheap,
    /// swapping footage dearer the further down the ranking it reaches, and
    /// replacing the hero shot on the drop dearest of all.
    cost: f32,
}

/// Plain words for every finding id: what it means and what to do about it.
/// Hard findings show this beside the message; every id has one.
pub(crate) fn advice(id: &str) -> &'static str {
    match id {
        "beat-count" => {
            "The shots must exactly fill the bars up to the end card: the music is the clock. Drag an edit point or change a shot's beats until the total matches; which shot gives or takes is your call, so there is no automatic fix."
        }
        "bar-order" => {
            "The start, drop, end-card and end bars are out of order. Set them so start <= drop < card <= end in the cut's settings or its plan file."
        }
        "past-track" => {
            "The cut ends after the song does. Move the end bar back inside the track, or pick a longer song."
        }
        "off-drop" => {
            "No cut lands exactly on the drop, so the hero shot misses the moment the song hits. Fix moves the nearest edit point onto the drop."
        }
        "drop-subject" => {
            "The shot on the drop is not what the rules ask for there (usually fire). Fix swaps in the strongest shot that is."
        }
        "missing-clip" => {
            "The shot points at a clip the shot log does not have (renamed, moved or never analysed). Swap it for other footage, or analyse the clip again."
        }
        "outside-clip" => {
            "The shot asks for frames before its clip starts or after it ends, so the render would come out short. Fix moves the in-point back inside the clip."
        }
        "bad-span" => {
            "The shot runs over a short bad stretch inside good footage: a focus hunt, a whip, a jolt, or a dark or blown moment. Fix moves the in-point clear of it, or swaps the shot."
        }
        "crop-outside" => {
            "The crop centre puts part of the vertical crop outside the picture. Fix re-centres it inside the frame."
        }
        "empty-shot" => {
            "The shot is so short it gets no frames at all, so the render cannot cut it. Fix gives it a beat from a neighbour or lifts it."
        }
        "flash-shot" => {
            "The shot is only a few frames long: the eye sees that something changed but not what. Fix gives it a beat from a neighbour or lifts it."
        }
        "flagged" => {
            "The range is flagged (dark, whip or soft focus). Move the in-point to clean footage, or swap the shot."
        }
        "bad-moment" => {
            "The caption says this is not a performing moment (a transition, someone idle). Move the in-point, or swap the shot."
        }
        "wide-action" => {
            "The action wanders wider than the crop, so the performer leaves the frame. Swap for a tighter shot, or reframe."
        }
        "same-stretch" => {
            "Two shots start close together in one clip, so the cut shows the same stretch twice. Move one in-point further on, or swap one."
        }
        "repeat-performer" => {
            "The same performer comes back too soon. Swap one of the shots for someone else."
        }
        "lookalike" => {
            "Reads as the same shot as an earlier one (same spot, performer and action). Swap one for something that looks different."
        }
        "reused" => {
            "This moment was already used in another night's cut. Move the in-point to a moment not used before, or swap it."
        }
        "jump-cut" => {
            "The cut goes from a clip to a few seconds later in the same clip, so the picture visibly skips. Swap one side for another clip, or pick up where the first shot left off."
        }
        "exposure-jump" => {
            "The brightness jumps across the cut by more than the render can even out. Swap one side for a shot closer in brightness."
        }
        "colour-jump" => {
            "The mid-tone colour jumps across the cut by more than the render's colour match can close. Swap one side for a shot closer in colour, or accept it when the change is the look."
        }
        _ => "An unknown rule: read its message and edit the plan by hand.",
    }
}

/// Whether finding id `id` can ever have an automatic fix.
pub(crate) fn fixable(id: &str) -> bool {
    !matches!(id, "beat-count" | "bar-order" | "past-track")
}

/// A shot's segment id for a label (its `seg`, else the segment holding its
/// in-point, else `clip@in`).
fn seg_name(plan: &Plan, k: usize, index: &Index) -> String {
    let Some(shot) = plan.shots.get(k) else {
        return "?".to_owned();
    };
    if !shot.seg.is_empty() {
        return shot.seg.clone();
    }
    index
        .segment_at(&shot.clip, shot.t_in, shot.t_in + 0.1)
        .map_or_else(
            || format!("{}@{:.2}", shot.clip, shot.t_in),
            |g| g.id.clone(),
        )
}

/// Seconds rounded to hundredths, as plans store them.
fn hundredths(t: f32) -> f32 {
    (t * 100.0).round() / 100.0
}

/// Candidates that swap shot `k` (0-based) for its best alternatives that
/// `keep` lets through, at most two per clip so a handful of tries covers
/// several clips (the ranking alone tends to offer one clip's windows in a
/// row).
fn swaps(
    plan: &Plan,
    k: usize,
    night: Night<'_>,
    limit: usize,
    keep: impl Fn(&Alternative) -> bool,
) -> Vec<Candidate> {
    /// Alternatives ranked before filtering.
    const POOL: usize = 120;
    /// Tries per clip.
    const PER_CLIP: usize = 2;
    let old = seg_name(plan, k, night.index);
    let hero = crate::alternatives::want_for(plan, k, night.index) == crate::draft::Want::Hero;
    let mut per: BTreeMap<String, usize> = BTreeMap::new();
    crate::alternatives::rank(plan, k, night.index, night.map, night.people, POOL)
        .into_iter()
        .filter(|a| keep(a))
        .filter(|a| {
            let n = per.entry(a.clip.clone()).or_insert(0);
            *n += 1;
            *n <= PER_CLIP
        })
        .take(limit)
        .enumerate()
        .map(|(rank, a)| Candidate {
            label: format!("Swap shot {} {old} → {}", k + 1, a.seg),
            ops: vec![EditOp::ReplaceShot {
                index: k + 1,
                clip: a.clip.clone(),
                t_in: hundredths(a.t_in),
                cx: Some(hundredths(a.cx)),
                reason: format!("swapped: {}", a.reason),
            }],
            removed: None,
            cost: 2.0 + rank as f32 * 0.1 + if hero { 10.0 } else { 0.0 },
        })
        .collect()
}

/// Whether a cut from `a` to `b` stays within the seam rules' brightness
/// and colour limits (with a little margin), or cannot be judged.
fn smooth(index: &Index, a: Side<'_>, b: Side<'_>) -> bool {
    /// Margin under the limits, so a fix is not one sample from failing.
    const MARGIN: f32 = 0.9;
    crate::plan::seam_jump(index, a, b).is_none_or(|(stops, colour)| {
        stops <= crate::plan::EXPOSURE_JUMP_STOPS * MARGIN
            && colour.is_none_or(|d| d <= crate::plan::COLOUR_JUMP * MARGIN)
    })
}

/// A filter for alternatives to shot `k` that keeps both its cuts smooth.
fn smooth_at<'a>(
    plan: &'a Plan,
    k: usize,
    index: &'a Index,
    timing: &'a [(f32, f32)],
) -> impl Fn(&Alternative) -> bool + 'a {
    move |alt| {
        let dur = timing.get(k).map_or(0.0, |t| t.1);
        let me: Side<'_> = (alt.clip.as_str(), alt.t_in, dur);
        let side = |j: usize| -> Option<Side<'_>> {
            let s = plan.shots.get(j)?;
            Some((s.clip.as_str(), s.t_in, timing.get(j)?.1))
        };
        k.checked_sub(1)
            .and_then(side)
            .is_none_or(|before| smooth(index, before, me))
            && side(k + 1).is_none_or(|after| smooth(index, me, after))
    }
}

/// Candidates that move shot `k`'s in-point within its clip: clear of the
/// clip's rejected bad spans ([`crate::analyze::spans::dodge`]), past the
/// edges of the segments it overlaps, back inside the clip, and a few
/// seconds either way. Nearest first.
fn shifts(plan: &Plan, k: usize, night: Night<'_>, timing: &[(f32, f32)]) -> Vec<Candidate> {
    let (Some(shot), Some(&(_, dur))) = (plan.shots.get(k), timing.get(k)) else {
        return Vec::new();
    };
    let Some(clip) = night.index.clip(&shot.clip) else {
        return Vec::new();
    };
    let hi = clip.probe.duration as f32 - dur - 0.02;
    if hi < 0.0 {
        return Vec::new();
    }
    let t_in = shot.t_in;
    let mut ts: Vec<f32> = [0.5, 1.0, 2.0, 3.0, 5.0, 8.0]
        .into_iter()
        .flat_map(|d| [t_in + d, t_in - d])
        .collect();
    let segs: Vec<&crate::index::Scored> = night
        .index
        .segments
        .iter()
        .filter(|g| g.clip == shot.clip)
        .collect();
    for g in segs
        .iter()
        .filter(|g| g.seg.end > t_in && g.seg.start < t_in + dur)
    {
        ts.push(g.seg.end + 0.1);
        ts.push(g.seg.start - dur - 0.1);
    }
    let spans: Vec<&crate::analyze::spans::BadSpan> = segs
        .iter()
        .flat_map(|g| g.seg.bad_spans.iter())
        .filter(|s| plan.rules.reject_flags.contains(&s.flag))
        .collect();
    if let Some(t) = crate::analyze::spans::dodge(&spans, t_in, dur, 0.0, hi) {
        ts.push(t);
    }
    ts.push(t_in.clamp(0.0, hi));
    let mut ts: Vec<f32> = ts
        .into_iter()
        .filter(|t| t.is_finite() && (0.0..=hi).contains(t))
        .map(hundredths)
        .filter(|t| (t - t_in).abs() >= 0.01)
        .collect();
    ts.sort_by(|a, b| (a - t_in).abs().total_cmp(&(b - t_in).abs()));
    ts.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    ts.into_iter()
        .map(|t| Candidate {
            label: format!("Move shot {}'s in-point {t_in:.2} → {t:.2} s", k + 1),
            ops: vec![EditOp::SetIn {
                index: k + 1,
                t_in: t,
            }],
            removed: None,
            cost: 1.0 + (t - t_in).abs() / 10.0,
        })
        .collect()
}

/// Candidates for a shot too short to read: lift it, or give it beats
/// from a neighbour that can spare them.
fn lengthen(plan: &Plan, k: usize) -> Vec<Candidate> {
    let mut out = Vec::new();
    let Some(shot) = plan.shots.get(k) else {
        return out;
    };
    for j in [k + 1, k.wrapping_sub(1)] {
        let Some(n) = plan.shots.get(j) else {
            continue;
        };
        for give in 1..=2 {
            if n.beats > give {
                out.push(Candidate {
                    label: format!(
                        "Lengthen shot {} to {} beats (shot {} gives {give})",
                        k + 1,
                        shot.beats + give,
                        j + 1
                    ),
                    ops: vec![
                        EditOp::SetBeats {
                            index: k + 1,
                            beats: shot.beats + give,
                        },
                        EditOp::SetBeats {
                            index: j + 1,
                            beats: n.beats - give,
                        },
                    ],
                    removed: None,
                    cost: 1.0,
                });
            }
        }
    }
    if plan.shots.len() > 1 {
        let heir = if k == 0 { 2 } else { k };
        out.push(Candidate {
            label: format!("Lift shot {} (its beats go to shot {heir})", k + 1),
            ops: vec![EditOp::RemoveShot { index: k + 1 }],
            removed: Some(k + 1),
            cost: 1.5,
        });
    }
    out
}

/// Candidates that put an edit point on the drop: the shot over it ends
/// there and the next one starts there, then (optionally) the best shots
/// for the drop swapped into it.
fn onto_drop(plan: &Plan, night: Night<'_>) -> Vec<Candidate> {
    let drop_beat = plan.drop_bar.saturating_sub(plan.start_bar) * 4;
    let mut start = 0;
    let mut over = None;
    for (k, s) in plan.shots.iter().enumerate() {
        if start < drop_beat && drop_beat < start + s.beats {
            over = Some((k, start));
            break;
        }
        start += s.beats;
    }
    let Some((k, start)) = over else {
        return Vec::new();
    };
    let (Some(a), Some(b)) = (plan.shots.get(k), plan.shots.get(k + 1)) else {
        return Vec::new();
    };
    let rebalance = vec![
        EditOp::SetBeats {
            index: k + 1,
            beats: drop_beat - start,
        },
        EditOp::SetBeats {
            index: k + 2,
            beats: b.beats + (start + a.beats - drop_beat),
        },
    ];
    let label = format!(
        "Cut on the drop: shot {} ends at bar {}, shot {} starts there",
        k + 1,
        plan.drop_bar,
        k + 2
    );
    let mut out = vec![Candidate {
        label: label.clone(),
        ops: rebalance.clone(),
        removed: None,
        cost: 1.0,
    }];
    let base = ops::Night {
        index: Some(night.index),
        people: night.people,
    };
    if let Ok((moved, _)) = ops::apply(plan, &rebalance, &ops::Scope::Whole, base) {
        for c in swaps(&moved, k + 1, night, 6, |_| true) {
            let mut all = rebalance.clone();
            all.extend(c.ops);
            let swapped = c.label.split(" → ").nth(1).unwrap_or_default().to_owned();
            out.push(Candidate {
                label: format!("{label}, with {swapped} on the drop"),
                ops: all,
                removed: None,
                cost: c.cost + 1.0,
            });
        }
    }
    out
}

/// The candidate edits for finding `f`, best guesses first.
fn candidates(plan: &Plan, f: &Finding, night: Night<'_>) -> Vec<Candidate> {
    /// Alternatives tried per shot.
    const SWAPS: usize = 10;
    let timing = plan.timing(night.map);
    let n = plan.shots.len();
    let any = |_: &Alternative| true;
    match Place::of(f) {
        Place::Seam(s) if s >= 1 && s < n => {
            let (a, b) = (s - 1, s);
            match f.id {
                "jump-cut" => {
                    let (ca, cb) = (plan.shots[a].clip.clone(), plan.shots[b].clip.clone());
                    let mut out = swaps(plan, b, night, SWAPS, |x| x.clip != ca);
                    out.extend(swaps(plan, a, night, SWAPS, |x| x.clip != cb));
                    let a_out = plan.shots[a].t_in + timing.get(a).map_or(0.0, |t| t.1);
                    let len = night
                        .index
                        .clip(&cb)
                        .map_or(0.0, |c| c.probe.duration as f32);
                    let dur_b = timing.get(b).map_or(0.0, |t| t.1);
                    for (t, how) in [
                        (a_out, "picks up where shot {a} left off"),
                        (
                            a_out + crate::plan::JUMP_CUT_GAP + 0.1,
                            "skips on far enough",
                        ),
                    ] {
                        let t = hundredths(t);
                        if t >= 0.0 && t + dur_b <= len {
                            out.push(Candidate {
                                label: format!(
                                    "Move shot {}'s in-point to {t:.2} s ({})",
                                    b + 1,
                                    how.replace("{a}", &(a + 1).to_string())
                                ),
                                ops: vec![EditOp::SetIn {
                                    index: b + 1,
                                    t_in: t,
                                }],
                                removed: None,
                                cost: 1.0,
                            });
                        }
                    }
                    out
                }
                _ => {
                    // Exposure and colour jumps (and any other seam rule):
                    // change the shot after the cut first, then the one before.
                    let mut out = swaps(
                        plan,
                        b,
                        night,
                        SWAPS,
                        smooth_at(plan, b, night.index, &timing),
                    );
                    out.extend(swaps(
                        plan,
                        a,
                        night,
                        SWAPS,
                        smooth_at(plan, a, night.index, &timing),
                    ));
                    out.extend(shifts(plan, b, night, &timing).into_iter().take(8));
                    out.extend(shifts(plan, a, night, &timing).into_iter().take(8));
                    out
                }
            }
        }
        Place::Shot(s) if s >= 1 && s <= n => {
            let k = s - 1;
            match f.id {
                "flash-shot" | "empty-shot" => lengthen(plan, k),
                "crop-outside" => {
                    let half = plan.aspect.crop_frac() / 2.0;
                    let old = plan.shots[k].cx;
                    let cx = hundredths(old.clamp(half.min(0.5), (1.0 - half).max(0.5)));
                    vec![Candidate {
                        label: format!("Reframe shot {s} (crop centre {old:.2} → {cx:.2})"),
                        ops: vec![EditOp::SetCx { index: s, cx }],
                        removed: None,
                        cost: 1.0,
                    }]
                }
                "repeat-performer" | "lookalike" | "missing-clip" | "drop-subject" => {
                    swaps(plan, k, night, SWAPS, any)
                }
                "outside-clip" | "bad-span" | "flagged" | "bad-moment" | "reused"
                | "same-stretch" | "wide-action" => {
                    let mut out = shifts(plan, k, night, &timing);
                    out.extend(swaps(plan, k, night, SWAPS, any));
                    out
                }
                _ => swaps(plan, k, night, SWAPS, any),
            }
        }
        Place::Plan if f.id == "off-drop" => onto_drop(plan, night),
        Place::Seam(_) | Place::Shot(_) | Place::Plan => Vec::new(),
    }
}

/// The plain name of finding `f` with its measure for colour and exposure
/// jumps: `colour jump 0.43`.
fn clears(f: &Finding) -> String {
    let name = f.id.replace('-', " ");
    let measured = matches!(f.id, "colour-jump" | "exposure-jump")
        .then(|| {
            f.message
                .split_whitespace()
                .find(|w| w.contains('.') && w.parse::<f32>().is_ok())
        })
        .flatten();
    match measured {
        Some(v) => format!("{name} {v}"),
        None => name,
    }
}

/// Counts findings by (id, place).
fn tally<'a>(
    findings: impl Iterator<Item = (&'static str, Option<Place>)> + 'a,
) -> BTreeMap<(&'static str, Place), usize> {
    let mut out = BTreeMap::new();
    for (id, place) in findings {
        if let Some(p) = place {
            *out.entry((id, p)).or_insert(0) += 1;
        }
    }
    out
}

/// Up to `limit` automatic fixes for finding `f` of `plan`, best first:
/// each clears `f` from its place and adds no hard finding. Empty when
/// there is none (or the id never has one: see [`fixable`]).
pub(crate) fn fixes(plan: &Plan, f: &Finding, night: Night<'_>, limit: usize) -> Vec<Fix> {
    if !fixable(f.id) || limit == 0 {
        return Vec::new();
    }
    let before = night.check(plan);
    let hard_before = crate::plan::hard_count(&before);
    let target = Place::of(f);
    let base = ops::Night {
        index: Some(night.index),
        people: night.people,
    };
    let mut out: Vec<((usize, usize, f32), Fix)> = Vec::new();
    for c in candidates(plan, f, night) {
        let Ok((edited, _)) = ops::apply(plan, &c.ops, &ops::Scope::Whole, base) else {
            continue;
        };
        if edited == *plan || out.iter().any(|(_, x)| x.plan == edited) {
            continue;
        }
        let after = night.check(&edited);
        let moved = |p: Place| match c.removed {
            Some(r) => p.after_removing(r),
            None => Some(p),
        };
        if let Some(at) = moved(target)
            && after.iter().any(|g| g.id == f.id && Place::of(g) == at)
        {
            continue;
        }
        if crate::plan::hard_count(&after) > hard_before {
            continue;
        }
        let was = tally(before.iter().map(|g| (g.id, moved(Place::of(g)))));
        let mut seen: BTreeMap<(&'static str, Place), usize> = BTreeMap::new();
        let mut new = Vec::new();
        for g in &after {
            let key = (g.id, Place::of(g));
            let k = seen.entry(key).or_insert(0);
            *k += 1;
            if *k > was.get(&key).copied().unwrap_or(0) {
                new.push(g.clone());
            }
        }
        if new.iter().any(Finding::is_hard) {
            continue;
        }
        let open = |p: &Plan, list: &[Finding]| list.iter().filter(|g| !p.is_accepted(g)).count();
        let left = open(&edited, &after);
        let more = (open(plan, &before) + new.len()).saturating_sub(left + 1);
        out.push((
            (new.len(), left, c.cost),
            Fix {
                label: c.label,
                ops: c.ops,
                plan: edited,
                clears: clears(f),
                more,
                new,
            },
        ));
    }
    // Fewest new findings, then fewest left, then the gentlest edit; the
    // sort is stable, so among equals the candidate order holds.
    out.sort_by(|(a, _), (b, _)| {
        (a.0, a.1)
            .cmp(&(b.0, b.1))
            .then_with(|| a.2.total_cmp(&b.2))
    });
    out.into_iter().take(limit).map(|(_, fix)| fix).collect()
}

/// A fix [`fix_all`] made.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Applied {
    /// The finding it was for, as the check printed it before the fix.
    pub(crate) finding: Finding,
    /// The fix.
    pub(crate) fix: Fix,
}

/// Applies automatic fixes to `plan` greedily: hard findings first, the
/// best fix for each, re-checking after every one. A fix is only kept when
/// it leaves fewer open findings (hard first, then soft) than before, so
/// the loop ends; accepted findings are left alone. Returns the fixed plan
/// and what was done, in order.
pub(crate) fn fix_all(plan: &Plan, night: Night<'_>) -> (Plan, Vec<Applied>) {
    /// More passes than any plan has findings.
    const PASSES: usize = 64;
    let mut plan = plan.clone();
    let mut done = Vec::new();
    let score = |p: &Plan, list: &[Finding]| {
        let open: Vec<&Finding> = list.iter().filter(|f| !p.is_accepted(f)).collect();
        let hard = open.iter().filter(|f| f.is_hard()).count();
        (hard, open.len() - hard)
    };
    for _ in 0..PASSES {
        let findings = night.check(&plan);
        let now = score(&plan, &findings);
        let mut order: Vec<&Finding> = findings
            .iter()
            .filter(|f| !plan.is_accepted(f) && fixable(f.id))
            .collect();
        order.sort_by_key(|f| f.severity);
        let mut step = None;
        for f in order {
            if let Some(fix) = fixes(&plan, f, night, 1).into_iter().next()
                && score(&fix.plan, &night.check(&fix.plan)) < now
            {
                step = Some(Applied {
                    finding: f.clone(),
                    fix,
                });
                break;
            }
        }
        let Some(applied) = step else {
            break;
        };
        plan = applied.fix.plan.clone();
        done.push(applied);
    }
    (plan, done)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{Night, advice, fix_all, fixable, fixes};
    use crate::analyze::segments::{Camera, Segment};
    use crate::analyze::{ClipIndex, Sample};
    use crate::beats::{Bar, MusicMap};
    use crate::index::{Index, Scored};
    use crate::media::Probe;
    use crate::people::People;
    use crate::plan::{Aspect, Finding, Memory, Plan, Rules, Shot};

    /// Clips A..F, 120 s each, a fire segment every 10 s, sampled every
    /// 0.25 s in a neutral grey; `paint` recolours a clip.
    pub(crate) fn index() -> Index {
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
        for (id, flame) in [
            ("A", 0.2),
            ("B", 0.15),
            ("C", 0.12),
            ("D", 0.1),
            ("E", 0.08),
            ("F", 0.06),
        ] {
            let samples = (0..480)
                .map(|i| Sample {
                    t: i as f32 * 0.25,
                    luma: 0.3,
                    p95: 0.8,
                    sharp: 1.0,
                    lit: 0.6,
                    lit_frac: 0.05,
                    moving: 0.02,
                    cx: 0.5,
                    flame,
                    mid_r: 0.3,
                    mid_g: 0.3,
                    mid_b: 0.3,
                    mid_frac: 0.3,
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
                    seg: Segment {
                        start: t,
                        end: t + 10.0,
                        camera: Camera::Static,
                        luma: 0.3,
                        p95: 0.8,
                        sharp: 1.0,
                        flame,
                        moving: 0.02,
                        cx: 0.5,
                        cx_spread: 0.05,
                        cy: 0.5,
                        zoom_factor: 1.0,
                        ..Segment::default()
                    },
                    subject: "fire".into(),
                    q: 0.8,
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

    /// Gives clip `clip` this mid-tone colour (and brightness).
    pub(crate) fn paint(index: &mut Index, clip: &str, luma: f32, mid: [f32; 3]) {
        for c in index.clips.iter_mut().filter(|c| c.id == clip) {
            for s in &mut c.samples {
                s.luma = luma;
                s.mid_r = mid[0];
                s.mid_g = mid[1];
                s.mid_b = mid[2];
            }
        }
    }

    /// 120 BPM, 2 s bars.
    pub(crate) fn map() -> MusicMap {
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
            ..Default::default()
        }
    }

    fn shot(clip: &str, t: f32, beats: usize) -> Shot {
        Shot {
            clip: clip.into(),
            t_in: t,
            beats,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: String::new(),
        }
    }

    /// Three one-bar shots of A, B, C at 15 s; the drop on bar 0, the card
    /// on bar 3. Clean on the grey night.
    pub(crate) fn plan() -> Plan {
        Plan {
            name: "t".into(),
            aspect: Aspect::Wide,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 0,
            card_bar: 3,
            end_bar: 4,
            end_card: String::new(),
            rules: Rules {
                person_gap: 0,
                ..Rules::default()
            },
            finish: Default::default(),
            shots: vec![shot("A", 15.0, 4), shot("B", 15.0, 4), shot("C", 15.0, 4)],
            comments: Vec::new(),
            accepted: Vec::new(),
        }
    }

    /// Checks the fix for the first finding with `id`: it exists, clears
    /// the finding from its place, and adds no hard finding.
    fn fixed(plan: &Plan, index: &Index, id: &str) -> anyhow::Result<Plan> {
        let (map, people, memory) = (map(), People::default(), Memory::default());
        let night = Night {
            index,
            map: &map,
            people: &people,
            memory: &memory,
        };
        let before = night.check(plan);
        let f = before
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| anyhow::anyhow!("no {id} to fix: {before:?}"))?;
        let all = fixes(plan, f, night, 3);
        let fix = all
            .first()
            .ok_or_else(|| anyhow::anyhow!("no fix for {f:?}"))?;
        let after = night.check(&fix.plan);
        assert!(
            !after
                .iter()
                .any(|g| g.id == id && g.seam == f.seam && g.shot == f.shot),
            "{} clears {id}: {after:?}",
            fix.label
        );
        assert!(
            crate::plan::hard_count(&after) <= crate::plan::hard_count(&before),
            "{} adds no hard finding: {after:?}",
            fix.label
        );
        assert!(
            fix.preview().starts_with(&fix.label) && fix.preview().contains("clears"),
            "the preview says what it does: {}",
            fix.preview()
        );
        Ok(fix.plan.clone())
    }

    fn findings(plan: &Plan, index: &Index) -> Vec<Finding> {
        crate::plan::check_with(plan, index, &map(), &People::default(), &Memory::default())
    }

    #[test]
    fn the_synthetic_plan_starts_clean() -> anyhow::Result<()> {
        let got = findings(&plan(), &index());
        assert!(got.is_empty(), "a clean start: {got:?}");
        Ok(())
    }

    #[test]
    fn a_colour_jump_is_fixed_by_swapping_a_side() -> anyhow::Result<()> {
        let mut index = index();
        paint(&mut index, "B", 0.3, [0.0, 0.05, 0.95]);
        let plan = plan();
        let out = fixed(&plan, &index, "colour-jump")?;
        assert!(
            out.shots.iter().all(|s| s.clip != "B"),
            "the blue shot is swapped out: {:?}",
            out.shots
        );
        assert!(
            findings(&out, &index).is_empty(),
            "nothing left and nothing new"
        );
        Ok(())
    }

    #[test]
    fn the_preview_names_the_swap_and_the_measure() -> anyhow::Result<()> {
        let mut index = index();
        paint(&mut index, "C", 0.3, [0.0, 0.05, 0.95]);
        let (map, people, memory) = (map(), People::default(), Memory::default());
        let night = Night {
            index: &index,
            map: &map,
            people: &people,
            memory: &memory,
        };
        let plan = plan();
        let got = night.check(&plan);
        let f = got
            .iter()
            .find(|f| f.id == "colour-jump")
            .ok_or_else(|| anyhow::anyhow!("no jump: {got:?}"))?;
        let fix = fixes(&plan, f, night, 1)
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no fix"))?;
        let text = fix.preview();
        assert!(
            text.starts_with("Swap shot 3 C/2 → ")
                && text.contains("clears colour jump 0.44")
                && text.ends_with("no new findings)"),
            "reads like a sentence: {text}"
        );
        Ok(())
    }

    #[test]
    fn an_exposure_jump_is_fixed() -> anyhow::Result<()> {
        let mut index = index();
        paint(&mut index, "B", 0.02, [0.3, 0.3, 0.3]);
        let mut plan = plan();
        plan.drop_bar = 2;
        plan.card_bar = 3;
        let got = findings(&plan, &index);
        assert!(
            got.iter().any(|f| f.id == "exposure-jump"),
            "a dark B: {got:?}"
        );
        fixed(&plan, &index, "exposure-jump")?;
        Ok(())
    }

    #[test]
    fn a_jump_cut_is_fixed() -> anyhow::Result<()> {
        let mut plan = plan();
        plan.rules.same_clip_gap = 0.0;
        plan.shots[1] = shot("A", 19.0, 4);
        let out = fixed(&plan, &index(), "jump-cut")?;
        assert!(
            out.shots[0].clip != out.shots[1].clip || (out.shots[1].t_in - 17.0).abs() < 0.02,
            "another clip, or picked up where shot 1 left off: {:?}",
            out.shots
        );
        Ok(())
    }

    #[test]
    fn a_flash_and_an_empty_shot_are_lengthened_or_lifted() -> anyhow::Result<()> {
        let mut plan = plan();
        plan.shots[0].beats = 1;
        plan.shots[1].beats = 7;
        plan.finish.cut_lead_frames = 8;
        fixed(&plan, &index(), "flash-shot")?;
        let mut plan = self::plan();
        plan.shots[0].beats = 0;
        plan.shots[1].beats = 8;
        fixed(&plan, &index(), "empty-shot")?;
        Ok(())
    }

    #[test]
    fn bad_stretches_are_dodged_by_the_in_point() -> anyhow::Result<()> {
        let mut index = index();
        for g in index.segments.iter_mut().filter(|g| g.id == "B/2") {
            g.seg.bad_spans.push(crate::analyze::spans::BadSpan {
                start: 15.5,
                end: 16.0,
                flag: "whip".into(),
                value: 1.0,
            });
        }
        let plan = plan();
        let out = fixed(&plan, &index, "bad-span")?;
        assert_eq!(out.shots[1].clip, "B", "the in-point moves, the clip stays");
        // Flags and captioned moments of the whole segment: move off it.
        let mut index = self::index();
        for g in index.segments.iter_mut().filter(|g| g.id == "B/2") {
            g.flags.push("dark".into());
        }
        fixed(&plan, &index, "flagged")?;
        let mut index = self::index();
        for g in index.segments.iter_mut().filter(|g| g.id == "B/2") {
            g.caption = Some(crate::caption::Caption {
                moment: "transition".into(),
                ..Default::default()
            });
        }
        fixed(&plan, &index, "bad-moment")?;
        Ok(())
    }

    #[test]
    fn repeats_are_swapped_for_other_footage() -> anyhow::Result<()> {
        let mut plan = plan();
        plan.rules.person_gap = 8;
        plan.shots[2] = shot("A", 60.0, 4);
        fixed(&plan, &index(), "repeat-performer")?;
        let mut plan = self::plan();
        plan.shots[2] = shot("A", 25.0, 4);
        fixed(&plan, &index(), "same-stretch")?;
        let mut index = index();
        let sig = crate::analyze::segments::Signature {
            hist: (0..32).map(|i| if i == 3 { 1.0 } else { 0.0 }).collect(),
            grid: (0..16).map(|i| if i == 5 { 1.0 } else { 0.0 }).collect(),
            cx: 0.5,
        };
        for g in index.segments.iter_mut().filter(|g| g.clip != "C") {
            g.seg.sig = sig.clone();
        }
        let mut plan = self::plan();
        plan.shots[2] = shot("D", 15.0, 4);
        fixed(&plan, &index, "lookalike")?;
        Ok(())
    }

    #[test]
    fn a_shot_outside_its_clip_or_crop_is_brought_back() -> anyhow::Result<()> {
        let mut plan = plan();
        plan.shots[1].t_in = 119.0;
        let fixed_plan = fixed(&plan, &index(), "outside-clip")?;
        assert_eq!(fixed_plan.shots[1].clip, "B", "moved, not swapped");
        let mut plan = self::plan();
        plan.aspect = Aspect::Vertical;
        plan.shots[0].cx = 0.05;
        fixed(&plan, &index(), "crop-outside")?;
        let mut plan = self::plan();
        plan.shots[1].clip = "Z".into();
        fixed(&plan, &index(), "missing-clip")?;
        Ok(())
    }

    #[test]
    fn the_drop_gets_a_cut_and_the_right_subject() -> anyhow::Result<()> {
        let mut plan = plan();
        plan.drop_bar = 1;
        plan.shots = vec![shot("A", 15.0, 6), shot("B", 15.0, 6)];
        let fixed_plan = fixed(&plan, &index(), "off-drop")?;
        assert_eq!(
            fixed_plan.shots[0].beats, 4,
            "shot 1 now ends on the drop: {:?}",
            fixed_plan.shots
        );
        let mut index = index();
        for g in index.segments.iter_mut().filter(|g| g.clip == "A") {
            g.subject = "led".into();
        }
        fixed(&self::plan(), &index, "drop-subject")?;
        Ok(())
    }

    #[test]
    fn the_rules_on_bars_have_no_automatic_fix_but_say_what_to_do() -> anyhow::Result<()> {
        let mut plan = plan();
        plan.shots[2].beats = 8;
        let (index, map, people, memory) = (index(), map(), People::default(), Memory::default());
        let night = Night {
            index: &index,
            map: &map,
            people: &people,
            memory: &memory,
        };
        let got = night.check(&plan);
        let f = got
            .iter()
            .find(|f| f.id == "beat-count")
            .ok_or_else(|| anyhow::anyhow!("no beat-count: {got:?}"))?;
        assert!(fixes(&plan, f, night, 3).is_empty(), "no guess");
        for (id, _, _) in crate::plan::IDS {
            assert!(
                advice(id).len() > 40 && !advice(id).starts_with("An unknown"),
                "{id} is explained"
            );
        }
        assert!(
            !fixable("beat-count") && !fixable("bar-order") && !fixable("past-track"),
            "the bar rules are the editor's call"
        );
        Ok(())
    }

    #[test]
    fn fix_all_clears_what_it_can_and_stops() -> anyhow::Result<()> {
        let mut index = index();
        paint(&mut index, "B", 0.3, [0.0, 0.05, 0.95]);
        let mut plan = plan();
        plan.shots[2].t_in = 119.0;
        let (map, people, memory) = (map(), People::default(), Memory::default());
        let night = Night {
            index: &index,
            map: &map,
            people: &people,
            memory: &memory,
        };
        let before = night.check(&plan);
        assert!(
            before.iter().any(|f| f.id == "outside-clip")
                && before.iter().any(|f| f.id == "colour-jump"),
            "a hard and a soft finding: {before:?}"
        );
        let (fixed, done) = fix_all(&plan, night);
        assert!(
            night.check(&fixed).is_empty(),
            "both fixed: {:?}",
            night.check(&fixed)
        );
        assert!(
            done.first().is_some_and(|a| a.finding.id == "outside-clip"),
            "hard first: {done:?}"
        );
        let (again, more) = fix_all(&fixed, night);
        assert!(more.is_empty() && again == fixed, "nothing left to do");
        Ok(())
    }

    #[test]
    fn accepted_findings_are_left_alone_and_lapse_when_the_shot_changes() -> anyhow::Result<()> {
        let mut index = index();
        paint(&mut index, "C", 0.3, [0.0, 0.05, 0.95]);
        let mut plan = plan();
        let got = findings(&plan, &index);
        let jump = got
            .iter()
            .find(|f| f.id == "colour-jump")
            .ok_or_else(|| anyhow::anyhow!("no jump: {got:?}"))?
            .clone();
        assert!(plan.set_accepted(&jump, true), "accepted");
        assert!(!plan.set_accepted(&jump, true), "once");
        let text = toml::to_string_pretty(&plan)?;
        assert!(
            text.contains("accepted = [\"colour-jump@2 B@15.00x4 C@15.00x4\"]"),
            "stored in the plan by id, cut and shots: {text}"
        );
        let back: Plan = toml::from_str(&text)?;
        assert!(back.is_accepted(&jump), "round trip");
        let report = crate::plan::report(&back, &got);
        assert!(
            report.contains("(accepted)")
                && report.trim_end().ends_with("ok (3 shots), 1 accepted"),
            "no longer counted: {report}"
        );
        let (map, people, memory) = (map(), People::default(), Memory::default());
        let night = Night {
            index: &index,
            map: &map,
            people: &people,
            memory: &memory,
        };
        let (same, done) = fix_all(&back, night);
        assert!(done.is_empty() && same == back, "accepted is left alone");
        // Change the shot after the cut: the acceptance no longer applies.
        let mut moved = back.clone();
        moved.shots[2].t_in = 45.0;
        let again = findings(&moved, &index);
        assert!(
            again
                .iter()
                .any(|f| f.id == "colour-jump" && !moved.is_accepted(f)),
            "a changed cut is judged afresh: {again:?}"
        );
        let mut undo = back;
        assert!(undo.set_accepted(&jump, false), "taken back");
        assert!(undo.accepted.is_empty(), "nothing left");
        let hard = Finding {
            severity: crate::plan::Severity::Hard,
            ..jump
        };
        assert!(
            !undo.set_accepted(&hard, true),
            "hard ones cannot be accepted"
        );
        Ok(())
    }
}

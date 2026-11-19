// ============================================================================
// Module:       analyze::moments
// Description:  The moment detector: a per-sample "burst" signal (onset of
//               flame / LED area and moving light) and the moments it peaks
//               at, with sub-sample onset times.
//
// Dependencies: none
// ============================================================================

//! Moments: where the fire flares.
//!
//! A fire or LED performance has accents: a fireball, a staff lit, poi
//! swung toward the lens. Each shows as a fast *rise* in how much of the
//! frame is fire, LED or moving light. The burst of sample `i` is
//!
//! ```text
//! A(i)     = flame + led + moving          (fractions of the frame)
//! burst(i) = max(0, A(i) - min(A(i-1), A(i-2)))
//! ```
//!
//! a rise over the last ~0.33 s at the 6 Hz analysis rate. A *moment* is a
//! local maximum of the burst that is at least [`BURST_MIN`] of the frame
//! and at least doubles the level it rose from, at least [`SPACING`]
//! samples from a stronger one. Its time is refined between samples to
//! where `A` crossed halfway from the old level to the new one, so the
//! planner can put the flare on a cut to within a frame of the estimate.
//!
//! Samples on a camera cut or whip (colour change above [`CUT_CHANGE`])
//! never burst: a new shot is not a flare.

use super::Sample;

/// Smallest burst that counts as a moment, fraction of the frame.
pub(crate) const BURST_MIN: f32 = 0.004;

/// Colour change above which a sample is a cut, not a flare.
const CUT_CHANGE: f32 = 0.35;

/// Minimum distance between two moments, samples (0.5 s at 6 Hz).
const SPACING: usize = 3;

/// The activity a burst is a rise of.
fn activity(s: &Sample) -> f32 {
    s.flame + s.led + s.moving
}

/// Fills `burst` on every sample.
pub(crate) fn bursts(samples: &mut [Sample]) {
    let a: Vec<f32> = samples.iter().map(activity).collect();
    for i in 0..samples.len() {
        let base = if i < 2 { a[0] } else { a[i - 1].min(a[i - 2]) };
        let cut = samples[i].change > CUT_CHANGE || (i > 0 && samples[i - 1].change > CUT_CHANGE);
        samples[i].burst = if cut { 0.0 } else { (a[i] - base).max(0.0) };
    }
}

/// One detected moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Moment {
    /// Refined onset time, seconds.
    pub(crate) t: f32,
    /// Burst size, fraction of the frame.
    pub(crate) burst: f32,
}

/// Moments among `samples` (already carrying `burst`), strongest first.
pub(crate) fn moments(samples: &[Sample]) -> Vec<Moment> {
    let mut picks: Vec<usize> = Vec::new();
    let mut order: Vec<usize> = (0..samples.len())
        .filter(|&i| {
            let s = &samples[i];
            s.burst >= BURST_MIN && s.burst >= 0.5 * activity(s)
        })
        .collect();
    order.sort_by(|&x, &y| samples[y].burst.total_cmp(&samples[x].burst));
    for i in order {
        if picks.iter().all(|&p| p.abs_diff(i) >= SPACING) {
            picks.push(i);
        }
    }
    picks
        .into_iter()
        .map(|i| Moment {
            t: onset(samples, i),
            burst: samples[i].burst,
        })
        .collect()
}

/// Time between samples where activity crossed halfway up the rise that
/// peaks at sample `i`.
fn onset(samples: &[Sample], i: usize) -> f32 {
    let a = |k: usize| activity(&samples[k]);
    let lo = i.saturating_sub(2);
    let base = (lo..i).map(a).fold(a(i), f32::min);
    let half = base + 0.5 * (a(i) - base);
    for k in (lo..i).rev() {
        let (before, after) = (a(k), a(k + 1));
        if before <= half && after >= half {
            let span = after - before;
            let f = if span > f32::EPSILON {
                (half - before) / span
            } else {
                1.0
            };
            return samples[k].t + f * (samples[k + 1].t - samples[k].t);
        }
    }
    samples[i].t
}

#[cfg(test)]
mod tests {
    use super::{BURST_MIN, bursts, moments};
    use crate::analyze::Sample;

    /// Six seconds at 6 Hz with a fireball rising between 2.0 and 2.33 s.
    fn fireball() -> Vec<Sample> {
        (0..36)
            .map(|i| {
                let t = i as f32 / 6.0;
                Sample {
                    t,
                    flame: if i >= 14 { 0.05 } else { 0.002 },
                    ..Sample::default()
                }
            })
            .collect()
    }

    #[test]
    fn a_fireball_is_one_moment_at_its_onset() -> anyhow::Result<()> {
        let mut s = fireball();
        bursts(&mut s);
        assert!(s[14].burst > 0.04, "the rise bursts: {}", s[14].burst);
        assert!(s[20].burst < 1e-6, "a steady flame does not");
        let m = moments(&s);
        assert_eq!(m.len(), 1, "one moment: {m:?}");
        // The level jumps between sample 13 (2.167 s) and 14 (2.333 s);
        // halfway up is halfway between.
        assert!(
            (m[0].t - 2.25).abs() < 0.01,
            "onset between the samples: {}",
            m[0].t
        );
        Ok(())
    }

    #[test]
    fn a_cut_is_not_a_burst() -> anyhow::Result<()> {
        let mut s = fireball();
        s[14].change = 0.6;
        bursts(&mut s);
        assert!(
            moments(&s).iter().all(|m| m.burst < BURST_MIN || m.t > 2.6),
            "the rise coincides with a cut"
        );
        Ok(())
    }

    #[test]
    fn flicker_on_a_big_flame_is_not_a_moment() -> anyhow::Result<()> {
        let mut s: Vec<Sample> = (0..36)
            .map(|i| Sample {
                t: i as f32 / 6.0,
                flame: if i % 2 == 0 { 0.10 } else { 0.094 },
                ..Sample::default()
            })
            .collect();
        bursts(&mut s);
        assert!(
            moments(&s).is_empty(),
            "a 6% wobble on a big flame is not a flare"
        );
        Ok(())
    }
}

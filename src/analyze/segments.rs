// ============================================================================
// Module:       analyze::segments
// Description:  Splits a clip's samples into segments of one camera behaviour
//               (static, pan, zoom, whip, shake) between cuts, and summarises
//               each one.
//
// Dependencies: serde
// ============================================================================

//! Segmentation.
//!
//! A segment is the unit a planner picks from: a stretch of one clip with
//! one camera behaviour and no cut inside it, at most [`MAX_LEN`] seconds
//! long. Its summary numbers are what the shot log prints.

use super::Sample;

/// Longest segment, seconds. Longer runs are split evenly.
const MAX_LEN: f32 = 6.0;

/// Shortest run of one camera class that survives smoothing, in samples.
const MIN_RUN: usize = 4;

/// What the camera is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Camera {
    /// Locked off or near enough.
    Static,
    /// A steady horizontal or vertical move.
    Pan,
    /// Optical zoom in (or a push in).
    ZoomIn,
    /// Optical zoom out (or a pull back).
    ZoomOut,
    /// Too fast to read: a whip pan or a blur.
    Whip,
    /// Handheld jitter without a steady direction.
    Shaky,
    /// Too dark or featureless to tell.
    Unknown,
}

impl Camera {
    /// The label the shot log prints.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Pan => "pan",
            Self::ZoomIn => "zoom-in",
            Self::ZoomOut => "zoom-out",
            Self::Whip => "whip",
            Self::Shaky => "shaky",
            Self::Unknown => "unknown",
        }
    }
}

/// A summarised stretch of a clip.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Segment {
    /// Start, seconds.
    pub(crate) start: f32,
    /// End, seconds.
    pub(crate) end: f32,
    /// Dominant camera behaviour.
    pub(crate) camera: Camera,
    /// Whether the segment starts at a detected cut.
    pub(crate) after_cut: bool,
    /// Mean luma.
    pub(crate) luma: f32,
    /// Mean 95th-percentile luma.
    pub(crate) p95: f32,
    /// Mean clipped fraction.
    pub(crate) clipped: f32,
    /// Median focus measure.
    pub(crate) sharp: f32,
    /// Mean flame fraction.
    pub(crate) flame: f32,
    /// Mean LED fraction.
    pub(crate) led: f32,
    /// Mean white-light fraction.
    pub(crate) white: f32,
    /// Mean moving-light fraction.
    pub(crate) moving: f32,
    /// Median horizontal centre of the action.
    pub(crate) cx: f32,
    /// How far that centre wanders (10th to 90th percentile).
    pub(crate) cx_spread: f32,
    /// Median vertical centre of the action.
    pub(crate) cy: f32,
    /// Total zoom over the segment as a factor (1.6 = zoomed in 1.6x).
    pub(crate) zoom_factor: f32,
    /// Total horizontal pan, frame widths.
    pub(crate) pan: f32,
    /// Handheld jitter: spread of the pan speed around its local mean.
    pub(crate) jitter: f32,
}

/// Median of a slice (0 when empty).
pub(crate) fn median(values: &[f32]) -> f32 {
    percentile(values, 0.5)
}

/// Linear-interpolated percentile of a slice (0 when empty).
pub(crate) fn percentile(values: &[f32], p: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    let pos = p.clamp(0.0, 1.0) * (v.len() - 1) as f32;
    let lo = pos.floor() as usize;
    let hi = pos.ceil() as usize;
    v[lo] + (v[hi] - v[lo]) * (pos - lo as f32)
}

/// Smoothed value of `f` over samples `i-r..=i+r`.
fn local_mean(samples: &[Sample], i: usize, r: usize, f: impl Fn(&Sample) -> f32) -> f32 {
    let lo = i.saturating_sub(r);
    let hi = (i + r + 1).min(samples.len());
    let n = (hi - lo).max(1) as f32;
    samples[lo..hi].iter().map(&f).sum::<f32>() / n
}

/// Classifies every sample's camera behaviour.
fn classify(samples: &[Sample]) -> (Vec<Camera>, Vec<f32>) {
    let mut out = Vec::with_capacity(samples.len());
    let mut jitter = Vec::with_capacity(samples.len());
    for i in 0..samples.len() {
        let px = local_mean(samples, i, 1, |s| s.pan_x);
        let py = local_mean(samples, i, 1, |s| s.pan_y);
        let zoom = local_mean(samples, i, 1, |s| s.zoom);
        let ok = local_mean(samples, i, 1, |s| f32::from(u8::from(s.motion_ok)));
        let change = local_mean(samples, i, 1, |s| s.change);
        // Jitter: how much the raw pan differs from its 1 s mean.
        let wide_x = local_mean(samples, i, 3, |s| s.pan_x);
        let wide_y = local_mean(samples, i, 3, |s| s.pan_y);
        let lo = i.saturating_sub(3);
        let hi = (i + 4).min(samples.len());
        let j = samples[lo..hi]
            .iter()
            .map(|s| (s.pan_x - wide_x).powi(2) + (s.pan_y - wide_y).powi(2))
            .sum::<f32>()
            / (hi - lo).max(1) as f32;
        let j = j.sqrt();
        jitter.push(j);
        let speed = px.hypot(py);
        let class = if speed > 0.9 || (ok < 0.5 && change > 0.25) {
            Camera::Whip
        } else if ok < 0.5 {
            Camera::Unknown
        } else if zoom > 0.10 {
            Camera::ZoomIn
        } else if zoom < -0.10 {
            Camera::ZoomOut
        } else if j > 0.22 {
            Camera::Shaky
        } else if speed > 0.07 {
            Camera::Pan
        } else {
            Camera::Static
        };
        out.push(class);
    }
    (out, jitter)
}

/// Replaces runs shorter than [`MIN_RUN`] with the longer neighbour's class.
fn smooth(classes: &mut [Camera]) {
    let mut changed = true;
    let mut guard = 0;
    while changed && guard < 8 {
        changed = false;
        guard += 1;
        let mut runs: Vec<(usize, usize, Camera)> = Vec::new();
        for (i, c) in classes.iter().enumerate() {
            match runs.last_mut() {
                Some((_, end, class)) if class == c => *end = i + 1,
                Some(_) | None => runs.push((i, i + 1, *c)),
            }
        }
        for k in 0..runs.len() {
            let (s, e, _) = runs[k];
            if e - s >= MIN_RUN || runs.len() == 1 {
                continue;
            }
            let left = k.checked_sub(1).map(|j| runs[j]);
            let right = runs.get(k + 1).copied();
            let pick = match (left, right) {
                (Some(l), Some(r)) => Some(if l.1 - l.0 >= r.1 - r.0 { l.2 } else { r.2 }),
                (Some(l), None) => Some(l.2),
                (None, Some(r)) => Some(r.2),
                (None, None) => None,
            };
            if let Some(class) = pick {
                for c in &mut classes[s..e] {
                    *c = class;
                }
                changed = true;
                break;
            }
        }
    }
}

/// Gives short `Unknown` runs (under 2 s: a dark moment, a flare) the class
/// of the run before them, so a brief loss of texture does not split a
/// steady shot in two.
fn absorb_unknown(classes: &mut [Camera]) {
    const LONG: usize = 12;
    let mut i = 0;
    while i < classes.len() {
        if classes[i] != Camera::Unknown {
            i += 1;
            continue;
        }
        let start = i;
        while i < classes.len() && classes[i] == Camera::Unknown {
            i += 1;
        }
        if i - start >= LONG {
            continue;
        }
        let fill = start
            .checked_sub(1)
            .map(|p| classes[p])
            .or_else(|| classes.get(i).copied());
        if let Some(fill) = fill {
            for c in &mut classes[start..i] {
                *c = fill;
            }
        }
    }
}

/// Indices where a cut happens (the sample after the cut).
fn cuts(samples: &[Sample]) -> Vec<usize> {
    let mut out = Vec::new();
    for i in 1..samples.len() {
        let c = samples[i].change;
        let lo = i.saturating_sub(3);
        let hi = (i + 4).min(samples.len());
        let around: Vec<f32> = (lo..hi)
            .filter(|&k| k != i)
            .map(|k| samples[k].change)
            .collect();
        if c > 0.35 && c > 3.0 * median(&around).max(0.02) {
            out.push(i);
        }
    }
    out
}

/// Summarises samples `lo..hi` as a segment.
fn summarise(
    samples: &[Sample],
    jitter: &[f32],
    lo: usize,
    hi: usize,
    camera: Camera,
    after_cut: bool,
) -> Segment {
    let s = &samples[lo..hi];
    let n = s.len().max(1) as f32;
    let mean = |f: fn(&Sample) -> f32| s.iter().map(f).sum::<f32>() / n;
    let cxs: Vec<f32> = s.iter().map(|x| x.cx).collect();
    let dt = if samples.len() > 1 {
        samples[1].t - samples[0].t
    } else {
        1.0 / 6.0
    };
    let zoom_ln: f32 = s.iter().map(|x| x.zoom * dt).sum();
    Segment {
        start: s.first().map_or(0.0, |x| x.t),
        end: s.last().map_or(0.0, |x| x.t) + dt,
        camera,
        after_cut,
        luma: mean(|x| x.luma),
        p95: mean(|x| x.p95),
        clipped: mean(|x| x.clipped),
        sharp: median(&s.iter().map(|x| x.sharp).collect::<Vec<_>>()),
        flame: mean(|x| x.flame),
        led: mean(|x| x.led),
        white: mean(|x| x.white),
        moving: mean(|x| x.moving),
        cx: median(&cxs),
        cx_spread: percentile(&cxs, 0.9) - percentile(&cxs, 0.1),
        cy: median(&s.iter().map(|x| x.cy).collect::<Vec<_>>()),
        zoom_factor: zoom_ln.exp(),
        pan: s.iter().map(|x| x.pan_x * dt).sum(),
        jitter: median(&jitter[lo..hi]),
    }
}

/// Splits samples into segments.
pub(crate) fn segment(samples: &[Sample]) -> Vec<Segment> {
    if samples.is_empty() {
        return Vec::new();
    }
    let (mut classes, jitter) = classify(samples);
    absorb_unknown(&mut classes);
    smooth(&mut classes);
    let cut_at = cuts(samples);
    let dt = if samples.len() > 1 {
        samples[1].t - samples[0].t
    } else {
        1.0 / 6.0
    };
    let max_n = ((MAX_LEN / dt) as usize).max(1);

    // Boundaries: cuts and camera-class changes.
    let mut bounds = vec![0];
    for i in 1..samples.len() {
        if cut_at.contains(&i) || classes[i] != classes[i - 1] {
            bounds.push(i);
        }
    }
    bounds.push(samples.len());
    bounds.dedup();

    let mut out = Vec::new();
    for pair in bounds.windows(2) {
        let (lo, hi) = (pair[0], pair[1]);
        let parts = (hi - lo).div_ceil(max_n).max(1);
        let len = (hi - lo) as f32 / parts as f32;
        for p in 0..parts {
            let a = lo + (p as f32 * len).round() as usize;
            let b = (lo + ((p + 1) as f32 * len).round() as usize).min(hi);
            if b > a {
                let after_cut = p == 0 && cut_at.contains(&lo);
                out.push(summarise(samples, &jitter, a, b, classes[a], after_cut));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Camera, percentile, segment};
    use crate::analyze::Sample;

    fn series(n: usize, f: impl Fn(usize) -> Sample) -> Vec<Sample> {
        (0..n).map(f).collect()
    }

    #[test]
    fn percentiles_interpolate() -> anyhow::Result<()> {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((percentile(&v, 0.5) - 3.0).abs() < 1e-6, "median");
        assert!((percentile(&v, 0.25) - 2.0).abs() < 1e-6, "quartile");
        assert!(percentile(&[], 0.5).abs() < 1e-6, "empty is zero");
        Ok(())
    }

    #[test]
    fn a_static_then_zoom_clip_splits_in_two() -> anyhow::Result<()> {
        let s = series(36, |i| Sample {
            t: i as f32 / 6.0,
            motion_ok: true,
            zoom: if i >= 18 { 0.3 } else { 0.0 },
            cx: 0.5,
            ..Sample::default()
        });
        let segs = segment(&s);
        assert_eq!(segs.len(), 2, "two segments: {segs:?}");
        assert_eq!(segs[0].camera, Camera::Static, "first static");
        assert_eq!(segs[1].camera, Camera::ZoomIn, "then zoom in");
        assert!(
            segs[1].zoom_factor > 1.5,
            "3 s at 0.3/s is ~2.4x, got {}",
            segs[1].zoom_factor
        );
        Ok(())
    }

    #[test]
    fn long_runs_are_capped() -> anyhow::Result<()> {
        let s = series(6 * 20, |i| Sample {
            t: i as f32 / 6.0,
            motion_ok: true,
            ..Sample::default()
        });
        let segs = segment(&s);
        assert!(
            segs.iter().all(|g| g.end - g.start <= 6.01),
            "all <= 6 s: {segs:?}"
        );
        assert_eq!(segs.len(), 4, "20 s splits into 4");
        Ok(())
    }
}

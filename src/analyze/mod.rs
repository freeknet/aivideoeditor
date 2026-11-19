// ============================================================================
// Module:       analyze
// Description:  Turns one video file into a time series of samples (exposure,
//               focus, lights, camera motion) and then into segments.
//
// Dependencies: crate::media, serde
// ============================================================================

//! Per-clip analysis.
//!
//! [`analyze_clip`] decodes a clip at [`SAMPLE_FPS`] and records one
//! [`Sample`] per decoded frame, then hands the series to
//! [`segments::segment`]. Everything a planner needs to reason about a clip
//! without watching it ends up in the returned [`ClipIndex`].

pub(crate) mod features;
pub(crate) mod moments;
pub(crate) mod motion;
pub(crate) mod segments;

use std::path::Path;

use anyhow::Result;

use crate::media::{self, FrameReader, Probe};

/// Analysis sample rate, frames per second.
pub(crate) const SAMPLE_FPS: f64 = 6.0;

/// Width frames are decoded at for analysis.
const DECODE_W: usize = 480;

/// How many samples back the moving-light comparison looks (0.5 s).
const FLUX_LAG: usize = 3;

/// Version of the per-sample measurements. A cached clip analysed by an
/// older version is analysed again, since its samples lack fields the
/// scoring now reads (lit region, mid-tones, bursts, signatures).
pub(crate) const ANALYSIS_VERSION: u32 = 2;

/// One analysis sample.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Sample {
    /// Time in seconds from the start of the clip.
    pub(crate) t: f32,
    /// Mean luma.
    pub(crate) luma: f32,
    /// 95th-percentile luma.
    pub(crate) p95: f32,
    /// Clipped fraction.
    pub(crate) clipped: f32,
    /// Focus measure.
    pub(crate) sharp: f32,
    /// Flame emitter fraction.
    pub(crate) flame: f32,
    /// LED emitter fraction.
    pub(crate) led: f32,
    /// White emitter fraction.
    pub(crate) white: f32,
    /// Fraction of the frame that is light which moved in the last 0.5 s
    /// after cancelling camera motion: spinning props, not fixtures.
    pub(crate) moving: f32,
    /// Horizontal centre of the moving light (0..1), or of all light when
    /// nothing moved; 0.5 when there is no light.
    pub(crate) cx: f32,
    /// Vertical centre, as `cx`.
    pub(crate) cy: f32,
    /// Pan speed, frame widths per second (positive: content moves right).
    pub(crate) pan_x: f32,
    /// Tilt speed, frame heights per second.
    pub(crate) pan_y: f32,
    /// Zoom rate, natural-log units per second (positive: zooming in).
    pub(crate) zoom: f32,
    /// Whether the motion estimate had enough texture to trust.
    pub(crate) motion_ok: bool,
    /// Colour change from the previous sample, 0..1.
    pub(crate) change: f32,
    /// Mean luma of the lit region (pixels above
    /// [`features::LIT_FLOOR`]): the night exposure measure.
    #[serde(default)]
    pub(crate) lit: f32,
    /// Fraction of the frame that is lit.
    #[serde(default)]
    pub(crate) lit_frac: f32,
    /// 95th-percentile luma of the lit region.
    #[serde(default)]
    pub(crate) lit_p95: f32,
    /// Mean red of the mid-tones (not black, not highlight, not emitter).
    #[serde(default)]
    pub(crate) mid_r: f32,
    /// Mean green of the mid-tones.
    #[serde(default)]
    pub(crate) mid_g: f32,
    /// Mean blue of the mid-tones.
    #[serde(default)]
    pub(crate) mid_b: f32,
    /// Fraction of the frame in the mid-tones.
    #[serde(default)]
    pub(crate) mid_frac: f32,
    /// Onset of fire / LED area and moving light over the last ~0.3 s,
    /// as a fraction of the frame (see [`moments`]).
    #[serde(default)]
    pub(crate) burst: f32,
}

/// Everything known about one clip.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ClipIndex {
    /// Short id, the file stem (e.g. `MVI_7987`).
    pub(crate) id: String,
    /// Absolute path of the source.
    pub(crate) path: String,
    /// Size in bytes and modification time, to invalidate the cache.
    pub(crate) stamp: (u64, u64),
    /// ffprobe result.
    pub(crate) probe: Probe,
    /// Analysis samples at [`SAMPLE_FPS`].
    pub(crate) samples: Vec<Sample>,
    /// Segments derived from the samples.
    pub(crate) segments: Vec<segments::Segment>,
    /// Coarse colour signature of the whole clip.
    pub(crate) hist: Vec<f32>,
    /// [`ANALYSIS_VERSION`] the samples were measured with (0: before
    /// versioning).
    #[serde(default)]
    pub(crate) version: u32,
}

/// Shifts a binary mask by whole pixels, dilating by one so small jitter
/// does not count as movement.
fn shifted_dilated(mask: &[u8], w: usize, h: usize, dx: i32, dy: i32) -> Vec<u8> {
    let mut out = vec![0_u8; mask.len()];
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] == 0 {
                continue;
            }
            for oy in -1..=1 {
                for ox in -1..=1 {
                    let nx = x as i32 + dx + ox;
                    let ny = y as i32 + dy + oy;
                    if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                        out[ny as usize * w + nx as usize] = 1;
                    }
                }
            }
        }
    }
    out
}

/// Weighted centre of the set pixels of `mask`, or `None` if empty.
fn centroid(mask: &[u8], w: usize, h: usize) -> Option<(f32, f32, usize)> {
    let (mut sx, mut sy, mut n) = (0.0_f64, 0.0_f64, 0_usize);
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] != 0 {
                sx += x as f64;
                sy += y as f64;
                n += 1;
            }
        }
    }
    (n > 0).then(|| {
        (
            (sx / n as f64 / w as f64) as f32,
            (sy / n as f64 / h as f64) as f32,
            n,
        )
    })
}

/// Analyses one clip.
pub(crate) fn analyze_clip(path: &Path, stamp: (u64, u64)) -> Result<ClipIndex> {
    let probe = media::probe(path)?;
    let mut reader = FrameReader::open(path, &probe, SAMPLE_FPS, DECODE_W)?;
    let dt = (1.0 / SAMPLE_FPS) as f32;

    let mut samples = Vec::new();
    let mut history: Vec<(Vec<u8>, f32, f32)> = Vec::new(); // (emitters, tx, ty) per sample
    let mut prev: Option<features::FrameFeatures> = None;
    let mut clip_hist = [0.0_f32; 32];
    // Per-sample colour histogram and emitter grid, for segment signatures.
    let mut hists: Vec<[f32; 32]> = Vec::new();
    let mut grids: Vec<[f32; 16]> = Vec::new();
    let mut i = 0_usize;
    while let Some(frame) = reader.next_frame()? {
        let f = features::measure(&frame);
        let (w, h) = (f.pw, f.ph);
        let (m, change) = match &prev {
            Some(p) => (
                motion::estimate(&p.plane, &f.plane, w, h, &p.emitters),
                features::hist_distance(&p.hist, &f.hist),
            ),
            None => (motion::Motion::default(), 0.0),
        };
        for (a, b) in clip_hist.iter_mut().zip(f.hist.iter()) {
            *a += b;
        }

        // Moving light: emitters now that were not there FLUX_LAG samples
        // ago, after moving the old mask along with the camera.
        let mut moving = 0.0;
        let mut centre = centroid(&f.emitters, w, h);
        if history.len() >= FLUX_LAG {
            let base = history.len() - FLUX_LAG;
            let (old, _, _) = &history[base];
            let (mut sx, mut sy) = (m.tx, m.ty);
            for (_, tx, ty) in &history[base + 1..] {
                sx += tx;
                sy += ty;
            }
            let old = shifted_dilated(old, w, h, sx.round() as i32, sy.round() as i32);
            let fresh: Vec<u8> = f
                .emitters
                .iter()
                .zip(&old)
                .map(|(now, before)| u8::from(*now != 0 && *before == 0))
                .collect();
            if let Some((x, y, n)) = centroid(&fresh, w, h) {
                moving = n as f32 / (w * h) as f32;
                if n >= 4 {
                    centre = Some((x, y, n));
                }
            }
        }
        let (cx, cy) = centre.map_or((0.5, 0.5), |(x, y, _)| (x, y));

        samples.push(Sample {
            t: i as f32 * dt,
            luma: f.luma,
            p95: f.p95,
            clipped: f.clipped,
            sharp: f.sharp,
            flame: f.flame,
            led: f.led,
            white: f.white,
            moving,
            cx,
            cy,
            pan_x: if m.reliable {
                m.tx / w as f32 / dt
            } else {
                0.0
            },
            pan_y: if m.reliable {
                m.ty / h as f32 / dt
            } else {
                0.0
            },
            zoom: if m.reliable {
                (1.0 + m.zoom).max(0.5).ln() / dt
            } else {
                0.0
            },
            motion_ok: m.reliable || prev.is_none(),
            change,
            lit: f.lit,
            lit_frac: f.lit_frac,
            lit_p95: f.lit_p95,
            mid_r: f.mid[0],
            mid_g: f.mid[1],
            mid_b: f.mid[2],
            mid_frac: f.mid_frac,
            burst: 0.0,
        });
        hists.push(f.hist);
        grids.push(f.grid);
        history.push((
            f.emitters.clone(),
            if m.reliable { m.tx } else { 0.0 },
            if m.reliable { m.ty } else { 0.0 },
        ));
        if history.len() > FLUX_LAG + 1 {
            history.remove(0);
        }
        prev = Some(f);
        i += 1;
    }
    let n = samples.len().max(1) as f32;
    let hist = clip_hist.iter().map(|v| v / n).collect();
    moments::bursts(&mut samples);
    let mut segments = segments::segment(&samples);
    segments::sign(&mut segments, &samples, &hists, &grids);
    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(ClipIndex {
        id,
        path: path.display().to_string(),
        stamp,
        probe,
        samples,
        segments,
        hist,
        version: ANALYSIS_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::{centroid, shifted_dilated};

    #[test]
    fn a_shifted_mask_lands_where_expected() -> anyhow::Result<()> {
        let (w, h) = (10, 10);
        let mut mask = vec![0_u8; w * h];
        mask[5 * w + 5] = 1;
        let moved = shifted_dilated(&mask, w, h, 2, 0);
        assert_eq!(moved[5 * w + 7], 1, "centre moved by +2 in x");
        assert_eq!(moved[5 * w + 5], 0, "old spot is clear");
        let (x, y, n) = centroid(&mask, w, h).ok_or_else(|| anyhow::anyhow!("empty"))?;
        assert!(
            (x - 0.5).abs() < 1e-6 && (y - 0.5).abs() < 1e-6 && n == 1,
            "centroid of one pixel"
        );
        Ok(())
    }
}

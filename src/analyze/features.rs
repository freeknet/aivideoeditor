// ============================================================================
// Module:       analyze::features
// Description:  Per-frame measurements: exposure, focus, light emitters
//               (fire / LED / white), colour signature, and a luma plane for
//               motion estimation.
//
// Dependencies: crate::media (Frame)
// ============================================================================

//! What one decoded frame says on its own.
//!
//! These are deliberately cheap, explainable numbers rather than a learned
//! model: a planner (human or LLM) has to be able to read "luma 0.04, sharp
//! 0.2" and know the shot is too dark and soft without seeing it.
//!
//! **Emitters.** A pixel counts as a light source when it is bright *and*
//! clearly brighter than its neighbourhood (a 15 px box mean). That keeps
//! coloured wall washes (a whole pink room) from reading as LED props while
//! still catching flames, LED hoops and lamps. Emitters are then split by
//! hue into warm (fire), coloured (LED) and white.

use crate::media::Frame;

/// Width of the analysis luma plane, in pixels (the frame is downsampled to
/// this before any per-pixel work).
pub(crate) const PLANE_W: usize = 240;

/// Luma above which a pixel counts as part of the lit region. Night
/// footage is mostly black; exposure is judged on what is above this floor
/// (the performer, the fire, the lit ground), not on the frame average.
pub(crate) const LIT_FLOOR: f32 = 0.08;

/// Mid-tone band for colour matching: above the noise floor, below the
/// highlights, and never an emitter (flames would set the white balance).
const MID_BAND: (f32, f32) = (0.08, 0.7);

/// Columns and rows of the emitter layout grid.
pub(crate) const GRID: (usize, usize) = (4, 4);

/// Measurements for one frame.
#[derive(Clone, Debug, Default)]
pub(crate) struct FrameFeatures {
    /// Mean luma, 0..1.
    pub(crate) luma: f32,
    /// 95th-percentile luma, 0..1.
    pub(crate) p95: f32,
    /// Fraction of clipped pixels (luma > 0.97).
    pub(crate) clipped: f32,
    /// Focus: mean absolute Laplacian over pixels that are not near-black,
    /// scaled so ~1.0 is crisp detail at the analysis size.
    pub(crate) sharp: f32,
    /// Fraction of the frame that is warm emitter (flame).
    pub(crate) flame: f32,
    /// Fraction that is saturated non-warm emitter (LED / coloured light).
    pub(crate) led: f32,
    /// Fraction that is white emitter (lamps, bulbs, flame cores).
    pub(crate) white: f32,
    /// Fraction of the frame above [`LIT_FLOOR`].
    pub(crate) lit_frac: f32,
    /// Mean luma of the lit region (0 when nothing is lit).
    pub(crate) lit: f32,
    /// 95th-percentile luma of the lit region.
    pub(crate) lit_p95: f32,
    /// Mean RGB (0..1) of mid-tone, non-emitter pixels.
    pub(crate) mid: [f32; 3],
    /// Fraction of the frame those mid-tone pixels cover.
    pub(crate) mid_frac: f32,
    /// Emitter share of each [`GRID`] cell, row-major.
    pub(crate) grid: [f32; 16],
    /// Emitter mask over the analysis plane (1 = emitter).
    pub(crate) emitters: Vec<u8>,
    /// Coarse HSV histogram (8 hue x 2 sat x 2 val), normalised.
    pub(crate) hist: [f32; 32],
    /// Luma plane used for motion estimation and focus.
    pub(crate) plane: Vec<f32>,
    /// Width of [`Self::plane`].
    pub(crate) pw: usize,
    /// Height of [`Self::plane`].
    pub(crate) ph: usize,
}

/// Converts 8-bit RGB to HSV with hue in degrees and s, v in 0..1.
pub(crate) fn hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d <= f32::EPSILON {
        0.0
    } else if (max - r).abs() <= f32::EPSILON {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if (max - g).abs() <= f32::EPSILON {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max <= f32::EPSILON { 0.0 } else { d / max };
    (h, s, max)
}

/// Downsamples the frame to [`PLANE_W`] wide RGB by box averaging.
fn downsample(frame: &Frame) -> (Vec<[u8; 3]>, usize, usize) {
    let factor = (frame.width / PLANE_W).max(1);
    let w = frame.width / factor;
    let h = frame.height / factor;
    let mut out = Vec::with_capacity(w * h);
    let area = (factor * factor) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0_u32; 3];
            for dy in 0..factor {
                let row = (y * factor + dy) * frame.width;
                for dx in 0..factor {
                    let i = (row + x * factor + dx) * 3;
                    acc[0] += u32::from(frame.rgb[i]);
                    acc[1] += u32::from(frame.rgb[i + 1]);
                    acc[2] += u32::from(frame.rgb[i + 2]);
                }
            }
            out.push([
                (acc[0] / area) as u8,
                (acc[1] / area) as u8,
                (acc[2] / area) as u8,
            ]);
        }
    }
    (out, w, h)
}

/// Box mean of `plane` with radius `r`, via an integral image.
pub(crate) fn box_mean(plane: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let mut integral = vec![0.0_f64; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0.0_f64;
        for x in 0..w {
            row += f64::from(plane[y * w + x]);
            integral[(y + 1) * (w + 1) + x + 1] = integral[y * (w + 1) + x + 1] + row;
        }
    }
    let mut out = vec![0.0; w * h];
    for y in 0..h {
        let y0 = y.saturating_sub(r);
        let y1 = (y + r + 1).min(h);
        for x in 0..w {
            let x0 = x.saturating_sub(r);
            let x1 = (x + r + 1).min(w);
            let sum = integral[y1 * (w + 1) + x1]
                - integral[y0 * (w + 1) + x1]
                - integral[y1 * (w + 1) + x0]
                + integral[y0 * (w + 1) + x0];
            out[y * w + x] = (sum / ((y1 - y0) * (x1 - x0)) as f64) as f32;
        }
    }
    out
}

/// Measures one frame.
pub(crate) fn measure(frame: &Frame) -> FrameFeatures {
    let (px, w, h) = downsample(frame);
    let n = (w * h).max(1);
    let plane: Vec<f32> = px
        .iter()
        .map(|p| {
            (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])) / 255.0
        })
        .collect();
    let value: Vec<f32> = px
        .iter()
        .map(|p| f32::from(p[0].max(p[1]).max(p[2])) / 255.0)
        .collect();
    let local = box_mean(&value, w, h, 7);

    let mut hist = [0.0_f32; 32];
    let mut counts = [0_u32; 101];
    let mut lit_counts = [0_u32; 101];
    let (mut lit_n, mut lit_sum) = (0_u32, 0.0_f64);
    let (mut mid_n, mut mid_sum) = (0_u32, [0.0_f64; 3]);
    let mut grid = [0.0_f32; 16];
    let (mut flame, mut led, mut white, mut clipped) = (0_u32, 0_u32, 0_u32, 0_u32);
    let mut emitters = vec![0_u8; n];
    for (i, p) in px.iter().enumerate() {
        let (hue, s, v) = hsv(p[0], p[1], p[2]);
        let l = plane[i];
        counts[(l * 100.0).clamp(0.0, 100.0) as usize] += 1;
        if l > LIT_FLOOR {
            lit_n += 1;
            lit_sum += f64::from(l);
            lit_counts[(l * 100.0).clamp(0.0, 100.0) as usize] += 1;
        }
        if l > 0.97 {
            clipped += 1;
        }
        let hb = ((hue / 45.0) as usize).min(7);
        let sb = usize::from(s > 0.4);
        let vb = usize::from(v > 0.35);
        hist[hb * 4 + sb * 2 + vb] += 1.0;
        if v > 0.5 && v - local[i] > 0.16 {
            emitters[i] = 1;
            let cell = (i / w * GRID.1 / h.max(1)).min(GRID.1 - 1) * GRID.0
                + (i % w * GRID.0 / w.max(1)).min(GRID.0 - 1);
            grid[cell] += 1.0;
            if s < 0.25 {
                white += 1;
            } else if (5.0..=55.0).contains(&hue) {
                flame += 1;
            } else if s > 0.5 {
                led += 1;
            } else {
                white += 1;
            }
        } else if l > MID_BAND.0 && l < MID_BAND.1 {
            mid_n += 1;
            for (acc, c) in mid_sum.iter_mut().zip(p) {
                *acc += f64::from(*c) / 255.0;
            }
        }
    }
    for value in &mut hist {
        *value /= n as f32;
    }
    let cell_area = (n as f32 / (GRID.0 * GRID.1) as f32).max(1.0);
    for value in &mut grid {
        *value /= cell_area;
    }
    let p95 = percentile_of(&counts, n as u32, 0.95);
    let lit_p95 = if lit_n == 0 {
        0.0
    } else {
        percentile_of(&lit_counts, lit_n, 0.95)
    };
    let mid = if mid_n == 0 {
        [0.0; 3]
    } else {
        mid_sum.map(|s| (s / f64::from(mid_n)) as f32)
    };

    // Focus: Laplacian magnitude where there is something to see.
    let (mut lap, mut lit) = (0.0_f64, 0_u32);
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let c = plane[y * w + x];
            if c < 0.06 {
                continue;
            }
            let v = 4.0 * c
                - plane[y * w + x - 1]
                - plane[y * w + x + 1]
                - plane[(y - 1) * w + x]
                - plane[(y + 1) * w + x];
            lap += f64::from(v.abs());
            lit += 1;
        }
    }
    let sharp = if lit < (n as u32) / 50 {
        0.0
    } else {
        (lap / f64::from(lit) * 25.0) as f32
    };

    FrameFeatures {
        luma: plane.iter().sum::<f32>() / n as f32,
        p95,
        clipped: clipped as f32 / n as f32,
        sharp,
        flame: flame as f32 / n as f32,
        led: led as f32 / n as f32,
        white: white as f32 / n as f32,
        lit_frac: lit_n as f32 / n as f32,
        lit: if lit_n == 0 {
            0.0
        } else {
            (lit_sum / f64::from(lit_n)) as f32
        },
        lit_p95,
        mid,
        mid_frac: mid_n as f32 / n as f32,
        grid,
        emitters,
        hist,
        plane,
        pw: w,
        ph: h,
    }
}

/// Percentile `p` (0..1) of a 101-bin luma histogram holding `n` pixels.
fn percentile_of(counts: &[u32; 101], n: u32, p: f32) -> f32 {
    let mut seen = 0;
    for (bin, count) in counts.iter().enumerate() {
        seen += count;
        if seen as f32 >= p * n as f32 {
            return bin as f32 / 100.0;
        }
    }
    1.0
}

/// Histogram distance (half L1), 0 = identical, 1 = disjoint.
pub(crate) fn hist_distance(a: &[f32; 32], b: &[f32; 32]) -> f32 {
    0.5 * a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>()
}

#[cfg(test)]
mod tests {
    use super::{hist_distance, hsv, measure};
    use crate::media::Frame;

    #[test]
    fn hsv_of_primaries() -> anyhow::Result<()> {
        let (h, s, v) = hsv(255, 0, 0);
        assert!(
            h.abs() < 0.5 && (s - 1.0).abs() < 1e-3 && (v - 1.0).abs() < 1e-3,
            "red"
        );
        let (h, _, _) = hsv(0, 0, 255);
        assert!((h - 240.0).abs() < 0.5, "blue hue is 240, got {h}");
        Ok(())
    }

    #[test]
    fn a_small_orange_light_on_black_is_flame() -> anyhow::Result<()> {
        let (w, h) = (480, 270);
        let mut rgb = vec![0_u8; w * h * 3];
        for y in 120..150 {
            for x in 220..260 {
                let i = (y * w + x) * 3;
                rgb[i] = 255;
                rgb[i + 1] = 150;
                rgb[i + 2] = 30;
            }
        }
        let f = measure(&Frame {
            width: w,
            height: h,
            rgb,
        });
        assert!(f.flame > 0.005, "flame fraction {}", f.flame);
        assert!(f.led < 0.001, "no LED expected, got {}", f.led);
        assert!(f.luma < 0.05, "mostly black, got {}", f.luma);
        assert!(
            f.lit > 0.4 && f.lit_p95 > 0.5,
            "the lit region is well exposed: {} / {}",
            f.lit,
            f.lit_p95
        );
        assert!(
            f.lit_frac > 0.005 && f.lit_frac < 0.02,
            "a small lit region: {}",
            f.lit_frac
        );
        // The light sits around x 0.5, y 0.5: grid cells 5, 6, 9, 10.
        let centre: f32 = [5, 6, 9, 10].iter().map(|&c| f.grid[c]).sum();
        let total: f32 = f.grid.iter().sum();
        assert!(
            total > 0.0 && centre / total > 0.99,
            "emitters in the centre cells: {:?}",
            f.grid
        );
        Ok(())
    }

    #[test]
    fn mid_tones_skip_black_and_emitters() -> anyhow::Result<()> {
        let (w, h) = (480, 270);
        let mut rgb = vec![0_u8; w * h * 3];
        // A dim grey-blue ground on the bottom half, a flame on top.
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 3;
                if y > h / 2 {
                    rgb[i..i + 3].copy_from_slice(&[60, 70, 100]);
                } else if (60..90).contains(&y) && (200..240).contains(&x) {
                    rgb[i..i + 3].copy_from_slice(&[255, 150, 30]);
                }
            }
        }
        let f = measure(&Frame {
            width: w,
            height: h,
            rgb,
        });
        let want = [60.0 / 255.0, 70.0 / 255.0, 100.0 / 255.0];
        for (got, want) in f.mid.iter().zip(want) {
            assert!(
                (got - want).abs() < 0.02,
                "mid-tones are the ground only: {:?}",
                f.mid
            );
        }
        Ok(())
    }

    #[test]
    fn a_uniform_pink_wash_is_not_an_emitter() -> anyhow::Result<()> {
        let (w, h) = (480, 270);
        let rgb = [230_u8, 60, 180].repeat(w * h);
        let f = measure(&Frame {
            width: w,
            height: h,
            rgb,
        });
        assert!(
            f.led + f.flame + f.white < 0.001,
            "a wash has no local contrast"
        );
        assert!(
            hist_distance(&f.hist, &f.hist) < 1e-6,
            "self distance is zero"
        );
        Ok(())
    }
}

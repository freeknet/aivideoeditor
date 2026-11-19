// ============================================================================
// Module:       analyze::motion
// Description:  Global camera motion between two frames: translation and
//               zoom, from block matching on a two-level luma pyramid.
//
// Dependencies: none
// ============================================================================

//! Camera motion between consecutive samples.
//!
//! A grid of textured blocks is matched from the previous frame into the
//! current one (coarse search on a half-size plane, refined at full size).
//! The block vectors are fitted to `v = a·q + t`, where `q` is the block's
//! offset from the frame centre: `t` is the pan, `1 + a` the zoom factor.
//! Outliers (performers moving, flames flickering) are dropped and the fit
//! repeated, so a fire spinner in the middle of a static shot does not
//! read as a camera move.
//!
//! Night footage is mostly black, and black has no texture to match. When
//! too few blocks carry texture the result is marked unreliable rather
//! than guessed.

/// Motion between two frames, in analysis-plane pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Motion {
    /// Horizontal translation (positive: content moved right, camera panned left).
    pub(crate) tx: f32,
    /// Vertical translation.
    pub(crate) ty: f32,
    /// Zoom factor minus one (positive: zooming in).
    pub(crate) zoom: f32,
    /// Fraction of matched blocks that agreed with the fit, 0..1.
    pub(crate) agreement: f32,
    /// Whether enough texture existed to trust the estimate.
    pub(crate) reliable: bool,
    /// Mean best-match error of the blocks: high when nothing matched
    /// (a whip pan or a cut).
    pub(crate) residual: f32,
}

/// Halves a plane by 2x2 averaging.
fn half(plane: &[f32], w: usize, h: usize) -> (Vec<f32>, usize, usize) {
    let (hw, hh) = (w / 2, h / 2);
    let mut out = Vec::with_capacity(hw * hh);
    for y in 0..hh {
        for x in 0..hw {
            let i = 2 * y * w + 2 * x;
            out.push(0.25 * (plane[i] + plane[i + 1] + plane[i + w] + plane[i + w + 1]));
        }
    }
    (out, hw, hh)
}

/// Sum of absolute differences of a `b`x`b` block at (x0,y0) in `a`
/// against (x0+dx, y0+dy) in `c`, or `None` if it leaves the frame.
#[expect(
    clippy::too_many_arguments,
    reason = "a hot inner loop; a struct would only add noise"
)]
fn sad(
    a: &[f32],
    c: &[f32],
    w: usize,
    h: usize,
    x0: usize,
    y0: usize,
    b: usize,
    dx: i32,
    dy: i32,
) -> Option<f32> {
    let xs = x0 as i32 + dx;
    let ys = y0 as i32 + dy;
    if xs < 0 || ys < 0 || xs as usize + b > w || ys as usize + b > h {
        return None;
    }
    let (xs, ys) = (xs as usize, ys as usize);
    let mut sum = 0.0;
    for y in 0..b {
        let ra = (y0 + y) * w + x0;
        let rc = (ys + y) * w + xs;
        for x in 0..b {
            sum += (a[ra + x] - c[rc + x]).abs();
        }
    }
    Some(sum / (b * b) as f32)
}

/// Best integer offset for one block within `±radius` around `(cx, cy)`.
#[expect(
    clippy::too_many_arguments,
    reason = "a hot inner loop; a struct would only add noise"
)]
fn search(
    a: &[f32],
    c: &[f32],
    w: usize,
    h: usize,
    x0: usize,
    y0: usize,
    b: usize,
    center: (i32, i32),
    radius: i32,
) -> Option<(i32, i32, f32)> {
    let mut best: Option<(i32, i32, f32)> = None;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let (ox, oy) = (center.0 + dx, center.1 + dy);
            if let Some(cost) = sad(a, c, w, h, x0, y0, b, ox, oy)
                && best.is_none_or(|(_, _, bc)| cost < bc)
            {
                best = Some((ox, oy, cost));
            }
        }
    }
    best
}

/// How many pixels of a block a mask covers (0 for an empty mask).
fn masked(mask: &[u8], w: usize, x0: usize, y0: usize, b: usize) -> usize {
    if mask.is_empty() {
        return 0;
    }
    (0..b)
        .map(|y| {
            mask[(y0 + y) * w + x0..(y0 + y) * w + x0 + b]
                .iter()
                .filter(|&&m| m != 0)
                .count()
        })
        .sum()
}

/// Standard deviation of a block, a cheap texture measure.
fn block_std(a: &[f32], w: usize, x0: usize, y0: usize, b: usize) -> f32 {
    let mut sum = 0.0;
    let mut sq = 0.0;
    for y in 0..b {
        for x in 0..b {
            let v = a[(y0 + y) * w + x0 + x];
            sum += v;
            sq += v * v;
        }
    }
    let n = (b * b) as f32;
    let mean = sum / n;
    (sq / n - mean * mean).max(0.0).sqrt()
}

/// A block centre offset `q` and its measured motion `v`.
type Pair = ((f32, f32), (f32, f32));

/// Least-squares fit of `v = a·q + t` over `(q, v)` pairs.
fn fit(pairs: &[Pair]) -> (f32, f32, f32) {
    let n = pairs.len().max(1) as f32;
    let (mut mqx, mut mqy, mut mvx, mut mvy) = (0.0, 0.0, 0.0, 0.0);
    for ((qx, qy), (vx, vy)) in pairs {
        mqx += qx;
        mqy += qy;
        mvx += vx;
        mvy += vy;
    }
    mqx /= n;
    mqy /= n;
    mvx /= n;
    mvy /= n;
    let (mut num, mut den) = (0.0, 0.0);
    for ((qx, qy), (vx, vy)) in pairs {
        let (dqx, dqy) = (qx - mqx, qy - mqy);
        num += dqx * (vx - mvx) + dqy * (vy - mvy);
        den += dqx * dqx + dqy * dqy;
    }
    let a = if den > 1e-6 { num / den } else { 0.0 };
    (a, mvx - a * mqx, mvy - a * mqy)
}

/// Estimates camera motion from `prev` to `cur` (same-size luma planes).
/// Blocks where `skip` (a 0/1 mask, or empty) covers more than a tenth of the
/// block are ignored: those are lights that move on their own.
pub(crate) fn estimate(prev: &[f32], cur: &[f32], w: usize, h: usize, skip: &[u8]) -> Motion {
    const BLOCK: usize = 12;
    const STEP: usize = 18;
    const MIN_TEXTURE: f32 = 0.012;
    let (p2, w2, h2) = half(prev, w, h);
    let (c2, _, _) = half(cur, w, h);
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);

    let mut pairs = Vec::new();
    let mut residual = 0.0;
    let mut y0 = STEP / 2;
    while y0 + BLOCK + STEP / 2 <= h {
        let mut x0 = STEP / 2;
        while x0 + BLOCK + STEP / 2 <= w {
            if block_std(prev, w, x0, y0, BLOCK) >= MIN_TEXTURE
                && masked(skip, w, x0, y0, BLOCK) * 10 <= BLOCK * BLOCK
            {
                // Coarse: half-size block at half-size coordinates.
                let coarse = search(&p2, &c2, w2, h2, x0 / 2, y0 / 2, BLOCK / 2, (0, 0), 7);
                // The coarse level can alias on fine texture, so also refine
                // around "no motion" and keep the better match.
                let guided = coarse.and_then(|(dx2, dy2, _)| {
                    search(prev, cur, w, h, x0, y0, BLOCK, (dx2 * 2, dy2 * 2), 2)
                });
                let local = search(prev, cur, w, h, x0, y0, BLOCK, (0, 0), 3);
                let best = match (guided, local) {
                    (Some(g), Some(l)) => Some(if l.2 <= g.2 { l } else { g }),
                    (Some(g), None) => Some(g),
                    (None, l) => l,
                };
                if let Some((dx, dy, cost)) = best {
                    let q = (
                        x0 as f32 + BLOCK as f32 / 2.0 - cx,
                        y0 as f32 + BLOCK as f32 / 2.0 - cy,
                    );
                    pairs.push((q, (dx as f32, dy as f32)));
                    residual += cost;
                }
            }
            x0 += STEP;
        }
        y0 += STEP;
    }
    let textured = pairs.len();
    if textured < 6 {
        return Motion {
            reliable: false,
            residual: if textured == 0 {
                0.0
            } else {
                residual / textured as f32
            },
            ..Motion::default()
        };
    }
    residual /= textured as f32;

    // Fit, drop blocks more than 1.5 px off, refit.
    let mut model = fit(&pairs);
    for _ in 0..2 {
        let (a, tx, ty) = model;
        let kept: Vec<_> = pairs
            .iter()
            .copied()
            .filter(|((qx, qy), (vx, vy))| {
                let ex = vx - (a * qx + tx);
                let ey = vy - (a * qy + ty);
                ex * ex + ey * ey <= 2.25
            })
            .collect();
        if kept.len() < 4 {
            break;
        }
        model = fit(&kept);
        if kept.len() == pairs.len() {
            break;
        }
        pairs = kept;
    }
    let (a, tx, ty) = model;
    let agree = pairs
        .iter()
        .filter(|((qx, qy), (vx, vy))| {
            let ex = vx - (a * qx + tx);
            let ey = vy - (a * qy + ty);
            ex * ex + ey * ey <= 2.25
        })
        .count();
    Motion {
        tx,
        ty,
        zoom: a,
        agreement: agree as f32 / textured as f32,
        reliable: agree * 4 >= textured,
        residual,
    }
}

#[cfg(test)]
mod tests {
    use super::estimate;

    /// A textured test plane: a smooth pattern plus noise-like detail.
    fn pattern(w: usize, h: usize, shift_x: f32, scale: f32) -> Vec<f32> {
        let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                // Sample the "world" at the point that lands on this pixel.
                let wx = (x as f32 - cx - shift_x) / scale + cx;
                let wy = (y as f32 - cy) / scale + cy;
                let v = 0.5
                    + 0.25 * (wx * 0.31).sin() * (wy * 0.23).cos()
                    + 0.2 * ((wx * 0.6 + wy * 0.35).sin() * (wx * 0.27 - wy * 0.45).cos());
                out.push(v.clamp(0.0, 1.0));
            }
        }
        out
    }

    #[test]
    fn a_pan_is_measured() -> anyhow::Result<()> {
        let (w, h) = (240, 135);
        let m = estimate(
            &pattern(w, h, 0.0, 1.0),
            &pattern(w, h, 5.0, 1.0),
            w,
            h,
            &[],
        );
        assert!(m.reliable, "textured planes are reliable: {m:?}");
        assert!((m.tx - 5.0).abs() < 0.8, "tx should be ~5, got {}", m.tx);
        assert!(m.zoom.abs() < 0.01, "no zoom, got {}", m.zoom);
        Ok(())
    }

    #[test]
    fn a_zoom_is_measured() -> anyhow::Result<()> {
        let (w, h) = (240, 135);
        let m = estimate(
            &pattern(w, h, 0.0, 1.0),
            &pattern(w, h, 0.0, 1.04),
            w,
            h,
            &[],
        );
        assert!(m.zoom > 0.02, "zoom-in should be positive, got {}", m.zoom);
        assert!(m.tx.abs() < 1.0, "no pan, got {}", m.tx);
        Ok(())
    }

    #[test]
    fn black_is_unreliable() -> anyhow::Result<()> {
        let (w, h) = (240, 135);
        let black = vec![0.0; w * h];
        assert!(
            !estimate(&black, &black, w, h, &[]).reliable,
            "no texture, no estimate"
        );
        Ok(())
    }
}

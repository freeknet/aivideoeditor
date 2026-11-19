// ============================================================================
// Module:       beats
// Description:  Music analysis: onset envelope, tempo, beat tracking, bars,
//               per-bar energy and drop candidates.
//
// Dependencies: crate::media, rustfft, serde_json
// ============================================================================

//! The music map an edit is cut to.
//!
//! - **Onsets:** spectral flux of the log-magnitude STFT (hop 512 at
//!   22.05 kHz, about 23 ms).
//! - **Tempo:** the onset envelope's autocorrelation, searched over
//!   90-180 BPM with a mild preference for 120-150 (bass music sits at
//!   140; halftime feels 70 but the grid is 140).
//! - **Beats:** dynamic-programming beat tracking (Ellis 2007): each beat
//!   is an onset peak about one period after the previous one.
//! - **Bars:** beats grouped in fours, with the phase that puts the most
//!   low-frequency energy (kicks) on the downbeat.
//! - **Drops:** the bars where low-end energy jumps the most from the four
//!   bars before to the four after.

use std::path::Path;

use anyhow::{Result, ensure};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use crate::media;

/// Analysis sample rate.
const RATE: u32 = 22_050;
/// STFT window.
const WIN: usize = 2048;
/// STFT hop.
const HOP: usize = 512;

/// One bar of the track.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Bar {
    /// Start time, seconds.
    pub(crate) t: f32,
    /// RMS loudness over the bar.
    pub(crate) rms: f32,
    /// Low-end (below ~150 Hz) energy, relative to the track's loudest bar.
    pub(crate) low: f32,
}

/// A drop candidate.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Drop {
    /// Bar index.
    pub(crate) bar: usize,
    /// Time, seconds.
    pub(crate) t: f32,
    /// How big the jump is (higher is stronger).
    pub(crate) score: f32,
}

/// The whole map.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct MusicMap {
    /// Source file.
    pub(crate) path: String,
    /// Duration, seconds.
    pub(crate) duration: f32,
    /// Tempo.
    pub(crate) bpm: f32,
    /// Beat times, seconds.
    pub(crate) beats: Vec<f32>,
    /// Bars (four beats each).
    pub(crate) bars: Vec<Bar>,
    /// Drop candidates, strongest first.
    pub(crate) drops: Vec<Drop>,
}

impl MusicMap {
    /// Loads `<work>/music.json`.
    pub(crate) fn load(work: &Path) -> Result<Self> {
        let path = work.join("music.json");
        let text = std::fs::read_to_string(&path).map_err(|e| {
            anyhow::anyhow!("reading {} (run `ave beats` first): {e}", path.display())
        })?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Time of beat `k` counted from bar `bar`'s downbeat, extrapolating
    /// past the last detected beat at the track tempo.
    pub(crate) fn beat_time(&self, bar: usize, k: usize) -> f32 {
        let period = 60.0 / self.bpm;
        let first = bar * 4;
        let i = first + k;
        match self.beats.get(i) {
            Some(t) => *t,
            None => {
                let last = self.beats.len().saturating_sub(1);
                self.beats.last().copied().unwrap_or(0.0) + (i - last) as f32 * period
            }
        }
    }
}

/// Spectral flux and low-band energy per STFT frame.
fn envelopes(samples: &[f32]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WIN);
    let window: Vec<f32> = (0..WIN)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / WIN as f32).cos())
        .collect();
    let low_bins = (150.0 * WIN as f32 / RATE as f32).ceil() as usize;
    let mut prev = vec![0.0_f32; WIN / 2];
    let (mut flux, mut low, mut rms) = (Vec::new(), Vec::new(), Vec::new());
    let mut buf = vec![Complex::new(0.0, 0.0); WIN];
    let mut start = 0;
    while start + WIN <= samples.len() {
        let frame = &samples[start..start + WIN];
        for (b, (s, w)) in buf.iter_mut().zip(frame.iter().zip(&window)) {
            *b = Complex::new(s * w, 0.0);
        }
        fft.process(&mut buf);
        let mut f = 0.0;
        let mut l = 0.0;
        for k in 1..WIN / 2 {
            let mag = buf[k].norm();
            let lm = (1.0 + 100.0 * mag).ln();
            f += (lm - prev[k]).max(0.0);
            prev[k] = lm;
            if k <= low_bins {
                l += mag * mag;
            }
        }
        flux.push(f);
        low.push(l);
        rms.push((frame.iter().map(|s| s * s).sum::<f32>() / WIN as f32).sqrt());
        start += HOP;
    }
    // Normalise and remove the local mean so the beat tracker sees peaks.
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let m = mean(&flux).max(1e-9);
    let flux: Vec<f32> = flux.iter().map(|f| f / m).collect();
    let smooth = crate::analyze::features::box_mean(&flux, flux.len(), 1, 8);
    let flux = flux
        .iter()
        .zip(&smooth)
        .map(|(f, s)| (f - s).max(0.0))
        .collect();
    (flux, low, rms)
}

/// Tempo in frames per beat, from the onset envelope.
fn tempo(env: &[f32], fps: f32) -> f32 {
    let lag_of = |bpm: f32| 60.0 * fps / bpm;
    let (lo, hi) = (lag_of(180.0).floor() as usize, lag_of(90.0).ceil() as usize);
    let mut best = (0.0, lag_of(120.0));
    for lag in lo.max(1)..=hi {
        let mut ac = 0.0;
        for i in lag..env.len() {
            ac += env[i] * env[i - lag];
        }
        ac /= (env.len() - lag).max(1) as f32;
        let bpm = 60.0 * fps / lag as f32;
        let prior = (-0.5 * ((bpm / 135.0).log2() / 0.6).powi(2)).exp();
        let score = ac * prior;
        if score > best.0 {
            best = (score, lag as f32);
        }
    }
    // Refine with a parabola through the neighbours.
    let lag = best.1 as usize;
    let ac = |l: usize| -> f32 {
        if l == 0 || l >= env.len() {
            return 0.0;
        }
        (l..env.len()).map(|i| env[i] * env[i - l]).sum::<f32>() / (env.len() - l) as f32
    };
    let (a, b, c) = (ac(lag.saturating_sub(1)), ac(lag), ac(lag + 1));
    let denom = a - 2.0 * b + c;
    if denom.abs() > 1e-9 {
        lag as f32 + 0.5 * (a - c) / denom
    } else {
        lag as f32
    }
}

/// Dynamic-programming beat tracker; returns beat frame indices.
fn track(env: &[f32], period: f32) -> Vec<usize> {
    const TIGHTNESS: f32 = 100.0;
    let n = env.len();
    let mut score = vec![0.0_f32; n];
    let mut back = vec![usize::MAX; n];
    for t in 0..n {
        let lo = (t as f32 - 2.0 * period).max(0.0) as usize;
        let hi = (t as f32 - 0.5 * period).max(0.0) as usize;
        let mut best = (0.0_f32, usize::MAX);
        for (p, prev) in score.iter().enumerate().take(hi.min(t)).skip(lo) {
            let d = ((t - p) as f32 / period).ln();
            let s = prev - TIGHTNESS * d * d;
            if best.1 == usize::MAX || s > best.0 {
                best = (s, p);
            }
        }
        score[t] = env[t] + if best.1 == usize::MAX { 0.0 } else { best.0 };
        back[t] = best.1;
    }
    // Start from the best-scoring frame in the last period.
    let tail = n.saturating_sub(period.ceil() as usize);
    let mut t = (tail..n)
        .max_by(|&a, &b| score[a].total_cmp(&score[b]))
        .unwrap_or(0);
    let mut beats = Vec::new();
    while t != usize::MAX {
        beats.push(t);
        t = back[t];
    }
    beats.reverse();
    beats
}

/// The shift (seconds, within ±0.25) that puts the beat grid on the kicks.
///
/// Spectral flux peaks a little after an onset, by an amount that depends
/// on the window and the sound, so the tracked grid can sit late. The fix
/// is measured rather than assumed: a low-passed (under ~120 Hz) envelope
/// at 128-sample resolution, its rises scored at every beat for each
/// candidate shift, the best shift kept.
fn kick_offset(samples: &[f32], beats: &[f32]) -> f32 {
    const ENV_HOP: usize = 128;
    // Two one-pole low-passes at ~120 Hz.
    let a = (-2.0 * std::f32::consts::PI * 120.0 / RATE as f32).exp();
    let (mut y1, mut y2) = (0.0_f32, 0.0_f32);
    let mut env = Vec::with_capacity(samples.len() / ENV_HOP + 1);
    let mut acc = 0.0;
    for (i, s) in samples.iter().enumerate() {
        y1 = (1.0 - a) * s + a * y1;
        y2 = (1.0 - a) * y1 + a * y2;
        acc += y2 * y2;
        if (i + 1) % ENV_HOP == 0 {
            env.push((acc / ENV_HOP as f32).sqrt());
            acc = 0.0;
        }
    }
    let rise: Vec<f32> = env
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if i == 0 {
                0.0
            } else {
                (v - env[i - 1]).max(0.0)
            }
        })
        .collect();
    let step = ENV_HOP as f32 / RATE as f32;
    let strength = |t: f32| -> f32 {
        let c = (t / step).round() as i64;
        (c - 3..=c + 3)
            .filter_map(|k| usize::try_from(k).ok().and_then(|k| rise.get(k)))
            .fold(0.0_f32, |m, v| m.max(*v))
    };
    let mut best = (f32::MIN, 0.0);
    let span = (0.25 / step) as i64;
    for k in -span..=span {
        let shift = k as f32 * step;
        let score: f32 = beats.iter().map(|&t| strength(t + shift)).sum();
        if score > best.0 {
            best = (score, shift);
        }
    }
    best.1
}

/// Analyses `path` into a music map.
pub(crate) fn analyze(path: &Path) -> Result<MusicMap> {
    let samples = media::audio_mono(path, RATE)?;
    ensure!(
        samples.len() > RATE as usize * 4,
        "{} is shorter than 4 seconds",
        path.display()
    );
    let fps = RATE as f32 / HOP as f32;
    let (env, low, rms) = envelopes(&samples);
    let period = tempo(&env, fps);
    let beat_frames = track(&env, period);
    let to_t = |f: usize| (f * HOP + WIN / 2) as f32 / RATE as f32;
    let shift = kick_offset(
        &samples,
        &beat_frames.iter().map(|&f| to_t(f)).collect::<Vec<_>>(),
    );
    log::debug!("beat grid shifted {shift:+.3}s onto the kicks");
    let beats: Vec<f32> = beat_frames
        .iter()
        .map(|&f| (to_t(f) + shift).max(0.0))
        .collect();
    let bpm = 60.0 * fps / period;

    // Downbeat phase: the one whose beats carry the most low end.
    let low_at = |f: usize| -> f32 {
        let lo = f.saturating_sub(2);
        let hi = (f + 3).min(low.len());
        low[lo..hi].iter().sum()
    };
    let phase = (0..4)
        .max_by(|&a, &b| {
            let s = |ph: usize| {
                beat_frames
                    .iter()
                    .skip(ph)
                    .step_by(4)
                    .map(|&f| low_at(f))
                    .sum::<f32>()
            };
            s(a).total_cmp(&s(b))
        })
        .unwrap_or(0);
    let beats: Vec<f32> = beats.into_iter().skip(phase).collect();
    let frames: Vec<usize> = beat_frames.into_iter().skip(phase).collect();

    let mut bars = Vec::new();
    for chunk in frames.chunks(4) {
        let (Some(&a), Some(&last)) = (chunk.first(), chunk.last()) else {
            continue;
        };
        let b = (last + period.round() as usize).min(rms.len());
        if b <= a {
            continue;
        }
        let r = rms[a..b].iter().map(|x| x * x).sum::<f32>() / (b - a) as f32;
        let l = low[a..b].iter().sum::<f32>() / (b - a) as f32;
        bars.push(Bar {
            t: to_t(a) + shift,
            rms: r.sqrt(),
            low: l,
        });
    }
    let max_low = bars.iter().map(|b| b.low).fold(1e-12_f32, f32::max);
    for b in &mut bars {
        b.low /= max_low;
    }

    let mut drops: Vec<Drop> = (4..bars.len().saturating_sub(3))
        .map(|i| {
            let before = bars[i - 4..i].iter().map(|b| b.low).sum::<f32>() / 4.0;
            let after = bars[i..i + 4].iter().map(|b| b.low).sum::<f32>() / 4.0;
            Drop {
                bar: i,
                t: bars[i].t,
                score: after - before,
            }
        })
        .filter(|d| d.score > 0.15)
        .collect();
    drops.sort_by(|a, b| b.score.total_cmp(&a.score));
    // Keep candidates at least 8 bars apart.
    let mut kept: Vec<Drop> = Vec::new();
    for d in drops {
        if kept.iter().all(|k| k.bar.abs_diff(d.bar) >= 8) {
            kept.push(d);
        }
    }

    Ok(MusicMap {
        path: path.display().to_string(),
        duration: samples.len() as f32 / RATE as f32,
        bpm,
        beats,
        bars,
        drops: kept,
    })
}

#[cfg(test)]
mod tests {
    use super::{tempo, track};

    #[test]
    fn a_click_track_is_found() -> anyhow::Result<()> {
        // 43 frames/s envelope, a click every 18 frames (~143 BPM).
        let fps = 43.07;
        let env: Vec<f32> = (0..2000)
            .map(|i| if i % 18 == 0 { 1.0 } else { 0.0 })
            .collect();
        let period = tempo(&env, fps);
        assert!(
            (period - 18.0).abs() < 0.6,
            "period ~18 frames, got {period}"
        );
        let beats = track(&env, period);
        let on_click = beats.iter().filter(|&&b| b % 18 == 0).count();
        assert!(
            on_click * 10 >= beats.len() * 9,
            "beats land on clicks: {on_click}/{}",
            beats.len()
        );
        Ok(())
    }
}

// ============================================================================
// Module:       gui::pages::plan::waveform
// Description:  Min/max peaks of a music file for the A1 track, decoded once
//               per file on a worker thread and cached.
//
// Dependencies: egui; crate::media (audio_mono)
// ============================================================================

//! The music waveform.
//!
//! The track is decoded to 8 kHz mono once (`media::audio_mono`, ffmpeg) on
//! its own thread and folded into min/max pairs every 5 ms. Drawing then
//! only reads the buckets under each visible pixel column, so zooming and
//! scrolling cost nothing however long the song is.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Decode rate.
const RATE: u32 = 8000;
/// Samples per bucket: 5 ms at 8 kHz.
const BUCKET: usize = 40;

/// Min/max pairs over fixed buckets.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Peaks {
    /// Buckets per second.
    pub(super) rate: f32,
    /// Lowest sample in each bucket.
    pub(super) min: Vec<f32>,
    /// Highest sample in each bucket.
    pub(super) max: Vec<f32>,
    /// Largest absolute sample, for normalising.
    pub(super) peak: f32,
}

impl Peaks {
    /// Folds `samples` (at `rate` Hz) into buckets of `bucket` samples.
    pub(super) fn from_samples(samples: &[f32], rate: u32, bucket: usize) -> Self {
        let bucket = bucket.max(1);
        let (mut min, mut max) = (Vec::new(), Vec::new());
        let mut peak = 0.0_f32;
        for chunk in samples.chunks(bucket) {
            let lo = chunk.iter().copied().fold(f32::MAX, f32::min);
            let hi = chunk.iter().copied().fold(f32::MIN, f32::max);
            peak = peak.max(lo.abs()).max(hi.abs());
            min.push(lo);
            max.push(hi);
        }
        Self {
            rate: rate as f32 / bucket as f32,
            min,
            max,
            peak,
        }
    }

    /// Lowest and highest sample between `t0` and `t1` seconds, normalised
    /// to the track's peak; `None` outside the track.
    pub(super) fn span(&self, t0: f32, t1: f32) -> Option<(f32, f32)> {
        let n = self.min.len();
        if n == 0 || t1 < 0.0 {
            return None;
        }
        let a = (t0.max(0.0) * self.rate) as usize;
        let b = ((t1 * self.rate).ceil() as usize).max(a + 1).min(n);
        if a >= n {
            return None;
        }
        let lo = self.min[a..b].iter().copied().fold(f32::MAX, f32::min);
        let hi = self.max[a..b].iter().copied().fold(f32::MIN, f32::max);
        let norm = self.peak.max(1e-6);
        Some((lo / norm, hi / norm))
    }

    /// The loudest sample between `t0` and `t1` seconds as a fraction of
    /// full scale (not of the track's peak), the larger of |min| and |max|,
    /// as Selects draws its bars: a column at 1.0 clipped. `None` outside
    /// the track.
    pub(super) fn level(&self, t0: f32, t1: f32) -> Option<f32> {
        let (lo, hi) = self.span(t0, t1)?;
        let norm = self.peak.max(1e-6);
        Some((lo.abs().max(hi.abs()) * norm).min(1.0))
    }
}

/// The cache, shared with the decode threads.
#[derive(Debug, Default)]
pub(super) struct Waves {
    files: Arc<Mutex<HashMap<PathBuf, Wave>>>,
}

/// What [`Waves::get`] knows about a file.
#[derive(Clone, Debug)]
pub(super) enum Wave {
    /// Still decoding.
    Loading,
    /// The peaks.
    Ready(Arc<Peaks>),
    /// Decoding failed.
    Failed(String),
}

impl Waves {
    /// The peaks of `path`, starting a decode the first time it is asked for.
    pub(super) fn get(&self, ctx: &egui::Context, path: &Path) -> Wave {
        let Ok(mut files) = self.files.lock() else {
            return Wave::Failed("waveform cache poisoned".to_owned());
        };
        if let Some(known) = files.get(path) {
            return known.clone();
        }
        files.insert(path.to_path_buf(), Wave::Loading);
        let shared = Arc::clone(&self.files);
        let path = path.to_path_buf();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("waveform".into())
            .spawn(move || {
                let entry = match crate::media::audio_mono(&path, RATE) {
                    Ok(samples) => {
                        Wave::Ready(Arc::new(Peaks::from_samples(&samples, RATE, BUCKET)))
                    }
                    Err(e) => Wave::Failed(format!("{e:#}")),
                };
                if let Ok(mut files) = shared.lock() {
                    files.insert(path, entry);
                }
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            return Wave::Failed(e.to_string());
        }
        Wave::Loading
    }
}

#[cfg(test)]
mod tests {
    use super::Peaks;

    #[test]
    fn peaks_fold_buckets_and_spans_normalise() -> anyhow::Result<()> {
        let samples: Vec<f32> = (0..80).map(|i| if i < 40 { 0.25 } else { -0.5 }).collect();
        let p = Peaks::from_samples(&samples, 40, 10);
        assert_eq!(p.min.len(), 8, "80 samples / 10");
        assert!((p.rate - 4.0).abs() < 1e-6, "4 buckets a second");
        assert!((p.peak - 0.5).abs() < 1e-6, "largest magnitude");
        let first = p.span(0.0, 0.9).ok_or_else(|| anyhow::anyhow!("no span"))?;
        assert!(
            (first.1 - 0.5).abs() < 1e-6,
            "0.25 of a 0.5 peak: {first:?}"
        );
        let whole = p.span(0.0, 2.0).ok_or_else(|| anyhow::anyhow!("no span"))?;
        assert!((whole.0 + 1.0).abs() < 1e-6, "the loud half: {whole:?}");
        assert!(p.span(5.0, 6.0).is_none(), "past the end");
        let level = p
            .level(0.0, 0.9)
            .ok_or_else(|| anyhow::anyhow!("no level"))?;
        assert!(
            (level - 0.25).abs() < 1e-6,
            "levels are of full scale, not the peak: {level}"
        );
        Ok(())
    }
}

// ============================================================================
// Module:       gui::pages::music::peaks
// Description:  The track's waveform as min/max peaks, decoded once on a
//               worker thread and kept for the session.
//
// Dependencies: egui; crate::media (audio_mono)
// ============================================================================

//! Waveform peaks.
//!
//! The track is decoded once (ffmpeg, mono, 22.05 kHz) on its own thread
//! and folded into min/max pairs at two resolutions: 64 samples (~3 ms,
//! enough for the most zoomed-in view) and 1024 samples (~46 ms, for the
//! overview). Drawing asks [`Peaks::range`] for one pixel column at a time
//! and reads whichever level is finer than a pixel, so even a full-width
//! overview of a long track is a few thousand reads a frame.

use std::sync::mpsc::{Receiver, TryRecvError};

/// Decode rate.
const RATE: u32 = 22_050;
/// Samples per fine bin.
const FINE: usize = 64;
/// Fine bins per coarse bin.
const COARSE: usize = 16;

/// Min/max peaks of a track.
#[derive(Debug, Default)]
pub(crate) struct Peaks {
    fine: Vec<(f32, f32)>,
    coarse: Vec<(f32, f32)>,
    /// The loudest absolute sample, for normalising the drawing.
    pub(crate) top: f32,
}

impl Peaks {
    /// Folds mono samples at [`RATE`] into peaks.
    fn from_samples(samples: &[f32]) -> Self {
        let fold = |chunk: &[(f32, f32)]| {
            chunk.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (a, b)| {
                (lo.min(*a), hi.max(*b))
            })
        };
        let fine: Vec<(f32, f32)> = samples
            .chunks(FINE)
            .map(|c| {
                c.iter()
                    .fold((f32::MAX, f32::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)))
            })
            .collect();
        let coarse = fine.chunks(COARSE).map(fold).collect();
        let top = fine
            .iter()
            .fold(1e-6_f32, |m, (lo, hi)| m.max(lo.abs()).max(hi.abs()));
        Self { fine, coarse, top }
    }

    /// Min and max over `[t0, t1)` seconds, or `None` outside the track.
    pub(crate) fn range(&self, t0: f32, t1: f32) -> Option<(f32, f32)> {
        let per_fine = FINE as f32 / RATE as f32;
        let a = (t0.max(0.0) / per_fine) as usize;
        let b = ((t1.max(0.0) / per_fine).ceil() as usize).max(a + 1);
        if a >= self.fine.len() {
            return None;
        }
        let (level, a, b) = if b - a > 2 * COARSE {
            (
                &self.coarse,
                a / COARSE,
                b.div_ceil(COARSE).max(a / COARSE + 1),
            )
        } else {
            (&self.fine, a, b)
        };
        let b = b.min(level.len());
        let slice = level.get(a..b)?;
        let (lo, hi) = slice.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (x, y)| {
            (lo.min(*x), hi.max(*y))
        });
        (lo <= hi).then_some((lo, hi))
    }
}

/// Where the peaks for the current track are.
#[derive(Debug, Default)]
enum Load {
    /// Nothing asked for yet.
    #[default]
    Idle,
    /// Decoding on a worker.
    Loading(Receiver<Result<Peaks, String>>),
    /// Ready.
    Ready(Peaks),
    /// Decoding failed.
    Failed(String),
}

/// The peaks of one track, loaded on demand.
#[derive(Debug, Default)]
pub(crate) struct Cache {
    path: String,
    load: Load,
}

impl Cache {
    /// The peaks of `path`, starting a decode the first time it is asked
    /// for (or when the track changes). `None` while decoding.
    pub(crate) fn get(&mut self, path: &str, ctx: &egui::Context) -> Option<&Peaks> {
        if self.path != path || matches!(self.load, Load::Idle) {
            self.path = path.to_owned();
            self.load = Load::Loading(spawn(path.to_owned(), ctx.clone()));
        }
        if let Load::Loading(rx) = &self.load {
            match rx.try_recv() {
                Ok(Ok(peaks)) => self.load = Load::Ready(peaks),
                Ok(Err(e)) => self.load = Load::Failed(e),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.load = Load::Failed("the waveform worker stopped".to_owned());
                }
            }
        }
        match &self.load {
            Load::Ready(peaks) => Some(peaks),
            Load::Idle | Load::Loading(_) | Load::Failed(_) => None,
        }
    }

    /// Why the waveform is missing, if decoding failed.
    pub(crate) fn error(&self) -> Option<&str> {
        match &self.load {
            Load::Failed(e) => Some(e),
            Load::Idle | Load::Loading(_) | Load::Ready(_) => None,
        }
    }
}

/// Decodes `path` into peaks on a new thread.
fn spawn(path: String, ctx: egui::Context) -> Receiver<Result<Peaks, String>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = crate::media::audio_mono(std::path::Path::new(&path), RATE)
            .map(|s| Peaks::from_samples(&s))
            .map_err(|e| format!("{e:#}"));
        let _ = tx.send(result);
        ctx.request_repaint();
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::{Peaks, RATE};

    #[test]
    fn peaks_cover_the_samples_at_both_levels() -> anyhow::Result<()> {
        // One second of a ramp from -1 to 1.
        let n = RATE as usize;
        let samples: Vec<f32> = (0..n).map(|i| -1.0 + 2.0 * i as f32 / n as f32).collect();
        let p = Peaks::from_samples(&samples);
        assert!((p.top - 1.0).abs() < 1e-3, "loudest sample: {}", p.top);
        let Some((lo, hi)) = p.range(0.0, 1.0) else {
            anyhow::bail!("the whole second has peaks");
        };
        assert!(lo < -0.99 && hi > 0.99, "coarse level spans it: {lo} {hi}");
        let Some((lo, hi)) = p.range(0.5, 0.501) else {
            anyhow::bail!("a millisecond has peaks");
        };
        assert!(
            lo > -0.05 && hi < 0.05,
            "the fine level is local: {lo} {hi}"
        );
        assert!(p.range(5.0, 6.0).is_none(), "nothing past the end");
        Ok(())
    }
}

// ============================================================================
// Module:       gui::pages::plan::waveform
// Description:  Min/max peaks and clipped runs of a music file for the A1
//               track, decoded once per file on a worker thread and cached.
//
// Dependencies: anyhow, egui; crate::media (tool), ffmpeg, ffprobe
// ============================================================================

//! The music waveform.
//!
//! The track is decoded once on its own thread at its **own** sample rate
//! and channel count (no resampling, no downmix) and folded into min/max
//! pairs every 5 ms, streamed so a long song never sits in memory. Drawing
//! then only reads the buckets under each visible pixel column, so zooming
//! and scrolling cost nothing however long the song is.
//!
//! Clipping is judged on those raw samples: a bucket is clipped when one
//! channel holds [`CLIP_RUN`] consecutive samples at or above
//! [`CLIP_LEVEL`] of full scale. Resampling and downmixing are avoided on
//! purpose: ffmpeg's stereo-to-mono matrix sums at 0.707 per channel and a
//! resampler rings past full scale on a limited master, which made a
//! mastered track look clipped from end to end.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};

/// A bucket is 5 ms: this many buckets a second.
const BUCKETS_PER_SECOND: u32 = 200;
/// A sample this loud (of full scale) is at the ceiling. 16-bit full scale
/// decodes to 32767/32768, so integer sources reach it.
const CLIP_LEVEL: f32 = 0.999;
/// Consecutive ceiling samples on one channel that count as clipping; a
/// single sample touching the ceiling is a peak, not a clip.
const CLIP_RUN: u32 = 3;

/// Min/max pairs over fixed buckets, with the buckets that clipped.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Peaks {
    /// Buckets per second.
    pub(super) rate: f32,
    /// Lowest sample in each bucket, over every channel.
    pub(super) min: Vec<f32>,
    /// Highest sample in each bucket, over every channel.
    pub(super) max: Vec<f32>,
    /// Whether each bucket holds a clipped run.
    pub(super) clipped: Vec<bool>,
    /// Largest absolute sample, for normalising.
    pub(super) peak: f32,
}

/// Folds interleaved samples into [`Peaks`] as they arrive.
#[derive(Debug)]
struct Folder {
    /// Channels per frame.
    channels: usize,
    /// Frames per bucket.
    bucket: usize,
    /// The channel the next sample belongs to.
    channel: usize,
    /// Frames in the open bucket.
    frames: usize,
    /// The open bucket's lowest sample.
    lo: f32,
    /// The open bucket's highest sample.
    hi: f32,
    /// Whether the open bucket clipped.
    clip: bool,
    /// Each channel's current run of ceiling samples.
    runs: Vec<u32>,
    /// What is folded so far.
    out: Peaks,
}

impl Folder {
    /// A folder for `channels` interleaved channels at `rate` Hz, `bucket`
    /// frames to a bucket.
    fn new(channels: usize, rate: u32, bucket: usize) -> Self {
        let channels = channels.max(1);
        let bucket = bucket.max(1);
        Self {
            channels,
            bucket,
            channel: 0,
            frames: 0,
            lo: f32::MAX,
            hi: f32::MIN,
            clip: false,
            runs: vec![0; channels],
            out: Peaks {
                rate: rate as f32 / bucket as f32,
                ..Peaks::default()
            },
        }
    }

    /// Takes the next interleaved sample.
    fn push(&mut self, s: f32) {
        self.lo = self.lo.min(s);
        self.hi = self.hi.max(s);
        if let Some(run) = self.runs.get_mut(self.channel) {
            if s.abs() >= CLIP_LEVEL {
                *run += 1;
                if *run >= CLIP_RUN {
                    self.clip = true;
                }
            } else {
                *run = 0;
            }
        }
        self.channel += 1;
        if self.channel == self.channels {
            self.channel = 0;
            self.frames += 1;
            if self.frames == self.bucket {
                self.close();
            }
        }
    }

    /// Ends the open bucket, if it holds anything.
    fn close(&mut self) {
        if self.lo <= self.hi {
            self.out.peak = self.out.peak.max(self.lo.abs()).max(self.hi.abs());
            self.out.min.push(self.lo);
            self.out.max.push(self.hi);
            self.out.clipped.push(self.clip);
        }
        self.frames = 0;
        self.lo = f32::MAX;
        self.hi = f32::MIN;
        self.clip = false;
    }

    /// The peaks, with the last partial bucket closed.
    fn finish(mut self) -> Peaks {
        self.close();
        self.out
    }
}

impl Peaks {
    /// Folds interleaved `samples` of `channels` channels (at `rate` Hz)
    /// into buckets of `bucket` frames. Decoding streams through a
    /// [`Folder`] instead; this is the same fold over a slice.
    #[cfg(test)]
    fn from_samples(samples: &[f32], channels: usize, rate: u32, bucket: usize) -> Self {
        let mut folder = Folder::new(channels, rate, bucket);
        for &s in samples {
            folder.push(s);
        }
        folder.finish()
    }

    /// The bucket range between `t0` and `t1` seconds; `None` outside the
    /// track.
    fn range(&self, t0: f32, t1: f32) -> Option<std::ops::Range<usize>> {
        let n = self.min.len();
        if n == 0 || t1 < 0.0 {
            return None;
        }
        let a = (t0.max(0.0) * self.rate) as usize;
        let b = ((t1 * self.rate).ceil() as usize).max(a + 1).min(n);
        (a < n).then_some(a..b)
    }

    /// Lowest and highest sample between `t0` and `t1` seconds, normalised
    /// to the track's peak; `None` outside the track.
    pub(super) fn span(&self, t0: f32, t1: f32) -> Option<(f32, f32)> {
        let r = self.range(t0, t1)?;
        let lo = self
            .min
            .get(r.clone())?
            .iter()
            .copied()
            .fold(f32::MAX, f32::min);
        let hi = self.max.get(r)?.iter().copied().fold(f32::MIN, f32::max);
        let norm = self.peak.max(1e-6);
        Some((lo / norm, hi / norm))
    }

    /// The loudest sample between `t0` and `t1` seconds as a fraction of
    /// full scale (not of the track's peak), the larger of |min| and |max|,
    /// as Selects draws its bars. `None` outside the track.
    pub(super) fn level(&self, t0: f32, t1: f32) -> Option<f32> {
        let (lo, hi) = self.span(t0, t1)?;
        let norm = self.peak.max(1e-6);
        Some((lo.abs().max(hi.abs()) * norm).min(1.0))
    }

    /// Whether the source clipped anywhere between `t0` and `t1` seconds.
    pub(super) fn clipped(&self, t0: f32, t1: f32) -> bool {
        self.range(t0, t1)
            .and_then(|r| self.clipped.get(r))
            .is_some_and(|c| c.iter().any(|&b| b))
    }
}

/// The first audio stream's sample rate and channel count.
fn audio_format(path: &Path) -> Result<(u32, usize)> {
    let output = Command::new(crate::media::tool("ffprobe"))
        .args(["-v", "error", "-select_streams", "a:0"])
        .args(["-show_entries", "stream=sample_rate,channels"])
        .args(["-of", "csv=p=0"])
        .arg(path)
        .output()
        .with_context(|| format!("running ffprobe on {}", path.display()))?;
    if !output.status.success() {
        bail!(
            "ffprobe failed on {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    parse_format(&String::from_utf8_lossy(&output.stdout))
        .with_context(|| format!("{} has no audio stream", path.display()))
}

/// Reads ffprobe's `sample_rate,channels` line.
fn parse_format(text: &str) -> Option<(u32, usize)> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let mut fields = line.trim().split(',');
    let rate = fields.next()?.trim().parse::<u32>().ok()?;
    let channels = fields.next()?.trim().parse::<usize>().ok()?;
    (rate > 0 && channels > 0).then_some((rate, channels))
}

/// Decodes `path` at its own rate and channel count and folds it, streamed.
fn decode(path: &Path) -> Result<Peaks> {
    let (rate, channels) = audio_format(path)?;
    let bucket = (rate / BUCKETS_PER_SECOND).max(1) as usize;
    let mut child = Command::new(crate::media::tool("ffmpeg"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vn", "-f", "f32le", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("decoding audio of {}", path.display()))?;
    let mut folder = Folder::new(channels, rate, bucket);
    let read = (|| -> Result<()> {
        let Some(mut out) = child.stdout.take() else {
            bail!("ffmpeg gave no output");
        };
        let mut buf = vec![0_u8; 1 << 16];
        let mut carry: Vec<u8> = Vec::with_capacity(4);
        loop {
            let n = out.read(&mut buf)?;
            if n == 0 {
                return Ok(());
            }
            let mut bytes = buf.get(..n).unwrap_or_default();
            if !carry.is_empty() {
                let need = 4 - carry.len();
                let take = need.min(bytes.len());
                carry.extend_from_slice(bytes.get(..take).unwrap_or_default());
                bytes = bytes.get(take..).unwrap_or_default();
                if let Ok(word) = <[u8; 4]>::try_from(carry.as_slice()) {
                    folder.push(f32::from_le_bytes(word));
                    carry.clear();
                }
            }
            let (words, rest) = bytes.as_chunks::<4>();
            for w in words {
                folder.push(f32::from_le_bytes(*w));
            }
            carry.extend_from_slice(rest);
        }
    })();
    let status = child.wait()?;
    read?;
    if !status.success() {
        bail!("ffmpeg could not decode audio from {}", path.display());
    }
    Ok(folder.finish())
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
                let entry = match decode(&path) {
                    Ok(peaks) => Wave::Ready(Arc::new(peaks)),
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
    use super::{Peaks, parse_format};

    #[test]
    fn peaks_fold_buckets_and_spans_normalise() -> anyhow::Result<()> {
        let samples: Vec<f32> = (0..80).map(|i| if i < 40 { 0.25 } else { -0.5 }).collect();
        let p = Peaks::from_samples(&samples, 1, 40, 10);
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

    /// A stereo sine at `amp` (a cycle every ~1250 samples, so a 40-sample
    /// bucket sees either a crest or a crossing), hard-limited at `ceiling`.
    fn sine(amp: f32, ceiling: f32) -> Vec<f32> {
        (0..8000)
            .flat_map(|i| {
                let s = (amp * (i as f32 * 0.005).sin()).clamp(-ceiling, ceiling);
                [s, s]
            })
            .collect()
    }

    #[test]
    fn a_loud_master_is_not_clipped() -> anyhow::Result<()> {
        // Peaks at 0.997 of full scale, -0.03 dB, like a limited master:
        // every column is loud, none clipped.
        let p = Peaks::from_samples(&sine(0.997, 1.0), 2, 8000, 40);
        assert!(p.peak > 0.99, "it is loud: {}", p.peak);
        assert!(
            !p.clipped.iter().any(|&c| c),
            "a loud signal that never sits at the ceiling has no clip marks"
        );
        assert!(!p.clipped(0.0, 1.0), "no marks over the whole second");
        Ok(())
    }

    #[test]
    fn a_hard_clipped_signal_is_marked() -> anyhow::Result<()> {
        // Driven to twice full scale and cut flat at 1.0.
        let p = Peaks::from_samples(&sine(2.0, 1.0), 2, 8000, 40);
        let marked = p.clipped.iter().filter(|&&c| c).count();
        assert!(marked > 0, "flat tops at full scale are marked");
        assert!(
            marked < p.clipped.len(),
            "the zero crossings are not: {marked} of {}",
            p.clipped.len()
        );
        assert!(p.clipped(0.0, 1.0), "the second holds a clip");
        Ok(())
    }

    #[test]
    fn a_lone_peak_at_the_ceiling_is_not_a_clip() -> anyhow::Result<()> {
        let mut samples = vec![0.1_f32; 100];
        if let Some(s) = samples.get_mut(50) {
            *s = 1.0;
        }
        let p = Peaks::from_samples(&samples, 1, 100, 10);
        assert!(!p.clipped(0.0, 1.0), "one sample at the ceiling is a peak");
        Ok(())
    }

    #[test]
    fn ffprobe_format_lines_parse() -> anyhow::Result<()> {
        assert_eq!(
            parse_format("44100,2\n"),
            Some((44100, 2)),
            "rate, channels"
        );
        assert_eq!(parse_format("\n48000,1"), Some((48000, 1)), "blank lines");
        assert_eq!(parse_format(""), None, "no audio stream");
        assert_eq!(parse_format("0,2"), None, "a zero rate");
        Ok(())
    }
}

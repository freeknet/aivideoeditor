// ============================================================================
// Module:       gui::player::audio
// Description:  Sound for the viewer: ffmpeg decodes f32 PCM on a thread, a
//               cpal output stream plays it and counts what it played.
//
// Dependencies: cpal, std::process, std::sync; crate::media
// ============================================================================

//! Audio playback, and the clock video follows while it plays.
//!
//! The output device is opened lazily, on the first play, by a thread that
//! owns the cpal stream for the life of the [`Audio`] (cpal streams are not
//! `Send` on every platform, and owning it there keeps the UI thread free of
//! device calls). With no device, or a device that refuses us, the failure
//! is logged once and every later call is a cheap no-op: the viewer keeps
//! playing on its wall clock, silently.
//!
//! A *session* is "this file from this second". [`Audio::start`] bumps a
//! generation counter, empties the buffer and spawns a decoder thread that
//! runs `ffmpeg -ss <t> -i <file> -t <len> -vn -ac 2 -ar <device rate> -f
//! f32le -` and keeps about two seconds buffered. The device callback pops
//! samples and counts the frames it consumed; [`Audio::status`] turns that
//! count into a position, which is the master clock while playing.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
use cpal::{FromSample, SampleFormat, SizedSample};

/// Seconds of audio the decoder keeps ahead of the device.
const BUFFER_SECONDS: usize = 2;

/// State shared by the UI, the decoder thread and the device callback.
struct Shared {
    /// Device sample rate; 0 until the device is open.
    rate: u32,
    /// The device could not be opened (or died).
    failed: bool,
    /// Interleaved stereo samples waiting to be played.
    buf: VecDeque<f32>,
    /// Session generation; a decoder whose generation is stale stops.
    generation: u64,
    /// Source position at which the current session began, seconds.
    origin: f64,
    /// Stereo frames the device has consumed this session.
    consumed: u64,
    /// Whether the callback should consume samples.
    playing: bool,
    /// The decoder has delivered everything it will.
    eof: bool,
    /// Linear gain.
    volume: f32,
    /// Silence, while still consuming (the clock keeps running).
    muted: bool,
}

/// What a session is doing, for the master clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Status {
    /// Source position of the sample being played, seconds.
    pub(crate) pos: f64,
    /// Whether any sample has been played yet.
    pub(crate) started: bool,
    /// The session played everything it had.
    pub(crate) drained: bool,
    /// No device: there will never be sound.
    pub(crate) failed: bool,
}

/// The viewer's sound.
pub(crate) struct Audio {
    shared: Arc<Mutex<Shared>>,
    /// Dropping this ends the device thread (and closes the stream).
    keep: Option<mpsc::Sender<()>>,
    /// Whether the device thread has been started.
    opened: bool,
}

impl Default for Audio {
    fn default() -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                rate: 0,
                failed: false,
                buf: VecDeque::new(),
                generation: 0,
                origin: 0.0,
                consumed: 0,
                playing: false,
                eof: false,
                volume: 1.0,
                muted: false,
            })),
            keep: None,
            opened: false,
        }
    }
}

impl Audio {
    /// Plays `path` from file second `t` for `len` seconds; its first sample
    /// is source position `origin`. Replaces any session.
    pub(crate) fn start(&mut self, path: &Path, t: f64, len: f64, origin: f64) {
        self.open_device();
        let generation = {
            let Ok(mut s) = self.shared.lock() else {
                return;
            };
            if s.failed {
                return;
            }
            s.generation += 1;
            s.buf.clear();
            s.origin = origin;
            s.consumed = 0;
            s.eof = false;
            s.playing = true;
            s.generation
        };
        let job = Decode {
            shared: self.shared.clone(),
            path: path.to_path_buf(),
            t: t.max(0.0),
            len: len.max(0.05),
            generation,
        };
        let spawned = std::thread::Builder::new()
            .name("ave-audio-decode".into())
            .spawn(move || job.run());
        if let Err(err) = spawned {
            log::warn!("could not start the audio decoder thread: {err}");
        }
    }

    /// Stops the session (the buffer is dropped; the device stays open).
    pub(crate) fn stop(&mut self) {
        if let Ok(mut s) = self.shared.lock() {
            s.generation += 1;
            s.buf.clear();
            s.playing = false;
            s.eof = true;
        }
    }

    /// Sets gain and mute.
    pub(crate) fn set_volume(&self, volume: f32, muted: bool) {
        if let Ok(mut s) = self.shared.lock() {
            s.volume = volume.clamp(0.0, 1.0);
            s.muted = muted;
        }
    }

    /// Where the session is.
    pub(crate) fn status(&self) -> Status {
        let Ok(s) = self.shared.lock() else {
            return Status {
                pos: 0.0,
                started: false,
                drained: true,
                failed: true,
            };
        };
        let pos = if s.rate == 0 {
            s.origin
        } else {
            s.origin + s.consumed as f64 / f64::from(s.rate)
        };
        Status {
            pos,
            started: s.consumed > 0,
            drained: s.eof && s.buf.len() < 2,
            failed: s.failed,
        }
    }

    /// Opens the output device on its own thread, once.
    fn open_device(&mut self) {
        if self.opened {
            return;
        }
        self.opened = true;
        let (keep, wait) = mpsc::channel::<()>();
        let shared = self.shared.clone();
        let spawned = std::thread::Builder::new()
            .name("ave-audio-out".into())
            .spawn(move || device_thread(&shared, &wait));
        match spawned {
            Ok(_) => self.keep = Some(keep),
            Err(err) => {
                log::warn!("could not start the audio output thread: {err}");
                if let Ok(mut s) = self.shared.lock() {
                    s.failed = true;
                }
            }
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.stop();
        self.keep = None;
    }
}

/// Opens the default output device and keeps its stream alive until the
/// [`Audio`] is dropped.
fn device_thread(shared: &Arc<Mutex<Shared>>, wait: &mpsc::Receiver<()>) {
    let stream = match open_stream(shared) {
        Ok(stream) => stream,
        Err(err) => {
            log::warn!("no audio output, playing silently: {err:#}");
            if let Ok(mut s) = shared.lock() {
                s.failed = true;
                s.buf.clear();
            }
            return;
        }
    };
    // Blocks until the sender is dropped; the stream lives until then.
    let _ = wait.recv();
    drop(stream);
}

/// Builds and starts the output stream; records the device rate.
fn open_stream(shared: &Arc<Mutex<Shared>>) -> anyhow::Result<cpal::Stream> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no default output device"))?;
    let supported = device.default_output_config()?;
    let rate = supported.sample_rate();
    let channels = usize::from(supported.channels()).max(1);
    let format = supported.sample_format();
    let config = supported.config();
    let stream = if format == SampleFormat::F32 {
        build::<f32>(&device, config, shared.clone(), channels)?
    } else if format == SampleFormat::I16 {
        build::<i16>(&device, config, shared.clone(), channels)?
    } else if format == SampleFormat::U16 {
        build::<u16>(&device, config, shared.clone(), channels)?
    } else if format == SampleFormat::I32 {
        build::<i32>(&device, config, shared.clone(), channels)?
    } else {
        anyhow::bail!("unsupported output sample format {format:?}");
    };
    stream.play()?;
    if let Ok(mut s) = shared.lock() {
        s.rate = rate;
    }
    log::info!("audio output: {rate} Hz, {channels} channels, {format:?}");
    Ok(stream)
}

/// An output stream of sample type `T`.
fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    shared: Arc<Mutex<Shared>>,
    channels: usize,
) -> anyhow::Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let failed = shared.clone();
    let stream = device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], _| fill(data, channels, &shared),
        move |err| {
            log::warn!("audio output error: {err}");
            if let Ok(mut s) = failed.lock() {
                s.failed = true;
            }
        },
        None,
    )?;
    Ok(stream)
}

/// The device callback: never blocks on the lock (silence instead).
fn fill<T>(data: &mut [T], channels: usize, shared: &Mutex<Shared>)
where
    T: SizedSample + FromSample<f32>,
{
    let silence = T::from_sample(0.0_f32);
    let Ok(mut s) = shared.try_lock() else {
        data.fill(silence);
        return;
    };
    let gain = if s.muted { 0.0 } else { s.volume };
    for frame in data.chunks_mut(channels) {
        let (l, r) = if s.playing && s.buf.len() >= 2 {
            let l = s.buf.pop_front().unwrap_or(0.0);
            let r = s.buf.pop_front().unwrap_or(0.0);
            s.consumed += 1;
            (l * gain, r * gain)
        } else {
            (0.0, 0.0)
        };
        mix_into(frame, l, r, silence);
    }
}

/// Writes one stereo frame into a device frame of any channel count.
fn mix_into<T: SizedSample + FromSample<f32>>(frame: &mut [T], l: f32, r: f32, silence: T) {
    match frame {
        [] => {}
        [mono] => *mono = T::from_sample((l + r) * 0.5),
        [a, b, rest @ ..] => {
            *a = T::from_sample(l);
            *b = T::from_sample(r);
            rest.fill(silence);
        }
    }
}

/// A decoder thread's orders.
struct Decode {
    shared: Arc<Mutex<Shared>>,
    path: PathBuf,
    t: f64,
    len: f64,
    generation: u64,
}

impl Decode {
    /// The thread body.
    fn run(self) {
        if let Err(err) = self.decode() {
            log::warn!("audio decode of {} failed: {err:#}", self.path.display());
        }
        if let Ok(mut s) = self.shared.lock()
            && s.generation == self.generation
        {
            s.eof = true;
        }
    }

    /// Whether this session is still the current one.
    fn current(&self) -> bool {
        self.shared
            .lock()
            .map(|s| s.generation == self.generation)
            .unwrap_or(false)
    }

    fn decode(&self) -> anyhow::Result<()> {
        let Some(rate) = self.wait_for_rate() else {
            return Ok(());
        };
        let mut cmd = crate::media::command("ffmpeg");
        cmd.args(["-v", "error", "-nostdin"])
            .args(["-ss", &format!("{:.3}", self.t), "-i"])
            .arg(&self.path)
            .args(["-t", &format!("{:.3}", self.len), "-vn", "-sn"])
            .args(["-ac", "2", "-ar", &rate.to_string(), "-f", "f32le", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            anyhow::bail!("ffmpeg stdout was not captured");
        };
        let cap = rate as usize * 2 * BUFFER_SECONDS;
        let mut bytes = vec![0_u8; 16 * 1024];
        let mut carry: Vec<u8> = Vec::new();
        'read: loop {
            let n = match out.read(&mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            carry.extend_from_slice(&bytes[..n]);
            let whole = carry.len() / 4 * 4;
            let samples: Vec<f32> = carry[..whole]
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            carry.drain(..whole);
            loop {
                let Ok(mut s) = self.shared.lock() else {
                    break 'read;
                };
                if s.generation != self.generation {
                    break 'read;
                }
                if s.buf.len() < cap {
                    s.buf.extend(samples);
                    break;
                }
                drop(s);
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }

    /// The device rate, once the device is open; `None` if it never opens
    /// or this session is superseded first.
    fn wait_for_rate(&self) -> Option<u32> {
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            {
                let s = self.shared.lock().ok()?;
                if s.failed || s.generation != self.generation {
                    return None;
                }
                if s.rate > 0 {
                    return Some(s.rate);
                }
            }
            if !self.current() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::mix_into;

    #[test]
    fn stereo_maps_onto_any_channel_count() -> anyhow::Result<()> {
        let mut mono = [0.0_f32; 1];
        mix_into(&mut mono, 0.2, 0.4, 0.0);
        assert!((mono[0] - 0.3).abs() < 1e-6, "mono is the average");
        let mut surround = [9.0_f32; 6];
        mix_into(&mut surround, 0.2, 0.4, 0.0);
        assert_eq!(
            surround,
            [0.2, 0.4, 0.0, 0.0, 0.0, 0.0],
            "L, R, then silence"
        );
        Ok(())
    }
}

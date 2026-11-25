// ============================================================================
// Module:       gui::player::video
// Description:  One decoder thread per playing stretch of a file: ffmpeg
//               streams scaled RGBA frames into a ring the UI reads.
//
// Dependencies: egui (ColorImage, Context), std::process; crate::media
// ============================================================================

//! Video decoding for the viewer.
//!
//! A [`VideoStream`] is "this file, from this second to that one". Opening
//! one spawns a thread that probes the file (once per file, cached in
//! [`Probes`]), then runs
//!
//! ```text
//! ffmpeg -ss <start> -i <file> -t <len> -vf fps=<src fps>,scale=<w>:<h> -pix_fmt rgba -f rawvideo -
//! ```
//!
//! and pushes each frame, stamped with its file time, into a [`Ring`].
//! Input-side `-ss` costs one GOP, so a restart is cheap enough to do on
//! every seek. Dropping the stream cancels the ring; the thread notices at
//! its next frame, kills ffmpeg and exits, so the UI never waits on it.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use super::ring::{Frame, Ring};
use crate::media::Probe;

/// Probes shared between decoder threads and the UI, by file.
pub(crate) type Probes = Arc<Mutex<HashMap<PathBuf, Probe>>>;

/// Frames of look-ahead each decoder may hold (about half a second at 24
/// fps; 2 MB each at 960 px).
const RING_FRAMES: usize = 12;

/// A decoder for one stretch of one file.
pub(crate) struct VideoStream {
    /// The file.
    pub(crate) path: PathBuf,
    /// First file time decoded, seconds.
    pub(crate) start: f64,
    /// Which segment of the source this serves.
    pub(crate) seg: usize,
    /// Frame width asked of ffmpeg.
    pub(crate) width: u32,
    ring: Arc<Ring>,
}

impl VideoStream {
    /// Starts decoding `path` from `start` to `end` (file seconds), frames
    /// `width` pixels wide at most, repainting `ctx` as frames land.
    pub(crate) fn open(
        ctx: &egui::Context,
        probes: &Probes,
        path: &Path,
        seg: usize,
        start: f64,
        end: f64,
        width: u32,
    ) -> Self {
        let ring = Arc::new(Ring::new(RING_FRAMES));
        let job = Job {
            ctx: ctx.clone(),
            probes: probes.clone(),
            // A heavy original plays from its proxy (same frames, far lighter).
            path: crate::proxy::for_playback(path),
            start: start.max(0.0),
            end,
            width,
            ring: ring.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("ave-video".into())
            .spawn(move || job.run());
        if let Err(err) = spawned {
            log::warn!("could not start a video decoder thread: {err}");
            ring.end();
        }
        Self {
            path: path.to_path_buf(),
            start,
            seg,
            width,
            ring,
        }
    }

    /// The ring this decoder fills.
    pub(crate) fn ring(&self) -> &Ring {
        &self.ring
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        self.ring.cancel();
    }
}

/// What a decoder thread needs.
struct Job {
    ctx: egui::Context,
    probes: Probes,
    path: PathBuf,
    start: f64,
    end: f64,
    width: u32,
    ring: Arc<Ring>,
}

impl Job {
    /// The thread body: never panics, always ends the ring.
    fn run(self) {
        if let Err(err) = self.decode() {
            log::warn!("video decode of {} failed: {err:#}", self.path.display());
        }
        self.ring.end();
        self.ctx.request_repaint();
    }

    fn decode(&self) -> anyhow::Result<()> {
        let probe = probe_cached(&self.probes, &self.path)?;
        let fps = match probe.fps_f64() {
            f if f.is_finite() && f >= 1.0 => f,
            _ => 24.0,
        };
        let (w, h) = frame_size(probe.width, probe.height, self.width);
        let len = (self.end - self.start).max(0.05);
        let filter = format!("fps={fps:.6},scale={w}:{h}:flags=bilinear");
        let mut cmd = crate::media::command("ffmpeg");
        cmd.args(["-v", "error", "-nostdin", "-threads", "0"])
            .args(["-ss", &format!("{:.3}", self.start), "-i"])
            .arg(&self.path)
            .args(["-t", &format!("{len:.3}"), "-an", "-sn", "-vf", &filter])
            .args(["-pix_fmt", "rgba", "-f", "rawvideo", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd.spawn()?;
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            anyhow::bail!("ffmpeg stdout was not captured");
        };
        let size = [w as usize, h as usize];
        let mut buf = vec![0_u8; size[0] * size[1] * 4];
        let mut k: u64 = 0;
        loop {
            if out.read_exact(&mut buf).is_err() {
                break;
            }
            let image = egui::ColorImage::from_rgba_premultiplied(size, &buf);
            let frame = Frame {
                t: self.start + k as f64 / fps,
                image: Arc::new(image),
            };
            k += 1;
            if !self.ring.push(frame) {
                break;
            }
            self.ctx.request_repaint();
        }
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }
}

/// The probe for `path`, from the cache or ffprobe.
pub(crate) fn probe_cached(probes: &Probes, path: &Path) -> anyhow::Result<Probe> {
    if let Ok(map) = probes.lock()
        && let Some(p) = map.get(path)
    {
        return Ok(p.clone());
    }
    let probe = crate::media::probe(path)?;
    if let Ok(mut map) = probes.lock() {
        map.insert(path.to_path_buf(), probe.clone());
    }
    Ok(probe)
}

/// The decoded frame size for a `src_w`×`src_h` source at most `want`
/// pixels wide: never upscaled, both sides even.
pub(crate) fn frame_size(src_w: u32, src_h: u32, want: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        let w = want.max(2) & !1;
        return (w, (w * 9 / 16).max(2) & !1);
    }
    let w = want.clamp(2, src_w.max(2)) & !1;
    let h = (f64::from(w) * f64::from(src_h) / f64::from(src_w)).round() as u32;
    (w, h.max(2) & !1)
}

#[cfg(test)]
mod tests {
    use super::{Probes, VideoStream, frame_size};
    use crate::gui::player::ring::Pick;
    use std::time::{Duration, Instant};

    /// Writes a 3 s 24 fps test clip, or `None` when ffmpeg is missing.
    fn test_clip(dir: &std::path::Path) -> Option<std::path::PathBuf> {
        let out = dir.join("clip.mp4");
        let status = crate::media::command("ffmpeg")
            .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc=size=320x180:rate=24:duration=3")
            .args(["-pix_fmt", "yuv420p"])
            .arg(&out)
            .status()
            .ok()?;
        status.success().then_some(out)
    }

    #[test]
    fn a_decoder_streams_timestamped_frames_from_its_start() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let Some(clip) = test_clip(dir.path()) else {
            eprintln!("ffmpeg not available; skipping");
            return Ok(());
        };
        let ctx = egui::Context::default();
        let probes = Probes::default();
        let stream = VideoStream::open(&ctx, &probes, &clip, 0, 1.0, 3.0, 160);
        let fd = 1.0 / 24.0;
        let deadline = Instant::now() + Duration::from_secs(30);
        let first = loop {
            let (choice, frame) = stream.ring().take(1.0, fd);
            if let (Pick::Show(_), Some(frame)) = (choice, frame) {
                break frame;
            }
            assert!(
                Instant::now() < deadline,
                "no frame within 30 s: {choice:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            (first.t - 1.0).abs() < 1e-6,
            "starts at the seek: {}",
            first.t
        );
        assert_eq!(first.image.size, [160, 90], "scaled, aspect kept");
        // A step forward is served from the look-ahead.
        let deadline = Instant::now() + Duration::from_secs(30);
        let later = loop {
            let (choice, frame) = stream.ring().take(1.0 + 5.0 * fd, fd);
            if let (Pick::Show(_), Some(frame)) = (choice, frame)
                && frame.t > 1.0 + 4.5 * fd
            {
                break frame;
            }
            assert!(Instant::now() < deadline, "no later frame: {choice:?}");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            (later.t - (1.0 + 5.0 * fd)).abs() < 1e-6,
            "the sixth frame: {}",
            later.t
        );
        let (back, _) = stream.ring().take(0.2, fd);
        assert_eq!(back, Pick::Restart, "seeking back needs a new decoder");
        Ok(())
    }

    #[test]
    fn frames_are_scaled_down_never_up_and_even() -> anyhow::Result<()> {
        assert_eq!(frame_size(1920, 1080, 960), (960, 540), "half of 1080p");
        assert_eq!(frame_size(1280, 720, 1920), (1280, 720), "never upscaled");
        assert_eq!(frame_size(1080, 1920, 961), (960, 1706), "portrait, even");
        assert_eq!(frame_size(0, 0, 640), (640, 360), "unknown size");
        Ok(())
    }
}

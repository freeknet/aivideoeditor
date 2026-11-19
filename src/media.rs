// ============================================================================
// Module:       media
// Description:  ffmpeg / ffprobe: locating the tools, probing a file, and
//               streaming decoded frames or audio samples into memory.
//
// Dependencies: anyhow, serde_json, std::process
// ============================================================================

//! Everything that talks to ffmpeg.
//!
//! The rest of the program never builds an ffmpeg command line of its own
//! for reading media: it asks this module for a [`Probe`], a
//! [`FrameReader`] or a buffer of mono samples. Rendering is the one
//! exception, and lives in `render`.
//!
//! The tools are found through `AVE_FFMPEG_DIR` when set, else `PATH`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

use anyhow::{Context, Result, bail};

/// Path to an ffmpeg-suite executable (`ffmpeg`, `ffprobe`).
pub(crate) fn tool(name: &str) -> PathBuf {
    match std::env::var_os("AVE_FFMPEG_DIR") {
        Some(dir) => Path::new(&dir).join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
        None => PathBuf::from(name),
    }
}

/// What ffprobe reports about one media file.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Probe {
    /// Duration in seconds.
    pub(crate) duration: f64,
    /// Video width in pixels (0 for audio-only files).
    pub(crate) width: u32,
    /// Video height in pixels.
    pub(crate) height: u32,
    /// Frame rate as a numerator/denominator pair, e.g. `[24000, 1001]`.
    pub(crate) fps: [u32; 2],
    /// Video codec name.
    pub(crate) vcodec: String,
    /// Whether the file has an audio stream.
    pub(crate) has_audio: bool,
}

impl Probe {
    /// Frame rate as a float.
    pub(crate) fn fps_f64(&self) -> f64 {
        if self.fps[1] == 0 {
            0.0
        } else {
            f64::from(self.fps[0]) / f64::from(self.fps[1])
        }
    }
}

/// Runs ffprobe on `path`.
pub(crate) fn probe(path: &Path) -> Result<Probe> {
    let output = Command::new(tool("ffprobe"))
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
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
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let streams = json["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let duration = json["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);
    let fps = video
        .and_then(|v| v["r_frame_rate"].as_str())
        .and_then(parse_ratio)
        .unwrap_or([0, 1]);
    let dim = |key: &str| {
        video
            .and_then(|v| v[key].as_u64())
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0)
    };
    Ok(Probe {
        duration,
        width: dim("width"),
        height: dim("height"),
        fps,
        vcodec: video
            .and_then(|v| v["codec_name"].as_str())
            .unwrap_or("")
            .to_owned(),
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
    })
}

/// Parses `"24000/1001"`.
fn parse_ratio(text: &str) -> Option<[u32; 2]> {
    let (n, d) = text.split_once('/')?;
    Some([n.trim().parse().ok()?, d.trim().parse().ok()?])
}

/// A decoded RGB frame.
pub(crate) struct Frame {
    /// Width in pixels.
    pub(crate) width: usize,
    /// Height in pixels.
    pub(crate) height: usize,
    /// Packed RGB24, row-major.
    pub(crate) rgb: Vec<u8>,
}

/// Streams frames from ffmpeg at a fixed sample rate and size.
pub(crate) struct FrameReader {
    child: Child,
    stdout: ChildStdout,
    width: usize,
    height: usize,
}

impl FrameReader {
    /// Starts decoding `path` at `fps` frames per second, scaled to
    /// `width` pixels wide (height follows the aspect ratio, kept even).
    pub(crate) fn open(path: &Path, probe: &Probe, fps: f64, width: usize) -> Result<Self> {
        let height = if probe.width == 0 {
            width * 9 / 16
        } else {
            let h = width as f64 * f64::from(probe.height) / f64::from(probe.width);
            ((h / 2.0).round() as usize).max(1) * 2
        };
        let filter = format!("fps={fps},scale={width}:{height}:flags=area,format=rgb24");
        let mut child = Command::new(tool("ffmpeg"))
            .args(["-v", "error", "-nostdin", "-threads", "4", "-i"])
            .arg(path)
            .args(["-an", "-sn", "-vf", &filter, "-f", "rawvideo", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("starting ffmpeg on {}", path.display()))?;
        let stdout = child
            .stdout
            .take()
            .context("ffmpeg stdout was not captured")?;
        Ok(Self {
            child,
            stdout,
            width,
            height,
        })
    }

    /// The next frame, or `None` at the end of the stream.
    pub(crate) fn next_frame(&mut self) -> Result<Option<Frame>> {
        let mut rgb = vec![0_u8; self.width * self.height * 3];
        let mut filled = 0;
        while filled < rgb.len() {
            let n = self.stdout.read(&mut rgb[filled..])?;
            if n == 0 {
                return Ok(None);
            }
            filled += n;
        }
        Ok(Some(Frame {
            width: self.width,
            height: self.height,
            rgb,
        }))
    }
}

impl Drop for FrameReader {
    fn drop(&mut self) {
        // The stream may be abandoned early; ffmpeg must not outlive it.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Decodes the audio of `path` to mono f32 samples at `rate` Hz.
pub(crate) fn audio_mono(path: &Path, rate: u32) -> Result<Vec<f32>> {
    let output = Command::new(tool("ffmpeg"))
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vn",
            "-ac",
            "1",
            "-ar",
            &rate.to_string(),
            "-f",
            "f32le",
            "-",
        ])
        .output()
        .with_context(|| format!("decoding audio of {}", path.display()))?;
    if !output.status.success() {
        bail!(
            "ffmpeg could not decode audio from {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

/// Decodes one frame of `path` at `t` seconds as packed RGB24, scaled to
/// `width` pixels wide (height follows the aspect, kept even). Used for
/// thumbnails: seeking is input-side, so it costs one GOP, not the clip.
pub(crate) fn frame_rgb(path: &Path, t: f64, width: u32) -> Result<Frame> {
    let output = Command::new(tool("ffmpeg"))
        .args(["-v", "error", "-nostdin", "-ss", &format!("{t:.3}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-an", "-sn"])
        .args([
            "-vf",
            &format!("scale={width}:-2:flags=bilinear,format=rgb24"),
        ])
        .args(["-f", "rawvideo", "-"])
        .output()
        .with_context(|| format!("decoding a frame of {}", path.display()))?;
    let width = width as usize;
    let row = width * 3;
    if !output.status.success() || row == 0 || output.stdout.len() < row {
        bail!("ffmpeg returned no frame at {t:.2}s of {}", path.display());
    }
    let height = output.stdout.len() / row;
    let mut rgb = output.stdout;
    rgb.truncate(row * height);
    Ok(Frame { width, height, rgb })
}

/// Writes one JPEG frame of `path` at `t` seconds, scaled to `width`.
pub(crate) fn still(path: &Path, t: f64, width: u32, crop: Option<&str>, out: &Path) -> Result<()> {
    let mut filter = String::new();
    if let Some(crop) = crop {
        filter.push_str(crop);
        filter.push(',');
    }
    filter.push_str(&format!("scale={width}:-2:flags=lanczos"));
    let status = Command::new(tool("ffmpeg"))
        .args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-ss",
            &format!("{t:.3}"),
            "-i",
        ])
        .arg(path)
        .args(["-frames:v", "1", "-vf", &filter, "-q:v", "3"])
        .arg(out)
        .status()
        .with_context(|| format!("extracting a still from {}", path.display()))?;
    if !status.success() {
        bail!(
            "ffmpeg could not extract a still at {t:.2}s from {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_ratio;

    #[test]
    fn ratios_parse() -> anyhow::Result<()> {
        assert_eq!(parse_ratio("24000/1001"), Some([24000, 1001]), "NTSC film");
        assert_eq!(parse_ratio("30/1"), Some([30, 1]), "integer rate");
        assert_eq!(parse_ratio("junk"), None, "not a ratio");
        Ok(())
    }
}

// ============================================================================
// Module:       proxy
// Description:  Playback copies of heavy clips: small H.264 files the editor's
//               player decodes instead of 4K / HEVC originals.
//
// Dependencies: anyhow; crate::{index, media, commands}
// ============================================================================

//! Proxies: what the player plays when the original is too heavy to decode
//! in real time.
//!
//! A 4K, 60 fps, 10-bit HEVC clip takes about a second of CPU to decode a
//! second of picture, so a player that starts a decoder per shot (and warms
//! the next few) falls behind and stutters. Its proxy, `<work>/proxies/
//! <clip id>.mp4`, is H.264 at most [`LONG_SIDE`] pixels on its long side,
//! the source frame rate (so shot times line up frame for frame) and a
//! keyframe every [`GOP`] frames (so a seek lands fast), with no audio: the
//! player takes sound from the original. Renders always use the originals.
//!
//! Made on the NVIDIA GPU when there is one (decode, scale and encode on the
//! card: 30 s of 4K in about 3.5 s), else on the CPU. Only clips that need
//! one ([`needed`]) get one; a proxy older than its source is made again.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::analyze::ClipIndex;
use crate::commands::{ItemState, Progress, Report};

/// A proxy's long side, pixels: enough for a 9:16 crop of a wide shot to
/// stay sharp in the preview.
const LONG_SIDE: u32 = 1280;

/// Frames between keyframes: a seek decodes at most this many to land.
const GOP: u32 = 15;

/// Whether `clip` is heavy enough to need a proxy: bigger than 1080p, or a
/// codec that is slow to decode (HEVC, ProRes, AV1).
pub(crate) fn needed(clip: &ClipIndex) -> bool {
    let p = &clip.probe;
    let pixels = u64::from(p.width) * u64::from(p.height);
    pixels > 1920 * 1080 || matches!(p.vcodec.as_str(), "hevc" | "prores" | "av1")
}

/// Where `clip`'s proxy lives.
pub(crate) fn path(work: &Path, clip_id: &str) -> PathBuf {
    work.join("proxies").join(format!("{clip_id}.mp4"))
}

/// The proxy for `clip`, when one exists and is not older than its source.
pub(crate) fn fresh(work: &Path, clip: &ClipIndex) -> Option<PathBuf> {
    let proxy = path(work, &clip.id);
    let made = std::fs::metadata(&proxy).ok()?.modified().ok()?;
    let source = std::fs::metadata(&clip.path).ok()?.modified().ok()?;
    (made >= source).then_some(proxy)
}

/// The clips of `index` that need a proxy and have no fresh one.
pub(crate) fn missing<'a>(work: &Path, index: &'a crate::index::Index) -> Vec<&'a ClipIndex> {
    index
        .clips
        .iter()
        .filter(|c| needed(c) && fresh(work, c).is_none())
        .collect()
}

/// Original path to fresh proxy, for the open project: what the player
/// reads in place of each heavy original.
static PLAYBACK: std::sync::Mutex<std::collections::BTreeMap<PathBuf, PathBuf>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

/// Records the fresh proxies of `index` (none for `None`) as the ones the
/// player uses; called whenever the open project's index is (re)loaded.
pub(crate) fn register(work: &Path, index: Option<&crate::index::Index>) {
    let map = index
        .map(|i| {
            i.clips
                .iter()
                .filter_map(|c| fresh(work, c).map(|p| (PathBuf::from(&c.path), p)))
                .collect()
        })
        .unwrap_or_default();
    if let Ok(mut playback) = PLAYBACK.lock() {
        *playback = map;
    }
}

/// The file to decode for playback of `original`: its proxy when it has a
/// fresh one, else the original itself.
pub(crate) fn for_playback(original: &Path) -> PathBuf {
    PLAYBACK
        .lock()
        .ok()
        .and_then(|m| m.get(original).cloned())
        .unwrap_or_else(|| original.to_path_buf())
}

/// Makes every missing proxy (every needed one with `force`); returns how
/// many it made.
pub(crate) fn run(work: &Path, force: bool, progress: &Progress) -> Result<usize> {
    let index = crate::index::Index::load(work)?;
    let todo: Vec<&ClipIndex> = if force {
        index.clips.iter().filter(|c| needed(c)).collect()
    } else {
        missing(work, &index)
    };
    let total = todo.len();
    progress.stage(
        "proxy",
        format!("{total} clips to copy for smooth playback"),
    );
    std::fs::create_dir_all(work.join("proxies"))?;
    let began = Instant::now();
    let mut seconds_done = 0.0_f64;
    let seconds_all: f64 = todo.iter().map(|c| c.probe.duration).sum();
    for (n, clip) in todo.iter().enumerate() {
        progress.check()?;
        progress.report(&Report::item(
            "proxy",
            n,
            total,
            &clip.id,
            ItemState::Started,
        ));
        let started = Instant::now();
        match make(work, clip, progress) {
            Ok(()) => {
                seconds_done += clip.probe.duration;
                let rate = seconds_done / began.elapsed().as_secs_f64().max(0.1);
                let eta = ((seconds_all - seconds_done) / rate.max(0.01)) as f32;
                progress.report(
                    &Report::item("proxy", n + 1, total, &clip.id, ItemState::Done)
                        .took(started.elapsed().as_secs_f32())
                        .remaining(Some(eta)),
                );
            }
            Err(e) => {
                progress.check()?;
                progress.report(
                    &Report::item("proxy", n + 1, total, &clip.id, ItemState::Failed)
                        .because(&format!("{e:#}")),
                );
            }
        }
    }
    Ok(total)
}

/// The proxy's size for a `width` x `height` source: the long side at most
/// [`LONG_SIDE`], both sides even.
fn size(width: u32, height: u32) -> (u32, u32) {
    let (w, h) = (width.max(2), height.max(2));
    let scale = (f64::from(LONG_SIDE) / f64::from(w.max(h))).min(1.0);
    let even = |v: u32| ((f64::from(v) * scale / 2.0).round() as u32 * 2).max(2);
    (even(w), even(h))
}

/// Makes `clip`'s proxy: the GPU first, the CPU if that fails. Written to a
/// temporary file and renamed, so a proxy that exists is a whole one.
fn make(work: &Path, clip: &ClipIndex, progress: &Progress) -> Result<()> {
    let out = path(work, &clip.id);
    let tmp = out.with_extension("part.mp4");
    let (w, h) = size(clip.probe.width, clip.probe.height);
    let gop = GOP.to_string();
    let gpu: Vec<String> = [
        "-hwaccel",
        "cuda",
        "-hwaccel_output_format",
        "cuda",
        "-i",
        &clip.path,
        "-an",
        "-sn",
        "-dn",
        "-map",
        "0:v:0",
        "-vf",
        &format!("scale_cuda={w}:{h}:format=nv12"),
        "-c:v",
        "h264_nvenc",
        "-preset",
        "p4",
        "-cq",
        "28",
        "-g",
        &gop,
        "-bf",
        "0",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let cpu: Vec<String> = [
        "-i",
        &clip.path,
        "-an",
        "-sn",
        "-dn",
        "-map",
        "0:v:0",
        "-vf",
        &format!("scale={w}:{h}:flags=bilinear,format=yuv420p"),
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-crf",
        "26",
        "-g",
        &gop,
        "-bf",
        "0",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    let first = encode(&gpu, &tmp, progress);
    if let Err(e) = first {
        progress.check()?;
        log::info!("no GPU proxy for {} ({e:#}); making it on the CPU", clip.id);
        encode(&cpu, &tmp, progress)?;
    }
    std::fs::rename(&tmp, &out).with_context(|| format!("moving {} into place", out.display()))?;
    Ok(())
}

/// Runs one ffmpeg encode into `out`, stopping it if the step is cancelled.
fn encode(args: &[String], out: &Path, progress: &Progress) -> Result<()> {
    let mut child = crate::media::command("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .args(["-movflags", "+faststart"])
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting ffmpeg")?;
    loop {
        if progress.cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(out);
            progress.check()?;
        }
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let mut err = String::new();
            if let Some(mut pipe) = child.stderr.take() {
                use std::io::Read as _;
                let _ = pipe.read_to_string(&mut err);
            }
            let _ = std::fs::remove_file(out);
            bail!(
                "ffmpeg failed: {}",
                err.lines().next().unwrap_or("no message")
            );
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::{needed, size};
    use crate::analyze::ClipIndex;

    fn clip(width: u32, height: u32, vcodec: &str) -> ClipIndex {
        ClipIndex {
            id: "c".into(),
            path: "c.mp4".into(),
            stamp: (0, 0),
            probe: crate::media::Probe {
                duration: 10.0,
                width,
                height,
                fps: [30, 1],
                vcodec: vcodec.into(),
                has_audio: false,
                recorded_at: None,
                camera: None,
            },
            samples: Vec::new(),
            segments: Vec::new(),
            hist: Vec::new(),
            version: crate::analyze::ANALYSIS_VERSION,
        }
    }

    /// Windows only: elsewhere a backslash is not a separator.
    #[cfg(windows)]
    #[test]
    fn the_player_finds_a_copy_whichever_way_the_path_is_written() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("Thanksgiving 2026").join("DJI_0125.MP4");
        std::fs::create_dir_all(
            source
                .parent()
                .ok_or_else(|| anyhow::anyhow!("no parent"))?,
        )?;
        std::fs::write(&source, b"x")?;
        let mut c = clip(3840, 2160, "hevc");
        c.id = "DJI_0125".into();
        // As the index stores it: forward slashes, then a backslash.
        c.path = format!(
            "{}\\DJI_0125.MP4",
            source
                .parent()
                .map(|p| p.display().to_string().replace('\\', "/"))
                .unwrap_or_default()
        );
        let copy = super::path(dir.path(), "DJI_0125");
        std::fs::create_dir_all(copy.parent().ok_or_else(|| anyhow::anyhow!("no parent"))?)?;
        std::fs::write(&copy, b"y")?;
        let index = crate::index::Index {
            clips: vec![c],
            ..crate::index::Index::default()
        };
        super::register(dir.path(), Some(&index));
        assert_eq!(
            super::for_playback(&source),
            copy,
            "the player's backslash path finds the copy registered under the index's"
        );
        super::register(dir.path(), None);
        Ok(())
    }

    #[test]
    fn only_heavy_clips_get_a_proxy() -> anyhow::Result<()> {
        assert!(needed(&clip(3840, 2160, "h264")), "4K");
        assert!(needed(&clip(1920, 1080, "hevc")), "HEVC at 1080p");
        assert!(
            !needed(&clip(1920, 1080, "h264")),
            "1080p H.264 plays as it is"
        );
        Ok(())
    }

    #[test]
    fn a_proxy_keeps_the_shape_with_even_sides() -> anyhow::Result<()> {
        assert_eq!(size(3840, 2160), (1280, 720), "wide 4K");
        assert_eq!(size(2160, 3840), (720, 1280), "tall 4K");
        assert_eq!(size(1280, 720), (1280, 720), "never scaled up");
        assert_eq!(size(4096, 2160), (1280, 676), "even height");
        Ok(())
    }
}

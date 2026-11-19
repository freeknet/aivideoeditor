// ============================================================================
// Module:       index
// Description:  Indexes a folder of footage: analyses every clip (cached,
//               in parallel), scores and flags segments, and writes the
//               machine index and the human/LLM-readable shot log.
//
// Dependencies: crate::analyze, rayon, serde_json
// ============================================================================

//! The footage index.
//!
//! `ave index <footage> --work <dir>` writes:
//!
//! - `<work>/clips/<id>.json`: one [`ClipIndex`] per source, reused while
//!   the source's size and modification time are unchanged;
//! - `<work>/index.json`: every clip, with scored segments;
//! - `<work>/shotlog.md`: the same as text, one line per segment. This is
//!   what a planner reads instead of watching the footage.
//!
//! Scores and flags are relative to the whole shoot where that matters
//! (focus varies by lens and light, so "soft" means soft for this night).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::analyze::segments::{Camera, Segment, median};
use crate::analyze::{self, ClipIndex};

/// File extensions treated as video.
const VIDEO_EXT: [&str; 8] = ["mov", "mp4", "m4v", "mkv", "avi", "mts", "mxf", "webm"];

/// A segment with its shoot-relative score, flags and caption.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Scored {
    /// `<clip id>/<n>`, n from 1.
    pub(crate) id: String,
    /// Clip id.
    pub(crate) clip: String,
    /// The raw segment.
    #[serde(flatten)]
    pub(crate) seg: Segment,
    /// What carries the shot: `fire`, `led`, `light` or `none`.
    pub(crate) subject: String,
    /// Quality 0..1 (exposure, focus, steadiness, action).
    pub(crate) q: f32,
    /// Problems: `dark`, `soft`, `shaky`, `whip`, `blown`, `no-action`.
    pub(crate) flags: Vec<String>,
    /// Vision-model caption, once `ave caption` has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) caption: Option<crate::caption::Caption>,
}

/// The whole index.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Index {
    /// Footage folder that was indexed.
    pub(crate) source: String,
    /// Shoot-wide median focus of lit segments, the "normal" sharpness.
    pub(crate) sharp_ref: f32,
    /// Clips, in name order.
    pub(crate) clips: Vec<ClipIndex>,
    /// Every segment, scored, in clip then time order.
    pub(crate) segments: Vec<Scored>,
}

impl Index {
    /// Loads `<work>/index.json`.
    pub(crate) fn load(work: &Path) -> Result<Self> {
        let path = work.join("index.json");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {} (run `ave index` first)", path.display()))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Saves `<work>/index.json` and regenerates `<work>/shotlog.md`.
    pub(crate) fn save(&self, work: &Path) -> Result<()> {
        std::fs::create_dir_all(work)?;
        std::fs::write(work.join("index.json"), serde_json::to_string(self)?)?;
        std::fs::write(work.join("shotlog.md"), shotlog(self))?;
        Ok(())
    }

    /// The clip a segment belongs to.
    pub(crate) fn clip(&self, id: &str) -> Option<&ClipIndex> {
        self.clips.iter().find(|c| c.id == id)
    }
}

/// Size and mtime of a file, for cache invalidation.
fn stamp(path: &Path) -> Result<(u64, u64)> {
    let meta = std::fs::metadata(path)?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    Ok((meta.len(), mtime))
}

/// Video files directly in `dir` (not recursive), sorted by name.
pub(crate) fn list_videos(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
        let path = entry?.path();
        let is_video = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .is_some_and(|e| VIDEO_EXT.contains(&e.as_str()));
        if is_video && path.is_file() {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Analyses (or loads from cache) one clip.
fn clip_index(path: &Path, cache: &Path, force: bool) -> Result<ClipIndex> {
    let st = stamp(path)?;
    let id = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cached = cache.join(format!("{id}.json"));
    if !force
        && let Ok(text) = std::fs::read_to_string(&cached)
        && let Ok(clip) = serde_json::from_str::<ClipIndex>(&text)
        && clip.stamp == st
    {
        return Ok(clip);
    }
    let started = std::time::Instant::now();
    let clip =
        analyze::analyze_clip(path, st).with_context(|| format!("analysing {}", path.display()))?;
    log::info!(
        "{id}: {:.1}s, {} segments, analysed in {:.1}s",
        clip.probe.duration,
        clip.segments.len(),
        started.elapsed().as_secs_f32()
    );
    std::fs::write(&cached, serde_json::to_string(&clip)?)?;
    Ok(clip)
}

/// Scores every segment against the shoot.
fn score(clips: &[ClipIndex]) -> (f32, Vec<Scored>) {
    let lit: Vec<f32> = clips
        .iter()
        .flat_map(|c| c.segments.iter())
        .filter(|s| s.p95 > 0.3)
        .map(|s| s.sharp)
        .collect();
    let sharp_ref = median(&lit).max(0.05);
    let mut out = Vec::new();
    for clip in clips {
        for (k, seg) in clip.segments.iter().enumerate() {
            let mut flags = Vec::new();
            let dark = seg.p95 < 0.30 || seg.luma < 0.035;
            let rel_sharp = seg.sharp / sharp_ref;
            if dark {
                flags.push("dark".to_owned());
            }
            if rel_sharp < 0.6 {
                flags.push("soft".to_owned());
            }
            match seg.camera {
                Camera::Shaky => flags.push("shaky".to_owned()),
                Camera::Whip => flags.push("whip".to_owned()),
                Camera::Static
                | Camera::Pan
                | Camera::ZoomIn
                | Camera::ZoomOut
                | Camera::Unknown => {}
            }
            if seg.clipped > 0.12 {
                flags.push("blown".to_owned());
            }
            if seg.moving < 0.0008 {
                flags.push("no-action".to_owned());
            }
            if seg.end - seg.start < 1.0 {
                flags.push("short".to_owned());
            }
            let subject = if seg.flame > 0.003 && seg.flame >= seg.led {
                "fire"
            } else if seg.led > 0.003 {
                "led"
            } else if seg.white + seg.flame + seg.led > 0.003 {
                "light"
            } else {
                "none"
            };
            // Exposure: best with highlights between 0.5 and 0.95.
            let expo = if seg.p95 < 0.5 {
                (seg.p95 / 0.5).powf(1.5)
            } else if seg.clipped > 0.06 {
                (1.0 - (seg.clipped - 0.06) * 4.0).max(0.2)
            } else {
                1.0
            };
            let focus = rel_sharp.clamp(0.0, 1.4) / 1.4;
            let steady = match seg.camera {
                Camera::Static | Camera::ZoomIn | Camera::ZoomOut => 1.0,
                Camera::Pan => 0.85,
                Camera::Unknown => 0.5,
                Camera::Shaky => 0.35,
                Camera::Whip => 0.05,
            };
            let action = (seg.moving / 0.01).sqrt().clamp(0.0, 1.0);
            let q = expo * (0.45 + 0.55 * focus) * steady * (0.35 + 0.65 * action);
            out.push(Scored {
                id: format!("{}/{}", clip.id, k + 1),
                clip: clip.id.clone(),
                seg: seg.clone(),
                subject: subject.to_owned(),
                q,
                flags,
                caption: None,
            });
        }
    }
    (sharp_ref, out)
}

/// Indexes `source` into `work`, keeping captions from an earlier run.
pub(crate) fn run(source: &Path, work: &Path, jobs: usize, force: bool) -> Result<Index> {
    let cache = work.join("clips");
    std::fs::create_dir_all(&cache)?;
    let videos = list_videos(source)?;
    anyhow::ensure!(!videos.is_empty(), "no video files in {}", source.display());
    log::info!(
        "indexing {} clips from {} with {jobs} jobs",
        videos.len(),
        source.display()
    );
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.max(1))
        .build()?;
    let clips: Vec<ClipIndex> = pool.install(|| {
        videos
            .par_iter()
            .map(|p| clip_index(p, &cache, force))
            .collect::<Result<Vec<_>>>()
    })?;
    let (sharp_ref, mut segments) = score(&clips);
    if let Ok(old) = Index::load(work) {
        for seg in &mut segments {
            if let Some(prev) = old
                .segments
                .iter()
                .find(|o| o.id == seg.id && o.seg == seg.seg)
            {
                seg.caption.clone_from(&prev.caption);
            }
        }
    }
    let index = Index {
        source: source.display().to_string(),
        sharp_ref,
        clips,
        segments,
    };
    index.save(work)?;
    Ok(index)
}

/// Percent with one decimal, for the shot log.
fn pct(x: f32) -> String {
    format!("{:.1}%", x * 100.0)
}

/// Renders the shot log.
pub(crate) fn shotlog(index: &Index) -> String {
    let total: f64 = index.clips.iter().map(|c| c.probe.duration).sum();
    let mut s = String::new();
    let _ = writeln!(s, "# Shot log\n");
    let _ = writeln!(
        s,
        "{} clips, {} segments, {:.1} min of footage, from `{}`.\n",
        index.clips.len(),
        index.segments.len(),
        total / 60.0,
        index.source
    );
    let _ = writeln!(
        s,
        "Each line: `id` time · camera · exposure (mean/p95 luma) · focus (1.0 = this shoot's median) · \
         fire / LED share of frame · moving light · action centre x ± drift · quality 0-1 · flags.\n\
         Captions (from the vision model) follow on the next line when present.\n"
    );
    for clip in &index.clips {
        let p = &clip.probe;
        let _ = writeln!(
            s,
            "## {} · {:.1}s · {}x{} {:.3}fps {}\n",
            clip.id,
            p.duration,
            p.width,
            p.height,
            p.fps_f64(),
            p.vcodec
        );
        for seg in index.segments.iter().filter(|g| g.clip == clip.id) {
            let g = &seg.seg;
            let mut cam = g.camera.label().to_owned();
            if matches!(g.camera, Camera::ZoomIn | Camera::ZoomOut) {
                let _ = write!(cam, " x{:.2}", g.zoom_factor);
            }
            if matches!(g.camera, Camera::Pan) {
                let _ = write!(cam, " {:+.2}w", g.pan);
            }
            let flags = if seg.flags.is_empty() {
                String::new()
            } else {
                format!(" · **{}**", seg.flags.join(" "))
            };
            let _ = writeln!(
                s,
                "- `{}` {:.1}-{:.1} · {} · exp {:.2}/{:.2} · focus {:.2} · {} · fire {} · led {} · move {} · cx {:.2}±{:.2} · q {:.2}{}",
                seg.id,
                g.start,
                g.end,
                cam,
                g.luma,
                g.p95,
                g.sharp / index.sharp_ref,
                seg.subject,
                pct(g.flame),
                pct(g.led),
                pct(g.moving),
                g.cx,
                g.cx_spread / 2.0,
                seg.q,
                flags
            );
            if let Some(c) = &seg.caption {
                let _ = writeln!(s, "  - {}", c.one_line());
            }
        }
        s.push('\n');
    }
    s
}

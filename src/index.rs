// ============================================================================
// Module:       index
// Description:  Indexes a folder of footage: analyses every clip (cached,
//               in parallel), scores and flags segments, and writes the
//               machine index and the human/LLM-readable shot log.
//
// Dependencies: crate::analyze, crate::commands (progress), rayon, serde_json
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
//!
//! **The night-tuned cull.** Exposure is judged on the *lit region*, the
//! pixels above a luma floor of 0.08, not on the frame average: a sharp
//! fire spinner on black is a well-exposed shot even though 97 % of the
//! frame is black. A segment is `dark` when almost nothing is lit (lit share
//! under `min_lit`), when its lit region is dim (mean lit luma under
//! `dark`), or when nothing in it is bright (lit-region p95 under
//! `highlights`: no flame, no LED, no lit face). Every flag records the
//! measured value and the threshold it crossed (`why`), and the shot log
//! prints them.
//!
//! Defaults were calibrated on 2026-11-11 (417 segments): the old
//! frame-average rule (p95 < 0.30 or mean < 0.035) flagged 130 segments
//! dark, yet the vision model called only one of the 83 it captioned "too
//! dark". Lit-region means there sit between 0.19 and 0.29 (p5..p90) and
//! lit-region p95 between 0.45 and 0.89, so `dark = 0.18` and
//! `highlights = 0.40` flag the dim tail (about 3 %), including the one
//! segment the vision model called too dark.
//!
//! The thresholds are per night, read from `<work>/ave.toml`:
//!
//! ```toml
//! [thresholds]          # or [pages.thresholds], which survives GUI saves
//! dark = 0.18           # lit-region mean luma below this is dark
//! highlights = 0.40     # lit-region p95 below this is dark
//! min_lit = 0.003       # lit share of the frame below this is dark
//! soft = 0.6            # focus below this x the night's median is soft
//! blown = 0.12          # clipped share of the frame above this is blown
//! no_action = 0.0008    # moving-light share below this is no-action
//! short = 1.0           # segments shorter than this (s) are short
//! ```
//!
//! Each key is optional and read on its own; a bad value costs that key,
//! not the table. `[thresholds]` wins over `[pages.thresholds]` key by key.
//! Re-run `ave index` after editing: cached clips make that a re-score.
//!
//! **One bad clip does not cost the night.** A clip that cannot be read
//! (ffprobe or ffmpeg fails, no frame decodes, the file is garbage) is
//! skipped with a warning and the rest index as usual. Skipped clips are
//! recorded in `index.json` (`skipped`, with the reason) and listed at the
//! top of the shot log; they are not cached, so the next run tries them
//! again. The run fails only when every clip is skipped.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::analyze::segments::{Camera, Segment, median};
use crate::analyze::{self, ClipIndex};
use crate::commands::{ItemState, Progress, Report};

/// File extensions treated as video.
pub(crate) const VIDEO_EXT: [&str; 8] = ["mov", "mp4", "m4v", "mkv", "avi", "mts", "mxf", "webm"];

/// The technical-cull thresholds of one night (see the module docs).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Thresholds {
    /// Lit-region mean luma below this is `dark`.
    pub(crate) dark: f32,
    /// Lit-region 95th-percentile luma below this is `dark` (nothing
    /// bright: no flame, LED or lit face).
    pub(crate) highlights: f32,
    /// Lit share of the frame below this is `dark` (nothing to see).
    pub(crate) min_lit: f32,
    /// Focus below this fraction of the night's median is `soft`.
    pub(crate) soft: f32,
    /// Clipped share of the frame above this is `blown`.
    pub(crate) blown: f32,
    /// Moving-light share below this is `no-action`.
    pub(crate) no_action: f32,
    /// Segments shorter than this many seconds are `short`.
    pub(crate) short: f32,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            dark: 0.18,
            highlights: 0.40,
            min_lit: 0.003,
            soft: 0.6,
            blown: 0.12,
            no_action: 0.0008,
            short: 1.0,
        }
    }
}

impl Thresholds {
    /// Reads the thresholds from the text of an `ave.toml`, key by key;
    /// returns them with one warning per value that could not be used.
    pub(crate) fn parse(text: &str) -> (Self, Vec<String>) {
        let mut out = Self::default();
        let mut warnings = Vec::new();
        let Ok(table) = text.parse::<toml::Table>() else {
            return (
                out,
                vec!["ave.toml does not parse; default thresholds".into()],
            );
        };
        let pages = table
            .get("pages")
            .and_then(|p| p.get("thresholds"))
            .and_then(toml::Value::as_table);
        let own = table.get("thresholds").and_then(toml::Value::as_table);
        for source in [pages, own].into_iter().flatten() {
            for (key, value) in source {
                let number = value
                    .as_float()
                    .or_else(|| value.as_integer().map(|i| i as f64))
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .map(|v| v as f32);
                let slot = match key.as_str() {
                    "dark" => &mut out.dark,
                    "min_lit" => &mut out.min_lit,
                    "highlights" => &mut out.highlights,
                    "soft" => &mut out.soft,
                    "blown" => &mut out.blown,
                    "no_action" => &mut out.no_action,
                    "short" => &mut out.short,
                    other => {
                        warnings.push(format!("thresholds: unknown key `{other}`"));
                        continue;
                    }
                };
                match number {
                    Some(v) => *slot = v,
                    None => warnings.push(format!(
                        "thresholds: `{key}` must be a non-negative number, got {value}"
                    )),
                }
            }
        }
        (out, warnings)
    }

    /// The thresholds for the night whose work folder is `work`.
    pub(crate) fn for_work(work: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(work.join("ave.toml")) else {
            return Self::default();
        };
        let (t, warnings) = Self::parse(&text);
        for w in warnings {
            log::warn!("{w}");
        }
        t
    }
}

/// Why a flag was raised: the measurement and the threshold it crossed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FlagWhy {
    /// The flag (`dark`, `soft`, ...).
    pub(crate) flag: String,
    /// What was measured (`lit`, `lit_frac`, `focus`, ...).
    pub(crate) measure: String,
    /// The measured value.
    pub(crate) value: f32,
    /// The threshold it crossed.
    pub(crate) threshold: f32,
}

impl FlagWhy {
    /// `lit 0.11 < 0.18`.
    pub(crate) fn text(&self) -> String {
        let op = if self.value < self.threshold {
            "<"
        } else {
            ">"
        };
        if self.threshold < 0.01 {
            format!(
                "{} {:.2}% {op} {:.2}%",
                self.measure,
                self.value * 100.0,
                self.threshold * 100.0
            )
        } else {
            format!(
                "{} {:.2} {op} {:.2}",
                self.measure, self.value, self.threshold
            )
        }
    }
}

/// Exposure quality 0..1 from the lit region: best when what is lit has
/// a mean luma of 0.3 or more and covers at least 2 % of the frame, and
/// marked down when more than 6 % of the frame clips.
pub(crate) fn exposure_q(lit: f32, lit_frac: f32, clipped: f32) -> f32 {
    let level = (lit / 0.30).clamp(0.0, 1.0).powf(1.5);
    let extent = 0.4 + 0.6 * (lit_frac / 0.02).clamp(0.0, 1.0).sqrt();
    let blown = if clipped > 0.06 {
        (1.0 - (clipped - 0.06) * 4.0).max(0.2)
    } else {
        1.0
    };
    level * extent * blown
}

/// A segment with its shoot-relative score, flags and caption.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
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
    /// The measurement and threshold behind each flag in `flags`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) why: Vec<FlagWhy>,
    /// Vision-model caption, once `ave caption` has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) caption: Option<crate::caption::Caption>,
}

/// A clip the index could not read, and why.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Skipped {
    /// Clip id (file stem).
    pub(crate) clip: String,
    /// The error, one line.
    pub(crate) reason: String,
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
    /// The cull thresholds the flags were raised with.
    #[serde(default)]
    pub(crate) thresholds: Thresholds,
    /// Clips that could not be read, in name order; absent from `clips`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) skipped: Vec<Skipped>,
    /// The night's acts: clips grouped by recording-time gaps and setting
    /// changes, in recording order (see [`crate::analyze::sets`]).
    /// Recomputed on every save.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) sets: Vec<crate::analyze::sets::Set>,
    /// The work folder this index was loaded from or written to (not
    /// stored): how the planner finds sibling nights' plans.
    #[serde(skip)]
    pub(crate) work: Option<PathBuf>,
}

impl Index {
    /// Loads `<work>/index.json`.
    pub(crate) fn load(work: &Path) -> Result<Self> {
        let path = work.join("index.json");
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {} (run `ave index` first)", path.display()))?;
        let mut index: Self = serde_json::from_str(&text)?;
        index.work = Some(work.to_path_buf());
        Ok(index)
    }

    /// The scored segment of `clip` that covers most of `t0..t1`.
    pub(crate) fn segment_at(&self, clip: &str, t0: f32, t1: f32) -> Option<&Scored> {
        self.segments
            .iter()
            .filter(|g| g.clip == clip)
            .map(|g| (g.seg.end.min(t1) - g.seg.start.max(t0), g))
            .filter(|(overlap, _)| *overlap > 0.0)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, g)| g)
    }

    /// Saves `<work>/index.json` and regenerates `<work>/shotlog.md`.
    pub(crate) fn save(&self, work: &Path) -> Result<()> {
        std::fs::create_dir_all(work)?;
        let sets = crate::analyze::sets::group(self);
        let fresh;
        let index = if sets == self.sets {
            self
        } else {
            // Captions or times moved since the sets were grouped.
            fresh = Self {
                sets,
                ..self.clone()
            };
            &fresh
        };
        std::fs::write(work.join("index.json"), serde_json::to_string(index)?)?;
        std::fs::write(work.join("shotlog.md"), shotlog(index))?;
        Ok(())
    }

    /// The clip a segment belongs to.
    pub(crate) fn clip(&self, id: &str) -> Option<&ClipIndex> {
        self.clips.iter().find(|c| c.id == id)
    }
}

/// Size and mtime of a file, for cache invalidation.
pub(crate) fn stamp(path: &Path) -> Result<(u64, u64)> {
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

/// The clip id of a source file: its file stem.
pub(crate) fn clip_id(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What happened to one clip.
struct Analysed {
    /// Its analysis.
    clip: ClipIndex,
    /// Seconds spent analysing it; `None` when it came from the cache.
    took: Option<f32>,
}

/// Analyses (or loads from cache) one clip; `started` runs just before a
/// real analysis begins (not for a cache hit). Its cheers are measured
/// from the audio (cached apart, under `<work>/audio/`, so a cached clip
/// gains them without its video being analysed again).
fn clip_index(path: &Path, cache: &Path, force: bool, started: &dyn Fn()) -> Result<Analysed> {
    let mut a = clip_video(path, cache, force, started)?;
    let audio = cache.parent().unwrap_or(cache).join("audio");
    match analyze::cheer::cached(&audio, &a.clip.id, path, a.clip.probe.has_audio, force) {
        Ok(env) => analyze::cheer::assign(&mut a.clip.segments, &analyze::cheer::detect(&env)),
        Err(e) => log::warn!("{}: no cheers, the audio did not decode: {e:#}", a.clip.id),
    }
    Ok(a)
}

/// The video half of [`clip_index`].
fn clip_video(path: &Path, cache: &Path, force: bool, started: &dyn Fn()) -> Result<Analysed> {
    let st = stamp(path)?;
    let id = clip_id(path);
    let cached = cache.join(format!("{id}.json"));
    if !force
        && let Ok(text) = std::fs::read_to_string(&cached)
        && let Ok(clip) = serde_json::from_str::<ClipIndex>(&text)
        && clip.stamp == st
        && clip.version >= analyze::ANALYSIS_VERSION
    {
        return Ok(Analysed {
            clip: with_recording(clip, path, &cached),
            took: None,
        });
    }
    started();
    let began = std::time::Instant::now();
    let clip =
        analyze::analyze_clip(path, st).with_context(|| format!("analysing {}", path.display()))?;
    log::debug!(
        "{id}: {:.1}s, {} segments",
        clip.probe.duration,
        clip.segments.len()
    );
    std::fs::write(&cached, serde_json::to_string(&clip)?)?;
    Ok(Analysed {
        clip,
        took: Some(began.elapsed().as_secs_f32()),
    })
}

/// A cached clip from before recording times were probed gets its
/// `recorded_at` and `camera` from one ffprobe call (no re-analysis), and
/// the cache file is rewritten; a failed probe leaves the clip as it was.
fn with_recording(mut clip: ClipIndex, path: &Path, cached: &Path) -> ClipIndex {
    if clip.probe.recorded_at.is_some() {
        return clip;
    }
    match crate::media::probe(path) {
        Ok(fresh) => {
            clip.probe.recorded_at = fresh.recorded_at;
            clip.probe.camera = fresh.camera;
            match serde_json::to_string(&clip) {
                Ok(text) => {
                    if let Err(e) = std::fs::write(cached, text) {
                        log::warn!("{}: could not update the cache: {e}", clip.id);
                    }
                }
                Err(e) => log::warn!("{}: could not update the cache: {e}", clip.id),
            }
        }
        Err(e) => log::warn!("{}: no recording time: {e:#}", clip.id),
    }
    clip
}

/// Whether a segment is dark under `t`, with the reason.
fn dark(seg: &Segment, t: &Thresholds) -> Option<FlagWhy> {
    let why = |measure: &str, value: f32, threshold: f32| FlagWhy {
        flag: "dark".into(),
        measure: measure.into(),
        value,
        threshold,
    };
    if seg.lit_frac < t.min_lit {
        Some(why("lit share", seg.lit_frac, t.min_lit))
    } else if seg.lit < t.dark {
        Some(why("lit", seg.lit, t.dark))
    } else if seg.lit_p95 < t.highlights {
        Some(why("lit p95", seg.lit_p95, t.highlights))
    } else {
        None
    }
}

/// Scores every segment against the shoot.
pub(crate) fn score(clips: &[ClipIndex], t: &Thresholds) -> (f32, Vec<Scored>) {
    let lit: Vec<f32> = clips
        .iter()
        .flat_map(|c| c.segments.iter())
        .filter(|s| dark(s, t).is_none())
        .map(|s| s.sharp)
        .collect();
    let sharp_ref = median(&lit).max(0.05);
    let mut out = Vec::new();
    for clip in clips {
        let marks = analyze::spans::marks(&clip.samples, sharp_ref, t);
        for (k, seg) in clip.segments.iter().enumerate() {
            let mut why: Vec<FlagWhy> = Vec::new();
            let mut raise = |flag: &str, measure: &str, value: f32, threshold: f32| {
                why.push(FlagWhy {
                    flag: flag.into(),
                    measure: measure.into(),
                    value,
                    threshold,
                });
            };
            let rel_sharp = seg.sharp / sharp_ref;
            if let Some(d) = dark(seg, t) {
                raise(&d.flag, &d.measure, d.value, d.threshold);
            }
            if rel_sharp < t.soft {
                raise("soft", "focus", rel_sharp, t.soft);
            }
            match seg.camera {
                Camera::Shaky => raise("shaky", "jitter", seg.jitter, 0.22),
                Camera::Whip => raise("whip", "camera", 1.0, 0.0),
                Camera::Static
                | Camera::Pan
                | Camera::ZoomIn
                | Camera::ZoomOut
                | Camera::Unknown => {}
            }
            if seg.clipped > t.blown {
                raise("blown", "clipped", seg.clipped, t.blown);
            }
            if seg.moving < t.no_action {
                raise("no-action", "moving", seg.moving, t.no_action);
            }
            if seg.end - seg.start < t.short {
                raise("short", "length", seg.end - seg.start, t.short);
            }
            let flags: Vec<String> = why.iter().map(|w| w.flag.clone()).collect();
            let subject = if seg.flame > 0.003 && seg.flame >= seg.led {
                "fire"
            } else if seg.led > 0.003 {
                "led"
            } else if seg.white + seg.flame + seg.led > 0.003 {
                "light"
            } else {
                "none"
            };
            let expo = exposure_q(seg.lit, seg.lit_frac, seg.clipped);
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
            let mut seg = seg.clone();
            seg.bad_spans = analyze::spans::within(&marks, &seg, &flags);
            out.push(Scored {
                id: format!("{}/{}", clip.id, k + 1),
                clip: clip.id.clone(),
                seg,
                subject: subject.to_owned(),
                q,
                flags,
                why,
                caption: None,
            });
        }
    }
    (sharp_ref, out)
}

/// Splits per-clip results into the clips that indexed and the ones to
/// skip, in input order. Fails when nothing indexed: a night with no
/// readable clip is an error, not an empty shot log.
fn gather(results: Vec<(String, Result<ClipIndex>)>) -> Result<(Vec<ClipIndex>, Vec<Skipped>)> {
    let mut clips = Vec::new();
    let mut skipped = Vec::new();
    for (id, result) in results {
        match result {
            Ok(clip) => clips.push(clip),
            Err(e) => {
                let reason = format!("{e:#}")
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                log::warn!("skipping {id}: {reason}");
                skipped.push(Skipped { clip: id, reason });
            }
        }
    }
    if clips.is_empty() {
        let first = skipped
            .first()
            .map(|s| format!(" (first: {}: {})", s.clip, s.reason))
            .unwrap_or_default();
        anyhow::bail!(
            "every clip failed to index: {} skipped{first}",
            skipped.len()
        );
    }
    Ok((clips, skipped))
}

/// Indexes `source` into `work`, keeping captions from an earlier run.
///
/// Reports every clip to `progress` (`index 12/81 MVI_7925: done 3.2s`) and
/// stops between clips when it is cancelled; clips analysed before the stop
/// stay in the cache, so the next run picks up where this one stopped.
/// A clip that fails is reported `failed` with its reason and skipped (see
/// the module docs); the run fails only when every clip does.
pub(crate) fn run(
    source: &Path,
    work: &Path,
    jobs: usize,
    force: bool,
    progress: &Progress,
) -> Result<Index> {
    let cache = work.join("clips");
    std::fs::create_dir_all(&cache)?;
    let videos = list_videos(source)?;
    anyhow::ensure!(!videos.is_empty(), "no video files in {}", source.display());
    log::info!(
        "indexing {} clips from {} with {jobs} jobs",
        videos.len(),
        source.display()
    );
    progress.stage("index", format!("{} clips to check", videos.len()));
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs.max(1))
        .build()?;
    let total = videos.len();
    let finished = AtomicUsize::new(0);
    let analysed = AtomicUsize::new(0);
    let began = Instant::now();
    let one = |path: &PathBuf| -> (String, Result<ClipIndex>) {
        let id = clip_id(path);
        if let Err(e) = progress.check() {
            return (id, Err(e));
        }
        let start = || {
            let n = finished.load(Ordering::Relaxed);
            progress.report(&Report::item(
                "index",
                n,
                total,
                id.as_str(),
                ItemState::Started,
            ));
        };
        let result = clip_index(path, &cache, force, &start);
        let n = finished.fetch_add(1, Ordering::Relaxed) + 1;
        match &result {
            Ok(Analysed { took: None, .. }) => {
                progress.report(&Report::item(
                    "index",
                    n,
                    total,
                    id.as_str(),
                    ItemState::Cached,
                ));
            }
            Ok(Analysed {
                took: Some(secs), ..
            }) => {
                let done = analysed.fetch_add(1, Ordering::Relaxed) + 1;
                let elapsed = began.elapsed().as_secs_f32();
                // Wall seconds per analysed clip (parallelism included),
                // assuming the clips left all need real analysis.
                let eta = elapsed / done as f32 * total.saturating_sub(n) as f32;
                progress.report(
                    &Report::item("index", n, total, id.as_str(), ItemState::Done)
                        .took(*secs)
                        .remaining(Some(eta)),
                );
            }
            Err(e) if !progress.cancelled() => {
                progress.report(
                    &Report::item("index", n, total, id.as_str(), ItemState::Failed)
                        .because(&format!("{e:#}")),
                );
            }
            Err(_) => {}
        }
        (id, result.map(|a| a.clip))
    };
    let results: Vec<(String, Result<ClipIndex>)> =
        pool.install(|| videos.par_iter().map(one).collect());
    progress.check()?;
    let (clips, skipped) = gather(results)?;
    progress.stage("index", "scoring segments");
    let thresholds = Thresholds::for_work(work);
    let (sharp_ref, mut segments) = score(&clips, &thresholds);
    if let Ok(old) = Index::load(work) {
        for seg in &mut segments {
            // Same segment: same id and time range (the measurements may
            // have moved on with a new analysis version; the picture did not).
            if let Some(prev) = old.segments.iter().find(|o| {
                o.id == seg.id
                    && (o.seg.start - seg.seg.start).abs() < 1e-3
                    && (o.seg.end - seg.seg.end).abs() < 1e-3
            }) {
                seg.caption.clone_from(&prev.caption);
            }
        }
    }
    let mut index = Index {
        source: source.display().to_string(),
        sharp_ref,
        clips,
        segments,
        thresholds,
        skipped,
        sets: Vec::new(),
        work: Some(work.to_path_buf()),
    };
    index.sets = crate::analyze::sets::group(&index);
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
    if !index.skipped.is_empty() {
        let _ = writeln!(
            s,
            "**Skipped {} clip(s)** that could not be read (not in this log; re-run `ave index` to retry):\n",
            index.skipped.len()
        );
        for k in &index.skipped {
            let _ = writeln!(s, "- `{}`: {}", k.clip, k.reason);
        }
        s.push('\n');
    }
    let _ = writeln!(
        s,
        "Each line: `id` time · camera · exposure (mean/p95 luma, then mean luma of the lit region and how much \
         of the frame it covers) · focus (1.0 = this shoot's median) · fire / LED share of frame · moving light · \
         biggest burst (fire/LED onset) @ best-moment time · crowd cheer from the camera audio \
         (strength 0-1 @ onset) · action centre x ± drift · quality 0-1 · flags \
         (measured value vs threshold).\n\
         A `bad:` line lists short bad stretches inside a segment (a focus hunt, a whip, a jolt) with their \
         times and worst value.\n\
         Captions (from the vision model) follow on the next line when present.\n"
    );
    let t = &index.thresholds;
    let _ = writeln!(
        s,
        "Thresholds: dark lit < {:.2}, lit p95 < {:.2} or lit share < {} · soft focus < {:.2} · \
         blown clipped > {} · no-action moving < {}.\n",
        t.dark,
        t.highlights,
        pct(t.min_lit),
        t.soft,
        pct(t.blown),
        pct(t.no_action)
    );
    let mut cameras: Vec<(&str, usize)> = Vec::new();
    for c in index.clips.iter().filter_map(|c| c.probe.camera.as_deref()) {
        match cameras.iter_mut().find(|(k, _)| *k == c) {
            Some(slot) => slot.1 += 1,
            None => cameras.push((c, 1)),
        }
    }
    if !cameras.is_empty() {
        let list: Vec<String> = cameras.iter().map(|(c, n)| format!("{c} ({n})")).collect();
        let _ = writeln!(s, "Cameras: {}.\n", list.join(", "));
    }
    if !index.sets.is_empty() {
        let _ = writeln!(
            s,
            "Sets (the night's acts: clips in recording order, split by pauses over {} min and by lasting \
             indoor/outdoor changes in the captions; times on the camera clock):\n",
            crate::analyze::sets::GAP_SECS / 60
        );
        for set in &index.sets {
            let _ = writeln!(s, "- {}", set.heading());
        }
        s.push('\n');
    }
    // Clips under their sets in recording order, or by name with no sets.
    let mut order: Vec<(Option<&crate::analyze::sets::Set>, &ClipIndex)> = Vec::new();
    for set in &index.sets {
        order.extend(
            set.clips
                .iter()
                .filter_map(|id| index.clip(id))
                .map(|c| (Some(set), c)),
        );
    }
    order.extend(
        index
            .clips
            .iter()
            .filter(|c| !order.iter().any(|(_, o)| o.id == c.id))
            .map(|c| (None, c))
            .collect::<Vec<_>>(),
    );
    let level = if index.sets.is_empty() { "##" } else { "###" };
    let mut heading_of: Option<usize> = None;
    for (set, clip) in order {
        if let Some(set) = set
            && heading_of != Some(set.n)
        {
            heading_of = Some(set.n);
            let _ = writeln!(s, "## {}\n", set.heading());
        }
        let p = &clip.probe;
        let mut when = String::new();
        if let Some(r) = &p.recorded_at {
            let _ = write!(when, " · {}", r.clock());
        }
        if cameras.len() > 1
            && let Some(c) = &p.camera
        {
            let _ = write!(when, " · {c}");
        }
        let _ = writeln!(
            s,
            "{level} {}{when} · {:.1}s · {}x{} {:.3}fps {}\n",
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
                let reasons: Vec<String> = seg
                    .why
                    .iter()
                    .filter(|w| w.flag != "short" && w.flag != "whip")
                    .map(FlagWhy::text)
                    .collect();
                if reasons.is_empty() {
                    format!(" · **{}**", seg.flags.join(" "))
                } else {
                    format!(" · **{}** ({})", seg.flags.join(" "), reasons.join(", "))
                }
            };
            let peak = if g.moments.is_empty() {
                String::new()
            } else {
                format!(
                    " · burst {} @{:.2} ({} moments)",
                    pct(g.burst),
                    g.peak_t,
                    g.moments.len()
                )
            };
            let cheer = match g
                .cheers
                .iter()
                .max_by(|a, b| a.strength.total_cmp(&b.strength))
            {
                Some(c) if g.cheers.len() > 1 => format!(
                    " · cheer {:.2} @{:.2} ({} cheers)",
                    c.strength,
                    c.t,
                    g.cheers.len()
                ),
                Some(c) => format!(" · cheer {:.2} @{:.2}", c.strength, c.t),
                None => String::new(),
            };
            let _ = writeln!(
                s,
                "- `{}` {:.1}-{:.1} · {} · exp {:.2}/{:.2} lit {:.2} over {} · focus {:.2} · {} · fire {} · led {} · move {}{}{} · cx {:.2}±{:.2} · q {:.2}{}",
                seg.id,
                g.start,
                g.end,
                cam,
                g.luma,
                g.p95,
                g.lit,
                pct(g.lit_frac),
                g.sharp / index.sharp_ref,
                seg.subject,
                pct(g.flame),
                pct(g.led),
                pct(g.moving),
                peak,
                cheer,
                g.cx,
                g.cx_spread / 2.0,
                seg.q,
                flags
            );
            if !g.bad_spans.is_empty() {
                let spans: Vec<String> = g.bad_spans.iter().map(|b| b.text()).collect();
                let _ = writeln!(s, "  - bad: {}", spans.join(", "));
            }
            if let Some(c) = &seg.caption {
                let _ = writeln!(s, "  - {}", c.one_line());
            }
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::{Index, Skipped, Thresholds, gather, score, shotlog};
    use crate::analyze::ClipIndex;
    use crate::analyze::segments::{Camera, Segment};
    use crate::media::Probe;

    #[test]
    fn thresholds_are_read_key_by_key() -> anyhow::Result<()> {
        let (t, warnings) = Thresholds::parse(
            r#"
footage = "x"
[pages.thresholds]
dark = 0.2
soft = 0.5
[thresholds]
dark = 0.1
blown = "lots"
min_lit = 0
"#,
        );
        assert!((t.dark - 0.1).abs() < 1e-6, "[thresholds] wins: {t:?}");
        assert!(
            (t.soft - 0.5).abs() < 1e-6,
            "[pages.thresholds] is read: {t:?}"
        );
        assert!(t.min_lit.abs() < 1e-6, "an integer is a number: {t:?}");
        assert!(
            (t.blown - Thresholds::default().blown).abs() < 1e-6,
            "a bad value keeps the default: {t:?}"
        );
        assert_eq!(
            warnings.len(),
            1,
            "one warning for the bad value: {warnings:?}"
        );
        let (none, _) = Thresholds::parse("");
        assert_eq!(none, Thresholds::default(), "no table, defaults");
        Ok(())
    }

    /// A one-segment clip with the given lit-region numbers.
    fn clip(id: &str, luma: f32, p95: f32, lit: f32, lit_frac: f32) -> ClipIndex {
        let seg = Segment {
            start: 0.0,
            end: 4.0,
            camera: Camera::Static,
            after_cut: false,
            luma,
            p95,
            clipped: 0.0,
            sharp: 1.0,
            flame: 0.02,
            led: 0.0,
            white: 0.0,
            moving: 0.01,
            cx: 0.5,
            cx_spread: 0.1,
            cy: 0.5,
            zoom_factor: 1.0,
            pan: 0.0,
            jitter: 0.0,
            lit,
            lit_frac,
            lit_p95: 0.9,
            burst: 0.0,
            peak_t: 2.0,
            moments: Vec::new(),
            cheers: Vec::new(),
            sig: Default::default(),
            bad_spans: Vec::new(),
        };
        ClipIndex {
            id: id.into(),
            path: String::new(),
            stamp: (0, 0),
            probe: Probe {
                duration: 4.0,
                width: 1920,
                height: 1080,
                fps: [24, 1],
                vcodec: String::new(),
                has_audio: false,
                recorded_at: None,
                camera: None,
            },
            samples: Vec::new(),
            segments: vec![seg],
            hist: Vec::new(),
            version: crate::analyze::ANALYSIS_VERSION,
        }
    }

    #[test]
    fn a_fire_spinner_on_black_is_not_dark() -> anyhow::Result<()> {
        let clips = [
            // Frame mean 0.02 and p95 0.05 (the old rule called it dark),
            // but the 3 % that is lit is bright.
            clip("spinner", 0.02, 0.05, 0.45, 0.03),
            // Murky: lots of the frame just above the floor.
            clip("murk", 0.09, 0.14, 0.11, 0.6),
            // Black with a pinprick.
            clip("black", 0.005, 0.01, 0.5, 0.0005),
        ];
        let (_, scored) = score(&clips, &Thresholds::default());
        let flags = |id: &str| {
            scored
                .iter()
                .find(|s| s.clip == id)
                .map(|s| s.flags.clone())
                .unwrap_or_default()
        };
        assert!(
            !flags("spinner").contains(&"dark".to_owned()),
            "spinner: {:?}",
            flags("spinner")
        );
        assert!(flags("murk").contains(&"dark".to_owned()), "murk is dark");
        assert!(flags("black").contains(&"dark".to_owned()), "black is dark");
        let murk = scored
            .iter()
            .find(|s| s.clip == "murk")
            .and_then(|s| s.why.iter().find(|w| w.flag == "dark"))
            .ok_or_else(|| anyhow::anyhow!("no reason recorded"))?;
        assert!(
            murk.measure == "lit" && (murk.threshold - 0.18).abs() < 1e-6,
            "the flag carries value and threshold: {murk:?}"
        );
        assert_eq!(murk.text(), "lit 0.11 < 0.18", "printable");
        // A stricter night setting flags the spinner too.
        let strict = Thresholds {
            min_lit: 0.05,
            ..Thresholds::default()
        };
        let (_, scored) = score(&clips, &strict);
        assert!(
            scored
                .iter()
                .any(|s| s.clip == "spinner" && s.flags.contains(&"dark".to_owned())),
            "thresholds are honoured"
        );
        Ok(())
    }

    #[test]
    fn a_bad_clip_is_skipped_and_every_bad_clip_fails() -> anyhow::Result<()> {
        let results = vec![
            ("A".to_owned(), Ok(clip("A", 0.1, 0.5, 0.4, 0.05))),
            (
                "B".to_owned(),
                Err(anyhow::anyhow!("ffprobe exited 1").context("analysing B.mp4")),
            ),
            ("C".to_owned(), Ok(clip("C", 0.1, 0.5, 0.4, 0.05))),
        ];
        let (clips, skipped) = gather(results)?;
        let ids: Vec<&str> = clips.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["A", "C"], "the good clips index, in order");
        assert_eq!(
            skipped,
            [Skipped {
                clip: "B".into(),
                reason: "analysing B.mp4: ffprobe exited 1".into(),
            }],
            "the bad one is recorded with its reason on one line"
        );
        let all_bad = vec![
            ("A".to_owned(), Err(anyhow::anyhow!("no frames decoded"))),
            ("B".to_owned(), Err(anyhow::anyhow!("ffprobe exited 1"))),
        ];
        let err = gather(all_bad)
            .err()
            .ok_or_else(|| anyhow::anyhow!("every clip failing must fail the run"))?;
        let text = format!("{err:#}");
        assert!(
            text.contains("2 skipped") && text.contains("no frames decoded"),
            "{text}"
        );
        Ok(())
    }

    #[test]
    fn old_index_files_load_and_skips_reach_the_shotlog() -> anyhow::Result<()> {
        let old: Index =
            serde_json::from_str(r#"{"source":"x","sharp_ref":1.0,"clips":[],"segments":[]}"#)?;
        assert!(old.skipped.is_empty(), "no `skipped` field, none skipped");
        let index = Index {
            skipped: vec![Skipped {
                clip: "BROKEN".into(),
                reason: "no frames decoded".into(),
            }],
            ..Index::default()
        };
        let log = shotlog(&index);
        let head: String = log.lines().take(8).collect::<Vec<_>>().join("\n");
        assert!(
            head.contains("Skipped 1 clip") && head.contains("`BROKEN`: no frames decoded"),
            "listed at the top: {head}"
        );
        let back: Index = serde_json::from_str(&serde_json::to_string(&index)?)?;
        assert_eq!(
            back.skipped, index.skipped,
            "round-trips through index.json"
        );
        Ok(())
    }

    /// Writes a 2 s test clip, or `None` when ffmpeg is missing.
    fn test_clip(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
        let out = dir.join(name);
        let status = std::process::Command::new(crate::media::tool("ffmpeg"))
            .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"])
            .arg("testsrc=size=320x180:rate=24:duration=2")
            .args(["-pix_fmt", "yuv420p"])
            .arg(&out)
            .status()
            .ok()?;
        status.success().then_some(out)
    }

    #[test]
    fn a_garbage_clip_does_not_stop_the_night() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let footage = dir.path().join("video");
        std::fs::create_dir_all(&footage)?;
        if test_clip(&footage, "GOOD.mp4").is_none() {
            eprintln!("ffmpeg not available; skipping");
            return Ok(());
        }
        std::fs::write(footage.join("BAD.mp4"), b"this is not a video at all")?;
        let work = dir.path().join("_ave");
        let mut out = Vec::new();
        crate::commands::execute_with(
            crate::commands::Cmd::Index {
                footage: footage.clone(),
                work: work.clone(),
                jobs: 2,
                force: false,
            },
            &mut out,
            &crate::commands::Progress::default(),
        )?;
        let printed = String::from_utf8_lossy(&out);
        assert!(printed.starts_with("1 clips, "), "{printed}");
        assert!(printed.contains(", 1 skipped -> "), "{printed}");
        let index = Index::load(&work)?;
        let ids: Vec<&str> = index.clips.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["GOOD"], "the good clip indexed");
        assert_eq!(
            index
                .skipped
                .iter()
                .map(|k| k.clip.as_str())
                .collect::<Vec<_>>(),
            ["BAD"],
            "the garbage one is recorded"
        );
        let log = std::fs::read_to_string(work.join("shotlog.md"))?;
        assert!(log.contains("`BAD`:"), "the shot log lists it: {log}");
        // Only garbage: the run fails.
        std::fs::remove_file(footage.join("GOOD.mp4"))?;
        let all_bad = crate::commands::execute_with(
            crate::commands::Cmd::Index {
                footage,
                work,
                jobs: 1,
                force: false,
            },
            &mut Vec::new(),
            &crate::commands::Progress::default(),
        );
        assert!(all_bad.is_err(), "every clip failing is an error");
        Ok(())
    }
}

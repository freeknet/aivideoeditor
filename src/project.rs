// ============================================================================
// Module:       project
// Description:  An open night: its work folder, its settings file, and the
//               engine artefacts loaded from it (index, music, people, plans);
//               per-clip status, measured speeds and analysis estimates.
//
// Dependencies: crate::{index, beats, people, plan, recipe, media}, serde,
//               serde_json, toml
// ============================================================================

//! The open project ("night").
//!
//! A project is a work folder (a night's `_ave`) plus `ave.toml` beside the
//! engine files. Every page reads the same [`Project`]; a page that writes a
//! file (people, a plan, review marks) reloads just that part afterwards, so
//! the GUI and the command line always agree on what is on disk.
//!
//! Loading never fails as a whole: a night that has been indexed but not
//! captioned or beat-mapped opens with those parts `None`, and the Night page
//! shows which pipeline steps are still to run.
//!
//! **Status and estimates.** [`NightStatus`] (`<work>/status.json`) keeps
//! what the cache and shot log cannot: why a clip failed, and how fast this
//! machine indexed and captioned. [`speeds`] picks the newest measurement
//! (this night, else a sibling night, else the cache files' timestamps or
//! the caption log, else an assumption it names), and [`estimate_index`] /
//! [`estimate_caption`] turn footage length into expected wall time.
//! [`clip_rows`] is the per-clip list a reopened night shows.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::beats::MusicMap;
use crate::index::Index;
use crate::people::People;

/// The night's own settings, `<work>/ave.toml`. Every field is optional so
/// a hand-written or older file still loads.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Settings {
    /// What the project is called, when not its folder's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    /// Folder of source clips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) footage: Option<PathBuf>,
    /// The approved music track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) track: Option<PathBuf>,
    /// End card image per aspect key (`9x16`, `4x5`, `16x9`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) end_cards: BTreeMap<String, PathBuf>,
    /// Free-form per-page settings, keyed by page, so a page can persist
    /// its own options without changing this struct.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) pages: BTreeMap<String, toml::Table>,
}

/// The music map for `work`: `music.json`, or, when there is none and no
/// track is set, the no-music stand-in ([`MusicMap::paced`]). A track that
/// is set but not mapped is an error: cut to it after mapping it.
pub(crate) fn music_for(work: &Path) -> Result<MusicMap> {
    if work.join("music.json").exists() {
        return MusicMap::load(work);
    }
    let track = std::fs::read_to_string(work.join("ave.toml"))
        .ok()
        .and_then(|text| toml::from_str::<Settings>(&text).ok())
        .and_then(|s| s.track);
    match track {
        Some(track) => anyhow::bail!(
            "{} is set as the track but not mapped yet: run `ave beats` on it first",
            track.display()
        ),
        None => Ok(MusicMap::paced()),
    }
}

/// The work folder for a folder the user picked.
///
/// - an `_ave` folder, or one holding `index.json` / `ave.toml`, is the work folder;
/// - a folder with an `_ave` inside opens that;
/// - a footage folder gets an `_ave` of its own, except the
///   `<project>/originals/video` layout, which gets `<project>/_ave`.
///
/// Only that exact layout climbs out of the picked folder: a folder that
/// just happens to be called `Videos` (Windows' own library folder, say)
/// keeps its project inside it, never in the folder above.
pub(crate) fn work_dir_for(picked: &Path) -> PathBuf {
    let is_work = picked.file_name().is_some_and(|n| n == "_ave")
        || picked.join("index.json").exists()
        || picked.join("ave.toml").exists();
    if is_work {
        return picked.to_path_buf();
    }
    if picked.join("_ave").is_dir() {
        return picked.join("_ave");
    }
    let is = |p: &Path, name: &str| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name));
    let originals = picked.parent().filter(|p| is(p, "originals"));
    if (is(picked, "video") || is(picked, "videos"))
        && let Some(project) = originals.and_then(Path::parent)
    {
        return project.join("_ave");
    }
    if is(picked, "originals")
        && let Some(project) = picked.parent()
    {
        return project.join("_ave");
    }
    picked.join("_ave")
}

/// Where the clips are when `folder` itself holds none: the nearest
/// subfolder (two levels down at most, the first by name) that does, with
/// how many it holds. Camera cards put clips in `DCIM/100CANON`, and a
/// picked parent folder is the commonest wrong pick.
pub(crate) fn clips_below(folder: &Path) -> Option<(PathBuf, usize)> {
    let subfolders = |dir: &Path| -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.is_dir() && p.file_name().is_none_or(|n| n != "_ave"))
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    };
    let count = |dir: &Path| crate::index::list_videos(dir).map_or(0, |v| v.len());
    for child in subfolders(folder) {
        let n = count(&child);
        if n > 0 {
            return Some((child, n));
        }
    }
    for child in subfolders(folder) {
        for grandchild in subfolders(&child) {
            let n = count(&grandchild);
            if n > 0 {
                return Some((grandchild, n));
            }
        }
    }
    None
}

/// Why `folder` cannot be a project's footage, if it cannot: it is missing
/// or holds no video clips. The message names a subfolder with clips when
/// there is one, so the fix is one click away.
pub(crate) fn footage_problem(folder: &Path) -> Option<String> {
    let name = folder.display();
    match crate::index::list_videos(folder) {
        Err(_) => Some(format!("{name} could not be read.")),
        Ok(clips) if !clips.is_empty() => None,
        Ok(_) => Some(match clips_below(folder) {
            Some((sub, n)) => format!(
                "{name} has no video clips in it, but {} has {n}. Pick that folder instead.",
                sub.display()
            ),
            None => format!(
                "{name} has no video clips in it ({}). Pick the folder your clips are in.",
                crate::index::VIDEO_EXT.join(", ")
            ),
        }),
    }
}

/// An open night.
#[derive(Debug)]
pub(crate) struct Project {
    /// The work folder (`…/<night>/_ave`).
    pub(crate) work: PathBuf,
    /// `ave.toml`.
    pub(crate) settings: Settings,
    /// The shot log, once `index` has run.
    pub(crate) index: Option<Index>,
    /// The music map, once `beats` has run.
    pub(crate) music: Option<MusicMap>,
    /// Who is in which clip (empty until drafted).
    pub(crate) people: People,
    /// Plan files in `<work>/plans`, sorted by name.
    pub(crate) plans: Vec<PathBuf>,
    /// Problems met while loading, shown on the Night page.
    pub(crate) warnings: Vec<String>,
}

impl Project {
    /// Opens the night whose work folder is `work` (created if missing).
    pub(crate) fn open(work: &Path) -> Result<Self> {
        std::fs::create_dir_all(work).with_context(|| format!("creating {}", work.display()))?;
        let mut project = Self {
            work: work.to_path_buf(),
            settings: Settings::default(),
            index: None,
            music: None,
            people: People::default(),
            plans: Vec::new(),
            warnings: Vec::new(),
        };
        project.reload();
        Ok(project)
    }

    /// The project's name: the one set in its settings, else the folder above
    /// `_ave`, or the work folder's own.
    pub(crate) fn name(&self) -> String {
        if let Some(name) = self
            .settings
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
        {
            return name.to_owned();
        }
        self.folder_name()
    }

    /// The name the project's folder gives it: the folder above `_ave`, or
    /// the work folder's own.
    pub(crate) fn folder_name(&self) -> String {
        let named = if self.work.file_name().is_some_and(|n| n == "_ave") {
            self.work.parent()
        } else {
            Some(self.work.as_path())
        };
        named
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.work.display().to_string())
    }

    /// Re-reads everything from disk.
    pub(crate) fn reload(&mut self) {
        self.warnings.clear();
        self.reload_settings();
        self.reload_index();
        self.reload_music();
        self.reload_people();
        self.reload_plans();
    }

    /// Re-reads `ave.toml`.
    pub(crate) fn reload_settings(&mut self) {
        let path = self.work.join("ave.toml");
        self.settings = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|e| {
                self.warnings.push(format!("ave.toml: {e}"));
                Settings::default()
            }),
            Err(_) => Settings::default(),
        };
        if self.settings.footage.is_none()
            && let Some(index) = &self.index
        {
            self.settings.footage = Some(PathBuf::from(&index.source));
        }
    }

    /// Writes `ave.toml`.
    pub(crate) fn save_settings(&self) -> Result<()> {
        let text = toml::to_string_pretty(&self.settings)?;
        std::fs::write(self.work.join("ave.toml"), text)?;
        Ok(())
    }

    /// Re-reads `index.json`.
    pub(crate) fn reload_index(&mut self) {
        self.index = if self.work.join("index.json").exists() {
            Index::load(&self.work)
                .map_err(|e| self.warnings.push(format!("index: {e:#}")))
                .ok()
        } else {
            None
        };
        crate::proxy::register(&self.work, self.index.as_ref());
        if self.settings.footage.is_none()
            && let Some(index) = &self.index
        {
            self.settings.footage = Some(PathBuf::from(&index.source));
        }
    }

    /// Whether this project is cut without music: no track, and the paced
    /// stand-in ([`MusicMap::paced`]) in place of a beat map.
    pub(crate) fn no_music(&self) -> bool {
        self.settings.track.is_none() && self.music.as_ref().is_some_and(MusicMap::no_music)
    }

    /// Re-reads `music.json`. A project with no `music.json` and no track
    /// is cut without music: it gets the paced stand-in, so nothing waits
    /// on a music step nobody asked for. With a track it waits to be mapped.
    pub(crate) fn reload_music(&mut self) {
        self.music = if self.work.join("music.json").exists() {
            MusicMap::load(&self.work)
                .map_err(|e| self.warnings.push(format!("music: {e:#}")))
                .ok()
        } else if self.settings.track.is_none() {
            Some(MusicMap::paced())
        } else {
            None
        };
        // A night mapped from the command line has its track only in the map.
        if self.settings.track.is_none()
            && let Some(music) = &self.music
            && !music.no_music()
        {
            self.settings.track = Some(PathBuf::from(&music.path));
        }
    }

    /// Re-reads `people.toml`.
    pub(crate) fn reload_people(&mut self) {
        self.people = People::load(&self.work).unwrap_or_else(|e| {
            self.warnings.push(format!("people: {e:#}"));
            People::default()
        });
    }

    /// Re-lists `plans/*.toml`.
    pub(crate) fn reload_plans(&mut self) {
        let dir = self.work.join("plans");
        let mut plans: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                    .collect()
            })
            .unwrap_or_default();
        plans.sort();
        self.plans = plans;
    }
}

/// `<work>/status.json`: per-clip status and measured throughput.
pub(crate) const STATUS_FILE: &str = "status.json";

/// The quality floor the caption step uses by default (`ave caption
/// --min-q`), for counting what it will caption.
pub(crate) const CAPTION_MIN_Q: f32 = 0.15;

/// Where one clip stands in indexing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClipState {
    /// Not analysed yet.
    #[default]
    Pending,
    /// Being analysed (only while a run is going).
    Analysing,
    /// Analysed; in the cache.
    Indexed,
    /// Analysis failed; see the reason.
    IndexFailed,
}

/// One clip's recorded status.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ClipStatus {
    /// Where it stands.
    #[serde(default)]
    pub(crate) state: ClipState,
    /// Why the last analysis failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    /// Seconds its analysis took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) secs: Option<f32>,
}

/// How fast this machine ran the slow steps, measured on real runs.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Throughput {
    /// Wall seconds of indexing per second of newly analysed footage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) index_ratio: Option<f32>,
    /// Wall seconds per captioned segment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) caption_secs: Option<f32>,
    /// Segments per second of footage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) segments_per_sec: Option<f32>,
    /// Share of segments the caption step takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) captionable: Option<f32>,
}

/// `<work>/status.json`. Written by index and caption runs; read by Home to
/// show what is done and to estimate what is left. Losing it costs the
/// failure reasons and measured speeds, nothing else: what is indexed and
/// captioned is always derived from the cache and the shot log.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct NightStatus {
    /// Per clip id.
    #[serde(default)]
    pub(crate) clips: BTreeMap<String, ClipStatus>,
    /// Segments whose last caption attempt failed, with the reason.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) caption_failed: BTreeMap<String, String>,
    /// Measured speeds.
    #[serde(default)]
    pub(crate) throughput: Throughput,
}

impl NightStatus {
    /// Reads `<work>/status.json`; empty when missing or unreadable.
    pub(crate) fn load(work: &Path) -> Self {
        std::fs::read_to_string(work.join(STATUS_FILE))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Writes `<work>/status.json` through a temp file and a rename.
    pub(crate) fn save(&self, work: &Path) -> Result<()> {
        std::fs::create_dir_all(work)?;
        let tmp = work.join(format!("{STATUS_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, work.join(STATUS_FILE))?;
        Ok(())
    }

    /// Notes one progress report.
    pub(crate) fn observe(
        &mut self,
        step: &str,
        item: &str,
        state: crate::commands::ItemState,
        secs: Option<f32>,
        reason: Option<&str>,
    ) {
        use crate::commands::ItemState as S;
        match step {
            "index" => {
                let clip = self.clips.entry(item.to_owned()).or_default();
                match state {
                    S::Queued => clip.state = ClipState::Pending,
                    S::Started => clip.state = ClipState::Analysing,
                    S::Cached | S::Done => {
                        clip.state = ClipState::Indexed;
                        clip.reason = None;
                        if secs.is_some() {
                            clip.secs = secs;
                        }
                    }
                    S::Failed => {
                        clip.state = ClipState::IndexFailed;
                        clip.reason = reason.map(str::to_owned);
                    }
                }
            }
            "caption" => match state {
                S::Failed => {
                    self.caption_failed
                        .insert(item.to_owned(), reason.unwrap_or_default().to_owned());
                }
                S::Done => {
                    self.caption_failed.remove(item);
                }
                S::Queued | S::Started | S::Cached => {}
            },
            _ => {}
        }
    }

    /// Ends a run: a clip still marked as being analysed was interrupted.
    pub(crate) fn settle(&mut self) {
        for clip in self.clips.values_mut() {
            if clip.state == ClipState::Analysing {
                clip.state = ClipState::Pending;
            }
        }
    }

    /// Records speeds and densities from a finished index: `wall` seconds
    /// spent analysing `source` seconds of new footage.
    pub(crate) fn measure_index(&mut self, index: &Index, wall: Option<f32>, source: f64) {
        if let Some(wall) = wall
            && source >= 20.0
        {
            self.throughput.index_ratio = Some(wall / source as f32);
        }
        let total: f64 = index.clips.iter().map(|c| c.probe.duration).sum();
        if total > 0.0 && !index.segments.is_empty() {
            self.throughput.segments_per_sec = Some((index.segments.len() as f64 / total) as f32);
            let takes = index.segments.iter().filter(|s| captionable(s)).count();
            self.throughput.captionable = Some(takes as f32 / index.segments.len() as f32);
        }
    }
}

/// Whether the caption step (at its default settings) captions `seg`.
pub(crate) fn captionable(seg: &crate::index::Scored) -> bool {
    seg.q >= CAPTION_MIN_Q && seg.seg.end - seg.seg.start >= 1.0
}

/// A measured (or assumed) speed and where it came from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Basis {
    /// The number.
    pub(crate) value: f32,
    /// "measured on 2026-11-11", "assumed": shown beside the estimate.
    pub(crate) from: String,
}

/// The speeds an estimate is computed from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Speeds {
    /// Wall seconds of indexing per second of new footage.
    pub(crate) index: Basis,
    /// Wall seconds per captioned segment.
    pub(crate) caption: Basis,
    /// Captioned segments per second of footage (segments per second times
    /// the captionable share), for footage not indexed yet.
    pub(crate) density: Basis,
}

/// Indexing speed when nothing has been measured: twice realtime.
const ASSUMED_INDEX_RATIO: f32 = 0.5;
/// Captioning speed when nothing has been measured: 7 s a segment
/// (Qwen3-VL 8B on the editing PC, 2026-11-11: 338 segments in 40 min).
const ASSUMED_CAPTION_SECS: f32 = 7.0;
/// Captioned segments per second of footage when nothing has been
/// measured (2026-11-11: 417 segments, 338 captioned, from about 25 min).
const ASSUMED_DENSITY: f32 = 0.22;

/// Other nights' work folders beside this one (`<parent>/<night>/_ave`),
/// newest name first.
pub(crate) fn sibling_works(work: &Path) -> Vec<PathBuf> {
    let night = crate::recipe::night_dir(work);
    let Some(parent) = night.parent() else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = std::fs::read_dir(parent)
        .map(|e| {
            e.filter_map(Result::ok)
                .map(|e| e.path().join("_ave"))
                .filter(|w| w.is_dir() && *w != work)
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out.reverse();
    out
}

/// "this night" or the night's name, for a basis line.
fn night_label(work: &Path, this: &Path) -> String {
    if work == this {
        "this night".to_owned()
    } else {
        crate::recipe::Names::for_work(work).night
    }
}

/// The wall-clock ratio of an index run from its cache files' finish times:
/// `(finished at, clip seconds)` per clip. Takes the largest run (files
/// written less than ten minutes apart); the first file's own work started
/// before anything was written, so its seconds are left out.
pub(crate) fn ratio_from_mtimes(points: &[(f64, f64)]) -> Option<f32> {
    let mut points = points.to_vec();
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
    for p in points {
        match runs.last_mut() {
            Some(run) if run.last().is_some_and(|l| p.0 - l.0 < 600.0) => run.push(p),
            Some(_) | None => runs.push(vec![p]),
        }
    }
    let run = runs.into_iter().max_by_key(Vec::len)?;
    if run.len() < 3 {
        return None;
    }
    let span = run.last()?.0 - run.first()?.0;
    let source: f64 = run.iter().skip(1).map(|p| p.1).sum();
    (span > 0.0 && source > 0.0).then(|| ((span / source) as f32).clamp(0.005, 20.0))
}

/// The index ratio measured from `work`'s clip cache, if it holds a run.
fn cache_ratio(work: &Path) -> Option<f32> {
    /// The part of a cached clip this needs.
    #[derive(serde::Deserialize)]
    struct Cached {
        /// ffprobe result.
        probe: crate::media::Probe,
    }
    let points: Vec<(f64, f64)> = std::fs::read_dir(work.join("clips"))
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let t = e
                .metadata()
                .ok()?
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_secs_f64();
            let text = std::fs::read_to_string(e.path()).ok()?;
            let c: Cached = serde_json::from_str(&text).ok()?;
            Some((t, c.probe.duration))
        })
        .collect();
    ratio_from_mtimes(&points)
}

/// Seconds per segment from a caption log (`10/338 captioned, 86s`): the
/// newest such line's seconds over its count.
pub(crate) fn caption_log_rate(text: &str) -> Option<f32> {
    text.lines().rev().find_map(|line| {
        let (head, tail) = line.split_once(" captioned, ")?;
        let (n, _) = head.rsplit(' ').next()?.split_once('/')?;
        let n: f32 = n.parse().ok()?;
        let secs: f32 = tail.trim().strip_suffix('s')?.parse().ok()?;
        (n > 0.0 && secs > 0.0).then_some(secs / n)
    })
}

/// The speeds to estimate `work` with: measured on this night, else on the
/// newest sibling night that measured them, else assumed. Reads cache files
/// and logs: call off the UI thread.
pub(crate) fn speeds(work: &Path) -> Speeds {
    let mut nights = vec![work.to_path_buf()];
    nights.extend(sibling_works(work));
    let statuses: Vec<(PathBuf, NightStatus)> = nights
        .iter()
        .map(|w| (w.clone(), NightStatus::load(w)))
        .collect();
    let measured = |pick: &dyn Fn(&Throughput) -> Option<f32>| {
        statuses.iter().find_map(|(w, s)| {
            pick(&s.throughput).map(|value| Basis {
                value,
                from: format!("measured on {}", night_label(w, work)),
            })
        })
    };
    let index = measured(&|t| t.index_ratio)
        .or_else(|| {
            nights.iter().find_map(|w| {
                cache_ratio(w).map(|value| Basis {
                    value,
                    from: format!("from {}'s clip cache", night_label(w, work)),
                })
            })
        })
        .unwrap_or_else(|| Basis {
            value: ASSUMED_INDEX_RATIO,
            from: "assumed (2\u{d7} realtime)".to_owned(),
        });
    let caption = measured(&|t| t.caption_secs)
        .or_else(|| {
            nights.iter().find_map(|w| {
                let text = std::fs::read_to_string(w.join("caption.log")).ok()?;
                caption_log_rate(&text).map(|value| Basis {
                    value,
                    from: format!("from {}'s caption log", night_label(w, work)),
                })
            })
        })
        .unwrap_or_else(|| Basis {
            value: ASSUMED_CAPTION_SECS,
            from: "assumed".to_owned(),
        });
    let density =
        measured(&|t| Some(t.segments_per_sec? * t.captionable?)).unwrap_or_else(|| Basis {
            value: ASSUMED_DENSITY,
            from: "assumed".to_owned(),
        });
    Speeds {
        index,
        caption,
        density,
    }
}

/// One video in the footage folder.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FootageClip {
    /// Clip id (file stem).
    pub(crate) id: String,
    /// The file.
    pub(crate) path: PathBuf,
    /// Seconds, when known (from the cache or ffprobe).
    pub(crate) duration: Option<f64>,
    /// Whether the cache holds a current analysis of it.
    pub(crate) cached: bool,
}

/// Lists the footage folder with durations and cache state. Durations
/// come from the cache, else from ffprobe when `probe` is set (slow: call
/// off the UI thread).
pub(crate) fn scan_footage(work: &Path, footage: &Path, probe: bool) -> Result<Vec<FootageClip>> {
    /// The part of a cached clip this needs.
    #[derive(serde::Deserialize)]
    struct Cached {
        /// Size and mtime when analysed.
        stamp: (u64, u64),
        /// ffprobe result.
        probe: crate::media::Probe,
        /// Analysis version.
        #[serde(default)]
        version: u32,
    }
    let videos = crate::index::list_videos(footage)?;
    Ok(videos
        .into_iter()
        .map(|path| {
            let id = crate::index::clip_id(&path);
            let cache: Option<Cached> =
                std::fs::read_to_string(work.join("clips").join(format!("{id}.json")))
                    .ok()
                    .and_then(|t| serde_json::from_str(&t).ok());
            let stamp = crate::index::stamp(&path).ok();
            let cached = cache.as_ref().is_some_and(|c| {
                Some(c.stamp) == stamp && c.version >= crate::analyze::ANALYSIS_VERSION
            });
            let duration = cache.map(|c| c.probe.duration).or_else(|| {
                probe
                    .then(|| crate::media::probe(&path).ok().map(|p| p.duration))
                    .flatten()
            });
            FootageClip {
                id,
                path,
                duration,
                cached,
            }
        })
        .collect())
}

/// What a step is expected to take.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StepEstimate {
    /// What it will do: "2 of 81 clips to analyse · 3.4 min of footage".
    pub(crate) what: String,
    /// Expected wall seconds.
    pub(crate) secs: f32,
    /// Where the speed came from.
    pub(crate) basis: String,
}

/// "45 s", "3 min", "1 h 20 min".
pub(crate) fn duration_text(secs: f32) -> String {
    let secs = secs.max(0.0).round() as u64;
    match secs {
        0..90 => format!("{secs} s"),
        90..3600 => format!("{} min", (secs + 30) / 60),
        _ => format!("{} h {} min", secs / 3600, (secs % 3600 + 30) / 60),
    }
}

/// The index step: clips not in the cache, at the measured ratio.
pub(crate) fn estimate_index(clips: &[FootageClip], speeds: &Speeds) -> StepEstimate {
    let new: Vec<&FootageClip> = clips.iter().filter(|c| !c.cached).collect();
    let footage: f64 = new.iter().filter_map(|c| c.duration).sum();
    let unknown = new.iter().filter(|c| c.duration.is_none()).count();
    let mut what = if new.is_empty() {
        format!("{} clips, all analysed already", clips.len())
    } else {
        format!(
            "{} of {} clips to analyse \u{b7} {:.1} min of footage",
            new.len(),
            clips.len(),
            footage / 60.0
        )
    };
    if unknown > 0 {
        what.push_str(&format!(" ({unknown} without a known length)"));
    }
    StepEstimate {
        what,
        secs: footage as f32 * speeds.index.value,
        basis: speeds.index.from.clone(),
    }
}

/// The caption step: what the shot log says is left, or, before indexing,
/// what the footage's length suggests.
pub(crate) fn estimate_caption(
    index: Option<&Index>,
    clips: &[FootageClip],
    speeds: &Speeds,
) -> StepEstimate {
    let (n, what) = match index {
        Some(index) => {
            let n = index
                .segments
                .iter()
                .filter(|s| s.caption.is_none() && captionable(s))
                .count();
            (n, format!("{n} segments to caption"))
        }
        None => {
            let footage: f64 = clips.iter().filter_map(|c| c.duration).sum();
            let n = (footage as f32 * speeds.density.value).round() as usize;
            (n, format!("about {n} segments to caption"))
        }
    };
    StepEstimate {
        what,
        secs: n as f32 * speeds.caption.value,
        basis: speeds.caption.from.clone(),
    }
}

/// How one clip stands, for the processing list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowState {
    /// Not analysed yet.
    Pending,
    /// Analysed.
    Indexed,
    /// Analysed and every captionable segment captioned.
    Captioned,
    /// Its analysis failed.
    Failed,
}

/// One clip in the processing list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ClipRow {
    /// Clip id.
    pub(crate) id: String,
    /// The file.
    pub(crate) path: PathBuf,
    /// Seconds, when known.
    pub(crate) duration: Option<f64>,
    /// Where it stands.
    pub(crate) state: RowState,
    /// Why it failed.
    pub(crate) reason: Option<String>,
    /// Captioned segments, of how many the caption step takes.
    pub(crate) captions: (usize, usize),
}

/// The processing list's rows, from the footage scan, the shot log and the
/// status file: what a reopened night shows before anything runs.
pub(crate) fn clip_rows(
    clips: &[FootageClip],
    index: Option<&Index>,
    status: &NightStatus,
) -> Vec<ClipRow> {
    clips
        .iter()
        .map(|c| {
            let in_index = index.and_then(|i| i.clip(&c.id));
            let captions = index.map_or((0, 0), |i| {
                let segs: Vec<_> = i
                    .segments
                    .iter()
                    .filter(|s| s.clip == c.id && captionable(s))
                    .collect();
                (
                    segs.iter().filter(|s| s.caption.is_some()).count(),
                    segs.len(),
                )
            });
            let recorded = status.clips.get(&c.id);
            let failed = recorded.filter(|r| r.state == ClipState::IndexFailed);
            let state = if in_index.is_some() || c.cached {
                if captions.1 > 0 && captions.0 == captions.1 {
                    RowState::Captioned
                } else {
                    RowState::Indexed
                }
            } else if failed.is_some() {
                RowState::Failed
            } else {
                RowState::Pending
            };
            ClipRow {
                id: c.id.clone(),
                path: c.path.clone(),
                duration: c.duration.or_else(|| in_index.map(|ci| ci.probe.duration)),
                state,
                reason: failed.and_then(|r| r.reason.clone()),
                captions,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::Project;

    #[test]
    fn a_name_set_in_the_settings_beats_the_folder() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("ADUPU").join("_ave");
        let mut project = Project::open(&work)?;
        assert_eq!(project.name(), "ADUPU", "the folder, until named");
        project.settings.name = Some("Thanksgiving 2026".to_owned());
        project.save_settings()?;
        let again = Project::open(&work)?;
        assert_eq!(
            again.name(),
            "Thanksgiving 2026",
            "the name, saved with the project"
        );
        assert_eq!(again.folder_name(), "ADUPU", "the folder is still known");
        Ok(())
    }

    #[test]
    fn an_empty_folder_opens_with_nothing_run() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("night").join("_ave");
        let project = Project::open(&work)?;
        assert!(project.index.is_none(), "nothing has run yet");
        assert!(
            project.no_music(),
            "with no track the project is cut without music, nothing to map"
        );
        assert_eq!(project.name(), "night", "named after the folder above _ave");
        assert!(project.plans.is_empty(), "no plans yet");
        Ok(())
    }

    #[test]
    fn footage_folders_map_to_the_nights_work_folder() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let video = dir
            .path()
            .join("2026-11-11")
            .join("originals")
            .join("video");
        std::fs::create_dir_all(&video)?;
        assert_eq!(
            super::work_dir_for(&video),
            dir.path().join("2026-11-11").join("_ave"),
            "originals/video"
        );
        let work = dir.path().join("2026-11-11").join("_ave");
        assert_eq!(
            super::work_dir_for(&work),
            work,
            "an _ave folder is its own work folder"
        );
        let library = dir.path().join("Users").join("me").join("Videos");
        std::fs::create_dir_all(&library)?;
        assert_eq!(
            super::work_dir_for(&library),
            library.join("_ave"),
            "a folder called Videos keeps its project inside it, not in the folder above"
        );
        let plain = dir.path().join("Wedding clips");
        std::fs::create_dir_all(&plain)?;
        assert_eq!(
            super::work_dir_for(&plain),
            plain.join("_ave"),
            "any folder of clips"
        );
        Ok(())
    }

    #[test]
    fn an_empty_footage_folder_is_refused_and_points_at_the_clips() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let card = dir.path().join("card");
        let clips = card.join("DCIM").join("100CANON");
        std::fs::create_dir_all(&clips)?;
        let Some(why) = super::footage_problem(&card) else {
            anyhow::bail!("an empty folder was accepted");
        };
        assert!(why.contains("no video clips"), "{why}");
        assert!(!why.contains("100CANON"), "no clips anywhere yet: {why}");
        std::fs::write(clips.join("MVI_0001.MP4"), b"")?;
        let Some(why) = super::footage_problem(&card) else {
            anyhow::bail!("a folder whose clips are two levels down was accepted");
        };
        assert!(why.contains("100CANON") && why.contains("has 1"), "{why}");
        assert_eq!(
            super::footage_problem(&clips),
            None,
            "the folder holding the clips is fine"
        );
        Ok(())
    }

    #[test]
    fn settings_round_trip() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut project = Project::open(dir.path())?;
        project.settings.track = Some("song.wav".into());
        project
            .settings
            .end_cards
            .insert("9x16".into(), "card.png".into());
        project.save_settings()?;
        project.reload_settings();
        assert_eq!(
            project.settings.track.as_deref(),
            Some(std::path::Path::new("song.wav")),
            "track kept"
        );
        assert_eq!(project.settings.end_cards.len(), 1, "end card kept");
        Ok(())
    }

    /// Speeds with round numbers.
    fn speeds() -> super::Speeds {
        let b = |value: f32, from: &str| super::Basis {
            value,
            from: from.to_owned(),
        };
        super::Speeds {
            index: b(0.5, "measured on this night"),
            caption: b(7.0, "assumed"),
            density: b(0.25, "assumed"),
        }
    }

    /// A footage clip.
    fn clip(id: &str, duration: Option<f64>, cached: bool) -> super::FootageClip {
        super::FootageClip {
            id: id.to_owned(),
            path: format!("{id}.mov").into(),
            duration,
            cached,
        }
    }

    #[test]
    fn index_estimates_count_only_new_footage() {
        let clips = [
            clip("A", Some(60.0), true),
            clip("B", Some(120.0), false),
            clip("C", Some(60.0), false),
            clip("D", None, false),
        ];
        let e = super::estimate_index(&clips, &speeds());
        assert!(
            (e.secs - 90.0).abs() < 1e-3,
            "180 s of new footage at 0.5: {e:?}"
        );
        assert!(
            e.what.starts_with("3 of 4 clips") && e.what.contains("1 without a known length"),
            "{e:?}"
        );
        assert_eq!(
            e.basis, "measured on this night",
            "says where the speed is from"
        );
        let done = super::estimate_index(&clips[..1], &speeds());
        assert!(done.secs.abs() < 1e-6, "all cached: nothing to wait for");
    }

    #[test]
    fn caption_estimates_use_the_shot_log_when_there_is_one() {
        let clips = [clip("A", Some(400.0), false)];
        let before = super::estimate_caption(None, &clips, &speeds());
        assert!(
            (before.secs - 700.0).abs() < 1e-3,
            "400 s x 0.25 = 100 segments x 7 s: {before:?}"
        );
        let seg = |q: f32, captioned: bool| crate::index::Scored {
            q,
            seg: crate::analyze::segments::Segment {
                start: 0.0,
                end: 3.0,
                ..Default::default()
            },
            caption: captioned.then(crate::caption::Caption::default),
            ..Default::default()
        };
        let index = crate::index::Index {
            segments: vec![seg(0.5, false), seg(0.5, true), seg(0.05, false)],
            ..Default::default()
        };
        let after = super::estimate_caption(Some(&index), &clips, &speeds());
        assert_eq!(
            after.what, "1 segments to caption",
            "only uncaptioned, above min q"
        );
        assert!((after.secs - 7.0).abs() < 1e-3, "{after:?}");
    }

    #[test]
    fn speeds_are_read_from_cache_times_and_caption_logs() {
        let points = [
            (1000.0, 30.0),
            (1003.0, 30.0),
            (1006.0, 30.0),
            (99_000.0, 5.0),
        ];
        let ratio = super::ratio_from_mtimes(&points);
        assert!(
            ratio.is_some_and(|r| (r - 0.1).abs() < 1e-4),
            "6 s for the 60 s after the first, the far-off file is another run: {ratio:?}"
        );
        assert_eq!(
            super::ratio_from_mtimes(&points[..2]),
            None,
            "too few to measure"
        );
        let log = "[..] captioning 338 segments\n[..] 10/338 captioned, 86s\n[..] 20/338 captioned, 140s\ndone";
        assert_eq!(
            super::caption_log_rate(log),
            Some(7.0),
            "newest line: 140 s / 20"
        );
        assert_eq!(super::caption_log_rate("nothing here"), None);
    }

    #[test]
    fn speeds_fall_back_to_named_assumptions() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-18").join("_ave");
        std::fs::create_dir_all(&work)?;
        let s = super::speeds(&work);
        assert_eq!(s.index.from, "assumed (2\u{d7} realtime)", "{s:?}");
        assert!((s.caption.value - 7.0).abs() < 1e-6, "{s:?}");
        // A sibling night that measured captions lends its speed.
        let last = dir.path().join("2026-11-11").join("_ave");
        let mut status = super::NightStatus::default();
        status.throughput.caption_secs = Some(6.0);
        status.save(&last)?;
        let s = super::speeds(&work);
        assert_eq!(
            (s.caption.value, s.caption.from.as_str()),
            (6.0, "measured on 2026-11-11"),
            "{s:?}"
        );
        Ok(())
    }

    #[test]
    fn clip_rows_show_what_a_reopened_night_has_done() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut status = super::NightStatus::default();
        use crate::commands::ItemState;
        status.observe("index", "B", ItemState::Failed, None, Some("unreadable"));
        status.observe("index", "C", ItemState::Started, None, None);
        status.settle();
        status.save(dir.path())?;
        let status = super::NightStatus::load(dir.path());
        let clips = [
            clip("A", Some(5.0), true),
            clip("B", Some(5.0), false),
            clip("C", Some(5.0), false),
        ];
        let rows = super::clip_rows(&clips, None, &status);
        let states: Vec<super::RowState> = rows.iter().map(|r| r.state).collect();
        assert_eq!(
            states,
            [
                super::RowState::Indexed,
                super::RowState::Failed,
                super::RowState::Pending
            ],
            "cached, failed (from status.json), interrupted"
        );
        assert_eq!(rows[1].reason.as_deref(), Some("unreadable"), "reason kept");
        Ok(())
    }

    #[test]
    fn durations_read_like_people_say_them() {
        assert_eq!(super::duration_text(42.0), "42 s");
        assert_eq!(super::duration_text(600.0), "10 min");
        assert_eq!(super::duration_text(4800.0), "1 h 20 min");
    }
}

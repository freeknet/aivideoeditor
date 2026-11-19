// ============================================================================
// Module:       project
// Description:  An open night: its work folder, its settings file, and the
//               engine artefacts loaded from it (index, music, people, plans).
//
// Dependencies: crate::{index, beats, people, plan}, serde, toml
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

/// The work folder for a folder the user picked.
///
/// - an `_ave` folder, or one holding `index.json` / `ave.toml`, is the work folder;
/// - a folder with an `_ave` inside opens that;
/// - a footage folder (`<night>/originals/video`, or any folder of clips)
///   gets `<night>/_ave`, where `<night>` skips an `originals/video` tail.
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
    let mut night = picked;
    let is = |p: &Path, name: &str| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name));
    if is(night, "video") || is(night, "videos") {
        night = night.parent().unwrap_or(night);
    }
    if is(night, "originals") {
        night = night.parent().unwrap_or(night);
    }
    night.join("_ave")
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

    /// The night's name: the folder above `_ave`, or the work folder's own.
    pub(crate) fn name(&self) -> String {
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
        if self.settings.footage.is_none()
            && let Some(index) = &self.index
        {
            self.settings.footage = Some(PathBuf::from(&index.source));
        }
    }

    /// Re-reads `music.json`.
    pub(crate) fn reload_music(&mut self) {
        self.music = if self.work.join("music.json").exists() {
            MusicMap::load(&self.work)
                .map_err(|e| self.warnings.push(format!("music: {e:#}")))
                .ok()
        } else {
            None
        };
        // A night mapped from the command line has its track only in the map.
        if self.settings.track.is_none()
            && let Some(music) = &self.music
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

#[cfg(test)]
mod tests {
    use super::Project;

    #[test]
    fn an_empty_folder_opens_with_nothing_run() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("night").join("_ave");
        let project = Project::open(&work)?;
        assert!(
            project.index.is_none() && project.music.is_none(),
            "nothing has run yet"
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
}

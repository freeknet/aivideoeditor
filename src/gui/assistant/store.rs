// ============================================================================
// Module:       gui::assistant::store
// Description:  What the assistant keeps on disk: the conversation per plan
//               (`<work>/chat/<plan>.jsonl`), plan versions written by
//               applying a proposal (`plans/.history/<plan>/v<N>.toml`), and
//               the panel's preferences (model, key, local server).
//
// Dependencies: serde_json, toml, dirs; super::model; crate::plan
// ============================================================================

//! Files.
//!
//! - **Chat log**: one JSON [`LogLine`] per line, appended as the turn runs,
//!   so a crash loses at most the line being written. Unreadable lines are
//!   skipped, not fatal.
//! - **Plan versions**: applying a proposal writes the plan file and a copy
//!   at `plans/.history/<plan>/v<N>.toml` whose first line is a comment
//!   `# v<N> · <time> · <what>`. The first time, the plan as it was is kept
//!   as `v1`, so the very first agent edit can be undone too. Reverting is
//!   writing an older version as a new one: history only grows.
//! - **Preferences**: `<config dir>/aivideoeditor/assistant.toml`, beside
//!   the app config, every field optional.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::model::{LogLine, now, stamp};
use crate::plan::Plan;

/// The chat log of plan `plan`.
pub(crate) fn chat_path(work: &Path, plan: &str) -> PathBuf {
    work.join("chat").join(format!("{plan}.jsonl"))
}

/// Appends one line to a chat log.
pub(crate) fn append(path: &Path, line: &LogLine) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    writeln!(file, "{}", serde_json::to_string(line)?)?;
    Ok(())
}

/// Reads a chat log: its lines, and how many could not be read.
pub(crate) fn load(path: &Path) -> (Vec<LogLine>, usize) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (Vec::new(), 0);
    };
    let mut bad = 0;
    let lines = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            serde_json::from_str(l)
                .map_err(|e| {
                    log::warn!("{}: skipping a chat line: {e}", path.display());
                    bad += 1;
                })
                .ok()
        })
        .collect();
    (lines, bad)
}

/// One saved plan version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Version {
    /// Its number.
    pub(crate) n: usize,
    /// Its file.
    pub(crate) path: PathBuf,
    /// Its header comment, without the `#`.
    pub(crate) header: String,
}

/// The versions of plan `plan`, oldest first.
pub(crate) fn versions(work: &Path, plan: &str) -> Vec<Version> {
    let dir = work.join("plans").join(".history").join(plan);
    let mut out: Vec<Version> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter_map(|path| {
                    let n = path
                        .file_stem()?
                        .to_str()?
                        .strip_prefix('v')?
                        .parse()
                        .ok()?;
                    let header = std::fs::read_to_string(&path)
                        .ok()?
                        .lines()
                        .next()
                        .and_then(|l| l.strip_prefix('#'))
                        .map(|l| l.trim().to_owned())
                        .unwrap_or_default();
                    Some(Version { n, path, header })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|v| v.n);
    out
}

/// Writes `plan` as the new current version of plan file `plans/<stem>.toml`
/// and as `plans/.history/<stem>/v<N>.toml`; returns `N`.
pub(crate) fn save_version(work: &Path, stem: &str, plan: &Plan, what: &str) -> Result<usize> {
    let file = work.join("plans").join(format!("{stem}.toml"));
    let dir = work.join("plans").join(".history").join(stem);
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut last = versions(work, stem).last().map_or(0, |v| v.n);
    if last == 0
        && let Ok(text) = std::fs::read_to_string(&file)
    {
        let header = format!("# v1 · {} · as it was before the assistant\n", stamp(now()));
        std::fs::write(dir.join("v1.toml"), header + &text)?;
        last = 1;
    }
    let n = last + 1;
    let what: String = what
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(140)
        .collect();
    let body = toml::to_string_pretty(plan)?;
    std::fs::write(
        dir.join(format!("v{n}.toml")),
        format!("# v{n} · {} · {what}\n{body}", stamp(now())),
    )?;
    // Through a temp file and a rename, so a crash cannot leave half a plan.
    let tmp = file.with_extension("toml.tmp");
    std::fs::write(&tmp, &body)?;
    std::fs::rename(&tmp, &file).with_context(|| format!("writing {}", file.display()))?;
    Ok(n)
}

/// The panel's preferences.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Prefs {
    /// Model id, or `local`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) model: Option<String>,
    /// Anthropic API key (used when `ANTHROPIC_API_KEY` is not set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) api_key: Option<String>,
    /// Local llama.cpp server URL (else `AVE_LLM_URL`, else the default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) local_url: Option<String>,
}

/// Where the preferences live.
pub(crate) fn prefs_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(crate::APP_ID).join("assistant.toml"))
}

impl Prefs {
    /// Loads the preferences, forgiving a missing or broken file.
    pub(crate) fn load() -> Self {
        prefs_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Saves the preferences.
    pub(crate) fn save(&self) -> Result<()> {
        let path = prefs_path().context("no config folder on this system")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// The API key: the environment first, then the saved one.
    pub(crate) fn key(&self) -> Option<String> {
        std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .or_else(|| self.api_key.clone().filter(|k| !k.trim().is_empty()))
    }

    /// The local server URL.
    pub(crate) fn local(&self) -> String {
        self.local_url
            .clone()
            .filter(|u| !u.trim().is_empty())
            .or_else(|| std::env::var("AVE_LLM_URL").ok())
            .unwrap_or_else(|| "http://127.0.0.1:8089".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use crate::gui::assistant::model::{LogLine, Status};

    #[test]
    fn chat_logs_append_and_skip_bad_lines() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = super::chat_path(dir.path(), "reel-30");
        let line = LogLine::Status {
            id: "p".into(),
            status: Status::Discarded,
        };
        super::append(&path, &line)?;
        std::fs::write(&path, std::fs::read_to_string(&path)? + "not json\n")?;
        super::append(&path, &line)?;
        let (lines, bad) = super::load(&path);
        assert_eq!(lines.len(), 2, "two good lines");
        assert_eq!(bad, 1, "one bad line");
        assert_eq!(
            super::load(&dir.path().join("none.jsonl")).0.len(),
            0,
            "missing log is empty"
        );
        Ok(())
    }

    #[test]
    fn versions_keep_the_original_and_grow() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut plan = crate::gui::assistant::ops::tests::plan(2);
        let file = dir.path().join("plans").join("t.toml");
        plan.save(&file)?;
        plan.shots[0].note = "changed".into();
        let n = super::save_version(dir.path(), "t", &plan, "swap 1\nmore")?;
        assert_eq!(n, 2, "the original became v1");
        let vs = super::versions(dir.path(), "t");
        assert_eq!(vs.len(), 2, "two versions");
        assert!(vs[1].header.ends_with("swap 1"), "header: {}", vs[1].header);
        let current = crate::plan::Plan::load(&file)?;
        assert_eq!(current.shots[0].note, "changed", "plan file updated");
        let v2 = crate::plan::Plan::load(&vs[1].path)?;
        assert_eq!(v2, current, "history copy parses despite the comment");
        assert_eq!(
            super::save_version(dir.path(), "t", &plan, "again")?,
            3,
            "numbers grow"
        );
        Ok(())
    }
}

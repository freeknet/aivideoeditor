// ============================================================================
// Module:       gui::assistant::store
// Description:  What the assistant keeps on disk: the night's chats (an index
//               at `<work>/chat/chats.json`, one log per chat under
//               `<work>/chat/chats/`, the old per-plan logs migrated), plan
//               versions written by applying a proposal
//               (`plans/.history/<plan>/v<N>.toml`), and the panel's
//               preferences (model, key, local server).
//
// Dependencies: serde_json, toml, dirs; super::model; crate::plan
// ============================================================================

//! Files.
//!
//! - **Chat log**: one JSON [`LogLine`] per line, appended as the turn runs,
//!   so a crash loses at most the line being written. Unreadable lines are
//!   skipped, not fatal.
//! - **Chats**: a night has any number of conversations. `chat/chats.json`
//!   lists them ([`ChatMeta`]: id, title, the plan as an attribute, times,
//!   archived); each one's log is `chat/chats/<id>.jsonl` and its Claude
//!   Code session id `chat/chats/<id>.session`. The index is written
//!   through a temp file and a rename.
//! - **Migration**: before chats, a conversation belonged to a plan and
//!   lived at `chat/<plan>.jsonl` (an earlier one, set aside by "New chat",
//!   at `chat/<plan>.<unix time>.jsonl`; the session at
//!   `chat/<plan>.session`). Loading the index adopts every such file it
//!   does not list as a chat titled after its plan, without moving it: the
//!   old file is read until the chat is next written, which copies it to
//!   the new place first. Old files are never rewritten.
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

/// The old (pre-chats) log of plan `plan`: `chat/<plan>.jsonl`. Only
/// tests write one now, to build a night from before chats.
#[cfg(test)]
pub(crate) fn chat_path(work: &Path, plan: &str) -> PathBuf {
    work.join("chat").join(format!("{plan}.jsonl"))
}

/// One conversation of the night, as the index lists it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ChatMeta {
    /// Stable id: the log's and the session's file stem.
    pub(crate) id: String,
    /// What the list shows.
    pub(crate) title: String,
    /// The plan (file stem) the chat is about.
    pub(crate) plan: String,
    /// When it started, Unix seconds.
    #[serde(default)]
    pub(crate) created: u64,
    /// When it last changed, Unix seconds.
    #[serde(default)]
    pub(crate) updated: u64,
    /// Hidden from the recent list.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) archived: bool,
    /// The person named it: never auto-titled again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) renamed: bool,
    /// The old per-plan log it was migrated from, a file name in `chat/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) legacy: Option<String>,
}

/// The title of a chat nobody has written in yet.
pub(crate) const UNTITLED: &str = "New chat";

impl ChatMeta {
    /// A fresh, untitled chat about `plan`, with an id unused in `index`.
    pub(crate) fn fresh(index: &Index, plan: &str) -> Self {
        let t = now();
        Self {
            id: new_id(index),
            title: UNTITLED.to_owned(),
            plan: plan.to_owned(),
            created: t,
            updated: t,
            archived: false,
            renamed: false,
            legacy: None,
        }
    }
}

/// The night's chats.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Index {
    /// Every chat, in no particular order.
    #[serde(default)]
    pub(crate) chats: Vec<ChatMeta>,
}

impl Index {
    /// The chat with this id.
    pub(crate) fn get(&self, id: &str) -> Option<&ChatMeta> {
        self.chats.iter().find(|c| c.id == id)
    }

    /// Adds `meta`, or replaces the chat with its id.
    pub(crate) fn upsert(&mut self, meta: &ChatMeta) {
        match self.chats.iter_mut().find(|c| c.id == meta.id) {
            Some(c) => c.clone_from(meta),
            None => self.chats.push(meta.clone()),
        }
    }

    /// The chats, newest change first: the archived ones if `archived`,
    /// else the others.
    pub(crate) fn recent(&self, archived: bool) -> Vec<&ChatMeta> {
        let mut out: Vec<&ChatMeta> = self
            .chats
            .iter()
            .filter(|c| c.archived == archived)
            .collect();
        out.sort_by(|a, b| b.updated.cmp(&a.updated).then_with(|| a.id.cmp(&b.id)));
        out
    }
}

/// Where the index lives.
pub(crate) fn index_path(work: &Path) -> PathBuf {
    work.join("chat").join("chats.json")
}

/// The folder of the chats' logs and sessions.
fn chats_dir(work: &Path) -> PathBuf {
    work.join("chat").join("chats")
}

/// The log of chat `id` (where it is written).
pub(crate) fn log_path(work: &Path, id: &str) -> PathBuf {
    chats_dir(work).join(format!("{id}.jsonl"))
}

/// The Claude Code session id of chat `id`.
pub(crate) fn session_path(work: &Path, id: &str) -> PathBuf {
    chats_dir(work).join(format!("{id}.session"))
}

/// The file chat `meta` is read from: its own log, else (not written
/// since the migration) the old per-plan one.
pub(crate) fn log_file(work: &Path, meta: &ChatMeta) -> PathBuf {
    let own = log_path(work, &meta.id);
    match &meta.legacy {
        Some(old) if !own.exists() => work.join("chat").join(old),
        Some(_) | None => own,
    }
}

/// Reads chat `meta`'s log: its lines, and how many could not be read.
pub(crate) fn load_chat(work: &Path, meta: &ChatMeta) -> (Vec<LogLine>, usize) {
    load(&log_file(work, meta))
}

/// Appends one line to chat `meta`'s log. A migrated chat's old log is
/// copied to the new place first, and stays where it was.
pub(crate) fn append_chat(work: &Path, meta: &ChatMeta, line: &LogLine) -> Result<()> {
    let own = log_path(work, &meta.id);
    if !own.exists()
        && let Some(old) = &meta.legacy
    {
        let old = work.join("chat").join(old);
        if old.exists() {
            if let Some(dir) = own.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::copy(&old, &own)
                .with_context(|| format!("copying {} to {}", old.display(), own.display()))?;
        }
    }
    append(&own, line)
}

/// Reads the index, adopting any old per-plan log it does not list (see
/// the module docs). A missing or broken index is an empty one.
pub(crate) fn load_index(work: &Path) -> Index {
    let mut index: Index = std::fs::read_to_string(index_path(work))
        .ok()
        .and_then(|t| {
            serde_json::from_str(&t)
                .map_err(|e| log::warn!("{}: unreadable chat index: {e}", work.display()))
                .ok()
        })
        .unwrap_or_default();
    if migrate(work, &mut index)
        && let Err(e) = save_index(work, &index)
    {
        log::warn!("could not save the chat index: {e:#}");
    }
    index
}

/// Adopts the old per-plan logs in `chat/`; returns whether any was new.
fn migrate(work: &Path, index: &mut Index) -> bool {
    let dir = work.join("chat");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| n.ends_with(".jsonl"))
        .collect();
    names.sort();
    let mut found = false;
    for name in names {
        if index
            .chats
            .iter()
            .any(|c| c.legacy.as_deref() == Some(name.as_str()))
        {
            continue;
        }
        let stem = name.trim_end_matches(".jsonl");
        let (plan, earlier) = match stem.rsplit_once('.') {
            Some((plan, ts)) if !ts.is_empty() && ts.bytes().all(|b| b.is_ascii_digit()) => {
                (plan, ts.parse::<u64>().ok())
            }
            Some(_) | None => (stem, None),
        };
        let path = dir.join(&name);
        let (log, _) = load(&path);
        let stamps: Vec<u64> = log
            .iter()
            .filter_map(|l| match l {
                LogLine::Msg { msg } if msg.ts > 0 => Some(msg.ts),
                LogLine::Msg { .. }
                | LogLine::Proposal { .. }
                | LogLine::Status { .. }
                | LogLine::Turn { .. }
                | LogLine::Notice { .. }
                | LogLine::Image { .. } => None,
            })
            .collect();
        let touched = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        let last = stamps.iter().copied().max().unwrap_or(touched);
        let safe: String = stem
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let mut id = format!("p-{safe}");
        let mut n = 2;
        while index.get(&id).is_some() {
            id = format!("p-{safe}-{n}");
            n += 1;
        }
        if earlier.is_none() {
            let old = dir.join(format!("{plan}.session"));
            let new = session_path(work, &id);
            if old.exists() && !new.exists() {
                let copied = std::fs::create_dir_all(chats_dir(work))
                    .and_then(|()| std::fs::copy(&old, &new).map(|_| ()));
                if let Err(e) = copied {
                    log::warn!("could not carry over {}: {e}", old.display());
                }
            }
        }
        index.chats.push(ChatMeta {
            id,
            title: if earlier.is_some() {
                format!("{plan} (earlier)")
            } else {
                plan.to_owned()
            },
            plan: plan.to_owned(),
            created: stamps.iter().copied().min().unwrap_or(last),
            updated: earlier.unwrap_or(last),
            archived: false,
            renamed: false,
            legacy: Some(name.clone()),
        });
        found = true;
    }
    found
}

/// Writes the index, through a temp file and a rename.
pub(crate) fn save_index(work: &Path, index: &Index) -> Result<()> {
    let path = index_path(work);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(index)?)?;
    std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Deletes chat `meta`'s files: its log, its session and the old log it
/// was migrated from. The caller drops it from the index.
pub(crate) fn delete_chat(work: &Path, meta: &ChatMeta) -> Result<()> {
    let mut files = vec![log_path(work, &meta.id), session_path(work, &meta.id)];
    if let Some(old) = &meta.legacy {
        files.push(work.join("chat").join(old));
    }
    for file in files {
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("deleting {}", file.display())),
        }
    }
    Ok(())
}

/// An id no chat in `index` has: `c` and the time in hex.
fn new_id(index: &Index) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let base = format!("c{nanos:x}");
    let mut id = base.clone();
    let mut n = 2;
    while index.get(&id).is_some() {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

/// A chat's title from its first message: the first line, spaces
/// collapsed, cut at a word near 40 characters.
pub(crate) fn auto_title(text: &str) -> String {
    const MAX: usize = 40;
    let line = text.trim().lines().next().unwrap_or_default();
    let flat = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return UNTITLED.to_owned();
    }
    if flat.chars().count() <= MAX {
        return flat;
    }
    let cut: String = flat.chars().take(MAX).collect();
    let short = match cut.rfind(' ') {
        Some(at) if at >= MAX / 2 => cut.get(..at).unwrap_or(&cut).to_owned(),
        Some(_) | None => cut.clone(),
    };
    format!("{}…", short.trim_end())
}

/// How long ago `then` was, at `now`: `just now`, `5m ago`, `3h ago`,
/// `2d ago`, else the date.
pub(crate) fn ago(now: u64, then: u64) -> String {
    let d = now.saturating_sub(then);
    match d {
        0..60 => "just now".to_owned(),
        60..3_600 => format!("{}m ago", d / 60),
        3_600..86_400 => format!("{}h ago", d / 3_600),
        86_400..604_800 => format!("{}d ago", d / 86_400),
        _ => stamp(then).chars().take(10).collect(),
    }
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

    /// A user message saying `text`.
    fn said(text: &str) -> LogLine {
        use crate::gui::assistant::model::{Block, Msg, Role};
        LogLine::Msg {
            msg: Msg::new(
                Role::User,
                vec![Block::Text {
                    text: text.to_owned(),
                }],
            ),
        }
    }

    #[test]
    fn an_old_per_plan_log_becomes_a_chat_titled_after_its_plan() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path();
        let old = super::chat_path(work, "reel-30");
        super::append(&old, &said("tighten the drop"))?;
        let earlier = work.join("chat").join("reel-30.1700000000.jsonl");
        super::append(&earlier, &said("first go"))?;
        std::fs::write(work.join("chat").join("reel-30.session"), "sess-1\n")?;
        std::fs::write(work.join("chat").join("claude-mcp.json"), "{}")?;

        let index = super::load_index(work);
        assert_eq!(index.chats.len(), 2, "both old logs adopted: {index:?}");
        let chat = index
            .chats
            .iter()
            .find(|c| c.legacy.as_deref() == Some("reel-30.jsonl"))
            .ok_or_else(|| anyhow::anyhow!("current log not adopted"))?;
        assert_eq!(chat.title, "reel-30", "titled after its plan");
        assert_eq!(chat.plan, "reel-30", "the plan is an attribute");
        assert!(!chat.archived, "an adopted chat is live");
        let (lines, _) = super::load_chat(work, chat);
        assert_eq!(lines.len(), 1, "the old file is read in place");
        assert_eq!(
            std::fs::read_to_string(super::session_path(work, &chat.id))?.trim(),
            "sess-1",
            "the chat keeps its Claude Code session"
        );
        let set_aside = index
            .recent(false)
            .into_iter()
            .find(|c| c.legacy.as_deref() == Some("reel-30.1700000000.jsonl"))
            .ok_or_else(|| anyhow::anyhow!("earlier log not adopted"))?;
        assert_eq!(set_aside.updated, 1_700_000_000, "dated by its file name");
        assert!(super::index_path(work).exists(), "the index was written");

        // Loading again adopts nothing twice.
        assert_eq!(super::load_index(work), index, "migration is idempotent");

        // Writing moves the chat to its own log; the old file stays as it was.
        super::append_chat(work, chat, &said("and the end"))?;
        assert_eq!(
            super::load_chat(work, chat).0.len(),
            2,
            "old lines carried over, new line appended"
        );
        assert_eq!(super::load(&old).0.len(), 1, "the old log is not rewritten");
        assert_eq!(
            super::log_file(work, chat),
            super::log_path(work, &chat.id),
            "read from its own log now"
        );
        Ok(())
    }

    #[test]
    fn chats_are_created_renamed_archived_and_deleted() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path();
        let mut index = super::Index::default();
        let mut a = super::ChatMeta::fresh(&index, "reel-a");
        assert_eq!(a.title, super::UNTITLED, "a new chat is untitled");
        index.upsert(&a);
        let mut b = super::ChatMeta::fresh(&index, "reel-b");
        assert_ne!(a.id, b.id, "ids are unique");
        b.updated = a.updated + 10;
        index.upsert(&b);
        assert_eq!(
            index.recent(false).first().map(|c| c.id.clone()),
            Some(b.id.clone()),
            "newest first"
        );
        a.title = "Opening".into();
        a.renamed = true;
        index.upsert(&a);
        a.archived = true;
        index.upsert(&a);
        assert_eq!(index.chats.len(), 2, "upsert replaces by id");
        assert_eq!(
            index.recent(false).len(),
            1,
            "archived chats leave the list"
        );
        assert_eq!(
            index.recent(true).first().map(|c| c.title.as_str()),
            Some("Opening"),
            "and are listed as archived, renamed"
        );
        super::append_chat(work, &b, &said("hello"))?;
        super::save_index(work, &index)?;
        assert_eq!(super::load_index(work), index, "the index round-trips");
        super::delete_chat(work, &b)?;
        assert!(
            !super::log_path(work, &b.id).exists(),
            "deleting removes the log"
        );
        assert_eq!(
            super::auto_title("  Put the   best fire shot on the drop and trim the end please"),
            "Put the best fire shot on the drop and…",
            "auto title: first words, cut at a word"
        );
        assert_eq!(super::auto_title("short one"), "short one", "short kept");
        assert_eq!(super::ago(1_000, 990), "just now", "seconds");
        assert_eq!(super::ago(10_000, 10_000 - 7_300), "2h ago", "hours");
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

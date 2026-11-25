// ============================================================================
// Module:       gui::assistant::claude_code
// Description:  The Claude Code runtime: one headless `claude -p` process per
//               conversation, driven over stream-json, with `ave mcp` as its
//               only tool server; its event stream mapped onto the panel's
//               events.
//
// Dependencies: serde_json, crossbeam-channel; super::{agent, model, tools,
//               provider}; crate::{mcp, plan}
// ============================================================================

//! Claude Code as the panel's runtime.
//!
//! This is how Selects runs its agent, mapped onto ours (see
//! `docs/research/04-selects-agent.md` §9.1): the `claude` CLI runs headless
//! (`-p --input-format stream-json --output-format stream-json`), stays up
//! for the whole conversation, and takes each user turn as one stream-json
//! line on stdin. Its tools are **only** `ave mcp --work <night>` (started
//! with the app's own executable) plus Read/Glob/Grep for the shot log and
//! the skill files; Bash, Edit, Write and the web tools are off, nothing
//! prompts (`--permission-mode dontAsk`), and the person's own settings,
//! CLAUDE.md, skills and MCP servers stay out (`--setting-sources ""`,
//! `--strict-mcp-config`, `--disable-slash-commands`). The session id is
//! saved beside the chat log so a restart resumes the same conversation.
//!
//! [`Stream`] turns stdout lines into [`Event`]s and is a pure state
//! machine, tested on recorded lines. [`Session`] owns the process: a reader
//! thread feeds the stream and forwards events; Stop kills the process (the
//! next turn resumes the session).

use std::collections::HashMap;
use std::io::{BufRead as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};

use super::agent::{Event, Guard};
use super::model::{Block, Msg, Proposal, Role, now};
use super::provider::{Delta, Usage};
use super::tools::{self, EDIT, MCP_PREFIX};
use crate::plan::Plan;

/// What the panel shows when the CLI is not signed in.
pub(crate) const LOGIN_HINT: &str = "Claude Code is not signed in (its sign-in can expire). \
Press Sign in to log in with your browser, then send again. Until then the panel uses the \
Claude API key or the local model.";

/// Whether `claude auth status --json` output says the CLI is signed in;
/// `None` when it is not that shape (an older CLI without `auth`).
pub(crate) fn signed_in(status: &str) -> Option<bool> {
    serde_json::from_str::<Value>(status.trim())
        .ok()?
        .get("loggedIn")?
        .as_bool()
}

/// Asks the CLI whether it is signed in (blocking: run it off the UI thread).
fn auth_status(exe: &Path) -> Option<bool> {
    let mut cmd = Command::new(exe);
    cmd.args(["auth", "status", "--json"])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    crate::media::quiet(&mut cmd);
    // Signed out may exit non-zero and still print the JSON.
    let out = cmd.output().ok()?;
    signed_in(&String::from_utf8_lossy(&out.stdout))
}

/// A sign-in check running in the background, or a browser sign-in and the
/// check after it.
///
/// The sign-in is Claude Code's own (`claude auth login`): the CLI opens the
/// browser, takes the person through claude.ai, and keeps the credentials in
/// its own store. The app never sees a token; it only asks the CLI afterwards
/// whether it is signed in. The CLI runs in a console window of its own so
/// that, if the browser cannot hand the code back, the person can paste it.
#[derive(Debug)]
pub(crate) struct Auth {
    /// The answer: signed in or not (`None`: the CLI did not say).
    rx: crossbeam_channel::Receiver<Option<bool>>,
    /// Whether this is a sign-in (the person is at the browser), not a check.
    pub(crate) signing_in: bool,
}

impl Auth {
    /// Checks whether `exe` is signed in; `on_done` is called with the answer
    /// ready (to repaint).
    pub(crate) fn check(exe: &Path, on_done: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let exe = exe.to_path_buf();
        let spawned = std::thread::Builder::new()
            .name("claude-auth-status".to_owned())
            .spawn(move || {
                let _ = tx.send(auth_status(&exe));
                on_done();
            });
        if let Err(e) = spawned {
            log::warn!("could not check the Claude Code sign-in: {e}");
        }
        Self {
            rx,
            signing_in: false,
        }
    }

    /// Starts `claude auth login`, which opens the browser; once it exits the
    /// sign-in is checked and `on_done` is called.
    pub(crate) fn login(exe: &Path, on_done: impl Fn() + Send + 'static) -> Result<Self, String> {
        let mut cmd = Command::new(exe);
        cmd.args(["auth", "login"]);
        own_console(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "could not start the Claude sign-in ({}): {e}",
                exe.display()
            )
        })?;
        let (tx, rx) = crossbeam_channel::bounded(1);
        let exe = exe.to_path_buf();
        std::thread::Builder::new()
            .name("claude-auth-login".to_owned())
            .spawn(move || {
                let _ = child.wait();
                let _ = tx.send(auth_status(&exe));
                on_done();
            })
            .map_err(|e| format!("could not wait for the Claude sign-in: {e}"))?;
        Ok(Self {
            rx,
            signing_in: true,
        })
    }

    /// The answer once it is in: `Some(Some(true))` signed in,
    /// `Some(Some(false))` not, `Some(None)` unknown; `None` still running.
    pub(crate) fn poll(&self) -> Option<Option<bool>> {
        match self.rx.try_recv() {
            Ok(answer) => Some(answer),
            Err(crossbeam_channel::TryRecvError::Empty) => None,
            Err(crossbeam_channel::TryRecvError::Disconnected) => Some(None),
        }
    }
}

/// Gives a child a console window of its own on Windows (the app may have
/// none), so the person can see and answer it.
fn own_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        cmd.creation_flags(CREATE_NEW_CONSOLE);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Finds the `claude` executable on `PATH`.
pub(crate) fn find() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AVE_CLAUDE").map(PathBuf::from)
        && p.is_file()
    {
        return Some(p);
    }
    let names: &[&str] = if cfg!(windows) {
        &["claude.exe", "claude.cmd", "claude.bat"]
    } else {
        &["claude"]
    };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
}

/// Everything a session is started with.
#[derive(Clone, Debug)]
pub(crate) struct Config {
    /// The `claude` executable.
    pub(crate) exe: PathBuf,
    /// The night's work folder (the agent's working directory).
    pub(crate) work: PathBuf,
    /// The MCP config file to pass.
    pub(crate) mcp_config: PathBuf,
    /// The appended system prompt, as a file.
    pub(crate) prompt_file: PathBuf,
    /// The folder of skill files the agent may Read.
    pub(crate) skills: PathBuf,
    /// A session to resume.
    pub(crate) resume: Option<String>,
    /// A model for `--model`, if the person picked one.
    pub(crate) model: Option<String>,
}

/// The built-in tools the agent keeps.
const BUILTIN: &str = "Read,Glob,Grep";

/// Tools denied outright, whatever a setting says.
const DENIED: &str =
    "Bash,PowerShell,Edit,Write,MultiEdit,NotebookEdit,WebFetch,WebSearch,Task,Agent";

/// The command-line arguments for a session.
pub(crate) fn args(cfg: &Config) -> Vec<String> {
    let mut a: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--strict-mcp-config",
        "--permission-mode",
        "dontAsk",
        "--setting-sources",
        "",
        "--disable-slash-commands",
        "--tools",
        BUILTIN,
        "--disallowedTools",
        DENIED,
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    a.push("--allowedTools".to_owned());
    a.push(format!("mcp__ave,{BUILTIN}"));
    a.push("--mcp-config".to_owned());
    a.push(cfg.mcp_config.display().to_string());
    a.push("--add-dir".to_owned());
    a.push(cfg.skills.display().to_string());
    a.push("--append-system-prompt-file".to_owned());
    a.push(cfg.prompt_file.display().to_string());
    if let Some(id) = &cfg.resume {
        a.push("--resume".to_owned());
        a.push(id.clone());
    }
    if let Some(m) = &cfg.model {
        a.push("--model".to_owned());
        a.push(m.clone());
    }
    a
}

/// The MCP config: `ave mcp --work <work>`, started with `ave`.
pub(crate) fn mcp_config(ave: &Path, work: &Path) -> Value {
    json!({"mcpServers": {"ave": {
        "type": "stdio",
        "command": ave.display().to_string(),
        "args": ["mcp", "--work", work.display().to_string()],
    }}})
}

/// One user turn as a stream-json line.
pub(crate) fn user_line(text: &str) -> String {
    json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": text}]},
        "parent_tool_use_id": null,
    })
    .to_string()
}

/// A tool call seen in the stream, kept until its result arrives.
#[derive(Clone, Debug)]
struct Call {
    /// Tool name, without the MCP prefix.
    name: String,
    /// Arguments.
    input: Value,
    /// For `plan_edit`: the plan and its version when the call was made.
    base: Option<(Plan, String)>,
}

/// Turns Claude Code's stdout lines into panel events.
#[derive(Debug)]
pub(crate) struct Stream {
    /// The night, to read plans for proposal cards.
    work: PathBuf,
    /// When the current turn started.
    started: Instant,
    /// The model the session runs.
    model: String,
    /// The session id from `init`.
    pub(crate) session_id: Option<String>,
    /// Calls waiting for results.
    calls: HashMap<String, Call>,
    /// The repeat guard.
    guard: Guard,
    /// Set when the CLI reported it is not signed in.
    pub(crate) auth_failed: bool,
    /// Set when the guard tripped: the process must be stopped.
    pub(crate) must_stop: bool,
    /// Whether a turn is running (a `result` not yet seen).
    pub(crate) busy: bool,
    /// The prompt size of the last request (from an assistant message's
    /// usage): how full the context is.
    context: u64,
}

impl Stream {
    /// A stream for a session in `work`.
    pub(crate) fn new(work: &Path) -> Self {
        Self {
            work: work.to_path_buf(),
            started: Instant::now(),
            model: "claude-code".to_owned(),
            session_id: None,
            calls: HashMap::new(),
            guard: Guard::default(),
            auth_failed: false,
            must_stop: false,
            busy: false,
            context: 0,
        }
    }

    /// A turn starts now.
    pub(crate) fn begin(&mut self) {
        self.started = Instant::now();
        self.guard = Guard::default();
        self.must_stop = false;
        self.busy = true;
    }

    /// The events one stdout line produces.
    pub(crate) fn line(&mut self, line: &str) -> Vec<Event> {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        if v["parent_tool_use_id"].is_string() {
            // A subagent's traffic; we run none, and show none.
            return Vec::new();
        }
        match v["type"].as_str().unwrap_or_default() {
            "system" => self.system(&v),
            "stream_event" => partial(&v["event"]),
            "assistant" => self.assistant(&v),
            "user" => self.user(&v),
            "result" => self.result(&v),
            _ => Vec::new(),
        }
    }

    /// `system` lines: `init` carries the session, model and MCP status.
    fn system(&mut self, v: &Value) -> Vec<Event> {
        if v["subtype"] != "init" {
            return Vec::new();
        }
        if let Some(id) = v["session_id"].as_str() {
            self.session_id = Some(id.to_owned());
        }
        if let Some(m) = v["model"].as_str() {
            m.clone_into(&mut self.model);
        }
        let mut out = Vec::new();
        let servers = v["mcp_servers"].as_array().cloned().unwrap_or_default();
        let ave = servers.iter().find(|s| s["name"] == "ave");
        for s in servers.iter().filter(|s| s["name"] != "ave") {
            log::warn!("claude code: unexpected MCP server {s}");
        }
        if servers.is_empty() {
            return out;
        }
        match ave.and_then(|s| s["status"].as_str()) {
            Some("connected") => {}
            Some(status) => out.push(Event::Notice {
                text: format!(
                    "ave's tool server did not start in Claude Code ({status}); the agent cannot \
                     read the project."
                ),
                error: true,
            }),
            None => out.push(Event::Notice {
                text: "ave's tool server is missing from Claude Code's session.".to_owned(),
                error: true,
            }),
        }
        out
    }

    /// A complete assistant content block (Claude Code sends one per line).
    fn assistant(&mut self, v: &Value) -> Vec<Event> {
        let m = &v["message"];
        let content = m["content"].as_array().cloned().unwrap_or_default();
        if let Some(kind) = v["error"].as_str() {
            let text = content
                .iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let auth = kind.contains("auth") || is_auth_text(&text);
            if auth {
                self.auth_failed = true;
            }
            return vec![Event::Notice {
                text: if auth {
                    format!("{text}. {LOGIN_HINT}")
                } else {
                    format!("Claude Code: {text} ({kind})")
                },
                error: true,
            }];
        }
        let u = &m["usage"];
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        let prompt =
            n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
        if prompt > 0 {
            self.context = prompt;
        }
        let mut out = Vec::new();
        let mut blocks = Vec::new();
        let mut looped: Option<String> = None;
        for b in &content {
            match b["type"].as_str().unwrap_or_default() {
                "text" => {
                    let text = b["text"].as_str().unwrap_or_default();
                    if !text.is_empty() {
                        blocks.push(Block::Text {
                            text: text.to_owned(),
                        });
                    }
                }
                "thinking" => blocks.push(Block::Thinking {
                    thinking: b["thinking"].as_str().unwrap_or_default().to_owned(),
                    // Claude Code keeps its own history; never resent by us.
                    signature: String::new(),
                }),
                "tool_use" => {
                    let id = b["id"].as_str().unwrap_or_default().to_owned();
                    let full = b["name"].as_str().unwrap_or_default();
                    let name = full.strip_prefix(MCP_PREFIX).unwrap_or(full).to_owned();
                    let input = b["input"].clone();
                    out.push(Event::Delta(Delta::Tool(tools::describe(
                        &name, &input, true,
                    ))));
                    if self.guard.repeated(&name, &input) && !self.must_stop {
                        self.must_stop = true;
                        looped = Some(name.clone());
                    }
                    let base = (name == EDIT).then(|| self.plan_now(&input)).flatten();
                    self.calls.insert(
                        id.clone(),
                        Call {
                            name: name.clone(),
                            input: input.clone(),
                            base,
                        },
                    );
                    blocks.push(Block::ToolUse { id, name, input });
                }
                _ => {}
            }
        }
        if !blocks.is_empty() {
            out.insert(0, Event::Msg(Msg::new(Role::Assistant, blocks)));
        }
        if let Some(name) = looped {
            out.push(Event::Notice {
                text: Guard::notice(&name),
                error: true,
            });
        }
        out
    }

    /// The plan an edit names, and its version, as it is on disk now.
    fn plan_now(&self, input: &Value) -> Option<(Plan, String)> {
        let name = input["name"].as_str()?;
        let path = self.work.join("plans").join(format!("{name}.toml"));
        Some((
            Plan::load(&path).ok()?,
            crate::mcp::plan_version(&path).ok()?,
        ))
    }

    /// A user line: tool results (our own turns are not echoed).
    fn user(&mut self, v: &Value) -> Vec<Event> {
        let content = v["message"]["content"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::new();
        let mut blocks = Vec::new();
        for b in content.iter().filter(|b| b["type"] == "tool_result") {
            let id = b["tool_use_id"].as_str().unwrap_or_default().to_owned();
            let is_error = b["is_error"].as_bool().unwrap_or(false);
            let (text, images) = match &b["content"] {
                Value::String(s) => (s.clone(), Vec::new()),
                Value::Array(parts) => tools::split_content(parts),
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => {
                    (String::new(), Vec::new())
                }
            };
            for (mime, data) in images {
                out.push(Event::Image {
                    tool_use_id: id.clone(),
                    mime,
                    data,
                });
            }
            if let Some(call) = self.calls.remove(&id)
                && !is_error
                && call.name == EDIT
                && let Some((base, base_version)) = call.base
            {
                out.extend(edit_events(&id, &call.input, base, base_version, &text));
            }
            blocks.push(Block::ToolResult {
                tool_use_id: id,
                content: text,
                is_error,
            });
        }
        if !blocks.is_empty() {
            out.insert(0, Event::Msg(Msg::new(Role::User, blocks)));
        }
        out
    }

    /// The turn's end: usage, cost, and any error.
    fn result(&mut self, v: &Value) -> Vec<Event> {
        self.busy = false;
        if let Some(id) = v["session_id"].as_str() {
            self.session_id = Some(id.to_owned());
        }
        let mut out = Vec::new();
        let text = v["result"].as_str().unwrap_or_default();
        if v["is_error"].as_bool().unwrap_or(false) && !self.auth_failed {
            if is_auth_text(text) {
                self.auth_failed = true;
                out.push(Event::Notice {
                    text: LOGIN_HINT.to_owned(),
                    error: true,
                });
            } else {
                let why = if text.is_empty() {
                    v["subtype"].as_str().unwrap_or("error").to_owned()
                } else {
                    text.to_owned()
                };
                out.push(Event::Notice {
                    text: format!("Claude Code stopped: {why}"),
                    error: true,
                });
            }
        }
        let u = &v["usage"];
        let n = |k: &str| u[k].as_u64().unwrap_or(0);
        let cached = n("cache_read_input_tokens");
        // `modelUsage` names each model's context window.
        let window = v["modelUsage"]
            .as_object()
            .and_then(|models| {
                models
                    .values()
                    .filter_map(|m| m["contextWindow"].as_u64())
                    .max()
            })
            .unwrap_or(0);
        out.push(Event::Done {
            secs: v["duration_ms"].as_f64().map_or_else(
                || self.started.elapsed().as_secs_f32(),
                |ms| (ms / 1000.0) as f32,
            ),
            model: self.model.clone(),
            usage: Usage {
                input: n("input_tokens") + cached + n("cache_creation_input_tokens"),
                cached,
                output: n("output_tokens"),
                context: self.context,
                window,
            },
            cost: v["total_cost_usd"].as_f64(),
        });
        out
    }
}

/// Whether an error text means "not signed in".
fn is_auth_text(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("authenticate")
        || t.contains("oauth")
        || t.contains("/login")
        || t.contains("not logged in")
        || t.contains("invalid api key")
}

/// Partial-message events: live text, thinking and tool starts.
fn partial(e: &Value) -> Vec<Event> {
    match e["type"].as_str().unwrap_or_default() {
        "content_block_delta" => {
            let d = &e["delta"];
            match d["type"].as_str().unwrap_or_default() {
                "text_delta" => vec![Event::Delta(Delta::Text(
                    d["text"].as_str().unwrap_or_default().to_owned(),
                ))],
                "thinking_delta" => vec![Event::Delta(Delta::Thinking(
                    d["thinking"].as_str().unwrap_or_default().to_owned(),
                ))],
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

/// The card for a `plan_edit` result: a staged proposal, or an applied one
/// when the agent committed it.
pub(crate) fn edit_events(
    id: &str,
    input: &Value,
    base: Plan,
    base_version: String,
    result: &str,
) -> Vec<Event> {
    let Ok(ops) = serde_json::from_value(input["ops"].clone()) else {
        return Vec::new();
    };
    let proposal = Proposal {
        id: id.to_owned(),
        plan: input["name"].as_str().unwrap_or_default().to_owned(),
        summary: input["reason"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned(),
        ops,
        base,
        base_version,
        ts: now(),
    };
    let mut out = vec![Event::Proposal(Box::new(proposal))];
    if let Some(version) = committed_version(result) {
        out.push(Event::Committed {
            id: id.to_owned(),
            version,
        });
    }
    out
}

/// `committed as v7` → 7.
pub(crate) fn committed_version(result: &str) -> Option<usize> {
    result
        .split("committed as v")
        .nth(1)?
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()
}

/// A running Claude Code process.
#[derive(Debug)]
pub(crate) struct Session {
    /// The process (shared with the reader, which stops it on a loop).
    child: Arc<Mutex<Child>>,
    /// Its stdin.
    stdin: ChildStdin,
    /// Events from the reader.
    pub(crate) rx: crossbeam_channel::Receiver<Event>,
    /// The stream state, shared with the reader.
    stream: Arc<Mutex<Stream>>,
    /// False once the process has exited.
    alive: Arc<AtomicBool>,
    /// The work folder and the chat (id) this session serves.
    pub(crate) key: (PathBuf, String),
}

impl Session {
    /// Starts `claude` with `cfg` for chat `chat`; `on_event` is called
    /// after each event (to repaint).
    pub(crate) fn start(
        cfg: &Config,
        chat: &str,
        on_event: impl Fn() + Send + 'static,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(&cfg.exe);
        cmd.args(args(cfg))
            .current_dir(&cfg.work)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::media::quiet(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start Claude Code ({}): {e}", cfg.exe.display()))?;
        let stdin = child.stdin.take().ok_or("Claude Code has no stdin")?;
        let stdout = child.stdout.take().ok_or("Claude Code has no stdout")?;
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(child));
        let stream = Arc::new(Mutex::new(Stream::new(&cfg.work)));
        let alive = Arc::new(AtomicBool::new(true));
        let tail = Arc::new(Mutex::new(Vec::<String>::new()));
        if let Some(err) = stderr {
            let tail = tail.clone();
            let _ = std::thread::Builder::new()
                .name("claude-code-stderr".to_owned())
                .spawn(move || {
                    for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                        log::debug!("claude code: {line}");
                        if let Ok(mut t) = tail.lock() {
                            t.push(line);
                            let extra = t.len().saturating_sub(6);
                            t.drain(..extra);
                        }
                    }
                });
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        let (r_child, r_stream, r_alive) = (child.clone(), stream.clone(), alive.clone());
        std::thread::Builder::new()
            .name("claude-code".to_owned())
            .spawn(move || {
                for line in std::io::BufReader::new(stdout)
                    .lines()
                    .map_while(Result::ok)
                {
                    let (events, stop) = match r_stream.lock() {
                        Ok(mut s) => {
                            let e = s.line(&line);
                            (e, s.must_stop)
                        }
                        Err(_) => break,
                    };
                    for e in events {
                        let _ = tx.send(e);
                    }
                    if stop {
                        if let Ok(mut c) = r_child.lock() {
                            let _ = c.kill();
                        }
                        break;
                    }
                    on_event();
                }
                r_alive.store(false, Ordering::Relaxed);
                let busy = r_stream.lock().map(|s| s.busy).unwrap_or(false);
                if busy {
                    let why = tail.lock().map(|t| t.join(" ")).unwrap_or_default();
                    let stopped = r_stream.lock().map(|s| s.must_stop).unwrap_or(false);
                    if !stopped {
                        let _ = tx.send(Event::Notice {
                            text: if why.trim().is_empty() {
                                "Claude Code exited before it finished.".to_owned()
                            } else {
                                format!("Claude Code exited: {}", why.trim())
                            },
                            error: true,
                        });
                    }
                    let _ = tx.send(Event::Done {
                        secs: 0.0,
                        model: "claude-code".to_owned(),
                        usage: Usage::default(),
                        cost: None,
                    });
                }
                on_event();
            })
            .map_err(|e| format!("could not start the Claude Code reader: {e}"))?;
        Ok(Self {
            child,
            stdin,
            rx,
            stream,
            alive,
            key: (cfg.work.clone(), chat.to_owned()),
        })
    }

    /// Whether the process is still running.
    pub(crate) fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// The session id, once `init` has been seen.
    pub(crate) fn session_id(&self) -> Option<String> {
        self.stream.lock().ok().and_then(|s| s.session_id.clone())
    }

    /// Whether the CLI said it is not signed in.
    pub(crate) fn auth_failed(&self) -> bool {
        self.stream.lock().is_ok_and(|s| s.auth_failed)
    }

    /// Sends one user turn.
    pub(crate) fn send(&mut self, text: &str) -> Result<(), String> {
        if let Ok(mut s) = self.stream.lock() {
            s.begin();
        }
        writeln!(self.stdin, "{}", user_line(text))
            .and_then(|()| self.stdin.flush())
            .map_err(|e| format!("Claude Code is not listening: {e}"))
    }

    /// Stops the process (Stop, a new chat, or the panel going away).
    pub(crate) fn kill(&self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.alive.store(false, Ordering::Relaxed);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The file chat `chat`'s session id is kept in:
/// `<work>/chat/chats/<id>.session`.
pub(crate) fn session_file(work: &Path, chat: &str) -> PathBuf {
    super::store::session_path(work, chat)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use super::{Config, Event, Stream};
    use crate::gui::assistant::model::{Block, Role};
    use crate::gui::assistant::provider::Delta;

    /// Recorded from `claude` 2.1.292 with an expired sign-in.
    const LOGGED_OUT: [&str; 3] = [
        r#"{"type":"system","subtype":"init","cwd":"C:\\night\\_ave","session_id":"0a85d52d-21d9-48f3-bdb6-77c696b7ae8f","tools":["Glob","Grep","Read"],"mcp_servers":[],"model":"claude-opus-5-5","permissionMode":"dontAsk","slash_commands":[],"apiKeySource":"none","skills":[],"plugins":[]}"#,
        r#"{"type":"assistant","message":{"id":"ebbbf6af","model":"<synthetic>","role":"assistant","stop_reason":"stop_sequence","type":"message","usage":{"input_tokens":0,"output_tokens":0},"content":[{"type":"text","text":"Failed to authenticate: OAuth session expired and could not be refreshed"}]},"parent_tool_use_id":null,"session_id":"0a85d52d-21d9-48f3-bdb6-77c696b7ae8f","error":"authentication_failed"}"#,
        r#"{"type":"result","subtype":"success","is_error":true,"duration_ms":591,"duration_api_ms":0,"num_turns":1,"result":"Failed to authenticate: OAuth session expired and could not be refreshed","stop_reason":"stop_sequence","session_id":"0a85d52d-21d9-48f3-bdb6-77c696b7ae8f","total_cost_usd":0,"usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}}"#,
    ];

    /// A signed-in turn, in the shapes Claude Code emits: init with our MCP
    /// server, partial deltas, one assistant line per content block, a tool
    /// result with an image, and the result with usage and cost.
    const TURN: [&str; 9] = [
        r#"{"type":"system","subtype":"init","session_id":"s-1","model":"claude-opus-5-5","tools":["Read","mcp__ave__search_shots","mcp__ave__plan_edit"],"mcp_servers":[{"name":"ave","status":"connected"}]}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Find fire."}},"parent_tool_use_id":null,"session_id":"s-1"}"#,
        r#"{"type":"assistant","message":{"id":"m1","role":"assistant","model":"claude-opus-5-5","content":[{"type":"thinking","thinking":"Find fire.","signature":"sig"}]},"parent_tool_use_id":null,"session_id":"s-1"}"#,
        r#"{"type":"assistant","message":{"id":"m1","role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"mcp__ave__look","input":{"items":[{"segment":"A/1"}]}}]},"parent_tool_use_id":null,"session_id":"s-1"}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":[{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"/9j/AA=="}},{"type":"text","text":"1 A @2.00"}]}]},"parent_tool_use_id":null,"session_id":"s-1"}"#,
        r#"{"type":"assistant","message":{"id":"m2","role":"assistant","content":[{"type":"tool_use","id":"toolu_2","name":"mcp__ave__plan_edit","input":{"name":"t","reason":"fire on 2","ops":[{"op":"set_note","index":2,"note":"fire 9%"}]}}]},"parent_tool_use_id":null,"session_id":"s-1"}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_2","content":[{"type":"text","text":"set note: #2 B @10.00 → #2 B @10.00\ncheck: 0 problems after (0 new, 0 fixed)\nstaged, not saved: the person sees it as a card to apply. base_version: 0123456789abcdef\n"}]}]},"parent_tool_use_id":null,"session_id":"s-1"}"#,
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Staged "}},"parent_tool_use_id":null,"session_id":"s-1"}"#,
        r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":4200,"num_turns":3,"result":"Staged [shot 2](ave://shot/t/2).","session_id":"s-1","total_cost_usd":0.0123,"usage":{"input_tokens":20,"cache_creation_input_tokens":100,"cache_read_input_tokens":880,"output_tokens":64}}"#,
    ];

    fn feed(stream: &mut Stream, lines: &[&str]) -> Vec<Event> {
        lines.iter().flat_map(|l| stream.line(l)).collect()
    }

    #[test]
    fn a_logged_out_cli_says_how_to_log_in() -> anyhow::Result<()> {
        let mut s = Stream::new(&PathBuf::from("."));
        s.begin();
        let events = feed(&mut s, &LOGGED_OUT);
        assert!(s.auth_failed, "auth failure noticed");
        let notices: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                Event::Notice { text, .. } => Some(text.as_str()),
                Event::Delta(_)
                | Event::Msg(_)
                | Event::Proposal(_)
                | Event::Committed { .. }
                | Event::Image { .. }
                | Event::Ask { .. }
                | Event::Done { .. } => None,
            })
            .collect();
        assert_eq!(notices.len(), 1, "one notice, not one per line: {events:?}");
        assert!(notices[0].contains("Press Sign in"), "{}", notices[0]);
        assert!(
            matches!(events.last(), Some(Event::Done { .. })),
            "the turn ends"
        );
        assert_eq!(
            s.session_id.as_deref(),
            Some("0a85d52d-21d9-48f3-bdb6-77c696b7ae8f"),
            "session id kept"
        );
        Ok(())
    }

    #[test]
    fn auth_status_is_read_from_the_cli_json() -> anyhow::Result<()> {
        // The shape of `claude auth status --json` from `claude` 2.1.330.
        let yes = r#"{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","subscriptionType":"max"}"#;
        assert_eq!(super::signed_in(yes), Some(true), "signed in");
        assert_eq!(
            super::signed_in("{\"loggedIn\": false}\n"),
            Some(false),
            "signed out"
        );
        assert_eq!(
            super::signed_in("error: unknown command 'auth'"),
            None,
            "an older CLI is unknown, not signed out"
        );
        Ok(())
    }

    #[test]
    fn a_turn_maps_onto_panel_events() -> anyhow::Result<()> {
        let dir = crate::mcp::tests::night(3)?;
        let mut s = Stream::new(dir.path());
        s.begin();
        let events = feed(&mut s, &TURN);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Delta(Delta::Thinking(t)) if t == "Find fire.")),
            "live thinking"
        );
        assert!(
            events.iter().any(
                |e| matches!(e, Event::Delta(Delta::Tool(t)) if t == "Making a contact sheet")
            ),
            "tool status line"
        );
        assert!(
            events.iter().any(|e| matches!(e, Event::Msg(m) if m.role == Role::Assistant
                && m.blocks.iter().any(|b| matches!(b, Block::ToolUse { name, .. } if name == "look")))),
            "tool call recorded without the MCP prefix"
        );
        assert!(
            events.iter().any(
                |e| matches!(e, Event::Image { tool_use_id, mime, .. } if tool_use_id == "toolu_1" && mime == "image/jpeg")
            ),
            "the contact sheet surfaces"
        );
        let proposal = events.iter().find_map(|e| match e {
            Event::Proposal(p) => Some(p.clone()),
            Event::Delta(_)
            | Event::Msg(_)
            | Event::Committed { .. }
            | Event::Image { .. }
            | Event::Ask { .. }
            | Event::Notice { .. }
            | Event::Done { .. } => None,
        });
        let proposal = proposal.ok_or_else(|| anyhow::anyhow!("no card: {events:?}"))?;
        assert_eq!(proposal.plan, "t", "card for plan t");
        assert_eq!(proposal.base.shots.len(), 3, "base read at call time");
        assert_eq!(proposal.base_version.len(), 16, "with its version");
        assert!(
            !events.iter().any(|e| matches!(e, Event::Committed { .. })),
            "staged, not committed"
        );
        assert!(
            matches!(events.last(), Some(Event::Done { usage, cost: Some(c), secs, .. })
                if usage.input == 1000 && usage.cached == 880 && (*c - 0.0123).abs() < 1e-9 && (*secs - 4.2).abs() < 1e-3),
            "usage and cost: {:?}",
            events.last()
        );
        assert!(!s.busy, "turn over");
        Ok(())
    }

    #[test]
    fn the_last_request_says_how_full_the_context_is() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut s = Stream::new(dir.path());
        s.begin();
        let events = feed(
            &mut s,
            &[
                r#"{"type":"assistant","message":{"id":"m1","role":"assistant","usage":{"input_tokens":5,"cache_read_input_tokens":40000,"cache_creation_input_tokens":2000,"output_tokens":9},"content":[{"type":"text","text":"Hi"}]},"parent_tool_use_id":null,"session_id":"s"}"#,
                r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":10,"session_id":"s","usage":{"input_tokens":5,"output_tokens":9},"modelUsage":{"claude-opus-5-5":{"inputTokens":5,"contextWindow":200000}}}"#,
            ],
        );
        assert!(
            matches!(events.last(), Some(Event::Done { usage, .. })
                if usage.context == 42_005 && usage.window == 200_000),
            "context and window: {:?}",
            events.last()
        );
        Ok(())
    }

    #[test]
    fn three_identical_calls_stop_the_turn() -> anyhow::Result<()> {
        let mut s = Stream::new(&PathBuf::from("."));
        s.begin();
        let call = |id: &str| {
            json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": id,
                "name": "mcp__ave__search_shots", "input": {"query": "fire"}}]}})
            .to_string()
        };
        s.line(&call("a"));
        s.line(&call("b"));
        assert!(!s.must_stop, "two are fine");
        let events = s.line(&call("c"));
        assert!(s.must_stop, "the third stops");
        assert!(
            events.iter().any(
                |e| matches!(e, Event::Notice { text, error: true } if text.contains("search_shots"))
            ),
            "the person is told: {events:?}"
        );
        Ok(())
    }

    #[test]
    fn committed_edits_and_server_problems() -> anyhow::Result<()> {
        assert_eq!(
            super::committed_version("committed as v7; version now abc"),
            Some(7),
            "version parsed"
        );
        assert_eq!(super::committed_version("staged"), None, "staged");
        let mut s = Stream::new(&PathBuf::from("."));
        let events = s.line(
            r#"{"type":"system","subtype":"init","session_id":"x","mcp_servers":[{"name":"ave","status":"failed"}]}"#,
        );
        assert!(
            matches!(&events[..], [Event::Notice { error: true, text }] if text.contains("failed")),
            "a dead tool server is reported: {events:?}"
        );
        Ok(())
    }

    #[test]
    fn arguments_isolate_the_session() -> anyhow::Result<()> {
        let cfg = Config {
            exe: "claude".into(),
            work: "w".into(),
            mcp_config: "w/chat/claude-mcp.json".into(),
            prompt_file: "w/chat/claude-prompt.md".into(),
            skills: "skills".into(),
            resume: Some("s-1".into()),
            model: None,
        };
        let a = super::args(&cfg);
        let after = |flag: &str| {
            a.iter()
                .position(|x| x == flag)
                .and_then(|i| a.get(i + 1))
                .cloned()
                .unwrap_or_default()
        };
        assert_eq!(after("--input-format"), "stream-json", "stream input");
        assert_eq!(after("--setting-sources"), "", "no user settings");
        assert_eq!(after("--permission-mode"), "dontAsk", "nothing prompts");
        assert_eq!(after("--tools"), "Read,Glob,Grep", "read-only built-ins");
        assert!(after("--disallowedTools").contains("Bash"), "no shell");
        assert!(after("--allowedTools").starts_with("mcp__ave"), "our tools");
        assert_eq!(after("--resume"), "s-1", "resumes");
        assert!(
            a.contains(&"--strict-mcp-config".to_owned()),
            "only our MCP"
        );
        let line: serde_json::Value = serde_json::from_str(&super::user_line("hi"))?;
        assert_eq!(line["message"]["content"][0]["text"], "hi", "user line");
        let mcp = super::mcp_config(&PathBuf::from("ave.exe"), &PathBuf::from("w"));
        assert_eq!(mcp["mcpServers"]["ave"]["args"][0], "mcp", "ave mcp");
        Ok(())
    }
}

// ============================================================================
// Module:       gui::assistant
// Description:  The chat panel left of the workspace: a conversation about a
//               plan, reference chips and an `@` composer, and the model's
//               edits shown as diff cards to apply, preview or discard.
//
// Dependencies: egui; crate::gui::pages (PageCtx); crate::{plan, refs}; the
//               submodules for everything that is not drawing
// ============================================================================

//! The assistant panel.
//!
//! **Contract.** The workspace calls [`panel`] inside the chat pane. The
//! assistant never edits a plan by itself: the model's `propose_edit` calls
//! become cards showing every operation before → after with thumbnails, its
//! reason and the predicted rule check. **Apply** writes a new plan version
//! (`plans/.history/<plan>/v<N>.toml`), so undoing an agent turn is reverting
//! to a version, never a step in anyone's undo stack.
//!
//! The pieces (all but this file are plain Rust, tested without a window):
//!
//! | Module | What |
//! |---|---|
//! | [`model`] | the conversation and chat-log lines |
//! | [`context`] | system prompt, per-message snapshot, the shot log copy |
//! | [`tools`] | tool schemas and running them |
//! | [`ops`] | edit operations, applying them, scope and beat checks |
//! | [`agent`] | the tool loop of one turn, the events, the repeat guard |
//! | [`claude_code`] | Claude Code as the runtime, over stream-json |
//! | [`provider`] | Claude (streamed) and a local llama.cpp server |
//! | [`skills`] | the agent's skills and trigger matching |
//! | [`links`] | `ave://` links in the agent's text, shown as chips |
//! | [`store`] | chat logs, plan versions, preferences |
//! | [`complete`] | the `@` autocomplete and chip/token merging |
//! | [`look`] | the chat's own drawing pieces (activity rows, the mode switch) |
//! | [`cards`] | the first-cut settings card, step approvals and their progress |
//! | [`markdown`] | the bold/bullet text the model writes |
//!
//! A conversation belongs to one plan and lives in `<work>/chat/<plan>.jsonl`;
//! switching plans switches conversations, and reopening the night restores
//! the panel as it was.

mod agent;
mod cards;
mod claude_code;
mod complete;
mod context;
mod links;
mod look;
mod markdown;
mod model;
pub(crate) mod ops;
mod provider;
mod skills;
pub(crate) mod store;
mod tools;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};

use egui::{Align, CornerRadius, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, Vec2};

use self::agent::Event;
use self::model::{Block, LogLine, Msg, Proposal, Role, Status};
use self::ops::{EditOp, Effect, Night, OpError, Scope};
use self::provider::Delta;
use crate::gui::app::Action;
use crate::gui::pages::PageCtx;
use crate::gui::ui::theme::{self, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::gui::ui::{icons, widgets};
use crate::plan::{Plan, Problem, Shot};
use crate::project::Project;
use crate::refs::{Modifier, Reference, Target};

/// Models the selector offers: id and label. `claude-code` is the Claude
/// Code CLI on the person's own plan, `local` the llama.cpp server.
const MODELS: [(&str, &str); 5] = [
    (CLAUDE_CODE, "Claude Code"),
    ("claude-opus-5-5", "Claude Opus 5.5"),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
    ("claude-haiku-5-5", "Claude Haiku 5.5"),
    ("local", "Local (llama.cpp)"),
];

/// The runtime id of Claude Code in [`MODELS`].
const CLAUDE_CODE: &str = "claude-code";

/// The settings key this panel uses in `ave.toml`'s `[pages]`.
const PAGE_KEY: &str = crate::mcp::PAGE_KEY;

/// What the request may touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeChoice {
    /// The whole plan.
    Whole,
    /// The shots selected or referenced.
    Selected,
    /// Shots before the drop.
    Build,
    /// Shots from the drop on.
    FromDrop,
}

impl ScopeChoice {
    /// Chip label.
    fn label(self) -> &'static str {
        match self {
            Self::Whole => "Whole plan",
            Self::Selected => "Selected shots",
            Self::Build => "Before the drop",
            Self::FromDrop => "From the drop",
        }
    }
}

/// The panel's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Tab {
    /// The conversation.
    #[default]
    Chat,
    /// Applied proposals and plan versions.
    History,
}

/// What the composer box is for, as Selects' Chat / Comments switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Mode {
    /// Talking to the assistant.
    #[default]
    Chat,
    /// The open plan's review comments.
    Comments,
}

/// A question the model is waiting on.
#[derive(Clone, Debug)]
struct Ask {
    /// The `ask_user` call id.
    id: String,
    /// The question.
    question: String,
    /// Offered answers.
    options: Vec<String>,
    /// Results of the same step's other calls, sent with the answer.
    held: Vec<Block>,
}

/// The open conversation.
#[derive(Debug)]
struct Chat {
    /// The night's work folder.
    work: PathBuf,
    /// The plan (file stem).
    plan: String,
    /// The log, in order.
    log: Vec<LogLine>,
    /// A question waiting for an answer.
    ask: Option<Ask>,
    /// The plan text last sent, so an unchanged plan is not resent.
    sent_plan: Option<String>,
    /// The plan file as last read, and its modification time.
    cached: Option<(SystemTime, Plan)>,
}

impl Chat {
    /// The plan file's path.
    fn plan_path(&self) -> PathBuf {
        self.work.join("plans").join(format!("{}.toml", self.plan))
    }

    /// The chat log's path.
    fn log_path(&self) -> PathBuf {
        store::chat_path(&self.work, &self.plan)
    }

    /// The plan as it is on disk now (re-read when the file changes).
    fn current(&mut self) -> Option<Plan> {
        let path = self.plan_path();
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        if self.cached.as_ref().is_none_or(|(t, _)| *t != mtime) {
            let plan = Plan::load(&path).ok()?;
            self.cached = Some((mtime, plan));
        }
        self.cached.as_ref().map(|(_, p)| p.clone())
    }

    /// Appends and persists a log line; returns a write error to show.
    fn push(&mut self, line: LogLine) -> Option<String> {
        let err = store::append(&self.log_path(), &line)
            .err()
            .map(|e| format!("could not save the conversation: {e:#}"));
        self.log.push(line);
        err
    }
}

/// A turn in progress.
#[derive(Debug)]
struct Turn {
    /// Events from the worker.
    rx: crossbeam_channel::Receiver<Event>,
    /// Set to stop the worker.
    cancel: Arc<AtomicBool>,
    /// When it started.
    started: Instant,
    /// Text streamed since the last complete message.
    text: String,
    /// Thinking streamed since the last complete message.
    thinking: String,
    /// Tools announced since the last complete message.
    tools: Vec<String>,
    /// Whether Claude Code runs this turn (Stop kills its process).
    claude: bool,
}

/// A proposal card's computed state.
#[derive(Debug)]
struct Card {
    /// Which ops are ticked.
    picked: Vec<bool>,
    /// The ticks the result was computed for.
    computed: Vec<bool>,
    /// The plan after the ticked ops, and what each did.
    result: Result<(Plan, Vec<Effect>), OpError>,
    /// The check before.
    before: Option<Vec<Problem>>,
    /// The check after.
    after: Option<Vec<Problem>>,
}

/// The panel's state: the conversation and any pending proposal.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// Preferences, loaded on first draw.
    prefs: Option<store::Prefs>,
    /// The open conversation.
    chat: Option<Chat>,
    /// The composer's text.
    input: String,
    /// The running turn.
    turn: Option<Turn>,
    /// The scope the person picked (`None`: follow the selection).
    scope: Option<ScopeChoice>,
    /// The tab on show.
    tab: Tab,
    /// Chat or comments, as the composer's switch says.
    mode: Mode,
    /// Whether the settings strip is open.
    settings_open: bool,
    /// The API key being typed.
    key_draft: String,
    /// The local URL being typed.
    url_draft: String,
    /// The highlighted autocomplete row.
    pick: usize,
    /// The text at which the autocomplete was dismissed with Escape.
    dismissed: Option<String>,
    /// Proposal cards, by proposal id.
    cards: HashMap<String, Card>,
    /// The last error to show.
    error: Option<String>,
    /// The `claude` executable, looked up once (`Some(None)`: not installed).
    claude: Option<Option<PathBuf>>,
    /// The running Claude Code session.
    session: Option<claude_code::Session>,
    /// Why Claude Code cannot be used right now (not signed in).
    claude_down: Option<String>,
    /// Contact sheets shown under tool calls.
    images: Sheets,
    /// The "Here's our plan" card, while open.
    first_cut: Option<cards::FirstCut>,
    /// The first cut being drafted: its job, plan name and settings.
    drafting: Option<(u64, String, cards::FirstCut)>,
    /// Slow steps the agent asked for, waiting for the person.
    approvals: Vec<cards::Approval>,
    /// Steps the chat started, followed until they end.
    running: Vec<cards::Running>,
}

/// Contact-sheet textures by file (`None`: unreadable).
#[derive(Default)]
struct Sheets(HashMap<String, Option<egui::TextureHandle>>);

impl std::fmt::Debug for Sheets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Sheets").field(&self.0.len()).finish()
    }
}

/// Draws the panel.
pub(crate) fn panel(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    // Messages handed over by other regions (timeline comments) land in the
    // composer for the user to send.
    for message in ctx.chat_inbox.drain(..) {
        if !state.input.is_empty() {
            state.input.push_str(
                "

",
            );
        }
        state.input.push_str(&message);
    }
    if state.prefs.is_none() {
        let prefs = store::Prefs::load();
        state.url_draft = prefs.local();
        state.prefs = Some(prefs);
    }
    if state.claude.is_none() {
        state.claude = Some(claude_code::find());
    }
    let Some(project) = ctx.project.as_ref() else {
        widgets::panel_header(ui, ctx.theme, "Assistant", |_| {});
        widgets::empty_state(
            ui,
            ctx.theme,
            icons::MESSAGE_SQUARE,
            "No night open",
            "Open a night to talk about its plans.",
            |_| {},
        );
        return;
    };
    sync_chat(project, state, &ctx.selection.clone());
    poll(ctx, state, ui.ctx());
    tend_approvals(ctx, state);
    follow_jobs(ctx, state);

    header(ctx, state, ui);
    if state.settings_open {
        settings(ctx, state, ui);
    }
    if state.chat.is_none() {
        if let Some(e) = state.error.clone() {
            error_row(ctx, state, ui, &e);
        }
        widgets::empty_state(
            ui,
            ctx.theme,
            icons::FILE_TEXT,
            "No plans yet",
            "Draft one (ave plan) and the assistant can edit it.",
            |_| {},
        );
        return;
    }
    match state.tab {
        Tab::Chat => {
            // Selects' composer form: 12 px in from the panel's sides, 12
            // px off its foot (the panel's own inset is 8).
            egui::Panel::bottom("assistant_composer")
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::new().inner_margin(Margin {
                    left: theme::as_margin(SPACE_XS),
                    right: theme::as_margin(SPACE_XS),
                    top: theme::as_margin(SPACE_SM),
                    bottom: theme::as_margin(SPACE_XS),
                }))
                .show(ui, |ui| composer(ctx, state, ui));
            match state.mode {
                Mode::Chat => conversation(ctx, state, ui),
                Mode::Comments => comments(ctx, state, ui),
            }
        }
        Tab::History => {
            if let Some(e) = state.error.clone() {
                error_row(ctx, state, ui, &e);
            }
            history(ctx, state, ui);
        }
    }
}

/// The last error as a notice row with a dismiss button.
fn error_row(ctx: &PageCtx<'_>, state: &mut State, ui: &mut Ui, error: &str) {
    look::notice_row(ui, ctx.theme, error, look::Tone::Error, |ui| {
        if widgets::icon_btn(ui, ctx.theme, icons::X, "Dismiss", "").clicked() {
            state.error = None;
        }
    });
}

/// Opens the right conversation: the plan saved for this night, else the
/// plan of a selected shot, else the first plan.
fn sync_chat(project: &Project, state: &mut State, selection: &[Reference]) {
    let stems: Vec<String> = project
        .plans
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    if let Some(chat) = &state.chat
        && chat.work == project.work
        && stems.contains(&chat.plan)
    {
        return;
    }
    if state.turn.is_some() {
        return;
    }
    let saved = project
        .settings
        .pages
        .get(PAGE_KEY)
        .and_then(|t| t.get("plan"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned);
    let selected = selection.iter().find_map(|r| match &r.target {
        Target::Shot { plan, .. } | Target::Plan { name: plan } => Some(plan.clone()),
        Target::Segment { .. }
        | Target::Range { .. }
        | Target::Frame { .. }
        | Target::Bars { .. }
        | Target::Person { .. } => None,
    });
    let plan = saved
        .filter(|p| stems.contains(p))
        .or(selected.filter(|p| stems.contains(p)))
        .or_else(|| stems.first().cloned());
    state.chat = plan.map(|plan| open_chat(&project.work, &plan));
    reopened(state);
}

/// After another conversation opened: drop the old one's cards and read
/// back any step request still waiting for an answer.
fn reopened(state: &mut State) {
    state.cards.clear();
    state.first_cut = None;
    state.approvals = state
        .chat
        .as_ref()
        .map(|c| {
            model::unanswered(&model::messages(&c.log))
                .into_iter()
                .filter(|(_, name, _)| name == crate::mcp::RUN_STEP)
                .filter_map(|(id, _, input)| {
                    cards::Step::from_input(&input).map(|step| cards::Approval {
                        id,
                        step,
                        estimate: None,
                        restored: true,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
}

/// Adds a notice line to the conversation.
fn note(state: &mut State, text: String, error: bool) {
    if let Some(chat) = state.chat.as_mut()
        && let Some(e) = chat.push(LogLine::Notice { text, error })
    {
        state.error = Some(e);
    }
}

/// A tool call that needs the person before anything happens: a slow step
/// to approve, or a first cut asked for without settings.
fn intercept(
    ctx: &PageCtx<'_>,
    state: &mut State,
    id: &str,
    name: &str,
    input: &serde_json::Value,
) {
    if name == crate::mcp::RUN_STEP {
        if let Some(step) = cards::Step::from_input(input)
            && !state.approvals.iter().any(|a| a.id == id)
        {
            state.approvals.push(cards::Approval {
                id: id.to_owned(),
                step,
                estimate: None,
                restored: false,
            });
        }
    } else if name == "plan_draft"
        && input.get("seconds").is_none()
        && input.get("aspect").is_none()
        && ctx
            .project
            .as_ref()
            .is_some_and(|p| p.index.is_some() && p.music.is_some())
    {
        state.first_cut = Some(cards::FirstCut::new(
            ctx.selection,
            Some(id.to_owned()),
            input,
        ));
    }
}

/// Estimates the waiting steps once each, and starts them at once when the
/// person lets the assistant act by itself.
fn tend_approvals(ctx: &mut PageCtx<'_>, state: &mut State) {
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    for a in state.approvals.iter_mut().filter(|a| a.estimate.is_none()) {
        a.estimate = Some(a.step.estimate(project));
    }
    if allow_commit(ctx) {
        let auto: Vec<String> = state
            .approvals
            .iter()
            .filter(|a| !a.restored && matches!(a.estimate, Some(Ok(_))))
            .map(|a| a.id.clone())
            .collect();
        for id in auto {
            decide(ctx, state, &id, cards::Choice::Go, true);
        }
    }
}

/// Approves or cancels step request `id`.
fn decide(ctx: &mut PageCtx<'_>, state: &mut State, id: &str, choice: cards::Choice, auto: bool) {
    let Some(pos) = state.approvals.iter().position(|a| a.id == id) else {
        return;
    };
    let a = state.approvals.remove(pos);
    let what = a.step.what();
    let (text, error) = match choice {
        cards::Choice::Cancel => (format!("Not started: {what}."), false),
        cards::Choice::Go => {
            let job = ctx
                .project
                .as_ref()
                .ok_or_else(|| "no night is open".to_owned())
                .and_then(|p| a.step.job(p));
            match job {
                Ok((label, cmd)) => {
                    let job = ctx.jobs.spawn(label, cmd);
                    state.running.push(cards::Running {
                        job,
                        step: a.step.clone(),
                        auto,
                    });
                    let about = match &a.estimate {
                        Some(Ok(e)) => {
                            format!(", about {}", crate::project::duration_text(e.secs))
                        }
                        Some(Err(_)) | None => String::new(),
                    };
                    let how = if auto { "Auto-approved" } else { "Approved" };
                    (format!("{how}: {what}{about}."), false)
                }
                Err(e) => (format!("Could not start {what}: {e}"), true),
            }
        }
    };
    note(state, text, error);
}

/// Follows the jobs the chat started: a line when each ends, and for a
/// first cut, its rules written and the agent's part put in the composer.
fn follow_jobs(ctx: &mut PageCtx<'_>, state: &mut State) {
    use crate::jobs::Status as Job;
    let mut notes = Vec::new();
    state.running.retain(|r| {
        let Some(job) = ctx.jobs.get(r.job) else {
            return false;
        };
        let what = r.step.what();
        let took = crate::project::duration_text(job.took.unwrap_or_default());
        match &job.status {
            Job::Running => return true,
            Job::Done => notes.push((
                format!("{} finished in {took}.", cards::capitalised(&what)),
                false,
            )),
            Job::Failed(_) if job.cancelled() => {
                notes.push((format!("Stopped: {what}. What finished is kept."), false));
            }
            Job::Failed(e) => {
                notes.push((format!("{} failed: {e}", cards::capitalised(&what)), true))
            }
        }
        false
    });
    for (text, error) in notes {
        note(state, text, error);
    }
    let Some((job, name, card)) = state.drafting.clone() else {
        return;
    };
    let status = ctx.jobs.get(job).map(|j| j.status.clone());
    match status {
        Some(Job::Running) => {}
        Some(Job::Done) => {
            state.drafting = None;
            let Some(project) = ctx.project.as_mut() else {
                return;
            };
            let path = project.work.join("plans").join(format!("{name}.toml"));
            let rules = Plan::load(&path).and_then(|mut plan| {
                let before = plan.rules.clone();
                card.apply_rules(&mut plan.rules);
                if plan.rules != before {
                    plan.save(&path)?;
                }
                Ok(())
            });
            project.reload_plans();
            match rules {
                Ok(()) => note(state, format!("Drafted {name}: {}.", card.summary()), false),
                Err(e) => note(
                    state,
                    format!("Drafted {name}, but its rules could not be set: {e:#}"),
                    true,
                ),
            }
            if let Some(message) = card.follow_up(&name) {
                if !state.input.trim().is_empty() {
                    state.input.push_str("\n\n");
                }
                state.input.push_str(&message);
            }
        }
        Some(Job::Failed(e)) => {
            state.drafting = None;
            note(state, format!("Drafting {name} failed: {e}"), true);
        }
        None => state.drafting = None,
    }
}

/// Continue on the first-cut card: the plan page's draft job for these
/// settings.
fn start_first_cut(ctx: &mut PageCtx<'_>, state: &mut State) {
    let Some(card) = state.first_cut.as_mut() else {
        return;
    };
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    if project.index.is_none() || project.music.is_none() {
        card.error = Some("Index the footage and map the music first.".to_owned());
        return;
    }
    if state.drafting.is_some() {
        card.error = Some("A first cut is being drafted already.".to_owned());
        return;
    }
    let taken: Vec<String> = project
        .plans
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    let name = card.name(&taken);
    let (label, cmd) =
        crate::gui::pages::plan::draft_job(project, &name, card.aspect, card.seconds());
    let job = ctx.jobs.spawn(label, cmd);
    let card = card.clone();
    state.first_cut = None;
    note(
        state,
        format!("Drafting {name}: {}\u{2026}", card.summary()),
        false,
    );
    state.drafting = Some((job, name, card));
}

/// Loads the conversation of `plan`.
fn open_chat(work: &Path, plan: &str) -> Chat {
    let (log, bad) = store::load(&store::chat_path(work, plan));
    if bad > 0 {
        log::warn!("{plan}: {bad} unreadable chat lines skipped");
    }
    let msgs = model::messages(&log);
    // A question still open when the app closed is still open.
    let ask = model::unanswered(&msgs)
        .into_iter()
        .find(|(_, name, _)| name == tools::ASK)
        .map(|(id, _, input)| Ask {
            id,
            question: input["question"].as_str().unwrap_or_default().to_owned(),
            options: input["options"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|o| o.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            held: Vec::new(),
        });
    Chat {
        work: work.to_path_buf(),
        plan: plan.to_owned(),
        log,
        ask,
        sent_plan: None,
        cached: None,
    }
}

/// Holds or releases the timeline lock (`plans/.lock-<plan>`) for the
/// conversation's plan: the timeline shows read-only while a turn may edit it.
fn set_lock(state: &State, on: bool) {
    let Some(chat) = state.chat.as_ref() else {
        return;
    };
    let path = chat.work.join("plans").join(format!(".lock-{}", chat.plan));
    if on {
        let _ = std::fs::write(&path, "assistant turn");
    } else {
        let _ = std::fs::remove_file(&path);
    }
}

/// Drains the turn's events into the conversation.
fn poll(ctx: &mut PageCtx<'_>, state: &mut State, egui_ctx: &egui::Context) {
    let Some(turn) = state.turn.as_mut() else {
        return;
    };
    egui_ctx.request_repaint_after(std::time::Duration::from_millis(100));
    if turn.claude && turn.cancel.load(Ordering::Relaxed) {
        // Stop: the process goes; the next message resumes the session.
        state.session = None;
        state.turn = None;
        set_lock(state, false);
        if let Some(chat) = state.chat.as_mut() {
            state.error = chat.push(LogLine::Notice {
                text: "Stopped.".to_owned(),
                error: false,
            });
        }
        reload_after_edits(ctx, state);
        return;
    }
    let mut done = false;
    let mut proposals = Vec::new();
    let mut committed = Vec::new();
    let mut calls: Vec<(String, String, serde_json::Value)> = Vec::new();
    while let Ok(event) = turn.rx.try_recv() {
        let Some(chat) = state.chat.as_mut() else {
            break;
        };
        let err =
            match event {
                Event::Delta(Delta::Text(t)) => {
                    turn.text.push_str(&t);
                    None
                }
                Event::Delta(Delta::Thinking(t)) => {
                    turn.thinking.push_str(&t);
                    None
                }
                Event::Delta(Delta::Tool(name)) => {
                    // The in-process loop announces bare names; Claude Code
                    // sends a described label.
                    turn.tools.push(if name.contains(' ') {
                        name
                    } else {
                        tools::describe(&name, &serde_json::Value::Null, true)
                    });
                    None
                }
                Event::Msg(msg) => {
                    turn.text.clear();
                    turn.thinking.clear();
                    turn.tools.clear();
                    if msg.role == Role::Assistant {
                        calls.extend(msg.tool_uses().map(|(id, name, input)| {
                            (id.to_owned(), name.to_owned(), input.clone())
                        }));
                    }
                    chat.push(LogLine::Msg { msg })
                }
                Event::Proposal(p) => {
                    proposals.push(p.id.clone());
                    chat.push(LogLine::Proposal { proposal: p })
                }
                Event::Committed { id, version } => {
                    committed.push((id, version));
                    None
                }
                Event::Image {
                    tool_use_id,
                    mime,
                    data,
                } => save_image(chat, &tool_use_id, &mime, &data),
                Event::Ask {
                    id,
                    question,
                    options,
                    held,
                } => {
                    chat.ask = Some(Ask {
                        id,
                        question,
                        options,
                        held,
                    });
                    None
                }
                Event::Notice { text, error } => chat.push(LogLine::Notice { text, error }),
                Event::Done {
                    secs,
                    model,
                    usage,
                    cost,
                } => {
                    done = true;
                    chat.push(LogLine::Turn {
                        secs,
                        model,
                        input_tokens: usage.input,
                        output_tokens: usage.output,
                        cached_tokens: usage.cached,
                        cost_usd: cost,
                    })
                }
            };
        if err.is_some() {
            state.error = err;
        }
        if done {
            break;
        }
    }
    for (id, name, input) in calls {
        intercept(ctx, state, &id, &name, &input);
    }
    for (id, version) in committed {
        let ops = state
            .chat
            .as_ref()
            .and_then(|c| proposal_in(&c.log, &id))
            .map(|p| (0..p.ops.len()).collect())
            .unwrap_or_default();
        set_status(
            state,
            &id,
            Status::Applied {
                version,
                ops,
                ts: model::now(),
            },
        );
        proposals.retain(|p| *p != id);
    }
    if done {
        let claude = state.turn.as_ref().is_some_and(|t| t.claude);
        state.turn = None;
        set_lock(state, false);
        if claude {
            claude_turn_over(state);
        }
        reload_after_edits(ctx, state);
    }
    if auto_apply(ctx) {
        for id in proposals {
            let clean =
                card_for(ctx, state, &id).is_some_and(|c| match (&c.result, &c.before, &c.after) {
                    (Ok(_), Some(b), Some(a)) => ops::compare(b, a).0.is_empty(),
                    (Ok(_) | Err(_), _, _) => false,
                });
            if clean {
                apply(ctx, state, &id);
            }
        }
    }
}

/// After a Claude Code turn: keep the session id for a restart, and fall
/// back when the CLI is not signed in or died.
fn claude_turn_over(state: &mut State) {
    let Some(session) = state.session.as_ref() else {
        return;
    };
    if session.auth_failed() {
        state.claude_down =
            Some("Claude Code is not signed in: run `claude` once and log in.".to_owned());
        state.session = None;
        return;
    }
    let (work, plan) = session.key.clone();
    let file = claude_code::session_file(&work, &plan);
    if !session.alive() {
        // A session that cannot be resumed is not tried again.
        let _ = std::fs::remove_file(&file);
        state.session = None;
        return;
    }
    if let Some(id) = session.session_id()
        && std::fs::read_to_string(&file)
            .ok()
            .as_deref()
            .map(str::trim)
            != Some(id.as_str())
        && let Err(e) = std::fs::write(&file, &id)
    {
        state.error = Some(format!("could not save the Claude Code session: {e}"));
    }
}

/// Plans may have changed under the agent: re-read them.
fn reload_after_edits(ctx: &mut PageCtx<'_>, state: &mut State) {
    if let Some(chat) = state.chat.as_mut() {
        chat.cached = None;
    }
    if let Some(project) = ctx.project.as_mut() {
        project.reload_plans();
    }
}

/// Saves a tool's image beside the chat log and records it.
fn save_image(chat: &mut Chat, tool_use_id: &str, mime: &str, data: &str) -> Option<String> {
    use base64::Engine as _;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(data) {
        Ok(b) => b,
        Err(e) => return Some(format!("a tool image could not be read: {e}")),
    };
    let ext = if mime.contains("png") { "png" } else { "jpg" };
    let n = chat
        .log
        .iter()
        .filter(|l| matches!(l, LogLine::Image { .. }))
        .count();
    let safe: String = tool_use_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let file = format!("img/{}-{safe}-{n}.{ext}", chat.plan);
    let path = chat.work.join("chat").join(&file);
    if let Some(dir) = path.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        return Some(format!("could not save a tool image: {e}"));
    }
    if let Err(e) = std::fs::write(&path, bytes) {
        return Some(format!("could not save a tool image: {e}"));
    }
    chat.push(LogLine::Image {
        tool_use_id: tool_use_id.to_owned(),
        file,
    })
}

/// Proposal `id` in a log.
fn proposal_in(log: &[LogLine], id: &str) -> Option<Proposal> {
    log.iter().find_map(|l| match l {
        LogLine::Proposal { proposal } if proposal.id == id => Some((**proposal).clone()),
        LogLine::Msg { .. }
        | LogLine::Proposal { .. }
        | LogLine::Status { .. }
        | LogLine::Turn { .. }
        | LogLine::Notice { .. }
        | LogLine::Image { .. } => None,
    })
}

// Whether this night applies clean proposals without asking.
fn auto_apply(ctx: &PageCtx<'_>) -> bool {
    ctx.project
        .as_ref()
        .and_then(|p| p.settings.pages.get(PAGE_KEY))
        .and_then(|t| t.get("auto_apply"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
}

/// Whether this night lets the agent save edits itself.
fn allow_commit(ctx: &PageCtx<'_>) -> bool {
    ctx.project
        .as_ref()
        .is_some_and(crate::mcp::commits_allowed)
}

/// Saves one per-night option.
fn set_page_option(ctx: &mut PageCtx<'_>, key: &str, value: toml::Value) -> Option<String> {
    let project = ctx.project.as_mut()?;
    project
        .settings
        .pages
        .entry(PAGE_KEY.to_owned())
        .or_default()
        .insert(key.to_owned(), value);
    project
        .save_settings()
        .err()
        .map(|e| format!("could not save ave.toml: {e:#}"))
}

/// The panel's top row: plan, tabs, new chat, settings.
fn header(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let stems: Vec<String> = ctx
        .project
        .as_ref()
        .map(|p| {
            p.plans
                .iter()
                .filter_map(|x| x.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    let theme = ctx.theme;
    let mut switch_to = None;
    let mut new_chat = false;
    let current = state
        .chat
        .as_ref()
        .map(|c| c.plan.clone())
        .unwrap_or_default();
    let idle = state.turn.is_none();
    let can_reset = idle && state.chat.as_ref().is_some_and(|c| !c.log.is_empty());
    let tab = state.tab;
    widgets::panel_bar(
        ui,
        theme,
        |ui| {
            // Selects' Chats button: the conversation list. Here a
            // conversation belongs to a plan, so the list is the plans.
            let id = ui.make_persistent_id("assistant-chats");
            let open = egui::Popup::is_id_open(ui.ctx(), id);
            let button = widgets::icon_toggle_sized(
                ui,
                theme,
                icons::MENU,
                "Chats: the plan this conversation is about",
                "",
                open,
                32.0,
            );
            let _ = egui::Popup::menu(&button)
                .id(id)
                .width(280.0)
                .gap(SPACE_XS)
                .show(|ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(SPACE_SM);
                        widgets::menu_heading(ui, theme, "Plans");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let r = ui
                                .add_enabled_ui(can_reset, |ui| {
                                    widgets::button_with(
                                        ui,
                                        theme,
                                        widgets::ButtonKind::Ghost,
                                        Some(icons::PLUS),
                                        "New chat",
                                    )
                                })
                                .inner;
                            if widgets::tip(
                                r,
                                theme,
                                "Start a fresh conversation (the old one is kept beside it)",
                                "",
                            )
                            .clicked()
                            {
                                new_chat = true;
                                ui.close();
                            }
                        });
                    });
                    widgets::menu_separator(ui, theme);
                    ui.add_enabled_ui(idle, |ui| {
                        for s in &stems {
                            if widgets::menu_item(ui, theme, None, s, "", *s == current).clicked() {
                                if *s != current {
                                    switch_to = Some(s.clone());
                                }
                                ui.close();
                            }
                        }
                    });
                });
            let title = match tab {
                Tab::Chat => current.clone(),
                Tab::History => format!("History of {current}"),
            };
            let mut job = egui::text::LayoutJob::simple_singleline(
                title,
                theme::font(theme::BODY_SIZE),
                theme::rgb(theme.text_muted),
            );
            job.wrap.max_width = (ui.available_width() - 32.0 * 2.0 - SPACE_SM * 2.0).max(0.0);
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            ui.add_space(SPACE_XS);
            ui.add(egui::Label::new(job).selectable(false))
                .on_hover_text("The plan this conversation is about");
        },
        |ui| {
            if widgets::icon_toggle_sized(
                ui,
                theme,
                icons::SETTINGS_2,
                "Assistant settings: model key, local server, autonomy",
                "",
                state.settings_open,
                32.0,
            )
            .clicked()
            {
                state.settings_open = !state.settings_open;
            }
            if widgets::icon_toggle_sized(
                ui,
                theme,
                icons::HISTORY,
                "History: applied proposals and plan versions",
                "",
                state.tab == Tab::History,
                32.0,
            )
            .clicked()
            {
                state.tab = match state.tab {
                    Tab::Chat => Tab::History,
                    Tab::History => Tab::Chat,
                };
            }
        },
    );
    if new_chat && let Some(chat) = state.chat.as_mut() {
        let old = chat.log_path();
        let kept = old.with_file_name(format!("{}.{}.jsonl", chat.plan, model::now()));
        // A new conversation is a new Claude Code session too.
        state.session = None;
        let _ = std::fs::remove_file(claude_code::session_file(&chat.work, &chat.plan));
        if let Err(e) = std::fs::rename(&old, &kept) {
            state.error = Some(format!("could not archive the conversation: {e}"));
        } else {
            *chat = open_chat(&chat.work.clone(), &chat.plan.clone());
            reopened(state);
            state.tab = Tab::Chat;
        }
    }
    if let Some(plan) = switch_to
        && let Some(project) = ctx.project.as_ref()
    {
        state.chat = Some(open_chat(&project.work, &plan));
        reopened(state);
        state.error = set_page_option(ctx, "plan", toml::Value::String(plan));
    }
}

/// The settings strip: key, local server, autonomy.
fn settings(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    egui::Frame::new()
        .stroke(theme::ink_line(theme))
        .fill(theme::ink(theme, theme::INK_03))
        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(theme::margin(SPACE_MD))
        .outer_margin(Margin {
            left: 0,
            right: 0,
            top: theme::as_margin(SPACE_SM),
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = SPACE_SM;
            let Some(prefs) = state.prefs.as_mut() else {
                return;
            };
            let env_key = std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.trim().is_empty());
            widgets::menu_heading(ui, theme, "Claude API key");
            if env_key {
                widgets::muted(ui, theme, "Using ANTHROPIC_API_KEY from the environment.");
            } else {
                let at = store::prefs_path().map(|p| p.display().to_string()).unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.key_draft)
                            .password(true)
                            .hint_text(if prefs.api_key.is_some() { "saved — paste to replace" } else { "sk-ant-…" })
                            .desired_width(ui.available_width() - 64.0),
                    )
                    .accessible_name("Claude API key")
                    .on_hover_text(format!("Stored in plain text at {at}. Or set ANTHROPIC_API_KEY. Keys: console.anthropic.com."));
                    if widgets::outline_button(ui, theme, "Save").clicked() {
                        let key = state.key_draft.trim().to_owned();
                        prefs.api_key = (!key.is_empty()).then_some(key);
                        state.key_draft.clear();
                        state.error = prefs.save().err().map(|e| format!("{e:#}"));
                    }
                });
            }
            widgets::menu_heading(ui, theme, "Local model server");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut state.url_draft).desired_width(ui.available_width() - 64.0))
                    .accessible_name("Local model server")
                    .on_hover_text("llama.cpp's server (AVE_LLM_URL). Small local models plan poorly; use it offline.");
                if widgets::outline_button(ui, theme, "Save").clicked() {
                    prefs.local_url = Some(state.url_draft.trim().to_owned());
                    state.error = prefs.save().err().map(|e| format!("{e:#}"));
                }
            });
            let mut auto = auto_apply(ctx);
            if widgets::switch(ui, theme, &mut auto, "Apply proposals that keep the check clean")
                .on_hover_text("Off: every proposal waits for you (suggest only). On: proposals that add no check problems are applied at once, still as a revertible version.")
                .changed()
            {
                state.error = set_page_option(ctx, "auto_apply", toml::Value::Boolean(auto));
            }
        });
    ui.add_space(SPACE_XS);
}

/// The model the person picked: Claude Code when it is installed and
/// nothing else was chosen.
fn model_id(state: &State) -> String {
    state
        .prefs
        .as_ref()
        .and_then(|p| p.model.clone())
        .unwrap_or_else(|| {
            if matches!(state.claude, Some(Some(_))) {
                CLAUDE_CODE.to_owned()
            } else {
                MODELS[1].0.to_owned()
            }
        })
}

/// What will answer the next message.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Runtime {
    /// The Claude Code CLI.
    ClaudeCode(PathBuf),
    /// The Messages API with this model.
    Api(String),
    /// The local llama.cpp server.
    Local,
}

/// The runtime for the picked model, falling back from Claude Code (not
/// installed, or not signed in) to the API when there is a key, else local.
fn runtime(state: &State) -> Runtime {
    let model = model_id(state);
    if model == CLAUDE_CODE {
        if let (Some(Some(exe)), None) = (&state.claude, &state.claude_down) {
            return Runtime::ClaudeCode(exe.clone());
        }
        return if state.prefs.as_ref().and_then(store::Prefs::key).is_some() {
            Runtime::Api(MODELS[1].0.to_owned())
        } else {
            Runtime::Local
        };
    }
    if model == "local" {
        Runtime::Local
    } else {
        Runtime::Api(model)
    }
}

/// Why sending is impossible right now, if it is.
fn blocked_reason(state: &State) -> Option<String> {
    match runtime(state) {
        Runtime::Api(_) if state.prefs.as_ref().and_then(store::Prefs::key).is_none() => Some(
            "No Claude API key. Set ANTHROPIC_API_KEY, paste a key in the assistant settings, or pick Claude Code or the local model."
                .to_owned(),
        ),
        Runtime::ClaudeCode(_) | Runtime::Api(_) | Runtime::Local => None,
    }
}

/// A note when the picked runtime is not the one that will answer.
fn fallback_note(state: &State) -> Option<String> {
    if model_id(state) != CLAUDE_CODE {
        return None;
    }
    let using = match runtime(state) {
        Runtime::ClaudeCode(_) => return None,
        Runtime::Api(m) => m,
        Runtime::Local => "the local model".to_owned(),
    };
    Some(match (&state.claude, &state.claude_down) {
        (_, Some(why)) => format!("{why} Using {using}."),
        (Some(None) | None, None) => format!(
            "Claude Code is not installed (no `claude` on PATH); using {using}. Install it from claude.com/code and log in once."
        ),
        (Some(Some(_)), None) => format!("Using {using}."),
    })
}

/// The scope in force: the person's pick, else the selection.
fn effective_scope(
    state: &State,
    refs: &[(Reference, complete::Origin)],
    selection: &[Reference],
) -> ScopeChoice {
    if let Some(s) = state.scope {
        return s;
    }
    if !scoped_shots(state, refs, selection).0.is_empty() {
        ScopeChoice::Selected
    } else {
        ScopeChoice::Whole
    }
}

/// Shots of this plan named by references or the selection: the ones that
/// may change, and the ones marked keep.
fn scoped_shots(
    state: &State,
    refs: &[(Reference, complete::Origin)],
    selection: &[Reference],
) -> (BTreeSet<usize>, BTreeSet<usize>) {
    let plan = state
        .chat
        .as_ref()
        .map(|c| c.plan.as_str())
        .unwrap_or_default();
    let mut change = BTreeSet::new();
    let mut keep = BTreeSet::new();
    for r in refs.iter().map(|(r, _)| r).chain(selection) {
        if let Target::Shot { plan: p, index } = &r.target
            && p == plan
        {
            match r.modifier {
                Modifier::Keep => {
                    keep.insert(*index);
                }
                Modifier::Avoid
                | Modifier::About
                | Modifier::Replace
                | Modifier::Like
                | Modifier::Hero => {
                    change.insert(*index);
                }
            }
        }
    }
    for k in &keep {
        change.remove(k);
    }
    (change, keep)
}

/// The scope as the engine checks it.
fn resolve_scope(
    choice: ScopeChoice,
    plan: &Plan,
    change: &BTreeSet<usize>,
    keep: &BTreeSet<usize>,
) -> Scope {
    let all = 1..=plan.shots.len();
    let drop_beat = plan.drop_bar.saturating_sub(plan.start_bar) * 4;
    let mut starts = Vec::new();
    let mut beat = 0;
    for s in &plan.shots {
        starts.push(beat);
        beat += s.beats;
    }
    let set: BTreeSet<usize> = match choice {
        ScopeChoice::Whole => all.collect(),
        ScopeChoice::Selected => change.clone(),
        ScopeChoice::Build => all.filter(|k| starts[k - 1] < drop_beat).collect(),
        ScopeChoice::FromDrop => all.filter(|k| starts[k - 1] >= drop_beat).collect(),
    };
    let set: BTreeSet<usize> = set.difference(keep).copied().collect();
    if set.len() == plan.shots.len() {
        Scope::Whole
    } else {
        Scope::Shots(set)
    }
}

/// The composer, as Selects': the answer card when the model asked
/// something, then one dark box holding the notices, the Chat / Comments
/// switch, the scope pills, the staged references, the input and the
/// add / model / send toolbar.
fn composer(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    let busy = state.turn.is_some();
    let mut send: Option<String> = None;
    let id = ui.make_persistent_id("assistant_input");

    if state.mode == Mode::Chat
        && let Some(a) = state.approvals.first().cloned()
        && let Some(choice) = cards::approval(ui, theme, &a)
    {
        decide(ctx, state, &a.id, choice, false);
    }
    if state.mode == Mode::Chat
        && let Some(ask) = state.chat.as_ref().and_then(|c| c.ask.clone())
        && !busy
    {
        if let Some(answer) = options_card(ctx, ui, &ask, id) {
            send = Some(answer);
        }
        ui.add_space(SPACE_SM);
    }

    let refs = complete::merge(ctx.refs, &state.input);
    let selection = ctx.selection.clone();
    let scope = effective_scope(state, &refs, &selection);
    let (change, keep) = scoped_shots(state, &refs, &selection);

    // The `@` list and Enter to send, from the cursor as it was last frame.
    let focused = ui.memory(|m| m.has_focus(id));
    let cursor = egui::text_edit::TextEditState::load(ui.ctx(), id)
        .and_then(|s| s.cursor.char_range())
        .map(|r| usize::from(r.primary.index));
    let active = cursor
        .filter(|_| focused && state.mode == Mode::Chat)
        .and_then(|c| complete::active(&state.input, c))
        .filter(|_| state.dismissed.as_deref() != Some(state.input.as_str()));
    let cands = active
        .as_ref()
        .map(|a| complete::candidates(&a.query, &sources(ctx, state), 8))
        .unwrap_or_default();
    let mut accept: Option<usize> = None;
    if focused && !cands.is_empty() {
        ui.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                state.pick = (state.pick + 1) % cands.len();
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                state.pick = (state.pick + cands.len() - 1) % cands.len();
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                || i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
            {
                accept = Some(state.pick.min(cands.len() - 1));
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                state.dismissed = Some(state.input.clone());
            }
        });
    } else if focused
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
        && !state.input.trim().is_empty()
    {
        send = Some(state.input.clone());
    }

    let blocked = blocked_reason(state);
    let border = if focused {
        theme::ring(theme)
    } else {
        theme::rgb(theme.raised)
    };
    let frame = egui::Frame::new()
        .fill(theme::rgb(theme.app))
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(theme::RADIUS_XL))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            // Notices: what failed, why sending is blocked, which runtime
            // is answering instead of the one picked.
            if let Some(e) = state.error.clone() {
                error_row(ctx, state, ui, &e);
            }
            if let Some(reason) = &blocked {
                look::notice_row(ui, theme, reason, look::Tone::Warning, |_| {});
            } else if let Some(note) = fallback_note(state) {
                let down = state.claude_down.is_some();
                let mut retry = false;
                look::notice_row(ui, theme, &note, look::Tone::Hint, |ui| {
                    if down && widgets::ghost_button(ui, theme, "Retry Claude Code").clicked() {
                        retry = true;
                    }
                });
                if retry {
                    state.claude_down = None;
                }
            }
            egui::Frame::new()
                .inner_margin(Margin {
                    left: 10,
                    right: 10,
                    top: 10,
                    bottom: 0,
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        look::mode_switch(
                            ui,
                            theme,
                            &mut state.mode,
                            &[(Mode::Chat, "Chat"), (Mode::Comments, "Comments")],
                        );
                    });
                });
            match state.mode {
                Mode::Chat => {
                    scope_row(ctx, state, ui, scope, &change, &keep);
                    staged_refs(ctx, ui, &refs);
                    input(ctx, state, ui, id);
                    toolbar(ctx, state, ui, blocked.as_deref(), &mut send);
                }
                Mode::Comments => comments_toolbar(ctx, state, ui, id),
            }
        });

    if !cands.is_empty() {
        let anchor = frame.response.rect;
        let _ = egui::Area::new(id.with("ac"))
            .order(egui::Order::Foreground)
            .fixed_pos(anchor.left_top() + Vec2::new(SPACE_MD, -SPACE_SM))
            .pivot(egui::Align2::LEFT_BOTTOM)
            .show(ui.ctx(), |ui| {
                widgets::popover_frame(theme).show(ui, |ui| {
                    ui.set_width((anchor.width() - SPACE_MD * 2.0).max(220.0));
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (k, c) in cands.iter().enumerate() {
                        let on = k == state.pick.min(cands.len() - 1);
                        let r = look::completion_row(
                            ui,
                            theme,
                            look::token_icon(&c.token),
                            &c.label,
                            &c.detail,
                            on,
                        );
                        if r.clicked() {
                            accept = Some(k);
                        }
                        if r.hovered() {
                            state.pick = k;
                        }
                    }
                });
            });
    }
    if let (Some(k), Some(a)) = (accept, active.as_ref())
        && let Some(c) = cands.get(k)
    {
        let token = format!("{}{}", a.modifier.prefix(), c.token);
        let (text, at) = complete::insert(&state.input, a.start, a.end, &token);
        state.input = text;
        let mut s = egui::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
        s.cursor.set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(at),
        )));
        s.store(ui.ctx(), id);
        ui.memory_mut(|m| m.request_focus(id));
        state.pick = 0;
    }

    if let Some(text) = send
        && !busy
    {
        if let Some(reason) = blocked {
            state.error = Some(reason);
        } else {
            submit(ctx, state, ui.ctx(), &text, scope);
        }
    }
}

/// The scope pills under the switch: what the request may change, and the
/// shots marked keep.
fn scope_row(
    ctx: &PageCtx<'_>,
    state: &mut State,
    ui: &mut Ui,
    scope: ScopeChoice,
    change: &BTreeSet<usize>,
    keep: &BTreeSet<usize>,
) {
    let theme = ctx.theme;
    egui::Frame::new()
        .inner_margin(Margin {
            left: 12,
            right: 12,
            top: 8,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(SPACE_XS);
                for choice in [
                    ScopeChoice::Whole,
                    ScopeChoice::Selected,
                    ScopeChoice::Build,
                    ScopeChoice::FromDrop,
                ] {
                    let on = scope == choice;
                    let label = match choice {
                        ScopeChoice::Selected => format!("{} ({})", choice.label(), change.len()),
                        ScopeChoice::Whole | ScopeChoice::Build | ScopeChoice::FromDrop => {
                            choice.label().to_owned()
                        }
                    };
                    let hint = if state.scope.is_none() && on {
                        "Scope follows the selection; click to pin one"
                    } else {
                        "What the assistant may change"
                    };
                    let r = widgets::tip(look::scope_pill(ui, theme, &label, on), theme, hint, "");
                    if r.clicked() {
                        state.scope = if state.scope == Some(choice) {
                            None
                        } else {
                            Some(choice)
                        };
                    }
                }
                if !keep.is_empty() {
                    ui.label(
                        RichText::new(format!(
                            "keeping {}",
                            keep.iter()
                                .map(|k| format!("#{k}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        ))
                        .size(theme::BODY_SIZE)
                        .color(theme::rgb(theme.text_muted)),
                    );
                }
            });
        });
}

/// The references staged from the app (R on a shot, the `+` menu), as
/// Selects shows mentions: lime text, no chip. Clicking one cycles its
/// intent; the small cross drops it. Tokens typed in the message are lime
/// in the input itself.
fn staged_refs(ctx: &mut PageCtx<'_>, ui: &mut Ui, refs: &[(Reference, complete::Origin)]) {
    let staged: Vec<(usize, &Reference)> = refs
        .iter()
        .filter_map(|(r, origin)| match origin {
            complete::Origin::Chip(i) => Some((*i, r)),
            complete::Origin::Typed => None,
        })
        .collect();
    if staged.is_empty() {
        return;
    }
    let theme = ctx.theme;
    let mut cycle: Option<usize> = None;
    let mut remove: Option<usize> = None;
    let mut clear = false;
    egui::Frame::new()
        .inner_margin(Margin {
            left: 16,
            right: 12,
            top: 10,
            bottom: 0,
        })
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(SPACE_XS, SPACE_XS);
                for (i, r) in &staged {
                    let facts = ctx
                        .project
                        .as_ref()
                        .map(|p| {
                            crate::refs::resolve(r, p)
                                .map_or_else(|e| format!("unresolved: {e}"), |x| x.facts)
                        })
                        .unwrap_or_default();
                    let token = format!("{}@{}", r.modifier.prefix(), r.short());
                    let mention = ui
                        .add(
                            egui::Label::new(
                                RichText::new(token)
                                    .size(theme::LARGE_SIZE)
                                    .color(theme::rgb(theme.accent)),
                            )
                            .sense(Sense::click()),
                        )
                        .on_hover_text(format!(
                            "{r}\n{} · {facts}\nclick to change intent",
                            r.modifier.label()
                        ));
                    if mention.clicked() {
                        cycle = Some(*i);
                    }
                    if small_x(ui, theme, "Remove reference").clicked() {
                        remove = Some(*i);
                    }
                    ui.add_space(SPACE_XS);
                }
                if staged.len() > 1
                    && ui
                        .add(
                            egui::Label::new(
                                RichText::new("Clear all")
                                    .size(theme::BODY_SIZE)
                                    .color(theme::rgb(theme.text_muted)),
                            )
                            .sense(Sense::click()),
                        )
                        .clicked()
                {
                    clear = true;
                }
            });
        });
    if clear {
        ctx.actions.push(Action::ClearRefs);
    }
    if let Some(i) = cycle
        && let Some(r) = ctx.refs.get_mut(i)
    {
        let at = Modifier::ALL
            .iter()
            .position(|m| *m == r.modifier)
            .unwrap_or(0);
        r.modifier = Modifier::ALL[(at + 1) % Modifier::ALL.len()];
    }
    if let Some(i) = remove
        && i < ctx.refs.len()
    {
        ctx.refs.remove(i);
    }
}

/// A 14 px cross that removes something: muted, the text colour under the
/// pointer.
fn small_x(ui: &mut Ui, theme: &crate::theme::Palette, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(icons::SMALL), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let ink = if response.hovered() {
        theme::rgb(theme.text)
    } else {
        theme::rgb(theme.text_muted)
    };
    icons::paint(ui.painter(), rect, icons::X, 12.0, ink);
    widgets::tip(response, theme, label, "")
}

/// The message input: 14 px, four rows, frameless inside the box, typed
/// references painted lime. Enter sends, Shift+Enter breaks the line.
fn input(ctx: &PageCtx<'_>, state: &mut State, ui: &mut Ui, id: egui::Id) {
    let theme = ctx.theme;
    let hint = if state.chat.as_ref().is_some_and(|c| c.ask.is_some()) {
        "Answer, or say what you mean…"
    } else {
        "Ask for an edit or about the footage…"
    };
    let intents: Vec<String> = Modifier::ALL
        .iter()
        .filter(|m| !m.prefix().is_empty())
        .map(|m| format!("{} {}", m.prefix(), m.label()))
        .collect();
    let tooltip = format!(
        "Type @ to reference a shot, person, bars or plan; before the @, {} \
         set the intent. Shift+Enter for a new line.",
        intents.join(", ")
    );
    let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let job = look::mention_job(
            theme,
            buf.as_str(),
            theme::LARGE_SIZE,
            theme::rgb(theme.text),
            wrap,
        );
        ui.fonts_mut(|f| f.layout_job(job))
    };
    egui::Frame::new()
        .inner_margin(Margin {
            left: 16,
            right: 16,
            top: 12,
            bottom: 6,
        })
        .show(ui, |ui| {
            let output = egui::TextEdit::multiline(&mut state.input)
                .id(id)
                .frame(egui::Frame::NONE)
                .margin(Margin::ZERO)
                .hint_text(
                    RichText::new(hint)
                        .size(theme::LARGE_SIZE)
                        .color(theme::rgb(theme.text_muted)),
                )
                .desired_rows(4)
                .desired_width(f32::INFINITY)
                .layouter(&mut layouter)
                .return_key(egui::KeyboardShortcut::new(
                    egui::Modifiers::SHIFT,
                    egui::Key::Enter,
                ))
                .show(ui);
            if output.response.changed() {
                state.pick = 0;
            }
            let _ = widgets::tip(output.response.response, theme, &tooltip, "Enter");
        });
}

/// The bottom row of the box: the add menu on the left; the model picker
/// and the send (or stop) button on the right, all 32 px.
fn toolbar(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut Ui,
    blocked: Option<&str>,
    send: &mut Option<String>,
) {
    let theme = ctx.theme;
    egui::Frame::new()
        .inner_margin(theme::margin(10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                add_menu(ctx, state, ui);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if let Some(turn) = &state.turn {
                        if widgets::filled_icon_button(
                            ui,
                            theme,
                            widgets::ButtonKind::Secondary,
                            icons::SQUARE,
                            "Stop",
                            "",
                            32.0,
                        )
                        .clicked()
                        {
                            turn.cancel.store(true, Ordering::Relaxed);
                        }
                        ui.add_space(SPACE_XS);
                        ui.label(
                            RichText::new(format!("{:.0}s", turn.started.elapsed().as_secs_f32()))
                                .font(theme::tabular(ui.ctx(), theme::TINY_SIZE))
                                .color(theme::rgb(theme.text_faint)),
                        );
                    } else {
                        let can = blocked.is_none() && !state.input.trim().is_empty();
                        let r = ui
                            .add_enabled_ui(can, |ui| {
                                widgets::filled_icon_button(
                                    ui,
                                    theme,
                                    widgets::ButtonKind::Secondary,
                                    icons::ARROW_UP,
                                    "Send",
                                    "Enter",
                                    32.0,
                                )
                            })
                            .inner;
                        if r.clicked() {
                            *send = Some(state.input.clone());
                        }
                    }
                    ui.add_space(SPACE_XS);
                    model_menu(ctx, state, ui);
                });
            });
        });
}

/// The model picker: a ghost trigger naming the model, opening the models
/// and the permission choice (Selects puts Permissions in this menu).
fn model_menu(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    let current = model_id(state);
    let label = MODELS
        .iter()
        .find(|(m, _)| *m == current)
        .map_or(current.as_str(), |(_, l)| l)
        .to_owned();
    let id = ui.make_persistent_id("assistant-model");
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let trigger = widgets::tip(
        look::picker(ui, theme, &label, "Model", open),
        theme,
        "Model and permissions",
        "",
    );
    let allow = allow_commit(ctx);
    let mut pick: Option<&str> = None;
    let mut permission: Option<bool> = None;
    let _ = egui::Popup::menu(&trigger)
        .id(id)
        .width(240.0)
        .gap(SPACE_XS)
        .show(|ui| {
            ui.horizontal(|ui| {
                ui.add_space(SPACE_SM);
                widgets::menu_heading(ui, theme, "Model");
            });
            for (m, l) in MODELS {
                if widgets::menu_item(ui, theme, None, l, "", current == m).clicked() {
                    pick = Some(m);
                    ui.close();
                }
            }
            widgets::menu_separator(ui, theme);
            ui.horizontal(|ui| {
                ui.add_space(SPACE_SM);
                widgets::menu_heading(ui, theme, "Permissions");
            });
            for (value, name, hint) in [
                (
                    false,
                    "Ask",
                    "The assistant stages every edit as a card; you apply it.",
                ),
                (
                    true,
                    "Let it apply edits",
                    "The assistant may save edits itself; each one is a card you can revert.",
                ),
            ] {
                if widgets::menu_item(ui, theme, None, name, "", allow == value)
                    .on_hover_text(hint)
                    .clicked()
                {
                    permission = Some(value);
                    ui.close();
                }
            }
        });
    if let Some(m) = pick
        && let Some(prefs) = state.prefs.as_mut()
    {
        prefs.model = Some(m.to_owned());
        state.error = prefs.save().err().map(|e| format!("{e:#}"));
    }
    if let Some(value) = permission
        && value != allow
    {
        state.error = set_page_option(ctx, crate::mcp::ALLOW_COMMIT, toml::Value::Boolean(value));
    }
}

/// The question the model asked, as Selects' options card above the
/// composer: the question, then one bordered list of numbered answers and
/// a last row for saying something else. Returns the answer picked.
fn options_card(ctx: &PageCtx<'_>, ui: &mut Ui, ask: &Ask, input: egui::Id) -> Option<String> {
    let theme = ctx.theme;
    let mut chosen = None;
    egui::Frame::new()
        .fill(theme::rgb(theme.panel))
        .stroke(Stroke::new(1.0, theme::accent_fill(theme)))
        .corner_radius(CornerRadius::same(theme::RADIUS_XL))
        .inner_margin(theme::margin_xy(SPACE_MD, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    RichText::new(&ask.question)
                        .size(theme::LARGE_SIZE)
                        .color(look::tone_200(theme)),
                )
                .wrap(),
            );
            ui.add_space(SPACE_SM);
            egui::Frame::new()
                .stroke(Stroke::new(1.0, theme::rgb(theme.raised)))
                .corner_radius(CornerRadius::same(theme::RADIUS_MD))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (k, o) in ask.options.iter().enumerate() {
                        if option_row(ui, theme, Some(k + 1), o).clicked() {
                            chosen = Some(o.clone());
                        }
                    }
                    if option_row(ui, theme, None, "Something else: type it below").clicked() {
                        ui.memory_mut(|m| m.request_focus(input));
                    }
                });
        });
    chosen
}

/// One answer in the options card: a 20 px number circle, the answer in
/// 14 px, a chevron on hover; a hairline above every row but the first.
fn option_row(
    ui: &mut Ui,
    theme: &crate::theme::Palette,
    number: Option<usize>,
    label: &str,
) -> egui::Response {
    let width = ui.available_width();
    let text_x = SPACE_MD - 2.0 + 20.0 + SPACE_SM;
    let ink = if number.is_some() {
        theme::rgb(theme.text)
    } else {
        theme::rgb(theme.text_muted)
    };
    let galley = ui.painter().layout(
        label.to_owned(),
        theme::font(theme::LARGE_SIZE),
        ink,
        (width - text_x - theme::SPACE_XL).max(40.0),
    );
    let height = (galley.size().y + SPACE_LG).max(36.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let first = number == Some(1);
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    let painter = ui.painter();
    if !first {
        painter.hline(
            rect.x_range(),
            rect.top() + 0.5,
            Stroke::new(1.0, theme::rgb(theme.raised)),
        );
    }
    if lift > 0.0 {
        painter.rect_filled(
            rect.shrink(1.0),
            CornerRadius::same(theme::RADIUS),
            theme::rgb(theme.raised).gamma_multiply(0.6 * lift),
        );
    }
    let circle = Rect::from_center_size(
        egui::pos2(rect.left() + SPACE_MD - 2.0 + 10.0, rect.top() + 18.0),
        Vec2::splat(20.0),
    );
    match number {
        Some(n) => {
            painter.circle_filled(circle.center(), 10.0, theme::accent_fill(theme));
            look::centred(
                ui,
                circle,
                &n.to_string(),
                theme::SMALL_SIZE,
                theme::rgb(theme.text_muted),
            );
        }
        None => {
            icons::paint(
                painter,
                circle,
                icons::PEN,
                icons::SMALL,
                theme::rgb(theme.text_muted),
            );
        }
    }
    ui.painter().galley(
        egui::pos2(rect.left() + text_x, rect.top() + 18.0 - 10.0),
        galley,
        ink,
    );
    if lift > 0.0 {
        let slot = Rect::from_center_size(
            egui::pos2(rect.right() - SPACE_MD, rect.top() + 18.0),
            Vec2::splat(icons::SMALL),
        );
        icons::paint(
            ui.painter(),
            slot,
            icons::CHEVRON_RIGHT,
            icons::SMALL,
            theme::rgb(theme.text_muted).gamma_multiply(lift),
        );
    }
    response
}

/// The composer in Comments mode: one action, putting the open comments
/// in a message for the assistant.
fn comments_toolbar(ctx: &PageCtx<'_>, state: &mut State, ui: &mut Ui, input: egui::Id) {
    let theme = ctx.theme;
    let message = comments_message(ctx, state);
    egui::Frame::new()
        .inner_margin(theme::margin(10.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let can = message.is_some() && state.turn.is_none();
                let r = ui
                    .add_enabled_ui(can, |ui| {
                        widgets::button_sized(
                            ui,
                            theme,
                            widgets::ButtonKind::Ghost,
                            Some(icons::MESSAGE_SQUARE),
                            "Ask about open comments",
                            32.0,
                        )
                    })
                    .inner;
                let r = widgets::tip(
                    r,
                    theme,
                    "Write every open comment into a message, with its time and shot, to send from Chat",
                    "",
                );
                if r.clicked()
                    && let Some(text) = message
                {
                    if !state.input.trim().is_empty() {
                        state.input.push_str("\n\n");
                    }
                    state.input.push_str(&text);
                    state.mode = Mode::Chat;
                    ui.memory_mut(|m| m.request_focus(input));
                }
            });
        });
}

/// The open comments of the conversation's plan as one message: each with
/// its time and the shot under it as a reference. `None` when there are
/// none.
fn comments_message(ctx: &PageCtx<'_>, state: &mut State) -> Option<String> {
    let chat = state.chat.as_mut()?;
    let name = chat.plan.clone();
    let plan = chat.current()?;
    let timing = ctx
        .project
        .as_ref()
        .and_then(|p| p.music.as_ref())
        .map(|m| plan.timing(m))
        .unwrap_or_default();
    let mut open: Vec<&crate::plan::Comment> =
        plan.comments.iter().filter(|c| !c.resolved).collect();
    if open.is_empty() {
        return None;
    }
    open.sort_by(|a, b| a.time.total_cmp(&b.time));
    let lines: Vec<String> = open
        .iter()
        .map(|c| {
            let shot = timing
                .iter()
                .position(|&(at, dur)| c.time >= at && c.time < at + dur)
                .map(|k| format!(" @shot:{name}#{}", k + 1))
                .unwrap_or_default();
            format!("- {}{shot}: {}", clock(c.time), c.note.trim())
        })
        .collect();
    Some(format!(
        "Address these review comments on {name} as one proposal I can apply:\n{}",
        lines.join("\n")
    ))
}

/// `m:ss`.
fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The list in Comments mode: the plan's review comments in time order;
/// clicking one plays the plan from there.
fn comments(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    let Some(chat) = state.chat.as_mut() else {
        return;
    };
    let Some(plan) = chat.current() else {
        return;
    };
    let mut order: Vec<&crate::plan::Comment> = plan.comments.iter().collect();
    order.sort_by(|a, b| a.time.total_cmp(&b.time));
    if order.is_empty() {
        widgets::empty_state(
            ui,
            theme,
            icons::MESSAGE_SQUARE_TEXT,
            "No comments",
            "Press F on the timeline to leave one at the playhead.",
            |_| {},
        );
        return;
    }
    let mut seek: Option<f32> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add_space(SPACE_SM);
            for (k, c) in order.iter().enumerate() {
                let fade = if c.resolved { 0.5 } else { 1.0 };
                let row = egui::Frame::new()
                    .inner_margin(theme::margin_xy(SPACE_SM, SPACE_SM - 2.0))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = SPACE_SM;
                            ui.label(
                                RichText::new(clock(c.time))
                                    .font(theme::tabular(ui.ctx(), theme::SMALL_SIZE))
                                    .color(theme::rgb(theme.text_muted).gamma_multiply(fade)),
                            );
                            ui.label(
                                RichText::new(&c.author)
                                    .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                                    .color(theme::rgb(theme.text_muted).gamma_multiply(fade)),
                            );
                            if c.resolved {
                                icons::icon(
                                    ui,
                                    icons::CIRCLE_CHECK,
                                    12.0,
                                    theme::rgb(theme.text_muted),
                                );
                            }
                        });
                        ui.add(
                            egui::Label::new(
                                RichText::new(if c.note.trim().is_empty() {
                                    "(no text)"
                                } else {
                                    c.note.trim()
                                })
                                .size(theme::LARGE_SIZE)
                                .color(look::tone_200(theme).gamma_multiply(fade)),
                            )
                            .wrap(),
                        );
                    });
                let rect = row.response.rect;
                let response =
                    ui.interact(rect, ui.scope_id().with(("comment", k)), Sense::click());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, true, c.note.as_str())
                });
                if response.hovered() {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(theme::RADIUS),
                        theme::ink(theme, theme::INK_05),
                    );
                }
                if response.on_hover_text("Play the plan from here").clicked() {
                    seek = Some(c.time);
                }
            }
        });
    if let Some(t) = seek
        && load_sequence(ctx, &plan)
    {
        ctx.player.seek(f64::from(t));
    }
}

/// The `+` menu: add the selection, the plan, a shot, a person, some bars.
fn add_menu(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    let src = sources(ctx, state);
    let selection = ctx.selection.clone();
    let mut add: Vec<Reference> = Vec::new();
    let id = ui.make_persistent_id("assistant-add");
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let button = widgets::icon_toggle_sized(
        ui,
        theme,
        icons::PLUS,
        "Add a reference: the selection, a shot, a person, bars or a plan (or type @)",
        "",
        open,
        32.0,
    );
    let _ = egui::Popup::menu(&button)
        .id(id)
        .width(240.0)
        .gap(SPACE_XS)
        .show(|ui| {
            let r = ui
                .add_enabled_ui(!selection.is_empty(), |ui| {
                    widgets::menu_item(
                        ui,
                        theme,
                        Some(icons::POINTER),
                        &format!("Selection ({})", selection.len()),
                        "",
                        false,
                    )
                })
                .inner;
            if r.clicked() {
                add.extend(selection.iter().cloned());
                ui.close();
            }
            if let Some(chat) = &state.chat
                && widgets::menu_item(
                    ui,
                    theme,
                    Some(icons::LAYERS),
                    &format!("This plan ({})", chat.plan),
                    "",
                    false,
                )
                .clicked()
            {
                add.push(Reference::new(Target::Plan {
                    name: chat.plan.clone(),
                }));
                ui.close();
            }
            widgets::menu_separator(ui, theme);
            ui.menu_button(icons::with_label(icons::FILM, "Shot"), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for (plan, slot, desc) in &src.shots {
                            if ui.button(format!("#{slot}  {desc}")).clicked() {
                                add.push(Reference::new(Target::Shot {
                                    plan: plan.clone(),
                                    index: *slot,
                                }));
                                ui.close();
                            }
                        }
                    });
            });
            ui.menu_button(icons::with_label(icons::USER, "Person"), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for name in &src.people {
                            if ui.button(name).clicked() {
                                add.push(Reference::new(Target::Person { name: name.clone() }));
                                ui.close();
                            }
                        }
                    });
            });
            ui.menu_button(icons::with_label(icons::MUSIC, "Bars"), |ui| {
                for (from, to, label) in &src.bars {
                    if ui.button(format!("{label}  (bars {from}–{to})")).clicked() {
                        add.push(Reference::new(Target::Bars {
                            from: *from,
                            to: *to,
                        }));
                        ui.close();
                    }
                }
            });
            ui.menu_button(icons::with_label(icons::LAYERS, "Plan"), |ui| {
                for name in &src.plans {
                    if ui.button(name).clicked() {
                        add.push(Reference::new(Target::Plan { name: name.clone() }));
                        ui.close();
                    }
                }
            });
        });
    for r in add {
        ctx.actions.push(Action::AddRef(r));
    }
}

/// What the autocomplete and the `+` menu can offer.
fn sources(ctx: &PageCtx<'_>, state: &mut State) -> complete::Sources {
    let mut s = complete::Sources::default();
    let Some(project) = ctx.project.as_ref() else {
        return s;
    };
    s.plans = project
        .plans
        .iter()
        .filter_map(|p| p.file_stem().map(|x| x.to_string_lossy().into_owned()))
        .collect();
    let mut people: Vec<String> = project.people.names.values().cloned().collect();
    people.sort();
    people.dedup();
    s.people = people;
    if let Some(chat) = state.chat.as_mut()
        && let Some(plan) = chat.current()
    {
        s.shots = plan
            .shots
            .iter()
            .enumerate()
            .map(|(k, sh)| {
                (
                    chat.plan.clone(),
                    k + 1,
                    format!(
                        "{} {}{}",
                        sh.clip,
                        sh.who,
                        if sh.note.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", sh.note)
                        }
                    ),
                )
            })
            .collect();
        let last = plan.end_bar.saturating_sub(1);
        if plan.drop_bar > plan.start_bar {
            s.bars
                .push((plan.start_bar, plan.drop_bar - 1, "the build".to_owned()));
        }
        s.bars.push((
            plan.drop_bar,
            (plan.drop_bar + 3).min(last),
            "the drop".to_owned(),
        ));
        if plan.card_bar > plan.drop_bar {
            s.bars
                .push((plan.drop_bar, plan.card_bar - 1, "from the drop".to_owned()));
        }
        if plan.end_bar > plan.card_bar {
            s.bars.push((plan.card_bar, last, "end card".to_owned()));
        }
    }
    if let Some(index) = &project.index {
        s.segments = index
            .segments
            .iter()
            .map(|g| {
                (
                    g.id.clone(),
                    format!(
                        "{} q{:.2} {}",
                        g.subject,
                        g.q,
                        g.caption
                            .as_ref()
                            .map(|c| c.summary.as_str())
                            .unwrap_or_default()
                    ),
                )
            })
            .collect();
    }
    s
}

/// Sends a message and starts the turn: on Claude Code, or on a worker
/// thread running the in-process loop.
fn submit(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    egui_ctx: &egui::Context,
    text: &str,
    choice: ScopeChoice,
) {
    let runtime = runtime(state);
    let playhead = playhead(ctx);
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    let Some(prefs) = state.prefs.clone() else {
        return;
    };
    let refs = complete::merge(ctx.refs, text);
    let selection = ctx.selection.clone();
    let (change, keep) = scoped_shots(state, &refs, &selection);
    let Some(chat) = state.chat.as_mut() else {
        return;
    };
    let Some(plan) = chat.current() else {
        state.error = Some(format!("could not read {}", chat.plan_path().display()));
        return;
    };
    let scope = resolve_scope(choice, &plan, &change, &keep);
    if matches!(&scope, Scope::Shots(s) if s.is_empty()) {
        state.error =
            Some("The scope has no shots in it: select or reference some, or widen it.".to_owned());
        return;
    }

    // The reminder sent with the message.
    let index = project.index.as_ref();
    let music = project.music.as_ref();
    let problems = ops::check(&plan, index, music, &project.people);
    let plan_text = context::plan_text(&plan, music);
    let fresh = chat.sent_plan.as_deref() != Some(plan_text.as_str());
    let skill_dir = skills_dir();
    let snapshot = context::Snapshot {
        plan: fresh.then(|| plan_text.clone()),
        music: if fresh {
            music.map(|m| context::music_text(&plan, m))
        } else {
            None
        },
        check: context::problems_text(problems.as_deref()),
        scope: scope.describe(),
        refs: refs
            .iter()
            .map(|(r, _)| {
                let facts = crate::refs::resolve(r, project)
                    .map_or_else(|e| format!("(unresolved: {e})"), |x| x.facts);
                (r.to_string(), format!("{} · {facts}", r.modifier.label()))
            })
            .collect(),
        selection: selection.iter().map(ToString::to_string).collect(),
        playhead,
        skills: skills::matching(text)
            .into_iter()
            .map(|s| {
                let file = skill_dir
                    .as_ref()
                    .map(|d| skills::file(d, &s.name).display().to_string())
                    .unwrap_or_default();
                (s.name, file)
            })
            .collect(),
    };
    chat.sent_plan = Some(plan_text);
    let reminder = context::snapshot(&snapshot);

    // The message: answers to open calls first, then context, then text.
    let mut blocks = Vec::new();
    let msgs = model::messages(&chat.log);
    let ask = chat.ask.take();
    for (id, _, _) in model::unanswered(&msgs) {
        let content = match &ask {
            Some(a) if a.id == id => format!("The person answered: {text}"),
            Some(_) | None => "not run: the conversation was interrupted".to_owned(),
        };
        let held = ask.as_ref().and_then(|a| {
            a.held
                .iter()
                .find(|b| matches!(b, Block::ToolResult { tool_use_id, .. } if *tool_use_id == id))
        });
        blocks.push(held.cloned().unwrap_or(Block::ToolResult {
            tool_use_id: id,
            content,
            is_error: false,
        }));
    }
    blocks.push(Block::Context {
        text: reminder.clone(),
    });
    let mut said = text.trim().to_owned();
    let typed = Reference::scan(text);
    let extra: Vec<String> = ctx
        .refs
        .iter()
        .filter(|r| !typed.iter().any(|t| t.target == r.target))
        .map(ToString::to_string)
        .collect();
    if !extra.is_empty() {
        said = format!("{said}\n\n{}", extra.join(" "));
    }
    blocks.push(Block::Text { text: said.clone() });
    let msg = Msg::new(Role::User, blocks);
    if let Some(e) = chat.push(LogLine::Msg { msg }) {
        state.error = Some(e);
    }
    let work = chat.work.clone();
    let stem = chat.plan.clone();

    let cancel = Arc::new(AtomicBool::new(false));
    let started = match runtime {
        Runtime::ClaudeCode(exe) => {
            let message = format!("{reminder}\n\n{said}");
            start_claude(state, egui_ctx, exe, &work, &stem, &message)
        }
        Runtime::Api(model) => {
            let Some(key) = prefs.key() else {
                state.error = blocked_reason(state);
                return;
            };
            let provider = Box::new(provider::Claude {
                key,
                model,
                base: std::env::var("ANTHROPIC_BASE_URL")
                    .unwrap_or_else(|_| "https://api.anthropic.com".to_owned()),
            });
            start_loop(state, egui_ctx, provider, project, plan, scope, &cancel)
        }
        Runtime::Local => {
            let provider = Box::new(provider::Local { url: prefs.local() });
            start_loop(state, egui_ctx, provider, project, plan, scope, &cancel)
        }
    };
    match started {
        Ok((rx, claude)) => {
            state.turn = Some(Turn {
                rx,
                cancel,
                started: Instant::now(),
                text: String::new(),
                thinking: String::new(),
                tools: Vec::new(),
                claude,
            });
            set_lock(state, true);
            state.input.clear();
            state.dismissed = None;
            if !ctx.refs.is_empty() {
                ctx.actions.push(Action::ClearRefs);
            }
        }
        Err(e) => state.error = Some(e),
    }
}

/// Where the viewer is, for the reminder.
fn playhead(ctx: &PageCtx<'_>) -> Option<String> {
    let dur = ctx.player.duration();
    (dur > 0.0).then(|| {
        format!(
            "{:.2}s into what the viewer has loaded ({dur:.2}s long)",
            ctx.player.position()
        )
    })
}

/// The folder the skills are written to for Claude Code.
fn skills_dir() -> Option<PathBuf> {
    store::prefs_path()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .map(|d| d.join("agent").join("skills"))
}

/// Events of a turn, and whether Claude Code runs it.
type Started = Result<(crossbeam_channel::Receiver<Event>, bool), String>;

/// Starts the in-process loop on a worker thread.
fn start_loop(
    state: &State,
    egui_ctx: &egui::Context,
    provider: Box<dyn provider::Provider>,
    project: &Project,
    plan: Plan,
    scope: Scope,
    cancel: &Arc<AtomicBool>,
) -> Started {
    let chat = state.chat.as_ref().ok_or("no conversation")?;
    let env = tools::Env {
        work: project.work.clone(),
        plan,
        index: project.index.as_ref().map(|i| Arc::new(i.clone())),
        people: project.people.clone(),
        scope,
    };
    let job = agent::Job {
        provider,
        system: context::system(),
        tools: tools::definitions(),
        msgs: model::messages(&chat.log),
        env,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let flag = cancel.clone();
    let repaint = egui_ctx.clone();
    std::thread::Builder::new()
        .name("assistant".to_owned())
        .spawn(move || {
            agent::run(
                job,
                &mut |e| {
                    let _ = tx.send(e);
                    repaint.request_repaint();
                },
                &flag,
            );
        })
        .map_err(|e| format!("could not start the assistant: {e}"))?;
    Ok((rx, false))
}

/// Sends `message` to this conversation's Claude Code session, starting
/// (or resuming) the process when needed.
fn start_claude(
    state: &mut State,
    egui_ctx: &egui::Context,
    exe: PathBuf,
    work: &Path,
    plan: &str,
    message: &str,
) -> Started {
    let key = (work.to_path_buf(), plan.to_owned());
    if state
        .session
        .as_ref()
        .is_none_or(|s| s.key != key || !s.alive())
    {
        state.session = None;
        let cfg = claude_config(exe, work, plan)?;
        let repaint = egui_ctx.clone();
        state.session = Some(claude_code::Session::start(&cfg, plan, move || {
            repaint.request_repaint();
        })?);
    }
    let session = state.session.as_mut().ok_or("Claude Code did not start")?;
    // Events left over from an earlier, stopped turn are not this turn's.
    while session.rx.try_recv().is_ok() {}
    session.send(message)?;
    Ok((session.rx.clone(), true))
}

/// Writes the session's MCP config, prompt and skills, and reads the
/// session to resume.
fn claude_config(exe: PathBuf, work: &Path, plan: &str) -> Result<claude_code::Config, String> {
    let fail = |what: &str, e: &dyn std::fmt::Display| format!("could not prepare {what}: {e}");
    let skills = skills_dir().ok_or("no configuration folder for the skills")?;
    let skills = skills::install(&skills).map_err(|e| fail("the skills", &e))?;
    let chat = work.join("chat");
    std::fs::create_dir_all(&chat).map_err(|e| fail("the chat folder", &e))?;
    let ave = std::env::current_exe().map_err(|e| fail("ave's tool server", &e))?;
    let mcp_config = chat.join("claude-mcp.json");
    std::fs::write(&mcp_config, claude_code::mcp_config(&ave, work).to_string())
        .map_err(|e| fail("the MCP config", &e))?;
    let prompt_file = chat.join("claude-prompt.md");
    let prompt = context::claude_code(&skills.display().to_string(), &skills::listing(&skills));
    std::fs::write(&prompt_file, prompt).map_err(|e| fail("the prompt", &e))?;
    let resume = std::fs::read_to_string(claude_code::session_file(work, plan))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    Ok(claude_code::Config {
        exe,
        work: work.to_path_buf(),
        mcp_config,
        prompt_file,
        skills,
        resume,
        model: None,
    })
}

/// The conversation, as Selects' message list: 16 px above and below,
/// 12 px in from the left, 12 px between messages; tool and thinking
/// activity as quiet rows between them.
fn conversation(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    let Some(chat) = state.chat.as_ref() else {
        return;
    };
    // Tool results by call id, to show under each call.
    let mut results: HashMap<String, (String, bool)> = HashMap::new();
    for line in &chat.log {
        if let LogLine::Msg { msg } = line {
            for b in &msg.blocks {
                if let Block::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } = b
                {
                    results.insert(tool_use_id.clone(), (content.clone(), *is_error));
                }
            }
        }
    }
    let proposals: HashMap<String, Proposal> = chat
        .log
        .iter()
        .filter_map(|l| match l {
            LogLine::Proposal { proposal } => Some((proposal.id.clone(), (**proposal).clone())),
            LogLine::Msg { .. }
            | LogLine::Status { .. }
            | LogLine::Turn { .. }
            | LogLine::Notice { .. }
            | LogLine::Image { .. } => None,
        })
        .collect();
    // Images by call id.
    let mut images: HashMap<String, Vec<String>> = HashMap::new();
    for line in &chat.log {
        if let LogLine::Image { tool_use_id, file } = line {
            images
                .entry(tool_use_id.clone())
                .or_default()
                .push(file.clone());
        }
    }
    let chat_dir = chat.work.join("chat");
    let log = chat.log.clone();
    let mut open: Option<Reference> = None;
    let mut send: Option<String> = None;
    let open_ask = chat
        .ask
        .as_ref()
        .filter(|_| state.turn.is_none())
        .map(|a| a.id.clone());
    let running = state.running.clone();
    let mut stop: Vec<u64> = Vec::new();
    let mut first_cut = false;
    let mut card_choice: Option<cards::Choice> = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin {
                    left: theme::as_margin(SPACE_XS),
                    right: theme::as_margin(SPACE_XS),
                    top: theme::as_margin(SPACE_SM),
                    bottom: theme::as_margin(SPACE_SM),
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if log.is_empty()
                        && state.turn.is_none()
                        && running.is_empty()
                        && state.first_cut.is_none()
                    {
                        empty_state(ctx, state, ui, &mut send, &mut first_cut);
                    }
                    for (i, line) in log.iter().enumerate() {
                        match line {
                            LogLine::Msg { msg } => match msg.role {
                                Role::User => user_bubble(ctx, ui, msg),
                                Role::Assistant => {
                                    for (k, b) in msg.blocks.iter().enumerate() {
                                        match b {
                                            Block::Thinking { thinking, .. }
                                                if !thinking.trim().is_empty() =>
                                            {
                                                look::activity(
                                                    ui,
                                                    theme,
                                                    ("thought", i, k),
                                                    &look::Activity {
                                                        label: "Thought",
                                                        live: false,
                                                        failed: false,
                                                        right: None,
                                                        body: Some((thinking, look::Body::Thinking)),
                                                    },
                                                );
                                            }
                                            Block::Text { text } => {
                                                if let Some(r) =
                                                    message(ctx, ui, ("msg", i, k), text, msg.ts)
                                                {
                                                    open = Some(r);
                                                }
                                            }
                                            Block::ToolUse { id, name, input } => {
                                                if name == tools::EDIT
                                                    && let Some(p) = proposals.get(id)
                                                {
                                                    proposal_card(ctx, state, ui, p, &log);
                                                } else if name == tools::ASK {
                                                    // An open question is the options
                                                    // card above the composer, not a line here.
                                                    if open_ask.as_deref() == Some(id.as_str()) {
                                                        continue;
                                                    }
                                                    let q = input["question"]
                                                        .as_str()
                                                        .unwrap_or_default();
                                                    ui.add_space(SPACE_MD);
                                                    ui.horizontal(|ui| {
                                                        ui.add_space(SPACE_XS);
                                                        ui.add(
                                                            egui::Label::new(
                                                                RichText::new(q)
                                                                    .size(theme::LARGE_SIZE)
                                                                    .color(look::tone_200(theme)),
                                                            )
                                                            .wrap(),
                                                        );
                                                    });
                                                } else {
                                                    let failed =
                                                        results.get(id).is_some_and(|(_, e)| *e);
                                                    let label =
                                                        tools::describe(name, input, false);
                                                    look::activity(
                                                        ui,
                                                        theme,
                                                        ("tool", i, k),
                                                        &look::Activity {
                                                            label: &label,
                                                            live: false,
                                                            failed,
                                                            right: None,
                                                            body: Some((
                                                                results
                                                                    .get(id)
                                                                    .map_or("(no result)", |(c, _)| {
                                                                        c.as_str()
                                                                    }),
                                                                look::Body::Output,
                                                            )),
                                                        },
                                                    );
                                                    for file in images.get(id).into_iter().flatten()
                                                    {
                                                        sheet(state, ui, &chat_dir, file);
                                                    }
                                                }
                                            }
                                            Block::Thinking { .. }
                                            | Block::Context { .. }
                                            | Block::ToolResult { .. }
                                            | Block::Opaque { .. } => {}
                                        }
                                    }
                                }
                            },
                            LogLine::Turn {
                                secs,
                                model,
                                input_tokens,
                                output_tokens,
                                cached_tokens,
                                cost_usd,
                            } => {
                                let cost = cost_usd
                                    .filter(|c| *c > 0.0)
                                    .map(|c| format!(" · ${c:.3}"))
                                    .unwrap_or_default();
                                let cached = if *input_tokens > 0 {
                                    format!(
                                        " ({:.0}% cached)",
                                        *cached_tokens as f32 * 100.0 / *input_tokens as f32
                                    )
                                } else {
                                    String::new()
                                };
                                ui.horizontal(|ui| {
                                    ui.add_space(SPACE_XS);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(format!(
                                                "turn took {} · {model} · {} in{cached} · {} out{cost}",
                                                duration(*secs),
                                                kilo(*input_tokens),
                                                kilo(*output_tokens)
                                            ))
                                            .font(theme::tabular(ui.ctx(), theme::SMALL_SIZE))
                                            .color(theme::rgb(theme.text_faint)),
                                        )
                                        .wrap(),
                                    );
                                });
                            }
                            LogLine::Notice { text, error } => {
                                ui.add_space(SPACE_XS);
                                ui.horizontal(|ui| {
                                    ui.add_space(SPACE_XS);
                                    ui.spacing_mut().item_spacing.x = SPACE_SM;
                                    let color = theme::rgb(if *error {
                                        theme.danger
                                    } else {
                                        theme.text_muted
                                    });
                                    let glyph = if *error {
                                        icons::CIRCLE_ALERT
                                    } else {
                                        icons::INFO
                                    };
                                    icons::icon(ui, glyph, 12.0, color);
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(text).size(theme::BODY_SIZE).color(color),
                                        )
                                        .wrap(),
                                    );
                                });
                            }
                            LogLine::Proposal { .. }
                            | LogLine::Status { .. }
                            | LogLine::Image { .. } => {}
                        }
                    }
                    for r in &running {
                        if let Some(job) = ctx.jobs.get(r.job)
                            && cards::progress(ui, theme, r, job)
                        {
                            stop.push(r.job);
                        }
                    }
                    if let Some(card) = state.first_cut.as_mut() {
                        ui.add_space(SPACE_MD);
                        card_choice = cards::first_cut(ui, theme, card, ctx.selection);
                    }
                    if let Some(turn) = &state.turn {
                        let elapsed = Some(duration(turn.started.elapsed().as_secs_f32()));
                        ui.add_space(SPACE_SM);
                        if !turn.thinking.is_empty() {
                            look::activity(
                                ui,
                                theme,
                                "live_thought",
                                &look::Activity {
                                    label: "Thinking",
                                    live: true,
                                    failed: false,
                                    right: elapsed.clone(),
                                    body: Some((&turn.thinking, look::Body::Thinking)),
                                },
                            );
                        }
                        for (k, t) in turn.tools.iter().enumerate() {
                            look::activity(
                                ui,
                                theme,
                                ("live_tool", k),
                                &look::Activity {
                                    label: t,
                                    live: true,
                                    failed: false,
                                    right: None,
                                    body: None,
                                },
                            );
                        }
                        if !turn.text.is_empty() {
                            ui.add_space(SPACE_SM);
                            markdown_label(ctx, ui, &link_labels(&turn.text));
                        }
                        if turn.thinking.is_empty() {
                            let what = if turn.text.is_empty() && turn.tools.is_empty() {
                                "Thinking"
                            } else {
                                "Working"
                            };
                            look::activity(
                                ui,
                                theme,
                                "live_status",
                                &look::Activity {
                                    label: what,
                                    live: true,
                                    failed: false,
                                    right: elapsed,
                                    body: None,
                                },
                            );
                        }
                    }
                });
        });
    if let Some(text) = send
        && state.turn.is_none()
    {
        if let Some(reason) = blocked_reason(state) {
            state.error = Some(reason);
        } else {
            let refs = complete::merge(ctx.refs, &text);
            let selection = ctx.selection.clone();
            let scope = effective_scope(state, &refs, &selection);
            submit(ctx, state, ui.ctx(), &text, scope);
        }
    }
    if let Some(r) = open {
        open_link(ctx, state, &r);
    }
    for job in stop {
        let _ = ctx.jobs.cancel(job);
    }
    if first_cut {
        state.first_cut = Some(cards::FirstCut::new(
            ctx.selection,
            None,
            &serde_json::Value::Null,
        ));
    }
    match card_choice {
        Some(cards::Choice::Go) => start_first_cut(ctx, state),
        Some(cards::Choice::Cancel) => {
            let asked = state.first_cut.take().is_some_and(|c| c.tool.is_some());
            if asked {
                note(
                    state,
                    "First cut cancelled: nothing was drafted.".to_owned(),
                    false,
                );
            }
        }
        None => {}
    }
}

/// `12s`, `1m 12s`.
fn duration(secs: f32) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

/// The model's text with `ave://` links: the prose in 14 px `tone-200`
/// (link labels inline), the links under it as lime text, and the hover
/// meta row. Returns the link clicked.
fn message(
    ctx: &PageCtx<'_>,
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    text: &str,
    ts: u64,
) -> Option<Reference> {
    let theme = ctx.theme;
    let pieces = links::split(text);
    ui.add_space(SPACE_MD);
    let prose = markdown_label(ctx, ui, &link_labels(text)).rect;
    let targets: Vec<(String, Reference)> = pieces
        .iter()
        .filter_map(|p| match p {
            links::Piece::Link { label, target } => Some((label.clone(), target.clone())),
            links::Piece::Text(_) => None,
        })
        .collect();
    let plain: String = pieces
        .into_iter()
        .map(|p| match p {
            links::Piece::Text(t) => t,
            links::Piece::Link { label, .. } => label,
        })
        .collect();
    let mut clicked = None;
    let mut over = prose;
    if !targets.is_empty() {
        let row = ui.push_id(id, |ui| {
            ui.add_space(SPACE_XS);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(SPACE_XS);
                ui.spacing_mut().item_spacing = Vec2::new(SPACE_MD, SPACE_XS);
                for (k, (label, target)) in targets.iter().enumerate() {
                    let r = ui
                        .push_id(k, |ui| {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(label)
                                        .size(theme::BODY_SIZE)
                                        .color(theme::rgb(theme.accent)),
                                )
                                .sense(Sense::click()),
                            )
                        })
                        .inner;
                    let r =
                        r.on_hover_text(format!("{target} · select it and show it in the viewer"));
                    if r.hovered() {
                        ui.painter().hline(
                            r.rect.x_range(),
                            r.rect.bottom(),
                            Stroke::new(1.0, theme::rgb(theme.accent)),
                        );
                    }
                    if r.clicked() {
                        clicked = Some(target.clone());
                    }
                }
            });
        });
        over = over.union(row.response.rect);
    }
    look::meta_row(ui, theme, over, false, &model::stamp(ts), &plain);
    clicked
}

/// `text` with every `ave://` link replaced by its label in bold.
fn link_labels(text: &str) -> String {
    links::split(text)
        .into_iter()
        .map(|p| match p {
            links::Piece::Text(t) => t,
            links::Piece::Link { label, .. } => format!("**{label}**"),
        })
        .collect()
}

/// A contact sheet under a tool call.
fn sheet(state: &mut State, ui: &mut Ui, dir: &Path, file: &str) {
    let tex = state.images.0.entry(file.to_owned()).or_insert_with(|| {
        let bytes = std::fs::read(dir.join(file)).ok()?;
        let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
        Some(
            ui.ctx()
                .load_texture(format!("assistant-{file}"), color, Default::default()),
        )
    });
    if let Some(tex) = tex {
        let w = ui.available_width().min(tex.size_vec2().x);
        let h = w * tex.size_vec2().y / tex.size_vec2().x.max(1.0);
        ui.add(egui::Image::new((tex.id(), Vec2::new(w, h))));
    }
}

/// Follows an `ave://` link: selects the thing and loads it in the viewer.
fn open_link(ctx: &mut PageCtx<'_>, state: &mut State, r: &Reference) {
    *ctx.selection = vec![r.clone()];
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    let index = project.index.as_ref();
    match &r.target {
        Target::Segment { id } => {
            if let Some((path, s)) = index.and_then(|i| {
                let s = i.segments.iter().find(|s| s.id == *id)?;
                Some((i.clip(&s.clip)?.path.clone(), s))
            }) {
                let (t0, t1, cx) = (f64::from(s.seg.start), f64::from(s.seg.end), s.seg.cx);
                ctx.player.open_clip(Path::new(&path), t0, t1, cx);
            }
        }
        Target::Range { clip, t0, t1 } => {
            if let Some(c) = index.and_then(|i| i.clip(clip)) {
                let path = PathBuf::from(&c.path);
                ctx.player
                    .open_clip(&path, f64::from(*t0), f64::from(*t1), 0.5);
            }
        }
        Target::Frame { clip, t } => {
            if let Some(c) = index.and_then(|i| i.clip(clip)) {
                let path = PathBuf::from(&c.path);
                let t = f64::from(*t);
                ctx.player.open_clip(&path, t, t + 2.0, 0.5);
            }
        }
        Target::Shot { plan, index: k } => {
            if let Some(p) = plan_on_disk(state, plan)
                && let Some(music) = ctx.project.as_ref().and_then(|x| x.music.clone())
            {
                let at = p
                    .timing(&music)
                    .get(k.saturating_sub(1))
                    .map_or(0.0, |(t, _)| f64::from(*t));
                if load_sequence(ctx, &p) {
                    ctx.player.seek(at);
                }
            }
        }
        Target::Bars { from, .. } => {
            let open = state.chat.as_ref().map(|c| c.plan.clone());
            if let Some(p) = open.and_then(|name| plan_on_disk(state, &name))
                && let Some(music) = ctx.project.as_ref().and_then(|x| x.music.clone())
                && *from >= p.start_bar
                && *from < p.end_bar
            {
                let at = music.beat_time(*from, 0) - music.beat_time(p.start_bar, 0);
                if load_sequence(ctx, &p) {
                    ctx.player.seek(f64::from(at));
                }
            }
        }
        Target::Plan { name } => {
            if let Some(p) = plan_on_disk(state, name) {
                load_sequence(ctx, &p);
            }
        }
        Target::Person { .. } => {}
    }
}

/// `1.2k`.
fn kilo(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f32 / 1000.0)
    } else {
        n.to_string()
    }
}

/// What an empty conversation shows, as Selects' chat: one centred 14 px
/// muted line, then quick actions as outline pills that send a prompt.
fn empty_state(
    ctx: &PageCtx<'_>,
    state: &State,
    ui: &mut Ui,
    send: &mut Option<String>,
    first_cut: &mut bool,
) {
    let theme = ctx.theme;
    let plan = state.chat.as_ref().map_or("this plan", |c| c.plan.as_str());
    let line = format!(
        "Edit {plan} by talking about it. Every change comes back as a card you apply or discard."
    );
    let area = ui.available_rect_before_wrap();
    let id = ui.scope_id().with("assistant-empty");
    // The content's height, measured last frame, centres it this frame.
    let last: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(160.0);
    let top = area.top() + ((area.height() - last) / 2.0).max(SPACE_LG);
    let width = (area.width() - SPACE_LG * 2.0).clamp(0.0, 300.0);
    let column = Rect::from_min_size(
        egui::pos2(area.center().x - width / 2.0, top),
        Vec2::new(width, area.height()),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(column)
            .layout(Layout::top_down(Align::Center)),
    );
    child.spacing_mut().item_spacing.y = 6.0;
    child
        .add(
            egui::Label::new(
                RichText::new(line)
                    .size(theme::LARGE_SIZE)
                    .color(theme::ring(theme)),
            )
            .wrap()
            .halign(Align::Center),
        )
        .on_hover_text("Select shots and press R, or type @, to point at them.");
    child.add_space(SPACE_SM);
    if widgets::tip(
        look::suggestion(&mut child, theme, "Suggest a first cut"),
        theme,
        "Pick the length, aspect, pacing and rules, then draft a new plan",
        "",
    )
    .clicked()
    {
        *first_cut = true;
    }
    for (label, prompt) in [
        (
            "Fix what the check flags",
            "Fix everything the check flags, changing as little as possible.",
        ),
        (
            "Put the best fire shot on the drop",
            "Put the strongest fire shot on the drop.",
        ),
        (
            "Find a stronger opening shot",
            "The opening is weak: find a better first shot.",
        ),
        (
            "Which performers repeat too soon?",
            "Which shots repeat a performer too soon?",
        ),
    ] {
        if widgets::tip(
            look::suggestion(&mut child, theme, label),
            theme,
            prompt,
            "",
        )
        .clicked()
        {
            *send = Some(prompt.to_owned());
        }
    }
    let used = child.min_rect().height();
    ui.data_mut(|d| d.insert_temp(id, used));
    ui.advance_cursor_after_rect(Rect::from_min_size(
        area.min,
        Vec2::new(
            area.width(),
            (top - area.top() + used).max(area.height() - SPACE_LG),
        ),
    ));
}

/// A message the person sent, as Selects': right-aligned, at most 85 % of
/// the list, an outlined 10 px bubble with no fill, 14 px white text with
/// mentions in lime, and the hover meta row under it.
fn user_bubble(ctx: &PageCtx<'_>, ui: &mut Ui, msg: &Msg) {
    let theme = ctx.theme;
    let text = msg.text();
    let answered = msg
        .blocks
        .iter()
        .any(|b| matches!(b, Block::ToolResult { .. }));
    if text.trim().is_empty() {
        return;
    }
    ui.add_space(SPACE_MD);
    let pad = Vec2::new(SPACE_MD, 6.0);
    let max = (ui.available_width() * 0.85 - pad.x * 2.0).max(40.0);
    let job = look::mention_job(
        theme,
        text.trim(),
        theme::LARGE_SIZE,
        theme::rgb(theme.text),
        max,
    );
    let galley = ui.painter().layout_job(job);
    let size = galley.size() + pad * 2.0;
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), size.y), Sense::hover());
    let bubble = Rect::from_min_size(egui::pos2(row.right() - size.x, row.top()), size);
    ui.painter().rect_stroke(
        bubble,
        CornerRadius::same(theme::RADIUS_LG),
        theme::input_line(theme),
        egui::StrokeKind::Inside,
    );
    ui.painter()
        .galley(bubble.min + pad, galley, theme::rgb(theme.text));
    let when = model::stamp(msg.ts);
    let note = if answered {
        format!("{when} · answer")
    } else {
        when
    };
    look::meta_row(ui, theme, bubble, true, &note, text.trim());
}

/// The model's text, with bold, code and bullets: 14 px in `tone-200`,
/// bold and headings in the foreground.
fn markdown_label(ctx: &PageCtx<'_>, ui: &mut Ui, text: &str) -> egui::Response {
    let theme = ctx.theme;
    let body = look::tone_200(theme);
    let strong = theme::rgb(theme.text);
    let size = theme::LARGE_SIZE;
    let mut job = egui::text::LayoutJob::default();
    let lines = markdown::parse(text.trim());
    for (n, line) in lines.iter().enumerate() {
        if n > 0 {
            job.append("\n", 0.0, egui::TextFormat::simple(theme::font(size), body));
        }
        let (lead, color) = match line.kind {
            markdown::Kind::Bullet(depth) => (12.0 + 12.0 * depth as f32, body),
            markdown::Kind::Para => (0.0, body),
            markdown::Kind::Heading => (0.0, strong),
        };
        if let markdown::Kind::Bullet(_) = line.kind {
            job.append(
                "•  ",
                lead,
                egui::TextFormat::simple(theme::font(size), theme::rgb(theme.text_muted)),
            );
        }
        for s in &line.spans {
            let mut format = egui::TextFormat::simple(
                if s.code {
                    theme::mono(size - 1.0)
                } else {
                    theme::font(size)
                },
                if s.bold || matches!(line.kind, markdown::Kind::Heading) {
                    strong
                } else {
                    color
                },
            );
            if s.code {
                format.background = theme::rgb(theme.raised);
            }
            if s.bold {
                format.underline = Stroke::NONE;
            }
            job.append(&s.text, 0.0, format);
        }
    }
    let width = ui.available_width() - SPACE_XS * 2.0;
    job.wrap.max_width = width;
    ui.horizontal(|ui| {
        ui.add_space(SPACE_XS);
        ui.add(egui::Label::new(job).wrap())
    })
    .inner
}

/// The card's state for proposal `id`, computed if needed.
fn card_for<'a>(ctx: &PageCtx<'_>, state: &'a mut State, id: &str) -> Option<&'a Card> {
    let chat = state.chat.as_ref()?;
    let proposal = proposal_in(&chat.log, id)?;
    let project = ctx.project.as_ref()?;
    let card = state.cards.entry(id.to_owned()).or_insert_with(|| Card {
        picked: vec![true; proposal.ops.len()],
        computed: Vec::new(),
        result: Err(OpError {
            op: None,
            message: String::new(),
        }),
        before: None,
        after: None,
    });
    if card.computed != card.picked {
        let ops: Vec<EditOp> = proposal
            .ops
            .iter()
            .zip(&card.picked)
            .filter(|(_, p)| **p)
            .map(|(o, _)| o.clone())
            .collect();
        let night = Night {
            index: project.index.as_ref(),
            people: &project.people,
        };
        card.result = ops::apply(&proposal.base, &ops, &Scope::Whole, night);
        card.before = ops::check(
            &proposal.base,
            project.index.as_ref(),
            project.music.as_ref(),
            &project.people,
        );
        card.after = card.result.as_ref().ok().and_then(|(p, _)| {
            ops::check(
                p,
                project.index.as_ref(),
                project.music.as_ref(),
                &project.people,
            )
        });
        card.computed = card.picked.clone();
    }
    Some(card)
}

/// A proposal: summary, each op before → after, the predicted check, and
/// the buttons.
fn proposal_card(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut Ui,
    p: &Proposal,
    log: &[LogLine],
) {
    let theme = ctx.theme;
    let status = model::status_of(log, &p.id);
    let current = plan_on_disk(state, &p.plan);
    let stale = current.as_ref().is_some_and(|c| *c != p.base);
    let mut action: Option<CardAction> = None;
    let pending = matches!(status, Status::Pending);
    let stem = p.plan.clone();
    ui.add_space(SPACE_SM);
    // Selects' edit-agent card while it waits for you (a key-tinted
    // hairline and wash, the title in lime); a plain hairline card after.
    let (fill, stroke, title) = if pending {
        (
            theme::translucent(theme.accent, 13),
            theme::translucent(theme.accent, 51),
            theme::rgb(theme.accent),
        )
    } else {
        (
            egui::Color32::TRANSPARENT,
            theme::ink(theme, theme::INK_LINE),
            theme::rgb(theme.text_muted),
        )
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, stroke))
        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(theme::margin_xy(SPACE_MD, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = SPACE_XS + 2.0;
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Proposal")
                        .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                        .color(title),
                );
                if p.plan
                    != state
                        .chat
                        .as_ref()
                        .map(|c| c.plan.as_str())
                        .unwrap_or_default()
                {
                    ui.label(
                        RichText::new(format!("in {}", p.plan))
                            .size(theme::BODY_SIZE)
                            .color(theme::rgb(theme.text_muted)),
                    );
                }
            });
            ui.add(
                egui::Label::new(
                    RichText::new(&p.summary)
                        .size(theme::LARGE_SIZE)
                        .color(look::tone_200(theme)),
                )
                .wrap(),
            );
            let Some(card) = card_for(ctx, state, &p.id) else {
                return;
            };
            let effects = card
                .result
                .as_ref()
                .map(|(_, e)| e.clone())
                .unwrap_or_default();
            let mut picked = card.picked.clone();
            let mut eff = effects.iter();
            for (k, op) in p.ops.iter().enumerate() {
                card_rule(ui, theme);
                ui.horizontal(|ui| {
                    if pending {
                        ui.checkbox(&mut picked[k], "")
                            .on_hover_text("Include this change");
                    }
                    ui.label(
                        RichText::new(op.label())
                            .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                            .color(theme::rgb(theme.text)),
                    );
                });
                let effect = if picked[k] { eff.next() } else { None };
                match effect {
                    Some(e) => effect_row(ctx, ui, e, &stem),
                    None => {
                        ui.label(
                            RichText::new("(not included)")
                                .size(theme::BODY_SIZE)
                                .color(theme::rgb(theme.text_faint)),
                        );
                    }
                }
                if !op.reason().is_empty() && !matches!(op, EditOp::SetNote { .. }) {
                    ui.add(
                        egui::Label::new(
                            RichText::new(op.reason())
                                .size(theme::BODY_SIZE)
                                .italics()
                                .color(theme::rgb(theme.text_muted)),
                        )
                        .wrap(),
                    );
                }
            }
            card_rule(ui, theme);
            let Some(card) = state.cards.get_mut(&p.id) else {
                return;
            };
            card.picked = picked;
            match (&card.result, &card.before, &card.after) {
                (Err(e), _, _) if !e.message.is_empty() => {
                    card_line(
                        ui,
                        Some(icons::CIRCLE_X),
                        &format!("Cannot apply: {e}"),
                        theme::rgb(theme.danger),
                    );
                }
                (Ok(_), Some(before), Some(after)) => {
                    let (new, fixed) = ops::compare(before, after);
                    let (glyph, text, color) = if after.is_empty() {
                        (
                            icons::CIRCLE_CHECK,
                            "Check passes".to_owned(),
                            theme.success,
                        )
                    } else if new.is_empty() {
                        (
                            icons::INFO,
                            format!("Check: {} problems ({fixed} fixed, none new)", after.len()),
                            theme.text_muted,
                        )
                    } else {
                        (
                            icons::CIRCLE_X,
                            format!(
                                "Check: {} new problem{} ({fixed} fixed)",
                                new.len(),
                                if new.len() == 1 { "" } else { "s" }
                            ),
                            theme.danger,
                        )
                    };
                    let r = card_line(ui, Some(glyph), &text, theme::rgb(color));
                    if !new.is_empty() {
                        r.on_hover_text(
                            new.iter()
                                .map(|q| format!("shot {}: {}", q.shot, q.message))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        );
                        for q in new.iter().take(3) {
                            card_line(
                                ui,
                                None,
                                &format!("shot {}: {}", q.shot, q.message),
                                theme::rgb(theme.danger),
                            );
                        }
                    }
                }
                (Ok(_) | Err(_), _, _) => {
                    card_line(
                        ui,
                        None,
                        "Check unavailable (needs the index and music map)",
                        theme::rgb(theme.text_faint),
                    );
                }
            }
            let n = card.picked.iter().filter(|x| **x).count();
            let ok = card.result.is_ok() && n > 0;
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(SPACE_XS, SPACE_XS);
                match &status {
                    Status::Pending => {
                        if stale {
                            card_line(
                                ui,
                                Some(icons::TRIANGLE_ALERT),
                                "The plan changed since this was proposed; ask again.",
                                theme::rgb(theme.warning),
                            );
                            if widgets::ghost_button(ui, theme, "Discard").clicked() {
                                action = Some(CardAction::Discard);
                            }
                            return;
                        }
                        let all = n == p.ops.len();
                        let label = if all {
                            "Apply".to_owned()
                        } else {
                            format!("Apply {n} of {}", p.ops.len())
                        };
                        if ui
                            .add_enabled_ui(ok, |ui| widgets::primary_button(ui, theme, &label))
                            .inner
                            .clicked()
                        {
                            action = Some(CardAction::Apply);
                        }
                        let preview = ui
                            .add_enabled_ui(ok, |ui| {
                                widgets::button_with(
                                    ui,
                                    theme,
                                    widgets::ButtonKind::Ghost,
                                    Some(icons::PLAY),
                                    "Preview",
                                )
                            })
                            .inner;
                        if widgets::tip(preview, theme, "Play the edited plan in the viewer", "")
                            .clicked()
                        {
                            action = Some(CardAction::Preview);
                        }
                        if widgets::ghost_button(ui, theme, "Discard").clicked() {
                            action = Some(CardAction::Discard);
                        }
                    }
                    Status::Applied { version, ts, .. } => {
                        card_line(
                            ui,
                            Some(icons::CIRCLE_CHECK),
                            &format!("Applied as v{version} · {}", model::stamp(*ts)),
                            theme::rgb(theme.success),
                        );
                        let revert = widgets::ghost_button(ui, theme, "Revert");
                        if widgets::tip(
                            revert,
                            theme,
                            "Restore the plan as it was before this (as a new version)",
                            "",
                        )
                        .clicked()
                        {
                            action = Some(CardAction::Revert);
                        }
                    }
                    Status::Discarded => {
                        card_line(ui, None, "Discarded", theme::rgb(theme.text_faint));
                    }
                    Status::Reverted { version, .. } => {
                        card_line(
                            ui,
                            Some(icons::ROTATE_CCW),
                            &format!("Reverted (v{version})"),
                            theme::rgb(theme.text_faint),
                        );
                    }
                }
            });
        });
    match action {
        Some(CardAction::Apply) => apply(ctx, state, &p.id),
        Some(CardAction::Preview) => preview(ctx, state, &p.id),
        Some(CardAction::Discard) => set_status(state, &p.id, Status::Discarded),
        Some(CardAction::Revert) => revert(ctx, state, p),
        None => {}
    }
}

/// A hairline across a card, between its sections.
fn card_rule(ui: &mut Ui, theme: &crate::theme::Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, theme::ink_line(theme));
}

/// A 12 px line in a card, led by an optional 12 px icon in its colour.
fn card_line(ui: &mut Ui, glyph: Option<char>, text: &str, color: egui::Color32) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_XS + 2.0;
        if let Some(glyph) = glyph {
            icons::icon(ui, glyph, 12.0, color);
        }
        ui.add(egui::Label::new(RichText::new(text).size(theme::BODY_SIZE).color(color)).wrap())
    })
    .inner
}

/// A card button.
#[derive(Clone, Copy, Debug)]
enum CardAction {
    /// Apply the ticked ops.
    Apply,
    /// Play the result.
    Preview,
    /// Throw it away.
    Discard,
    /// Undo an applied proposal.
    Revert,
}

/// One op's before → after.
fn effect_row(ctx: &mut PageCtx<'_>, ui: &mut Ui, e: &Effect, plan: &str) {
    let theme = ctx.theme;
    ui.horizontal(|ui| {
        let side = |ui: &mut Ui, s: Option<&(usize, Shot)>, ctx: &mut PageCtx<'_>| {
            let size = Vec2::new(56.0, 32.0);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            let name = s.map_or_else(|| "no shot".to_owned(), |(slot, _)| format!("Shot {slot}"));
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
            match s {
                Some((slot, shot)) => {
                    let path = ctx
                        .project
                        .as_ref()
                        .and_then(|p| p.index.as_ref())
                        .and_then(|i| i.clip(&shot.clip))
                        .map(|c| PathBuf::from(&c.path));
                    match path {
                        Some(path) => ctx.thumbs.paint(
                            ui,
                            rect,
                            &path,
                            f64::from(shot.t_in) + 0.4,
                            shot.cx,
                            theme::rgb(theme.hover),
                        ),
                        None => {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::same(4),
                                theme::rgb(theme.hover),
                            );
                        }
                    }
                    let r = response.on_hover_text(format!(
                        "#{slot} {}@{:.2} · {} beats · cx {:.2} · {}\n{}",
                        shot.clip, shot.t_in, shot.beats, shot.cx, shot.who, shot.note
                    ));
                    if r.clicked() {
                        *ctx.selection = vec![Reference::new(Target::Shot {
                            plan: plan.to_owned(),
                            index: *slot,
                        })];
                    }
                }
                None => {
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(4),
                        Stroke::new(1.0, theme::rgb(theme.border)),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        };
        side(ui, e.before.as_ref(), ctx);
        icons::icon(ui, icons::CHEVRON_RIGHT, 12.0, theme::rgb(theme.text_muted));
        side(ui, e.after.as_ref(), ctx);
        ui.vertical(|ui| {
            let line = |s: Option<&(usize, Shot)>| {
                s.map_or_else(
                    || "—".to_owned(),
                    |(slot, sh)| format!("#{slot} {}@{:.1} {}b", sh.clip, sh.t_in, sh.beats),
                )
            };
            let font = theme::tabular(ui.ctx(), theme::SMALL_SIZE);
            ui.label(
                RichText::new(line(e.before.as_ref()))
                    .font(font.clone())
                    .color(theme::rgb(theme.text_muted)),
            );
            ui.label(
                RichText::new(line(e.after.as_ref()))
                    .font(font)
                    .color(theme::rgb(theme.text)),
            );
        });
    });
}

/// Records a proposal's new status.
fn set_status(state: &mut State, id: &str, status: Status) {
    if let Some(chat) = state.chat.as_mut()
        && let Some(e) = chat.push(LogLine::Status {
            id: id.to_owned(),
            status,
        })
    {
        state.error = Some(e);
    }
}

/// Applies the ticked ops of proposal `id`: a new plan version, through
/// the same engine path as the agent's `plan_edit` with `commit` (so a plan
/// that moved since the proposal is refused).
fn apply(ctx: &mut PageCtx<'_>, state: &mut State, id: &str) {
    let Some(card) = card_for(ctx, state, id) else {
        return;
    };
    let picked: Vec<usize> = card
        .picked
        .iter()
        .enumerate()
        .filter(|(_, p)| **p)
        .map(|(k, _)| k)
        .collect();
    let after = match &card.result {
        Ok((plan, _)) => plan.clone(),
        Err(e) => {
            state.error = Some(format!("cannot apply: {e}"));
            return;
        }
    };
    let Some(chat) = state.chat.as_mut() else {
        return;
    };
    let Some(p) = proposal_in(&chat.log, id) else {
        return;
    };
    let reason = format!("assistant: {}", p.summary);
    let saved = if p.base_version.is_empty() {
        store::save_version(&chat.work, &p.plan, &after, &reason)
    } else {
        let ops: Vec<EditOp> = picked
            .iter()
            .filter_map(|k| p.ops.get(*k).cloned())
            .collect();
        crate::mcp::commit_edit(&chat.work, &p.plan, &ops, &reason, &p.base_version)
    };
    match saved {
        Ok(version) => {
            chat.cached = None;
            set_status(
                state,
                id,
                Status::Applied {
                    version,
                    ops: picked,
                    ts: model::now(),
                },
            );
            if let Some(project) = ctx.project.as_mut() {
                project.reload_plans();
            }
        }
        Err(e) => state.error = Some(format!("could not apply: {e:#}")),
    }
}

/// Restores the plan a proposal started from, as a new version.
fn revert(ctx: &mut PageCtx<'_>, state: &mut State, p: &Proposal) {
    let Some(chat) = state.chat.as_mut() else {
        return;
    };
    match store::save_version(
        &chat.work,
        &p.plan,
        &p.base,
        &format!("revert: {}", p.summary),
    ) {
        Ok(version) => {
            chat.cached = None;
            set_status(
                state,
                &p.id,
                Status::Reverted {
                    version,
                    ts: model::now(),
                },
            );
            if let Some(project) = ctx.project.as_mut() {
                project.reload_plans();
            }
        }
        Err(e) => state.error = Some(format!("could not save the plan: {e:#}")),
    }
}

/// Plan `name` as it is on disk now.
fn plan_on_disk(state: &mut State, name: &str) -> Option<Plan> {
    let chat = state.chat.as_mut()?;
    if chat.plan == name {
        return chat.current();
    }
    Plan::load(&chat.work.join("plans").join(format!("{name}.toml"))).ok()
}

/// Plays the edited plan in the viewer.
fn preview(ctx: &mut PageCtx<'_>, state: &mut State, id: &str) {
    let plan = match card_for(ctx, state, id).map(|c| &c.result) {
        Some(Ok((plan, _))) => plan.clone(),
        Some(Err(_)) | None => return,
    };
    play(ctx, &plan);
}

/// Opens `plan` as a sequence in the viewer and plays it.
fn play(ctx: &mut PageCtx<'_>, plan: &Plan) {
    if load_sequence(ctx, plan) {
        ctx.player.play();
    }
}

/// Loads `plan` as a sequence in the viewer; false when the night cannot
/// (no index or music map yet).
fn load_sequence(ctx: &mut PageCtx<'_>, plan: &Plan) -> bool {
    let Some(project) = ctx.project.as_ref() else {
        return false;
    };
    let (Some(music), Some(index)) = (project.music.as_ref(), project.index.as_ref()) else {
        return false;
    };
    let shots: Vec<crate::gui::player::SeqShot> = plan
        .shots
        .iter()
        .zip(plan.timing(music))
        .filter_map(|(s, (_, dur))| {
            let clip = index.clip(&s.clip)?;
            Some(crate::gui::player::SeqShot {
                path: clip.path.clone().into(),
                t_in: f64::from(s.t_in),
                dur: f64::from(dur),
                cx: s.cx,
            })
        })
        .collect();
    let start = f64::from(music.beat_time(plan.start_bar, 0));
    ctx.player
        .open_sequence(shots, Some((plan.music.clone().into(), start)));
    true
}

/// The History tab: what the assistant applied, and every saved version.
fn history(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    let Some(chat) = state.chat.as_ref() else {
        return;
    };
    let mut restore: Option<(usize, PathBuf)> = None;
    let mut play_version: Option<PathBuf> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin {
                    left: theme::as_margin(SPACE_XS),
                    right: theme::as_margin(SPACE_XS),
                    top: theme::as_margin(SPACE_MD),
                    bottom: theme::as_margin(SPACE_SM),
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = SPACE_XS + 2.0;
                    widgets::menu_heading(ui, theme, "Applied proposals");
                    let mut any = false;
                    for line in chat.log.iter().rev() {
                        if let LogLine::Status { id, status } = line {
                            let summary = proposal_in(&chat.log, id)
                                .map(|p| p.summary)
                                .unwrap_or_default();
                            let (when, what) = match status {
                                Status::Applied { version, ops, ts } => (
                                    model::stamp(*ts),
                                    format!(
                                        "v{version} · {} op{} · {summary}",
                                        ops.len(),
                                        if ops.len() == 1 { "" } else { "s" }
                                    ),
                                ),
                                Status::Reverted { version, ts } => {
                                    (model::stamp(*ts), format!("v{version} · reverted: {summary}"))
                                }
                                Status::Pending | Status::Discarded => continue,
                            };
                            any = true;
                            ui.add(
                                egui::Label::new(
                                    RichText::new(what)
                                        .size(theme::BODY_SIZE)
                                        .color(look::tone_200(theme)),
                                )
                                .wrap(),
                            );
                            ui.label(
                                RichText::new(when)
                                    .font(theme::tabular(ui.ctx(), theme::TINY_SIZE))
                                    .color(theme::rgb(theme.text_faint)),
                            );
                        }
                    }
                    if !any {
                        ui.label(
                            RichText::new("Nothing applied yet.")
                                .size(theme::BODY_SIZE)
                                .color(theme::rgb(theme.text_muted)),
                        );
                    }
                    ui.add_space(SPACE_MD);
                    widgets::menu_heading(ui, theme, &format!("Versions of {}", chat.plan));
                    let versions = store::versions(&chat.work, &chat.plan);
                    if versions.is_empty() {
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    "No saved versions yet. Applying a proposal keeps the plan as it was as v1.",
                                )
                                .size(theme::BODY_SIZE)
                                .color(theme::rgb(theme.text_muted)),
                            )
                            .wrap(),
                        );
                    }
                    let last = versions.last().map(|v| v.n);
                    for v in versions.iter().rev() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = SPACE_SM;
                            let current = Some(v.n) == last;
                            ui.label(
                                RichText::new(format!("v{}", v.n))
                                    .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                                    .color(theme::rgb(theme.text)),
                            );
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                if !current {
                                    let r = widgets::ghost_button(ui, theme, "Restore");
                                    if widgets::tip(
                                        r,
                                        theme,
                                        "Make this the current plan (saved as a new version)",
                                        "",
                                    )
                                    .clicked()
                                    {
                                        restore = Some((v.n, v.path.clone()));
                                    }
                                }
                                if widgets::icon_btn(ui, theme, icons::PLAY, "Play this version", "")
                                    .clicked()
                                {
                                    play_version = Some(v.path.clone());
                                }
                                if current {
                                    widgets::pill(ui, theme, "current", widgets::Tone::Neutral);
                                }
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    let mut job = egui::text::LayoutJob::simple_singleline(
                                        v.header
                                            .split(" · ")
                                            .skip(1)
                                            .collect::<Vec<_>>()
                                            .join(" · "),
                                        theme::font(theme::BODY_SIZE),
                                        theme::rgb(theme.text_muted),
                                    );
                                    job.wrap.max_width = ui.available_width();
                                    job.wrap.max_rows = 1;
                                    job.wrap.break_anywhere = true;
                                    ui.add(egui::Label::new(job))
                                        .on_hover_text(v.header.clone());
                                });
                            });
                        });
                    }
                });
        });
    if let Some(path) = play_version {
        match Plan::load(&path) {
            Ok(plan) => play(ctx, &plan),
            Err(e) => state.error = Some(format!("{e:#}")),
        }
    }
    if let Some((n, path)) = restore
        && let Some(chat) = state.chat.as_mut()
    {
        let result = Plan::load(&path).and_then(|plan| {
            store::save_version(&chat.work, &chat.plan, &plan, &format!("restored v{n}"))
        });
        match result {
            Ok(_) => {
                chat.cached = None;
                if let Some(project) = ctx.project.as_mut() {
                    project.reload_plans();
                }
            }
            Err(e) => state.error = Some(format!("could not restore v{n}: {e:#}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::{NodeT as _, Queryable};

    use super::model::{Block, LogLine, Msg, Role};

    /// A night with one plan (`reel-a`, two shots), and with `log` a
    /// conversation in which the person mentioned its first shot. No index,
    /// no music.
    fn night(log: bool) -> anyhow::Result<(tempfile::TempDir, std::path::PathBuf)> {
        use crate::plan::{Aspect, Plan, Rules, Shot};
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-12").join("_ave");
        std::fs::create_dir_all(work.join("plans"))?;
        let shot = |clip: &str, t_in: f32| Shot {
            clip: clip.into(),
            t_in,
            beats: 4,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: String::new(),
        };
        Plan {
            name: "reel-a".into(),
            aspect: Aspect::Vertical,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 1,
            card_bar: 2,
            end_bar: 3,
            end_card: String::new(),
            rules: Rules::default(),
            finish: Default::default(),
            shots: vec![shot("A", 1.0), shot("B", 2.0)],
            comments: Vec::new(),
        }
        .save(&work.join("plans").join("reel-a.toml"))?;
        if log {
            let said = Msg::new(
                Role::User,
                vec![Block::Text {
                    text: "tighten @shot:reel-a#1 please".into(),
                }],
            );
            super::store::append(
                &super::store::chat_path(&work, "reel-a"),
                &LogLine::Msg { msg: said },
            )?;
        }
        Ok((dir, work))
    }

    /// The app on the review workspace of `work`, chat open, headless.
    fn harness(
        work: std::path::PathBuf,
    ) -> egui_kittest::Harness<'static, Option<crate::gui::app::App>> {
        use crate::gui::app::{Action, App, Page};
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1680.0, 1000.0))
            .build_ui_state(
                move |ui, app: &mut Option<App>| {
                    let app = app.get_or_insert_with(|| {
                        let mut app = App::new(
                            ui.ctx(),
                            crate::config::Config::default(),
                            Some(work.clone()),
                        );
                        app.apply(ui.ctx(), Action::Go(Page::Review));
                        app
                    });
                    app.show(ui);
                },
                None,
            );
        harness.run_steps(6);
        harness
    }

    #[test]
    fn empty_chat_offers_quick_actions_and_an_icon_header() -> anyhow::Result<()> {
        let (_dir, work) = night(false)?;
        let mut h = harness(work);
        for label in [
            "Fix what the check flags",
            "Chats: the plan this conversation is about",
            "History: applied proposals and plan versions",
            "Assistant settings: model key, local server, autonomy",
            "Chat",
            "Comments",
            "Model",
        ] {
            assert!(
                h.query_all_by_label(label).next().is_some(),
                "the chat panel shows {label:?}"
            );
        }
        let send = h
            .query_by_label("Send")
            .ok_or_else(|| anyhow::anyhow!("no Send button"))?;
        assert!(
            send.accesskit_node().is_disabled(),
            "nothing typed, nothing to send"
        );
        assert!(
            h.query_by_label("New").is_none(),
            "no text button in the header: New chat lives in the Chats menu"
        );

        let history = "History: applied proposals and plan versions";
        h.query_by_label(history)
            .ok_or_else(|| anyhow::anyhow!("no History button"))?
            .click();
        h.run_steps(3);
        assert!(
            h.query_by_label("Versions of reel-a").is_some(),
            "History lists the plan's versions"
        );
        h.query_by_label(history)
            .ok_or_else(|| anyhow::anyhow!("no History button"))?
            .click();
        h.run_steps(3);
        // The timeline has a Comments toggle too; the chat's is leftmost.
        h.query_all_by_label("Comments")
            .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
            .ok_or_else(|| anyhow::anyhow!("no Comments switch"))?
            .click();
        h.run_steps(3);
        assert!(
            h.query_by_label("No comments").is_some(),
            "Comments mode shows the plan's (empty) comments"
        );
        Ok(())
    }

    #[test]
    fn a_sent_message_is_a_bubble_and_typing_enables_send() -> anyhow::Result<()> {
        let (_dir, work) = night(true)?;
        let mut h = harness(work);
        assert!(
            h.query_by_label("Copy message").is_some(),
            "the message has its meta row"
        );
        assert!(
            h.query_by_label("Fix what the check flags").is_none(),
            "no quick actions once the conversation has started"
        );
        let app = h
            .state_mut()
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no app"))?;
        // The local model needs no key or installed CLI, so Send's state
        // does not depend on the machine (CI has no `claude`, no API key).
        app.pages
            .assistant
            .prefs
            .get_or_insert_with(super::store::Prefs::default)
            .model = Some("local".into());
        app.pages.assistant.input = "and @shot:reel-a#2 too".into();
        h.run_steps(3);
        let send = h
            .query_by_label("Send")
            .ok_or_else(|| anyhow::anyhow!("no Send button"))?;
        assert!(
            !send.accesskit_node().is_disabled(),
            "typed text can be sent"
        );
        Ok(())
    }

    /// Appends `lines` to `reel-a`'s conversation.
    fn write_log(work: &std::path::Path, lines: Vec<LogLine>) -> anyhow::Result<()> {
        let path = super::store::chat_path(work, "reel-a");
        for line in &lines {
            super::store::append(&path, line)?;
        }
        Ok(())
    }

    /// A conversation in which the agent staged an edit of `reel-a` (shot 2
    /// moved to the front, with a note), proposed on the plan as it is on
    /// disk, with that file's version, as `plan_edit` stages it.
    fn staged_edit(work: &std::path::Path) -> anyhow::Result<()> {
        use super::ops::EditOp;
        let path = work.join("plans").join("reel-a.toml");
        let ops = vec![
            EditOp::MoveShot { from: 2, to: 1 },
            EditOp::SetNote {
                index: 1,
                note: "B opens: the brighter, closer shot".into(),
            },
        ];
        let input =
            serde_json::json!({"name": "reel-a", "reason": "Open on B", "ops": ops.clone()});
        write_log(
            work,
            vec![
                LogLine::Msg {
                    msg: Msg::new(
                        Role::User,
                        vec![Block::Text {
                            text: "Open on the stronger shot".into(),
                        }],
                    ),
                },
                LogLine::Msg {
                    msg: Msg::new(
                        Role::Assistant,
                        vec![
                            Block::Text {
                                text: "B is the stronger opener; staged it.".into(),
                            },
                            Block::ToolUse {
                                id: "p1".into(),
                                name: super::tools::EDIT.into(),
                                input,
                            },
                        ],
                    ),
                },
                LogLine::Proposal {
                    proposal: Box::new(super::model::Proposal {
                        id: "p1".into(),
                        plan: "reel-a".into(),
                        summary: "Open on B, the stronger shot".into(),
                        ops,
                        base: crate::plan::Plan::load(&path)?,
                        base_version: crate::mcp::plan_version(&path)?,
                        ts: super::model::now(),
                    }),
                },
            ],
        )
    }

    /// Whether some widget or text shows `text`.
    fn shows(h: &egui_kittest::Harness<'_, Option<crate::gui::app::App>>, text: &str) -> bool {
        h.query_all_by_label_contains(text).next().is_some()
    }

    #[test]
    fn a_staged_edit_is_a_card_whose_apply_saves_a_version() -> anyhow::Result<()> {
        let (_dir, work) = night(false)?;
        staged_edit(&work)?;
        let mut h = harness(work.clone());
        for label in ["Apply", "Preview", "Discard"] {
            assert!(
                h.query_all_by_label(label).next().is_some(),
                "the proposal card offers {label}"
            );
        }
        assert!(
            shows(&h, "Open on B, the stronger shot"),
            "the card's title is the reason"
        );
        h.query_all_by_label("Apply")
            .next()
            .ok_or_else(|| anyhow::anyhow!("no Apply button"))?
            .click();
        h.run_steps(4);
        let plan = crate::plan::Plan::load(&work.join("plans").join("reel-a.toml"))?;
        assert_eq!(
            plan.shots.first().map(|s| s.clip.as_str()),
            Some("B"),
            "Apply wrote the edited plan"
        );
        assert!(
            !super::store::versions(&work, "reel-a").is_empty(),
            "as a plan version"
        );
        assert!(shows(&h, "Applied as v"), "the card says it was applied");
        assert!(
            h.query_all_by_label("Apply").next().is_none(),
            "and offers Apply no more"
        );
        Ok(())
    }

    #[test]
    fn the_first_cut_card_opens_from_the_quick_action_and_cancel_closes_it() -> anyhow::Result<()> {
        let (_dir, work) = night(false)?;
        let mut h = harness(work);
        h.query_by_label("Suggest a first cut")
            .ok_or_else(|| anyhow::anyhow!("no first-cut quick action"))?
            .click();
        h.run_steps(3);
        for label in [
            "Here's our plan",
            "15 s",
            "30 s",
            "60 s",
            "Custom",
            "9:16",
            "4:5",
            "16:9",
            "Calm",
            "On the beat",
            "Bursts",
            "No repeated shots",
            "People spacing",
            "Drop on the hero",
            "Continue",
        ] {
            assert!(
                h.query_all_by_label(label).next().is_some(),
                "the first-cut card shows {label:?}"
            );
        }
        h.query_by_label("Continue")
            .ok_or_else(|| anyhow::anyhow!("no Continue"))?
            .click();
        h.run_steps(3);
        assert!(
            shows(&h, "Index the footage and map the music first"),
            "an unindexed night cannot be drafted, and the card says why"
        );
        h.query_by_label("Close the first-cut card")
            .ok_or_else(|| anyhow::anyhow!("no close button"))?
            .click();
        h.run_steps(3);
        assert!(
            h.query_by_label("Here's our plan").is_none(),
            "closing dismisses the card"
        );
        Ok(())
    }

    #[test]
    fn a_step_the_agent_asks_for_waits_for_approval_then_runs_as_a_job() -> anyhow::Result<()> {
        let (_dir, work) = night(false)?;
        let call = |id: &str, input: serde_json::Value| Block::ToolUse {
            id: id.into(),
            name: crate::mcp::RUN_STEP.into(),
            input,
        };
        write_log(
            &work,
            vec![LogLine::Msg {
                msg: Msg::new(
                    Role::Assistant,
                    vec![
                        call("s1", serde_json::json!({"step": "caption"})),
                        call(
                            "s2",
                            serde_json::json!({"step": "render", "plan": "reel-a"}),
                        ),
                    ],
                ),
            }],
        )?;
        let mut h = harness(work);
        assert!(shows(&h, "Allow this?"), "the request asks first");
        assert!(
            shows(&h, "Caption the shot log"),
            "and says what it will do"
        );
        h.query_all_by_label("Cancel")
            .next()
            .ok_or_else(|| anyhow::anyhow!("no Cancel"))?
            .click();
        h.run_steps(3);
        assert!(
            shows(&h, "Not started: caption the shot log"),
            "Cancel starts nothing and says so"
        );
        assert!(
            shows(&h, "Render reel-a"),
            "the next request comes up, with its estimate"
        );
        h.query_by_label("Approve")
            .ok_or_else(|| anyhow::anyhow!("no Approve"))?
            .click();
        h.run_steps(3);
        assert!(
            shows(&h, "Approved: render reel-a"),
            "Approve starts the job and says so"
        );
        // The night has no index, so the render fails at once; the chat
        // follows the job to its end.
        let mut ended = false;
        for _ in 0..50 {
            h.run_steps(2);
            if shows(&h, "Render reel-a failed") {
                ended = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(ended, "the job's end lands in the chat");
        assert!(!shows(&h, "Allow this?"), "no request is left waiting");
        Ok(())
    }
}

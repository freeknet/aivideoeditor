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
//! | [`agent`] | the tool loop of one turn |
//! | [`provider`] | Claude (streamed) and a local llama.cpp server |
//! | [`store`] | chat logs, plan versions, preferences |
//! | [`complete`] | the `@` autocomplete and chip/token merging |
//! | [`markdown`] | the bold/bullet text the model writes |
//!
//! A conversation belongs to one plan and lives in `<work>/chat/<plan>.jsonl`;
//! switching plans switches conversations, and reopening the night restores
//! the panel as it was.

mod agent;
mod complete;
mod context;
mod markdown;
mod model;
mod ops;
mod provider;
mod store;
mod tools;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Instant, SystemTime};

use egui::{Align, CornerRadius, Layout, RichText, Sense, Stroke, Ui, Vec2};

use self::agent::Event;
use self::model::{Block, LogLine, Msg, Proposal, Role, Status};
use self::ops::{EditOp, Effect, Night, OpError, Scope};
use self::provider::Delta;
use crate::gui::app::Action;
use crate::gui::pages::PageCtx;
use crate::gui::ui::theme::{self, SPACE_SM, SPACE_XS};
use crate::gui::ui::widgets;
use crate::plan::{Plan, Problem, Shot};
use crate::project::Project;
use crate::refs::{Modifier, Reference, Target};

/// Models the selector offers: id and label. `local` is the llama.cpp server.
const MODELS: [(&str, &str); 4] = [
    ("claude-opus-5-5", "Claude Opus 5.5"),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
    ("claude-haiku-5-5", "Claude Haiku 5.5"),
    ("local", "Local (llama.cpp)"),
];

/// The settings key this panel uses in `ave.toml`'s `[pages]`.
const PAGE_KEY: &str = "assistant";

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
}

/// Draws the panel.
pub(crate) fn panel(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    if state.prefs.is_none() {
        let prefs = store::Prefs::load();
        state.url_draft = prefs.local();
        state.prefs = Some(prefs);
    }
    let Some(project) = ctx.project.as_ref() else {
        widgets::title(ui, ctx.theme, "Assistant");
        widgets::muted(ui, ctx.theme, "Open a night to talk about its plans.");
        return;
    };
    sync_chat(project, state, &ctx.selection.clone());
    poll(ctx, state, ui.ctx());

    header(ctx, state, ui);
    if state.settings_open {
        settings(ctx, state, ui);
    }
    if let Some(e) = state.error.clone() {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(e).small().color(theme::rgb(ctx.theme.danger)));
            if ui.small_button("✕").clicked() {
                state.error = None;
            }
        });
    }
    if state.chat.is_none() {
        ui.add_space(SPACE_SM);
        widgets::muted(
            ui,
            ctx.theme,
            "This night has no plans yet. Draft one (ave plan) and the assistant can edit it.",
        );
        return;
    }
    match state.tab {
        Tab::Chat => {
            egui::Panel::bottom("assistant_composer")
                .resizable(false)
                .show_separator_line(true)
                .frame(egui::Frame::new().inner_margin(theme::margin_xy(0.0, SPACE_SM)))
                .show(ui, |ui| composer(ctx, state, ui));
            conversation(ctx, state, ui);
        }
        Tab::History => history(ctx, state, ui),
    }
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
    state.cards.clear();
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

/// Drains the worker's events into the conversation.
fn poll(ctx: &mut PageCtx<'_>, state: &mut State, egui_ctx: &egui::Context) {
    let Some(turn) = state.turn.as_mut() else {
        return;
    };
    egui_ctx.request_repaint_after(std::time::Duration::from_millis(100));
    let mut done = false;
    let mut proposals = Vec::new();
    while let Ok(event) = turn.rx.try_recv() {
        let Some(chat) = state.chat.as_mut() else {
            break;
        };
        let err = match event {
            Event::Delta(Delta::Text(t)) => {
                turn.text.push_str(&t);
                None
            }
            Event::Delta(Delta::Thinking(t)) => {
                turn.thinking.push_str(&t);
                None
            }
            Event::Delta(Delta::Tool(name)) => {
                turn.tools.push(name);
                None
            }
            Event::Msg(msg) => {
                turn.text.clear();
                turn.thinking.clear();
                turn.tools.clear();
                chat.push(LogLine::Msg { msg })
            }
            Event::Proposal(p) => {
                proposals.push(p.id.clone());
                chat.push(LogLine::Proposal { proposal: p })
            }
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
            Event::Done { secs, model, usage } => {
                done = true;
                chat.push(LogLine::Turn {
                    secs,
                    model,
                    input_tokens: usage.input,
                    output_tokens: usage.output,
                    cached_tokens: usage.cached,
                })
            }
        };
        if err.is_some() {
            state.error = err;
        }
    }
    if done {
        state.turn = None;
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

/// Whether this night applies clean proposals without asking.
fn auto_apply(ctx: &PageCtx<'_>) -> bool {
    ctx.project
        .as_ref()
        .and_then(|p| p.settings.pages.get(PAGE_KEY))
        .and_then(|t| t.get("auto_apply"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
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
    let mut switch_to = None;
    ui.horizontal(|ui| {
        let current = state
            .chat
            .as_ref()
            .map(|c| c.plan.clone())
            .unwrap_or_default();
        ui.add_enabled_ui(state.turn.is_none(), |ui| {
            egui::ComboBox::from_id_salt("assistant_plan")
                .selected_text(
                    RichText::new(if current.is_empty() {
                        "no plan"
                    } else {
                        &current
                    })
                    .strong(),
                )
                .width(120.0)
                .show_ui(ui, |ui| {
                    for s in &stems {
                        if ui.selectable_label(*s == current, s).clicked() && *s != current {
                            switch_to = Some(s.clone());
                        }
                    }
                })
                .response
                .on_hover_text("The plan this conversation is about");
        });
        for tab in [Tab::Chat, Tab::History] {
            let label = match tab {
                Tab::Chat => "Chat",
                Tab::History => "History",
            };
            if ui.selectable_label(state.tab == tab, label).clicked() {
                state.tab = tab;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button("⚙")
                .on_hover_text("Assistant settings: model key, local server, autonomy")
                .clicked()
            {
                state.settings_open = !state.settings_open;
            }
            let can_reset =
                state.turn.is_none() && state.chat.as_ref().is_some_and(|c| !c.log.is_empty());
            if ui
                .add_enabled(can_reset, egui::Button::new("New").small())
                .on_hover_text("Start a fresh conversation (the old one is kept beside it)")
                .clicked()
                && let Some(chat) = state.chat.as_mut()
            {
                let old = chat.log_path();
                let kept = old.with_file_name(format!("{}.{}.jsonl", chat.plan, model::now()));
                if let Err(e) = std::fs::rename(&old, &kept) {
                    state.error = Some(format!("could not archive the conversation: {e}"));
                } else {
                    *chat = open_chat(&chat.work.clone(), &chat.plan.clone());
                    state.cards.clear();
                }
            }
        });
    });
    if let Some(plan) = switch_to
        && let Some(project) = ctx.project.as_ref()
    {
        state.chat = Some(open_chat(&project.work, &plan));
        state.cards.clear();
        state.error = set_page_option(ctx, "plan", toml::Value::String(plan));
    }
}

/// The settings strip: key, local server, autonomy.
fn settings(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    egui::Frame::new()
        .fill(theme::rgb(theme.raised))
        .stroke(Stroke::new(1.0, theme::rgb(theme.border)))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin(SPACE_SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let Some(prefs) = state.prefs.as_mut() else {
                return;
            };
            let env_key = std::env::var("ANTHROPIC_API_KEY").is_ok_and(|k| !k.trim().is_empty());
            ui.label(RichText::new("Claude API key").strong());
            if env_key {
                widgets::muted(ui, theme, "Using ANTHROPIC_API_KEY from the environment.");
            } else {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.key_draft)
                            .password(true)
                            .hint_text(if prefs.api_key.is_some() { "saved — paste to replace" } else { "sk-ant-…" })
                            .desired_width(ui.available_width() - 60.0),
                    );
                    if ui.button("Save").clicked() {
                        let key = state.key_draft.trim().to_owned();
                        prefs.api_key = (!key.is_empty()).then_some(key);
                        state.key_draft.clear();
                        state.error = prefs.save().err().map(|e| format!("{e:#}"));
                    }
                });
                let at = store::prefs_path().map(|p| p.display().to_string()).unwrap_or_default();
                widgets::muted(
                    ui,
                    theme,
                    &format!("Stored in plain text at {at}. Or set ANTHROPIC_API_KEY. Keys: console.anthropic.com."),
                );
            }
            ui.add_space(SPACE_XS);
            ui.label(RichText::new("Local model server").strong());
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut state.url_draft).desired_width(ui.available_width() - 60.0));
                if ui.button("Save").clicked() {
                    prefs.local_url = Some(state.url_draft.trim().to_owned());
                    state.error = prefs.save().err().map(|e| format!("{e:#}"));
                }
            });
            widgets::muted(ui, theme, "llama.cpp's server (AVE_LLM_URL). Small local models plan poorly; use it offline.");
            ui.add_space(SPACE_XS);
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

/// The model the person picked.
fn model_id(state: &State) -> String {
    state
        .prefs
        .as_ref()
        .and_then(|p| p.model.clone())
        .unwrap_or_else(|| MODELS[0].0.to_owned())
}

/// Why sending is impossible right now, if it is.
fn blocked_reason(state: &State) -> Option<String> {
    let model = model_id(state);
    if model != "local" && state.prefs.as_ref().and_then(store::Prefs::key).is_none() {
        return Some(
            "No Claude API key. Set ANTHROPIC_API_KEY, paste a key under ⚙, or pick the local model."
                .to_owned(),
        );
    }
    None
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

/// The composer: question buttons, scope and reference chips, the input,
/// and the add / model / send row.
fn composer(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    let busy = state.turn.is_some();
    let mut send: Option<String> = None;

    if let Some(ask) = state.chat.as_ref().and_then(|c| c.ask.clone())
        && !busy
    {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Answer:")
                    .small()
                    .color(theme::rgb(theme.text_muted)),
            )
            .on_hover_text(&ask.question);
            for o in &ask.options {
                if widgets::button(ui, theme, o, widgets::Emphasis::Secondary).clicked() {
                    send = Some(o.clone());
                }
            }
        });
        ui.add_space(SPACE_XS);
    }

    let refs = complete::merge(ctx.refs, &state.input);
    let selection = ctx.selection.clone();
    let scope = effective_scope(state, &refs, &selection);
    let (change, keep) = scoped_shots(state, &refs, &selection);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(SPACE_XS);
        for choice in [
            ScopeChoice::Whole,
            ScopeChoice::Selected,
            ScopeChoice::Build,
            ScopeChoice::FromDrop,
        ] {
            let on = scope == choice;
            let mut label = choice.label().to_owned();
            if choice == ScopeChoice::Selected {
                label = format!("{label} ({})", change.len());
            }
            let text = RichText::new(label).small().color(theme::rgb(if on {
                theme.text
            } else {
                theme.text_muted
            }));
            let r = ui
                .selectable_label(on, text)
                .on_hover_text(if state.scope.is_none() && on {
                    "Scope follows the selection; click to pin one"
                } else {
                    "What the assistant may change"
                });
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
                    "· keeping {}",
                    keep.iter()
                        .map(|k| format!("#{k}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ))
                .small()
                .color(theme::rgb(theme.text_muted)),
            );
        }
    });

    chips(ctx, state, ui, &refs);

    // The input, with Enter to send and the `@` list.
    let id = ui.make_persistent_id("assistant_input");
    let focused = ui.memory(|m| m.has_focus(id));
    let cursor = egui::text_edit::TextEditState::load(ui.ctx(), id)
        .and_then(|s| s.cursor.char_range())
        .map(|r| usize::from(r.primary.index));
    let active = cursor
        .filter(|_| focused)
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
    let hint = if state.chat.as_ref().is_some_and(|c| c.ask.is_some()) {
        "Answer, or say what you mean…"
    } else {
        "Ask for an edit or about the footage…  @ to reference, + ! > ~ * for intent"
    };
    let output = egui::TextEdit::multiline(&mut state.input)
        .id(id)
        .hint_text(hint)
        .desired_rows(3)
        .desired_width(f32::INFINITY)
        .return_key(egui::KeyboardShortcut::new(
            egui::Modifiers::SHIFT,
            egui::Key::Enter,
        ))
        .show(ui);
    if output.response.changed() {
        state.pick = 0;
    }
    let input_rect = output.response.rect;

    if !cands.is_empty() {
        let area = egui::Area::new(id.with("ac"))
            .order(egui::Order::Foreground)
            .fixed_pos(input_rect.left_top() - Vec2::new(0.0, SPACE_XS))
            .pivot(egui::Align2::LEFT_BOTTOM)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(input_rect.width().max(220.0));
                    for (k, c) in cands.iter().enumerate() {
                        let on = k == state.pick.min(cands.len() - 1);
                        let r = ui.add(
                            egui::Button::selectable(
                                on,
                                egui::RichText::new(format!("{}  ", c.label)).strong(),
                            )
                            .right_text(
                                RichText::new(&c.detail)
                                    .small()
                                    .color(theme::rgb(theme.text_muted)),
                            )
                            .min_size(Vec2::new(ui.available_width(), 0.0)),
                        );
                        if r.clicked() {
                            accept = Some(k);
                        }
                        if r.hovered() {
                            state.pick = k;
                        }
                    }
                    ui.label(
                        RichText::new("↑↓ choose · Enter insert · Esc close")
                            .small()
                            .color(theme::rgb(theme.text_faint)),
                    );
                });
            });
        let _ = area;
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

    // The bottom row.
    let blocked = blocked_reason(state);
    ui.horizontal(|ui| {
        add_menu(ctx, state, ui);
        let current = model_id(state);
        let label = MODELS
            .iter()
            .find(|(m, _)| *m == current)
            .map_or(current.as_str(), |(_, l)| l);
        egui::ComboBox::from_id_salt("assistant_model")
            .selected_text(RichText::new(label).small())
            .width(130.0)
            .show_ui(ui, |ui| {
                for (m, l) in MODELS {
                    if ui.selectable_label(current == m, l).clicked()
                        && let Some(prefs) = state.prefs.as_mut()
                    {
                        prefs.model = Some(m.to_owned());
                        state.error = prefs.save().err().map(|e| format!("{e:#}"));
                    }
                }
            });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(turn) = &state.turn {
                if widgets::button(ui, theme, "Stop", widgets::Emphasis::Secondary).clicked() {
                    turn.cancel.store(true, Ordering::Relaxed);
                }
                ui.add(egui::Spinner::new());
                ui.label(
                    RichText::new(format!("{:.0}s", turn.started.elapsed().as_secs_f32()))
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
            } else {
                let can = blocked.is_none() && !state.input.trim().is_empty();
                let r = ui.add_enabled_ui(can, |ui| {
                    widgets::button(ui, theme, "Send", widgets::Emphasis::Primary)
                });
                let r = r
                    .inner
                    .on_hover_text("Enter to send · Shift+Enter for a new line");
                if r.clicked() {
                    send = Some(state.input.clone());
                }
            }
        });
    });
    if let Some(reason) = &blocked {
        ui.label(
            RichText::new(reason)
                .small()
                .color(theme::rgb(theme.warning)),
        );
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

/// The reference chips above the input.
fn chips(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut Ui,
    refs: &[(Reference, complete::Origin)],
) {
    if refs.is_empty() {
        return;
    }
    let theme = ctx.theme;
    let mut cycle: Option<usize> = None;
    let mut remove: Option<usize> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(SPACE_XS);
        for (k, (r, origin)) in refs.iter().enumerate() {
            let facts = ctx
                .project
                .as_ref()
                .map(|p| {
                    crate::refs::resolve(r, p)
                        .map_or_else(|e| format!("unresolved: {e}"), |x| x.facts)
                })
                .unwrap_or_default();
            let glyph = if r.modifier.prefix().is_empty() {
                "·"
            } else {
                r.modifier.prefix()
            };
            let fill = match r.modifier {
                Modifier::Avoid => theme.danger,
                Modifier::Keep => theme.success,
                Modifier::Hero => theme.warning,
                Modifier::About | Modifier::Replace | Modifier::Like => theme.accent,
            };
            egui::Frame::new()
                .fill(theme::translucent(fill, 40))
                .stroke(Stroke::new(1.0, theme::translucent(fill, 140)))
                .corner_radius(CornerRadius::same(theme::RADIUS))
                .inner_margin(theme::margin_xy(SPACE_XS + 2.0, 1.0))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    let typed = matches!(origin, complete::Origin::Typed);
                    let chip = ui
                        .add(
                            egui::Label::new(
                                RichText::new(format!("{glyph} {}", r.short()))
                                    .small()
                                    .color(theme::rgb(theme.text)),
                            )
                            .sense(Sense::click()),
                        )
                        .on_hover_text(format!(
                            "{r}\n{} · {facts}\n{}",
                            r.modifier.label(),
                            if typed {
                                "typed in the message"
                            } else {
                                "click to change intent"
                            }
                        ));
                    if chip.clicked() && !typed {
                        cycle = Some(k);
                    }
                    if ui
                        .add(
                            egui::Label::new(
                                RichText::new("×")
                                    .small()
                                    .color(theme::rgb(theme.text_muted)),
                            )
                            .sense(Sense::click()),
                        )
                        .on_hover_text("Remove")
                        .clicked()
                    {
                        remove = Some(k);
                    }
                });
        }
        if !ctx.refs.is_empty()
            && ui
                .add(
                    egui::Label::new(
                        RichText::new("clear all")
                            .small()
                            .color(theme::rgb(theme.text_muted)),
                    )
                    .sense(Sense::click()),
                )
                .clicked()
        {
            ctx.actions.push(Action::ClearRefs);
        }
    });
    if let Some(k) = cycle
        && let Some((_, complete::Origin::Chip(i))) = refs.get(k)
        && let Some(r) = ctx.refs.get_mut(*i)
    {
        let at = Modifier::ALL
            .iter()
            .position(|m| *m == r.modifier)
            .unwrap_or(0);
        r.modifier = Modifier::ALL[(at + 1) % Modifier::ALL.len()];
    }
    if let Some(k) = remove
        && let Some((r, origin)) = refs.get(k)
    {
        match origin {
            complete::Origin::Chip(i) => {
                if *i < ctx.refs.len() {
                    ctx.refs.remove(*i);
                }
            }
            complete::Origin::Typed => state.input = complete::remove_token(&state.input, r),
        }
    }
}

/// The `+` menu: add the selection, the plan, a shot, a person, some bars.
fn add_menu(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let src = sources(ctx, state);
    let selection = ctx.selection.clone();
    let mut add: Vec<Reference> = Vec::new();
    ui.menu_button("+", |ui| {
        if ui
            .add_enabled(
                !selection.is_empty(),
                egui::Button::new(format!("Selection ({})", selection.len())),
            )
            .clicked()
        {
            add.extend(selection.iter().cloned());
            ui.close();
        }
        if let Some(chat) = &state.chat
            && ui.button(format!("This plan ({})", chat.plan)).clicked()
        {
            add.push(Reference::new(Target::Plan {
                name: chat.plan.clone(),
            }));
            ui.close();
        }
        ui.menu_button("Shot", |ui| {
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
        ui.menu_button("Person", |ui| {
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
        ui.menu_button("Bars", |ui| {
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
        ui.menu_button("Plan", |ui| {
            for name in &src.plans {
                if ui.button(name).clicked() {
                    add.push(Reference::new(Target::Plan { name: name.clone() }));
                    ui.close();
                }
            }
        });
    })
    .response
    .on_hover_text("Add a reference: the selection, a shot, a person, bars or a plan");
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

/// Sends a message and starts the turn on a worker thread.
fn submit(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    egui_ctx: &egui::Context,
    text: &str,
    choice: ScopeChoice,
) {
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

    // The provider.
    let model = prefs
        .model
        .clone()
        .unwrap_or_else(|| MODELS[0].0.to_owned());
    let provider: Box<dyn provider::Provider> = if model == "local" {
        Box::new(provider::Local { url: prefs.local() })
    } else {
        let Some(key) = prefs.key() else {
            state.error = blocked_reason(&State {
                prefs: Some(prefs),
                ..State::default()
            });
            return;
        };
        Box::new(provider::Claude {
            key,
            model,
            base: std::env::var("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|_| "https://api.anthropic.com".to_owned()),
        })
    };

    // The snapshot sent with the message.
    let index = project.index.as_ref();
    let music = project.music.as_ref();
    let problems = ops::check(&plan, index, music, &project.people);
    let plan_text = context::plan_text(&plan, music);
    let fresh = chat.sent_plan.as_deref() != Some(plan_text.as_str());
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
    };
    chat.sent_plan = Some(plan_text);

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
        text: context::snapshot(&snapshot),
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
    blocks.push(Block::Text { text: said });
    let msg = Msg::new(Role::User, blocks);
    if let Some(e) = chat.push(LogLine::Msg { msg }) {
        state.error = Some(e);
    }

    // The worker.
    let env = tools::Env {
        plan,
        log: index
            .map(|i| context::ShotLog::new(i, &project.people))
            .unwrap_or_default(),
        index: index.map(|i| Arc::new(i.clone())),
        music: music.cloned(),
        people: project.people.clone(),
        scope,
    };
    let job = agent::Job {
        provider,
        system: context::SYSTEM.to_owned(),
        tools: tools::definitions(),
        msgs: model::messages(&chat.log),
        env,
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let repaint = egui_ctx.clone();
    let spawned = std::thread::Builder::new()
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
        });
    match spawned {
        Ok(_) => {
            state.turn = Some(Turn {
                rx,
                cancel,
                started: Instant::now(),
                text: String::new(),
                thinking: String::new(),
                tools: Vec::new(),
            });
            state.input.clear();
            state.dismissed = None;
            if !ctx.refs.is_empty() {
                ctx.actions.push(Action::ClearRefs);
            }
        }
        Err(e) => state.error = Some(format!("could not start the assistant: {e}")),
    }
}

/// The conversation.
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
            | LogLine::Notice { .. } => None,
        })
        .collect();
    let log = chat.log.clone();
    let open_ask = chat.ask.as_ref().map(|a| a.id.clone());
    let mut send: Option<String> = None;

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if log.is_empty() && state.turn.is_none() {
                empty_state(ctx, ui, &mut send);
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
                                        thought(ctx, ui, ("thought", i, k), thinking, false);
                                    }
                                    Block::Text { text } => markdown_label(ctx, ui, text),
                                    Block::ToolUse { id, name, input } => {
                                        if name == tools::PROPOSE
                                            && let Some(p) = proposals.get(id)
                                        {
                                            proposal_card(ctx, state, ui, p, &log);
                                        } else if name == tools::ASK {
                                            let q = input["question"].as_str().unwrap_or_default();
                                            ui.add_space(SPACE_XS);
                                            ui.label(
                                                RichText::new(q)
                                                    .strong()
                                                    .color(theme::rgb(theme.text)),
                                            );
                                            if open_ask.as_deref() == Some(id.as_str()) {
                                                widgets::muted(
                                                    ui,
                                                    theme,
                                                    "Pick an answer below, or type your own.",
                                                );
                                            }
                                        } else {
                                            tool_line(
                                                ctx,
                                                ui,
                                                ("tool", i, k),
                                                name,
                                                input,
                                                results.get(id),
                                            );
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
                    } => {
                        let cached = if *input_tokens > 0 {
                            format!(
                                " ({:.0}% cached)",
                                *cached_tokens as f32 * 100.0 / *input_tokens as f32
                            )
                        } else {
                            String::new()
                        };
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            ui.label(
                                RichText::new(format!(
                                    "{secs:.1}s · {model} · {} in{cached} · {} out",
                                    kilo(*input_tokens),
                                    kilo(*output_tokens)
                                ))
                                .small()
                                .color(theme::rgb(theme.text_faint)),
                            );
                        });
                        ui.add_space(SPACE_SM);
                    }
                    LogLine::Notice { text, error } => {
                        ui.label(RichText::new(text).small().color(theme::rgb(if *error {
                            theme.danger
                        } else {
                            theme.text_muted
                        })));
                    }
                    LogLine::Proposal { .. } | LogLine::Status { .. } => {}
                }
            }
            if let Some(turn) = &state.turn {
                if !turn.thinking.is_empty() {
                    thought(ctx, ui, "live_thought", &turn.thinking, true);
                }
                for t in &turn.tools {
                    ui.label(
                        RichText::new(format!("▸ {t}…"))
                            .small()
                            .color(theme::rgb(theme.text_muted)),
                    );
                }
                if !turn.text.is_empty() {
                    markdown_label(ctx, ui, &turn.text);
                }
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(12.0));
                    let what = if turn.text.is_empty() && turn.thinking.is_empty() {
                        "Thinking"
                    } else {
                        "Working"
                    };
                    ui.label(
                        RichText::new(format!(
                            "{what}… {:.0}s",
                            turn.started.elapsed().as_secs_f32()
                        ))
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                    );
                });
            }
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
}

/// `1.2k`.
fn kilo(n: u64) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f32 / 1000.0)
    } else {
        n.to_string()
    }
}

/// What an empty conversation shows: what the assistant can do.
fn empty_state(ctx: &PageCtx<'_>, ui: &mut Ui, send: &mut Option<String>) {
    let theme = ctx.theme;
    ui.add_space(SPACE_SM);
    ui.label(RichText::new("Edit this plan by talking about it.").strong());
    widgets::muted(
        ui,
        theme,
        "Every change comes back as a card you apply or discard; nothing is edited behind your back. \
         Select shots and press R, or type @, to point at them.",
    );
    ui.add_space(SPACE_SM);
    for s in [
        "Fix everything the check flags, changing as little as possible.",
        "Put the strongest fire shot on the drop.",
        "The opening is weak: find a better first shot.",
        "Which shots repeat a performer too soon?",
    ] {
        if ui
            .add(egui::Button::new(RichText::new(s).small()).wrap())
            .clicked()
        {
            *send = Some(s.to_owned());
        }
    }
}

/// A user message.
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
    ui.add_space(SPACE_SM);
    ui.with_layout(Layout::top_down(Align::Max), |ui| {
        egui::Frame::new()
            .fill(theme::rgb(theme.accent_soft))
            .corner_radius(CornerRadius::same(theme::RADIUS_LG))
            .inner_margin(theme::margin_xy(SPACE_SM + 2.0, SPACE_XS + 2.0))
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width() * 0.88);
                ui.add(egui::Label::new(RichText::new(&text).color(theme::rgb(theme.text))).wrap());
            });
        let when = model::stamp(msg.ts);
        let note = if answered {
            format!("{when} · answer")
        } else {
            when
        };
        ui.label(
            RichText::new(note)
                .small()
                .color(theme::rgb(theme.text_faint)),
        );
    });
    ui.add_space(SPACE_XS);
}

/// A collapsible thinking summary.
fn thought(ctx: &PageCtx<'_>, ui: &mut Ui, id: impl egui::AsIdSalt, text: &str, live: bool) {
    let theme = ctx.theme;
    let first: String = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .chars()
        .take(60)
        .collect();
    egui::CollapsingHeader::new(
        RichText::new(format!(
            "{} {first}…",
            if live { "Thinking:" } else { "Thought:" }
        ))
        .small()
        .color(theme::rgb(theme.text_muted)),
    )
    .id_salt(id)
    .default_open(false)
    .show(ui, |ui| {
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .small()
                    .color(theme::rgb(theme.text_muted)),
            )
            .wrap(),
        );
    });
}

/// A collapsible tool call with its result.
fn tool_line(
    ctx: &PageCtx<'_>,
    ui: &mut Ui,
    id: impl egui::AsIdSalt,
    name: &str,
    input: &serde_json::Value,
    result: Option<&(String, bool)>,
) {
    let theme = ctx.theme;
    let failed = result.is_some_and(|(_, e)| *e);
    let label = format!(
        "{} {}",
        if failed { "✖" } else { "▸" },
        tools::describe(name, input)
    );
    egui::CollapsingHeader::new(RichText::new(label).small().color(theme::rgb(if failed {
        theme.warning
    } else {
        theme.text_muted
    })))
    .id_salt(id)
    .default_open(false)
    .show(ui, |ui| {
        let body = result.map_or("(no result)", |(c, _)| c.as_str());
        ui.add(
            egui::Label::new(
                RichText::new(body)
                    .small()
                    .monospace()
                    .color(theme::rgb(theme.text_muted)),
            )
            .wrap(),
        );
    });
}

/// The model's text, with bold, code and bullets.
fn markdown_label(ctx: &PageCtx<'_>, ui: &mut Ui, text: &str) {
    let theme = ctx.theme;
    let body = widgets::blend(theme.text, theme.text_muted, 0.25);
    let strong = theme::rgb(theme.text);
    let size = theme::BODY_SIZE - 1.0;
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
                    egui::FontId::monospace(size - 1.0)
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
    job.wrap.max_width = ui.available_width();
    ui.add_space(SPACE_XS);
    ui.add(egui::Label::new(job).wrap());
}

/// The card's state for proposal `id`, computed if needed.
fn card_for<'a>(ctx: &PageCtx<'_>, state: &'a mut State, id: &str) -> Option<&'a Card> {
    let chat = state.chat.as_ref()?;
    let proposal = chat.log.iter().find_map(|l| match l {
        LogLine::Proposal { proposal } if proposal.id == id => Some(proposal.clone()),
        LogLine::Msg { .. }
        | LogLine::Proposal { .. }
        | LogLine::Status { .. }
        | LogLine::Turn { .. }
        | LogLine::Notice { .. } => None,
    })?;
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
    let current = state.chat.as_mut().and_then(Chat::current);
    let stale = current.as_ref().is_some_and(|c| *c != p.base);
    let mut action: Option<CardAction> = None;
    let pending = matches!(status, Status::Pending);
    let stem = state
        .chat
        .as_ref()
        .map(|c| c.plan.clone())
        .unwrap_or_default();
    ui.add_space(SPACE_XS);
    egui::Frame::new()
        .fill(theme::rgb(theme.raised))
        .stroke(Stroke::new(
            1.0,
            theme::rgb(if pending { theme.accent } else { theme.border }),
        ))
        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(theme::margin(SPACE_SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("Proposal")
                        .small()
                        .strong()
                        .color(theme::rgb(theme.accent)),
                );
                ui.label(RichText::new(&p.summary).color(theme::rgb(theme.text)));
            });
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
                ui.separator();
                ui.horizontal(|ui| {
                    if pending {
                        ui.checkbox(&mut picked[k], "");
                    }
                    ui.label(RichText::new(op.label()).small().strong());
                });
                let effect = if picked[k] { eff.next() } else { None };
                match effect {
                    Some(e) => effect_row(ctx, ui, e, &stem),
                    None => {
                        ui.label(
                            RichText::new("(not included)")
                                .small()
                                .color(theme::rgb(theme.text_faint)),
                        );
                    }
                }
                if !op.reason().is_empty() && !matches!(op, EditOp::SetNote { .. }) {
                    ui.label(
                        RichText::new(op.reason())
                            .small()
                            .italics()
                            .color(theme::rgb(theme.text_muted)),
                    );
                }
            }
            ui.separator();
            let Some(card) = state.cards.get_mut(&p.id) else {
                return;
            };
            card.picked = picked;
            match (&card.result, &card.before, &card.after) {
                (Err(e), _, _) if !e.message.is_empty() => {
                    ui.label(
                        RichText::new(format!("Cannot apply: {e}"))
                            .small()
                            .color(theme::rgb(theme.danger)),
                    );
                }
                (Ok(_), Some(before), Some(after)) => {
                    let (new, fixed) = ops::compare(before, after);
                    let (text, color) = if after.is_empty() {
                        ("✔ check passes".to_owned(), theme.success)
                    } else if new.is_empty() {
                        (
                            format!("check: {} problems ({fixed} fixed, none new)", after.len()),
                            theme.text_muted,
                        )
                    } else {
                        (
                            format!(
                                "✖ check: {} new problem{} ({fixed} fixed)",
                                new.len(),
                                if new.len() == 1 { "" } else { "s" }
                            ),
                            theme.danger,
                        )
                    };
                    let r = ui.label(RichText::new(text).small().color(theme::rgb(color)));
                    if !new.is_empty() {
                        r.on_hover_text(
                            new.iter()
                                .map(|q| format!("shot {}: {}", q.shot, q.message))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        );
                        for q in new.iter().take(3) {
                            ui.label(
                                RichText::new(format!("  shot {}: {}", q.shot, q.message))
                                    .small()
                                    .color(theme::rgb(theme.danger)),
                            );
                        }
                    }
                }
                (Ok(_) | Err(_), _, _) => {
                    ui.label(
                        RichText::new("check unavailable (needs the index and music map)")
                            .small()
                            .color(theme::rgb(theme.text_faint)),
                    );
                }
            }
            let n = card.picked.iter().filter(|x| **x).count();
            let ok = card.result.is_ok() && n > 0;
            ui.horizontal_wrapped(|ui| match &status {
                Status::Pending => {
                    if stale {
                        ui.label(
                            RichText::new("The plan changed since this was proposed; ask again.")
                                .small()
                                .color(theme::rgb(theme.warning)),
                        );
                        if ui.small_button("Discard").clicked() {
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
                        .add_enabled_ui(ok, |ui| {
                            widgets::button(ui, theme, &label, widgets::Emphasis::Primary)
                        })
                        .inner
                        .clicked()
                    {
                        action = Some(CardAction::Apply);
                    }
                    if ui
                        .add_enabled(ok, egui::Button::new("▶ Preview"))
                        .on_hover_text("Play the edited plan in the viewer")
                        .clicked()
                    {
                        action = Some(CardAction::Preview);
                    }
                    if ui.button("Discard").clicked() {
                        action = Some(CardAction::Discard);
                    }
                }
                Status::Applied { version, ts, .. } => {
                    ui.label(
                        RichText::new(format!("✔ Applied as v{version} · {}", model::stamp(*ts)))
                            .small()
                            .color(theme::rgb(theme.success)),
                    );
                    if ui
                        .small_button("Revert")
                        .on_hover_text("Restore the plan as it was before this (as a new version)")
                        .clicked()
                    {
                        action = Some(CardAction::Revert);
                    }
                }
                Status::Discarded => {
                    ui.label(
                        RichText::new("Discarded")
                            .small()
                            .color(theme::rgb(theme.text_faint)),
                    );
                }
                Status::Reverted { version, .. } => {
                    ui.label(
                        RichText::new(format!("Reverted (v{version})"))
                            .small()
                            .color(theme::rgb(theme.text_faint)),
                    );
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
        ui.label(RichText::new("→").color(theme::rgb(theme.text_muted)));
        side(ui, e.after.as_ref(), ctx);
        ui.vertical(|ui| {
            let line = |s: Option<&(usize, Shot)>| {
                s.map_or_else(
                    || "—".to_owned(),
                    |(slot, sh)| format!("#{slot} {}@{:.1} {}b", sh.clip, sh.t_in, sh.beats),
                )
            };
            ui.label(
                RichText::new(line(e.before.as_ref()))
                    .small()
                    .color(theme::rgb(theme.text_muted)),
            );
            ui.label(
                RichText::new(line(e.after.as_ref()))
                    .small()
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

/// Applies the ticked ops of proposal `id`: a new plan version.
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
    let summary = chat
        .log
        .iter()
        .find_map(|l| match l {
            LogLine::Proposal { proposal } if proposal.id == id => Some(proposal.summary.clone()),
            LogLine::Msg { .. }
            | LogLine::Proposal { .. }
            | LogLine::Status { .. }
            | LogLine::Turn { .. }
            | LogLine::Notice { .. } => None,
        })
        .unwrap_or_default();
    match store::save_version(
        &chat.work,
        &chat.plan,
        &after,
        &format!("assistant: {summary}"),
    ) {
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
        Err(e) => state.error = Some(format!("could not save the plan: {e:#}")),
    }
}

/// Restores the plan a proposal started from, as a new version.
fn revert(ctx: &mut PageCtx<'_>, state: &mut State, p: &Proposal) {
    let Some(chat) = state.chat.as_mut() else {
        return;
    };
    match store::save_version(
        &chat.work,
        &chat.plan,
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
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    let (Some(music), Some(index)) = (project.music.as_ref(), project.index.as_ref()) else {
        return;
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
    ctx.player.play();
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
            ui.add_space(SPACE_XS);
            widgets::title(ui, theme, "Applied proposals");
            let mut any = false;
            for line in chat.log.iter().rev() {
                if let LogLine::Status { id, status } = line {
                    let summary = chat
                        .log
                        .iter()
                        .find_map(|l| match l {
                            LogLine::Proposal { proposal } if proposal.id == *id => {
                                Some(proposal.summary.clone())
                            }
                            LogLine::Msg { .. }
                            | LogLine::Proposal { .. }
                            | LogLine::Status { .. }
                            | LogLine::Turn { .. }
                            | LogLine::Notice { .. } => None,
                        })
                        .unwrap_or_default();
                    let text = match status {
                        Status::Applied { version, ops, ts } => format!(
                            "{} · v{version} · {} op{} · {summary}",
                            model::stamp(*ts),
                            ops.len(),
                            if ops.len() == 1 { "" } else { "s" }
                        ),
                        Status::Reverted { version, ts } => {
                            format!("{} · v{version} · reverted: {summary}", model::stamp(*ts))
                        }
                        Status::Pending | Status::Discarded => continue,
                    };
                    any = true;
                    ui.add(egui::Label::new(RichText::new(text).small()).wrap());
                }
            }
            if !any {
                widgets::muted(ui, theme, "Nothing applied yet.");
            }
            ui.add_space(SPACE_SM);
            widgets::title(ui, theme, &format!("Versions of {}", chat.plan));
            let versions = store::versions(&chat.work, &chat.plan);
            if versions.is_empty() {
                widgets::muted(
                    ui,
                    theme,
                    "No saved versions yet. Applying a proposal keeps the plan as it was as v1.",
                );
            }
            let last = versions.last().map(|v| v.n);
            for v in versions.iter().rev() {
                ui.horizontal_wrapped(|ui| {
                    let current = Some(v.n) == last;
                    ui.label(
                        RichText::new(format!("v{}", v.n))
                            .strong()
                            .color(theme::rgb(if current { theme.accent } else { theme.text })),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                v.header
                                    .split(" · ")
                                    .skip(1)
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                            )
                            .small()
                            .color(theme::rgb(theme.text_muted)),
                        )
                        .wrap(),
                    );
                    if ui
                        .small_button("▶")
                        .on_hover_text("Play this version")
                        .clicked()
                    {
                        play_version = Some(v.path.clone());
                    }
                    if !current
                        && ui
                            .small_button("Restore")
                            .on_hover_text("Make this the current plan (saved as a new version)")
                            .clicked()
                    {
                        restore = Some((v.n, v.path.clone()));
                    }
                });
            }
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

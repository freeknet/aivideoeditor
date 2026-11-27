// ============================================================================
// Module:       gui::assistant::chats
// Description:  The night's chats in the panel: the Chats menu (new chat,
//               recent chats with plan and time, Load older, rename, archive,
//               delete), the header title, the tab strip of open chats, and
//               switching between them.
//
// Dependencies: egui; crate::gui::ui::{icons, keymap, modal, theme, widgets,
//               workspace}; super::store (the chat index)
// ============================================================================

//! Chats.
//!
//! The look is Selects' (`docs/research/07-selects-visual-chat-project.md`
//! §3.1): a 32 px Chats button opens a 320 px list headed by a ghost "New
//! chat" button; each row is the title in 14 px and, under it, the plan it
//! is about and how long ago it changed. The current chat's title fills the
//! header (double-click to rename). A panel wide enough also shows the open
//! chats as a strip of 28 px tabs with a close cross; right-click a tab for
//! "Close other tabs", rename and archive.
//!
//! Switching chats waits for the turn to finish: a streaming reply belongs
//! to the chat that asked. Each chat has its own Claude Code session, so a
//! switch ends the running process and the next message resumes the
//! chat's own session.

use crate::gui::ui::hint::Hint as _;
use egui::{Align, CornerRadius, Layout, Rect, Sense, Stroke, Ui, Vec2};

use super::{PAGE_KEY, PageCtx, State, Tab, find, open_chat, store};
use crate::gui::ui::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::gui::ui::{icons, keymap, modal, widgets, workspace};
use crate::theme::Palette;

/// How many chats the list shows at first, and how many more "Load older"
/// adds.
const PAGE: usize = 12;

/// The narrowest panel that shows the tab strip.
const STRIP_MIN_WIDTH: f32 = 300.0;

/// A chat being renamed.
#[derive(Clone, Debug)]
pub(super) struct Rename {
    /// The chat.
    pub(super) id: String,
    /// The title being typed.
    pub(super) draft: String,
    /// Give the field the keyboard next frame.
    focus: bool,
}

impl Rename {
    /// Starts renaming `meta`.
    fn of(meta: &store::ChatMeta) -> Self {
        Self {
            id: meta.id.clone(),
            draft: meta.title.clone(),
            focus: true,
        }
    }
}

/// The panel's chat chords, while the chat is the focused panel: New chat
/// and Find in chat.
pub(super) fn keys(ctx: &mut PageCtx<'_>, state: &mut State, ui: &Ui) {
    if ctx.blocked || !workspace::panel_focused(ui.ctx(), "chat") {
        return;
    }
    let preset = ctx.keymap;
    let (new, find) = ui.ctx().input_mut(|i| {
        (
            keymap::consume(i, "chat-new", preset),
            keymap::consume(i, "chat-find", preset),
        )
    });
    if new && let Some(plan) = state.chat.as_ref().map(|c| c.plan.clone()) {
        new_chat(ctx, state, &plan);
    }
    if find {
        open_find(state);
    }
}

/// Opens the find bar (or gives it the keyboard again).
pub(super) fn open_find(state: &mut State) {
    state.tab = Tab::Chat;
    match state.find.as_mut() {
        Some(f) => f.focus = true,
        None => state.find = Some(find::Find::open()),
    }
}

/// Writes the open chat into the index, stamped now. A chat is listed from
/// its first message (or its first rename) on.
pub(super) fn remember(state: &mut State) {
    let (Some(chat), Some((_, index))) = (state.chat.as_mut(), state.chats.as_mut()) else {
        return;
    };
    if chat.log.is_empty() && index.get(chat.id()).is_none() {
        return;
    }
    chat.meta.updated = super::model::now();
    index.upsert(&chat.meta);
    if let Err(e) = store::save_index(&chat.work, index) {
        state.error = Some(format!("could not save the chat list: {e:#}"));
    }
}

/// Before the first message of a chat goes out: titles it after the
/// message (unless the person named it) and lists it.
pub(super) fn first_message(state: &mut State, text: &str) {
    let Some(chat) = state.chat.as_mut() else {
        return;
    };
    let said_before = chat.log.iter().any(
        |l| matches!(l, super::model::LogLine::Msg { msg } if msg.role == super::model::Role::User),
    );
    if !said_before && !chat.meta.renamed {
        chat.meta.title = store::auto_title(text);
    }
    if chat.log.is_empty() {
        // Listed before its log exists, so a crash cannot orphan the log.
        if let Some((_, index)) = state.chats.as_mut() {
            index.upsert(&chat.meta);
        }
    }
    remember(state);
}

/// Saves which chat is open, its plan and the open tabs, for reopening the
/// night.
fn save_open(ctx: &mut PageCtx<'_>, state: &mut State) {
    let Some(chat) = state.chat.as_ref() else {
        return;
    };
    let Some(project) = ctx.project.as_mut() else {
        return;
    };
    let table = project
        .settings
        .pages
        .entry(PAGE_KEY.to_owned())
        .or_default();
    table.insert("chat".to_owned(), toml::Value::String(chat.id().to_owned()));
    table.insert("plan".to_owned(), toml::Value::String(chat.plan.clone()));
    table.insert(
        "tabs".to_owned(),
        toml::Value::Array(
            state
                .tabs
                .iter()
                .map(|t| toml::Value::String(t.clone()))
                .collect(),
        ),
    );
    if let Err(e) = project.save_settings() {
        state.error = Some(format!("could not save ave.toml: {e:#}"));
    }
}

/// Opens chat `meta` (and its tab). Nothing happens while a turn runs.
pub(super) fn activate(ctx: &mut PageCtx<'_>, state: &mut State, meta: store::ChatMeta) {
    if state.turn.is_some() {
        return;
    }
    if state.chat.as_ref().is_some_and(|c| c.id() == meta.id) {
        state.tab = Tab::Chat;
        return;
    }
    let Some(work) = ctx.project.as_ref().map(|p| p.work.clone()) else {
        return;
    };
    // The running process served the old chat; this one resumes its own.
    state.session = None;
    if !state.tabs.contains(&meta.id) {
        let at = state
            .chat
            .as_ref()
            .and_then(|c| state.tabs.iter().position(|t| t == c.id()))
            .map_or(state.tabs.len(), |k| k + 1);
        state.tabs.insert(at, meta.id.clone());
    }
    // An untouched new chat that loses focus is gone, and so is its tab.
    if let Some(old) = state.chat.as_ref()
        && old.log.is_empty()
        && state
            .chats
            .as_ref()
            .is_some_and(|(_, index)| index.get(old.id()).is_none())
    {
        let gone = old.id().to_owned();
        state.tabs.retain(|t| *t != gone);
    }
    state.chat = Some(open_chat(&work, meta));
    super::reopened(state);
    state.tab = Tab::Chat;
    state.recall = None;
    state.renaming = None;
    if let Some(f) = state.find.as_mut() {
        f.current = 0;
    }
    save_open(ctx, state);
}

/// Starts a new chat about `plan`; an empty new chat already open about it
/// is reused.
pub(super) fn new_chat(ctx: &mut PageCtx<'_>, state: &mut State, plan: &str) {
    if state.turn.is_some() {
        return;
    }
    if state
        .chat
        .as_ref()
        .is_some_and(|c| c.log.is_empty() && c.plan == plan && !c.meta.renamed)
    {
        state.tab = Tab::Chat;
        return;
    }
    let Some(index) = state.chats.as_ref().map(|(_, i)| i) else {
        return;
    };
    let meta = store::ChatMeta::fresh(index, plan);
    activate(ctx, state, meta);
}

/// The chat `id`'s entry: the index's, or the open chat's while unlisted.
fn meta_of(state: &State, id: &str) -> Option<store::ChatMeta> {
    state
        .chats
        .as_ref()
        .and_then(|(_, index)| index.get(id).cloned())
        .or_else(|| {
            state
                .chat
                .as_ref()
                .filter(|c| c.id() == id)
                .map(|c| c.meta.clone())
        })
}

/// After the open chat went away (archived, deleted, its tab closed):
/// opens the next tab, else the latest chat, else a new chat about the
/// same plan.
fn switch_away(ctx: &mut PageCtx<'_>, state: &mut State, from: usize) {
    let Some(plan) = state.chat.as_ref().map(|c| c.plan.clone()) else {
        return;
    };
    let stems = ctx
        .project
        .as_ref()
        .map(super::plan_stems)
        .unwrap_or_default();
    let current = state.chat.as_ref().map(|c| c.id().to_owned());
    // The chat being left loses its tab; the one after it takes its place.
    if let Some(id) = &current {
        state.tabs.retain(|t| t != id);
    }
    let neighbour = state
        .tabs
        .get(from.min(state.tabs.len().saturating_sub(1)))
        .cloned()
        .and_then(|t| meta_of(state, &t));
    let latest = || {
        state.chats.as_ref().and_then(|(_, index)| {
            index
                .recent(false)
                .into_iter()
                .find(|m| Some(m.id.as_str()) != current.as_deref() && stems.contains(&m.plan))
                .cloned()
        })
    };
    let next = neighbour.or_else(latest).or_else(|| {
        state
            .chats
            .as_ref()
            .map(|(_, index)| store::ChatMeta::fresh(index, &plan))
    });
    if let Some(meta) = next {
        activate(ctx, state, meta);
    }
}

/// Closes chat `id`'s tab (the chat stays in the list).
fn close_tab(ctx: &mut PageCtx<'_>, state: &mut State, id: &str) {
    let Some(at) = state.tabs.iter().position(|t| t == id) else {
        return;
    };
    let active = state.chat.as_ref().is_some_and(|c| c.id() == id);
    if active {
        if state.turn.is_some() {
            return;
        }
        state.tabs.remove(at);
        switch_away(ctx, state, at);
    } else {
        state.tabs.remove(at);
        save_open(ctx, state);
    }
}

/// Closes every tab but chat `id`'s.
fn close_others(ctx: &mut PageCtx<'_>, state: &mut State, id: &str) {
    if state.turn.is_some() && state.chat.as_ref().is_some_and(|c| c.id() != id) {
        return;
    }
    if let Some(meta) = meta_of(state, id) {
        state.tabs = vec![id.to_owned()];
        activate(ctx, state, meta);
        save_open(ctx, state);
    }
}

/// Archives (or brings back) chat `id`.
pub(super) fn archive(ctx: &mut PageCtx<'_>, state: &mut State, id: &str, on: bool) {
    let active = state.chat.as_ref().is_some_and(|c| c.id() == id);
    if active && on && state.turn.is_some() {
        return;
    }
    let Some(mut meta) = meta_of(state, id) else {
        return;
    };
    meta.archived = on;
    if let Some(chat) = state.chat.as_mut().filter(|c| c.id() == id) {
        chat.meta.archived = on;
    }
    let Some(work) = ctx.project.as_ref().map(|p| p.work.clone()) else {
        return;
    };
    if let Some((_, index)) = state.chats.as_mut() {
        index.upsert(&meta);
        if let Err(e) = store::save_index(&work, index) {
            state.error = Some(format!("could not save the chat list: {e:#}"));
        }
    }
    if on {
        let at = state.tabs.iter().position(|t| t == id).unwrap_or(0);
        if active {
            switch_away(ctx, state, at);
        } else {
            state.tabs.retain(|t| t != id);
            save_open(ctx, state);
        }
    }
}

/// Deletes chat `id` for good: its log, its session, its entry.
pub(super) fn delete(ctx: &mut PageCtx<'_>, state: &mut State, id: &str) {
    let active = state.chat.as_ref().is_some_and(|c| c.id() == id);
    if active && state.turn.is_some() {
        return;
    }
    let Some(meta) = meta_of(state, id) else {
        return;
    };
    let Some(work) = ctx.project.as_ref().map(|p| p.work.clone()) else {
        return;
    };
    if let Err(e) = store::delete_chat(&work, &meta) {
        state.error = Some(format!("could not delete the chat: {e:#}"));
        return;
    }
    if let Some((_, index)) = state.chats.as_mut() {
        index.chats.retain(|c| c.id != id);
        if let Err(e) = store::save_index(&work, index) {
            state.error = Some(format!("could not save the chat list: {e:#}"));
        }
    }
    let at = state.tabs.iter().position(|t| t == id).unwrap_or(0);
    if active {
        state.session = None;
        if let Some(chat) = state.chat.as_mut() {
            // Nothing of it is left to keep.
            chat.log.clear();
        }
        switch_away(ctx, state, at);
    } else {
        state.tabs.retain(|t| t != id);
        save_open(ctx, state);
    }
}

/// Renames chat `id` to `title` (blank: no change). A named chat is never
/// auto-titled again, and is listed even before its first message.
pub(super) fn rename(ctx: &PageCtx<'_>, state: &mut State, id: &str, title: &str) {
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return;
    }
    let Some(mut meta) = meta_of(state, id) else {
        return;
    };
    meta.title.clone_from(&title);
    meta.renamed = true;
    if let Some(chat) = state.chat.as_mut().filter(|c| c.id() == id) {
        chat.meta.title = title;
        chat.meta.renamed = true;
    }
    let Some(work) = ctx.project.as_ref().map(|p| p.work.clone()) else {
        return;
    };
    if let Some((_, index)) = state.chats.as_mut() {
        index.upsert(&meta);
        if let Err(e) = store::save_index(&work, index) {
            state.error = Some(format!("could not save the chat list: {e:#}"));
        }
    }
}

/// The rename field for `state.renaming`, in `rect`. Enter or leaving the
/// field keeps the title; Escape drops it.
fn rename_field(ctx: &PageCtx<'_>, state: &mut State, ui: &mut Ui, rect: Rect) {
    let Some(r) = state.renaming.as_mut() else {
        return;
    };
    let field = widgets::field_frame(ui, ctx.theme, rect, &mut r.draft, "Chat title", None);
    if r.focus {
        field.request_focus();
        r.focus = false;
    }
    if field.lost_focus() {
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if let Some(done) = state.renaming.take()
            && !escape
        {
            rename(ctx, state, &done.id, &done.draft);
        }
    }
}

/// The 32 px Chats button and its list.
pub(super) fn menu(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    let id = ui.make_persistent_id("assistant-chats");
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    state.list_open = open;
    // Another chat's rename lives in the list: it ends when the list closes.
    let elsewhere = state
        .renaming
        .as_ref()
        .is_some_and(|r| state.chat.as_ref().is_none_or(|c| c.id() != r.id));
    if !open && elsewhere {
        state.renaming = None;
    }
    let hint = keymap::command("chat-new").map_or_else(String::new, |c| c.hint(ctx.keymap));
    let button = widgets::icon_toggle_sized(ui, theme, icons::MENU, "Chats", "", open, 32.0);
    // The response is not needed: drawing it is the point.
    let _ = egui::Popup::menu(&button)
        .id(id)
        .width(320.0)
        .gap(SPACE_XS)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| list(ctx, state, ui, &hint));
}

/// What a row of the list asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowAction {
    /// Open it.
    Open,
    /// Rename it.
    Rename,
    /// Archive it (or bring it back).
    Archive,
    /// Delete it.
    Delete,
}

/// The chat list: New chat, the recent (or archived) chats, Load older.
fn list(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui, new_keys: &str) {
    let theme = ctx.theme;
    let idle = state.turn.is_none();
    let stems = ctx
        .project
        .as_ref()
        .map(super::plan_stems)
        .unwrap_or_default();
    let current = state.chat.as_ref().map(|c| c.meta.clone());
    let mut new_on: Option<String> = None;
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.horizontal(|ui| {
        ui.add_space(SPACE_SM);
        let heading = if state.show_archived {
            "Archived chats"
        } else {
            "Recent chats"
        };
        widgets::menu_heading(ui, theme, heading);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = ui
                .add_enabled_ui(idle, |ui| {
                    widgets::button_with(
                        ui,
                        theme,
                        widgets::ButtonKind::Ghost,
                        Some(icons::PLUS),
                        "New chat",
                    )
                })
                .inner;
            let about = current.as_ref().map_or("", |c| c.plan.as_str());
            if widgets::tip(
                r,
                theme,
                &format!("A fresh conversation about {about}"),
                new_keys,
            )
            .clicked()
            {
                new_on = current.as_ref().map(|c| c.plan.clone());
                ui.close();
            }
        });
    });
    if stems.len() > 1 {
        ui.add_enabled_ui(idle, |ui| {
            ui.menu_button("New chat about another cut", |ui| {
                for s in &stems {
                    if widgets::menu_item(ui, theme, None, s, "", false).clicked() {
                        new_on = Some(s.clone());
                        ui.close();
                    }
                }
            });
        });
    }
    widgets::menu_separator(ui, theme);

    let (rows, total) = {
        let listed = state
            .chats
            .as_ref()
            .map(|(_, index)| index.recent(state.show_archived))
            .unwrap_or_default();
        let mut rows: Vec<store::ChatMeta> = Vec::new();
        // The open chat heads the list until it is listed itself.
        if !state.show_archived
            && let Some(c) = &current
            && !listed.iter().any(|m| m.id == c.id)
        {
            rows.push(c.clone());
        }
        rows.extend(listed.into_iter().cloned());
        let total = rows.len();
        let shown = if state.list_len == 0 {
            PAGE
        } else {
            state.list_len
        };
        rows.truncate(shown);
        (rows, total)
    };
    let mut action: Option<(RowAction, store::ChatMeta)> = None;
    let now = super::model::now();
    egui::ScrollArea::vertical()
        .max_height((ui.ctx().content_rect().height() * 0.7).max(480.0) - 120.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            if rows.is_empty() {
                ui.add_space(SPACE_SM);
                ui.horizontal(|ui| {
                    ui.add_space(SPACE_SM);
                    widgets::muted(
                        ui,
                        theme,
                        if state.show_archived {
                            "No archived chats"
                        } else {
                            "No previous chats"
                        },
                    );
                });
                ui.add_space(SPACE_SM);
            }
            for meta in &rows {
                let active = current.as_ref().is_some_and(|c| c.id == meta.id);
                if state.renaming.as_ref().is_some_and(|r| r.id == meta.id) {
                    let (rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), theme::CONTROL_HEIGHT + SPACE_SM),
                        Sense::hover(),
                    );
                    rename_field(ctx, state, ui, rect.shrink2(Vec2::new(SPACE_XS, 2.0)));
                    continue;
                }
                if let Some(a) = row(ui, theme, meta, active, now, idle || active) {
                    action = Some((a, meta.clone()));
                }
            }
        });
    if total > rows.len() {
        ui.add_space(SPACE_XS);
        ui.vertical_centered(|ui| {
            if widgets::outline_button(ui, theme, "Load older").clicked() {
                state.list_len = rows.len() + PAGE;
            }
        });
        ui.add_space(SPACE_XS);
    }
    widgets::menu_separator(ui, theme);
    let archived = state
        .chats
        .as_ref()
        .map_or(0, |(_, index)| index.recent(true).len());
    let toggle = if state.show_archived {
        "Back to recent chats".to_owned()
    } else {
        format!("Archived chats ({archived})")
    };
    if (archived > 0 || state.show_archived)
        && widgets::menu_item(ui, theme, None, &toggle, "", false).clicked()
    {
        state.show_archived = !state.show_archived;
        state.list_len = 0;
    }

    if let Some(plan) = new_on {
        new_chat(ctx, state, &plan);
    }
    match action {
        Some((RowAction::Open, meta)) => {
            if meta.archived {
                archive(ctx, state, &meta.id, false);
                state.show_archived = false;
            }
            activate(ctx, state, meta);
            ui.close();
        }
        Some((RowAction::Rename, meta)) => state.renaming = Some(Rename::of(&meta)),
        Some((RowAction::Archive, meta)) => archive(ctx, state, &meta.id, !meta.archived),
        Some((RowAction::Delete, meta)) => {
            state.deleting = Some(meta.id.clone());
            ui.close();
        }
        None => {}
    }
}

/// One chat in the list: the title in 14 px; the plan and the time under
/// it; rename, archive and delete at the row's end. Returns
/// what was asked for.
fn row(
    ui: &mut Ui,
    theme: &Palette,
    meta: &store::ChatMeta,
    active: bool,
    now: u64,
    enabled: bool,
) -> Option<RowAction> {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 52.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, enabled, active, &meta.title)
    });
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    if active || lift > 0.0 {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS),
            theme::accent_fill(theme).gamma_multiply(if active { 1.0 } else { lift }),
        );
    }
    let inner = rect.shrink2(Vec2::new(SPACE_MD, SPACE_SM));
    let painter = ui.painter();
    // The three 24 px actions, centred at the row's end; the text keeps
    // clear of them.
    let actions = Rect::from_min_max(
        egui::pos2(inner.right() - 24.0 * 3.0 - 4.0, inner.center().y - 12.0),
        egui::pos2(inner.right(), inner.center().y + 12.0),
    );
    let text_w = (actions.left() - inner.left() - SPACE_SM).max(0.0);
    let title = one_line(
        &meta.title,
        theme::font(theme::LARGE_SIZE),
        theme::rgb(theme.text),
        text_w,
    );
    painter.galley(
        inner.left_top(),
        painter.layout_job(title),
        theme::rgb(theme.text),
    );
    let plan = one_line(
        &format!("{} · {}", meta.plan, store::ago(now, meta.updated)),
        theme::font(theme::BODY_SIZE),
        look_faint(theme),
        text_w,
    );
    painter.galley(
        egui::pos2(inner.left(), inner.bottom() - 16.0),
        painter.layout_job(plan),
        theme::ring(theme),
    );
    let mut asked = None;
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(actions)
            .layout(Layout::right_to_left(Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 2.0;
    let named = |what: &str| format!("{what} {}", meta.title);
    if bar
        .add_enabled_ui(enabled, |ui| {
            widgets::icon_toggle_sized(ui, theme, icons::TRASH, &named("Delete"), "", false, 24.0)
        })
        .inner
        .clicked()
    {
        asked = Some(RowAction::Delete);
    }
    let (glyph, verb) = if meta.archived {
        (icons::EYE, "Unarchive")
    } else {
        (icons::EYE_OFF, "Archive")
    };
    if bar
        .add_enabled_ui(enabled, |ui| {
            widgets::icon_toggle_sized(ui, theme, glyph, &named(verb), "", false, 24.0)
        })
        .inner
        .clicked()
    {
        asked = Some(RowAction::Archive);
    }
    if widgets::icon_toggle_sized(
        &mut bar,
        theme,
        icons::PENCIL,
        &named("Rename"),
        "",
        false,
        24.0,
    )
    .clicked()
    {
        asked = Some(RowAction::Rename);
    }
    if asked.is_none() && enabled && response.clicked() {
        asked = Some(RowAction::Open);
    }
    asked
}

/// Selects' `tone-500`, for times and plan names.
fn look_faint(theme: &Palette) -> egui::Color32 {
    theme::rgb(theme.text_faint)
}

/// `text` on one line, cut with an ellipsis at `width`.
fn one_line(
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    width: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    job
}

/// The header's title: the open chat's (double-click, or the pen, to
/// rename), or the history's.
pub(super) fn title(ctx: &PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    let Some(chat) = state.chat.as_ref() else {
        return;
    };
    let meta = chat.meta.clone();
    if !state.list_open && state.renaming.as_ref().is_some_and(|r| r.id == meta.id) {
        let size = Vec2::new(ui.available_width().max(60.0), 24.0);
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        rename_field(ctx, state, ui, rect);
        return;
    }
    let text = match state.tab {
        Tab::Chat => meta.title.clone(),
        Tab::History => format!("History of {}", meta.plan),
    };
    ui.add_space(SPACE_XS);
    let width = (ui.available_width() - 24.0).max(0.0);
    let job = one_line(
        &text,
        theme::font(theme::BODY_SIZE),
        theme::rgb(theme.text_muted),
        width,
    );
    let label = ui
        .add(
            egui::Label::new(job)
                .selectable(false)
                .sense(Sense::click()),
        )
        .hint(format!("About {} · double-click to rename", meta.plan));
    let pen = ui.rect_contains_pointer(label.rect.expand2(Vec2::new(28.0, 0.0)));
    let mut start = label.double_clicked();
    if pen
        && widgets::icon_toggle_sized(ui, theme, icons::PEN, "Rename chat", "", false, 20.0)
            .clicked()
    {
        start = true;
    }
    if start {
        state.renaming = Some(Rename::of(&meta));
    }
}

/// The strip of open chats under the header, when there are two or more
/// and the panel is wide enough: 28 px tabs (max 140 px) with a close
/// cross, a spinner on the one streaming, then New chat.
pub(super) fn tab_strip(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    if state.tabs.len() < 2 || ui.available_width() < STRIP_MIN_WIDTH || state.chat.is_none() {
        return;
    }
    let theme = ctx.theme;
    let current = state
        .chat
        .as_ref()
        .map(|c| c.id().to_owned())
        .unwrap_or_default();
    let busy = state.turn.is_some();
    let tabs: Vec<store::ChatMeta> = state
        .tabs
        .iter()
        .filter_map(|id| meta_of(state, id))
        .collect();
    let mut open: Option<store::ChatMeta> = None;
    let mut close: Option<String> = None;
    let mut others: Option<String> = None;
    let mut rename_id: Option<store::ChatMeta> = None;
    let mut archive_id: Option<String> = None;
    let mut new = false;
    ui.add_space(SPACE_XS);
    ui.horizontal(|ui| {
        ui.set_height(theme::CONTROL_HEIGHT);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let r = ui
                .add_enabled_ui(!busy, |ui| {
                    widgets::icon_toggle_sized(ui, theme, icons::PLUS, "New chat", "", false, 28.0)
                })
                .inner;
            if r.clicked() {
                new = true;
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("assistant-chat-tabs")
                    .auto_shrink([false, true])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            for meta in &tabs {
                                let active = meta.id == current;
                                let out = chat_tab(ui, theme, meta, active, active && busy);
                                if out.open && !active {
                                    open = Some(meta.clone());
                                }
                                if out.close {
                                    close = Some(meta.id.clone());
                                }
                                out.response.context_menu(|ui| {
                                    if widgets::menu_item(ui, theme, None, "Close tab", "", false)
                                        .clicked()
                                    {
                                        close = Some(meta.id.clone());
                                        ui.close();
                                    }
                                    if widgets::menu_item(
                                        ui,
                                        theme,
                                        None,
                                        "Close other tabs",
                                        "",
                                        false,
                                    )
                                    .clicked()
                                    {
                                        others = Some(meta.id.clone());
                                        ui.close();
                                    }
                                    if widgets::menu_item(ui, theme, None, "Rename", "", false)
                                        .clicked()
                                    {
                                        rename_id = Some(meta.clone());
                                        ui.close();
                                    }
                                    if widgets::menu_item(ui, theme, None, "Archive", "", false)
                                        .clicked()
                                    {
                                        archive_id = Some(meta.id.clone());
                                        ui.close();
                                    }
                                });
                            }
                        });
                    });
            });
        });
    });
    if new && let Some(plan) = state.chat.as_ref().map(|c| c.plan.clone()) {
        new_chat(ctx, state, &plan);
    }
    if let Some(meta) = open {
        activate(ctx, state, meta);
    }
    if let Some(id) = close {
        close_tab(ctx, state, &id);
    }
    if let Some(id) = others {
        close_others(ctx, state, &id);
    }
    if let Some(meta) = rename_id {
        if meta.id != current {
            activate(ctx, state, meta.clone());
        }
        state.renaming = Some(Rename::of(&meta));
    }
    if let Some(id) = archive_id {
        archive(ctx, state, &id, true);
    }
}

/// What a tab was asked for.
struct TabOut {
    /// The tab itself (for its context menu).
    response: egui::Response,
    /// Clicked: open it.
    open: bool,
    /// The cross or a middle click: close it.
    close: bool,
}

/// One tab: padding 6 × 8, 12 px, radius 4, a 1 px border; the open one
/// `tone-600` bordered on `tone-800`, the others muted with a hover fill.
fn chat_tab(
    ui: &mut Ui,
    theme: &Palette,
    meta: &store::ChatMeta,
    active: bool,
    streaming: bool,
) -> TabOut {
    let font = theme::font(theme::BODY_SIZE);
    let color = if active {
        theme::rgb(theme.text)
    } else {
        theme::rgb(theme.text_muted)
    };
    let lead = if streaming { 10.0 + SPACE_XS } else { 0.0 };
    let label = one_line(&meta.title, font, color, 140.0 - 16.0 - 14.0 - lead);
    let galley = ui.painter().layout_job(label);
    let size = Vec2::new(
        (galley.size().x + lead + 16.0 + 14.0).min(140.0),
        theme::CONTROL_HEIGHT,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, active, &meta.title));
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    let radius = CornerRadius::same(4);
    if active {
        ui.painter()
            .rect_filled(rect, radius, theme::rgb(theme.raised));
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, theme::selected_row(theme)),
            egui::StrokeKind::Inside,
        );
    } else if lift > 0.0 {
        ui.painter().rect_filled(
            rect,
            radius,
            theme::rgb(theme.raised).gamma_multiply(0.6 * lift),
        );
    }
    let mut x = rect.left() + 8.0;
    if streaming {
        let spin = Rect::from_center_size(egui::pos2(x + 5.0, rect.center().y), Vec2::splat(10.0));
        ui.put(spin, widgets::spinner(10.0, theme::rgb(theme.text)));
        x += lead;
    }
    ui.painter().galley(
        egui::pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    let cross = Rect::from_center_size(
        egui::pos2(rect.right() - 8.0 - 5.0, rect.center().y),
        Vec2::splat(14.0),
    );
    let x_response = ui.interact(cross, response.id.with("close"), Sense::click());
    x_response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Close {}", meta.title))
    });
    let ink = if x_response.hovered() {
        theme::rgb(theme.text)
    } else {
        color.gamma_multiply(0.6)
    };
    icons::paint(ui.painter(), cross, icons::X, 10.0, ink);
    let close = x_response.clicked() || response.middle_clicked();
    TabOut {
        open: response.clicked() && !close,
        close,
        response,
    }
}

/// "Delete chat?": the confirmation before a chat's messages go for good.
pub(super) fn confirm_delete(ctx: &mut PageCtx<'_>, state: &mut State, ui: &Ui) {
    let theme = ctx.theme;
    let title = state
        .deleting
        .as_ref()
        .and_then(|id| meta_of(state, id))
        .map(|m| m.title)
        .unwrap_or_default();
    let outcome = modal::show(
        ui.ctx(),
        theme,
        "Delete chat?",
        state.deleting.is_some(),
        380.0,
        |ui| {
            widgets::muted(
                ui,
                theme,
                &format!(
                    "\u{201c}{title}\u{201d} and its messages are deleted for good. Plan versions it \
                     applied stay in the history."
                ),
            );
            ui.add_space(theme::SPACE_LG);
            let mut choice = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_SM;
                if widgets::button_with(
                    ui,
                    theme,
                    widgets::ButtonKind::Destructive,
                    Some(icons::TRASH),
                    "Delete",
                )
                .clicked()
                {
                    choice = Some(true);
                }
                if widgets::ghost_button(ui, theme, "Cancel").clicked() {
                    choice = Some(false);
                }
            });
            choice
        },
    );
    match outcome.inner.flatten() {
        Some(true) => {
            if let Some(id) = state.deleting.take() {
                delete(ctx, state, &id);
            }
        }
        Some(false) => state.deleting = None,
        None if outcome.dismissed => state.deleting = None,
        None => {}
    }
}

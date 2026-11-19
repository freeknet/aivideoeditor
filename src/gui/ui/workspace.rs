// ============================================================================
// Module:       gui::ui::workspace
// Description:  The editing workspace: top bar with search and export, chat on
//               the left, browser, viewer and inspector, timeline at the foot.
//
// Dependencies: egui; crate::gui::{app, pages, assistant}
// ============================================================================

//! The workspace layout.
//!
//! One window, laid out the way editing apps (Selects, Resolve's Cut page)
//! are, because the work moves between finding footage, watching it,
//! talking about it and placing it on the timeline without changing screens:
//!
//! ```text
//! ┌ top bar: home · night · [ search shots, captions, people… ] · export ┐
//! │ chat     │ browser   │        viewer          │ inspector            │
//! │ (refs,   │ Shots     │                        │ (selection's         │
//! │  composer│ People    │                        │  properties)         │
//! │  )       │ Music     │                        │                      │
//! │          ├───────────┴────────────────────────┴──────────────────────┤
//! │          │ timeline: V1 shots on the beat grid · A1 music · checks   │
//! └──────────┴───────────────────────────────────────────────────────────┘
//! ```
//!
//! The Home screen (pipeline, recent nights) replaces all of it when no
//! night is open or the home button is pressed; Export docks on the right.
//!
//! Selection is app-wide: whatever is selected in the browser, timeline or
//! inspector is a list of [`Reference`]s. `R` stages it for the chat;
//! `Shift-R` asks which intent (keep, avoid, replace, more like, feature).

use super::theme::{self, SPACE_MD, SPACE_SM};
use super::{paint_glyph, widgets};
use crate::gui::app::{Action, App, BrowserTab, Page};
use crate::gui::pages::PageCtx;
use crate::refs::{Modifier, Reference};
use egui::{Align, Layout, Ui};

/// Builds a [`PageCtx`] from the app's fields; used once per region.
macro_rules! page_ctx {
    ($app:expr, $actions:expr, $blocked:expr) => {
        PageCtx {
            theme: &$app.theme,
            project: &mut $app.project,
            jobs: &mut $app.jobs,
            thumbs: &mut $app.thumbs,
            player: &mut $app.player,
            config: &mut $app.config,
            refs: &mut $app.refs,
            selection: &mut $app.selection,
            search: &mut $app.search,
            chat_inbox: &mut $app.chat_inbox,
            actions: $actions,
            blocked: $blocked,
        }
    };
}

/// Draws everything below the window frame.
pub fn draw(app: &mut App, ui: &mut Ui, actions: &mut Vec<Action>, blocked: bool) {
    top_bar(app, ui, actions);
    super::chrome::status_bar(app, ui);
    // Home's pipeline queue and Export's render queue advance every frame,
    // whether or not their page is on screen.
    crate::gui::pages::night::tick(&mut app.jobs, &mut app.pages.night);
    crate::gui::pages::deliver::tick(&mut app.jobs, &mut app.pages.deliver, ui.ctx());

    if app.home || app.project.is_none() {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::rgb(app.theme.app)))
            .show(ui, |ui| {
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::night::draw(&mut ctx, &mut app.pages.night, ui);
            });
        return;
    }

    let panel = theme::rgb(app.theme.panel);
    let line = egui::Stroke::new(1.0, theme::rgb(app.theme.border));
    let frame = egui::Frame::new()
        .fill(panel)
        .stroke(line)
        .inner_margin(theme::margin(SPACE_SM));

    if app.chat_open {
        egui::Panel::left("chat")
            .resizable(true)
            .default_size(330.0)
            .size_range(260.0..=560.0)
            .frame(frame)
            .show(ui, |ui| {
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::assistant::panel(&mut ctx, &mut app.pages.assistant, ui);
            });
    }
    egui::Panel::bottom("timeline")
        .resizable(true)
        .default_size(300.0)
        .size_range(160.0..=640.0)
        .frame(frame)
        .show(ui, |ui| {
            let mut ctx = page_ctx!(app, actions, blocked);
            crate::gui::pages::plan::draw(&mut ctx, &mut app.pages.plan, ui);
        });
    if app.export_open {
        egui::Panel::right("export")
            .resizable(true)
            .default_size(380.0)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    widgets::title(ui, &app.theme, "Export");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Close export").clicked() {
                            app.export_open = false;
                        }
                    });
                });
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::deliver::draw(&mut ctx, &mut app.pages.deliver, ui);
            });
    }
    if app.inspector_open {
        egui::Panel::right("inspector")
            .resizable(true)
            .default_size(270.0)
            .size_range(220.0..=480.0)
            .frame(frame)
            .show(ui, |ui| {
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::inspector::draw(&mut ctx, &mut app.pages.inspector, ui);
            });
    }
    egui::Panel::left("browser")
        .resizable(true)
        .default_size(400.0)
        .size_range(280.0..=900.0)
        .frame(frame)
        .show(ui, |ui| {
            browser_tabs(app, ui);
            ui.separator();
            let mut ctx = page_ctx!(app, actions, blocked);
            match app.browser_tab {
                BrowserTab::Shots => {
                    crate::gui::pages::review::draw(&mut ctx, &mut app.pages.review, ui)
                }
                BrowserTab::People => {
                    crate::gui::pages::people::draw(&mut ctx, &mut app.pages.people, ui)
                }
                BrowserTab::Music => {
                    crate::gui::pages::music::draw(&mut ctx, &mut app.pages.music, ui)
                }
            }
        });
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(app.theme.app))
                .inner_margin(theme::margin(SPACE_SM)),
        )
        .show(ui, |ui| {
            let mut ctx = page_ctx!(app, actions, blocked);
            crate::gui::pages::viewer::draw(&mut ctx, &mut app.pages.viewer, ui);
        });

    if !blocked && !ui.ctx().egui_wants_keyboard_input() {
        reference_keys(app, ui, actions);
    }
    reference_popup(app, ui.ctx(), actions);
}

/// The browser's tab strip.
fn browser_tabs(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        for tab in BrowserTab::ALL {
            let on = app.browser_tab == tab;
            let text = egui::RichText::new(tab.label()).color(theme::rgb(if on {
                app.theme.text
            } else {
                app.theme.text_muted
            }));
            if ui.selectable_label(on, text).clicked() {
                app.browser_tab = tab;
            }
        }
    });
}

/// `R` stages the selection for the chat; `Shift-R` asks for an intent.
fn reference_keys(app: &mut App, ui: &Ui, actions: &mut Vec<Action>) {
    let (r, shift) = ui.input(|i| (i.key_pressed(egui::Key::R), i.modifiers.shift));
    if !r || app.selection.is_empty() || ui.input(|i| i.modifiers.command || i.modifiers.alt) {
        return;
    }
    if shift {
        app.ref_popup = true;
    } else {
        for item in app.selection.clone() {
            actions.push(Action::AddRef(item));
        }
        app.chat_open = true;
    }
}

/// The "reference as…" chooser.
fn reference_popup(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    if !app.ref_popup {
        return;
    }
    let mut open = true;
    let mut chosen = None;
    egui::Window::new("Reference in chat as…")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("{} selected", app.selection.len()));
            for m in Modifier::ALL {
                let label = format!(
                    "{}  {}",
                    if m.prefix().is_empty() {
                        "·"
                    } else {
                        m.prefix()
                    },
                    m.label()
                );
                if ui.button(label).clicked() {
                    chosen = Some(m);
                }
            }
        });
    if let Some(m) = chosen {
        for item in app.selection.clone() {
            actions.push(Action::AddRef(Reference {
                modifier: m,
                ..item
            }));
        }
        app.chat_open = true;
        open = false;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    app.ref_popup = open;
}

/// The top bar: home, the night, search, export and app controls.
fn top_bar(app: &mut App, ui: &mut Ui, actions: &mut Vec<Action>) {
    let theme = app.theme.clone();
    egui::Panel::top("topbar")
        .exact_size(theme::HEADER_HEIGHT - 8.0)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.app))
                .inner_margin(theme::margin_xy(SPACE_MD, 0.0)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let home = widgets::icon_button(ui, &theme, "Home: pipeline and recent nights");
                paint_glyph(ui, &home, super::glyph::NIGHT);
                if home.response.clicked() {
                    actions.push(Action::Go(if app.home {
                        Page::Review
                    } else {
                        Page::Night
                    }));
                }
                let name = app
                    .project
                    .as_ref()
                    .map_or_else(|| crate::NAME.to_owned(), crate::project::Project::name);
                ui.label(
                    egui::RichText::new(name)
                        .font(theme::font(theme::TITLE_SIZE))
                        .color(theme::rgb(theme.text))
                        .strong(),
                );

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let settings = widgets::icon_button(ui, &theme, "Settings (Ctrl+,)");
                    paint_glyph(ui, &settings, super::glyph::SETTINGS);
                    if settings.response.clicked() {
                        actions.push(Action::OpenSettings);
                    }
                    let commands = widgets::icon_button(ui, &theme, "Commands (Ctrl+K)");
                    paint_glyph(ui, &commands, super::glyph::COMMAND);
                    if commands.response.clicked() {
                        actions.push(Action::OpenCommands);
                    }
                    let export = widgets::icon_button(ui, &theme, "Export (Ctrl+E)");
                    paint_glyph(ui, &export, super::glyph::DELIVER);
                    if export.response.clicked() {
                        app.export_open = !app.export_open;
                    }
                    let inspector = widgets::icon_button(ui, &theme, "Inspector (Ctrl+I)");
                    paint_glyph(ui, &inspector, super::glyph::ABOUT);
                    if inspector.response.clicked() {
                        app.inspector_open = !app.inspector_open;
                    }
                    let chat = widgets::icon_button(ui, &theme, "Chat (Ctrl+J)");
                    paint_glyph(ui, &chat, super::glyph::PLAN);
                    if chat.response.clicked() {
                        app.chat_open = !app.chat_open;
                    }
                    // The search box takes the middle, centred in what is left.
                    let width = (ui.available_width() - 40.0).clamp(160.0, 520.0);
                    let pad = ((ui.available_width() - width) / 2.0).max(0.0);
                    ui.add_space(pad);
                    let search = egui::TextEdit::singleline(&mut app.search)
                        .hint_text("Search shots, captions, people…   fire>0.3  dark  person:…")
                        .desired_width(width);
                    let response = ui.add(search);
                    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
                        response.request_focus();
                    }
                    if response.changed() {
                        app.browser_tab = BrowserTab::Shots;
                    }
                });
            });
        });
}

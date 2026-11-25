// ============================================================================
// Module:       gui::ui::settings
// Description:  The settings card: the theme picker and the switches, drawn in
//               the shared modal.
//
// Dependencies: egui; super::{modal, theme, widgets}, crate::gui::app
// ============================================================================

//! Settings.
//!
//! Every control here emits an [`Action`] rather than writing to the app,
//! so a setting changed here and the same setting changed from a shortcut
//! or the command palette are one code path — and, with the config on, one
//! saved preference.

use super::theme::{SPACE_LG, SPACE_SM};
use super::{modal, widgets};
use crate::gui::app::{Action, App};
use crate::gui::ui::hint::Hint as _;
use egui::Ui;

/// The settings card's width.
const WIDTH: f32 = 560.0;

/// Draws the settings card while it is open (and while it fades out).
pub fn draw(app: &App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let theme = app.theme.clone();
    let outcome = modal::show(ctx, &theme, "Settings", app.settings_open, WIDTH, |ui| {
        // The sections scroll in a short window; Done stays in view.
        let room = (ui.ctx().content_rect().height() - 220.0).max(240.0);
        egui::ScrollArea::vertical()
            .max_height(room)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                appearance(app, ui, actions);
                ui.add_space(SPACE_LG);
                keyboard(app, ui, actions);
                ui.add_space(SPACE_LG);
                behaviour(app, ui, actions);
                ui.add_space(SPACE_LG);
                captions(app, ui, actions);
            });
        ui.add_space(SPACE_LG);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            widgets::button(ui, &app.theme, "Done", widgets::Emphasis::Primary).clicked()
        })
        .inner
    });
    if outcome.dismissed || outcome.inner == Some(true) {
        actions.push(Action::CloseSettings);
    }
}

/// Captions: where the local vision model is, and whether it is ready.
fn captions(app: &App, ui: &mut Ui, actions: &mut Vec<Action>) {
    widgets::title(ui, &app.theme, "Captions");
    ui.add_space(SPACE_SM);
    widgets::muted(
        ui,
        &app.theme,
        "Captions describe each shot with a local vision model (llama.cpp and a Qwen3-VL model). Point at them here; the AVE_* environment variables win when set.",
    );
    ui.add_space(SPACE_SM);
    let rows: [(&str, Option<&String>, bool); 3] = [
        (
            "llama.cpp folder",
            app.config.caption_llama_dir.as_ref(),
            true,
        ),
        ("Model (.gguf)", app.config.caption_model.as_ref(), false),
        (
            "Projector (.gguf)",
            app.config.caption_mmproj.as_ref(),
            false,
        ),
    ];
    let rows_set = rows.iter().filter(|(_, v, _)| v.is_some()).count();
    for (i, (label, value, folder)) in rows.into_iter().enumerate() {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(120.0, super::theme::CONTROL_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(120.0);
                    ui.label(label);
                },
            );
            let shown = value.map_or("Not set", String::as_str);
            ui.add(
                egui::Label::new(egui::RichText::new(shown).color(super::theme::rgb(
                    if value.is_some() {
                        app.theme.text
                    } else {
                        app.theme.text_muted
                    },
                )))
                .truncate(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::outline_button(ui, &app.theme, "Choose\u{2026}").clicked() {
                    let dialog = rfd::FileDialog::new();
                    let picked = if folder {
                        dialog
                            .set_title("The folder llama-server is in")
                            .pick_folder()
                    } else {
                        dialog
                            .set_title(label)
                            .add_filter("GGUF model", &["gguf"])
                            .pick_file()
                    };
                    if let Some(p) = picked {
                        let mut paths = [None, None, None];
                        if let Some(slot) = paths.get_mut(i) {
                            *slot = Some(p.display().to_string());
                        }
                        actions.push(Action::SetCaptionPaths(paths));
                    }
                }
            });
        });
    }
    ui.add_space(SPACE_SM);
    let none_set = rows_set == 0;
    let (ok, line) = match crate::caption::not_set_up() {
        None => (true, "Ready: Caption runs on this machine.".to_owned()),
        Some(_) if none_set => (false, "Not ready: choose the three above.".to_owned()),
        Some(why) => (false, format!("Not ready: {why}.")),
    };
    ui.label(egui::RichText::new(line).color(super::theme::rgb(if ok {
        app.theme.success
    } else {
        app.theme.text_muted
    })));
}

/// The width of one theme card in the picker.
const THEME_CARD: egui::Vec2 = egui::vec2(124.0, 78.0);

/// The theme picker: one miniature of each theme, drawn in its own colours.
fn appearance(app: &App, ui: &mut Ui, actions: &mut Vec<Action>) {
    widgets::title(ui, &app.theme, "Theme");
    ui.add_space(SPACE_SM);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_SM);
        for candidate in app.catalog.themes() {
            let selected = candidate.id == app.theme.id;
            if theme_card(ui, &app.theme, candidate, selected).clicked() && !selected {
                actions.push(Action::SetTheme(candidate.id.clone()));
            }
        }
    });
    for (source, reason) in app.catalog.problems() {
        ui.label(
            egui::RichText::new(format!("{source}: {reason}"))
                .small()
                .color(super::theme::rgb(app.theme.warning)),
        );
    }
    if let Some(dir) = crate::theme::user_theme_dir() {
        widgets::muted(
            ui,
            &app.theme,
            &format!("Add your own: drop a .toml theme into {}", dir.display()),
        );
    }
}

/// One theme in the picker: a miniature window in the candidate's colours,
/// framed in the active theme's, with an accent ring when it is in force.
fn theme_card(
    ui: &mut Ui,
    active: &crate::theme::Palette,
    candidate: &crate::theme::Palette,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(THEME_CARD, egui::Sense::click());
    let response = response.hint(&candidate.name);
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    let ring = widgets::fade(ui.ctx(), response.id.with("selected"), selected);
    let painter = ui.painter();
    let radius = egui::CornerRadius::same(super::theme::RADIUS_LG);
    painter.rect_filled(rect, radius, super::theme::rgb(candidate.app));
    // A sidebar, a card, and the accent: enough to recognise a theme by.
    let sidebar = egui::Rect::from_min_size(rect.min, egui::vec2(28.0, rect.height()));
    painter.rect_filled(
        sidebar,
        egui::CornerRadius {
            nw: super::theme::RADIUS_LG,
            sw: super::theme::RADIUS_LG,
            ne: 0,
            se: 0,
        },
        super::theme::rgb(candidate.panel),
    );
    let card = egui::Rect::from_min_size(rect.min + egui::vec2(36.0, 10.0), egui::vec2(78.0, 30.0));
    painter.rect_filled(
        card,
        egui::CornerRadius::same(4),
        super::theme::rgb(candidate.raised),
    );
    painter.circle_filled(
        card.left_center() + egui::vec2(10.0, 0.0),
        5.0,
        super::theme::rgb(candidate.accent),
    );
    painter.text(
        rect.left_bottom() + egui::vec2(36.0, -10.0),
        egui::Align2::LEFT_BOTTOM,
        &candidate.name,
        super::theme::font(12.0),
        super::theme::rgb(candidate.text),
    );
    let stroke = active
        .border
        .lerp(active.border_strong, lift)
        .lerp(active.accent, ring);
    painter.rect_stroke(
        rect,
        radius,
        egui::Stroke::new(1.0 + ring, super::theme::rgb(stroke)),
        egui::StrokeKind::Outside,
    );
    widgets::focus_ring(ui, active, &response);
    response
}

/// The keyboard preset: which editor's habits the shortcuts follow, as
/// Selects asks on its first run.
fn keyboard(app: &App, ui: &mut Ui, actions: &mut Vec<Action>) {
    widgets::title(ui, &app.theme, "Keyboard");
    ui.add_space(SPACE_SM);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = super::theme::SPACE_XS;
        for preset in super::keymap::Preset::ALL {
            let on = app.keymap == preset;
            if widgets::tab(ui, &app.theme, preset.label(), on).clicked() && !on {
                actions.push(Action::SetKeymap(preset));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(
                ui,
                &app.theme,
                "Show all shortcuts",
                widgets::Emphasis::Secondary,
            )
            .clicked()
            {
                actions.push(Action::CloseSettings);
                actions.push(Action::ToggleShortcuts);
            }
        });
    });
    widgets::muted(
        ui,
        &app.theme,
        "Premiere and Final Cut move the panel keys (Shift+1…4 or Cmd+1…4) and export to where those editors put them.",
    );
}

/// The switches.
fn behaviour(app: &App, ui: &mut Ui, actions: &mut Vec<Action>) {
    widgets::title(ui, &app.theme, "Behaviour");
    ui.add_space(SPACE_SM);
    let mut collapsed = app.sidebar_collapsed;
    if widgets::switch(ui, &app.theme, &mut collapsed, "Icon-only sidebar").changed() {
        actions.push(Action::ToggleSidebar);
    }
    let mut reduce = app.reduce_motion;
    if widgets::switch(ui, &app.theme, &mut reduce, "Reduce motion").changed() {
        actions.push(Action::ToggleMotion);
    }
    ui.add_space(SPACE_SM);
    ui.horizontal(|ui| {
        let mut jobs = crate::gui::pages::night::pipeline::analysis_jobs();
        if ui
            .add(egui::DragValue::new(&mut jobs).range(1..=32).speed(0.05))
            .accessible_name("Clips analysed at once")
            .changed()
        {
            actions.push(Action::SetAnalysisJobs(jobs));
        }
        widgets::muted(
            ui,
            &app.theme,
            "clips analysed at once (each runs its own ffmpeg; more is faster on a strong machine)",
        );
    });
    ui.add_space(super::theme::SPACE_MD);
    if let Some(path) = crate::config::config_path() {
        widgets::muted(
            ui,
            &app.theme,
            &format!("Preferences are saved on exit to {}", path.display()),
        );
    }
}

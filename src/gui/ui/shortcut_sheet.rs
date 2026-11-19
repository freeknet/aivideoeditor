// ============================================================================
// Module:       gui::ui::shortcut_sheet
// Description:  The keyboard shortcut sheet (`?`): every command of the table,
//               grouped by region, with its keys under the preset in force.
//
// Dependencies: egui; super::{keymap, modal, theme, widgets}, crate::gui::app
// ============================================================================

//! The shortcut sheet.
//!
//! Printed from [`keymap::COMMANDS`], so it cannot list a key the app does
//! not have or miss one it does. Switching the preset in Settings changes
//! what it shows.

use super::theme::{self, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XS};
use super::{keymap, modal, widgets};
use crate::gui::app::{Action, App};
use egui::Ui;

/// The sheet's width.
const WIDTH: f32 = 760.0;

/// Draws the sheet while it is open (and while it fades out).
pub fn draw(app: &App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let theme = &app.theme;
    let preset = app.keymap;
    let outcome = modal::show(
        ctx,
        theme,
        "Keyboard shortcuts",
        app.shortcuts_open,
        WIDTH,
        |ui| {
            widgets::muted(
                ui,
                theme,
                &format!(
                    "{} preset. Change it in Settings; Selects' Premiere and Final Cut layouts are there too.",
                    preset.label()
                ),
            );
            ui.add_space(SPACE_SM);
            let height = (ctx.content_rect().height() * 0.7).max(240.0);
            egui::ScrollArea::vertical()
                .max_height(height)
                .show(ui, |ui| {
                    ui.columns(2, |columns| {
                        if let [left, right] = columns {
                            scope(left, app, keymap::Scope::Global);
                            for scope_ in [
                                keymap::Scope::Viewer,
                                keymap::Scope::Timeline,
                                keymap::Scope::Browser,
                                keymap::Scope::Chat,
                                keymap::Scope::Export,
                            ] {
                                scope(right, app, scope_);
                                right.add_space(SPACE_MD);
                            }
                        }
                    });
                });
        },
    );
    if outcome.dismissed {
        actions.push(Action::ToggleShortcuts);
    }
}

/// One region's commands: a heading, then key and label per row.
fn scope(ui: &mut Ui, app: &App, scope: keymap::Scope) {
    let theme = &app.theme;
    ui.label(
        egui::RichText::new(scope.label().to_uppercase())
            .font(theme::font(theme::SMALL_SIZE))
            .color(theme::rgb(theme.text_faint)),
    );
    ui.add_space(SPACE_XS);
    egui::Grid::new(("shortcut-sheet", scope.label()))
        .num_columns(2)
        .spacing([SPACE_LG, SPACE_XS])
        .show(ui, |ui| {
            for command in keymap::COMMANDS.iter().filter(|c| c.scope == scope) {
                let keys = command.hint(app.keymap);
                if keys.is_empty() {
                    continue;
                }
                key_cap(ui, app, &keys);
                ui.label(
                    egui::RichText::new(command.label)
                        .font(theme::font(theme::BODY_SIZE))
                        .color(theme::rgb(theme.text_muted)),
                );
                ui.end_row();
            }
        });
}

/// Keys drawn as a small cap: an ink outline around monospace text.
fn key_cap(ui: &mut Ui, app: &App, keys: &str) {
    let theme = &app.theme;
    egui::Frame::new()
        .stroke(theme::ink_line(theme))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin_xy(SPACE_XS + 2.0, 1.0))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(keys)
                    .font(egui::FontId::monospace(theme::SMALL_SIZE))
                    .color(theme::rgb(theme.text)),
            );
        });
}

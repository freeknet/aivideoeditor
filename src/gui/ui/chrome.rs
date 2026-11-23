// ============================================================================
// Module:       gui::ui::chrome
// Description:  Everything around the page rather than in it: the status bar
//               (the top bar is gui::ui::topbar).
//
// Dependencies: egui (Panel); super::{theme, widgets, icons}, crate::gui::app
// ============================================================================

//! The window's frame.
//!
//! The window keeps the platform's own title bar, so dragging, snapping and
//! the caption buttons behave exactly as the user expects on every runtime.
//! The top bar below it carries the night and the app-wide controls; the
//! status bar along the foot says what is in force: what is running, the
//! last error, the night, and where the shortcuts are.
//!
//! The status bar is a panel-coloured strip with a hairline above it, as
//! every edge in the workspace is: one surface, ink lines between.

use super::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use super::{icons, widgets};
use crate::gui::app::App;
use egui::{Align, Layout, Ui};

/// Draws the status bar.
pub fn status_bar(app: &App, ui: &mut Ui) {
    let theme = &app.theme;
    egui::Panel::bottom("status")
        .exact_size(theme::STATUS_HEIGHT)
        .resizable(false)
        .show_separator_line(true)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.panel))
                .inner_margin(theme::margin_xy(SPACE_MD, 0.0)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_XS + 2.0;
                let running: Vec<&str> = app.jobs.running().map(|j| j.label.as_str()).collect();
                let (glyph, color, text) = if let Some(notice) = &app.notice {
                    (icons::CIRCLE_ALERT, theme.danger, notice.clone())
                } else if running.is_empty() {
                    let night = app.project.as_ref().map_or_else(
                        || "No project open".to_owned(),
                        crate::project::Project::name,
                    );
                    (icons::CIRCLE_CHECK, theme.success, night)
                } else {
                    (
                        icons::LOADER,
                        theme.warning,
                        format!("Running: {}", running.join(", ")),
                    )
                };
                icons::icon(ui, glyph, icons::SMALL - 2.0, theme::rgb(color));
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(text)
                            .font(theme::font(theme::SMALL_SIZE))
                            .color(theme::rgb(theme.text_muted)),
                    )
                    .truncate(),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_SM;
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · v{}",
                            theme.name,
                            env!("CARGO_PKG_VERSION")
                        ))
                        .font(theme::font(theme::SMALL_SIZE))
                        .color(theme::rgb(theme.text_faint)),
                    );
                    let keys = super::keymap::command("shortcuts")
                        .map(|command| command.hint(app.keymap))
                        .unwrap_or_default();
                    if !keys.is_empty() {
                        ui.label(
                            egui::RichText::new("Shortcuts")
                                .font(theme::font(theme::SMALL_SIZE))
                                .color(theme::rgb(theme.text_faint)),
                        );
                        widgets::kbd(ui, theme, &keys);
                    }
                });
            });
        });
}

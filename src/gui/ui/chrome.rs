// ============================================================================
// Module:       gui::ui::chrome
// Description:  Everything around the page rather than in it: the header, the
//               sidebar and the status bar.
//
// Dependencies: egui (Panel); super::{theme, widgets}, crate::gui::app
// ============================================================================

//! The window's frame.
//!
//! The window keeps the platform's own title bar, so dragging, snapping and
//! the caption buttons behave exactly as the user expects on every runtime.
//! The header below it carries the mark, the name and the app-wide
//! controls; the sidebar carries navigation and collapses to icons; the
//! status bar says what is in force.
//!
//! Every panel turns its separator line off: each one is padded, so the
//! default line lands as a pale hairline floating in the gap, which reads
//! as a resize handle on a panel that cannot be resized.

use super::theme::{self, SPACE_LG};
use crate::gui::app::App;
use egui::{Align, Layout, Ui};

/// Draws the status bar.
pub fn status_bar(app: &App, ui: &mut Ui) {
    let theme = &app.theme;
    egui::Panel::bottom("status")
        .exact_size(theme::STATUS_HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.app))
                .inner_margin(theme::margin_xy(SPACE_LG, 0.0)),
        )
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let running: Vec<&str> = app.jobs.running().map(|j| j.label.as_str()).collect();
                let (color, text) = if let Some(notice) = &app.notice {
                    (theme.danger, notice.clone())
                } else if running.is_empty() {
                    let night = app
                        .project
                        .as_ref()
                        .map_or_else(|| "No night open".to_owned(), crate::project::Project::name);
                    (theme.success, night)
                } else {
                    (theme.warning, format!("Running: {}", running.join(", ")))
                };
                let (dot, _) = ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                ui.painter()
                    .circle_filled(dot.center(), 3.5, theme::rgb(color));
                ui.label(
                    egui::RichText::new(text)
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · v{}",
                            theme.name,
                            env!("CARGO_PKG_VERSION")
                        ))
                        .small()
                        .color(theme::rgb(theme.text_faint)),
                    );
                });
            });
        });
}

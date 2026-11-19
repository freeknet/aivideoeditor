// ============================================================================
// Module:       gui::pages::inspector
// Description:  The inspector: everything known about the selection (numbers,
//               caption, flags, who, where it is used), and its settings.
//
// Dependencies: egui; crate::{refs, gui::pages}
// ============================================================================

//! The inspector, docked on the workspace's right.

use super::PageCtx;
use crate::gui::ui::widgets;

/// The inspector's own state.
#[derive(Debug, Default)]
pub(crate) struct State {}

/// Draws the inspector into its panel.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let _ = state;
    widgets::title(ui, ctx.theme, "Inspector");
    if ctx.selection.is_empty() {
        widgets::muted(ui, ctx.theme, "Select a shot, segment, bar or person.");
        return;
    }
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    for item in ctx.selection.iter() {
        ui.label(egui::RichText::new(item.short()).strong());
        let facts = crate::refs::resolve(item, project).map_or_else(|e| e, |r| r.facts);
        widgets::muted(ui, ctx.theme, &facts);
        ui.add_space(crate::gui::ui::theme::SPACE_SM);
    }
}

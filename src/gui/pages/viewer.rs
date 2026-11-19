// ============================================================================
// Module:       gui::pages::viewer
// Description:  The viewer in the workspace centre: the shared player framed
//               for the chosen aspect, transport, safe-area overlays.
//
// Dependencies: egui; crate::gui::{pages, player}
// ============================================================================

//! The viewer region.

use super::PageCtx;
use crate::gui::ui::theme;

/// The viewer's own state.
#[derive(Debug, Default)]
pub(crate) struct State {}

/// Draws the viewer into the centre.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let _ = state;
    let size = egui::vec2(
        ui.available_width(),
        (ui.available_height() - 36.0).max(80.0),
    );
    let background = theme::rgb(ctx.theme.raised);
    ctx.player
        .show(ui, ctx.thumbs, size, 16.0 / 9.0, background);
    ctx.player.transport(ui);
    if ctx.keys_free(ui) {
        ctx.player.handle_keys(ui);
    }
}

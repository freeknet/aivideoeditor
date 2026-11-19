// ============================================================================
// Module:       gui::pages::people
// Description:  People: performer groups, the confirm queue, merge/split/rename
//               and manual clip ranges.
//
// Dependencies: egui; crate::gui::pages (PageCtx)
// ============================================================================

//! The People page. See `docs/GUI_CONTRACTS.md` and
//! `docs/research/02-ui-and-workflow.md` §9 for what it is meant to become.

use super::PageCtx;

/// The page's own state, kept across frames and page switches.
#[derive(Debug, Default)]
pub(crate) struct State {}

/// Draws the page into the central area.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let _ = state;
    super::placeholder(
        ctx,
        ui,
        "People",
        "People: performer groups, the confirm queue, merge/split/rename and manual clip ranges.",
    );
}

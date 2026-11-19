// ============================================================================
// Module:       gui::pages::plan
// Description:  Plan: the edit — storyboard, beat timeline, swap alternatives,
//               live rule check, with the assistant docked on the right.
//
// Dependencies: egui; crate::gui::{pages, assistant}
// ============================================================================

//! The Plan page. See `docs/GUI_CONTRACTS.md` and
//! `docs/research/02-ui-and-workflow.md` §9.7 for what it is meant to become.

use super::PageCtx;

/// The page's own state, kept across frames and page switches.
#[derive(Debug, Default)]
pub(crate) struct State {}

/// Draws the page into the central area. `assistant` is the docked panel's
/// state, owned by `pages::States` so the assistant module can grow apart.
pub(crate) fn draw(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    assistant: &mut crate::gui::assistant::State,
    ui: &mut egui::Ui,
) {
    let _ = state;
    if ctx.project.is_some() {
        egui::Panel::right("plan-assistant")
            .resizable(true)
            .default_size(320.0)
            .show(ui, |ui| crate::gui::assistant::panel(ctx, assistant, ui));
    }
    super::placeholder(
        ctx,
        ui,
        "Plan",
        "Plan: the edit — storyboard, beat timeline, swap alternatives, live rule check.",
    );
}

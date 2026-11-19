// ============================================================================
// Module:       gui::assistant
// Description:  The assistant panel: plain-language requests about the plan,
//               answered as proposals (a diff with reasons) to apply or discard.
//
// Dependencies: egui; crate::gui::pages (PageCtx)
// ============================================================================

//! The assistant panel the Plan page docks on its right.
//!
//! **Contract.** The Plan page calls [`panel`] inside the area it reserves;
//! the assistant never edits a plan directly, it returns a proposal that the
//! user applies, which saves a new plan version.

use crate::gui::pages::PageCtx;

/// The panel's state: the conversation and any pending proposal.
#[derive(Debug, Default)]
pub(crate) struct State {}

/// Draws the panel.
pub(crate) fn panel(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let _ = state;
    crate::gui::ui::widgets::muted(ui, ctx.theme, "Assistant: not built yet.");
    for r in ctx.refs.iter() {
        let facts = ctx
            .project
            .as_ref()
            .map(|p| crate::refs::resolve(r, p).map_or_else(|e| e, |x| x.facts))
            .unwrap_or_default();
        ui.label(egui::RichText::new(format!("{} {}", r.modifier.label(), r.short())).small())
            .on_hover_text(facts);
    }
    if !ctx.refs.is_empty() && ui.small_button("Clear references").clicked() {
        ctx.actions.push(crate::gui::app::Action::ClearRefs);
    }
}

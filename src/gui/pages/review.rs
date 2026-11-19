// ============================================================================
// Module:       gui::pages::review
// Description:  Review: the shot log as a culling grid with search, facets,
//               pick/reject and an inspector.
//
// Dependencies: egui; crate::gui::pages (PageCtx)
// ============================================================================

//! The Review page. See `docs/GUI_CONTRACTS.md` and
//! `docs/research/02-ui-and-workflow.md` §9 for what it is meant to become.

use super::PageCtx;

/// The page's own state, kept across frames and page switches.
#[derive(Debug, Default)]
pub(crate) struct State {}

/// Draws the page into the central area.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let _ = state;
    let query = ctx.search.to_lowercase();
    let matching = ctx
        .project
        .as_ref()
        .and_then(|p| p.index.as_ref())
        .map_or(0, |i| {
            i.segments
                .iter()
                .filter(|s| {
                    query.is_empty()
                        || s.caption
                            .as_ref()
                            .is_some_and(|c| c.summary.to_lowercase().contains(&query))
                })
                .count()
        });
    crate::gui::ui::widgets::muted(
        ui,
        ctx.theme,
        &format!("{matching} segments match. Shot browser not built yet."),
    );
}

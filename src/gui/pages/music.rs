// ============================================================================
// Module:       gui::pages::music
// Description:  Music: the track's waveform, beat grid, sections and drops, with grid fixes.
//
// Dependencies: egui; crate::gui::pages (PageCtx)
// ============================================================================

//! The Music page. See `docs/GUI_CONTRACTS.md` and
//! `docs/research/02-ui-and-workflow.md` §9 for what it is meant to become.

use super::PageCtx;

/// The page's own state, kept across frames and page switches.
#[derive(Debug, Default)]
pub(crate) struct State {}

/// Draws the page into the central area.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let _ = state;
    let track = ctx
        .project
        .as_ref()
        .and_then(|p| p.music.as_ref())
        .map(|m| (m.path.clone(), m.duration));
    match track {
        Some((path, duration)) => {
            if ui.button("▶ Play the track").clicked() {
                ctx.player
                    .open_audio(std::path::Path::new(&path), 0.0, f64::from(duration));
                ctx.player.play();
            }
        }
        None => {
            crate::gui::ui::widgets::muted(
                ui,
                ctx.theme,
                "Map the music on the Home screen first.",
            );
        }
    }
}

// ============================================================================
// Module:       gui::pages::plan
// Description:  The timeline at the foot of the workspace: plan shots on the
//               beat grid over the music, versions, swap alternatives, checks.
//
// Dependencies: egui; crate::{plan, gui::pages, gui::player}
// ============================================================================

//! The timeline. See `docs/GUI_CONTRACTS.md` and
//! `docs/research/02-ui-and-workflow.md` §9.7 for what it is meant to become.

use super::PageCtx;
use crate::gui::player::SeqShot;

/// The timeline's own state, kept across frames.
#[derive(Debug, Default)]
pub(crate) struct State {}

/// Draws the timeline into its panel.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let _ = state;
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    ui.horizontal(|ui| {
        for path in &project.plans {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if ui.button(format!("▶ {name}")).clicked()
                && let (Ok(plan), Some(music), Some(index)) = (
                    crate::plan::Plan::load(path),
                    project.music.as_ref(),
                    project.index.as_ref(),
                )
            {
                let shots: Vec<SeqShot> = plan
                    .shots
                    .iter()
                    .zip(plan.timing(music))
                    .filter_map(|(s, (_, dur))| {
                        let clip = index.clip(&s.clip)?;
                        Some(SeqShot {
                            path: clip.path.clone().into(),
                            t_in: f64::from(s.t_in),
                            dur: f64::from(dur),
                            cx: s.cx,
                        })
                    })
                    .collect();
                let start = f64::from(music.beat_time(plan.start_bar, 0));
                ctx.player
                    .open_sequence(shots, Some((plan.music.clone().into(), start)));
                ctx.player.play();
            }
        }
    });
    crate::gui::ui::widgets::muted(ui, ctx.theme, "Timeline not built yet.");
}

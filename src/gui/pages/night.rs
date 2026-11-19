// ============================================================================
// Module:       gui::pages::night
// Description:  Home: recent nights, new night from last week, the pipeline
//               as a step bar with live job progress, and the night's settings
//               and recipe.
//
// Dependencies: egui, rfd; crate::{commands, jobs, project, recipe},
//               crate::gui::pages
// ============================================================================

//! The Home screen: the project manager.
//!
//! - **Left, recent nights** (`recent`): every night opened lately plus its
//!   siblings on disk, each with the first clip's frame, its date and how
//!   far it got (indexed, captioned, planned, delivered). "New night…"
//!   sets a night up from last week's (`recipe::new_night`); "Open…" opens
//!   any folder.
//! - **Main, the open night**: the pipeline as a step bar (`pipeline`):
//!   Index → Caption → Music → People → Plans → Export, each with what it
//!   produced, when, and a Run button. Every run goes through one serial
//!   queue of `ctx.jobs` jobs, so "Run everything" and a single step can
//!   never race each other. Below it, the night's settings and recipe
//!   (`settings`), and a strip of the night's clips to skim.
//!
//! See `docs/research/02-ui-and-workflow.md` §6 and §9.3.

pub(crate) mod pipeline;
pub(crate) mod recent;
pub(crate) mod settings;

use std::path::PathBuf;

use super::PageCtx;
use crate::gui::app::{Action, Page};
use crate::gui::ui::{theme, widgets};

/// The page's own state.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The recent-nights column.
    recent: recent::State,
    /// The step bar and its job queue.
    pipeline: pipeline::State,
    /// The settings and recipe editor.
    settings: settings::State,
    /// The clip being previewed in the strip, by index.
    preview: Option<usize>,
    /// What the last "New night…" carried over, shown once.
    notice: Option<String>,
}

/// Work the page does every frame, shown or not: starting the next queued
/// pipeline job when the one before it finishes.
pub(crate) fn tick(jobs: &mut crate::jobs::Jobs, state: &mut State) {
    state.pipeline.tick(jobs);
}

/// Draws the page into the central area.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let line = egui::Stroke::new(1.0, theme::rgb(ctx.theme.border));
    egui::Panel::left("home-recent")
        .resizable(true)
        .default_size(300.0)
        .size_range(240.0..=420.0)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(ctx.theme.panel))
                .stroke(line)
                .inner_margin(theme::margin(theme::SPACE_MD)),
        )
        .show(ui, |ui| {
            if let Some(notice) = recent::draw(ctx, &mut state.recent, ui) {
                state.notice = Some(notice);
            }
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::rgb(ctx.theme.app)))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("home-main")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(theme::margin_xy(theme::SPACE_XL, theme::SPACE_LG))
                        .show(ui, |ui| main(ctx, state, ui));
                });
        });
}

/// The open night, or a welcome when there is none.
fn main(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    if ctx.project.is_none() {
        welcome(ctx, state, ui);
        return;
    }
    header(ctx, state, ui);
    state.settings.sync(ctx);
    pipeline::draw(ctx, &mut state.pipeline, &state.settings, ui);
    ui.add_space(theme::SPACE_XL);
    settings::draw(ctx, &mut state.settings, ui);
    ui.add_space(theme::SPACE_XL);
    clips(ctx, state, ui);
}

/// What Home shows with no night open.
fn welcome(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    ui.add_space(theme::SPACE_XL);
    super::heading(
        ctx,
        ui,
        "No night open",
        "Start this week's night from last week's — same deliverables, end cards and \
         naming — or open one from the list.",
    );
    ui.horizontal(|ui| {
        if widgets::button(ui, ctx.theme, "New night…", widgets::Emphasis::Primary).clicked()
            && let Some(notice) = recent::new_night(ctx, &state.recent)
        {
            state.notice = Some(notice);
        }
        if widgets::button(ui, ctx.theme, "Open…", widgets::Emphasis::Secondary).clicked() {
            ctx.actions.push(Action::PickProject);
        }
    });
    ui.add_space(theme::SPACE_LG);
    widgets::muted(
        ui,
        ctx.theme,
        "A night is a folder of footage. ave keeps its work (shot log, music map, people, \
         plans, renders) in an _ave folder beside it, and delivers to the night's videos folder.",
    );
}

/// The night's name, where it lives, and the way into the workspace.
fn header(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let Some((name, work)) = ctx.project.as_ref().map(|p| (p.name(), p.work.clone())) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(&name)
                    .font(theme::font(theme::HEADING_SIZE + 4.0))
                    .color(theme::rgb(ctx.theme.text))
                    .strong(),
            );
            widgets::muted(ui, ctx.theme, &work.display().to_string());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, ctx.theme, "Open editor", widgets::Emphasis::Primary)
                .on_hover_text("The workspace: shots, viewer, timeline (Ctrl+2)")
                .clicked()
            {
                ctx.actions.push(Action::Go(Page::Review));
            }
            if widgets::button(ui, ctx.theme, "Export…", widgets::Emphasis::Secondary).clicked() {
                ctx.actions.push(Action::Go(Page::Deliver));
            }
            if widgets::button(ui, ctx.theme, "Reveal", widgets::Emphasis::Secondary)
                .on_hover_text("Show the work folder in Explorer")
                .clicked()
            {
                super::deliver::files::open_folder(&work);
            }
        });
    });
    if let Some(notice) = state.notice.clone() {
        ui.add_space(theme::SPACE_SM);
        let mut dismiss = false;
        egui::Frame::new()
            .fill(theme::rgb(ctx.theme.accent_soft))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS))
            .inner_margin(theme::margin(theme::SPACE_SM))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&notice).color(theme::rgb(ctx.theme.text)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        dismiss = ui.small_button("✕").clicked();
                    });
                });
            });
        if dismiss {
            state.notice = None;
        }
    }
    if let Some(p) = ctx.project.as_ref() {
        for w in &p.warnings {
            ui.colored_label(theme::rgb(ctx.theme.warning), w);
        }
    }
    ui.add_space(theme::SPACE_LG);
}

/// A strip of the night's clips: click one to skim it in the viewer.
fn clips(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let Some(index) = ctx.project.as_ref().and_then(|p| p.index.as_ref()) else {
        return;
    };
    let clips: Vec<(String, PathBuf, f64)> = index
        .clips
        .iter()
        .map(|c| (c.id.clone(), PathBuf::from(&c.path), c.probe.duration))
        .collect();
    widgets::title(ui, ctx.theme, &format!("Clips · {}", clips.len()));
    ui.add_space(theme::SPACE_XS);
    let placeholder = theme::rgb(ctx.theme.raised);
    egui::ScrollArea::horizontal()
        .id_salt("night-clips")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, (id, path, duration)) in clips.iter().enumerate() {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(128.0, 72.0), egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        ctx.thumbs
                            .paint(ui, rect, path, duration * 0.5, 0.5, placeholder);
                    }
                    if state.preview == Some(i) {
                        ui.painter().rect_stroke(
                            rect,
                            egui::CornerRadius::same(4),
                            egui::Stroke::new(2.0, theme::rgb(ctx.theme.accent)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if response.on_hover_text(id).clicked() {
                        state.preview = Some(i);
                        ctx.player.open_clip(path, 0.0, *duration, 0.5);
                        ctx.player.play();
                    }
                }
            });
        });
    let loaded = matches!(ctx.player.source(), crate::gui::player::Source::Clip { .. });
    if state.preview.is_some() && loaded {
        ui.add_space(theme::SPACE_SM);
        let width = ui.available_width().min(640.0);
        let background = theme::rgb(ctx.theme.raised);
        ctx.player.show(
            ui,
            ctx.thumbs,
            egui::vec2(width, width * 9.0 / 16.0),
            16.0 / 9.0,
            background,
        );
        ctx.player.transport(ui);
        if ctx.keys_free(ui) {
            ctx.player.handle_keys(ui);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::gui::app::{Action, App, Page};

    /// Runs `frames` headless frames at a desktop size.
    fn run(ctx: &egui::Context, app: &mut App, frames: usize) {
        for _ in 0..frames {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1500.0, 900.0),
                )),
                ..Default::default()
            };
            ctx.run_pass(input, |ctx| app.ui(ctx))
                .drop_without_applying_deltas();
        }
    }

    #[test]
    fn home_and_export_draw_an_open_night_without_writing_to_it() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-11").join("_ave");
        std::fs::create_dir_all(work.join("plans"))?;
        std::fs::write(work.join("ave.toml"), "footage = \"nowhere\"\n")?;
        std::fs::write(work.join("plans").join("reel-30.toml"), "not a plan")?;
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, crate::config::Config::default(), Some(work.clone()));
        app.apply(&ctx, Action::Go(Page::Night));
        run(&ctx, &mut app, 4);
        assert!(app.project.is_some(), "the night opened");
        app.apply(&ctx, Action::Go(Page::Deliver));
        run(&ctx, &mut app, 4);
        assert!(app.export_open, "export docked");
        assert!(
            !work.join(crate::recipe::FILE).exists(),
            "opening and drawing a night never writes its recipe"
        );
        Ok(())
    }
}

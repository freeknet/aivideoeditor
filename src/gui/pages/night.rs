// ============================================================================
// Module:       gui::pages::night
// Description:  Night: open or create a night, run the pipeline steps, see what
//               each produced.
//
// Dependencies: egui; crate::{commands, jobs, project}, crate::gui::pages
// ============================================================================

//! The Night page: the project home.
//!
//! One card per pipeline step (index → caption → beats → people → plan),
//! each with what it has produced so far and a button that runs it as a
//! background job — the same command `ave <step>` runs in a terminal. Below
//! them, the job log, and a strip of the night's clips to skim.
//!
//! See `docs/research/02-ui-and-workflow.md` §9.3 for where this is going
//! (new night from last week, recipes, recent nights with thumbnails).

use std::path::PathBuf;

use super::PageCtx;
use crate::commands::Cmd;
use crate::gui::app::Action;
use crate::gui::ui::{theme, widgets};

/// The page's own state.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The job whose log is shown.
    shown_job: Option<u64>,
    /// The clip being previewed in the strip, by index.
    preview: Option<usize>,
}

/// One pipeline step as a card.
struct Step {
    title: &'static str,
    done: Option<String>,
    run: Option<(String, Cmd)>,
    hint: &'static str,
}

/// The steps, with what each has produced and the job that (re)runs it.
fn steps(ctx: &PageCtx<'_>) -> Vec<Step> {
    let Some(p) = ctx.project.as_ref() else {
        return Vec::new();
    };
    let work = p.work.clone();
    let index_done = p.index.as_ref().map(|i| {
        let flagged = i.segments.iter().filter(|s| !s.flags.is_empty()).count();
        format!(
            "{} clips, {} segments, {flagged} flagged",
            i.clips.len(),
            i.segments.len()
        )
    });
    let captions = p
        .index
        .as_ref()
        .map(|i| i.segments.iter().filter(|s| s.caption.is_some()).count());
    let footage = p.settings.footage.clone();
    let track = p.settings.track.clone();
    let people_n = p.people.names.len();
    vec![
        Step {
            title: "Index footage",
            done: index_done,
            run: footage.map(|f| {
                (
                    "index".to_owned(),
                    Cmd::Index {
                        footage: f,
                        work: work.clone(),
                        jobs: 4,
                        force: false,
                    },
                )
            }),
            hint: "Set the footage folder in ave.toml or open the footage folder itself.",
        },
        Step {
            title: "Caption with the vision model",
            done: captions
                .filter(|n| *n > 0)
                .map(|n| format!("{n} segments captioned")),
            run: p.index.as_ref().map(|_| {
                (
                    "caption".to_owned(),
                    Cmd::Caption {
                        work: work.clone(),
                        min_q: 0.15,
                        redo: false,
                        parallel: 3,
                    },
                )
            }),
            hint: "Index first.",
        },
        Step {
            title: "Map the music",
            done: p.music.as_ref().map(|m| {
                format!(
                    "{:.1} BPM, {} bars, {} drop candidates",
                    m.bpm,
                    m.bars.len(),
                    m.drops.len()
                )
            }),
            run: track.map(|t| {
                (
                    "beats".to_owned(),
                    Cmd::Beats {
                        track: t,
                        work: work.clone(),
                    },
                )
            }),
            hint: "Set the track in ave.toml.",
        },
        Step {
            title: "Draft who's who",
            done: (people_n > 0).then(|| format!("{people_n} clips named")),
            run: p.index.as_ref().map(|_| {
                (
                    "people".to_owned(),
                    Cmd::People {
                        work: work.clone(),
                        force: true,
                    },
                )
            }),
            hint: "Caption first.",
        },
        Step {
            title: "Plans",
            done: (!p.plans.is_empty()).then(|| format!("{} plans", p.plans.len())),
            run: None,
            hint: "Draft and edit plans on the Plan page.",
        },
    ]
}

/// Draws the page into the central area.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(theme::margin(theme::SPACE_XL))
                .show(ui, |ui| {
                    if ctx.project.is_none() {
                        super::no_project(ctx, ui);
                        recent(ctx, ui);
                        return;
                    }
                    let (name, work) = ctx
                        .project
                        .as_ref()
                        .map(|p| (p.name(), p.work.display().to_string()))
                        .unwrap_or_default();
                    super::heading(ctx, ui, &name, &work);
                    if let Some(p) = ctx.project.as_ref() {
                        for w in &p.warnings {
                            ui.colored_label(theme::rgb(ctx.theme.warning), w);
                        }
                    }
                    pipeline(ctx, state, ui);
                    ui.add_space(theme::SPACE_LG);
                    log(ctx, state, ui);
                    ui.add_space(theme::SPACE_LG);
                    clips(ctx, state, ui);
                });
        });
}

/// The pipeline cards.
fn pipeline(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let steps = steps(ctx);
    let busy: Vec<String> = ctx.jobs.running().map(|j| j.label.clone()).collect();
    let theme = ctx.theme;
    let mut start: Option<(String, Cmd)> = None;
    widgets::card_grid(ui, 240.0, steps.len(), |ui, i| {
        let Some(step) = steps.get(i) else { return };
        let id = egui::Id::unique("night-step").with(i);
        widgets::card(ui, theme, id, step.done.is_some(), |ui| {
            widgets::title(ui, theme, step.title);
            match &step.done {
                Some(done) => {
                    ui.colored_label(theme::rgb(theme.success), done);
                }
                None => {
                    widgets::muted(ui, theme, "Not run yet.");
                }
            }
            ui.add_space(theme::SPACE_SM);
            match &step.run {
                Some((label, cmd)) => {
                    let running = busy.contains(label);
                    let text = if running {
                        "Running…"
                    } else if step.done.is_some() {
                        "Run again"
                    } else {
                        "Run"
                    };
                    let emphasis = if step.done.is_some() {
                        widgets::Emphasis::Secondary
                    } else {
                        widgets::Emphasis::Primary
                    };
                    if ui
                        .add_enabled_ui(!running, |ui| widgets::button(ui, theme, text, emphasis))
                        .inner
                        .clicked()
                    {
                        start = Some((label.clone(), cmd.clone()));
                    }
                }
                None => {
                    widgets::muted(ui, theme, step.hint);
                }
            }
        });
    });
    if let Some((label, cmd)) = start {
        state.shown_job = Some(ctx.jobs.spawn(label, cmd));
    }
}

/// The log of the selected (or latest) job.
fn log(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let Some(id) = state
        .shown_job
        .or_else(|| ctx.jobs.list.last().map(|j| j.id))
    else {
        return;
    };
    let Some(job) = ctx.jobs.get(id) else { return };
    widgets::title(ui, ctx.theme, &format!("Job: {}", job.label));
    let status = match &job.status {
        crate::jobs::Status::Running => {
            format!("running for {:.0}s", job.started.elapsed().as_secs_f32())
        }
        crate::jobs::Status::Done => format!("done in {:.1}s", job.took.unwrap_or(0.0)),
        crate::jobs::Status::Failed(e) => format!("failed: {e}"),
    };
    widgets::muted(ui, ctx.theme, &status);
    let retry = matches!(job.status, crate::jobs::Status::Failed(_))
        .then(|| (job.label.clone(), job.cmd.clone()));
    if let Some((label, cmd)) = retry
        && widgets::button(ui, ctx.theme, "Retry", widgets::Emphasis::Secondary).clicked()
    {
        state.shown_job = Some(ctx.jobs.spawn(label, cmd));
        return;
    }
    let Some(job) = ctx.jobs.get(state.shown_job.unwrap_or(id)) else {
        return;
    };
    egui::Frame::new()
        .fill(theme::rgb(ctx.theme.panel))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin(theme::SPACE_SM))
        .show(ui, |ui| {
            for line in job.lines.iter().rev().take(12).rev() {
                ui.label(egui::RichText::new(line).monospace().small());
            }
        });
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
    widgets::title(ui, ctx.theme, "Clips");
    let placeholder = theme::rgb(ctx.theme.raised);
    egui::ScrollArea::horizontal()
        .id_salt("night-clips")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, (id, path, duration)) in clips.iter().enumerate() {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(128.0, 72.0), egui::Sense::click());
                    ctx.thumbs
                        .paint(ui, rect, path, duration * 0.5, 0.5, placeholder);
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

/// Recently opened nights.
fn recent(ctx: &mut PageCtx<'_>, ui: &mut egui::Ui) {
    let recent = ctx.config.recent_projects.clone().unwrap_or_default();
    if recent.is_empty() {
        return;
    }
    ui.add_space(theme::SPACE_LG);
    widgets::title(ui, ctx.theme, "Recent nights");
    for path in recent {
        if ui.link(&path).clicked() {
            ctx.actions.push(Action::OpenProject(PathBuf::from(&path)));
        }
    }
}

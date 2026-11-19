// ============================================================================
// Module:       gui::pages::night::processing
// Description:  The processing page: overall bars per analysis step, elapsed
//               and remaining time, cancel, "Open editor" once indexing is
//               done, and every clip's status and progress.
//
// Dependencies: egui; crate::{jobs, project}, crate::gui::pages::night::{
//               pipeline, scan, wizard}
// ============================================================================

//! Processing.
//!
//! After Selects' processing view: one bar per step the approval queued,
//! each with its own remaining time, the run's elapsed and remaining time,
//! and Stop. Below, every clip: its frame, name, length, where it stands
//! (queued, analysing, indexed, captioned, failed and why) and, while
//! captions run, how many of its segments are done.
//!
//! **Keep working.** "Open editor" unlocks the moment the shot log exists
//! and no index job is pending; captions carry on in the background and
//! the shot log fills in as they land.
//!
//! What a clip shows comes from the cache, the shot log and
//! `<work>/status.json` (`crate::project::clip_rows`), overlaid with the
//! running job's live reports, so a night reopened after a crash or a
//! cancel shows exactly what is done.

use std::path::PathBuf;
use std::time::Instant;

use super::{pipeline, scan, wizard::Planned};
use crate::commands::ItemState;
use crate::gui::app::{Action, Page};
use crate::gui::pages::PageCtx;
use crate::gui::ui::{theme, widgets};
use crate::jobs::{Job, Status};
use crate::project::{self, ClipRow, RowState};

/// One analysis run the page follows.
#[derive(Debug, Clone)]
pub(crate) struct Run {
    /// The night.
    pub(crate) work: PathBuf,
    /// When it was queued.
    pub(crate) started: Instant,
    /// Its steps, in order.
    pub(crate) steps: Vec<Planned>,
}

/// The page's state.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The run being followed.
    pub(crate) run: Option<Run>,
    /// The footage scan behind the clip list.
    scan: scan::Scan,
}

impl State {
    /// Starts following a run of `steps` on `work`.
    pub(crate) fn begin(&mut self, work: PathBuf, steps: Vec<Planned>) {
        self.run = Some(Run {
            work,
            started: Instant::now(),
            steps,
        });
    }
}

/// Where a step of the run stands.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Phase {
    /// Waiting its turn.
    Queued,
    /// Running: done fraction (if known), seconds left (if known), a line.
    Running {
        /// Items done of the run.
        fraction: Option<f32>,
        /// Seconds left, from its reports.
        left: Option<f32>,
        /// Seconds since it started.
        elapsed: f32,
        /// `120/417 · 4 min left`.
        text: String,
    },
    /// Finished, in this many seconds.
    Done(f32),
    /// Failed, with the first line of the error.
    Failed(String),
    /// Stopped by the user.
    Cancelled,
    /// Never started (the queue stopped before it).
    Skipped,
}

/// Where a planned step stands, from its newest job since the run began
/// and whether it is still queued.
fn phase(job: Option<&Job>, queued: bool) -> Phase {
    match job {
        Some(j) => match &j.status {
            Status::Running => {
                let elapsed = j.started.elapsed().as_secs_f32();
                let p = &j.progress;
                let left = p.remaining(elapsed);
                let mut text = match (p.total, &p.stage) {
                    (0, Some(stage)) => stage.clone(),
                    (0, None) => "starting\u{2026}".to_owned(),
                    (total, _) => format!("{}/{total}", p.done),
                };
                if p.failed > 0 {
                    text.push_str(&format!(" \u{b7} {} failed", p.failed));
                }
                if j.cancelling() {
                    text.push_str(" \u{b7} stopping\u{2026}");
                }
                Phase::Running {
                    fraction: p.fraction(),
                    left,
                    elapsed,
                    text,
                }
            }
            Status::Done => Phase::Done(j.took.unwrap_or(0.0)),
            Status::Failed(_) if j.cancelled() => Phase::Cancelled,
            Status::Failed(e) => Phase::Failed(e.lines().next().unwrap_or_default().to_owned()),
        },
        None if queued => Phase::Queued,
        None => Phase::Skipped,
    }
}

/// Seconds the run still needs: what running steps report (or their
/// estimate less the time they have had), plus queued steps' estimates.
pub(crate) fn remaining(steps: &[(Phase, f32)]) -> f32 {
    steps
        .iter()
        .map(|(phase, estimate)| match phase {
            Phase::Queued => *estimate,
            Phase::Running {
                left: Some(left), ..
            } => *left,
            Phase::Running {
                left: None,
                elapsed,
                ..
            } => (estimate - elapsed).max(0.0),
            Phase::Done(_) | Phase::Failed(_) | Phase::Cancelled | Phase::Skipped => 0.0,
        })
        .sum()
}

/// "1:05", "12:40", "1:02:03".
fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Overlays the running jobs' live reports on the rows.
fn live(rows: &mut [ClipRow], index: Option<&Job>, caption: Option<&Job>) {
    if let Some(job) = index.filter(|j| j.status == Status::Running) {
        for row in rows.iter_mut() {
            let Some(item) = job.progress.items.get(&row.id) else {
                continue;
            };
            match item.state {
                ItemState::Done | ItemState::Cached => {
                    // A re-analysed clip keeps its captions.
                    if row.state != RowState::Captioned {
                        row.state = RowState::Indexed;
                    }
                    row.reason = None;
                }
                ItemState::Failed => {
                    row.state = RowState::Failed;
                    row.reason.clone_from(&item.reason);
                }
                ItemState::Queued | ItemState::Started => {}
            }
        }
    }
    if let Some(job) = caption.filter(|j| j.status == Status::Running) {
        for (id, item) in &job.progress.items {
            if item.state != ItemState::Done {
                continue;
            }
            let clip = id.rsplit_once('/').map_or(id.as_str(), |(c, _)| c);
            if let Some(row) = rows.iter_mut().find(|r| r.id == clip) {
                row.captions.0 = (row.captions.0 + 1).min(row.captions.1);
            }
        }
    }
}

/// What the page asks the night page to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Back to the night page.
    Back,
}

/// Draws the page.
pub(crate) fn draw(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    queue: &mut pipeline::State,
    ui: &mut egui::Ui,
) -> Option<Outcome> {
    let theme = ctx.theme;
    let (name, work, footage) = ctx
        .project
        .as_ref()
        .map(|p| (p.name(), p.work.clone(), p.settings.footage.clone()))?;
    let finished = ctx
        .jobs
        .list
        .iter()
        .filter(|j| j.status != Status::Running)
        .count();
    if let Some(footage) = &footage {
        state.scan.want(&work, footage, false, finished);
    }
    let run = state.run.clone().filter(|r| r.work == work);
    let since = run.as_ref().map(|r| r.started);
    let newest = |jobs: &crate::jobs::Jobs, label: &str| {
        jobs.list
            .iter()
            .rev()
            .find(|j| j.label == label && since.is_none_or(|s| j.started >= s))
            .map(|j| j.id)
    };
    let index_job = newest(ctx.jobs, "index");
    let caption_job = newest(ctx.jobs, "caption");
    let queued: Vec<String> = queue.queued().map(str::to_owned).collect();
    let phases: Vec<(Planned, Phase)> = run
        .as_ref()
        .map(|r| {
            r.steps
                .iter()
                .map(|s| {
                    let job = newest(ctx.jobs, &s.label).and_then(|id| ctx.jobs.get(id));
                    (s.clone(), phase(job, queued.contains(&s.label)))
                })
                .collect()
        })
        .unwrap_or_default();
    let left = remaining(
        &phases
            .iter()
            .map(|(s, p)| (p.clone(), s.secs))
            .collect::<Vec<_>>(),
    );
    let index_pending =
        queued.iter().any(|l| l == "index") || ctx.jobs.running().any(|j| j.label == "index");
    let indexed = ctx.project.as_ref().is_some_and(|p| p.index.is_some());
    let editor_ready = indexed && !index_pending;
    let active = queue.active();

    let mut outcome = None;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(if active {
                    format!("Analysing {name}")
                } else {
                    format!("{name}: analysis")
                })
                .font(theme::font(theme::HEADING_SIZE + 4.0))
                .color(theme::rgb(theme.text))
                .strong(),
            );
            let line = match (&run, active) {
                (Some(r), true) => format!(
                    "{} elapsed \u{b7} about {} left",
                    clock(r.started.elapsed().as_secs_f64()),
                    project::duration_text(left)
                ),
                (Some(r), false) => format!(
                    "Finished after {}",
                    clock(r.started.elapsed().as_secs_f64())
                ),
                (None, true) => "Running".to_owned(),
                (None, false) => "Nothing running. What each clip has been through:".to_owned(),
            };
            widgets::muted(ui, theme, &line);
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let open = ui
                .add_enabled_ui(editor_ready, |ui| {
                    widgets::button(ui, theme, "Open editor", widgets::Emphasis::Primary)
                })
                .inner
                .on_hover_text("Captions keep running in the background")
                .on_disabled_hover_text("Unlocks when indexing finishes")
                .clicked();
            if open {
                ctx.actions.push(Action::Go(Page::Review));
            }
            if active
                && widgets::button(ui, theme, "Stop", widgets::Emphasis::Secondary)
                    .on_hover_text(
                        "Stop after the clip or segment in hand; what is finished is kept, \
                         and the next run picks up from there",
                    )
                    .clicked()
            {
                queue.cancel(ctx.jobs);
            }
            if widgets::button(ui, theme, "Night page", widgets::Emphasis::Secondary).clicked() {
                outcome = Some(Outcome::Back);
            }
        });
    });
    if let Some(why) = queue.stopped() {
        ui.add_space(theme::SPACE_SM);
        ui.colored_label(theme::rgb(theme.warning), why);
    }
    ui.add_space(theme::SPACE_LG);
    if !phases.is_empty() {
        steps(ctx, ui, &phases);
        ui.add_space(theme::SPACE_LG);
    }
    let scanned = state.scan.for_work(&work).cloned();
    let mut rows = match &scanned {
        Some(s) => project::clip_rows(
            &s.clips,
            ctx.project.as_ref().and_then(|p| p.index.as_ref()),
            &s.status,
        ),
        None => Vec::new(),
    };
    let index_job = index_job.and_then(|id| ctx.jobs.get(id));
    live(
        &mut rows,
        index_job,
        caption_job.and_then(|id| ctx.jobs.get(id)),
    );
    let analysing: Vec<String> = index_job
        .filter(|j| j.status == Status::Running)
        .map(|j| j.progress.working().map(str::to_owned).collect())
        .unwrap_or_default();
    clips(ctx, ui, &rows, &analysing, scanned.is_none());
    outcome
}

/// The per-step bars.
fn steps(ctx: &PageCtx<'_>, ui: &mut egui::Ui, phases: &[(Planned, Phase)]) {
    let theme = ctx.theme;
    egui::Frame::new()
        .fill(theme::rgb(theme.panel))
        .stroke(egui::Stroke::new(1.0, theme::rgb(theme.border)))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(theme::margin(theme::SPACE_MD))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (step, phase) in phases {
                let (word, color, fraction, detail) = match phase {
                    Phase::Queued => (
                        "queued",
                        theme.text_faint,
                        0.0,
                        format!("\u{2248} {}", project::duration_text(step.secs)),
                    ),
                    Phase::Running {
                        fraction,
                        left,
                        text,
                        ..
                    } => (
                        "running",
                        theme.accent,
                        fraction.unwrap_or(0.0),
                        match left {
                            Some(l) => format!("{text} \u{b7} {} left", project::duration_text(*l)),
                            None => text.clone(),
                        },
                    ),
                    Phase::Done(took) => (
                        "done",
                        theme.success,
                        1.0,
                        format!("took {}", project::duration_text(*took)),
                    ),
                    Phase::Failed(e) => ("failed", theme.danger, 0.0, e.clone()),
                    Phase::Cancelled => {
                        ("stopped", theme.warning, 0.0, "stopped by you".to_owned())
                    }
                    Phase::Skipped => ("skipped", theme.text_faint, 0.0, "did not run".to_owned()),
                };
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(150.0, 20.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        egui::pos2(rect.left() + 4.0, rect.center().y),
                        4.0,
                        theme::rgb(color),
                    );
                    ui.painter().text(
                        egui::pos2(rect.left() + 14.0, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &step.title,
                        theme::font(theme::BODY_SIZE),
                        theme::rgb(theme.text),
                    );
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(70.0, 20.0), egui::Sense::hover());
                    ui.painter().text(
                        rect.left_center(),
                        egui::Align2::LEFT_CENTER,
                        word,
                        theme::font(theme::BODY_SIZE - 2.0),
                        theme::rgb(theme.text_muted),
                    );
                    let bar_w = (ui.available_width() - 300.0).max(80.0);
                    ui.allocate_ui(egui::vec2(bar_w, 20.0), |ui| {
                        ui.set_width(bar_w);
                        ui.add_space(7.0);
                        widgets::meter(
                            ui,
                            theme,
                            egui::Id::unique(("processing-step", &step.label)),
                            fraction,
                            color,
                        );
                    });
                    ui.label(
                        egui::RichText::new(detail)
                            .small()
                            .color(theme::rgb(theme.text_muted)),
                    );
                    if matches!(phase, Phase::Running { fraction: None, .. }) {
                        ui.add(egui::Spinner::new().size(12.0));
                    }
                });
                ui.add_space(theme::SPACE_XS);
            }
        });
}

/// The per-clip list.
fn clips(
    ctx: &mut PageCtx<'_>,
    ui: &mut egui::Ui,
    rows: &[ClipRow],
    analysing: &[String],
    reading: bool,
) {
    let theme = ctx.theme;
    let done = rows
        .iter()
        .filter(|r| matches!(r.state, RowState::Indexed | RowState::Captioned))
        .count();
    let failed = rows.iter().filter(|r| r.state == RowState::Failed).count();
    let captions: (usize, usize) = rows
        .iter()
        .fold((0, 0), |a, r| (a.0 + r.captions.0, a.1 + r.captions.1));
    ui.horizontal(|ui| {
        widgets::title(ui, theme, &format!("Clips \u{b7} {}", rows.len()));
        let mut summary = format!("{done} indexed");
        if captions.1 > 0 {
            summary.push_str(&format!(
                " \u{b7} {}/{} segments captioned",
                captions.0, captions.1
            ));
        }
        if failed > 0 {
            summary.push_str(&format!(" \u{b7} {failed} failed"));
        }
        widgets::muted(ui, theme, &summary);
        if reading {
            ui.add(egui::Spinner::new().size(12.0));
        }
    });
    ui.add_space(theme::SPACE_SM);
    let height = 46.0;
    for row in rows {
        let width = ui.available_width();
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let working = analysing.contains(&row.id);
        let painter = ui.painter();
        if response.hovered() {
            painter.rect_filled(
                rect,
                egui::CornerRadius::same(theme::RADIUS),
                theme::rgb(theme.hover),
            );
        }
        painter.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            egui::Stroke::new(1.0, theme::rgb(theme.border)),
        );
        let thumb = egui::Rect::from_min_size(
            rect.left_top() + egui::vec2(theme::SPACE_XS, 5.0),
            egui::vec2(64.0, 36.0),
        );
        let t = row.duration.map_or(1.0, |d| d * 0.4);
        ctx.thumbs
            .paint(ui, thumb, &row.path, t, 0.5, theme::rgb(theme.raised));
        let painter = ui.painter();
        let y = rect.center().y;
        let x = thumb.right() + theme::SPACE_MD;
        painter.text(
            egui::pos2(x, y),
            egui::Align2::LEFT_CENTER,
            &row.id,
            theme::font(theme::BODY_SIZE),
            theme::rgb(theme.text),
        );
        painter.text(
            egui::pos2(x + 170.0, y),
            egui::Align2::LEFT_CENTER,
            row.duration.map_or_else(|| "\u{2014}".to_owned(), clock),
            theme::font(theme::BODY_SIZE - 1.0),
            theme::rgb(theme.text_muted),
        );
        let (word, color) = if working {
            ("analysing", theme.accent)
        } else {
            match row.state {
                RowState::Pending => ("queued", theme.text_faint),
                RowState::Indexed => ("indexed", theme.warning),
                RowState::Captioned => ("captioned", theme.success),
                RowState::Failed => ("failed", theme.danger),
            }
        };
        let sx = x + 250.0;
        painter.circle_filled(egui::pos2(sx + 4.0, y), 4.0, theme::rgb(color));
        painter.text(
            egui::pos2(sx + 14.0, y),
            egui::Align2::LEFT_CENTER,
            word,
            theme::font(theme::BODY_SIZE - 1.0),
            theme::rgb(theme.text),
        );
        let px = sx + 120.0;
        let bar_w = (rect.right() - px - theme::SPACE_MD).clamp(60.0, 260.0);
        if let Some(reason) = &row.reason {
            let galley = painter.layout(
                reason.clone(),
                theme::font(theme::BODY_SIZE - 2.0),
                theme::rgb(theme.danger),
                rect.right() - px - theme::SPACE_MD,
            );
            painter.galley(
                egui::pos2(px, y - galley.size().y / 2.0),
                galley,
                theme::rgb(theme.danger),
            );
        } else if row.captions.1 > 0 {
            let bar = egui::Rect::from_min_size(egui::pos2(px, y - 3.0), egui::vec2(bar_w, 6.0));
            let f = row.captions.0 as f32 / row.captions.1 as f32;
            painter.rect_filled(bar, egui::CornerRadius::same(3), theme::rgb(theme.hover));
            painter.rect_filled(
                egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * f, bar.height())),
                egui::CornerRadius::same(3),
                theme::rgb(if f >= 1.0 {
                    theme.success
                } else {
                    theme.accent
                }),
            );
            painter.text(
                egui::pos2(bar.right() + theme::SPACE_SM, y),
                egui::Align2::LEFT_CENTER,
                format!("{}/{} captioned", row.captions.0, row.captions.1),
                theme::font(theme::BODY_SIZE - 2.0),
                theme::rgb(theme.text_muted),
            );
        } else if working {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(
                egui::Rect::from_center_size(egui::pos2(px + 8.0, y), egui::vec2(16.0, 16.0)),
            ));
            child.add(egui::Spinner::new().size(12.0));
        }
        if let Some(reason) = &row.reason {
            response.on_hover_text(reason);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Phase, clock, live, remaining};
    use crate::project::{ClipRow, RowState};

    #[test]
    fn remaining_time_adds_reports_and_estimates() {
        let steps = [
            (Phase::Done(30.0), 40.0),
            (
                Phase::Running {
                    fraction: Some(0.5),
                    left: Some(100.0),
                    elapsed: 100.0,
                    text: String::new(),
                },
                500.0,
            ),
            (
                Phase::Running {
                    fraction: None,
                    left: None,
                    elapsed: 15.0,
                    text: String::new(),
                },
                20.0,
            ),
            (Phase::Queued, 60.0),
            (Phase::Skipped, 99.0),
        ];
        let left = remaining(&steps);
        assert!(
            (left - 165.0).abs() < 1e-3,
            "100 reported + 5 of an estimate + 60 queued, done and skipped count nothing: {left}"
        );
    }

    #[test]
    fn clocks_read_as_minutes_and_hours() {
        assert_eq!(clock(65.0), "1:05");
        assert_eq!(clock(3723.0), "1:02:03");
    }

    #[test]
    fn rows_without_live_jobs_are_left_alone() {
        let mut rows = vec![ClipRow {
            id: "A".to_owned(),
            path: "A.mov".into(),
            duration: Some(3.0),
            state: RowState::Pending,
            reason: None,
            captions: (0, 2),
        }];
        live(&mut rows, None, None);
        assert_eq!(
            rows[0].state,
            RowState::Pending,
            "nothing running, nothing changes"
        );
    }
}

// ============================================================================
// Module:       gui::pages::night::processing
// Description:  The processing page: a centred step list (spinner or check
//               per analysis step), elapsed and remaining time, Stop, "Open
//               editor" once indexing is done, and every clip's status.
//
// Dependencies: egui; crate::{jobs, project}, crate::gui::pages::night::{
//               parts, pipeline, scan, wizard}
// ============================================================================

//! Processing.
//!
//! After Selects' processing view, as a centred column: a card listing the
//! steps the approval queued, each with a spinner, check or cross, a bar
//! while it runs and its own remaining time; the run's elapsed and
//! remaining time; and Stop as an outline button. Below, every clip as a
//! 40 px row: its frame, name, length, where it stands
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

use super::parts::{self, clock};
use super::{pipeline, scan, wizard::Planned};
use crate::commands::ItemState;
use crate::gui::app::{Action, Page};
use crate::gui::pages::PageCtx;
use crate::gui::ui::{icons, theme, widgets};
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
    /// Whether the guide has already opened the editor on its first cut.
    pub(crate) handed_over: bool,
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
            handed_over: false,
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
    /// Finished, in this many seconds, with this many items skipped
    /// (clips the index could not read).
    Done(f32, usize),
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
            Status::Done => Phase::Done(j.took.unwrap_or(0.0), j.progress.failed),
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
            Phase::Done(..) | Phase::Failed(_) | Phase::Cancelled | Phase::Skipped => 0.0,
        })
        .sum()
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

/// The column's widest.
const COLUMN: f32 = 640.0;

/// Draws the page.
pub(crate) fn draw(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    queue: &mut pipeline::State,
    ui: &mut egui::Ui,
) -> Option<Outcome> {
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

    let theme = ctx.theme;
    let mut outcome = None;
    parts::centred(ui, COLUMN, |ui| {
        ui.add_space((ui.clip_rect().height() * 0.08).clamp(theme::SPACE_XL, 72.0));
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = theme::SPACE_XS;
            ui.label(
                egui::RichText::new(if active {
                    format!("Analysing {name}")
                } else {
                    format!("{name}: analysis")
                })
                .font(theme::semibold(ui.ctx(), theme::HEADING_SIZE + 4.0))
                .color(theme::rgb(theme.text)),
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
            ui.label(
                egui::RichText::new(line)
                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(theme.text_muted)),
            );
        });
        ui.add_space(theme::SPACE_XL);
        if let Some(why) = queue.stopped() {
            parts::notice(ui, theme, parts::Kind::Warning, why);
            ui.add_space(theme::SPACE_MD);
        }
        if let Some(why) = queue.skipped() {
            parts::notice(ui, theme, parts::Kind::Info, &why);
            ui.add_space(theme::SPACE_MD);
        }
        if !phases.is_empty() {
            steps(ctx, ui, &phases);
            ui.add_space(theme::SPACE_LG);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
            if widgets::button_with(
                ui,
                theme,
                widgets::ButtonKind::Ghost,
                Some(icons::CHEVRON_LEFT),
                "Home",
            )
            .clicked()
            {
                outcome = Some(Outcome::Back);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
                let open = ui
                    .add_enabled_ui(editor_ready, |ui| {
                        widgets::button_with(
                            ui,
                            theme,
                            widgets::ButtonKind::Key,
                            None,
                            "Open editor",
                        )
                    })
                    .inner
                    .on_hover_text("Captions keep running in the background")
                    .on_disabled_hover_text("Unlocks when indexing finishes")
                    .clicked();
                if open {
                    ctx.actions.push(Action::Go(Page::Review));
                }
                if active
                    && widgets::outline_button(ui, theme, "Stop")
                        .on_hover_text(
                            "Stop after the clip or segment in hand; what is finished is kept, \
                             and the next run picks up from there",
                        )
                        .clicked()
                {
                    queue.cancel(ctx.jobs);
                }
            });
        });
        ui.add_space(theme::SPACE_XL + theme::SPACE_SM);
        clips(ctx, ui, &rows, &analysing, scanned.is_none());
        ui.add_space(theme::SPACE_XL * 2.0);
    });
    outcome
}

/// A step's mark, the line on its right, and its done fraction.
fn look(step: &Planned, phase: &Phase) -> (parts::Mark, String, Option<f32>) {
    match phase {
        Phase::Queued => (
            parts::Mark::Queued,
            format!("\u{2248} {}", project::duration_text(step.secs)),
            None,
        ),
        Phase::Running {
            fraction,
            left,
            text,
            ..
        } => (
            parts::Mark::Running,
            match left {
                Some(l) => format!("{text} \u{b7} {} left", project::duration_text(*l)),
                None => text.clone(),
            },
            Some(fraction.unwrap_or(0.0)),
        ),
        Phase::Done(took, skipped) => (
            parts::Mark::Done,
            if *skipped > 0 {
                format!(
                    "took {} \u{b7} {skipped} skipped",
                    project::duration_text(*took)
                )
            } else {
                format!("took {}", project::duration_text(*took))
            },
            None,
        ),
        Phase::Failed(e) => (parts::Mark::Failed, e.clone(), None),
        Phase::Cancelled => (parts::Mark::Stopped, "stopped by you".to_owned(), None),
        Phase::Skipped => (parts::Mark::Skipped, "did not run".to_owned(), None),
    }
}

/// The step list: a card of rows, each with its mark (spinner, check,
/// cross), its name, where it stands on the right and, while it runs, a
/// progress bar.
fn steps(ctx: &PageCtx<'_>, ui: &mut egui::Ui, phases: &[(Planned, Phase)]) {
    // A step's row, and the extra a running one takes for its bar.
    const ROW: f32 = 44.0;
    const BAR_ROW: f32 = 16.0;
    let theme = ctx.theme;
    egui::Frame::new()
        .fill(theme::rgb(theme.panel))
        .stroke(theme::ink_line(theme))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_XL))
        .inner_margin(theme::margin_xy(theme::SPACE_LG, theme::SPACE_XS))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, (step, phase)) in phases.iter().enumerate() {
                let (mark, detail, fraction) = look(step, phase);
                let height = ROW + if fraction.is_some() { BAR_ROW } else { 0.0 };
                let width = ui.available_width();
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
                if i > 0 {
                    ui.painter()
                        .hline(rect.x_range(), rect.top(), theme::ink_line(theme));
                }
                let y = rect.top() + ROW / 2.0;
                let mark_rect = egui::Rect::from_center_size(
                    egui::pos2(rect.left() + icons::SIZE / 2.0, y),
                    egui::Vec2::splat(icons::SIZE),
                );
                parts::mark(ui, theme, mark_rect, mark);
                let done = matches!(mark, parts::Mark::Done | parts::Mark::Running);
                let x = mark_rect.right() + theme::SPACE_MD;
                let detail_galley = ui.painter().layout_no_wrap(
                    detail.clone(),
                    theme::tabular(ui.ctx(), theme::BODY_SIZE),
                    theme::rgb(theme.text_muted),
                );
                let full_w = detail_galley.size().x;
                let detail_w = full_w.min((rect.width() * 0.55).max(0.0));
                let detail_rect = egui::Rect::from_min_max(
                    egui::pos2(rect.right() - detail_w, rect.top()),
                    egui::pos2(rect.right(), rect.top() + ROW),
                );
                let detail_color = if matches!(mark, parts::Mark::Failed) {
                    theme::rgb(theme.danger)
                } else {
                    theme::rgb(theme.text_muted)
                };
                ui.painter().with_clip_rect(detail_rect).galley(
                    egui::pos2(detail_rect.left(), y - detail_galley.size().y / 2.0),
                    detail_galley,
                    detail_color,
                );
                ui.painter().text(
                    egui::pos2(x, y),
                    egui::Align2::LEFT_CENTER,
                    &step.title,
                    theme::font(theme::LARGE_SIZE),
                    theme::rgb(if done || mark == parts::Mark::Failed {
                        theme.text
                    } else {
                        theme.text_muted
                    }),
                );
                if let Some(f) = fraction {
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(x, rect.top() + ROW - 4.0),
                        egui::vec2(rect.right() - x, 8.0),
                    );
                    parts::paint_progress(
                        ui,
                        theme,
                        bar,
                        egui::Id::unique(("processing-step", &step.label)),
                        f,
                    );
                }
                if detail_w < full_w {
                    response.on_hover_text(detail);
                }
            }
        });
}

/// The per-clip list: a label with the counts, then a 40 px row per clip
/// (frame, name, length, captions done, a badge for where it stands).
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
        ui.set_min_height(theme::CONTROL_HEIGHT);
        parts::label(ui, theme, &format!("Clips \u{b7} {}", rows.len()));
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(summary)
                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(theme.text_muted)),
            );
            if reading {
                ui.add(
                    egui::Spinner::new()
                        .size(icons::SMALL - 2.0)
                        .color(theme::rgb(theme.text_muted)),
                );
            }
        });
    });
    ui.add_space(theme::SPACE_XS);
    ui.spacing_mut().item_spacing.y = 2.0;
    for row in rows {
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(width, super::recent::ROW_HEIGHT),
            egui::Sense::hover(),
        );
        if !ui.is_rect_visible(rect) {
            continue;
        }
        parts::row_fill(ui, theme, &response, false);
        let working = analysing.contains(&row.id);
        let thumb = egui::Rect::from_min_size(
            egui::pos2(rect.left() + theme::SPACE_XS + 2.0, rect.center().y - 13.5),
            egui::vec2(48.0, 27.0),
        );
        let t = row.duration.map_or(1.0, |d| d * 0.4);
        parts::thumb(ctx.thumbs, ui, theme, thumb, Some((row.path.as_path(), t)));
        let y = rect.center().y;
        let (word, tone) = if working {
            ("analysing", widgets::Tone::Neutral)
        } else {
            match row.state {
                RowState::Pending => ("queued", widgets::Tone::Neutral),
                RowState::Indexed => ("indexed", widgets::Tone::Neutral),
                RowState::Captioned => ("captioned", widgets::Tone::Success),
                RowState::Failed => ("failed", widgets::Tone::Danger),
            }
        };
        let badge_w =
            super::recent::paint_badge(ui, theme, rect.right() - theme::SPACE_SM, y, word, tone);
        let mut right = rect.right() - theme::SPACE_SM - badge_w - theme::SPACE_MD;
        if working && row.captions.1 == 0 {
            let spin = egui::Rect::from_center_size(
                egui::pos2(right - icons::SMALL / 2.0, y),
                egui::Vec2::splat(icons::SMALL),
            );
            parts::mark(ui, theme, spin, parts::Mark::Running);
            right = spin.left() - theme::SPACE_MD;
        }
        if row.reason.is_none() && row.captions.1 > 0 {
            let count = format!("{}/{}", row.captions.0, row.captions.1);
            let galley = ui.painter().layout_no_wrap(
                count,
                theme::tabular(ui.ctx(), theme::SMALL_SIZE),
                theme::rgb(theme.text_muted),
            );
            let text_x = right - galley.size().x;
            ui.painter().galley(
                egui::pos2(text_x, y - galley.size().y / 2.0),
                galley,
                theme::rgb(theme.text_muted),
            );
            let bar = egui::Rect::from_min_size(
                egui::pos2(text_x - theme::SPACE_SM - 96.0, y - 3.0),
                egui::vec2(96.0, 6.0),
            );
            parts::paint_progress(
                ui,
                theme,
                bar,
                egui::Id::unique(("processing-clip", &row.id)),
                row.captions.0 as f32 / row.captions.1 as f32,
            );
            right = bar.left() - theme::SPACE_MD;
        }
        let x = thumb.right() + theme::SPACE_SM + 2.0;
        let room = egui::Rect::from_min_max(
            egui::pos2(x, rect.top()),
            egui::pos2(right.max(x), rect.bottom()),
        );
        let painter = ui.painter().with_clip_rect(room.intersect(ui.clip_rect()));
        let name = painter.layout_no_wrap(
            row.id.clone(),
            theme::font(theme::LARGE_SIZE),
            theme::rgb(theme.text),
        );
        let name_w = name.size().x;
        painter.galley(
            egui::pos2(x, y - name.size().y / 2.0),
            name,
            theme::rgb(theme.text),
        );
        let length = row.duration.map_or_else(|| "\u{2014}".to_owned(), clock);
        let after = match &row.reason {
            Some(reason) => (format!("{length} \u{b7} {reason}"), theme.danger),
            None => (length, theme.text_muted),
        };
        painter.text(
            egui::pos2(x + name_w + theme::SPACE_SM, y),
            egui::Align2::LEFT_CENTER,
            after.0,
            theme::tabular(ui.ctx(), theme::BODY_SIZE),
            theme::rgb(after.1),
        );
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
            (Phase::Done(30.0, 0), 40.0),
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

// ============================================================================
// Module:       gui::pages::deliver
// Description:  Export: the recipe's deliverables × plan versions, check
//               status, file names before render, the render queue, and the
//               delivered files.
//
// Dependencies: egui, rfd; crate::{commands, plan, recipe},
//               crate::gui::pages (PageCtx)
// ============================================================================

//! The export panel, docked on the right of the workspace.
//!
//! - **Matrix**: one row per deliverable of the night's recipe, one cell per
//!   plan version of it (`reel-30`, `reel-30-v2`…). Each cell carries the
//!   plan's check (✔ / ✖ with the problems on hover) and its length. Tick
//!   cells; the row shows the exact file name each tick delivers as,
//!   before anything renders (`docs/research/02-ui-and-workflow.md` §9.8).
//! - **Render**: queues a render (+ OTIO) per ticked cell, serially. A plan
//!   failing its check is never rendered without an explicit "render
//!   anyway".
//! - **Delivered**: what the queue produced and what is already in the
//!   output folder, each with reveal / open folder / copy path.

pub(crate) mod files;
pub(crate) mod queue;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::PageCtx;
use crate::commands::Cmd;
use crate::gui::ui::{modal, theme, widgets};
use crate::recipe::{self, Recipe};

/// A plan's check, cached by file modification time.
#[derive(Clone, Debug)]
struct Check {
    /// The plan file's modification time when checked.
    mtime: Option<SystemTime>,
    /// The plan's own name (render output is named after it).
    name: String,
    /// Problems, one line each; empty when it passes.
    problems: Vec<String>,
    /// Its length in seconds, when the music map is loaded.
    seconds: Option<f32>,
    /// Why it could not be loaded, if it could not.
    error: Option<String>,
}

/// The recipe as loaded for the open night.
#[derive(Debug)]
struct Loaded {
    /// The night.
    work: PathBuf,
    /// The recipe.
    recipe: Recipe,
    /// `recipe.toml`'s modification time when read or written.
    mtime: Option<SystemTime>,
}

/// The panel's state, kept across frames and page switches.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The recipe.
    loaded: Option<Loaded>,
    /// Plan checks, by plan path.
    checks: BTreeMap<PathBuf, Check>,
    /// When checks were last refreshed, and the project's state then.
    checked: Option<(Instant, usize)>,
    /// Ticked cells, by plan path.
    ticked: BTreeSet<PathBuf>,
    /// A render of failing plans waiting for "render anyway".
    confirm: Option<Vec<PathBuf>>,
    /// The render queue.
    queue: queue::Queue,
    /// The last error (saving the recipe).
    error: Option<String>,
}

/// Work the panel does every frame, shown or not: advancing the queue.
pub(crate) fn tick(jobs: &mut crate::jobs::Jobs, state: &mut State, ctx: &egui::Context) {
    state.queue.tick(jobs, Some(ctx));
}

/// `recipe.toml`'s modification time.
fn recipe_mtime(work: &Path) -> Option<SystemTime> {
    std::fs::metadata(work.join(recipe::FILE))
        .and_then(|m| m.modified())
        .ok()
}

impl State {
    /// Loads or reloads the recipe, and re-checks plans that changed.
    fn sync(&mut self, ctx: &PageCtx<'_>) {
        let Some(p) = ctx.project.as_ref() else {
            self.loaded = None;
            return;
        };
        let mtime = recipe_mtime(&p.work);
        let stale = self
            .loaded
            .as_ref()
            .is_none_or(|l| l.work != p.work || l.mtime != mtime);
        if stale {
            if self.loaded.as_ref().is_some_and(|l| l.work != p.work) {
                self.ticked.clear();
                self.checks.clear();
            }
            let (recipe, _, _) = Recipe::load(&p.work);
            self.loaded = Some(Loaded {
                work: p.work.clone(),
                recipe,
                mtime,
            });
        }
        let finished = ctx
            .jobs
            .list
            .iter()
            .filter(|j| j.status != crate::jobs::Status::Running)
            .count();
        let due = self
            .checked
            .is_none_or(|(at, n)| at.elapsed() > Duration::from_secs(2) || n != finished);
        if !due {
            return;
        }
        let all_changed = self.checked.is_none_or(|(_, n)| n != finished);
        self.checked = Some((Instant::now(), finished));
        self.checks.retain(|path, _| p.plans.contains(path));
        for path in &p.plans {
            let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
            let current = self
                .checks
                .get(path)
                .is_some_and(|c| c.mtime == mtime && !all_changed);
            if current {
                continue;
            }
            let check = match crate::plan::Plan::load(path) {
                Ok(plan) => {
                    let (problems, seconds) = match (&p.index, &p.music) {
                        (Some(index), Some(music)) => (
                            crate::plan::check(&plan, index, music, &p.people)
                                .iter()
                                .map(|pr| {
                                    if pr.shot > 0 {
                                        format!("shot {}: {}", pr.shot, pr.message)
                                    } else {
                                        pr.message.clone()
                                    }
                                })
                                .collect(),
                            Some(
                                music.beat_time(plan.end_bar, 0)
                                    - music.beat_time(plan.start_bar, 0),
                            ),
                        ),
                        (None, _) => (vec!["no shot log: run Index".to_owned()], None),
                        (Some(_), None) => (vec!["no music map: run Music".to_owned()], None),
                    };
                    Check {
                        mtime,
                        name: plan.name,
                        problems,
                        seconds,
                        error: None,
                    }
                }
                Err(e) => Check {
                    mtime,
                    name: String::new(),
                    problems: Vec::new(),
                    seconds: None,
                    error: Some(format!("{e:#}")),
                },
            };
            self.checks.insert(path.clone(), check);
        }
        self.ticked.retain(|path| p.plans.contains(path));
    }

    /// Saves an edited recipe.
    fn save(&mut self) {
        let Some(l) = &mut self.loaded else { return };
        match l.recipe.save(&l.work) {
            Ok(()) => {
                l.mtime = recipe_mtime(&l.work);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("Could not save the recipe: {e:#}")),
        }
    }
}

/// One cell of the matrix.
struct Cell {
    /// The plan file.
    path: PathBuf,
    /// Its version.
    version: String,
    /// The file name it delivers as.
    filename: String,
}

/// One row of the matrix.
struct Row {
    /// Index into the recipe's deliverables.
    deliverable: usize,
    /// Its plan versions.
    cells: Vec<Cell>,
}

/// The matrix rows from the recipe and the night's plans.
fn rows(recipe: &Recipe, plans: &[PathBuf], names: &recipe::Names) -> Vec<Row> {
    let matched = recipe.match_plans(plans);
    recipe
        .deliverables
        .iter()
        .enumerate()
        .map(|(i, d)| Row {
            deliverable: i,
            cells: matched
                .iter()
                .filter(|m| m.deliverable == i)
                .map(|m| Cell {
                    path: m.path.clone(),
                    version: m.version.clone(),
                    filename: recipe.filename(d, &m.version, names),
                })
                .collect(),
        })
        .collect()
}

/// Draws the panel.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    if ctx.project.is_none() {
        super::no_project(ctx, ui);
        return;
    }
    state.sync(ctx);
    let Some(work) = ctx.project.as_ref().map(|p| p.work.clone()) else {
        return;
    };
    let plans = ctx
        .project
        .as_ref()
        .map(|p| p.plans.clone())
        .unwrap_or_default();
    let recipe = state
        .loaded
        .as_ref()
        .map(|l| l.recipe.clone())
        .unwrap_or_default();
    let names = recipe::Names::for_work(&work);
    let out = recipe.output_dir(&work, &names);
    let rows = rows(&recipe, &plans, &names);

    egui::ScrollArea::vertical()
        .id_salt("deliver-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            summary(ctx, state, ui, &rows);
            ui.add_space(theme::SPACE_SM);
            matrix(ctx, state, ui, &rows, &recipe, &out);
            ui.add_space(theme::SPACE_MD);
            options(ctx, state, ui, &out);
            ui.add_space(theme::SPACE_MD);
            render_bar(ctx, state, ui, &rows, &recipe, &work, &out);
            ui.add_space(theme::SPACE_LG);
            queue_view(ctx, state, ui);
            delivered(ctx, ui, &recipe, &names, &out);
        });
    confirm(ctx, state, ui, &rows, &recipe, &work, &out);
}

/// The line under the title: how many deliverables, plans, failures.
fn summary(ctx: &PageCtx<'_>, state: &State, ui: &mut egui::Ui, rows: &[Row]) {
    let plans: usize = rows.iter().map(|r| r.cells.len()).sum();
    let failing = rows
        .iter()
        .flat_map(|r| &r.cells)
        .filter(|c| {
            state
                .checks
                .get(&c.path)
                .is_some_and(|k| !k.problems.is_empty() || k.error.is_some())
        })
        .count();
    let mut text = format!("{} deliverables · {plans} plans", rows.len());
    if failing > 0 {
        text.push_str(&format!(" · {failing} failing check"));
    }
    widgets::muted(ui, ctx.theme, &text);
}

/// The deliverables × versions matrix.
fn matrix(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    rows: &[Row],
    recipe: &Recipe,
    out: &Path,
) {
    let theme = ctx.theme;
    let mut draft: Option<(String, Cmd)> = None;
    for row in rows {
        let Some(d) = recipe.deliverables.get(row.deliverable) else {
            continue;
        };
        egui::Frame::new()
            .fill(theme::rgb(theme.raised))
            .stroke(egui::Stroke::new(1.0, theme::rgb(theme.border)))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS))
            .inner_margin(theme::margin(theme::SPACE_SM))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&d.id)
                            .strong()
                            .color(theme::rgb(theme.text)),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · {:.0}s",
                            recipe::aspect_key(d.aspect),
                            d.seconds
                        ))
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                    );
                });
                if row.cells.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("no plan yet")
                                .small()
                                .color(theme::rgb(theme.text_faint)),
                        );
                        let drafting = ctx
                            .jobs
                            .running()
                            .any(|j| j.label == format!("plan {}", d.id));
                        let ready = ctx
                            .project
                            .as_ref()
                            .is_some_and(|p| p.index.is_some() && p.music.is_some());
                        let label = if drafting { "Drafting…" } else { "Draft" };
                        if ui
                            .add_enabled(ready && !drafting, egui::Button::new(label).small())
                            .on_disabled_hover_text("Needs the shot log and the music map")
                            .clicked()
                            && let Some(p) = ctx.project.as_ref()
                        {
                            draft = Some((
                                format!("plan {}", d.id),
                                Recipe::plan_cmd(d, &p.work, &p.settings),
                            ));
                        }
                    });
                    return;
                }
                ui.horizontal_wrapped(|ui| {
                    for cell in &row.cells {
                        cell_chip(ui, theme, state, cell);
                    }
                });
                for cell in row.cells.iter().filter(|c| state.ticked.contains(&c.path)) {
                    let dest = out.join(&cell.filename);
                    let replaces = dest.is_file();
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("→")
                                .small()
                                .color(theme::rgb(theme.text_faint)),
                        );
                        ui.label(
                            egui::RichText::new(&cell.filename)
                                .small()
                                .monospace()
                                .color(theme::rgb(theme.text)),
                        )
                        .on_hover_text(dest.display().to_string());
                        if replaces {
                            ui.label(
                                egui::RichText::new("replaces the delivered file")
                                    .small()
                                    .color(theme::rgb(theme.warning)),
                            );
                        }
                    });
                }
            });
        ui.add_space(theme::SPACE_XS);
    }
    if let Some((label, cmd)) = draft {
        let _ = ctx.jobs.spawn(label, cmd);
    }
}

/// One plan version: tick box, version, ✔/✖, length.
fn cell_chip(ui: &mut egui::Ui, theme: &crate::theme::Palette, state: &mut State, cell: &Cell) {
    let check = state.checks.get(&cell.path).cloned();
    let (mark, color, tip) = match &check {
        Some(c) if c.error.is_some() => (
            "✖",
            theme.danger,
            format!("does not load: {}", c.error.clone().unwrap_or_default()),
        ),
        Some(c) if c.problems.is_empty() => ("✔", theme.success, "passes its check".to_owned()),
        Some(c) => (
            "✖",
            theme.danger,
            format!(
                "{} problem(s):\n{}",
                c.problems.len(),
                c.problems
                    .iter()
                    .take(12)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        ),
        None => ("…", theme.text_faint, "checking".to_owned()),
    };
    let mut on = state.ticked.contains(&cell.path);
    let length = check
        .as_ref()
        .and_then(|c| c.seconds)
        .map(|s| format!(" {s:.1}s"))
        .unwrap_or_default();
    let stem = cell
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let response = ui.checkbox(&mut on, cell.version.as_str());
        ui.label(egui::RichText::new(mark).color(theme::rgb(color)))
            .on_hover_text(&tip);
        ui.label(
            egui::RichText::new(length)
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        if response
            .on_hover_text(format!("{stem}.toml\n{tip}"))
            .changed()
        {
            if on {
                state.ticked.insert(cell.path.clone());
            } else {
                state.ticked.remove(&cell.path);
            }
        }
    });
    ui.add_space(theme::SPACE_SM);
}

/// Output folder, encoder, OTIO.
fn options(ctx: &PageCtx<'_>, state: &mut State, ui: &mut egui::Ui, out: &Path) {
    let theme = ctx.theme;
    let mut changed = false;
    if let Some(e) = &state.error {
        ui.colored_label(theme::rgb(theme.danger), e);
    }
    let Some(loaded) = state.loaded.as_mut() else {
        return;
    };
    egui::Grid::new("deliver-options")
        .num_columns(2)
        .spacing([theme::SPACE_MD, theme::SPACE_SM])
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Output").color(theme::rgb(theme.text_muted)));
            ui.horizontal_wrapped(|ui| {
                let shown = out.display().to_string();
                ui.label(egui::RichText::new(&shown).small())
                    .on_hover_text(&shown);
                if ui.small_button("Change…").clicked() {
                    let night = recipe::night_dir(&loaded.work);
                    if let Some(dir) = rfd::FileDialog::new()
                        .set_title("Where the night's videos go")
                        .set_directory(if out.is_dir() {
                            out.to_path_buf()
                        } else {
                            night.clone()
                        })
                        .pick_folder()
                    {
                        loaded.recipe.output_dir = Some(
                            dir.strip_prefix(&night)
                                .map_or(dir.clone(), Path::to_path_buf)
                                .display()
                                .to_string(),
                        );
                        changed = true;
                    }
                }
                if out.is_dir() && ui.small_button("Open").clicked() {
                    files::open_folder(out);
                }
            });
            ui.end_row();
            ui.label(egui::RichText::new("Encoder").color(theme::rgb(theme.text_muted)));
            ui.horizontal(|ui| {
                for choice in recipe::EncoderChoice::ALL {
                    changed |= ui
                        .selectable_value(&mut loaded.recipe.encoder, choice, choice.label())
                        .changed();
                }
            });
            ui.end_row();
            ui.label(egui::RichText::new("Timeline").color(theme::rgb(theme.text_muted)));
            changed |= ui
                .checkbox(&mut loaded.recipe.otio, "OTIO for Resolve")
                .changed();
            ui.end_row();
        });
    if changed {
        state.save();
    }
}

/// Select all / none and the Render button.
fn render_bar(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    rows: &[Row],
    recipe: &Recipe,
    work: &Path,
    out: &Path,
) {
    let theme = ctx.theme;
    let passing: Vec<PathBuf> = rows
        .iter()
        .filter_map(|r| r.cells.first())
        .filter(|c| {
            state
                .checks
                .get(&c.path)
                .is_some_and(|k| k.problems.is_empty() && k.error.is_none())
        })
        .map(|c| c.path.clone())
        .collect();
    ui.horizontal(|ui| {
        if ui
            .small_button("Tick passing")
            .on_hover_text("Tick the main version of every deliverable whose plan passes its check")
            .clicked()
        {
            state.ticked.extend(passing.iter().cloned());
        }
        if ui.small_button("Clear").clicked() {
            state.ticked.clear();
        }
    });
    ui.add_space(theme::SPACE_SM);
    let n = state.ticked.len();
    let label = if n == 0 {
        "Render".to_owned()
    } else {
        format!("Render {n} selected")
    };
    let shortcut =
        ctx.keys_free(ui) && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
    let clicked = ui
        .add_enabled_ui(n > 0, |ui| {
            widgets::button(ui, theme, &label, widgets::Emphasis::Primary)
        })
        .inner
        .on_hover_text(
            "Renders one at a time, then copies each video to the output folder (Ctrl+Enter)",
        )
        .clicked();
    if !(clicked || (shortcut && n > 0)) {
        return;
    }
    let failing: Vec<PathBuf> = state
        .ticked
        .iter()
        .filter(|p| {
            state
                .checks
                .get(*p)
                .is_none_or(|k| !k.problems.is_empty() || k.error.is_some())
        })
        .cloned()
        .collect();
    if failing.is_empty() {
        let ticked: Vec<PathBuf> = state.ticked.iter().cloned().collect();
        enqueue(state, rows, recipe, work, out, &ticked, &[]);
    } else {
        state.confirm = Some(failing);
    }
}

/// Queues render (+ OTIO) jobs for `paths`; `forced` are rendered despite
/// failing their check.
fn enqueue(
    state: &mut State,
    rows: &[Row],
    recipe: &Recipe,
    work: &Path,
    out: &Path,
    paths: &[PathBuf],
    forced: &[PathBuf],
) {
    let mut tasks = Vec::new();
    for row in rows {
        let Some(d) = recipe.deliverables.get(row.deliverable) else {
            continue;
        };
        for cell in row.cells.iter().filter(|c| paths.contains(&c.path)) {
            let name = state
                .checks
                .get(&cell.path)
                .map(|c| c.name.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| {
                    cell.path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
            let force = forced.contains(&cell.path);
            let renders = work.join("renders");
            tasks.push(queue::Task {
                label: format!("render {name}"),
                cmd: Cmd::Render {
                    plan: cell.path.clone(),
                    work: work.to_path_buf(),
                    encoder: recipe.encoder_for(d).encoder(),
                    force,
                },
                kind: queue::Kind::Render {
                    video: renders.join(format!("{name}.mp4")),
                    deliver: out.join(&cell.filename),
                    forced: force,
                },
            });
            if recipe.otio {
                tasks.push(queue::Task {
                    label: format!("otio {name}"),
                    cmd: Cmd::Otio {
                        plan: cell.path.clone(),
                        work: work.to_path_buf(),
                    },
                    kind: queue::Kind::Otio {
                        timeline: renders.join(format!("{name}.otio")),
                    },
                });
            }
        }
    }
    state.queue.push(tasks);
    state.ticked.clear();
}

/// "Render anyway?" for ticked plans that fail their check.
fn confirm(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    rows: &[Row],
    recipe: &Recipe,
    work: &Path,
    out: &Path,
) {
    let theme = ctx.theme;
    let failing = state.confirm.clone().unwrap_or_default();
    let lines: Vec<String> = failing
        .iter()
        .map(|p| {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            match state.checks.get(p) {
                Some(c) if c.error.is_some() => format!("{stem}: does not load"),
                Some(c) => format!(
                    "{stem}: {} problem(s), first: {}",
                    c.problems.len(),
                    c.problems.first().cloned().unwrap_or_default()
                ),
                None => format!("{stem}: not checked yet"),
            }
        })
        .collect();
    let outcome = modal::show(
        ui.ctx(),
        theme,
        "Plans fail their check",
        state.confirm.is_some(),
        460.0,
        |ui| {
            widgets::muted(
                ui,
                theme,
                "These plans break the edit rules. Fix them on the timeline, or render them \
                 anyway and the outputs are marked as unchecked.",
            );
            ui.add_space(theme::SPACE_SM);
            for line in &lines {
                ui.colored_label(theme::rgb(theme.danger), line);
            }
            ui.add_space(theme::SPACE_LG);
            let mut choice = None;
            ui.horizontal(|ui| {
                if widgets::button(ui, theme, "Render anyway", widgets::Emphasis::Primary).clicked()
                {
                    choice = Some(2);
                }
                if widgets::button(
                    ui,
                    theme,
                    "Only the passing ones",
                    widgets::Emphasis::Secondary,
                )
                .clicked()
                {
                    choice = Some(1);
                }
                if widgets::button(ui, theme, "Cancel", widgets::Emphasis::Secondary).clicked() {
                    choice = Some(0);
                }
            });
            choice
        },
    );
    let choice = outcome.inner.flatten();
    let ticked: Vec<PathBuf> = state.ticked.iter().cloned().collect();
    match choice {
        Some(2) => {
            state.confirm = None;
            enqueue(state, rows, recipe, work, out, &ticked, &failing);
        }
        Some(1) => {
            state.confirm = None;
            let passing: Vec<PathBuf> = ticked
                .iter()
                .filter(|p| !failing.contains(p))
                .cloned()
                .collect();
            enqueue(state, rows, recipe, work, out, &passing, &[]);
        }
        Some(_) => state.confirm = None,
        None if outcome.dismissed => state.confirm = None,
        None => {}
    }
}

/// The running job, what waits, copies in flight, failures.
fn queue_view(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let theme = ctx.theme;
    let q = &state.queue;
    if !q.busy() && q.failures.is_empty() && q.outputs.is_empty() {
        return;
    }
    widgets::title(ui, theme, "Queue");
    if let Some((id, task)) = &q.current {
        let elapsed = ctx
            .jobs
            .get(*id)
            .map_or(0.0, |j| j.started.elapsed().as_secs_f32());
        let last = ctx
            .jobs
            .get(*id)
            .and_then(|j| j.lines.last().cloned())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(egui::RichText::new(&task.label).strong());
            ui.label(
                egui::RichText::new(format!("{elapsed:.0}s")).color(theme::rgb(theme.text_muted)),
            );
        });
        let progress = ctx
            .jobs
            .get(*id)
            .and_then(|j| super::night::pipeline::progress(&j.lines));
        if let Some((n, m)) = progress {
            widgets::meter(
                ui,
                theme,
                egui::Id::unique("deliver-progress"),
                n as f32 / m as f32,
                theme.accent,
            );
        }
        if !last.is_empty() {
            ui.label(
                egui::RichText::new(last)
                    .small()
                    .monospace()
                    .color(theme::rgb(theme.text_faint)),
            );
        }
    }
    for task in &q.pending {
        ui.label(
            egui::RichText::new(format!("waiting: {}", task.label))
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
    }
    for to in q.copying() {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(12.0));
            ui.label(
                egui::RichText::new(format!(
                    "copying to {}",
                    to.file_name().unwrap_or_default().to_string_lossy()
                ))
                .small(),
            );
        });
    }
    for f in &q.failures {
        ui.colored_label(theme::rgb(theme.danger), f);
    }
    let mut clear = false;
    if !q.outputs.is_empty() {
        ui.add_space(theme::SPACE_SM);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Finished").strong());
            clear = ui.small_button("Clear").clicked();
        });
        for o in q.outputs.clone() {
            output_row(ui, theme, &o.path, o.what, o.bytes, o.forced);
        }
    }
    if clear {
        state.queue.outputs.clear();
        state.queue.failures.clear();
    }
    ui.add_space(theme::SPACE_LG);
}

/// One file with reveal / open folder / copy path.
fn output_row(
    ui: &mut egui::Ui,
    theme: &crate::theme::Palette,
    path: &Path,
    what: &str,
    bytes: Option<u64>,
    forced: bool,
) {
    ui.horizontal_wrapped(|ui| {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        ui.label(egui::RichText::new(name).monospace())
            .on_hover_text(path.display().to_string());
        let mut meta = what.to_owned();
        if let Some(b) = bytes {
            meta.push_str(&format!(" · {}", files::size(b)));
        }
        ui.label(
            egui::RichText::new(meta)
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        if forced {
            ui.label(
                egui::RichText::new("unchecked")
                    .small()
                    .color(theme::rgb(theme.warning)),
            )
            .on_hover_text("Rendered from a plan that fails its check");
        }
        if ui.small_button("Reveal").clicked() {
            files::reveal(path);
        }
        if let Some(dir) = path.parent()
            && ui.small_button("Folder").clicked()
        {
            files::open_folder(dir);
        }
        if ui.small_button("Copy path").clicked() {
            ui.ctx().copy_text(path.display().to_string());
        }
    });
}

/// What is already in the output folder under the recipe's names.
fn delivered(
    ctx: &PageCtx<'_>,
    ui: &mut egui::Ui,
    recipe: &Recipe,
    names: &recipe::Names,
    out: &Path,
) {
    let theme = ctx.theme;
    let found: Vec<(PathBuf, u64)> = recipe
        .deliverables
        .iter()
        .map(|d| out.join(recipe.filename(d, recipe::MAIN_VERSION, names)))
        .filter_map(|p| {
            let len = std::fs::metadata(&p).ok().filter(|m| m.is_file())?.len();
            Some((p, len))
        })
        .collect();
    widgets::title(
        ui,
        theme,
        &format!(
            "Delivered · {} of {}",
            found.len(),
            recipe.deliverables.len()
        ),
    );
    if found.is_empty() {
        widgets::muted(ui, theme, "Nothing in the output folder yet.");
        return;
    }
    for (path, bytes) in found {
        output_row(ui, theme, &path, "video", Some(bytes), false);
    }
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::recipe::{Names, Recipe};
    use std::path::PathBuf;

    #[test]
    fn every_deliverable_is_a_row_with_its_versions() {
        let recipe = Recipe::default();
        let names = Names {
            night: "2026-11-11".to_owned(),
            date: "2026-11-11".to_owned(),
        };
        let plans: Vec<PathBuf> = [
            "reel-30.toml",
            "reel-30-v2.toml",
            "other.toml",
            "recap.toml",
        ]
        .iter()
        .map(|p| PathBuf::from("plans").join(p))
        .collect();
        let rows = rows(&recipe, &plans, &names);
        assert_eq!(rows.len(), recipe.deliverables.len(), "one row each");
        let reel: Vec<&str> = rows
            .iter()
            .find(|r| {
                recipe
                    .deliverables
                    .get(r.deliverable)
                    .is_some_and(|d| d.id == "reel-30")
            })
            .map(|r| r.cells.iter().map(|c| c.filename.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(
            reel,
            ["reel-30.mp4", "reel-30-v2.mp4"],
            "main first, distinct names"
        );
        let total: usize = rows.iter().map(|r| r.cells.len()).sum();
        assert_eq!(total, 3, "plans outside the recipe are not listed");
    }
}

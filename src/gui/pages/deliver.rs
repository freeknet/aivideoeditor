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
//!   plan version of it (`reel-30`, `reel-30-v2`…) as a tickable chip with
//!   the plan's check (a check or a cross, the problems on hover) and its length. Tick
//!   cells; the row shows the exact file name each tick delivers as,
//!   before anything renders (`docs/research/02-ui-and-workflow.md` §9.8).
//!   Plans that are a version of no deliverable follow under *Other cuts*,
//!   delivered under their own name, so every plan on disk is listed.
//! - **Render**: queues a render (+ OTIO) per ticked cell, serially. A plan
//!   failing its check is never rendered without an explicit "render
//!   anyway", and a file in the output folder that ave did not make (the
//!   user's own edit) is never replaced without an explicit "replace them":
//!   the default is to deliver beside it as `<name>-ave.mp4`.
//! - **Delivered**: what ave rendered into the output folder, each with
//!   reveal / open folder / copy path.

pub(crate) mod files;
pub(crate) mod queue;

use crate::gui::ui::hint::Hint as _;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::PageCtx;
use super::night::parts;
use crate::commands::Cmd;
use crate::gui::ui::{icons, keymap, modal, theme, widgets};
use crate::recipe::{self, Recipe};

/// A plan's check, cached by file modification time.
#[derive(Clone, Debug)]
struct Check {
    /// The plan file's modification time when checked.
    mtime: Option<SystemTime>,
    /// The plan's own name (render output is named after it).
    name: String,
    /// Hard findings (the ones that fail the check), one line each; empty
    /// when it passes. Soft findings do not stop a render, so they are not
    /// listed here.
    problems: Vec<String>,
    /// Its length in seconds, when the music map is loaded.
    seconds: Option<f32>,
    /// Its frame shape, once loaded.
    aspect: Option<crate::plan::Aspect>,
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
    /// Whether this project's passing, undelivered cuts were ticked on
    /// their own (once, when nothing was ticked).
    auto_ticked: bool,
    /// A render of failing plans waiting for "render anyway".
    confirm: Option<Vec<PathBuf>>,
    /// A render waiting for the user to say what to do about files in the
    /// output folder that ave did not make.
    overwrite: Option<Overwrite>,
    /// The night's delivery manifest, with its file's modification time
    /// when read.
    manifest: (Option<SystemTime>, files::Manifest),
    /// The render queue.
    queue: queue::Queue,
    /// The last error (saving the recipe).
    error: Option<String>,
}

/// A render held back because some of its delivery names hold files ave
/// did not make.
#[derive(Clone, Debug)]
struct Overwrite {
    /// The plans to render.
    paths: Vec<PathBuf>,
    /// Of those, the ones rendered despite failing their check.
    forced: Vec<PathBuf>,
    /// The files in the way.
    files: Vec<PathBuf>,
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
                self.auto_ticked = false;
                self.checks.clear();
            }
            let (recipe, _, _) = Recipe::load(&p.work);
            self.loaded = Some(Loaded {
                work: p.work.clone(),
                recipe,
                mtime,
            });
        }
        let manifest_mtime = std::fs::metadata(files::Manifest::path(&p.work))
            .and_then(|m| m.modified())
            .ok();
        if stale || self.manifest.0 != manifest_mtime {
            self.manifest = (manifest_mtime, files::Manifest::load(&p.work));
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
                                .filter(|pr| pr.is_hard())
                                .map(crate::plan::Finding::text)
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
                        aspect: Some(plan.aspect),
                        error: None,
                    }
                }
                Err(e) => Check {
                    mtime,
                    name: String::new(),
                    problems: Vec::new(),
                    seconds: None,
                    aspect: None,
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

/// One row of the matrix: a deliverable of the recipe, or a cut that fits
/// none of them.
struct Row {
    /// Index into the recipe's deliverables; `None` for a cut that fits no
    /// deliverable (listed under "Other cuts", named after its plan).
    deliverable: Option<usize>,
    /// Its plan versions (one for another cut).
    cells: Vec<Cell>,
}

impl Row {
    /// The cell whose file stands for the row: the main version, else the
    /// first.
    fn main(&self) -> Option<&Cell> {
        self.cells
            .iter()
            .find(|c| c.version == recipe::MAIN_VERSION)
            .or_else(|| self.cells.first())
    }
}

/// The matrix rows from the recipe and the night's plans: one per
/// deliverable, then one per plan that is a version of none of them, so
/// every plan on disk is listed.
fn rows(recipe: &Recipe, plans: &[PathBuf], names: &recipe::Names) -> Vec<Row> {
    let matched = recipe.match_plans(plans);
    let mut rows: Vec<Row> = recipe
        .deliverables
        .iter()
        .enumerate()
        .map(|(i, d)| Row {
            deliverable: Some(i),
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
        .collect();
    for path in plans
        .iter()
        .filter(|p| !matched.iter().any(|m| &m.path == *p))
    {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        rows.push(Row {
            deliverable: None,
            cells: vec![Cell {
                path: path.clone(),
                version: recipe::MAIN_VERSION.to_owned(),
                filename: recipe::sanitize(&format!("{stem}.mp4"), "cut.mp4"),
            }],
        });
    }
    rows
}

/// `n` and the noun, singular or plural.
fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Draws the panel.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    if ctx.project.is_none() {
        let theme = ctx.theme;
        let mut pick = false;
        widgets::empty_state(
            ui,
            theme,
            icons::SHARE,
            "No project open",
            "Open a project to render its videos.",
            |ui| {
                pick = widgets::outline_button(ui, theme, "Open project\u{2026}").clicked();
            },
        );
        if pick {
            ctx.actions.push(crate::gui::app::Action::PickProject);
        }
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
            summary(ctx, state, ui, &rows, &out);
            ui.add_space(theme::SPACE_XS);
            matrix(ctx, state, ui, &rows, &recipe, &out);
            ui.add_space(theme::SPACE_LG);
            options(ctx, state, ui, &out);
            ui.add_space(theme::SPACE_SM);
            render_bar(ctx, state, ui, &rows, &recipe, &work, &out);
            ui.add_space(theme::SPACE_XL);
            queue_view(ctx, state, ui);
            delivered(ctx, state, ui, &rows, &out);
            ui.add_space(theme::SPACE_LG);
        });
    confirm(ctx, state, ui, &rows, &recipe, &work, &out);
    confirm_overwrite(ctx, state, ui, &rows, &recipe, &work, &out);
}

/// A group's label: 12 px Medium, muted, on a 28 px line.
fn group_label(ui: &mut egui::Ui, theme: &crate::theme::Palette, text: &str) {
    ui.horizontal(|ui| {
        ui.set_min_height(theme::CONTROL_HEIGHT);
        ui.add_space(theme::SPACE_XS);
        parts::label(ui, theme, text);
    });
}

/// Who the file `cell` delivers as belongs to.
fn owner(state: &State, out: &Path, cell: &Cell) -> files::Owner {
    state.manifest.1.owner(&out.join(&cell.filename))
}

/// The counts at the top (cuts, deliverables, failures), with tick
/// passing and clear.
fn summary(ctx: &PageCtx<'_>, state: &mut State, ui: &mut egui::Ui, rows: &[Row], out: &Path) {
    let theme = ctx.theme;
    let cuts: usize = rows.iter().map(|r| r.cells.len()).sum();
    let deliverables = rows.iter().filter(|r| r.deliverable.is_some()).count();
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
    let passing: Vec<PathBuf> = rows
        .iter()
        .filter_map(Row::main)
        .filter(|c| {
            state
                .checks
                .get(&c.path)
                .is_some_and(|k| k.problems.is_empty() && k.error.is_none())
        })
        .map(|c| c.path.clone())
        .collect();
    // Opened with nothing ticked: the cuts that pass and whose delivery name
    // is free are ticked, once per project, so Render is ready to press.
    if !state.auto_ticked && !passing.is_empty() {
        state.auto_ticked = true;
        if state.ticked.is_empty() {
            let free: Vec<PathBuf> = rows
                .iter()
                .filter_map(Row::main)
                .filter(|c| {
                    passing.contains(&c.path) && owner(state, out, c) == files::Owner::Absent
                })
                .map(|c| c.path.clone())
                .collect();
            state.ticked.extend(free);
        }
    }
    let mut text = format!(
        "{} \u{b7} {}",
        count(cuts, "cut", "cuts"),
        count(deliverables, "deliverable", "deliverables")
    );
    if failing > 0 {
        text.push_str(&format!(" \u{b7} {failing} failing"));
    }
    ui.horizontal(|ui| {
        ui.set_min_height(theme::CONTROL_HEIGHT);
        ui.add_space(theme::SPACE_XS);
        ui.label(
            egui::RichText::new(text)
                .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                .color(theme::rgb(theme.text_muted)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let none = state.ticked.is_empty();
            let clear = ui
                .add_enabled_ui(!none, |ui| {
                    widgets::icon_btn(ui, theme, icons::ROTATE_CCW, "Clear the ticks", "")
                })
                .inner;
            if clear.clicked() {
                state.ticked.clear();
            }
            let tick = ui
                .add_enabled_ui(!passing.is_empty(), |ui| {
                    widgets::icon_btn(
                        ui,
                        theme,
                        icons::LIST_FILTER,
                        "Tick the main version of every cut that passes its check",
                        "",
                    )
                })
                .inner;
            if tick.clicked() {
                state.ticked.extend(passing.iter().cloned());
            }
        });
    });
}

/// A plan's check as a mark and its colour, with the tooltip saying why.
fn check_look(
    theme: &crate::theme::Palette,
    check: Option<&Check>,
) -> (char, crate::color::Rgb, String) {
    match check {
        Some(c) if c.error.is_some() => (
            icons::CIRCLE_X,
            theme.danger,
            format!("does not load: {}", c.error.clone().unwrap_or_default()),
        ),
        Some(c) if c.problems.is_empty() => (
            icons::CIRCLE_CHECK,
            theme.success,
            "passes its check".to_owned(),
        ),
        Some(c) => (
            icons::CIRCLE_X,
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
        None => (icons::LOADER, theme.text_faint, "checking".to_owned()),
    }
}

/// The deliverables: a row each (name, shape and length, a status
/// badge), its plan versions as tickable chips, and the file name each
/// tick delivers as; hairlines between them. Cuts that fit no deliverable
/// follow under "Other cuts".
fn matrix(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    rows: &[Row],
    recipe: &Recipe,
    out: &Path,
) {
    let mut draft: Option<(String, Cmd)> = None;
    let mut first = true;
    for row in rows.iter().filter(|r| r.deliverable.is_some()) {
        matrix_row(ctx, state, ui, row, recipe, out, first, &mut draft);
        first = false;
    }
    let mut others = rows.iter().filter(|r| r.deliverable.is_none()).peekable();
    if others.peek().is_some() {
        ui.add_space(theme::SPACE_SM);
        group_label(ui, ctx.theme, "Other cuts");
        let mut first = true;
        for row in others {
            matrix_row(ctx, state, ui, row, recipe, out, first, &mut draft);
            first = false;
        }
    }
    if let Some((label, cmd)) = draft {
        // The job's id is not needed: it shows in the job list by its label.
        let _ = ctx.jobs.spawn(label, cmd);
    }
}

/// One row of the matrix. `draft` takes the plan step of a Draft press.
#[expect(
    clippy::too_many_arguments,
    reason = "one row of the matrix needs the panel's whole context"
)]
fn matrix_row(
    ctx: &PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    row: &Row,
    recipe: &Recipe,
    out: &Path,
    first: bool,
    draft: &mut Option<(String, Cmd)>,
) {
    let theme = ctx.theme;
    let deliverable = row.deliverable.and_then(|i| recipe.deliverables.get(i));
    let main = row.main();
    let check = main.and_then(|c| state.checks.get(&c.path));
    // The title and its shape: the recipe's for a deliverable, the plan's
    // own for another cut.
    let (title, shape) = match deliverable {
        Some(d) => (
            d.id.clone(),
            format!("{} \u{b7} {:.0} s", recipe::aspect_key(d.aspect), d.seconds),
        ),
        None => {
            let stem = main
                .and_then(|c| c.path.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut shape: Vec<String> = Vec::new();
            if let Some(a) = check.and_then(|c| c.aspect) {
                shape.push(recipe::aspect_key(a).to_owned());
            }
            if let Some(s) = check.and_then(|c| c.seconds) {
                shape.push(format!("{s:.0} s"));
            }
            (stem, shape.join(" \u{b7} "))
        }
    };
    if !first {
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter()
            .hline(rect.x_range(), rect.center().y, theme::ink_line(theme));
    }
    let main_owner = main.map_or(files::Owner::Absent, |c| owner(state, out, c));
    let main_fails = check.is_some_and(|k| !k.problems.is_empty() || k.error.is_some());
    let file = main.map(|c| c.filename.clone()).unwrap_or_default();
    let (word, tone, why) = if row.cells.is_empty() {
        (
            "no plan",
            widgets::Tone::Neutral,
            "No cut drafted for this deliverable yet".to_owned(),
        )
    } else if main_fails {
        (
            "failing",
            widgets::Tone::Danger,
            "The main version breaks the edit rules: hover its chip for why".to_owned(),
        )
    } else if main_owner == files::Owner::Ave {
        (
            "delivered",
            widgets::Tone::Success,
            format!("{file} is in the output folder, rendered by ave"),
        )
    } else if main_owner == files::Owner::Other {
        (
            "file exists",
            widgets::Tone::Warning,
            format!(
                "{file} is already in the output folder and ave did not make it: \
                 Render asks before replacing it"
            ),
        )
    } else {
        (
            "ready",
            widgets::Tone::Neutral,
            "Passes its check and is not rendered yet".to_owned(),
        )
    };
    let ready = ctx
        .project
        .as_ref()
        .is_some_and(|p| p.index.is_some() && p.music.is_some());
    egui::Frame::new()
        .inner_margin(theme::margin_xy(theme::SPACE_XS, theme::SPACE_SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_min_height(theme::CONTROL_HEIGHT);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(d) = deliverable
                        && row.cells.is_empty()
                    {
                        let drafting = ctx
                            .jobs
                            .running()
                            .any(|j| j.label == format!("plan {}", d.id));
                        let label = if drafting {
                            "Drafting\u{2026}"
                        } else {
                            "Draft"
                        };
                        let response = ui
                            .add_enabled_ui(ready && !drafting, |ui| {
                                widgets::button_with(
                                    ui,
                                    theme,
                                    widgets::ButtonKind::Outline,
                                    Some(icons::SPARKLES),
                                    label,
                                )
                            })
                            .inner;
                        let response = if ready {
                            response.hint(format!("Draft a first cut of {}", d.id))
                        } else {
                            response.on_disabled_hover_text("Needs the shot log and the music map")
                        };
                        if response.clicked()
                            && let Some(p) = ctx.project.as_ref()
                        {
                            *draft = Some((
                                format!("plan {}", d.id),
                                Recipe::plan_cmd(d, &p.work, &p.settings),
                            ));
                        }
                    }
                    widgets::pill(ui, theme, word, tone).hint(&why);
                    // The name and shape take what is left, cut short
                    // rather than pushing the badge out of the panel.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&title)
                                    .font(theme::medium(ui.ctx(), theme::LARGE_SIZE))
                                    .color(theme::rgb(theme.text)),
                            )
                            .truncate(),
                        );
                        if !shape.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&shape)
                                        .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                                        .color(theme::rgb(theme.text_muted)),
                                )
                                .truncate(),
                            );
                        }
                    });
                });
            });
            if row.cells.is_empty() {
                return;
            }
            ui.add_space(theme::SPACE_XS);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(theme::SPACE_XS, theme::SPACE_XS);
                for cell in &row.cells {
                    cell_chip(ui, theme, state, cell);
                }
            });
            for cell in row.cells.iter().filter(|c| state.ticked.contains(&c.path)) {
                let dest = out.join(&cell.filename);
                let who = owner(state, out, cell);
                ui.add_space(theme::SPACE_XS);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SPACE_XS + 2.0;
                    icons::icon(
                        ui,
                        icons::CHEVRON_RIGHT,
                        icons::SMALL,
                        theme::rgb(theme.text_faint),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&cell.filename)
                                .font(theme::mono(theme::SMALL_SIZE))
                                .color(theme::rgb(theme.text)),
                        )
                        .truncate(),
                    )
                    .hint(parts::shown_path(&dest));
                    match who {
                        files::Owner::Absent => {}
                        files::Owner::Ave => {
                            widgets::pill(ui, theme, "replaces", widgets::Tone::Neutral)
                                .hint("Replaces the video ave delivered under this name");
                        }
                        files::Owner::Other => {
                            widgets::pill(ui, theme, "file exists", widgets::Tone::Warning).hint(
                                "A file ave did not make has this name: Render asks before \
                                 replacing it",
                            );
                        }
                    }
                });
            }
        });
}

/// One plan version as a tickable chip: a check box, the version, its
/// check mark and its length. Ticked, it takes the selected fill.
fn cell_chip(ui: &mut egui::Ui, theme: &crate::theme::Palette, state: &mut State, cell: &Cell) {
    let check = state.checks.get(&cell.path).cloned();
    let (mark, color, tip) = check_look(theme, check.as_ref());
    let on = state.ticked.contains(&cell.path);
    let length = check
        .as_ref()
        .and_then(|c| c.seconds)
        .map(|s| format!("{s:.1} s"))
        .unwrap_or_default();
    let stem = cell
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = ui.painter().layout_no_wrap(
        cell.version.clone(),
        theme::medium(ui.ctx(), theme::BODY_SIZE),
        theme::rgb(theme.text),
    );
    let time = ui.painter().layout_no_wrap(
        length,
        theme::tabular(ui.ctx(), theme::SMALL_SIZE),
        theme::rgb(theme.text_muted),
    );
    let box_size = icons::SMALL;
    let gap = theme::SPACE_XS + 2.0;
    let width = theme::SPACE_SM
        + box_size
        + gap
        + name.size().x
        + gap
        + icons::SMALL
        + if time.size().x > 0.0 {
            gap + time.size().x
        } else {
            0.0
        }
        + theme::SPACE_SM;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, theme::CONTROL_HEIGHT),
        egui::Sense::click(),
    );
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, &stem));
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    let painter = ui.painter();
    let fill = if on {
        theme::selected_fill(theme)
    } else {
        theme::ink(
            theme,
            egui::lerp(f32::from(theme::INK_03)..=f32::from(theme::INK_08), lift).round() as u8,
        )
    };
    painter.rect(
        rect,
        egui::CornerRadius::same(theme::RADIUS),
        fill,
        theme::ink_line(theme),
        egui::StrokeKind::Inside,
    );
    let mut x = rect.left() + theme::SPACE_SM;
    let tick = egui::Rect::from_min_size(
        egui::pos2(x, rect.center().y - box_size / 2.0),
        egui::Vec2::splat(box_size),
    );
    widgets::paint_tick_box(painter, theme, tick, on, true);
    x = tick.right() + gap;
    let name_w = name.size().x;
    painter.galley(
        egui::pos2(x, rect.center().y - name.size().y / 2.0),
        name,
        theme::rgb(theme.text),
    );
    x += name_w + gap;
    let mark_rect = egui::Rect::from_min_size(
        egui::pos2(x, rect.center().y - icons::SMALL / 2.0),
        egui::Vec2::splat(icons::SMALL),
    );
    icons::paint(painter, mark_rect, mark, icons::SMALL, theme::rgb(color));
    x = mark_rect.right() + gap;
    let time_h = time.size().y;
    painter.galley(
        egui::pos2(x, rect.center().y - time_h / 2.0),
        time,
        theme::rgb(theme.text_muted),
    );
    widgets::focus_ring(ui, theme, &response);
    if response.hint(format!("{stem}.toml\n{tip}")).clicked() {
        if on {
            state.ticked.remove(&cell.path);
        } else {
            state.ticked.insert(cell.path.clone());
        }
    }
}

/// Output folder, encoder, OTIO, as form rows.
fn options(ctx: &PageCtx<'_>, state: &mut State, ui: &mut egui::Ui, out: &Path) {
    // The label column, narrower than the night page's: this is a panel.
    const LABEL: f32 = 72.0;
    let theme = ctx.theme;
    let mut changed = false;
    if let Some(e) = &state.error {
        parts::status_line(ui, theme, parts::Kind::Danger, e);
    }
    let Some(loaded) = state.loaded.as_mut() else {
        return;
    };
    group_label(ui, theme, "Output");
    let row = |ui: &mut egui::Ui, label: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            ui.set_min_height(theme::CONTROL_HEIGHT);
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(LABEL, theme::CONTROL_HEIGHT),
                egui::Sense::hover(),
            );
            ui.painter().text(
                rect.left_center() + egui::vec2(theme::SPACE_XS, 0.0),
                egui::Align2::LEFT_CENTER,
                label,
                theme::medium(ui.ctx(), theme::BODY_SIZE),
                theme::rgb(theme.text_muted),
            );
            add(ui);
        });
        ui.add_space(theme::SPACE_XS + 2.0);
    };
    row(ui, "Folder", &mut |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let open = ui
                .add_enabled_ui(out.is_dir(), |ui| {
                    widgets::icon_btn(
                        ui,
                        theme,
                        icons::EXTERNAL_LINK,
                        "Open the output folder",
                        "",
                    )
                })
                .inner;
            if open.clicked() {
                files::open_folder(out);
            }
            if widgets::icon_btn(
                ui,
                theme,
                icons::FOLDER_OPEN,
                "Change the output folder",
                "",
            )
            .clicked()
            {
                let night = recipe::night_dir(&loaded.work);
                if let Some(dir) = rfd::FileDialog::new()
                    .set_title("Where the project's videos go")
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
            ui.add_space(theme::SPACE_XS);
            let width = ui.available_width();
            parts::output_box(ui, theme, out, width);
        });
    });
    row(ui, "Encoder", &mut |ui| {
        changed |= crate::gui::pages::night::settings::encoder_picker(
            ui,
            theme,
            &mut loaded.recipe.encoder,
            true,
        );
    });
    changed |=
        crate::gui::pages::night::settings::nvenc_hint(ui, theme, &mut loaded.recipe.encoder);
    row(ui, "Timeline", &mut |ui| {
        changed |=
            widgets::switch(ui, theme, &mut loaded.recipe.otio, "OTIO for Resolve").changed();
    });
    if changed {
        state.save();
    }
}

/// The Render button: the panel's key action, right under the options and
/// right-aligned, with why it is off (or what it will ask) beside it.
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
    let n = state.ticked.len();
    let label = if n == 0 {
        "Render".to_owned()
    } else {
        format!("Render {}", count(n, "cut", "cuts"))
    };
    let in_the_way = rows
        .iter()
        .flat_map(|r| &r.cells)
        .filter(|c| state.ticked.contains(&c.path) && owner(state, out, c) == files::Owner::Other)
        .count();
    // Say why Render is off rather than leave it silently dim, and say
    // before the press when it will ask about files in the folder.
    let note = if n > 0 && in_the_way > 0 {
        format!(
            "Asks before replacing {}",
            count(in_the_way, "file", "files")
        )
    } else if n > 0 {
        String::new()
    } else if rows.iter().all(|r| r.cells.is_empty()) {
        "Draft a cut first, then render it".to_owned()
    } else {
        "Tick a cut above to render it".to_owned()
    };
    let preset = ctx.keymap;
    let shortcut = ctx.keys_free(ui) && ui.input(|i| keymap::pressed(i, "deliver-render", preset));
    let keys = keymap::command("deliver-render")
        .map(|c| c.hint(preset))
        .unwrap_or_default();
    let mut clicked = false;
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(width, theme::BAR_CONTROL),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(width, theme::BAR_CONTROL));
            ui.add_space(theme::SPACE_XS);
            // Off, it is a plain grey button, faded: the key colour is
            // only for the action that can be taken now.
            let kind = if n > 0 {
                widgets::ButtonKind::Key
            } else {
                widgets::ButtonKind::Secondary
            };
            let response = ui
                .add_enabled_ui(n > 0, |ui| {
                    widgets::button_sized(
                        ui,
                        theme,
                        kind,
                        Some(icons::PLAY),
                        &label,
                        theme::BAR_CONTROL - 4.0,
                    )
                })
                .inner;
            let response = if n > 0 {
                widgets::tip(
                    response,
                    theme,
                    "Renders one at a time, then copies each video to the output folder",
                    &keys,
                )
            } else {
                response.on_disabled_hover_text(&note)
            };
            clicked = response.clicked();
            if !note.is_empty() {
                ui.add_space(theme::SPACE_SM);
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&note)
                            .font(theme::font(theme::BODY_SIZE))
                            .color(theme::rgb(if in_the_way > 0 && n > 0 {
                                theme.warning
                            } else {
                                theme.text_muted
                            })),
                    )
                    .wrap(),
                );
            }
        },
    );
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
        start(state, rows, recipe, work, out, ticked, Vec::new());
    } else {
        state.confirm = Some(failing);
    }
}

/// Renders `paths` (`forced` despite failing their check), unless some of
/// them would deliver over a file ave did not make: then asks first.
fn start(
    state: &mut State,
    rows: &[Row],
    recipe: &Recipe,
    work: &Path,
    out: &Path,
    paths: Vec<PathBuf>,
    forced: Vec<PathBuf>,
) {
    let in_the_way: Vec<PathBuf> = rows
        .iter()
        .flat_map(|r| &r.cells)
        .filter(|c| paths.contains(&c.path) && owner(state, out, c) == files::Owner::Other)
        .map(|c| out.join(&c.filename))
        .collect();
    if in_the_way.is_empty() {
        enqueue(state, rows, recipe, work, out, &paths, &forced, false);
    } else {
        state.overwrite = Some(Overwrite {
            paths,
            forced,
            files: in_the_way,
        });
    }
}

/// Queues render (+ OTIO) jobs for `paths`; `forced` are rendered despite
/// failing their check. A delivery name holding a file ave did not make is
/// replaced only with `replace`; otherwise the video is delivered beside
/// it (`reel-30-ave.mp4`).
#[expect(
    clippy::too_many_arguments,
    reason = "the queue needs the rows, the recipe, both folders and the user's choices"
)]
fn enqueue(
    state: &mut State,
    rows: &[Row],
    recipe: &Recipe,
    work: &Path,
    out: &Path,
    paths: &[PathBuf],
    forced: &[PathBuf],
    replace: bool,
) {
    let mut tasks = Vec::new();
    for row in rows {
        let encoder = row
            .deliverable
            .and_then(|i| recipe.deliverables.get(i))
            .map_or(recipe.encoder, |d| recipe.encoder_for(d));
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
            let mut deliver = out.join(&cell.filename);
            if !replace && state.manifest.1.owner(&deliver) == files::Owner::Other {
                deliver = state.manifest.1.free_name(&deliver);
            }
            tasks.push(queue::Task {
                label: format!("render {name}"),
                cmd: Cmd::Render {
                    plan: cell.path.clone(),
                    work: work.to_path_buf(),
                    encoder: encoder.encoder(),
                    force,
                },
                kind: queue::Kind::Render {
                    video: renders.join(format!("{name}.mp4")),
                    deliver,
                    forced: force,
                    replace,
                    work: work.to_path_buf(),
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
            parts::muted(
                ui,
                theme,
                "These plans break the edit rules. Fix them on the timeline, or render them \
                 anyway and the outputs are marked as unchecked.",
            );
            ui.add_space(theme::SPACE_MD);
            for line in &lines {
                parts::status_line(ui, theme, parts::Kind::Danger, line);
            }
            ui.add_space(theme::SPACE_LG);
            let mut choice = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
                if widgets::primary_button(ui, theme, "Render anyway").clicked() {
                    choice = Some(2);
                }
                if widgets::outline_button(ui, theme, "Only the passing ones").clicked() {
                    choice = Some(1);
                }
                if widgets::ghost_button(ui, theme, "Cancel").clicked() {
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
            start(state, rows, recipe, work, out, ticked, failing);
        }
        Some(1) => {
            state.confirm = None;
            let passing: Vec<PathBuf> = ticked
                .iter()
                .filter(|p| !failing.contains(p))
                .cloned()
                .collect();
            start(state, rows, recipe, work, out, passing, Vec::new());
        }
        Some(_) => state.confirm = None,
        None if outcome.dismissed => state.confirm = None,
        None => {}
    }
}

/// What to do about delivery names that hold files ave did not make:
/// deliver beside them (the default), replace them, or cancel. Lists every
/// file in the way.
fn confirm_overwrite(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    rows: &[Row],
    recipe: &Recipe,
    work: &Path,
    out: &Path,
) {
    let theme = ctx.theme;
    let pending = state.overwrite.clone();
    let in_the_way = pending
        .as_ref()
        .map(|o| o.files.clone())
        .unwrap_or_default();
    let beside = match in_the_way.as_slice() {
        [one] => format!(
            "Render as {}",
            state
                .manifest
                .1
                .free_name(one)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        ),
        _ => "Render beside them (-ave)".to_owned(),
    };
    let outcome = modal::show(
        ui.ctx(),
        theme,
        "Files already in the folder",
        pending.is_some(),
        480.0,
        |ui| {
            parts::muted(
                ui,
                theme,
                "ave did not make these files, so they may be your own edits. Render the new \
                 videos beside them under another name, or replace them.",
            );
            ui.add_space(theme::SPACE_MD);
            for file in &in_the_way {
                let name = file
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // Its size and age, so the user can tell their own edit.
                let line = std::fs::metadata(file).map_or(name.clone(), |m| {
                    let mut line = format!("{name} \u{b7} {}", files::size(m.len()));
                    if let Ok(t) = m.modified() {
                        line.push_str(&format!(
                            " \u{b7} changed {}",
                            super::night::pipeline::ago(t)
                        ));
                    }
                    line
                });
                parts::status_line(ui, theme, parts::Kind::Warning, &line);
            }
            ui.add_space(theme::SPACE_LG);
            let mut choice = None;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(theme::SPACE_SM, theme::SPACE_SM);
                if widgets::primary_button(ui, theme, &beside).clicked() {
                    choice = Some(1);
                }
                let replace = widgets::button_with(
                    ui,
                    theme,
                    widgets::ButtonKind::Destructive,
                    None,
                    "Replace them",
                );
                if replace
                    .hint("Overwrite these files with the new renders")
                    .clicked()
                {
                    choice = Some(2);
                }
                if widgets::ghost_button(ui, theme, "Cancel").clicked() {
                    choice = Some(0);
                }
            });
            choice
        },
    );
    let choice = outcome.inner.flatten();
    let Some(o) = pending else { return };
    match choice {
        Some(1) => {
            state.overwrite = None;
            enqueue(state, rows, recipe, work, out, &o.paths, &o.forced, false);
        }
        Some(2) => {
            state.overwrite = None;
            enqueue(state, rows, recipe, work, out, &o.paths, &o.forced, true);
        }
        Some(_) => state.overwrite = None,
        None if outcome.dismissed => state.overwrite = None,
        None => {}
    }
}

/// The running job (spinner, name, elapsed, progress bar, last line),
/// what waits, copies in flight, failures, and what finished.
fn queue_view(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let theme = ctx.theme;
    let q = &state.queue;
    if !q.busy() && q.failures.is_empty() && q.outputs.is_empty() {
        return;
    }
    group_label(ui, theme, "Queue");
    if let Some((id, task)) = &q.current {
        let elapsed = ctx
            .jobs
            .get(*id)
            .map_or(0.0, |j| j.started.elapsed().as_secs_f64());
        let last = ctx
            .jobs
            .get(*id)
            .and_then(|j| j.lines.last().cloned())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.set_min_height(theme::CONTROL_HEIGHT);
            ui.add_space(theme::SPACE_XS);
            let rect = ui
                .allocate_exact_size(egui::Vec2::splat(icons::SIZE), egui::Sense::hover())
                .0;
            parts::mark(ui, theme, rect, parts::Mark::Running);
            ui.label(
                egui::RichText::new(&task.label)
                    .font(theme::font(theme::LARGE_SIZE))
                    .color(theme::rgb(theme.text)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(parts::clock(elapsed))
                        .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                        .color(theme::rgb(theme.text_muted)),
                );
            });
        });
        let progress = ctx
            .jobs
            .get(*id)
            .and_then(|j| super::night::pipeline::progress(&j.lines));
        if let Some((n, m)) = progress {
            ui.add_space(theme::SPACE_XS);
            parts::progress_bar(
                ui,
                theme,
                egui::Id::unique("deliver-progress"),
                n as f32 / m as f32,
            );
            ui.add_space(theme::SPACE_XS);
            ui.label(
                egui::RichText::new(format!("{n} of {m}"))
                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(theme.text_muted)),
            );
        }
        if !last.is_empty() {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(last)
                        .font(theme::mono(theme::SMALL_SIZE))
                        .color(theme::rgb(theme.text_faint)),
                )
                .truncate(),
            );
        }
        ui.add_space(theme::SPACE_XS);
    }
    for task in &q.pending {
        ui.horizontal(|ui| {
            ui.set_min_height(theme::CONTROL_HEIGHT);
            ui.add_space(theme::SPACE_XS);
            icons::icon(ui, icons::CLOCK, icons::SIZE, theme::rgb(theme.text_faint));
            ui.label(
                egui::RichText::new(&task.label)
                    .font(theme::font(theme::LARGE_SIZE))
                    .color(theme::rgb(theme.text_muted)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::pill(ui, theme, "waiting", widgets::Tone::Neutral);
            });
        });
    }
    for to in q.copying() {
        ui.horizontal(|ui| {
            ui.set_min_height(theme::CONTROL_HEIGHT);
            ui.add_space(theme::SPACE_XS);
            let rect = ui
                .allocate_exact_size(egui::Vec2::splat(icons::SIZE), egui::Sense::hover())
                .0;
            parts::mark(ui, theme, rect, parts::Mark::Running);
            ui.label(
                egui::RichText::new(format!(
                    "Copying to {}",
                    to.file_name().unwrap_or_default().to_string_lossy()
                ))
                .font(theme::font(theme::BODY_SIZE))
                .color(theme::rgb(theme.text_muted)),
            );
        });
    }
    for f in &q.failures {
        parts::status_line(ui, theme, parts::Kind::Danger, f);
    }
    let mut clear = false;
    if !q.outputs.is_empty() {
        ui.add_space(theme::SPACE_SM);
        ui.horizontal(|ui| {
            ui.set_min_height(theme::CONTROL_HEIGHT);
            ui.add_space(theme::SPACE_XS);
            parts::label(ui, theme, "Finished");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                clear = widgets::ghost_button(ui, theme, "Clear").clicked();
            });
        });
        for o in q.outputs.clone() {
            output_row(ui, theme, &o.path, o.what, o.bytes, o.forced);
        }
    }
    if clear {
        state.queue.outputs.clear();
        state.queue.failures.clear();
    }
    ui.add_space(theme::SPACE_XL);
}

/// One file as a 40 px row: a file icon, its name, what it is and its
/// size, an "unchecked" badge for a forced render, and reveal, open folder
/// and copy path as icon buttons.
fn output_row(
    ui: &mut egui::Ui,
    theme: &crate::theme::Palette,
    path: &Path,
    what: &str,
    bytes: Option<u64>,
    forced: bool,
) {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut meta = what.to_owned();
    if let Some(b) = bytes {
        meta.push_str(&format!(" \u{b7} {}", files::size(b)));
    }
    let width = ui.available_width();
    ui.allocate_ui_with_layout(
        egui::vec2(width, 40.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_size(egui::vec2(width, 40.0));
            ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
            ui.add_space(theme::SPACE_XS);
            icons::icon(
                ui,
                if what == "video" {
                    icons::FILE_VIDEO
                } else {
                    icons::FILE_TEXT
                },
                icons::SIZE,
                theme::rgb(theme.text_muted),
            );
            let buttons = theme::CONTROL_HEIGHT * 3.0 + 4.0 + if forced { 80.0 } else { 0.0 };
            let room = (ui.available_width() - buttons).max(0.0);
            ui.allocate_ui_with_layout(
                egui::vec2(room, 40.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_width(room);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.add_space(3.0);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&name)
                                .font(theme::font(theme::BODY_SIZE))
                                .color(theme::rgb(theme.text)),
                        )
                        .truncate(),
                    )
                    .hint(parts::shown_path(path));
                    ui.label(
                        egui::RichText::new(&meta)
                            .font(theme::tabular(ui.ctx(), theme::SMALL_SIZE))
                            .color(theme::rgb(theme.text_muted)),
                    );
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if widgets::icon_btn(ui, theme, icons::COPY, "Copy path", "").clicked() {
                    ui.ctx().copy_text(parts::shown_path(path));
                }
                if let Some(dir) = path.parent()
                    && widgets::icon_btn(ui, theme, icons::FOLDER_OPEN, "Open folder", "").clicked()
                {
                    files::open_folder(dir);
                }
                if widgets::icon_btn(ui, theme, icons::EXTERNAL_LINK, "Reveal in Explorer", "")
                    .clicked()
                {
                    files::reveal(path);
                }
                if forced {
                    widgets::pill(ui, theme, "unchecked", widgets::Tone::Warning)
                        .hint("Rendered from a cut that fails its check");
                }
            });
        },
    );
}

/// What ave has delivered to the output folder, as rows with reveal, open
/// folder and copy path; a sentence when there is nothing yet. Files under
/// the delivery names that ave did not make are counted, not listed.
fn delivered(ctx: &PageCtx<'_>, state: &State, ui: &mut egui::Ui, rows: &[Row], out: &Path) {
    let theme = ctx.theme;
    let mut found: Vec<(PathBuf, u64)> = Vec::new();
    let mut others = 0;
    for cell in rows.iter().flat_map(|r| &r.cells) {
        let path = out.join(&cell.filename);
        match owner(state, out, cell) {
            files::Owner::Ave => {
                let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                found.push((path, bytes));
            }
            files::Owner::Other => others += 1,
            files::Owner::Absent => {}
        }
    }
    let done = rows
        .iter()
        .filter(|r| {
            r.main()
                .is_some_and(|c| owner(state, out, c) == files::Owner::Ave)
        })
        .count();
    ui.horizontal(|ui| {
        ui.set_min_height(theme::CONTROL_HEIGHT);
        ui.add_space(theme::SPACE_XS);
        parts::label(ui, theme, "Delivered");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(theme::SPACE_XS);
            let tone = if done > 0 && done == rows.len() {
                widgets::Tone::Success
            } else {
                widgets::Tone::Neutral
            };
            widgets::pill(ui, theme, &format!("{done} of {}", rows.len()), tone).hint(format!(
                "{done} of the {} rendered by ave and in the output folder",
                count(rows.len(), "cut", "cuts")
            ));
        });
    });
    if found.is_empty() {
        let sentence = if rows.iter().all(|r| r.cells.is_empty()) {
            "Nothing rendered yet. Draft a cut above, then press Render."
        } else {
            "Nothing rendered yet. Tick cuts above and press Render."
        };
        egui::Frame::new()
            .inner_margin(theme::margin_xy(theme::SPACE_MD, theme::SPACE_SM))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
                    icons::icon(ui, icons::FILM, icons::SMALL, theme::rgb(theme.text_faint));
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(sentence)
                                .font(theme::font(theme::BODY_SIZE))
                                .color(theme::rgb(theme.text_muted)),
                        )
                        .wrap(),
                    );
                });
            });
    }
    for (path, bytes) in found {
        output_row(ui, theme, &path, "video", Some(bytes), false);
    }
    if others > 0 {
        ui.add_space(theme::SPACE_XS);
        ui.horizontal(|ui| {
            ui.add_space(theme::SPACE_XS);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "{} under these names {} not made by ave and {} never replaced unasked.",
                        count(others, "file", "files"),
                        if others == 1 { "was" } else { "were" },
                        if others == 1 { "is" } else { "are" },
                    ))
                    .font(theme::font(theme::SMALL_SIZE))
                    .color(theme::rgb(theme.text_faint)),
                )
                .wrap(),
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::rows;
    use crate::recipe::{Names, Recipe};
    use std::path::PathBuf;

    #[test]
    fn every_deliverable_is_a_row_with_its_versions() {
        let recipe = Recipe::flow_night();
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
        assert_eq!(
            rows.iter().filter(|r| r.deliverable.is_some()).count(),
            recipe.deliverables.len(),
            "one row per deliverable"
        );
        let reel: Vec<&str> = rows
            .iter()
            .find(|r| {
                r.deliverable
                    .and_then(|i| recipe.deliverables.get(i))
                    .is_some_and(|d| d.id == "reel-30")
            })
            .map(|r| r.cells.iter().map(|c| c.filename.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(
            reel,
            ["reel-30.mp4", "reel-30-v2.mp4"],
            "main first, distinct names"
        );
        let others: Vec<&str> = rows
            .iter()
            .filter(|r| r.deliverable.is_none())
            .flat_map(|r| r.cells.iter().map(|c| c.filename.as_str()))
            .collect();
        assert_eq!(
            others,
            ["other.mp4"],
            "a plan outside the recipe is its own cut, named after it"
        );
    }

    /// A plan file the panel can list (it fails its check: no shot log).
    fn plan_text(name: &str) -> String {
        format!(
            "name = \"{name}\"\naspect = \"9x16\"\nmusic = \"\"\n\
             start_bar = 0\ndrop_bar = 1\ncard_bar = 2\nend_bar = 3\nshot = []\n"
        )
    }

    /// A night with no recipe, plans `names` and a `videos` folder beside
    /// it holding `reel-30.mp4`, a file ave did not make.
    fn night(names: &[&str]) -> anyhow::Result<(tempfile::TempDir, PathBuf)> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-11").join("_ave");
        std::fs::create_dir_all(work.join("plans"))?;
        for name in names {
            std::fs::write(
                work.join("plans").join(format!("{name}.toml")),
                plan_text(name),
            )?;
        }
        let videos = dir.path().join("2026-11-11").join("videos");
        std::fs::create_dir_all(&videos)?;
        std::fs::write(videos.join("reel-30.mp4"), b"the human's own edit")?;
        Ok((dir, work))
    }

    type Harness = egui_kittest::Harness<'static, Option<crate::gui::app::App>>;

    /// The app on `work` with the Export panel open and the timeline folded,
    /// headless.
    fn harness(work: PathBuf) -> Harness {
        use crate::gui::app::{Action, App, Page};
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1680.0, 1400.0))
            .with_accessibility_check(false)
            .build_ui_state(
                move |ui, app: &mut Option<App>| {
                    let app = app.get_or_insert_with(|| {
                        let mut app = App::new(
                            ui.ctx(),
                            crate::config::Config::default(),
                            Some(work.clone()),
                        );
                        app.apply(ui.ctx(), Action::Go(Page::Plan));
                        app.apply(ui.ctx(), Action::ToggleExport);
                        // Clicks land where the control is drawn: fold the
                        // timeline so the whole panel is on screen.
                        app.apply(
                            ui.ctx(),
                            Action::TogglePanel(crate::gui::ui::layout::Region::Timeline),
                        );
                        app
                    });
                    app.show(ui);
                },
                None,
            );
        h.run_steps(8);
        h
    }

    /// Clicks the node labelled exactly `label` with `role`.
    fn click(h: &Harness, role: egui::accesskit::Role, label: &str) -> anyhow::Result<()> {
        use egui_kittest::kittest::Queryable;
        h.query_all_by(|n| n.role() == role && n.label().is_some_and(|l| l == label))
            .next()
            .ok_or_else(|| anyhow::anyhow!("no {role:?} labelled {label:?}"))?
            .click();
        Ok(())
    }

    /// Whether any text on screen contains `text`.
    fn shows(h: &Harness, text: &str) -> bool {
        use egui_kittest::kittest::Queryable;
        h.query_all_by_label_contains(text).next().is_some()
    }

    #[test]
    fn with_no_recipe_every_plan_is_listed() -> anyhow::Result<()> {
        let (_dir, work) = night(&["reel-30", "teaser"])?;
        let h = harness(work);
        assert!(
            shows(&h, "2 cuts"),
            "the count names both plans, never \"0 plans\""
        );
        assert!(
            shows(&h, "Other cuts"),
            "a plan fitting no deliverable has its group"
        );
        for name in ["reel-30", "teaser"] {
            assert!(shows(&h, name), "{name} is listed (a tickable cut)");
        }
        Ok(())
    }

    #[test]
    fn nothing_rendered_says_so_in_a_sentence() -> anyhow::Result<()> {
        let (_dir, work) = night(&["teaser"])?;
        let h = harness(work);
        assert!(
            shows(
                &h,
                "Nothing rendered yet. Tick cuts above and press Render."
            ),
            "the empty Delivered section says what to do"
        );
        assert!(
            shows(&h, "Tick a cut above to render it"),
            "the dim Render button says why"
        );
        Ok(())
    }

    #[test]
    fn a_file_ave_did_not_make_is_not_replaced_without_asking() -> anyhow::Result<()> {
        use egui::accesskit::Role;
        let (dir, work) = night(&["reel-30"])?;
        let theirs = dir
            .path()
            .join("2026-11-11")
            .join("videos")
            .join("reel-30.mp4");
        let mut h = harness(work);
        click(&h, Role::CheckBox, "reel-30")?;
        h.run_steps(10);
        assert!(
            shows(&h, "file exists"),
            "the ticked cut says its name is taken"
        );
        assert!(
            shows(&h, "Asks before replacing 1 file"),
            "Render says it will ask"
        );
        click(&h, Role::Button, "Render 1 cut")?;
        h.run_steps(10);
        // The plan fails its check (no shot log): render it anyway.
        click(&h, Role::Button, "Render anyway")?;
        h.run_steps(10);
        assert!(
            shows(&h, "Files already in the folder"),
            "Render asks before replacing a file ave did not make"
        );
        let queued = |h: &Harness| {
            h.state().as_ref().map_or(0, |app| {
                let q = &app.pages.deliver.queue;
                q.pending.len() + usize::from(q.current.is_some())
            })
        };
        assert_eq!(queued(&h), 0, "nothing renders while it asks");
        click(&h, Role::Button, "Cancel")?;
        h.run_steps(10);
        assert_eq!(queued(&h), 0, "cancelled: nothing queued");
        assert_eq!(
            std::fs::read(&theirs)?,
            b"the human's own edit",
            "the file is untouched"
        );
        // Again (the tick stays after a cancel), and deliver beside it.
        click(&h, Role::Button, "Render 1 cut")?;
        h.run_steps(10);
        click(&h, Role::Button, "Render anyway")?;
        h.run_steps(10);
        click(&h, Role::Button, "Render as reel-30-ave.mp4")?;
        h.run_steps(1);
        let delivers: Vec<PathBuf> = h
            .state()
            .as_ref()
            .map(|app| {
                let q = &app.pages.deliver.queue;
                q.current
                    .iter()
                    .map(|(_, t)| t)
                    .chain(q.pending.iter())
                    .filter_map(|t| match &t.kind {
                        super::queue::Kind::Render { deliver, .. } => Some(deliver.clone()),
                        super::queue::Kind::Otio { .. } => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(
            delivers,
            [theirs.with_file_name("reel-30-ave.mp4")],
            "delivered beside the file, not over it"
        );
        Ok(())
    }
}

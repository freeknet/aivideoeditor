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
//! - **Render**: queues a render (+ OTIO) per ticked cell, serially. A plan
//!   failing its check is never rendered without an explicit "render
//!   anyway".
//! - **Delivered**: what the queue produced and what is already in the
//!   output folder, each with reveal / open folder / copy path.

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
            summary(ctx, state, ui, &rows);
            ui.add_space(theme::SPACE_XS);
            matrix(ctx, state, ui, &rows, &recipe, &out);
            ui.add_space(theme::SPACE_LG);
            options(ctx, state, ui, &out);
            ui.add_space(theme::SPACE_LG);
            render_bar(ctx, state, ui, &rows, &recipe, &work, &out);
            ui.add_space(theme::SPACE_XL);
            queue_view(ctx, state, ui);
            delivered(ctx, ui, &recipe, &names, &out);
            ui.add_space(theme::SPACE_LG);
        });
    confirm(ctx, state, ui, &rows, &recipe, &work, &out);
}

/// A group's label: 12 px Medium, muted, on a 28 px line.
fn group_label(ui: &mut egui::Ui, theme: &crate::theme::Palette, text: &str) {
    ui.horizontal(|ui| {
        ui.set_min_height(theme::CONTROL_HEIGHT);
        ui.add_space(theme::SPACE_XS);
        parts::label(ui, theme, text);
    });
}

/// The counts at the top (deliverables, plans, failures), with tick
/// passing and clear.
fn summary(ctx: &PageCtx<'_>, state: &mut State, ui: &mut egui::Ui, rows: &[Row]) {
    let theme = ctx.theme;
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
    // Opened with nothing ticked: the cuts that pass and are not delivered
    // yet are ticked, once per project, so Render is ready to press.
    if !state.auto_ticked && !passing.is_empty() {
        state.auto_ticked = true;
        if state.ticked.is_empty() {
            let out = state.loaded.as_ref().map(|l| {
                l.recipe
                    .output_dir(&l.work, &crate::recipe::Names::for_work(&l.work))
            });
            let undelivered = rows.iter().filter_map(|r| r.cells.first()).filter(|c| {
                passing.contains(&c.path)
                    && out.as_ref().is_none_or(|o| !o.join(&c.filename).is_file())
            });
            state.ticked.extend(undelivered.map(|c| c.path.clone()));
        }
    }
    let mut text = format!("{} deliverables \u{b7} {plans} plans", rows.len());
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
            if widgets::icon_btn(ui, theme, icons::ROTATE_CCW, "Clear the ticks", "").clicked() {
                state.ticked.clear();
            }
            if widgets::icon_btn(
                ui,
                theme,
                icons::LIST_FILTER,
                "Tick the main version of every deliverable whose plan passes its check",
                "",
            )
            .clicked()
            {
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
/// tick delivers as; hairlines between them.
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
    let ready = ctx
        .project
        .as_ref()
        .is_some_and(|p| p.index.is_some() && p.music.is_some());
    for (i, row) in rows.iter().enumerate() {
        let Some(d) = recipe.deliverables.get(row.deliverable) else {
            continue;
        };
        if i > 0 {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
            ui.painter()
                .hline(rect.x_range(), rect.center().y, theme::ink_line(theme));
        }
        let delivered = row
            .cells
            .iter()
            .find(|c| c.version == recipe::MAIN_VERSION)
            .is_some_and(|c| out.join(&c.filename).is_file());
        let main_fails = row.cells.first().is_some_and(|c| {
            state
                .checks
                .get(&c.path)
                .is_some_and(|k| !k.problems.is_empty() || k.error.is_some())
        });
        let (word, tone) = if row.cells.is_empty() {
            ("no plan", widgets::Tone::Neutral)
        } else if main_fails {
            ("failing", widgets::Tone::Danger)
        } else if delivered {
            ("delivered", widgets::Tone::Success)
        } else {
            ("ready", widgets::Tone::Neutral)
        };
        egui::Frame::new()
            .inner_margin(theme::margin_xy(theme::SPACE_XS, theme::SPACE_SM))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.set_min_height(theme::CONTROL_HEIGHT);
                    ui.label(
                        egui::RichText::new(&d.id)
                            .font(theme::medium(ui.ctx(), theme::LARGE_SIZE))
                            .color(theme::rgb(theme.text)),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "{} \u{b7} {:.0} s",
                            recipe::aspect_key(d.aspect),
                            d.seconds
                        ))
                        .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                        .color(theme::rgb(theme.text_muted)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if row.cells.is_empty() {
                            let drafting = ctx
                                .jobs
                                .running()
                                .any(|j| j.label == format!("plan {}", d.id));
                            let label = if drafting {
                                "Drafting\u{2026}"
                            } else {
                                "Draft"
                            };
                            let pressed = ui
                                .add_enabled_ui(ready && !drafting, |ui| {
                                    widgets::button_with(
                                        ui,
                                        theme,
                                        widgets::ButtonKind::Outline,
                                        Some(icons::SPARKLES),
                                        label,
                                    )
                                })
                                .inner
                                .on_disabled_hover_text("Needs the shot log and the music map")
                                .clicked();
                            if pressed && let Some(p) = ctx.project.as_ref() {
                                draft = Some((
                                    format!("plan {}", d.id),
                                    Recipe::plan_cmd(d, &p.work, &p.settings),
                                ));
                            }
                        }
                        widgets::pill(ui, theme, word, tone);
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
                    let replaces = dest.is_file();
                    ui.add_space(theme::SPACE_XS);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = theme::SPACE_XS + 2.0;
                        icons::icon(
                            ui,
                            icons::CHEVRON_RIGHT,
                            icons::SMALL,
                            theme::rgb(theme.text_faint),
                        );
                        ui.label(
                            egui::RichText::new(&cell.filename)
                                .font(theme::mono(theme::SMALL_SIZE))
                                .color(theme::rgb(theme.text)),
                        )
                        .hint(dest.display().to_string());
                        if replaces {
                            widgets::pill(ui, theme, "replaces", widgets::Tone::Warning)
                                .hint("Replaces the delivered file");
                        }
                    });
                }
            });
    }
    if let Some((label, cmd)) = draft {
        let _ = ctx.jobs.spawn(label, cmd);
    }
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
        let items: Vec<(recipe::EncoderChoice, &str)> = recipe::EncoderChoice::ALL
            .iter()
            .map(|c| (*c, c.short_label()))
            .collect();
        changed |= widgets::segmented(ui, theme, &mut loaded.recipe.encoder, &items);
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

/// The Render button: the panel's key action, with what it will do.
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
        format!("Render {n} selected")
    };
    let preset = ctx.keymap;
    let shortcut = ctx.keys_free(ui) && ui.input(|i| keymap::pressed(i, "deliver-render", preset));
    let keys = keymap::command("deliver-render")
        .map(|c| c.hint(preset))
        .unwrap_or_default();
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.add_space(theme::SPACE_XS);
        if n > 0 {
            ui.label(
                egui::RichText::new(format!("{n} ticked"))
                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(theme.text_muted)),
            );
        } else if rows.iter().any(|r| !r.cells.is_empty()) {
            // Say why Render is dim rather than leave it silently off.
            ui.label(
                egui::RichText::new("Tick a cut's version above to render it")
                    .font(theme::font(theme::SMALL_SIZE))
                    .color(theme::rgb(theme.text_muted)),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = ui
                .add_enabled_ui(n > 0, |ui| {
                    widgets::button_sized(
                        ui,
                        theme,
                        widgets::ButtonKind::Key,
                        Some(icons::PLAY),
                        &label,
                        theme::BAR_CONTROL - 4.0,
                    )
                })
                .inner;
            clicked = widgets::tip(
                response,
                theme,
                "Renders one at a time, then copies each video to the output folder",
                &keys,
            )
            .clicked();
        });
    });
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
                    .hint(path.display().to_string());
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
                    ui.ctx().copy_text(path.display().to_string());
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
                        .hint("Rendered from a plan that fails its check");
                }
            });
        },
    );
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
    ui.horizontal(|ui| {
        ui.set_min_height(theme::CONTROL_HEIGHT);
        ui.add_space(theme::SPACE_XS);
        parts::label(ui, theme, "Delivered");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let tone = if !found.is_empty() && found.len() == recipe.deliverables.len() {
                widgets::Tone::Success
            } else {
                widgets::Tone::Neutral
            };
            widgets::pill(
                ui,
                theme,
                &format!("{} of {}", found.len(), recipe.deliverables.len()),
                tone,
            );
        });
    });
    if found.is_empty() {
        // The empty state centres in the space it is given.
        const EMPTY_HEIGHT: f32 = 140.0;
        let width = ui.available_width();
        ui.allocate_ui(egui::vec2(width, EMPTY_HEIGHT), |ui| {
            ui.set_min_size(egui::vec2(width, EMPTY_HEIGHT));
            widgets::empty_state(
                ui,
                theme,
                icons::FILM,
                "Nothing delivered yet",
                "Renders are copied to the output folder under the recipe's names.",
                |_| {},
            );
        });
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

// ============================================================================
// Module:       gui::pages::music
// Description:  Music: the track's waveform, beat grid, sections and drops, with grid fixes.
//
// Dependencies: egui; crate::{beats, commands, refs}, crate::gui::pages (PageCtx)
// ============================================================================

//! The Music tab (browser pane), after FCP 12's beat grid and Resolve's
//! dual timeline (`docs/research/02-ui-and-workflow.md` §3.1, §9.6).
//!
//! - **Overview strip** of the whole track and a **zoomed view** with the
//!   beat grid ([`view`]); the waveform is decoded once on a worker
//!   ([`peaks`]). The playhead follows the shared player when it holds the
//!   track; clicking seeks, double-clicking plays.
//! - **Fix the grid:** drag a bar line in the ruler onto the beat that is
//!   really the "one", or `,` / `.` to move the downbeat a beat;
//!   `Shift+,` / `Shift+.` nudge the whole grid by 10 ms; type the tempo
//!   (or ×2 / ½); `D` marks or unmarks a drop at the bar under the
//!   pointer; rename a section; re-run the analysis. Every fix is written
//!   to `music.json` as a correction (the analysis itself is kept) and the
//!   project's map reloaded, so the planner, timeline and renderer see it.
//! - **Song ranges** per deliverable: select bars (drag, or `I` / `O`),
//!   then name the range (reel-15/30/60, feed-30, wide-60, recap).
//! - **References:** selecting bars sets the app selection to
//!   `@bars:a-b`; `R` / right-click → Reference in chat.
//!
//! Bars are numbered from 0, as in plans, references and the shot log.

mod peaks;
mod view;

use egui::RichText;

use super::PageCtx;
use crate::beats::{MusicMap, structure};
use crate::color::Rgb;
use crate::commands::Cmd;
use crate::gui::ui::{theme, widgets};
use crate::theme::Palette;

/// A correction to the music map, applied after the frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Edit {
    /// Move the downbeat by this many beats.
    ShiftDownbeat(i32),
    /// Move the whole grid by this many seconds.
    Nudge(f32),
    /// Set the tempo by hand (`None`: back to detected).
    SetBpm(Option<f32>),
    /// Forget offset, tempo and downbeat corrections.
    ResetGrid,
    /// Mark or unmark a drop at this bar.
    ToggleDrop(usize),
    /// Make the drop at this bar the main one.
    PrimaryDrop(usize),
    /// Rename section `i`.
    RenameSection(usize, String),
    /// Back to the detected sections.
    ResetSections,
    /// Set the named song range to bars `from..=to`.
    SetRange(String, usize, usize),
    /// Remove the named song range.
    RemoveRange(String),
}

/// What a drag in the zoomed view is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Drag {
    /// Nothing.
    #[default]
    None,
    /// Selecting bars from this one.
    Select {
        /// Where the drag started.
        from: usize,
    },
    /// Moving this bar's line (the downbeat).
    Grid {
        /// The bar being moved.
        bar: usize,
    },
    /// Scrubbing the playhead along the ruler.
    Scrub,
}

/// The page's own state, kept across frames and page switches.
#[derive(Debug)]
pub(crate) struct State {
    peaks: peaks::Cache,
    /// The zoomed view's first second.
    view_t0: f32,
    /// The zoomed view's width in seconds (0 until first shown).
    view_span: f32,
    /// Whether the beat grid is drawn (`0` toggles).
    show_grid: bool,
    /// Selected bars, inclusive.
    sel: Option<(usize, usize)>,
    /// Where Shift-click extends from.
    anchor: Option<usize>,
    /// The bar under the pointer.
    hover_bar: Option<usize>,
    /// The drag in progress.
    drag: Drag,
    /// Where the context menu was opened, seconds.
    menu_t: f32,
    /// The tempo being typed.
    bpm_edit: Option<f32>,
    /// A section being renamed: index and the text so far.
    rename: Option<(usize, String)>,
    /// Focus the rename field next frame.
    focus_rename: bool,
    /// The custom range name being typed.
    range_name: String,
    /// The re-analysis job.
    job: Option<u64>,
    /// The last save error.
    error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            peaks: peaks::Cache::default(),
            view_t0: 0.0,
            view_span: 0.0,
            show_grid: true,
            sel: None,
            anchor: None,
            hover_bar: None,
            drag: Drag::None,
            menu_t: 0.0,
            bpm_edit: None,
            rename: None,
            focus_rename: false,
            range_name: String::new(),
            job: None,
            error: None,
        }
    }
}

impl State {
    /// Keeps the zoomed window inside the track.
    fn clamp_view(&mut self, duration: f32) {
        let duration = duration.max(1.0);
        self.view_span = self.view_span.clamp(0.5, duration * 1.05);
        let lo = -0.02 * self.view_span;
        let hi = (duration - 0.98 * self.view_span).max(lo);
        self.view_t0 = self.view_t0.clamp(lo, hi);
    }

    /// Zooms by `factor` (above 1 shows more) keeping `at` where it is.
    fn zoom(&mut self, factor: f32, at: f32, duration: f32) {
        let old = self.view_span.max(1e-3);
        self.view_span = (old * factor).clamp(0.5, duration.max(1.0) * 1.05);
        self.view_t0 = at - (at - self.view_t0) * self.view_span / old;
        self.clamp_view(duration);
    }
}

/// The colour a section is drawn in.
pub(crate) fn section_color(theme: &Palette, name: &str) -> Rgb {
    match name {
        "drop" => theme.accent,
        "build" => theme.warning,
        "break" => theme.success,
        "intro" | "outro" => theme.text_faint,
        other => {
            let hash = other
                .bytes()
                .fold(7_usize, |h, b| h.wrapping_mul(31).wrapping_add(b.into()));
            theme.series(hash % 8, 8)
        }
    }
}

/// Draws the page into the central area.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let Some(project) = ctx.project.as_mut() else {
        super::no_project(ctx, ui);
        return;
    };
    let work = project.work.clone();
    let track = project.settings.track.clone();
    let Some(map) = project.music.take() else {
        no_music(ctx, state, ui, track, &work);
        return;
    };
    let mut map = map;
    let mut edits = Vec::new();
    body(ctx, state, ui, &map, &work, &mut edits);
    let changed = !edits.is_empty();
    for edit in edits {
        apply(&mut map, state, edit);
    }
    if changed {
        state.error = map
            .save(&work)
            .err()
            .map(|e| format!("saving music.json: {e:#}"));
    }
    if let Some(project) = ctx.project.as_mut() {
        project.music = Some(map);
        if changed && state.error.is_none() {
            project.reload_music();
        }
    }
}

/// Applies one correction.
fn apply(map: &mut MusicMap, state: &mut State, edit: Edit) {
    match edit {
        Edit::ShiftDownbeat(n) => map.shift_downbeat(n),
        Edit::Nudge(s) => map.nudge(s),
        Edit::SetBpm(bpm) => map.set_bpm(bpm),
        Edit::ResetGrid => map.reset_grid(),
        Edit::ToggleDrop(bar) => map.toggle_drop(bar),
        Edit::PrimaryDrop(bar) => map.make_primary_drop(bar),
        Edit::RenameSection(i, name) => map.rename_section(i, &name),
        Edit::ResetSections => map.reset_sections(),
        Edit::SetRange(name, from, to) => map.set_range(&name, from, to),
        Edit::RemoveRange(name) => map.remove_range(&name),
    }
    if let Some((a, b)) = state.sel {
        let last = map.bars.len().saturating_sub(1);
        state.sel = Some((a.min(last), b.min(last)));
    }
}

/// What the page shows before the music is mapped.
fn no_music(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    track: Option<std::path::PathBuf>,
    work: &std::path::Path,
) {
    widgets::muted(ui, ctx.theme, "The music is not mapped yet.");
    ui.add_space(theme::SPACE_SM);
    match track {
        Some(track) => {
            let running = job_running(ctx, state);
            ui.add_enabled_ui(!running, |ui| {
                let label = if running {
                    "Mapping…"
                } else {
                    "Map the music"
                };
                if widgets::button(ui, ctx.theme, label, widgets::Emphasis::Primary).clicked() {
                    state.job = Some(ctx.jobs.spawn(
                        "Map the music",
                        Cmd::Beats {
                            track,
                            work: work.to_path_buf(),
                        },
                    ));
                }
            });
        }
        None => {
            widgets::muted(
                ui,
                ctx.theme,
                "Choose the track on the Home screen, then map it.",
            );
        }
    }
}

/// Whether the re-analysis job is still running.
fn job_running(ctx: &PageCtx<'_>, state: &State) -> bool {
    state
        .job
        .and_then(|id| ctx.jobs.get(id))
        .is_some_and(|j| j.status == crate::jobs::Status::Running)
}

/// The page with a map.
fn body(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    map: &MusicMap,
    work: &std::path::Path,
    edits: &mut Vec<Edit>,
) {
    if state.view_span <= 0.0 {
        let first = map.bars.first().map_or(0.0, |b| b.t);
        state.view_span = (16.0 * 4.0 * map.period()).min(map.duration.max(1.0));
        state.view_t0 = first - 0.02 * state.view_span;
    }
    state.clamp_view(map.duration);
    let page = ui.max_rect();
    header(ctx, state, ui, map, work);
    ui.add_space(theme::SPACE_XS);
    toolbar(ctx, state, ui, map, edits);
    ui.add_space(theme::SPACE_XS);

    // The cache is borrowed for the frame; drawing needs `state` mutably too.
    let mut taken = std::mem::take(&mut state.peaks);
    let ready = taken.get(&map.path, ui.ctx());
    view::overview(ctx, state, ui, map, ready);
    ui.add_space(theme::SPACE_XS);
    view::zoomed(ctx, state, ui, map, ready, edits);
    if let Some(e) = taken.error() {
        widgets::muted(ui, ctx.theme, &format!("No waveform: {e}"));
    }
    state.peaks = taken;

    let pointer_here = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| page.contains(p));
    if pointer_here && ctx.keys_free(ui) {
        keys(ctx, state, ui, map, edits);
    }
    hint(ctx, ui);
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            selection_row(ctx, state, ui, map, edits);
            ui.add_space(theme::SPACE_SM);
            sections(ctx, state, ui, map, edits);
            ui.add_space(theme::SPACE_SM);
            drops(ctx, ui, map, edits);
            ui.add_space(theme::SPACE_SM);
            ranges(ctx, ui, map, edits);
        });
}

/// Track name, tempo, length, grid state and the re-analysis button.
fn header(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    map: &MusicMap,
    work: &std::path::Path,
) {
    let name = std::path::Path::new(&map.path)
        .file_name()
        .map_or_else(|| map.path.clone(), |n| n.to_string_lossy().into_owned());
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(name)
                .font(theme::font(theme::TITLE_SIZE))
                .color(theme::rgb(ctx.theme.text))
                .strong(),
        );
        ui.label(
            RichText::new(format!(
                "{:.2} BPM · {} bars · {}",
                map.bpm,
                map.bars.len(),
                clock(map.duration)
            ))
            .color(theme::rgb(ctx.theme.text_muted)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let running = job_running(ctx, state);
            let label = if running {
                "Analysing…"
            } else {
                "Re-analyse"
            };
            let button = ui
                .add_enabled(!running, egui::Button::new(label))
                .on_hover_text("Run the beat analysis again (your fixes are kept)");
            if button.clicked() {
                let track = std::path::PathBuf::from(&map.path);
                state.job = Some(ctx.jobs.spawn(
                    "Map the music",
                    Cmd::Beats {
                        track,
                        work: work.to_path_buf(),
                    },
                ));
            }
        });
    });
    let mut fixes = Vec::new();
    if map.downbeat_shift != 0 {
        fixes.push(format!("downbeat {:+} beats", map.downbeat_shift));
    }
    if map.grid_offset.abs() > 1e-6 {
        fixes.push(format!("offset {:+.0} ms", map.grid_offset * 1000.0));
    }
    if map.bpm_override.is_some() {
        fixes.push(format!(
            "tempo {:.2} (detected {:.2})",
            map.bpm,
            map.detected_bpm()
        ));
    }
    let line = if fixes.is_empty() {
        "Grid as detected".to_owned()
    } else {
        format!("Grid fixed: {}", fixes.join(", "))
    };
    ui.horizontal(|ui| {
        widgets::muted(ui, ctx.theme, &line);
        if let Some(e) = &state.error {
            ui.label(RichText::new(e).color(theme::rgb(ctx.theme.danger)));
        }
    });
}

/// Tempo, downbeat and offset controls.
fn toolbar(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    map: &MusicMap,
    edits: &mut Vec<Edit>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Tempo");
        let mut bpm = state.bpm_edit.unwrap_or(map.bpm);
        let field = ui.add(
            egui::DragValue::new(&mut bpm)
                .speed(0.01)
                .range(40.0..=300.0)
                .max_decimals(2)
                .suffix(" BPM"),
        );
        if field.changed() {
            state.bpm_edit = Some(bpm);
        }
        if (field.drag_stopped() || field.lost_focus())
            && let Some(v) = state.bpm_edit.take()
            && (v - map.bpm).abs() > 1e-3
        {
            edits.push(Edit::SetBpm(Some(v)));
        }
        if ui
            .small_button("×2")
            .on_hover_text("Double the tempo")
            .clicked()
        {
            edits.push(Edit::SetBpm(Some(map.bpm * 2.0)));
        }
        if ui
            .small_button("½")
            .on_hover_text("Halve the tempo")
            .clicked()
        {
            edits.push(Edit::SetBpm(Some(map.bpm / 2.0)));
        }
        if map.bpm_override.is_some()
            && ui
                .small_button("auto")
                .on_hover_text("Back to the detected tempo")
                .clicked()
        {
            edits.push(Edit::SetBpm(None));
        }
        ui.separator();
        ui.label("Downbeat");
        if ui
            .small_button("< beat")
            .on_hover_text("One beat earlier  ,")
            .clicked()
        {
            edits.push(Edit::ShiftDownbeat(-1));
        }
        if ui
            .small_button("beat >")
            .on_hover_text("One beat later  .")
            .clicked()
        {
            edits.push(Edit::ShiftDownbeat(1));
        }
        ui.separator();
        ui.label(format!("Offset {:+.0} ms", map.grid_offset * 1000.0));
        if ui
            .small_button("−10")
            .on_hover_text("Grid 10 ms earlier  Shift+,")
            .clicked()
        {
            edits.push(Edit::Nudge(-0.01));
        }
        if ui
            .small_button("+10")
            .on_hover_text("Grid 10 ms later  Shift+.")
            .clicked()
        {
            edits.push(Edit::Nudge(0.01));
        }
        ui.separator();
        ui.checkbox(&mut state.show_grid, "Grid")
            .on_hover_text("Show the beat grid  0");
        if !map.grid_is_detected()
            && ui
                .small_button("Reset")
                .on_hover_text("Back to the detected grid")
                .clicked()
        {
            edits.push(Edit::ResetGrid);
        }
    });
    let _ = ctx;
}

/// The keyboard: grid fixes, drops, in/out, zoom.
fn keys(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &egui::Ui,
    map: &MusicMap,
    edits: &mut Vec<Edit>,
) {
    let pressed: Vec<(egui::Key, egui::Modifiers)> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| {
                if let egui::Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    modifiers,
                    ..
                } = e
                {
                    Some((physical_key.unwrap_or(*key), *modifiers))
                } else {
                    None
                }
            })
            .collect()
    });
    let playhead_bar = view::playhead(ctx, map).and_then(|t| view::bar_near(map, t));
    let target = state
        .hover_bar
        .or(playhead_bar)
        .or(state.sel.map(|(a, _)| a));
    for (key, m) in pressed {
        if m.command || m.alt {
            continue;
        }
        // `egui::Key` has a hundred variants; compare rather than match.
        let centre = state.view_t0 + 0.5 * state.view_span;
        if key == egui::Key::Num0 {
            state.show_grid = !state.show_grid;
        } else if key == egui::Key::Comma {
            edits.push(if m.shift {
                Edit::Nudge(-0.01)
            } else {
                Edit::ShiftDownbeat(-1)
            });
        } else if key == egui::Key::Period {
            edits.push(if m.shift {
                Edit::Nudge(0.01)
            } else {
                Edit::ShiftDownbeat(1)
            });
        } else if key == egui::Key::D {
            if let Some(bar) = target {
                edits.push(Edit::ToggleDrop(bar));
            }
        } else if key == egui::Key::I {
            if let Some(bar) = playhead_bar.or(state.hover_bar) {
                let to = state.sel.map_or(bar, |(_, b)| b.max(bar));
                view::select(ctx, state, bar, to);
                state.anchor = Some(bar);
            }
        } else if key == egui::Key::O {
            if let Some(bar) = playhead_bar.or(state.hover_bar) {
                let from = state.sel.map_or(bar, |(a, _)| a.min(bar));
                view::select(ctx, state, from, bar);
            }
        } else if key == egui::Key::Equals || key == egui::Key::Plus {
            state.zoom(1.0 / 1.5, centre, map.duration);
        } else if key == egui::Key::Minus {
            state.zoom(1.5, centre, map.duration);
        } else if key == egui::Key::Escape {
            state.sel = None;
            ctx.selection.clear();
        }
    }
}

/// The shortcut line under the strips.
fn hint(ctx: &PageCtx<'_>, ui: &mut egui::Ui) {
    ui.label(
        RichText::new(
            "drag bars to select · drag a bar number to move the downbeat · , . beat · \
             Shift+, . 10 ms · D drop · I/O range · 0 grid · Ctrl+wheel zoom · R reference",
        )
        .small()
        .color(theme::rgb(ctx.theme.text_faint)),
    );
}

/// The selection: what it is, and naming it as a song range.
fn selection_row(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    map: &MusicMap,
    edits: &mut Vec<Edit>,
) {
    widgets::title(ui, ctx.theme, "Selection");
    let Some((from, to)) = state.sel else {
        widgets::muted(ui, ctx.theme, "Click or drag over bars to select them.");
        return;
    };
    let t0 = map.bars.get(from).map_or(0.0, |b| b.t);
    let t1 = map.bar_end(to);
    ui.label(format!(
        "bars {from}–{to} · {} bars · {:.1} s ({} – {})",
        to + 1 - from,
        t1 - t0,
        clock(t0),
        clock(t1)
    ));
    ui.horizontal_wrapped(|ui| {
        ui.label("Use for");
        for name in view::DELIVERABLES {
            let target = target_seconds(name);
            let hover = match target {
                Some(s) => format!("Save as the {name} song range (target {s} s)"),
                None => format!("Save as the {name} song range"),
            };
            if ui.small_button(name).on_hover_text(hover).clicked() {
                edits.push(Edit::SetRange(name.to_owned(), from, to));
            }
        }
        let field = ui.add(
            egui::TextEdit::singleline(&mut state.range_name)
                .hint_text("other name")
                .desired_width(90.0),
        );
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.small_button("Save").clicked() || enter) && !state.range_name.trim().is_empty() {
            edits.push(Edit::SetRange(state.range_name.trim().to_owned(), from, to));
            state.range_name.clear();
        }
    });
}

/// The section list, with renaming.
fn sections(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    map: &MusicMap,
    edits: &mut Vec<Edit>,
) {
    ui.horizontal(|ui| {
        widgets::title(ui, ctx.theme, "Sections");
        if !map.sections_user.is_empty()
            && ui
                .small_button("Detected")
                .on_hover_text("Back to the detected sections")
                .clicked()
        {
            edits.push(Edit::ResetSections);
        }
    });
    egui::Grid::new("music-sections")
        .num_columns(3)
        .spacing([theme::SPACE_SM, 2.0])
        .show(ui, |ui| {
            for (i, s) in map.sections.iter().enumerate() {
                let colour = section_color(ctx.theme, &s.name);
                widgets::swatch(ui, ctx.theme, colour, egui::vec2(10.0, 10.0));
                let renaming = state.rename.as_ref().is_some_and(|(k, _)| *k == i);
                if renaming {
                    let mut commit = None;
                    let mut cancel = false;
                    if let Some((_, text)) = state.rename.as_mut() {
                        let field = ui.add(egui::TextEdit::singleline(text).desired_width(110.0));
                        if state.focus_rename {
                            field.request_focus();
                            state.focus_rename = false;
                        }
                        if field.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                cancel = true;
                            } else {
                                commit = Some(text.clone());
                            }
                        }
                    }
                    if let Some(name) = commit {
                        edits.push(Edit::RenameSection(i, name));
                        state.rename = None;
                    } else if cancel {
                        state.rename = None;
                    }
                } else {
                    let label = ui
                        .add(
                            egui::Label::new(
                                RichText::new(&s.name).color(theme::rgb(colour)).strong(),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Click to select; double-click or right-click to rename");
                    if label.clicked() {
                        view::select(ctx, state, s.from_bar, s.to_bar);
                        if let Some(bar) = map.bars.get(s.from_bar) {
                            state.view_t0 = bar.t - 0.05 * state.view_span;
                            state.clamp_view(map.duration);
                        }
                    }
                    if label.double_clicked() {
                        state.rename = Some((i, s.name.clone()));
                        state.focus_rename = true;
                    }
                    label.context_menu(|ui| {
                        if ui.button("Rename…").clicked() {
                            state.rename = Some((i, s.name.clone()));
                            state.focus_rename = true;
                            ui.close();
                        }
                        ui.menu_button("Call it", |ui| {
                            for name in structure::NAMES {
                                if ui.button(name).clicked() {
                                    edits.push(Edit::RenameSection(i, name.to_owned()));
                                    ui.close();
                                }
                            }
                        });
                    });
                }
                let t0 = map.bars.get(s.from_bar).map_or(0.0, |b| b.t);
                ui.label(
                    RichText::new(format!(
                        "bars {}–{} · {} · {:.1} s",
                        s.from_bar,
                        s.to_bar,
                        clock(t0),
                        map.bar_end(s.to_bar) - t0
                    ))
                    .color(theme::rgb(ctx.theme.text_muted)),
                );
                ui.end_row();
            }
        });
}

/// The drop list.
fn drops(ctx: &mut PageCtx<'_>, ui: &mut egui::Ui, map: &MusicMap, edits: &mut Vec<Edit>) {
    widgets::title(ui, ctx.theme, "Drops");
    if map.drops.is_empty() {
        widgets::muted(ui, ctx.theme, "None found. Press D over a bar to mark one.");
        return;
    }
    egui::Grid::new("music-drops")
        .num_columns(3)
        .spacing([theme::SPACE_SM, 2.0])
        .show(ui, |ui| {
            for (i, d) in map.drops.iter().enumerate() {
                let what = match (i, d.user) {
                    (0, true) => "main · yours",
                    (0, false) => "main",
                    (_, true) => "yours",
                    (_, false) => "detected",
                };
                ui.label(
                    RichText::new(format!("bar {}", d.bar))
                        .color(theme::rgb(ctx.theme.accent))
                        .strong(),
                );
                ui.label(
                    RichText::new(format!("{} · score {:.2} · {what}", clock(d.t), d.score))
                        .color(theme::rgb(ctx.theme.text_muted)),
                );
                ui.horizontal(|ui| {
                    if ui
                        .small_button("play")
                        .on_hover_text("Play from the bar before")
                        .clicked()
                    {
                        let t = map.bars.get(d.bar.saturating_sub(2)).map_or(d.t, |b| b.t);
                        view::seek(ctx, map, t);
                        ctx.player.play();
                    }
                    if i > 0
                        && ui
                            .small_button("main")
                            .on_hover_text("Build edits around this drop")
                            .clicked()
                    {
                        edits.push(Edit::PrimaryDrop(d.bar));
                    }
                    if ui
                        .small_button("remove")
                        .on_hover_text("Not a drop")
                        .clicked()
                    {
                        edits.push(Edit::ToggleDrop(d.bar));
                    }
                });
                ui.end_row();
            }
        });
}

/// The song ranges.
fn ranges(ctx: &mut PageCtx<'_>, ui: &mut egui::Ui, map: &MusicMap, edits: &mut Vec<Edit>) {
    widgets::title(ui, ctx.theme, "Song ranges");
    if map.ranges.is_empty() {
        widgets::muted(
            ui,
            ctx.theme,
            "Select bars, then pick a deliverable to give it its part of the song.",
        );
        return;
    }
    let n = map.ranges.len();
    egui::Grid::new("music-ranges")
        .num_columns(3)
        .spacing([theme::SPACE_SM, 2.0])
        .show(ui, |ui| {
            for (i, r) in map.ranges.iter().enumerate() {
                let colour = ctx.theme.series(i, n.max(4));
                ui.label(RichText::new(&r.name).color(theme::rgb(colour)).strong());
                let t0 = map.bars.get(r.from_bar).map_or(0.0, |b| b.t);
                let secs = map.bar_end(r.to_bar) - t0;
                let fit = target_seconds(&r.name)
                    .map(|s| format!(" (target {s} s, {:+.1})", secs - s as f32))
                    .unwrap_or_default();
                ui.label(
                    RichText::new(format!(
                        "bars {}–{} · {:.1} s{fit}",
                        r.from_bar, r.to_bar, secs
                    ))
                    .color(theme::rgb(ctx.theme.text_muted)),
                );
                ui.horizontal(|ui| {
                    if ui
                        .small_button("play")
                        .on_hover_text("Play the range")
                        .clicked()
                    {
                        view::seek(ctx, map, t0);
                        ctx.player.play();
                    }
                    if ui
                        .small_button("remove")
                        .on_hover_text("Remove the range")
                        .clicked()
                    {
                        edits.push(Edit::RemoveRange(r.name.clone()));
                    }
                });
                ui.end_row();
            }
        });
}

/// The length a deliverable name asks for: `reel-30` → 30 s.
fn target_seconds(name: &str) -> Option<u32> {
    let digits: String = name
        .rsplit(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or_default()
        .to_owned();
    digits.parse().ok().filter(|s| *s > 0)
}

/// `m:ss.s`.
fn clock(t: f32) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor();
    format!("{}:{:04.1}", m as u32, t - m * 60.0)
}

#[cfg(test)]
mod tests {
    use super::{State, clock, target_seconds};

    #[test]
    fn deliverable_names_carry_their_length() -> anyhow::Result<()> {
        assert_eq!(target_seconds("reel-30"), Some(30), "reel-30");
        assert_eq!(target_seconds("wide-60"), Some(60), "wide-60");
        assert_eq!(target_seconds("recap"), None, "no number");
        assert_eq!(clock(75.25), "1:15.2", "minutes and seconds");
        Ok(())
    }

    #[test]
    fn the_zoomed_view_stays_in_the_track() -> anyhow::Result<()> {
        let mut s = State {
            view_span: 10.0,
            ..State::default()
        };
        s.view_t0 = 500.0;
        s.clamp_view(100.0);
        assert!(
            s.view_t0 + s.view_span <= 100.5,
            "clamped to the end: {}",
            s.view_t0
        );
        s.zoom(0.5, s.view_t0 + 5.0, 100.0);
        assert!((s.view_span - 5.0).abs() < 1e-4, "zoomed in");
        s.zoom(100.0, 50.0, 100.0);
        assert!(
            s.view_span <= 105.0 && s.view_t0 <= 0.0,
            "zoomed out to the track"
        );
        Ok(())
    }
}

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

use crate::gui::ui::hint::Hint as _;
use egui::RichText;

use super::PageCtx;
use crate::beats::{MusicMap, structure};
use crate::color::Rgb;
use crate::commands::Cmd;
use crate::gui::ui::{icons, keymap, theme, widgets};
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
    if project.no_music() {
        let theme = ctx.theme;
        let mut choose = false;
        widgets::empty_state(
            ui,
            theme,
            icons::MUSIC,
            "No music",
            "Cuts are paced every two seconds and keep the clips' own sound. Choose a track to cut to its beat instead.",
            |ui| {
                choose = widgets::outline_button(ui, theme, "Choose music\u{2026}").clicked();
            },
        );
        if choose {
            ctx.actions.push(crate::gui::app::Action::RunStep(
                crate::gui::app::Stage::Music,
            ));
        }
        return;
    }
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
    let theme = ctx.theme;
    match track {
        Some(track) => {
            let running = job_running(ctx, state);
            let mut map = false;
            widgets::empty_state(
                ui,
                theme,
                icons::MUSIC,
                "The music is not mapped yet",
                "Map the track's beats, bars and drops to cut to them.",
                |ui| {
                    ui.add_enabled_ui(!running, |ui| {
                        let label = if running {
                            "Mapping…"
                        } else {
                            "Map the music"
                        };
                        map = widgets::button_with(
                            ui,
                            theme,
                            widgets::ButtonKind::Key,
                            Some(icons::AUDIO_WAVEFORM),
                            label,
                        )
                        .clicked();
                    });
                },
            );
            if map {
                state.job = Some(ctx.jobs.spawn(
                    "Map the music",
                    Cmd::Beats {
                        track: Some(track),
                        none: false,
                        work: work.to_path_buf(),
                    },
                ));
            }
        }
        None => {
            let mut choose = false;
            widgets::empty_state(
                ui,
                theme,
                icons::MUSIC,
                "No track chosen",
                "Pick the track every cut is timed to; it is mapped right after.",
                |ui| {
                    choose = widgets::button_with(
                        ui,
                        theme,
                        widgets::ButtonKind::Outline,
                        Some(icons::MUSIC),
                        "Choose music\u{2026}",
                    )
                    .clicked();
                },
            );
            if choose {
                ctx.actions.push(crate::gui::app::Action::RunStep(
                    crate::gui::app::Stage::Music,
                ));
            }
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
    toolbar(ctx, state, ui, map, work, edits);
    header(ctx, state, ui, map);
    ui.add_space(theme::SPACE_SM);

    // The cache is borrowed for the frame; drawing needs `state` mutably too.
    let mut taken = std::mem::take(&mut state.peaks);
    let ready = taken.get(&map.path, ui.ctx());
    view::overview(ctx, state, ui, map, ready);
    ui.add_space(theme::SPACE_XS);
    view::zoomed(ctx, state, ui, map, ready, edits);
    if let Some(e) = taken.error() {
        ui.label(
            RichText::new(format!("No waveform: {e}"))
                .size(theme::SMALL_SIZE)
                .color(theme::rgb(ctx.theme.text_muted)),
        );
    }
    state.peaks = taken;

    let pointer_here = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| page.contains(p));
    if pointer_here && ctx.keys_free(ui) {
        keys(ctx, state, ui, map, edits);
    }
    ui.add_space(theme::SPACE_SM);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            selection_row(ctx, state, ui, map, edits);
            sections(ctx, state, ui, map, edits);
            drops(ctx, ui, map, edits);
            ranges(ctx, ui, map, edits);
        });
}

/// Height of the toolbar under the tab strip: with the 4 px the workspace
/// leaves under the strip, Selects' 40 px toolbar.
const TOOLBAR_H: f32 = theme::PANEL_HEADER_HEIGHT - theme::SPACE_XS;

/// The track's name and numbers, with how the grid was fixed in a tooltip.
fn header(ctx: &mut PageCtx<'_>, state: &State, ui: &mut egui::Ui, map: &MusicMap) {
    let name = std::path::Path::new(&map.path)
        .file_name()
        .map_or_else(|| map.path.clone(), |n| n.to_string_lossy().into_owned());
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
    let grid = if fixes.is_empty() {
        "Grid as detected".to_owned()
    } else {
        format!("Grid fixed: {}", fixes.join(", "))
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
        ui.add(
            egui::Label::new(
                RichText::new(name)
                    .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(ctx.theme.text)),
            )
            .truncate(),
        )
        .hint(&map.path);
        ui.add(
            egui::Label::new(
                RichText::new(format!("{} bars · {}", map.bars.len(), clock(map.duration)))
                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(ctx.theme.text_muted)),
            )
            .truncate(),
        )
        .hint(grid);
    });
    if let Some(e) = &state.error {
        ui.label(
            RichText::new(e)
                .size(theme::SMALL_SIZE)
                .color(theme::rgb(ctx.theme.danger)),
        );
    }
}

/// The toolbar: the tempo menu (with the grid nudges) and the downbeat on the left; the
/// grid toggle, reset, re-analysis and the keys on the right.
fn toolbar(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    map: &MusicMap,
    work: &std::path::Path,
    edits: &mut Vec<Edit>,
) {
    let palette = ctx.theme;
    let preset = ctx.keymap;
    let key = |id: &str| {
        keymap::command(id)
            .map(|c| c.hint(preset))
            .unwrap_or_default()
    };
    let running = job_running(ctx, state);
    let mut analyse = false;
    let gestures = format!(
        "Drag bars to select them; drag a bar number to move the downbeat.\n\
         {} / {}  downbeat a beat earlier / later\n\
         {} / {}  grid 10 ms earlier / later\n\
         {}  mark a drop · {} / {}  range in / out · {}  grid\n\
         Ctrl+wheel  zoom · R  reference in chat",
        key("music-downbeat-earlier"),
        key("music-downbeat-later"),
        key("music-nudge-earlier"),
        key("music-nudge-later"),
        key("music-drop"),
        key("music-from"),
        key("music-to"),
        key("music-grid"),
    );
    widgets::toolbar_row(
        ui,
        TOOLBAR_H,
        &mut (&mut *state, &mut *edits),
        |ui, (state, edits)| {
            tempo_menu(ui, palette, state, map, edits, &key);
            for (glyph, label, id, edit) in [
                (
                    icons::CHEVRON_LEFT,
                    "Downbeat one beat earlier",
                    "music-downbeat-earlier",
                    Edit::ShiftDownbeat(-1),
                ),
                (
                    icons::CHEVRON_RIGHT,
                    "Downbeat one beat later",
                    "music-downbeat-later",
                    Edit::ShiftDownbeat(1),
                ),
            ] {
                if widgets::icon_btn(ui, palette, glyph, label, &key(id)).clicked() {
                    edits.push(edit);
                }
            }
        },
        |ui, (state, edits)| {
            widgets::icon_btn(ui, palette, icons::KEYBOARD, &gestures, "");
            let label = if running {
                "Analysing…"
            } else {
                "Re-analyse the beats (your fixes are kept)"
            };
            let glyph = if running {
                icons::LOADER
            } else {
                icons::REFRESH
            };
            analyse = ui
                .add_enabled_ui(!running, |ui| {
                    widgets::icon_btn(ui, palette, glyph, label, "")
                })
                .inner
                .clicked();
            if !map.grid_is_detected()
                && widgets::icon_btn(
                    ui,
                    palette,
                    icons::ROTATE_CCW,
                    "Back to the detected grid",
                    "",
                )
                .clicked()
            {
                edits.push(Edit::ResetGrid);
            }
            if widgets::icon_toggle(
                ui,
                palette,
                icons::GRID,
                "Show the beat grid",
                &key("music-grid"),
                state.show_grid,
            )
            .clicked()
            {
                state.show_grid = !state.show_grid;
            }
        },
    );
    // No music has no file to re-analyse.
    if analyse && !map.no_music() {
        state.job = Some(ctx.jobs.spawn(
            "Map the music",
            Cmd::Beats {
                track: Some(std::path::PathBuf::from(&map.path)),
                none: false,
                work: work.to_path_buf(),
            },
        ));
    }
}

/// The tempo as a ghost dropdown; its popover types it, doubles or halves
/// it, nudges the grid 10 ms either way,
/// or goes back to the detected tempo.
fn tempo_menu(
    ui: &mut egui::Ui,
    palette: &Palette,
    state: &mut State,
    map: &MusicMap,
    edits: &mut Vec<Edit>,
    key: &dyn Fn(&str) -> String,
) {
    let id = ui.make_persistent_id("music-tempo");
    let open = widgets::menu_is_open(ui.ctx(), id);
    let response = widgets::dropdown_button(
        ui,
        palette,
        &format!("{:.2} BPM", map.bpm),
        "Tempo and grid",
        open,
    );
    // The response is not needed: drawing it is the point.
    let _ = widgets::menu(&response)
        .id(id)
        .width(220.0)
        .gap(theme::SPACE_XS)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = theme::SPACE_SM;
            ui.horizontal(|ui| {
                ui.add_space(theme::SPACE_SM);
                widgets::menu_heading(ui, palette, "Tempo");
            });
            ui.horizontal(|ui| {
                ui.add_space(theme::SPACE_SM);
                let mut bpm = state.bpm_edit.unwrap_or(map.bpm);
                let field = ui.add(
                    widgets::number(&mut bpm, "Tempo")
                        .step(0.01)
                        .drag_points_per_step(1.0)
                        .range(40.0..=300.0)
                        .decimals(2)
                        .suffix(" BPM")
                        .width(130.0),
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
            });
            widgets::menu_separator(ui, palette);
            if widgets::menu_item(ui, palette, None, "Double the tempo", "", false).clicked() {
                edits.push(Edit::SetBpm(Some(map.bpm * 2.0)));
            }
            if widgets::menu_item(ui, palette, None, "Halve the tempo", "", false).clicked() {
                edits.push(Edit::SetBpm(Some(map.bpm / 2.0)));
            }
            widgets::menu_separator(ui, palette);
            for (label, id, edit) in [
                (
                    "Grid 10 ms earlier",
                    "music-nudge-earlier",
                    Edit::Nudge(-0.01),
                ),
                ("Grid 10 ms later", "music-nudge-later", Edit::Nudge(0.01)),
            ] {
                if widgets::menu_item(ui, palette, None, label, &key(id), false).clicked() {
                    edits.push(edit);
                }
            }
            if map.bpm_override.is_some()
                && widgets::menu_item(
                    ui,
                    palette,
                    Some(icons::ROTATE_CCW),
                    &format!("Detected tempo ({:.2})", map.detected_bpm()),
                    "",
                    false,
                )
                .clicked()
            {
                edits.push(Edit::SetBpm(None));
            }
        });
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
    let preset = ctx.keymap;
    for (key, m) in pressed {
        // The keys are the Music rows of `keymap::COMMANDS` under the preset.
        let fires = |id| keymap::fires(id, preset, key, m);
        // `+` is Shift+`=` on most layouts and is read by its physical key.
        let zoom_in = fires("music-zoom-in")
            || keymap::fires(
                "music-zoom-in",
                preset,
                key,
                egui::Modifiers { shift: false, ..m },
            )
            || key == egui::Key::Plus;
        let centre = state.view_t0 + 0.5 * state.view_span;
        if fires("music-grid") {
            state.show_grid = !state.show_grid;
        } else if fires("music-nudge-earlier") {
            edits.push(Edit::Nudge(-0.01));
        } else if fires("music-nudge-later") {
            edits.push(Edit::Nudge(0.01));
        } else if fires("music-downbeat-earlier") {
            edits.push(Edit::ShiftDownbeat(-1));
        } else if fires("music-downbeat-later") {
            edits.push(Edit::ShiftDownbeat(1));
        } else if fires("music-drop") {
            if let Some(bar) = target {
                edits.push(Edit::ToggleDrop(bar));
            }
        } else if fires("music-from") {
            if let Some(bar) = playhead_bar.or(state.hover_bar) {
                let to = state.sel.map_or(bar, |(_, b)| b.max(bar));
                view::select(ctx, state, bar, to);
                state.anchor = Some(bar);
            }
        } else if fires("music-to") {
            if let Some(bar) = playhead_bar.or(state.hover_bar) {
                let from = state.sel.map_or(bar, |(a, _)| a.min(bar));
                view::select(ctx, state, from, bar);
            }
        } else if zoom_in {
            state.zoom(1.0 / 1.5, centre, map.duration);
        } else if fires("music-zoom-out") {
            state.zoom(1.5, centre, map.duration);
        } else if fires("music-deselect") {
            state.sel = None;
            ctx.selection.clear();
        }
    }
}

/// A list's heading: 12 px Medium, muted, 28 px, with `right` at the far
/// end (right to left).
fn heading(ui: &mut egui::Ui, palette: &Palette, title: &str, right: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(theme::SPACE_XS);
    ui.horizontal(|ui| {
        ui.set_min_height(theme::CONTROL_HEIGHT);
        ui.label(
            RichText::new(title)
                .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                .color(theme::rgb(palette.text_muted)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            right(ui);
        });
    });
}

/// A quiet line in a list: nothing there yet.
fn none(ui: &mut egui::Ui, palette: &Palette, text: &str, tooltip: &str) {
    let label = ui.label(
        RichText::new(text)
            .size(theme::BODY_SIZE)
            .color(theme::rgb(palette.text_faint)),
    );
    if !tooltip.is_empty() {
        label.hint(tooltip);
    }
}

/// The selection: what it is, and naming it as a song range.
fn selection_row(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    map: &MusicMap,
    edits: &mut Vec<Edit>,
) {
    heading(ui, ctx.theme, "Selection", |_| {});
    let Some((from, to)) = state.sel else {
        none(
            ui,
            ctx.theme,
            "No bars selected",
            "Click or drag over bars to select them",
        );
        return;
    };
    let t0 = map.bars.get(from).map_or(0.0, |b| b.t);
    let t1 = map.bar_end(to);
    ui.label(
        RichText::new(format!(
            "Bars {from}–{to} · {} bars · {:.1} s",
            to + 1 - from,
            t1 - t0,
        ))
        .size(theme::BODY_SIZE)
        .color(theme::rgb(ctx.theme.text)),
    )
    .hint(format!("{} – {}", clock(t0), clock(t1)));
    ui.add_space(theme::SPACE_XS);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(theme::SPACE_XS, theme::SPACE_XS);
        for name in view::DELIVERABLES {
            let target = target_seconds(name);
            let hover = match target {
                Some(s) => format!("Save as the {name} song range (target {s} s)"),
                None => format!("Save as the {name} song range"),
            };
            let button =
                widgets::button_with(ui, ctx.theme, widgets::ButtonKind::Outline, None, name);
            if widgets::tip(button, ctx.theme, &hover, "").clicked() {
                edits.push(Edit::SetRange(name.to_owned(), from, to));
            }
        }
    });
    ui.add_space(theme::SPACE_XS);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
        let width =
            (ui.available_width() - theme::CONTROL_HEIGHT - theme::SPACE_XS).clamp(60.0, 200.0);
        let field = widgets::input(&mut state.range_name)
            .label("Range name")
            .placeholder("Another range name")
            .width(width)
            .show(ui)
            .response;
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let save = widgets::icon_btn(ui, ctx.theme, icons::CHECK, "Save the range", "Enter");
        if (save.clicked() || enter) && !state.range_name.trim().is_empty() {
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
    heading(ui, ctx.theme, "Sections", |ui| {
        if !map.sections_user.is_empty()
            && widgets::icon_btn(
                ui,
                ctx.theme,
                icons::ROTATE_CCW,
                "Back to the detected sections",
                "",
            )
            .clicked()
        {
            edits.push(Edit::ResetSections);
        }
    });
    // A Material 3 list, a row per section: its colour, its name (a field
    // while it is renamed) and its bars. Click selects it; double-click or
    // the menu renames it.
    use egui_material3::{ListItem, MaterialList, VisualDensity};
    let palette = ctx.theme;
    let focus_rename = std::mem::take(&mut state.focus_rename);
    let mut rename_slot = state.rename.as_mut().map(|(k, text)| (*k, text));
    let field: std::cell::RefCell<Option<egui::Response>> = std::cell::RefCell::new(None);
    let list = map.sections.iter().enumerate().fold(
        MaterialList::new()
            .id(ui.make_persistent_id("music-sections"))
            .dividers(false)
            .fill_width(true),
        |list, (i, s)| {
            let editing = match rename_slot.take() {
                Some((k, text)) if k == i => Some(text),
                other => {
                    rename_slot = other;
                    None
                }
            };
            let field = &field;
            let t0 = map.bars.get(s.from_bar).map_or(0.0, |b| b.t);
            let bars = format!(
                "bars {}–{} · {:.1} s",
                s.from_bar,
                s.to_bar,
                map.bar_end(s.to_bar) - t0
            );
            list.item(
                ListItem::new(s.name.clone())
                    .dense(true)
                    .visual_density(VisualDensity::new(0.0, -4.0))
                    .content_ui(move |ui, _item| {
                        ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
                        let colour = section_color(palette, &s.name);
                        widgets::swatch(ui, palette, colour, egui::vec2(4.0, 16.0));
                        match editing {
                            Some(text) => {
                                let response = widgets::input(text)
                                    .label("Section name")
                                    .width(140.0)
                                    .show(ui)
                                    .response;
                                if focus_rename {
                                    response.request_focus();
                                }
                                *field.borrow_mut() = Some(response);
                            }
                            None => {
                                ui.label(
                                    RichText::new(&s.name)
                                        .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                                        .color(theme::rgb(palette.text)),
                                );
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(bars)
                                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                                    .color(theme::rgb(palette.text_muted)),
                            )
                            .hint(clock(t0));
                        });
                    }),
            )
        },
    );
    let rows = list.show(ui).items;
    if let Some(response) = field.into_inner()
        && response.lost_focus()
    {
        let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if let Some((k, text)) = state.rename.take()
            && !escape
        {
            edits.push(Edit::RenameSection(k, text));
        }
    }
    for ((i, s), row) in map.sections.iter().enumerate().zip(rows) {
        let row = row.hint("Click to select; double-click or right-click to rename");
        if row.clicked() {
            view::select(ctx, state, s.from_bar, s.to_bar);
            if let Some(bar) = map.bars.get(s.from_bar) {
                state.view_t0 = bar.t - 0.05 * state.view_span;
                state.clamp_view(map.duration);
            }
        }
        if row.double_clicked() {
            state.rename = Some((i, s.name.clone()));
            state.focus_rename = true;
        }
        widgets::context_menu(&row, |ui| {
            if ui.button("Rename…").clicked() {
                state.rename = Some((i, s.name.clone()));
                state.focus_rename = true;
                ui.close();
            }
            widgets::submenu(ui, "Call it", |ui| {
                for name in structure::NAMES {
                    if ui.button(name).clicked() {
                        edits.push(Edit::RenameSection(i, name.to_owned()));
                        ui.close();
                    }
                }
            });
        });
    }
}

/// The drop list.
fn drops(ctx: &mut PageCtx<'_>, ui: &mut egui::Ui, map: &MusicMap, edits: &mut Vec<Edit>) {
    heading(ui, ctx.theme, "Drops", |_| {});
    if map.drops.is_empty() {
        let key = keymap::command("music-drop")
            .map(|c| c.hint(ctx.keymap))
            .unwrap_or_default();
        none(
            ui,
            ctx.theme,
            "None found",
            &format!("Press {key} over a bar to mark one"),
        );
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
                    RichText::new(format!("Bar {}", d.bar))
                        .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                        .color(theme::rgb(ctx.theme.text)),
                );
                ui.label(
                    RichText::new(format!("{} · {what}", clock(d.t)))
                        .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                        .color(theme::rgb(ctx.theme.text_muted)),
                )
                .hint(format!("score {:.2}", d.score));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if widgets::icon_btn(ui, ctx.theme, icons::PLAY, "Play from the bar before", "")
                        .clicked()
                    {
                        let t = map.bars.get(d.bar.saturating_sub(2)).map_or(d.t, |b| b.t);
                        view::seek(ctx, map, t);
                        ctx.player.play();
                    }
                    if i > 0
                        && widgets::icon_btn(
                            ui,
                            ctx.theme,
                            icons::STAR,
                            "Make it the main drop: edits build around it",
                            "",
                        )
                        .clicked()
                    {
                        edits.push(Edit::PrimaryDrop(d.bar));
                    }
                    if widgets::icon_btn(ui, ctx.theme, icons::X, "Not a drop", "").clicked() {
                        edits.push(Edit::ToggleDrop(d.bar));
                    }
                });
                ui.end_row();
            }
        });
}

/// The song ranges.
fn ranges(ctx: &mut PageCtx<'_>, ui: &mut egui::Ui, map: &MusicMap, edits: &mut Vec<Edit>) {
    heading(ui, ctx.theme, "Song ranges", |_| {});
    if map.ranges.is_empty() {
        none(
            ui,
            ctx.theme,
            "None yet",
            "Select bars, then pick a deliverable to give it its part of the song",
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
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
                    widgets::swatch(ui, ctx.theme, colour, egui::vec2(4.0, 16.0));
                    ui.label(
                        RichText::new(&r.name)
                            .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                            .color(theme::rgb(ctx.theme.text)),
                    );
                });
                let t0 = map.bars.get(r.from_bar).map_or(0.0, |b| b.t);
                let secs = map.bar_end(r.to_bar) - t0;
                let fit = target_seconds(&r.name)
                    .map(|s| format!(" ({:+.1})", secs - s as f32))
                    .unwrap_or_default();
                ui.label(
                    RichText::new(format!(
                        "bars {}–{} · {:.1} s{fit}",
                        r.from_bar, r.to_bar, secs
                    ))
                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(ctx.theme.text_muted)),
                );
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if widgets::icon_btn(ui, ctx.theme, icons::PLAY, "Play the range", "").clicked()
                    {
                        view::seek(ctx, map, t0);
                        ctx.player.play();
                    }
                    if widgets::icon_btn(ui, ctx.theme, icons::TRASH, "Remove the range", "")
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

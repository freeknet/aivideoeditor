// ============================================================================
// Module:       gui::pages::plan
// Description:  The timeline at the foot of the workspace: plan shots on the
//               beat grid over the music, versions, swap alternatives, checks.
//
// Dependencies: egui; crate::{plan, alternatives, beats, refs, commands},
//               crate::gui::{pages, player, thumbs}
// ============================================================================

//! The timeline.
//!
//! One plan at a time, drawn the way editing apps draw a sequence: a ruler
//! in seconds and bars, V1 with the shots where `Plan::timing` puts them,
//! A1 with the song's waveform, the drop as a flag, a playhead tied to the
//! shared viewer. **The music is the clock**: every edit keeps the footage
//! filling `start_bar..card_bar` (see [`edit`]), so a reorder swaps content
//! between slots, dragging an edit point rolls beats between neighbours,
//! and Alt-drag slips the in-point.
//!
//! Every change goes on an undo stack, re-runs `plan::check` (the check bar
//! and the red edges), and is saved a moment later as a new version
//! ([`history`]): `plans/<name>.toml` for the engine, a numbered copy for
//! the version menu. `A` on a shot ranks replacements that fit its slot
//! ([`crate::alternatives`]); `1`–`8` swaps one in.
//!
//! Plans sit in tabs above the tracks ([`tabs`]), one per plan file, so a
//! plan the assistant writes shows up as a tab to compare against. Review
//! comments (`F`) are pinned to moments of the cut and handed to the chat
//! as one batch ([`comments`]). While the assistant commits an edit it
//! holds `plans/.lock-<name>` and the timeline goes read-only. A
//! storyboard lane ([`storyboard`]) shows the plan as cards with their
//! notes, grouped by song section. "Compare plans" in the tab strip
//! ([`compare`]) stacks the plan over another plan or a saved version on
//! one beat grid and lists what differs, shot by shot.
//!
//! Playback ends the way the render does: after the last shot the viewer
//! holds the end card for the span [`Plan::card_span`] gives it, over the
//! music, and V1 draws the card as a block in that span.
//!
//! Layout and the rest of the region contract: `docs/GUI_CONTRACTS.md`;
//! the spec this follows: `docs/research/02-ui-and-workflow.md` §3, §9.7.

mod comments;
mod compare;
mod edit;
mod history;
mod storyboard;
mod strips;
mod tabs;
mod timeline;
mod waveform;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::PageCtx;
use crate::alternatives::Alternative;
use crate::beats::MusicMap;
use crate::gui::app::Action;
use crate::gui::player::{EndCard, Player, SeqShot, Source};
use crate::gui::thumbs::Thumbs;
use crate::gui::ui::{icons, theme, widgets};
use crate::plan::{Aspect, Plan, Problem};
use crate::project::Project;
use crate::refs::{Reference, Target};
use crate::theme::Palette;

/// Seconds after the last change before it is saved as a version.
const AUTOSAVE_AFTER: f32 = 1.2;
/// How often the live file is checked for changes made elsewhere.
const DISK_CHECK: Duration = Duration::from_secs(1);
/// Key of this page's table in the night's settings.
const PAGE_KEY: &str = "plan";
/// Alternatives shown in the strip.
const ALTERNATIVES: usize = 8;
/// A lock file older than this is a crashed writer's and is ignored.
const LOCK_STALE: Duration = Duration::from_secs(300);
/// Width of the comments list beside the tracks.
const COMMENTS_W: f32 = 300.0;
/// Height of the check bar along the panel's foot.
const CHECK_H: f32 = 36.0;
/// Height of the selected shot's row.
const SELECTION_H: f32 = 40.0;
/// Height of the alternatives strip.
const ALTS_H: f32 = 104.0;

/// The pointer tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Tool {
    /// Click selects, drag reorders, edges roll.
    #[default]
    Select,
    /// Drag slips the in-point.
    Slip,
    /// Click zooms in, Alt-click out.
    Zoom,
    /// Drag scrolls.
    Hand,
}

/// A drag in progress.
#[derive(Debug)]
enum Drag {
    /// Rolling the edit point before shot `edit`.
    Roll {
        /// Index of the shot right of the edit.
        edit: usize,
        /// The plan when the drag began.
        before: Plan,
    },
    /// Moving shot `from` to slot `to`.
    Move {
        /// Where it was.
        from: usize,
        /// Where it would land.
        to: usize,
    },
    /// Slipping shot `shot`'s in-point.
    Slip {
        /// The shot.
        shot: usize,
        /// Its in-point when the drag began.
        t0: f32,
        /// The plan when the drag began.
        before: Plan,
    },
    /// Moving the crop centre in the framing strip.
    Frame {
        /// The shot.
        shot: usize,
        /// The plan when the drag began.
        before: Plan,
    },
}

/// The alternatives strip.
#[derive(Debug)]
struct Alts {
    /// Slot (0-based).
    slot: usize,
    /// Plan revision they were ranked for.
    rev: u64,
    /// Best first.
    items: Vec<Alternative>,
}

/// The "Draft new…" form.
#[derive(Debug)]
struct DraftForm {
    /// Plan name.
    name: String,
    /// Frame shape.
    aspect: Aspect,
    /// Target length.
    seconds: f32,
    /// End card image.
    end_card: String,
    /// Bars the card holds.
    card_bars: usize,
    /// Why the last attempt was refused.
    error: Option<String>,
}

/// A past version shown read-only.
#[derive(Debug)]
struct Viewing {
    /// Which.
    version: history::Version,
    /// Its plan.
    plan: Plan,
}

/// Zoom and scroll.
#[derive(Debug)]
struct View {
    /// Pixels per second.
    pps: f32,
    /// Seconds at the left edge.
    scroll: f32,
    /// Fit the whole plan on the next frame.
    fit: bool,
    /// Keep the playhead in view while playing.
    follow: bool,
    /// A span (seconds) to scroll into view on the next frame.
    reveal: Option<(f32, f32)>,
    /// Where the playhead was when the viewer last played this plan: drawn
    /// as a quiet mark while the viewer shows something else.
    parked: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            pps: 40.0,
            scroll: 0.0,
            fit: true,
            follow: true,
            reveal: None,
            parked: 0.0,
        }
    }
}

/// The timeline's own state, kept across frames.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The night the state belongs to.
    work: Option<PathBuf>,
    /// The open plan's name (file stem).
    name: Option<String>,
    /// The open plan as edited.
    plan: Option<Plan>,
    /// Why the plan could not be read.
    load_error: Option<String>,
    /// The live file's mtime after our last load or save.
    disk_stamp: Option<SystemTime>,
    /// When the live file was last checked.
    last_disk_check: Option<Instant>,
    /// Undo / redo.
    undo: edit::Undo,
    /// An unsaved change: when, and what.
    dirty: Option<(Instant, String)>,
    /// The save line in the header.
    status: String,
    /// Bumped on every change to the plan.
    rev: u64,
    /// What `problems` was computed for: revision and viewed version.
    checked: Option<(u64, Option<u32>)>,
    /// `plan::check` on the shown plan.
    problems: Vec<Problem>,
    /// Zoom and scroll.
    view: View,
    /// The pointer tool.
    tool: Tool,
    /// A drag in progress.
    drag: Option<Drag>,
    /// The anchor of Shift-click ranges.
    anchor: Option<usize>,
    /// The shot a context menu is open on.
    menu_shot: Option<usize>,
    /// The alternatives strip, when open.
    alts: Option<Alts>,
    /// A past version on show.
    viewing: Option<Viewing>,
    /// The open plan's versions, oldest first.
    versions: Vec<history::Version>,
    /// The "Draft new…" form, when open.
    draft: Option<DraftForm>,
    /// A draft job and the plan it writes.
    pending_job: Option<(u64, String)>,
    /// Music peaks.
    waves: waveform::Waves,
    /// V1 drawn without thumbnails.
    hide_thumbs: bool,
    /// A1's waveform dimmed.
    mute_music: bool,
    /// V1 dimmed to read the music.
    solo_music: bool,
    /// The note being typed for a shot.
    note_edit: Option<(usize, String)>,
    /// The storyboard lane above V1.
    storyboard: bool,
    /// The comments list beside the tracks (Comment mode).
    show_comments: bool,
    /// Resolved comments shown in the list and the ruler.
    show_resolved: bool,
    /// The comment last clicked.
    comment_sel: Option<usize>,
    /// The comment whose note is being typed, and the text so far.
    comment_edit: Option<(usize, String)>,
    /// A comment whose text box takes focus on the next frame.
    comment_focus: Option<usize>,
    /// What is known about each plan file, for the tabs.
    tab_info: BTreeMap<String, tabs::Info>,
    /// Whether the plan files have been scanned once.
    tabs_scanned: bool,
    /// When the plan list and the lock were last checked.
    last_tabs_check: Option<Instant>,
    /// The rename / duplicate form, when open.
    tab_form: Option<tabs::Form>,
    /// The assistant holds the open plan's lock: read-only.
    locked: bool,
    /// An A/B compare on show, in place of the tracks.
    compare: Option<compare::Compare>,
}

/// Everything about the shown plan a frame needs, computed once.
struct Shown<'a> {
    /// The palette.
    theme: &'a Palette,
    /// The night.
    project: &'a Project,
    /// The music map.
    map: &'a MusicMap,
    /// The plan on show (a version, or the one being edited).
    plan: Plan,
    /// Its name.
    name: String,
    /// Start and length of each shot, seconds from the plan start.
    timing: Vec<(f32, f32)>,
    /// Every beat's time from the start bar to the end bar.
    grid: Vec<f32>,
    /// Beat at which each shot starts, plus the end.
    starts: Vec<usize>,
    /// Seconds to the end card.
    card: f32,
    /// Seconds to the end bar.
    total: f32,
    /// Selected shots, 0-based.
    sel: BTreeSet<usize>,
    /// Problems per shot (0-based); whole-plan ones are not here.
    issues: BTreeMap<usize, Vec<String>>,
    /// Whether edits are allowed (not viewing an old version).
    editable: bool,
    /// The plan as the viewer plays it.
    cut: Cut,
    /// The plan revision this frame started from.
    rev: u64,
    /// Whether the viewer is playing this plan.
    ours: bool,
    /// The keyboard preset in force.
    preset: crate::gui::ui::keymap::Preset,
}

impl Shown<'_> {
    /// The source file of a clip.
    fn clip_path(&self, clip: &str) -> Option<PathBuf> {
        self.project
            .index
            .as_ref()
            .and_then(|i| i.clip(clip))
            .map(|c| PathBuf::from(&c.path))
    }

    /// A clip's length in seconds.
    fn clip_len(&self, clip: &str) -> Option<f32> {
        self.project
            .index
            .as_ref()
            .and_then(|i| i.clip(clip))
            .map(|c| c.probe.duration as f32)
    }

    /// The shot-log subject at a shot's in-point.
    fn subject(&self, clip: &str, t: f32) -> &str {
        self.project
            .index
            .as_ref()
            .and_then(|i| {
                i.segments
                    .iter()
                    .find(|g| g.clip == clip && g.seg.start <= t && t < g.seg.end)
            })
            .map_or("", |g| g.subject.as_str())
    }

    /// The single selected shot.
    fn single(&self) -> Option<usize> {
        (self.sel.len() == 1)
            .then(|| self.sel.first().copied())
            .flatten()
    }
}

/// The shared things a frame changes.
struct Io<'a> {
    /// Thumbnails.
    thumbs: &'a mut Thumbs,
    /// The viewer.
    player: &'a mut Player,
    /// The app-wide selection.
    selection: &'a mut Vec<Reference>,
    /// Actions for after the frame.
    actions: &'a mut Vec<Action>,
}

impl Io<'_> {
    /// Replaces the app-wide selection with shots of `name` (0-based).
    fn select(&mut self, name: &str, shots: &BTreeSet<usize>) {
        *self.selection = shots
            .iter()
            .map(|&k| {
                Reference::new(Target::Shot {
                    plan: name.to_owned(),
                    index: k + 1,
                })
            })
            .collect();
    }

    /// Loads the shown plan into the viewer unless it is already there.
    fn load(&mut self, s: &Shown<'_>) {
        if !s.ours {
            s.cut.open(self.player);
        }
    }

    /// Seeks the viewer to `t` seconds into the plan, loading it first.
    fn seek(&mut self, s: &Shown<'_>, t: f32) {
        self.load(s);
        self.player.seek(f64::from(t.max(0.0)));
    }
}

/// The aspect's file key (`9x16`).
fn aspect_key(a: Aspect) -> &'static str {
    match a {
        Aspect::Vertical => "9x16",
        Aspect::Feed => "4x5",
        Aspect::Wide => "16x9",
    }
}

/// Output width over height.
fn aspect_ratio(a: Aspect) -> f32 {
    let (w, h) = a.size();
    w as f32 / h as f32
}

/// The modification time of a file.
fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The lock the assistant holds on plan `name` while it commits an edit:
/// `plans/.lock-<name>`. Its presence makes the timeline read-only.
pub(crate) fn lock_path(work: &Path, name: &str) -> PathBuf {
    work.join("plans").join(format!(".lock-{name}"))
}

/// Whether the lock at `path` is held: the file exists and is younger than
/// [`LOCK_STALE`] at `now` (an older one is a crashed writer's).
fn lock_active(path: &Path, now: SystemTime) -> bool {
    mtime(path).is_some_and(|m| now.duration_since(m).map_or(true, |age| age < LOCK_STALE))
}

/// Whether a problem is structural (an error) rather than a content rule
/// the plan bends (a warning). `ave check` fails on both.
fn is_error(p: &Problem) -> bool {
    p.shot == 0
        || [
            "not in the index",
            "runs outside",
            "crop centre",
            "the drop lands",
        ]
        .iter()
        .any(|k| p.message.contains(k))
}

/// A plan as the viewer plays it: the shots back to back over the music,
/// then the end card for as long as the render holds it.
#[derive(Clone, Debug, PartialEq)]
struct Cut {
    /// The shots.
    shots: Vec<SeqShot>,
    /// The music file and the second the plan starts at.
    music: Option<(PathBuf, f64)>,
    /// The end card, when the plan names one and holds it a while.
    card: Option<EndCard>,
}

impl Cut {
    /// Loads it into the viewer (a no-op when it is already there).
    fn open(&self, player: &mut Player) {
        player.open_cut(self.shots.clone(), self.music.clone(), self.card.clone());
    }

    /// Whether the viewer holds this cut.
    fn is_loaded(&self, player: &Player) -> bool {
        matches!(player.source(), Source::Sequence { shots, .. } if *shots == self.shots)
            && player.end_card() == self.card.as_ref()
    }
}

/// The end card as the viewer holds it: the plan's image over the span the
/// render gives it ([`Plan::card_span`]); none without an image, as the
/// render then stops at the card bar.
fn end_card(plan: &Plan, map: &MusicMap) -> Option<EndCard> {
    let (from, to) = plan.card_span(map);
    (!plan.end_card.is_empty() && to > from).then(|| EndCard {
        path: PathBuf::from(&plan.end_card),
        dur: f64::from(to - from),
    })
}

/// The plan as the viewer plays it.
fn sequence(plan: &Plan, project: &Project, map: &MusicMap) -> Cut {
    let shots = plan
        .shots
        .iter()
        .zip(plan.timing(map))
        .map(|(s, (_, dur))| {
            let clip = project.index.as_ref().and_then(|i| i.clip(&s.clip));
            SeqShot {
                path: clip.map(|c| PathBuf::from(&c.path)).unwrap_or_default(),
                t_in: f64::from(s.t_in),
                dur: f64::from(dur),
                cx: s.cx,
            }
        })
        .collect();
    let music = (!plan.music.is_empty()).then(|| {
        (
            PathBuf::from(&plan.music),
            f64::from(map.beat_time(plan.start_bar, 0)),
        )
    });
    Cut {
        shots,
        music,
        card: end_card(plan, map),
    }
}

/// The shots of `name` in the app-wide selection, 0-based.
fn selected(selection: &[Reference], name: &str, len: usize) -> BTreeSet<usize> {
    selection
        .iter()
        .filter_map(|r| match &r.target {
            Target::Shot { plan, index } if plan == name && *index >= 1 && *index <= len => {
                Some(index - 1)
            }
            Target::Shot { .. }
            | Target::Segment { .. }
            | Target::Range { .. }
            | Target::Frame { .. }
            | Target::Bars { .. }
            | Target::Person { .. }
            | Target::Plan { .. } => None,
        })
        .collect()
}

impl State {
    /// Applies an edit to the open plan; records it for undo and autosave
    /// when it changed something.
    fn apply(&mut self, label: impl Into<String>, f: impl FnOnce(&mut Plan) -> bool) -> bool {
        if self.viewing.is_some() || self.locked {
            return false;
        }
        let Some(plan) = self.plan.as_mut() else {
            return false;
        };
        let before = plan.clone();
        if !f(plan) || *plan == before {
            return false;
        }
        let label = label.into();
        self.undo.push(before, label.clone());
        self.changed(label);
        true
    }

    /// Notes a change already made to the plan (undo is the caller's).
    fn changed(&mut self, label: String) {
        self.rev += 1;
        self.status = format!("edited: {label}");
        self.dirty = Some((Instant::now(), label));
    }

    /// Ends a drag that edited the plan live, recording it once.
    fn commit_drag(&mut self, before: Plan, label: String) {
        if self.plan.as_ref().is_some_and(|p| *p != before) {
            self.undo.push(before, label.clone());
            self.changed(label);
        }
    }

    /// Steps back.
    fn undo(&mut self) {
        if self.viewing.is_some() || self.locked {
            return;
        }
        if let Some(plan) = self.plan.as_mut()
            && let Some(label) = self.undo.undo(plan)
        {
            self.changed(format!("undo {label}"));
        }
    }

    /// Steps forward.
    fn redo(&mut self) {
        if self.viewing.is_some() || self.locked {
            return;
        }
        if let Some(plan) = self.plan.as_mut()
            && let Some(label) = self.undo.redo(plan)
        {
            self.changed(format!("redo {label}"));
        }
    }

    /// Saves the pending change as a new version.
    fn save(&mut self, project: &Project) {
        let (Some(name), Some(plan)) = (self.name.as_ref(), self.plan.as_ref()) else {
            return;
        };
        let change = self.dirty.take().map(|(_, l)| l).unwrap_or_default();
        match history::save(&project.work, name, plan, &change, history::now()) {
            Ok(n) => {
                self.status = format!("saved v{n}");
                self.disk_stamp = mtime(&history::live_path(&project.work, name));
                self.versions = history::list(&project.work, name);
            }
            Err(e) => self.status = format!("save failed: {e:#}"),
        }
    }

    /// Opens plan `name` (saving the current one first).
    fn open(&mut self, project: &mut Project, name: &str) {
        if self.dirty.is_some() {
            self.save(project);
        }
        let path = history::live_path(&project.work, name);
        match Plan::load(&path) {
            Ok(plan) => {
                self.plan = Some(plan);
                self.load_error = None;
            }
            Err(e) => {
                self.plan = None;
                self.load_error = Some(format!("{e:#}"));
            }
        }
        self.name = Some(name.to_owned());
        self.undo.clear();
        self.dirty = None;
        self.viewing = None;
        self.compare = None;
        self.alts = None;
        self.anchor = None;
        self.drag = None;
        self.note_edit = None;
        self.comment_edit = None;
        self.comment_sel = None;
        self.comment_focus = None;
        self.locked = lock_active(&lock_path(&project.work, name), SystemTime::now());
        self.view.fit = true;
        self.disk_stamp = mtime(&path);
        self.versions = history::list(&project.work, name);
        self.status = match self.versions.last() {
            Some(v) => format!("v{} · {}", v.n, v.saved),
            None => "no versions yet".to_owned(),
        };
        self.rev += 1;
        let table = project
            .settings
            .pages
            .entry(PAGE_KEY.to_owned())
            .or_default();
        if table.get("current").and_then(|v| v.as_str()) != Some(name) {
            table.insert("current".to_owned(), toml::Value::String(name.to_owned()));
            let _ = project.save_settings();
        }
    }

    /// Picks up a change to the live file made elsewhere (the assistant,
    /// the command line, a draft job), keeping the old state for undo.
    fn reload_from_disk(&mut self, project: &Project) {
        let Some(name) = self.name.clone() else {
            return;
        };
        let path = history::live_path(&project.work, &name);
        let stamp = mtime(&path);
        if stamp == self.disk_stamp {
            return;
        }
        self.disk_stamp = stamp;
        let Ok(fresh) = Plan::load(&path) else {
            return;
        };
        if self.plan.as_ref() == Some(&fresh) {
            return;
        }
        let unix = history::now();
        if let Some(old) = self.plan.take() {
            if self.versions.is_empty()
                && let Ok(text) = toml::to_string_pretty(&old)
            {
                let _ = history::record(&project.work, &name, &text, "original", unix);
            }
            self.undo.push(old, "change on disk");
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            let _ = history::record(&project.work, &name, &text, "changed on disk", unix);
        }
        self.plan = Some(fresh);
        self.load_error = None;
        self.versions = history::list(&project.work, &name);
        self.status = "reloaded: changed on disk".to_owned();
        self.alts = None;
        self.rev += 1;
    }
}

/// Draws the timeline into its panel.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let keys_free = ctx.keys_free(ui);
    let hovered = ui.ui_contains_pointer();
    let preset = ctx.keymap;
    let PageCtx {
        theme: palette,
        project,
        jobs,
        thumbs,
        player,
        selection,
        actions,
        ..
    } = ctx;
    let palette: &Palette = palette;
    let Some(project) = project.as_mut() else {
        return;
    };
    housekeeping(state, project, jobs, player);
    ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);

    header(state, project, jobs, player, palette, actions, ui);
    draft_window(state, project, jobs, palette, ui.ctx());
    tabs::form_window(state, project, player, palette, ui.ctx());
    if state.locked {
        lock_banner(palette, ui);
    }

    let Some(map) = project.music.as_ref() else {
        widgets::empty_state(
            ui,
            palette,
            icons::MUSIC,
            "No music map yet",
            "Run Beats on the Home screen, then draft a plan.",
            |_| {},
        );
        return;
    };
    let shown_plan = match (&state.viewing, &state.plan) {
        (Some(v), _) => v.plan.clone(),
        (None, Some(p)) => p.clone(),
        (None, None) => {
            let mut draft = false;
            match (&state.load_error, project.plans.is_empty()) {
                (Some(e), _) => widgets::empty_state(
                    ui,
                    palette,
                    icons::TRIANGLE_ALERT,
                    "Could not read the plan",
                    e,
                    |_| {},
                ),
                (None, true) => widgets::empty_state(
                    ui,
                    palette,
                    icons::FILM,
                    "No plans yet",
                    "Draft one on the beat grid from the shot log.",
                    |ui| {
                        draft = widgets::button_with(
                            ui,
                            palette,
                            widgets::ButtonKind::Secondary,
                            Some(icons::PLUS),
                            "Draft new plan",
                        )
                        .clicked();
                    },
                ),
                (None, false) => widgets::empty_state(
                    ui,
                    palette,
                    icons::LIST,
                    "Pick a plan",
                    "Open one of the tabs above.",
                    |_| {},
                ),
            }
            if draft {
                open_draft_form(state, project);
            }
            return;
        }
    };
    let name = state.name.clone().unwrap_or_default();

    // Re-check when the shown plan changed.
    let key = (state.rev, state.viewing.as_ref().map(|v| v.version.n));
    if state.checked != Some(key) {
        state.problems = match project.index.as_ref() {
            Some(index) => crate::plan::check(&shown_plan, index, map, &project.people),
            None => Vec::new(),
        };
        state.checked = Some(key);
    }
    let mut issues: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for p in state.problems.iter().filter(|p| p.shot > 0) {
        issues
            .entry(p.shot - 1)
            .or_default()
            .push(p.message.clone());
    }
    let cut = sequence(&shown_plan, project, map);
    let ours = cut.is_loaded(player);
    let (card, total) = shown_plan.card_span(map);
    let s = Shown {
        theme: palette,
        project,
        map,
        timing: shown_plan.timing(map),
        grid: edit::grid(&shown_plan, map),
        starts: edit::starts(&shown_plan),
        card,
        total,
        sel: selected(selection, &name, shown_plan.shots.len()),
        issues,
        editable: state.viewing.is_none() && !state.locked,
        cut,
        ours,
        preset,
        rev: state.rev,
        name,
        plan: shown_plan,
    };
    let mut io = Io {
        thumbs,
        player,
        selection,
        actions,
    };
    let rev_before = state.rev;

    if let Some(v) = &state.viewing {
        let (n, saved, change) = (
            v.version.n,
            v.version.saved.clone(),
            v.version.change.clone(),
        );
        strips::viewing_banner(state, &s, ui, n, &saved, &change);
    }

    let whole = ui.available_rect_before_wrap();
    if state.compare.is_some() {
        compare::view(state, &s, &mut io, ui, whole);
        if keys_free && hovered {
            compare::keys(state, &s, &mut io, ui);
        }
        return;
    }

    // Rows, bottom up: check bar, alternatives, the selected shot, tracks;
    // the comments list, when open, takes the right-hand side. The panel
    // inset is bled back so the canvas and rows run edge to edge.
    let clip = ui.clip_rect();
    let bleed = |gap: f32| gap.clamp(0.0, theme::PANEL_INSET - 1.0);
    let whole = egui::Rect::from_min_max(
        egui::pos2(
            whole.left() - bleed(whole.left() - clip.left()),
            whole.top(),
        ),
        egui::pos2(
            whole.right() + bleed(clip.right() - whole.right()),
            whole.bottom() + bleed(clip.bottom() - whole.bottom()),
        ),
    );
    let side_w = if state.show_comments {
        COMMENTS_W.min(whole.width() * 0.4)
    } else {
        0.0
    };
    let full = egui::Rect::from_min_max(
        whole.min,
        egui::pos2(whole.right() - side_w, whole.bottom()),
    );
    let side = egui::Rect::from_min_max(egui::pos2(whole.right() - side_w, whole.top()), whole.max);
    let check_h = CHECK_H;
    let sel_h = if s.sel.is_empty() { 0.0 } else { SELECTION_H };
    let alts_h = if state.alts.is_some() { ALTS_H } else { 0.0 };
    let tracks_h = (full.height() - check_h - sel_h - alts_h).max(80.0);
    let tracks = egui::Rect::from_min_size(full.min, egui::vec2(full.width(), tracks_h));
    let mut y = tracks.bottom();
    let sel_rect =
        egui::Rect::from_min_size(egui::pos2(full.left(), y), egui::vec2(full.width(), sel_h));
    y += sel_h;
    let alts_rect =
        egui::Rect::from_min_size(egui::pos2(full.left(), y), egui::vec2(full.width(), alts_h));
    y += alts_h;
    let check_rect = egui::Rect::from_min_max(egui::pos2(full.left(), y), full.max);
    ui.allocate_rect(whole, egui::Sense::hover());

    timeline::tracks(state, &s, &mut io, ui, tracks);
    if side_w > 0.0 {
        comments::panel(state, &s, &mut io, ui, side);
    }
    if sel_h > 0.0 {
        strips::selection_bar(state, &s, &mut io, ui, sel_rect);
    }
    if state.alts.is_some() {
        strips::alternatives(state, &s, &mut io, ui, alts_rect);
    }
    strips::check_bar(state, &s, &mut io, ui, check_rect);

    if keys_free && (hovered || state.show_comments || !s.sel.is_empty()) {
        keys(state, &s, &mut io, ui, hovered);
    }

    // Keep the viewer on the edited plan.
    if state.rev != rev_before
        && s.ours
        && let Some(plan) = state.plan.as_ref()
    {
        let pos = io.player.position();
        sequence(plan, project, map).open(io.player);
        io.player.seek(pos);
    }
    if state.dirty.is_some() {
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f32(AUTOSAVE_AFTER + 0.1));
    }
}

/// Saves, reloads and follows jobs between frames.
fn housekeeping(
    state: &mut State,
    project: &mut Project,
    jobs: &crate::jobs::Jobs,
    player: &mut Player,
) {
    if state.work.as_deref() != Some(project.work.as_path()) {
        *state = State {
            work: Some(project.work.clone()),
            waves: std::mem::take(&mut state.waves),
            storyboard: page_flag(project, "storyboard"),
            show_comments: page_flag(project, "comments"),
            ..State::default()
        };
    }
    // The plan list (a plan written elsewhere becomes a tab) and the lock.
    if state
        .last_tabs_check
        .is_none_or(|t| t.elapsed() >= DISK_CHECK)
    {
        state.last_tabs_check = Some(Instant::now());
        tabs::refresh(state, project);
        let locked = state
            .name
            .as_deref()
            .is_some_and(|n| lock_active(&lock_path(&project.work, n), SystemTime::now()));
        if locked != state.locked {
            state.locked = locked;
            state.drag = None;
            state.comment_edit = None;
            state.status = if locked {
                "chat is editing this plan".to_owned()
            } else {
                "chat finished editing".to_owned()
            };
            // Pick up what the assistant wrote as soon as it lets go.
            state.last_disk_check = None;
        }
    }
    // A finished draft: record it and open it.
    if let Some((id, name)) = state.pending_job.clone()
        && let Some(job) = jobs.get(id)
    {
        match &job.status {
            crate::jobs::Status::Running => {}
            crate::jobs::Status::Done => {
                state.pending_job = None;
                if let Ok(text) = std::fs::read_to_string(history::live_path(&project.work, &name))
                {
                    let _ = history::record(&project.work, &name, &text, "draft", history::now());
                }
                project.reload_plans();
                state.open(project, &name);
                open_in_player(state, project, player);
            }
            crate::jobs::Status::Failed(e) => {
                state.pending_job = None;
                state.status = format!("draft failed: {e}");
            }
        }
    }
    // First frame on this night: the remembered plan, or the first.
    if state.name.is_none() && !project.plans.is_empty() {
        let stems: Vec<String> = project
            .plans
            .iter()
            .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .collect();
        let remembered = project
            .settings
            .pages
            .get(PAGE_KEY)
            .and_then(|t| t.get("current"))
            .and_then(|v| v.as_str())
            .filter(|n| stems.iter().any(|s| s == n))
            .map(str::to_owned);
        if let Some(name) = remembered.or_else(|| stems.first().cloned()) {
            state.open(project, &name);
            if matches!(player.source(), Source::Empty) {
                open_in_player(state, project, player);
            }
        }
    }
    if let Some((at, _)) = &state.dirty
        && at.elapsed().as_secs_f32() >= AUTOSAVE_AFTER
        && state.drag.is_none()
        && !state.locked
    {
        state.save(project);
    }
    if state.dirty.is_none()
        && state.drag.is_none()
        && state
            .last_disk_check
            .is_none_or(|t| t.elapsed() >= DISK_CHECK)
    {
        state.last_disk_check = Some(Instant::now());
        state.reload_from_disk(project);
    }
}

/// Loads the open plan into the viewer (paused at its start).
fn open_in_player(state: &State, project: &Project, player: &mut Player) {
    if let (Some(plan), Some(map)) = (state.plan.as_ref(), project.music.as_ref()) {
        sequence(plan, project, map).open(player);
    }
}

/// What the header's right side shows, read before it is drawn (the tab
/// strip on the left holds the state mutably while it draws).
struct HeaderInfo {
    /// What Undo would undo.
    undo: Option<String>,
    /// What Redo would redo.
    redo: Option<String>,
    /// Unresolved comments.
    comments_open: usize,
    /// The comments list is showing.
    show_comments: bool,
    /// The storyboard lane is showing.
    storyboard: bool,
    /// What the compare menu offers, when a plan is open.
    compare: Option<compare::MenuInfo>,
    /// The open plan's versions, oldest first.
    versions: Vec<history::Version>,
    /// The version on show, when not the latest.
    viewing: Option<u32>,
    /// The open plan's aspect.
    aspect: Option<Aspect>,
    /// Whether the plan can be edited.
    editable: bool,
    /// Shots, length, tempo and bars.
    stats: Option<String>,
    /// The save line.
    status: String,
    /// An unsaved change is waiting.
    dirty: bool,
    /// A draft job is running.
    drafting: Option<String>,
}

/// What the header's right side asked for.
#[derive(Default)]
struct HeaderOut {
    /// Undo was pressed.
    undo: bool,
    /// Redo was pressed.
    redo: bool,
    /// The comments toggle was pressed.
    comments: bool,
    /// The storyboard toggle was pressed.
    storyboard: bool,
    /// A version was picked to view.
    version: Option<history::Version>,
    /// An aspect was picked.
    aspect: Option<Aspect>,
    /// Something was picked in the compare menu.
    compare: Option<compare::Pick>,
}

/// Width the header keeps for its right side; the tabs scroll in the rest.
const HEADER_RIGHT: f32 = 520.0;

/// The panel header: the plan tabs on the left; stats, aspect, versions,
/// compare, storyboard, comments, undo and redo on the right.
fn header(
    state: &mut State,
    project: &mut Project,
    jobs: &crate::jobs::Jobs,
    player: &mut Player,
    palette: &Palette,
    actions: &mut Vec<Action>,
    ui: &mut egui::Ui,
) {
    let info = HeaderInfo {
        undo: state.undo.undo_label().map(str::to_owned),
        redo: state.undo.redo_label().map(str::to_owned),
        comments_open: state
            .plan
            .as_ref()
            .map_or(0, |p| p.comments.iter().filter(|c| !c.resolved).count()),
        show_comments: state.show_comments,
        storyboard: state.storyboard,
        compare: compare::menu_info(state, project),
        versions: state.versions.clone(),
        viewing: state.viewing.as_ref().map(|v| v.version.n),
        aspect: state.plan.as_ref().map(|p| p.aspect),
        editable: state.viewing.is_none() && !state.locked,
        stats: match (state.plan.as_ref(), project.music.as_ref()) {
            (Some(plan), Some(map)) => {
                let len = map.beat_time(plan.end_bar, 0) - map.beat_time(plan.start_bar, 0);
                Some(format!(
                    "{} shots · {} · {:.0} bpm · bars {}–{}",
                    plan.shots.len(),
                    timeline::timecode(len, false),
                    map.bpm,
                    plan.start_bar,
                    plan.end_bar
                ))
            }
            (Some(_) | None, _) => None,
        },
        status: state.status.clone(),
        dirty: state.dirty.is_some(),
        drafting: match &state.pending_job {
            Some((_, name)) => Some(format!("Drafting {name}…")),
            None => jobs
                .running()
                .any(|j| matches!(j.cmd, crate::commands::Cmd::Plan { .. }))
                .then(|| "Drafting…".to_owned()),
        },
    };
    let mut out = HeaderOut::default();
    widgets::panel_bar(
        ui,
        palette,
        |ui| {
            let room = (ui.available_width() - HEADER_RIGHT).max(160.0);
            egui::ScrollArea::horizontal()
                .id_salt("plan_tabs")
                .max_width(room)
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        tabs::strip(state, project, player, palette, actions, ui);
                    });
                });
        },
        |ui| out = header_right(ui, palette, &info),
    );
    if out.undo {
        state.undo();
    }
    if out.redo {
        state.redo();
    }
    if out.comments {
        state.show_comments = !state.show_comments;
        set_page_flag(project, "comments", state.show_comments);
    }
    if out.storyboard {
        state.storyboard = !state.storyboard;
        set_page_flag(project, "storyboard", state.storyboard);
    }
    if let Some(v) = out.version {
        let latest = state.versions.last().map(|v| v.n);
        match history::load(&v) {
            Ok(plan) => {
                state.viewing = (Some(v.n) != latest).then(|| Viewing {
                    version: v.clone(),
                    plan,
                });
                state.alts = None;
            }
            Err(e) => state.status = format!("v{}: {e:#}", v.n),
        }
    }
    if let (Some(aspect), Some(plan)) = (out.aspect, state.plan.as_ref())
        && aspect != plan.aspect
    {
        state.apply(format!("aspect {}", aspect_key(aspect)), |p| {
            p.aspect = aspect;
            let half = aspect.crop_frac() / 2.0;
            for s in &mut p.shots {
                s.cx = s.cx.clamp(half, 1.0 - half);
            }
            true
        });
    }
    if let Some(pick) = out.compare {
        compare::take(state, project, pick);
    }
}

/// The header's right side, laid out right to left.
fn header_right(ui: &mut egui::Ui, t: &Palette, info: &HeaderInfo) -> HeaderOut {
    let mut out = HeaderOut::default();
    ui.add_enabled_ui(info.redo.is_some(), |ui| {
        let label = format!("Redo {}", info.redo.as_deref().unwrap_or(""))
            .trim_end()
            .to_owned();
        out.redo = widgets::icon_btn(ui, t, icons::REDO, &label, "Ctrl+Shift+Z").clicked();
    });
    ui.add_enabled_ui(info.undo.is_some(), |ui| {
        let label = format!("Undo {}", info.undo.as_deref().unwrap_or(""))
            .trim_end()
            .to_owned();
        out.undo = widgets::icon_btn(ui, t, icons::UNDO, &label, "Ctrl+Z").clicked();
    });
    widgets::bar_divider(ui, t);
    let comments = widgets::icon_toggle(
        ui,
        t,
        icons::MESSAGE_SQUARE,
        "Comments: review notes pinned to moments of the cut",
        "F adds one",
        info.show_comments,
    );
    if info.comments_open > 0 {
        count_badge(ui, t, comments.rect, info.comments_open);
    }
    out.comments = comments.clicked();
    out.storyboard = widgets::icon_toggle(
        ui,
        t,
        icons::LAYOUT_GRID,
        "Storyboard: shot cards with their notes, by song section",
        "",
        info.storyboard,
    )
    .clicked();
    if let Some(menu) = &info.compare {
        out.compare = compare::menu(ui, t, menu);
    }
    widgets::bar_divider(ui, t);

    let muted = theme::rgb(t.text_muted);
    let small = theme::font(theme::BODY_SIZE);
    let (status, color) = if info.dirty {
        ("Unsaved".to_owned(), theme::rgb(t.warning))
    } else if info.status.starts_with("save failed") {
        (info.status.clone(), theme::rgb(t.danger))
    } else {
        (info.status.clone(), theme::rgb(t.text_faint))
    };
    if ui.available_width() > 360.0 {
        ui.add(
            egui::Label::new(egui::RichText::new(status).font(small.clone()).color(color))
                .truncate(),
        );
    }
    // Versions.
    let label = match (info.viewing, info.versions.last()) {
        (Some(n), _) => format!("Viewing v{n}"),
        (None, Some(v)) => format!("v{}", v.n),
        (None, None) => "v–".to_owned(),
    };
    ui.add_enabled_ui(!info.versions.is_empty(), |ui| {
        let resp = text_menu_button(ui, t, Some(icons::HISTORY), &label);
        let resp = widgets::tip(resp, t, "Versions: every save keeps a numbered copy", "");
        let latest = info.versions.last().map(|v| v.n);
        let _ = egui::Popup::menu(&resp)
            .width(340.0)
            .gap(theme::SPACE_XS)
            .show(|ui| {
                ui.horizontal(|ui| {
                    ui.add_space(theme::SPACE_SM);
                    widgets::menu_heading(ui, t, "Versions");
                });
                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for v in info.versions.iter().rev() {
                            let text = format!(
                                "v{} · {} · {}",
                                v.n,
                                v.saved.trim_end_matches(" UTC"),
                                if v.change.is_empty() {
                                    "save"
                                } else {
                                    &v.change
                                },
                            );
                            let on = info.viewing.map_or(Some(v.n) == latest, |n| n == v.n);
                            let keys = if Some(v.n) == latest { "current" } else { "" };
                            if widgets::menu_item(ui, t, None, &text, keys, on).clicked() {
                                out.version = Some(v.clone());
                                ui.close();
                            }
                        }
                    });
            });
    });
    // Aspect.
    if let Some(aspect) = info.aspect {
        ui.add_enabled_ui(info.editable, |ui| {
            let resp = text_menu_button(ui, t, None, aspect_key(aspect));
            let resp = widgets::tip(resp, t, "Output aspect; crops follow each shot's cx", "");
            let _ = egui::Popup::menu(&resp)
                .width(180.0)
                .gap(theme::SPACE_XS)
                .show(|ui| {
                    for (a, glyph) in [
                        (Aspect::Vertical, icons::SMARTPHONE),
                        (Aspect::Feed, icons::PORTRAIT),
                        (Aspect::Wide, icons::MONITOR),
                    ] {
                        if widgets::menu_item(ui, t, Some(glyph), aspect_key(a), "", a == aspect)
                            .clicked()
                        {
                            out.aspect = Some(a);
                            ui.close();
                        }
                    }
                });
        });
    }
    if let Some(stats) = &info.stats
        && ui.available_width() > 220.0
    {
        ui.add(
            egui::Label::new(
                egui::RichText::new(stats)
                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                    .color(muted),
            )
            .truncate(),
        );
    }
    if let Some(drafting) = &info.drafting {
        ui.label(egui::RichText::new(drafting).font(small).color(muted));
        ui.spinner();
    }
    out
}

/// A quiet menu trigger in a header: 12 px Medium muted text (and an
/// optional icon) with a chevron, ink 8 % under the pointer.
fn text_menu_button(
    ui: &mut egui::Ui,
    t: &Palette,
    glyph: Option<char>,
    text: &str,
) -> egui::Response {
    let font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER);
    let icon_w = if glyph.is_some() {
        icons::SMALL + theme::SPACE_XS
    } else {
        0.0
    };
    let size = egui::vec2(
        theme::SPACE_SM
            + icon_w
            + galley.size().x
            + theme::SPACE_XS
            + icons::SMALL
            + theme::SPACE_XS,
        theme::CONTROL_HEIGHT,
    );
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    let painter = ui.painter();
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    if resp.hovered() || open {
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(theme::RADIUS_MD),
            theme::ink_hover(t),
        );
    }
    let ink = if resp.hovered() || open {
        theme::rgb(t.text)
    } else {
        theme::rgb(t.text_muted)
    };
    let mut x = rect.left() + theme::SPACE_SM;
    if let Some(g) = glyph {
        icons::paint(
            painter,
            egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - icons::SMALL / 2.0),
                egui::Vec2::splat(icons::SMALL),
            ),
            g,
            icons::SMALL,
            ink,
        );
        x += icon_w;
    }
    painter.galley(
        egui::pos2(x, rect.center().y - galley.size().y / 2.0),
        galley.clone(),
        ink,
    );
    x += galley.size().x + theme::SPACE_XS;
    icons::paint(
        painter,
        egui::Rect::from_min_size(
            egui::pos2(x, rect.center().y - icons::SMALL / 2.0),
            egui::Vec2::splat(icons::SMALL),
        ),
        icons::CHEVRON_DOWN,
        icons::SMALL,
        ink.gamma_multiply(0.7),
    );
    resp
}

/// A count over the top-right corner of an icon button.
fn count_badge(ui: &egui::Ui, t: &Palette, button: egui::Rect, n: usize) {
    let text = if n > 99 {
        "99+".to_owned()
    } else {
        n.to_string()
    };
    let galley = ui.painter().layout_no_wrap(
        text,
        theme::medium(ui.ctx(), theme::TINY_SIZE),
        theme::rgb(t.text_on_accent),
    );
    let w = (galley.size().x + 6.0).max(14.0);
    let rect = egui::Rect::from_min_size(
        egui::pos2(button.right() - w + 2.0, button.top() - 1.0),
        egui::vec2(w, 14.0),
    );
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::same(7), theme::rgb(t.accent));
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::rgb(t.text_on_accent),
    );
}

/// A boolean page option from the night's settings.
fn page_flag(project: &Project, key: &str) -> bool {
    project
        .settings
        .pages
        .get(PAGE_KEY)
        .and_then(|t| t.get(key))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
}

/// Stores a boolean page option.
fn set_page_flag(project: &mut Project, key: &str, on: bool) {
    project
        .settings
        .pages
        .entry(PAGE_KEY.to_owned())
        .or_default()
        .insert(key.to_owned(), toml::Value::Boolean(on));
    let _ = project.save_settings();
}

/// The banner over the tracks while the assistant holds the plan's lock.
fn lock_banner(palette: &Palette, ui: &mut egui::Ui) {
    banner(
        ui,
        theme::translucent(palette.accent, 26),
        icons::LOCK,
        theme::rgb(palette.accent),
        |ui| {
            ui.label(
                egui::RichText::new(
                    "Chat is editing this plan. The timeline is read-only until it finishes.",
                )
                .font(theme::font(theme::BODY_SIZE))
                .color(theme::rgb(palette.text)),
            );
            ui.spinner();
        },
    );
}

/// A slim notice row under the header: a tinted fill with 8 px corners,
/// an icon in `ink`, then `add`.
pub(super) fn banner(
    ui: &mut egui::Ui,
    fill: egui::Color32,
    glyph: char,
    ink: egui::Color32,
    add: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(theme::SPACE_XS);
    egui::Frame::new()
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(theme::margin_xy(theme::SPACE_SM, theme::SPACE_XS))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(theme::COMPACT_HEIGHT);
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
                icons::icon(ui, glyph, icons::SMALL, ink);
                add(ui);
            });
        });
    ui.add_space(theme::SPACE_XS);
}

/// Opens the "Draft new…" form, shaped like the open plan.
fn open_draft_form(state: &mut State, project: &Project) {
    let aspect = state.plan.as_ref().map_or(Aspect::Vertical, |p| p.aspect);
    state.draft = Some(DraftForm {
        name: String::new(),
        aspect,
        seconds: 30.0,
        end_card: end_card_for(project, aspect),
        card_bars: 2,
        error: None,
    });
}

/// The end card the night's settings name for an aspect.
fn end_card_for(project: &Project, aspect: Aspect) -> String {
    project
        .settings
        .end_cards
        .get(aspect_key(aspect))
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// Whether `name` is a usable plan file name.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The "Draft new…" window.
fn draft_window(
    state: &mut State,
    project: &mut Project,
    jobs: &mut crate::jobs::Jobs,
    palette: &Palette,
    ctx: &egui::Context,
) {
    let Some(form) = state.draft.as_mut() else {
        return;
    };
    let mut open = true;
    let mut go = false;
    let mut cancel = false;
    egui::Window::new("Draft a new plan")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| {
            egui::Grid::new("draft_form")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Name");
                    ui.add(
                        egui::TextEdit::singleline(&mut form.name)
                            .hint_text("reel-30")
                            .desired_width(180.0),
                    );
                    ui.end_row();
                    ui.label("Aspect");
                    ui.horizontal(|ui| {
                        let before = form.aspect;
                        for a in [Aspect::Vertical, Aspect::Feed, Aspect::Wide] {
                            ui.selectable_value(&mut form.aspect, a, aspect_key(a));
                        }
                        if form.aspect != before {
                            form.end_card = end_card_for(project, form.aspect);
                        }
                    });
                    ui.end_row();
                    ui.label("Length");
                    ui.horizontal(|ui| {
                        for secs in [15.0, 30.0, 60.0] {
                            ui.selectable_value(&mut form.seconds, secs, format!("{secs:.0}s"));
                        }
                        ui.add(
                            egui::DragValue::new(&mut form.seconds)
                                .range(8.0..=240.0)
                                .suffix(" s"),
                        );
                    });
                    ui.end_row();
                    ui.label("End card");
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut form.end_card)
                                .hint_text("image path (optional)")
                                .desired_width(180.0),
                        );
                        ui.add(
                            egui::DragValue::new(&mut form.card_bars)
                                .range(0..=8)
                                .suffix(" bars"),
                        );
                    });
                    ui.end_row();
                });
            let exists = project.plans.iter().any(|p| {
                p.file_stem()
                    .is_some_and(|s| s.to_string_lossy() == form.name)
            });
            if exists {
                ui.label(
                    egui::RichText::new(
                        "A plan with this name exists: it is kept in its history, then replaced.",
                    )
                    .size(11.0)
                    .color(theme::rgb(palette.warning)),
                );
            }
            if let Some(e) = &form.error {
                ui.label(
                    egui::RichText::new(e)
                        .size(11.0)
                        .color(theme::rgb(palette.danger)),
                );
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("Draft").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if go {
        let name = form.name.trim().to_owned();
        if !valid_name(&name) {
            form.error = Some("Use letters, digits, - and _ (e.g. reel-30).".to_owned());
        } else if project.music.is_none() || project.index.is_none() {
            form.error = Some("Index the footage and map the music first.".to_owned());
        } else {
            let _ = history::keep_original(&project.work, &name, history::now());
            let id = jobs.spawn(
                format!("plan {name}"),
                crate::commands::Cmd::Plan {
                    name: name.clone(),
                    work: project.work.clone(),
                    aspect: aspect_key(form.aspect).to_owned(),
                    seconds: form.seconds,
                    end_card: form.end_card.trim().to_owned(),
                    card_bars: form.card_bars,
                },
            );
            state.pending_job = Some((id, name));
            state.status = "drafting…".to_owned();
            state.draft = None;
            return;
        }
    }
    if !open || cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.draft = None;
    }
}

/// Opens the alternatives strip for slot `k`.
fn open_alternatives(state: &mut State, s: &Shown<'_>, k: usize) {
    let Some(index) = s.project.index.as_ref() else {
        state.status = "alternatives need the shot log (index)".to_owned();
        return;
    };
    let items =
        crate::alternatives::rank(&s.plan, k, index, s.map, &s.project.people, ALTERNATIVES);
    state.alts = Some(Alts {
        slot: k,
        rev: s.rev,
        items,
    });
}

/// Swaps alternative `n` (0-based) of the open strip into its slot.
fn take_alternative(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, n: usize) {
    let Some(alts) = state.alts.as_ref() else {
        return;
    };
    let (slot, Some(alt)) = (alts.slot, alts.items.get(n).cloned()) else {
        return;
    };
    let beats = s.plan.shots.get(slot).map_or(1, |sh| sh.beats);
    if state.apply(format!("swap shot {}", slot + 1), |p| {
        edit::swap(p, slot, alt.to_shot(beats))
    }) {
        state.alts = None;
        io.select(&s.name, &BTreeSet::from([slot]));
    }
}

/// Lifts the selected shots (each one's beats go to the shot before it).
fn lift_selected(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>) {
    if s.sel.is_empty() {
        return;
    }
    let list: Vec<String> = s.sel.iter().map(|k| (k + 1).to_string()).collect();
    let sel: Vec<usize> = s.sel.iter().rev().copied().collect();
    if state.apply(format!("lift {}", list.join(",")), |p| {
        let mut any = false;
        for k in sel {
            any |= edit::lift(p, k);
        }
        any
    }) {
        state.alts = None;
        io.select(&s.name, &BTreeSet::new());
    }
}

/// Plays the plan from the start of shot `k`.
fn play_from(s: &Shown<'_>, io: &mut Io<'_>, k: usize) {
    let t = s.timing.get(k).map_or(0.0, |&(t, _)| t);
    io.seek(s, t);
    io.player.play();
}

/// The region's keys.
fn keys(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, ui: &egui::Ui, hovered: bool) {
    use egui::{Key, Modifiers};
    let (undo, redo) = ui.input_mut(|i| {
        let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
            || i.consume_key(Modifiers::COMMAND, Key::Y);
        (i.consume_key(Modifiers::COMMAND, Key::Z), redo)
    });
    if undo {
        state.undo();
    }
    if redo {
        state.redo();
    }
    let plain = |key: Key| ui.input(|i| i.key_pressed(key) && i.modifiers.is_none());
    // F: a comment at the playhead (over the timeline or in Comment mode).
    if (hovered || state.show_comments) && plain(Key::F) {
        comments::add_at_playhead(state, s, io);
    }
    if plain(Key::Delete) || plain(Key::Backspace) {
        lift_selected(state, s, io);
    }
    if plain(Key::A)
        && let Some(k) = s.single().or_else(|| s.sel.first().copied())
    {
        open_alternatives(state, s, k);
    }
    if state.alts.is_some() {
        const DIGITS: [Key; 8] = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
        ];
        if let Some(n) = DIGITS.iter().position(|&d| plain(d)) {
            take_alternative(state, s, io, n);
        }
    }
    if plain(Key::Escape) {
        if state.alts.is_some() {
            state.alts = None;
        } else {
            io.select(&s.name, &BTreeSet::new());
        }
    }
    if hovered {
        if plain(Key::V) {
            state.tool = Tool::Select;
        }
        if plain(Key::T) {
            state.tool = Tool::Slip;
        }
        if ui.input(|i| i.key_pressed(Key::Z) && i.modifiers.shift && !i.modifiers.command) {
            state.view.fit = true;
        }
        if ui.input(|i| crate::gui::ui::keymap::pressed(i, "plan-compare", s.preset)) {
            match compare::default_choice(state, s.project) {
                Some((label, plan)) => compare::open(state, label, plan),
                None => state.status = "nothing to compare with yet".to_owned(),
            }
        }
        // Space plays the timeline: load it so the viewer's Space plays it.
        if plain(Key::Space) && !s.ours {
            let t = s
                .single()
                .and_then(|k| s.timing.get(k))
                .map_or(0.0, |&(t, _)| t);
            io.seek(s, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LOCK_STALE, is_error, lock_active, lock_path, selected, valid_name};

    #[test]
    fn the_assistant_lock_is_a_fresh_file() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = lock_path(dir.path(), "reel-30");
        assert!(
            path.ends_with("plans/.lock-reel-30"),
            "beside the plan, hidden from the plan list: {}",
            path.display()
        );
        let now = std::time::SystemTime::now();
        assert!(!lock_active(&path, now), "no file, no lock");
        std::fs::create_dir_all(dir.path().join("plans"))?;
        std::fs::write(&path, "")?;
        assert!(lock_active(&path, now), "a fresh file locks");
        assert!(
            !lock_active(&path, now + LOCK_STALE + std::time::Duration::from_secs(1)),
            "a stale one (crashed writer) does not"
        );
        Ok(())
    }
    use crate::plan::Problem;
    use crate::refs::{Reference, Target};

    #[test]
    fn plan_names_are_file_safe() -> anyhow::Result<()> {
        assert!(valid_name("reel-30"), "plain");
        assert!(valid_name("wide_60"), "underscore");
        assert!(!valid_name(""), "empty");
        assert!(!valid_name(".history"), "hidden");
        assert!(!valid_name("a/b"), "no paths");
        Ok(())
    }

    #[test]
    fn the_selection_maps_to_shots_of_this_plan() -> anyhow::Result<()> {
        let shot = |plan: &str, index| {
            Reference::new(Target::Shot {
                plan: plan.into(),
                index,
            })
        };
        let sel = vec![
            shot("reel-30", 2),
            shot("reel-15", 1),
            shot("reel-30", 9),
            Reference::new(Target::Person { name: "x".into() }),
        ];
        let got: Vec<usize> = selected(&sel, "reel-30", 5).into_iter().collect();
        assert_eq!(
            got,
            vec![1],
            "1-based to 0-based, other plans and out of range dropped"
        );
        Ok(())
    }

    /// A night with a music map (120 bpm: a bar is 2 s) and two plans,
    /// `reel-a` and `reel-b`, both ending on a 1-bar end card. No index:
    /// the timeline and the viewer need none.
    fn night() -> anyhow::Result<(tempfile::TempDir, std::path::PathBuf)> {
        use crate::plan::{Aspect, Plan, Rules, Shot};
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-12").join("_ave");
        std::fs::create_dir_all(work.join("plans"))?;
        crate::beats::MusicMap {
            path: "m.wav".into(),
            duration: 60.0,
            bpm: 120.0,
            beats: (0..120).map(|i| i as f32 * 0.5).collect(),
            bars: (0..30)
                .map(|i| crate::beats::Bar {
                    t: i as f32 * 2.0,
                    rms: 1.0,
                    low: 1.0,
                })
                .collect(),
            ..crate::beats::MusicMap::default()
        }
        .save(&work)?;
        let shot = |clip: &str, t_in: f32, note: &str| Shot {
            clip: clip.into(),
            t_in,
            beats: 4,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: note.into(),
        };
        let plan = |name: &str, shots: Vec<Shot>| Plan {
            name: name.into(),
            aspect: Aspect::Vertical,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 1,
            card_bar: 2,
            end_bar: 3,
            end_card: work.join("card.png").display().to_string(),
            rules: Rules::default(),
            finish: Default::default(),
            shots,
            comments: Vec::new(),
        };
        plan(
            "reel-a",
            vec![shot("A", 1.0, "opener"), shot("B", 2.0, "the drop")],
        )
        .save(&super::history::live_path(&work, "reel-a"))?;
        plan(
            "reel-b",
            vec![shot("B", 2.0, "drop first"), shot("C", 5.0, "new closer")],
        )
        .save(&super::history::live_path(&work, "reel-b"))?;
        Ok((dir, work))
    }

    /// The app on the timeline of `work`, headless.
    fn harness(
        work: std::path::PathBuf,
    ) -> egui_kittest::Harness<'static, Option<crate::gui::app::App>> {
        use crate::gui::app::{Action, App, Page};
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1680.0, 1000.0))
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
                        app
                    });
                    app.show(ui);
                },
                None,
            );
        harness.run_steps(6);
        harness
    }

    #[test]
    fn timeline_playback_holds_the_end_card_like_the_render() -> anyhow::Result<()> {
        let (_dir, work) = night()?;
        let mut h = harness(work.clone());
        let app = h
            .state_mut()
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no app"))?;
        assert_eq!(
            app.pages.plan.name.as_deref(),
            Some("reel-a"),
            "the first plan opens"
        );
        let card = app
            .player
            .end_card()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("the viewer has no end card"))?;
        assert_eq!(card.path, work.join("card.png"), "the plan's image");
        assert!(
            (card.dur - 2.0).abs() < 1e-6,
            "card_bar 2 to end_bar 3 is one bar, 2 s: {}",
            card.dur
        );
        assert!(
            (app.player.duration() - 6.0).abs() < 1e-6,
            "4 s of shots then the card, as long as the render: {}",
            app.player.duration()
        );
        app.player.seek(5.0);
        h.run_steps(3);
        let app = h
            .state()
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no app"))?;
        assert!(app.player.in_card(), "the playhead is on the card");
        assert_eq!(app.player.here(), None, "showing the card, not a clip");
        assert!(
            (app.player.position() - 5.0).abs() < 1e-6,
            "and stays there: the timeline does not stop at the last shot"
        );
        Ok(())
    }

    #[test]
    fn compare_lists_differences_plays_either_side_and_adopts_b() -> anyhow::Result<()> {
        use egui_kittest::kittest::Queryable;
        let (_dir, work) = night()?;
        let mut h = harness(work.clone());
        h.query_by_label("Compare plans")
            .ok_or_else(|| anyhow::anyhow!("no Compare menu in the tab strip"))?
            .click();
        h.run_steps(3);
        h.query_by_label("Plan reel-b")
            .ok_or_else(|| anyhow::anyhow!("reel-b is not offered"))?
            .click();
        h.run_steps(3);
        {
            let app = h
                .state()
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no app"))?;
            let c = app
                .pages
                .plan
                .compare
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("compare did not open"))?;
            assert_eq!(c.label, "reel-b", "B is the picked plan");
            let a = app
                .pages
                .plan
                .plan
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no plan"))?;
            let kinds: Vec<super::compare::Kind> = super::compare::diff(a, &c.plan)
                .iter()
                .map(|d| d.kind)
                .collect();
            assert_eq!(
                kinds,
                vec![
                    super::compare::Kind::Removed,
                    super::compare::Kind::Moved,
                    super::compare::Kind::Added
                ],
                "A's opener gone, the drop moved up, a new closer"
            );
        }
        assert!(
            h.query_all_by_label_contains("moved").next().is_some(),
            "the diff list shows the move"
        );
        assert!(
            h.query_all_by_label_contains("new closer").next().is_some(),
            "with the shot's note"
        );
        h.query_by_label("Play B")
            .ok_or_else(|| anyhow::anyhow!("no Play B"))?
            .click();
        h.run_steps(3);
        {
            let app = h
                .state()
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("no app"))?;
            let first = match app.player.source() {
                crate::gui::player::Source::Sequence { shots, .. } => shots.first().map(|s| s.t_in),
                crate::gui::player::Source::Empty
                | crate::gui::player::Source::Clip { .. }
                | crate::gui::player::Source::Audio { .. } => None,
            };
            assert_eq!(first, Some(2.0), "the viewer plays B (B @ 2.0s first)");
            assert!(app.player.end_card().is_some(), "B's end card too");
        }
        h.query_by_label("Adopt B")
            .ok_or_else(|| anyhow::anyhow!("no Adopt B"))?
            .click();
        h.run_steps(3);
        let app = h
            .state()
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no app"))?;
        assert!(
            app.pages.plan.compare.is_none(),
            "adopting ends the compare"
        );
        let live = crate::plan::Plan::load(&super::history::live_path(&work, "reel-a"))?;
        assert_eq!(live.name, "reel-a", "keeps its own name");
        let clips: Vec<&str> = live.shots.iter().map(|s| s.clip.as_str()).collect();
        assert_eq!(clips, vec!["B", "C"], "with B's shots");
        let versions = super::history::list(&work, "reel-a");
        assert_eq!(
            versions
                .iter()
                .map(|v| v.change.as_str())
                .collect::<Vec<_>>(),
            vec!["original", "adopt reel-b"],
            "saved through the version path, the original kept"
        );
        Ok(())
    }

    #[test]
    fn structural_problems_are_errors() -> anyhow::Result<()> {
        let p = |shot, m: &str| Problem {
            shot,
            message: m.into(),
        };
        assert!(is_error(&p(0, "shots fill 30 beats")), "whole-plan");
        assert!(
            is_error(&p(3, "A 1.00-9.00 runs outside the clip (5.00s)")),
            "range"
        );
        assert!(
            !is_error(&p(3, "`x` again, 2 shots after shot 1 (need 9)")),
            "repeat"
        );
        Ok(())
    }
}

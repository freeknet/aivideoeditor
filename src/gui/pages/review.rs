// ============================================================================
// Module:       gui::pages::review
// Description:  The browser's Shots tab: the shot log as a culling grid or a
//               dense list, filtered by the top bar's query, with pick /
//               reject / rating, more-like-this and references for chat.
//
// Dependencies: egui; crate::{search, refs, index}, crate::gui::pages
// ============================================================================

//! The Shots tab.
//!
//! The top bar's text is a [`crate::search`] query; every recognised token
//! shows as a removable chip above the results. Results are segments, as
//! thumbnail cards (hover to scrub through the segment) or as a dense list
//! of the shot-log numbers, virtualised so a night of 400+ segments scrolls
//! smoothly.
//!
//! | Gesture | Effect |
//! |---|---|
//! | click / Shift-click / Ctrl-click | select one / a range / toggle; the viewer loads the segment |
//! | double-click, Enter | play it |
//! | arrows (Shift extends) | move the selection |
//! | P / X / U | pick / reject / clear, then advance to the next shot |
//! | 1-5, 0 | rate, clear the rating |
//! | Shift-F | more like this (`like:<id>`) |
//! | Ctrl-A | select every result |
//! | right-click | the same as a menu, plus "Reference in chat ▸ …" |
//!
//! ## Review modes
//!
//! The header's Tape / Compare / Survey switch (or its keys, which follow
//! the keyboard preset through [`keymap::chord`]) changes how the results
//! are reviewed; `Esc` or the same key again comes back to the grid.
//!
//! | Mode | Key | What it is |
//! |---|---|---|
//! | Tape | T | every result back to back in the viewer; the playing card is lit and the selection follows the playhead; Shift+L Fast Review ([`tape`]) |
//! | Compare | C | the select and a candidate two-up with their numbers ([`compare`]) |
//! | Survey | G | the selection tiled; X drops one, `]` / `[` next / previous group ([`survey`]) |
//!
//! Marks are saved to `<work>/review.toml` (see [`crate::search::marks`]).
//! View options persist in `ave.toml` under `[pages.review]`.

mod compare;
mod grid;
mod list;
mod survey;
mod tape;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::PageCtx;
use crate::gui::app::Action;
use crate::gui::ui::keymap::{self, Preset};
use crate::gui::ui::theme::{self, SPACE_LG, SPACE_SM, SPACE_XS};
use crate::gui::ui::{icons, widgets};
use crate::index::Index;
use crate::refs::{Modifier, Reference, Target};
use crate::search::marks::{Marks, Verdict};
use crate::search::{self, Night, Query};
use crate::theme::Palette;

/// Result order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum Sort {
    /// Search score (quality when nothing ranks).
    #[default]
    Best,
    /// Quality, best first.
    Quality,
    /// Clip then time: the order it was shot.
    Time,
    /// Most fire first.
    Fire,
    /// Most moving light first.
    Motion,
}

impl Sort {
    /// Every order, in menu order.
    const ALL: [Self; 5] = [
        Self::Best,
        Self::Quality,
        Self::Time,
        Self::Fire,
        Self::Motion,
    ];

    /// The dropdown's label: one word.
    fn short(self) -> &'static str {
        match self {
            Self::Best => "Best",
            Self::Quality => "Quality",
            Self::Time => "Time",
            Self::Fire => "Fire",
            Self::Motion => "Motion",
        }
    }

    /// The menu label.
    fn label(self) -> &'static str {
        match self {
            Self::Best => "Best match",
            Self::Quality => "Quality",
            Self::Time => "Time",
            Self::Fire => "Fire",
            Self::Motion => "Motion",
        }
    }

    /// The word saved in `ave.toml`.
    fn key(self) -> &'static str {
        match self {
            Self::Best => "best",
            Self::Quality => "quality",
            Self::Time => "time",
            Self::Fire => "fire",
            Self::Motion => "motion",
        }
    }
}

/// Grid of cards or list of rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum View {
    /// Thumbnail cards.
    #[default]
    Grid,
    /// Dense rows.
    List,
}

/// How the results are being reviewed.
#[derive(Clone, Debug, PartialEq, Default)]
enum Mode {
    /// Browsing the grid or the list.
    #[default]
    Browse,
    /// The results play back to back in the viewer.
    Tape(tape::Tape),
    /// The select against a candidate.
    Compare(compare::Compare),
    /// The selection tiled n-up.
    Survey(survey::Survey),
}

/// Which review mode, without its data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// The grid or the list.
    Browse,
    /// Tape mode.
    Tape,
    /// Compare mode.
    Compare,
    /// Survey mode.
    Survey,
}

impl Mode {
    /// The mode's kind.
    fn kind(&self) -> Kind {
        match self {
            Self::Browse => Kind::Browse,
            Self::Tape(_) => Kind::Tape,
            Self::Compare(_) => Kind::Compare,
            Self::Survey(_) => Kind::Survey,
        }
    }
}

/// The tab's view options.
#[derive(Clone, Debug, PartialEq)]
struct Options {
    sort: Sort,
    view: View,
    /// Collapse each clip's segments into a stack.
    group: bool,
    /// Hide results scoring below this.
    cutoff: f32,
    /// Hovering a card scrubs through its segment.
    scrub: bool,
    /// Card width, points.
    card: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            sort: Sort::Best,
            view: View::Grid,
            group: false,
            cutoff: 0.0,
            scrub: true,
            card: 160.0,
        }
    }
}

impl Options {
    /// Reads `[pages.review]`, keeping defaults for anything missing or odd.
    fn from_table(t: Option<&toml::Table>) -> Self {
        let mut o = Self::default();
        let Some(t) = t else {
            return o;
        };
        if let Some(s) = t.get("sort").and_then(|v| v.as_str()) {
            o.sort = Sort::ALL
                .into_iter()
                .find(|x| x.key() == s)
                .unwrap_or(o.sort);
        }
        if let Some(v) = t.get("view").and_then(|v| v.as_str()) {
            o.view = if v == "list" { View::List } else { View::Grid };
        }
        if let Some(g) = t.get("group").and_then(toml::Value::as_bool) {
            o.group = g;
        }
        if let Some(c) = t.get("cutoff").and_then(toml::Value::as_float) {
            o.cutoff = (c as f32).clamp(0.0, 1.0);
        }
        if let Some(s) = t.get("scrub").and_then(toml::Value::as_bool) {
            o.scrub = s;
        }
        if let Some(c) = t.get("card").and_then(toml::Value::as_float) {
            o.card = (c as f32).clamp(CARD_MIN, CARD_MAX);
        }
        o
    }

    /// The options as `[pages.review]`.
    fn to_table(&self) -> toml::Table {
        let mut t = toml::Table::new();
        t.insert("sort".into(), self.sort.key().into());
        t.insert(
            "view".into(),
            match self.view {
                View::Grid => "grid",
                View::List => "list",
            }
            .into(),
        );
        t.insert("group".into(), self.group.into());
        t.insert("cutoff".into(), f64::from(self.cutoff).into());
        t.insert("scrub".into(), self.scrub.into());
        t.insert("card".into(), f64::from(self.card.round()).into());
        t
    }
}

/// Narrowest card.
const CARD_MIN: f32 = 110.0;
/// Widest card.
const CARD_MAX: f32 = 320.0;

/// Where a result sits in a clip stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stack {
    /// Not grouped.
    Alone,
    /// The best of a clip's results, standing for `count` of them.
    Top {
        /// Results in the stack, this one included.
        count: usize,
        /// Whether the rest are shown after it.
        open: bool,
    },
    /// One of an open stack's other results.
    Member,
}

/// One result, ready to draw.
#[derive(Clone, Debug)]
pub(super) struct Item {
    /// Index into `Index::segments`.
    pub(super) seg: usize,
    /// Search score.
    pub(super) score: f32,
    /// Why it matched.
    pub(super) reason: String,
    /// Stack position.
    pub(super) stack: Stack,
}

/// What the cached results were computed from.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    search: String,
    marks_rev: u64,
    segments: usize,
    captioned: usize,
    people: usize,
    sort: Sort,
    group: bool,
    cutoff: u32,
    expanded_rev: u64,
}

/// The tab's state, kept across frames.
#[derive(Debug, Default)]
pub(crate) struct State {
    work: Option<PathBuf>,
    marks: Marks,
    marks_rev: u64,
    opts: Options,
    key: Option<Key>,
    items: Vec<Item>,
    /// Hits before the cut-off.
    matched: usize,
    paths: HashMap<String, PathBuf>,
    /// Segment ids by index position, so results map to ids without the index.
    ids: Vec<String>,
    expanded: BTreeSet<String>,
    expanded_rev: u64,
    /// Where a Shift-click range starts (segment id).
    anchor: Option<String>,
    /// The focused result (segment id): where arrows move from.
    cursor: Option<String>,
    /// Scroll the cursor into view next frame.
    reveal: bool,
    /// The last click landed outside the tab: its keys are off.
    inactive: bool,
    scroll_y: f32,
    view_h: f32,
    columns: usize,
    /// The last problem saving or loading marks.
    error: Option<String>,
    /// View options changed and are not saved yet.
    opts_dirty: bool,
    /// Tape, compare, survey or plain browsing.
    mode: Mode,
    /// Fast Review is wanted on the next tape too.
    fast_review: bool,
    /// The results grouped by clip, in result order (segment ids): what
    /// Survey's next / previous group steps through.
    groups: Vec<Vec<String>>,
    /// The keyboard preset in force.
    preset: Preset,
}

/// Something the user asked for this frame, applied after drawing.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Do {
    /// Click on result `item` with these modifiers.
    Click(usize, egui::Modifiers),
    /// Select `item` alone (right-click on an unselected result).
    Only(usize),
    /// Select and play `item`.
    Play(usize),
    /// Play the cursor.
    PlayCursor,
    /// Set (or clear) the verdict on the selection; advance after.
    Verdict(Option<Verdict>, bool),
    /// Rate the selection.
    Rate(u8),
    /// Move the cursor by this many results; extend the selection.
    Move(isize, bool),
    /// More like the cursor.
    Like,
    /// Stage the selection for chat.
    Ref(Modifier),
    /// Copy the selection as reference text.
    Copy,
    /// Open or close the stack `item` tops.
    ToggleStack(usize),
    /// Select every result.
    All,
    /// Replace the query.
    Search(String),
    /// Enter this review mode, or leave it when it is the current one.
    Mode(Kind),
    /// Turn Fast Review on or off.
    FastReview,
    /// Compare: move the candidate by this many results.
    Candidate(isize),
    /// Compare: the candidate becomes the select.
    Promote,
    /// Show this segment in the viewer, selection unchanged.
    Preview(String),
    /// Survey: focus tile `n`.
    SurveyFocus(usize),
    /// Survey: move the focus by this many tiles.
    SurveyMove(isize),
    /// Survey: drop this tile (the focused one when `None`).
    SurveyRemove(Option<usize>),
    /// Survey: the next (`1`) or previous (`-1`) group.
    Group(isize),
    /// Put the cursor in the top bar's search.
    FocusSearch,
}

/// Everything the grid and the list draw from.
pub(super) struct Paint<'a> {
    /// Palette.
    pub(super) theme: &'a Palette,
    /// The night's numbers.
    pub(super) night: &'a Night<'a>,
    /// The results.
    pub(super) items: &'a [Item],
    /// Source files by clip id.
    pub(super) paths: &'a HashMap<String, PathBuf>,
    /// Selected segment ids.
    pub(super) selected: &'a HashSet<String>,
    /// The focused segment id.
    pub(super) cursor: Option<&'a str>,
    /// Hover scrubbing on.
    pub(super) scrub: bool,
    /// The segment id under the tape's playhead.
    pub(super) playing: Option<&'a str>,
    /// The tab has the keyboard (the last click landed in it): the
    /// selection outline is the key colour, grey otherwise.
    pub(super) focused: bool,
    /// The keyboard preset, for the keys tooltips quote.
    pub(super) preset: Preset,
}

/// Draws the tab.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    if ctx.project.is_none() {
        super::no_project(ctx, ui);
        return;
    }
    let keys_free = ctx.keys_free(ui) && !state.inactive;
    track_focus(state, ui);
    sync_project(ctx, state);
    state.preset = ctx.keymap;
    follow_tape(ctx, state);
    let mut dos: Vec<Do> = Vec::new();
    // The marks leave the state while the night borrows them.
    let marks = std::mem::take(&mut state.marks);
    let drawn = body(ctx, state, ui, &marks, keys_free, &mut dos);
    state.marks = marks;
    let Some(opts_changed) = drawn else {
        return;
    };
    // Saved once the pointer is up, not every frame of a slider drag.
    state.opts_dirty |= opts_changed;
    if state.opts_dirty && !ui.input(|i| i.pointer.any_down()) {
        state.opts_dirty = false;
        save_options(ctx, state);
    }
    for d in dos {
        apply(ctx, state, d, ui.ctx());
    }
}

/// Draws the header and the results; `None` when there is nothing to show.
/// Returns whether a view option changed.
fn body(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    marks: &Marks,
    keys_free: bool,
    dos: &mut Vec<Do>,
) -> Option<bool> {
    let mut opts_changed = false;
    {
        let project = ctx.project.as_ref()?;
        let Some(index) = project.index.as_ref() else {
            widgets::empty_state(
                ui,
                ctx.theme,
                icons::FILM,
                "No shot log yet",
                "Run Index on the Home screen, then cull here.",
                |_| {},
            );
            return None;
        };
        let night = Night::new(index, &project.people, marks);
        refresh(state, &night, ctx.search);
        let query = Query::parse(ctx.search);
        opts_changed |= header(
            ui,
            ctx.theme,
            state,
            marks.counts(),
            &query,
            index.segments.len(),
            dos,
        );
        let selected: HashSet<String> = selected_ids(ctx.selection).into_iter().collect();
        if keys_free {
            keys(ui, state, dos);
        }
        let playing = match &state.mode {
            Mode::Tape(t) => t.playing(),
            Mode::Browse | Mode::Compare(_) | Mode::Survey(_) => None,
        };
        let paint = Paint {
            theme: ctx.theme,
            night: &night,
            items: &state.items,
            paths: &state.paths,
            selected: &selected,
            cursor: state.cursor.as_deref(),
            scrub: state.opts.scrub,
            playing,
            focused: !state.inactive,
            preset: state.preset,
        };
        let mut survey_cols = None;
        match &state.mode {
            Mode::Compare(c) => compare::draw(ui, &paint, ctx.thumbs, c, dos),
            Mode::Survey(s) => survey_cols = Some(survey::draw(ui, &paint, ctx.thumbs, s, dos)),
            Mode::Browse | Mode::Tape(_) => {}
        }
        if matches!(state.mode, Mode::Compare(_) | Mode::Survey(_)) {
            // The two-up and the tiles replace the results.
        } else if state.items.is_empty() {
            empty(ui, ctx.theme, &query, state.matched, dos);
        } else {
            let reveal_row = state.reveal.then(|| cursor_pos(state)).flatten();
            let out = match state.opts.view {
                View::Grid => grid::draw(
                    ui,
                    &paint,
                    ctx.thumbs,
                    state.opts.card,
                    reveal_row,
                    (state.scroll_y, state.view_h),
                ),
                View::List => list::draw(
                    ui,
                    &paint,
                    ctx.thumbs,
                    reveal_row,
                    (state.scroll_y, state.view_h),
                ),
            };
            state.scroll_y = out.scroll_y;
            state.view_h = out.view_h;
            state.columns = out.columns.max(1);
            state.reveal = false;
            for (item, response) in out.responses {
                interact(&paint, item, &response, dos);
            }
            dos.extend(out.toggles.into_iter().map(Do::ToggleStack));
            if let Some(sort) = out.sort {
                state.opts.sort = sort;
                opts_changed = true;
            }
            if let Some(pos) = out.hover_scrub
                && state.mode == Mode::Browse
                && let Some(item) = state.items.get(pos.0)
                && let Some(s) = index.segments.get(item.seg)
                && state.cursor.as_deref() == Some(s.id.as_str())
            {
                // Scrubbing the selected card drives the viewer too.
                ctx.player.pause();
                ctx.player.seek(f64::from(pos.1 - s.seg.start));
            }
        }
        if let (Some(cols), Mode::Survey(s)) = (survey_cols, &mut state.mode) {
            s.columns = cols;
        }
    }
    Some(opts_changed)
}

/// Turns the tab's keys off after a click elsewhere, on after one inside.
fn track_focus(state: &mut State, ui: &egui::Ui) {
    let area = ui.max_rect();
    let pressed = ui.input(|i| {
        i.pointer
            .any_pressed()
            .then(|| i.pointer.interact_pos())
            .flatten()
    });
    if let Some(pos) = pressed {
        state.inactive = !area.contains(pos);
    }
}

/// Reloads marks and options when another night is opened.
fn sync_project(ctx: &mut PageCtx<'_>, state: &mut State) {
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    if state.work.as_deref() == Some(project.work.as_path()) {
        return;
    }
    let (marks, warnings) = Marks::load(&project.work);
    *state = State {
        work: Some(project.work.clone()),
        marks,
        marks_rev: state.marks_rev + 1,
        opts: Options::from_table(project.settings.pages.get("review")),
        error: warnings.first().cloned(),
        ..State::default()
    };
}

/// Recomputes the results when the query, marks, index or options change.
fn refresh(state: &mut State, night: &Night<'_>, search_text: &str) {
    let index = night.index;
    let key = Key {
        search: search_text.trim().to_owned(),
        marks_rev: state.marks_rev,
        segments: index.segments.len(),
        captioned: index
            .segments
            .iter()
            .filter(|s| s.caption.is_some())
            .count(),
        people: night.people.names.len(),
        sort: state.opts.sort,
        group: state.opts.group,
        cutoff: state.opts.cutoff.to_bits(),
        expanded_rev: state.expanded_rev,
    };
    if state.key.as_ref() == Some(&key) {
        return;
    }
    if state.paths.len() != index.clips.len() {
        state.paths = index
            .clips
            .iter()
            .map(|c| (c.id.clone(), PathBuf::from(&c.path)))
            .collect();
    }
    if state.ids.len() != index.segments.len()
        || state.ids.first() != index.segments.first().map(|s| &s.id)
    {
        state.ids = index.segments.iter().map(|s| s.id.clone()).collect();
    }
    let query = Query::parse(search_text);
    let hits = search::search(&query, night);
    state.matched = hits.len();
    let mut hits: Vec<search::Hit> = hits
        .into_iter()
        .filter(|h| h.score >= state.opts.cutoff)
        .collect();
    sort_hits(&mut hits, index, state.opts.sort);
    state.groups = clip_groups(&hits, index);
    state.items = if state.opts.group {
        stack(hits, index, &state.expanded)
    } else {
        hits.into_iter()
            .map(|h| Item {
                seg: h.seg,
                score: h.score,
                reason: h.reason,
                stack: Stack::Alone,
            })
            .collect()
    };
    state.key = Some(key);
}

/// Orders hits.
fn sort_hits(hits: &mut [search::Hit], index: &Index, sort: Sort) {
    let seg = |h: &search::Hit| index.segments.get(h.seg);
    let by = |f: fn(&crate::index::Scored) -> f32| {
        move |a: &search::Hit, b: &search::Hit| {
            let fa = seg(a).map_or(0.0, f);
            let fb = seg(b).map_or(0.0, f);
            fb.total_cmp(&fa).then(a.seg.cmp(&b.seg))
        }
    };
    match sort {
        Sort::Best => {}
        Sort::Quality => hits.sort_by(by(|s| s.q)),
        Sort::Fire => hits.sort_by(by(|s| s.seg.flame)),
        Sort::Motion => hits.sort_by(by(|s| s.seg.moving)),
        Sort::Time => hits.sort_by_key(|h| h.seg),
    }
}

/// The hits' segment ids grouped by clip, groups in the order their first
/// hit comes: Survey's groups.
fn clip_groups(hits: &[search::Hit], index: &Index) -> Vec<Vec<String>> {
    let mut order: Vec<&str> = Vec::new();
    let mut groups: HashMap<&str, Vec<String>> = HashMap::new();
    for s in hits.iter().filter_map(|h| index.segments.get(h.seg)) {
        let members = groups.entry(s.clip.as_str()).or_insert_with(|| {
            order.push(s.clip.as_str());
            Vec::new()
        });
        members.push(s.id.clone());
    }
    order
        .into_iter()
        .filter_map(|clip| groups.remove(clip))
        .collect()
}

/// Groups hits by clip: each clip's first (best by the current order)
/// result stands for the rest, which follow it only when its stack is open.
fn stack(hits: Vec<search::Hit>, index: &Index, expanded: &BTreeSet<String>) -> Vec<Item> {
    let clip_of = |h: &search::Hit| {
        index
            .segments
            .get(h.seg)
            .map(|s| s.clip.clone())
            .unwrap_or_default()
    };
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<search::Hit>> = HashMap::new();
    for h in hits {
        let clip = clip_of(&h);
        if !groups.contains_key(&clip) {
            order.push(clip.clone());
        }
        groups.entry(clip).or_default().push(h);
    }
    let mut items = Vec::new();
    for clip in order {
        let Some(members) = groups.remove(&clip) else {
            continue;
        };
        let count = members.len();
        let open = expanded.contains(&clip);
        for (k, h) in members.into_iter().enumerate() {
            let stack = match (k, count) {
                (_, 1) => Stack::Alone,
                (0, _) => Stack::Top { count, open },
                (_, _) if open => Stack::Member,
                (_, _) => break,
            };
            items.push(Item {
                seg: h.seg,
                score: h.score,
                reason: h.reason,
                stack,
            });
        }
    }
    items
}

/// The selected segment ids, in selection order.
fn selected_ids(selection: &[Reference]) -> Vec<String> {
    selection
        .iter()
        .filter_map(|r| match &r.target {
            Target::Segment { id } => Some(id.clone()),
            Target::Shot { .. }
            | Target::Range { .. }
            | Target::Frame { .. }
            | Target::Bars { .. }
            | Target::Person { .. }
            | Target::Plan { .. } => None,
        })
        .collect()
}

/// The cursor's position in the results.
fn cursor_pos(state: &State) -> Option<usize> {
    let cursor = state.cursor.as_deref()?;
    position_of(state, cursor)
}

/// Where segment `id` is in the results.
fn position_of(state: &State, id: &str) -> Option<usize> {
    state
        .items
        .iter()
        .position(|it| state.ids.get(it.seg).is_some_and(|s| s == id))
}

/// The segment id of result `item`.
fn id_at(state: &State, item: usize) -> Option<String> {
    state
        .items
        .get(item)
        .and_then(|it| state.ids.get(it.seg))
        .cloned()
}

/// Height of the toolbar under the tab strip: the workspace leaves 4 px
/// under the strip, so the two make Selects' 40 px toolbar.
const TOOLBAR_H: f32 = theme::PANEL_HEADER_HEIGHT - SPACE_XS;

/// The toolbar under the tab strip, Selects' Project toolbar: filter,
/// order and smart searches on the left (or, in a review mode, where it
/// stands), the review modes and the view toggle on the right; then the
/// query's chips while searching. Returns whether a view option changed.
fn header(
    ui: &mut egui::Ui,
    theme: &Palette,
    state: &mut State,
    (picks, rejects): (usize, usize),
    query: &Query,
    total: usize,
    dos: &mut Vec<Do>,
) -> bool {
    let before = state.opts.clone();
    let width = visible_width(ui);
    let sort_label = if state.opts.sort == Sort::Best && !query.ranks() {
        "Best"
    } else {
        state.opts.sort.short()
    };
    let sort_w = ui
        .painter()
        .layout_no_wrap(
            sort_label.to_owned(),
            theme::medium(ui.ctx(), theme::BODY_SIZE),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x
        + icons::SMALL
        + SPACE_XS
        + SPACE_SM * 2.0;
    let button = theme::CONTROL_HEIGHT + 2.0;
    let left_core = button * 2.0 + sort_w + 2.0;
    let modes_full = button * 5.0 + SPACE_SM + 1.0;
    let browsing = state.mode == Mode::Browse;
    // Narrow panes fold the review modes into one menu, then drop search.
    let fold_modes = left_core + modes_full + SPACE_XS > width;
    let right_w = if fold_modes {
        button * 3.0 + SPACE_SM + 1.0
    } else {
        modes_full
    };
    let search = browsing && left_core + right_w + button + SPACE_XS <= width;
    let room = width - left_core - right_w - if search { button } else { 0.0 } - SPACE_SM;
    let error = state.error.clone();
    widgets::toolbar_row(
        ui,
        TOOLBAR_H,
        &mut (&mut *state, &mut *dos),
        |ui, (state, dos)| {
            if browsing {
                if search
                    && widgets::icon_btn(
                        ui,
                        theme,
                        icons::SEARCH,
                        "Search shots",
                        &keys_of("search", state.preset),
                    )
                    .clicked()
                {
                    dos.push(Do::FocusSearch);
                }
                filter_menu(ui, theme, state, (picks, rejects));
                sort_menu(ui, theme, state, sort_label);
                smart_menu(ui, theme, dos);
                count(ui, theme, state, query, total, room);
            } else {
                mode_status(ui, theme, state, dos);
            }
        },
        |ui, (state, dos)| {
            for (view, glyph, label) in [
                (View::List, icons::LIST, "List view"),
                (View::Grid, icons::LAYOUT_GRID, "Grid view"),
            ] {
                if widgets::icon_toggle(ui, theme, glyph, label, "", state.opts.view == view)
                    .clicked()
                {
                    state.opts.view = view;
                }
            }
            widgets::bar_divider(ui, theme);
            if fold_modes {
                modes_menu(ui, theme, state, dos);
            } else {
                let kind = state.mode.kind();
                // Right to left: the last drawn is the leftmost.
                for (k, glyph, id) in MODES.iter().rev() {
                    let label = keymap::command(id).map_or("", |c| c.label);
                    if widgets::icon_toggle(
                        ui,
                        theme,
                        *glyph,
                        label,
                        &keys_of(id, state.preset),
                        kind == *k,
                    )
                    .clicked()
                    {
                        dos.push(Do::Mode(*k));
                    }
                }
            }
            if let Some(err) = &error {
                icons::icon(
                    ui,
                    icons::TRIANGLE_ALERT,
                    icons::SMALL,
                    theme::rgb(theme.warning),
                )
                .on_hover_text(err);
            }
        },
    );
    if !query.is_empty() {
        chips(ui, theme, query, dos);
    }
    state.opts != before
}

/// How many results, muted, when there is room; the details in its
/// tooltip.
fn count(
    ui: &mut egui::Ui,
    theme: &Palette,
    state: &State,
    query: &Query,
    total: usize,
    room: f32,
) {
    if room < 24.0 {
        return;
    }
    let shown = state.items.len();
    let stacks = state
        .items
        .iter()
        .filter(|i| matches!(i.stack, Stack::Top { .. }))
        .count();
    let mut detail = vec![format!("{} shots", state.matched)];
    if !query.is_empty() {
        detail.push(format!("of {total}"));
    }
    if state.matched != shown && !state.opts.group {
        detail.push(format!("{shown} above the minimum score"));
    }
    if state.opts.group && stacks > 0 {
        detail.push(format!("{stacks} stacks"));
    }
    let text = if room >= 72.0 {
        format!("{} shots", state.matched)
    } else {
        state.matched.to_string()
    };
    ui.add_space(SPACE_XS);
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                .color(theme::rgb(theme.text_muted)),
        )
        .truncate(),
    )
    .on_hover_text(detail.join(" · "));
}

/// The filter button and its popover: the minimum score, stacks, hover
/// scrub and card size.
fn filter_menu(
    ui: &mut egui::Ui,
    theme: &Palette,
    state: &mut State,
    (picks, rejects): (usize, usize),
) {
    let id = ui.make_persistent_id("review-filter");
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let active = state.opts.cutoff > 0.0 || state.opts.group;
    let response = widgets::icon_toggle(
        ui,
        theme,
        icons::LIST_FILTER,
        "Filter and display",
        "",
        open || active,
    );
    let _ = egui::Popup::menu(&response)
        .id(id)
        .width(240.0)
        .gap(SPACE_XS)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = SPACE_SM;
            ui.spacing_mut().slider_width = 200.0;
            pad(ui, |ui| {
                widgets::menu_heading(ui, theme, "Minimum score");
            });
            pad(ui, |ui| {
                let label = format!("Hide shots scoring under {:.2}", state.opts.cutoff);
                ui.add(
                    egui::Slider::new(&mut state.opts.cutoff, 0.0..=0.95)
                        .step_by(0.05)
                        .show_value(false)
                        .trailing_fill(true),
                )
                .on_hover_text("Quality when nothing is searched")
                .accessible_name(label.clone());
                ui.label(
                    egui::RichText::new(format!("{:.2}", state.opts.cutoff))
                        .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                        .color(theme::rgb(theme.text_muted)),
                );
            });
            widgets::menu_separator(ui, theme);
            pad(ui, |ui| {
                widgets::switch(ui, theme, &mut state.opts.group, "Stack each clip's shots");
            });
            pad(ui, |ui| {
                widgets::switch(ui, theme, &mut state.opts.scrub, "Scrub on hover");
            });
            if state.opts.view == View::Grid {
                widgets::menu_separator(ui, theme);
                pad(ui, |ui| {
                    widgets::menu_heading(ui, theme, "Card size");
                });
                pad(ui, |ui| {
                    ui.add(
                        egui::Slider::new(&mut state.opts.card, CARD_MIN..=CARD_MAX)
                            .show_value(false),
                    )
                    .accessible_name("Card size");
                });
            }
            if picks + rejects > 0 {
                widgets::menu_separator(ui, theme);
                pad(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!("{picks} picked · {rejects} rejected"))
                            .color(theme::rgb(theme.text_muted)),
                    );
                });
            }
        });
}

/// A row inset by a menu item's padding.
fn pad(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_space(SPACE_SM);
        add(ui);
    });
}

/// The order, as a ghost dropdown.
fn sort_menu(ui: &mut egui::Ui, theme: &Palette, state: &mut State, label: &str) {
    let id = ui.make_persistent_id("review-sort");
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let response = widgets::dropdown_button(ui, theme, label, "Order", open);
    let _ = egui::Popup::menu(&response)
        .id(id)
        .width(200.0)
        .gap(SPACE_XS)
        .show(|ui| {
            pad(ui, |ui| {
                widgets::menu_heading(ui, theme, "Order by");
            });
            for s in Sort::ALL {
                if widgets::menu_item(ui, theme, None, s.label(), "", state.opts.sort == s)
                    .clicked()
                {
                    state.opts.sort = s;
                }
            }
        });
}

/// The saved searches.
fn smart_menu(ui: &mut egui::Ui, theme: &Palette, dos: &mut Vec<Do>) {
    let id = ui.make_persistent_id("review-smart");
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let response = widgets::icon_toggle(ui, theme, icons::SPARKLES, "Smart searches", "", open);
    let _ = egui::Popup::menu(&response)
        .id(id)
        .width(220.0)
        .gap(SPACE_XS)
        .show(|ui| {
            pad(ui, |ui| {
                widgets::menu_heading(ui, theme, "Smart searches");
            });
            for (label, q) in SMART {
                if widgets::menu_item(ui, theme, None, label, "", false)
                    .on_hover_text(*q)
                    .clicked()
                {
                    dos.push(Do::Search((*q).to_owned()));
                }
            }
        });
}

/// The review modes folded into one menu, for a narrow pane.
fn modes_menu(ui: &mut egui::Ui, theme: &Palette, state: &State, dos: &mut Vec<Do>) {
    let id = ui.make_persistent_id("review-modes");
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let kind = state.mode.kind();
    let glyph = MODES
        .iter()
        .find(|(k, _, _)| *k == kind)
        .map_or(icons::CIRCLE_PLAY, |(_, g, _)| *g);
    let response = widgets::icon_toggle(
        ui,
        theme,
        glyph,
        "Review modes",
        "",
        open || kind != Kind::Browse,
    );
    let _ = egui::Popup::menu(&response)
        .id(id)
        .width(240.0)
        .gap(SPACE_XS)
        .show(|ui| {
            pad(ui, |ui| {
                widgets::menu_heading(ui, theme, "Review");
            });
            for (k, glyph, cmd) in MODES {
                let label = keymap::command(cmd).map_or("", |c| c.label);
                if widgets::menu_item(
                    ui,
                    theme,
                    Some(glyph),
                    label,
                    &keys_of(cmd, state.preset),
                    kind == k,
                )
                .clicked()
                {
                    dos.push(Do::Mode(k));
                }
            }
        });
}

/// The toolbar's left side in a review mode: leave, where it stands, and
/// the mode's own controls (their keys in the tooltips).
fn mode_status(ui: &mut egui::Ui, theme: &Palette, state: &State, dos: &mut Vec<Do>) {
    let p = state.preset;
    let name = match &state.mode {
        Mode::Browse => return,
        Mode::Tape(_) => "Tape",
        Mode::Compare(_) => "Compare",
        Mode::Survey(_) => "Survey",
    };
    if widgets::icon_btn(
        ui,
        theme,
        icons::X,
        &format!("Leave {}", name.to_lowercase()),
        &keys_of("review-exit", p),
    )
    .clicked()
    {
        dos.push(Do::Mode(Kind::Browse));
    }
    match &state.mode {
        Mode::Browse => {}
        Mode::Tape(t) => {
            if widgets::icon_toggle(
                ui,
                theme,
                icons::ZAP,
                keymap::command("review-fast").map_or("Fast Review", |c| c.label),
                &keys_of("review-fast", p),
                t.fast,
            )
            .clicked()
            {
                dos.push(Do::FastReview);
            }
        }
        Mode::Compare(_) => {
            for (glyph, label, step) in [
                (icons::CHEVRON_LEFT, "Previous candidate", -1),
                (icons::CHEVRON_RIGHT, "Next candidate", 1),
            ] {
                if widgets::icon_btn(
                    ui,
                    theme,
                    glyph,
                    label,
                    &keys_of("review-compare-candidate", p),
                )
                .clicked()
                {
                    dos.push(Do::Candidate(step));
                }
            }
            if widgets::icon_btn(
                ui,
                theme,
                icons::ARROW_UP,
                "Make the candidate the select",
                &keys_of("review-compare-promote", p),
            )
            .clicked()
            {
                dos.push(Do::Promote);
            }
        }
        Mode::Survey(_) => {
            for (glyph, label, id, step) in [
                (
                    icons::CHEVRON_LEFT,
                    "Previous group",
                    "review-prev-group",
                    -1,
                ),
                (icons::CHEVRON_RIGHT, "Next group", "review-next-group", 1),
            ] {
                if widgets::icon_btn(ui, theme, glyph, label, &keys_of(id, p)).clicked() {
                    dos.push(Do::Group(step));
                }
            }
        }
    }
    let status = match &state.mode {
        Mode::Browse => String::new(),
        Mode::Tape(t) => format!("Tape {} of {}", t.last.map_or(0, |k| k + 1), t.ids.len()),
        // The lit Compare toggle says which mode this is; its buttons fill
        // the row.
        Mode::Compare(_) => String::new(),
        Mode::Survey(s) => match s.group {
            Some(g) => format!("Group {} of {}", g + 1, state.groups.len()),
            None => format!("{} shots", s.ids.len()),
        },
    };
    ui.add_space(SPACE_XS);
    ui.add(
        egui::Label::new(
            egui::RichText::new(status)
                .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                .color(theme::rgb(theme.text)),
        )
        .truncate(),
    );
}

/// The review modes the toolbar offers: (mode, icon, command id).
const MODES: [(Kind, char, &str); 3] = [
    (Kind::Tape, icons::CIRCLE_PLAY, "review-tape"),
    (Kind::Compare, icons::COLUMNS, "review-compare"),
    (Kind::Survey, icons::GRID, "review-survey"),
];

/// Just the keys of a command under `preset`.
fn keys_of(id: &str, preset: Preset) -> String {
    keymap::command(id).map_or_else(String::new, |c| c.hint(preset))
}

/// Saved searches: (label, query).
const SMART: &[(&str, &str)] = &[
    ("Best fire", "fire>0.5 -dark -soft -whip"),
    ("LED", "led -dark -whip"),
    ("Performing, sharp", "moment:performing focus>0.9 -blocked"),
    ("Crowd", "crowd"),
    ("DJ booth", "dj"),
    ("Picks", "pick"),
    ("Unreviewed", "unreviewed"),
    ("Rejected", "reject"),
    ("Flagged", "-clean"),
    ("Clean", "clean"),
    ("Steady", "move:static"),
    ("Needs captions", "uncaptioned"),
];

/// The query's tokens as removable chips: 24 px, 6 px corners, ink fill,
/// a close icon; a negated token tinted with the danger colour.
fn chips(ui: &mut egui::Ui, theme: &Palette, query: &Query, dos: &mut Vec<Do>) {
    ui.add_space(SPACE_XS);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACE_XS, SPACE_XS);
        let font = theme::medium(ui.ctx(), theme::BODY_SIZE);
        for (i, c) in query.clauses.iter().enumerate() {
            let galley =
                ui.painter()
                    .layout_no_wrap(c.label(), font.clone(), theme::rgb(theme.text));
            let size = egui::vec2(
                galley.size().x + SPACE_SM + icons::SMALL + SPACE_XS * 2.0,
                theme::COMPACT_HEIGHT,
            );
            let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
            let label = format!("Remove {}", c.label());
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
            let fill = match (c.negate, resp.hovered()) {
                (true, false) => theme::translucent(theme.danger, 38),
                (true, true) => theme::translucent(theme.danger, 64),
                (false, false) => theme::ink(theme, theme::INK_08),
                (false, true) => theme::ink(theme, theme::INK_12),
            };
            ui.painter()
                .rect_filled(rect, egui::CornerRadius::same(theme::RADIUS), fill);
            let ink = theme::rgb(theme.text);
            ui.painter().galley(
                egui::pos2(
                    rect.left() + SPACE_SM,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                ink,
            );
            let slot = egui::Rect::from_center_size(
                egui::pos2(
                    rect.right() - SPACE_XS - icons::SMALL / 2.0,
                    rect.center().y,
                ),
                egui::Vec2::splat(icons::SMALL - 2.0),
            );
            icons::paint(
                ui.painter(),
                slot,
                icons::X,
                icons::SMALL - 2.0,
                theme::rgb(theme.text_muted),
            );
            if resp.on_hover_text("Remove").clicked() {
                dos.push(Do::Search(query.without(i)));
            }
        }
    });
}

/// No results: say why and offer a way out.
fn empty(ui: &mut egui::Ui, theme: &Palette, query: &Query, matched: usize, dos: &mut Vec<Do>) {
    let (title, hint) = if matched > 0 {
        (
            format!("{matched} shots under the minimum score"),
            "Lower it in the filter menu to see them.",
        )
    } else if query.is_empty() {
        (
            "The shot log is empty".to_owned(),
            "Index the night on the Home screen.",
        )
    } else {
        (
            "No shots match".to_owned(),
            "Try fewer words, or a smart search.",
        )
    };
    let mut clear = false;
    widgets::empty_state(ui, theme, icons::SEARCH, &title, hint, |ui| {
        if !query.is_empty() && widgets::outline_button(ui, theme, "Clear search").clicked() {
            clear = true;
        }
    });
    if clear {
        dos.push(Do::Search(String::new()));
    }
}

/// Removes the first press of `chord` (an arrow, which is not a row of its
/// own) from this frame's input, matched exactly ([`keymap::matches`]).
/// Held-key repeats count when `repeat`.
fn take_chord(ui: &egui::Ui, chord: egui::KeyboardShortcut, repeat: bool) -> bool {
    ui.input_mut(|i| {
        let at = i.events.iter().position(|e| {
            matches!(
                e,
                egui::Event::Key { key, pressed: true, repeat: r, modifiers, .. }
                    if (repeat || !*r) && keymap::matches(chord, *key, *modifiers)
            )
        });
        at.map(|k| i.events.remove(k)).is_some()
    })
}

/// Whether the command `id` was pressed under `preset`; the press is
/// removed so nothing drawn later reads it too ([`keymap::consume`]).
fn take(ui: &egui::Ui, id: &str, preset: Preset) -> bool {
    ui.input_mut(|i| keymap::consume(i, id, preset))
}

/// Reads the review modes' keys; returns whether the arrows are taken.
fn mode_keys(ui: &egui::Ui, state: &State, dos: &mut Vec<Do>) -> bool {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let p = state.preset;
    let kind = state.mode.kind();
    if kind != Kind::Browse && take(ui, "review-exit", p) {
        dos.push(Do::Mode(Kind::Browse));
    }
    for (k, _, id) in MODES {
        if take(ui, id, p) {
            dos.push(Do::Mode(k));
        }
    }
    if take(ui, "review-fast", p) {
        dos.push(Do::FastReview);
    }
    let arrow = |key| take_chord(ui, KeyboardShortcut::new(Modifiers::NONE, key), true);
    match &state.mode {
        Mode::Browse | Mode::Tape(_) => false,
        Mode::Compare(_) => {
            if take(ui, "review-compare-promote", p) {
                dos.push(Do::Promote);
            }
            for (key, step) in [(Key::ArrowLeft, -1), (Key::ArrowRight, 1)] {
                if arrow(key) {
                    dos.push(Do::Candidate(step));
                }
            }
            true
        }
        Mode::Survey(s) => {
            if take(ui, "review-survey-remove", p) {
                dos.push(Do::SurveyRemove(None));
            }
            if take(ui, "review-next-group", p) {
                dos.push(Do::Group(1));
            }
            if take(ui, "review-prev-group", p) {
                dos.push(Do::Group(-1));
            }
            let cols = s.columns.max(1) as isize;
            for (key, step) in [
                (Key::ArrowLeft, -1),
                (Key::ArrowRight, 1),
                (Key::ArrowUp, -cols),
                (Key::ArrowDown, cols),
            ] {
                if arrow(key) {
                    dos.push(Do::SurveyMove(step));
                }
            }
            true
        }
    }
}

/// Reads the tab's keys.
fn keys(ui: &egui::Ui, state: &State, dos: &mut Vec<Do>) {
    use egui::Key;
    // The modes' own keys first: Survey's X drops a tile rather than rejects.
    let arrows_taken = mode_keys(ui, state, dos);
    let plain = |key: Key| ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key));
    if plain(Key::P) {
        dos.push(Do::Verdict(Some(Verdict::Pick), true));
    }
    if plain(Key::X) {
        dos.push(Do::Verdict(Some(Verdict::Reject), true));
    }
    if plain(Key::U) {
        dos.push(Do::Verdict(None, true));
    }
    for (key, stars) in [
        (Key::Num0, 0),
        (Key::Num1, 1),
        (Key::Num2, 2),
        (Key::Num3, 3),
        (Key::Num4, 4),
        (Key::Num5, 5),
    ] {
        if plain(key) {
            dos.push(Do::Rate(stars));
        }
    }
    if plain(Key::Enter) {
        dos.push(Do::PlayCursor);
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, Key::F)) {
        dos.push(Do::Like);
    }
    if arrows_taken {
        return;
    }
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::A)) {
        dos.push(Do::All);
    }
    let cols = match state.opts.view {
        View::Grid => state.columns.max(1) as isize,
        View::List => 1,
    };
    for (key, step) in [
        (Key::ArrowLeft, -1),
        (Key::ArrowRight, 1),
        (Key::ArrowUp, -cols),
        (Key::ArrowDown, cols),
    ] {
        if state.opts.view == View::List && matches!(key, Key::ArrowLeft | Key::ArrowRight) {
            // In a list, left/right stay the viewer's frame steps.
            continue;
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
            dos.push(Do::Move(step, false));
        } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, key)) {
            dos.push(Do::Move(step, true));
        }
    }
}

/// Turns a result's response into intents, and gives it its context menu.
fn interact(paint: &Paint<'_>, item: usize, response: &egui::Response, dos: &mut Vec<Do>) {
    let Some(it) = paint.items.get(item) else {
        return;
    };
    let Some(seg) = paint.night.index.segments.get(it.seg) else {
        return;
    };
    let selected = paint.selected.contains(&seg.id);
    if response.double_clicked() {
        dos.push(Do::Play(item));
    } else if response.clicked() {
        let mods = response.ctx.input(|i| i.modifiers);
        dos.push(Do::Click(item, mods));
    }
    if response.secondary_clicked() && !selected {
        dos.push(Do::Only(item));
    }
    let id = seg.id.clone();
    response.context_menu(|ui| {
        ui.set_min_width(190.0);
        let n = paint.selected.len().max(1);
        let what = if n > 1 {
            format!("{n} shots")
        } else {
            id.clone()
        };
        ui.label(egui::RichText::new(what).size(11.0).weak());
        fn item_btn(ui: &mut egui::Ui, dos: &mut Vec<Do>, label: &str, key: &str, d: Do) {
            let button = egui::Button::new(label).shortcut_text(key);
            if ui.add(button).clicked() {
                dos.push(d);
                ui.close();
            }
        }
        item_btn(
            ui,
            dos,
            "Pick",
            "P",
            Do::Verdict(Some(Verdict::Pick), false),
        );
        item_btn(
            ui,
            dos,
            "Reject",
            "X",
            Do::Verdict(Some(Verdict::Reject), false),
        );
        item_btn(ui, dos, "Clear mark", "U", Do::Verdict(None, false));
        ui.menu_button("Rating", |ui| {
            for r in (0..=5u8).rev() {
                let label = match r {
                    0 => "No rating".to_owned(),
                    1 => "1 star".to_owned(),
                    _ => format!("{r} stars"),
                };
                if ui.button(label).clicked() {
                    dos.push(Do::Rate(r));
                    ui.close();
                }
            }
        });
        ui.separator();
        item_btn(ui, dos, "Play", "Enter", Do::Play(item));
        item_btn(ui, dos, "More like this", "Shift+F", Do::Like);
        if it.stack != Stack::Alone {
            let label = match it.stack {
                Stack::Top { open: true, .. } | Stack::Member => "Collapse stack",
                Stack::Top { open: false, .. } | Stack::Alone => "Expand stack",
            };
            item_btn(ui, dos, label, "", Do::ToggleStack(item));
        }
        ui.separator();
        ui.menu_button("Reference in chat", |ui| {
            for m in Modifier::ALL {
                let label = if m.prefix().is_empty() {
                    m.label().to_owned()
                } else {
                    format!("{}  {}", m.prefix(), m.label())
                };
                if ui.button(label).clicked() {
                    dos.push(Do::Ref(m));
                    ui.close();
                }
            }
        });
        item_btn(ui, dos, "Copy reference", "", Do::Copy);
    });
}

/// Applies one intent.
fn apply(ctx: &mut PageCtx<'_>, state: &mut State, d: Do, egui_ctx: &egui::Context) {
    match d {
        Do::Click(item, mods) => click(ctx, state, item, mods),
        Do::Only(item) => click(ctx, state, item, egui::Modifiers::NONE),
        Do::Play(item) => {
            click(ctx, state, item, egui::Modifiers::NONE);
            ctx.player.play();
        }
        Do::PlayCursor => {
            if let Some(pos) = cursor_pos(state) {
                open(ctx, state, pos);
                ctx.player.play();
            }
        }
        Do::Verdict(v, advance) => {
            let targets = targets(ctx, state);
            for id in &targets {
                state.marks.set_verdict(id, v);
            }
            marks_changed(ctx, state);
            if advance && targets.len() == 1 {
                match state.mode.kind() {
                    Kind::Browse => step(ctx, state, 1, false),
                    Kind::Tape => tape_step(ctx, state, 1),
                    Kind::Compare => move_candidate(ctx, state, 1),
                    Kind::Survey => {
                        if let Mode::Survey(s) = &mut state.mode {
                            s.step(1);
                        }
                        show_survey(ctx, state);
                    }
                }
            }
        }
        Do::Rate(r) => {
            for id in targets(ctx, state) {
                state.marks.set_rating(&id, r);
            }
            marks_changed(ctx, state);
        }
        Do::Move(delta, extend) => step(ctx, state, delta, extend),
        Do::Like => {
            if let Some(id) = state
                .cursor
                .clone()
                .or_else(|| selected_ids(ctx.selection).into_iter().next())
            {
                *ctx.search = format!("like:{id}");
                state.reveal = true;
            }
        }
        Do::Ref(m) => {
            for id in targets(ctx, state) {
                ctx.actions.push(Action::AddRef(
                    Reference::new(Target::Segment { id }).with(m),
                ));
            }
        }
        Do::Copy => {
            let text: Vec<String> = targets(ctx, state)
                .into_iter()
                .map(|id| Reference::new(Target::Segment { id }).to_string())
                .collect();
            egui_ctx.copy_text(text.join(" "));
        }
        Do::ToggleStack(item) => {
            if let Some(it) = state.items.get(item)
                && let Some(clip) = clip_of(ctx, it.seg)
            {
                if !state.expanded.remove(&clip) {
                    state.expanded.insert(clip);
                }
                state.expanded_rev += 1;
            }
        }
        Do::All => {
            let ids: Vec<String> = (0..state.items.len())
                .filter_map(|i| id_at(state, i))
                .collect();
            set_selection(ctx, ids);
        }
        Do::Search(text) => *ctx.search = text,
        Do::FocusSearch => ctx.actions.push(Action::FocusSearch),
        Do::Mode(kind) => {
            let current = state.mode.kind();
            leave(ctx, state);
            if kind != current {
                match kind {
                    Kind::Browse => {}
                    Kind::Tape => enter_tape(ctx, state),
                    Kind::Compare => enter_compare(ctx, state),
                    Kind::Survey => enter_survey(ctx, state),
                }
            }
        }
        Do::FastReview => {
            state.fast_review = !state.fast_review;
            if let Mode::Tape(t) = &mut state.mode {
                t.fast = state.fast_review;
                let rate = t.last.map_or(1.0, |k| t.rate_for(k));
                ctx.player.set_rate(rate);
            }
        }
        Do::Candidate(delta) => move_candidate(ctx, state, delta),
        Do::Promote => {
            let order = result_ids(state);
            if let Mode::Compare(c) = &mut state.mode {
                let select = c.candidate.clone();
                let next = compare::next_candidate(&order, &select, &select, 0);
                c.candidate = next.unwrap_or_else(|| c.select.clone());
                c.select = select;
            }
            show_compare(ctx, state);
        }
        Do::Preview(id) => open_id(ctx, state, &id),
        Do::SurveyFocus(at) => {
            if let Mode::Survey(s) = &mut state.mode {
                s.focus = at.min(s.ids.len().saturating_sub(1));
            }
            show_survey(ctx, state);
        }
        Do::SurveyMove(delta) => {
            if let Mode::Survey(s) = &mut state.mode {
                s.step(delta);
            }
            show_survey(ctx, state);
        }
        Do::SurveyRemove(at) => {
            if let Mode::Survey(s) = &mut state.mode {
                let at = at.unwrap_or(s.focus);
                s.remove(at);
            }
            show_survey(ctx, state);
        }
        Do::Group(dir) => {
            if let Mode::Survey(s) = &state.mode {
                let from = s.group.or_else(|| {
                    s.ids
                        .first()
                        .and_then(|id| survey::group_of(&state.groups, id))
                });
                if let Some(g) = survey::next_group(&state.groups, from, dir)
                    && let Some(members) = state.groups.get(g)
                {
                    let ids = members.iter().take(survey::MAX).cloned().collect();
                    state.mode = Mode::Survey(survey::Survey::new(ids, 0, Some(g)));
                    show_survey(ctx, state);
                }
            }
        }
    }
}

/// The segment with this id.
pub(super) fn segment<'a>(night: &Night<'a>, id: &str) -> Option<&'a crate::index::Scored> {
    night.index.segments.iter().find(|s| s.id == id)
}

/// The results' segment ids, in order.
fn result_ids(state: &State) -> Vec<String> {
    (0..state.items.len())
        .filter_map(|i| id_at(state, i))
        .collect()
}

/// Makes `id` the focused, selected shot and scrolls it into view.
fn focus_on(ctx: &mut PageCtx<'_>, state: &mut State, id: String) {
    set_selection(ctx, vec![id.clone()]);
    state.anchor = Some(id.clone());
    state.cursor = Some(id);
    state.reveal = true;
}

/// Keeps the selection on the shot under the tape's playhead; drops back
/// to browsing when the viewer was given something else.
fn follow_tape(ctx: &mut PageCtx<'_>, state: &mut State) {
    let Mode::Tape(t) = &mut state.mode else {
        return;
    };
    match tape::follow(t, ctx.player) {
        tape::Follow::Gone => state.mode = Mode::Browse,
        tape::Follow::Same => {}
        tape::Follow::Now(id) => focus_on(ctx, state, id),
    }
}

/// Loads the results into the viewer as a tape and plays it from the
/// focused shot.
fn enter_tape(ctx: &mut PageCtx<'_>, state: &mut State) {
    let Some(index) = ctx.project.as_ref().and_then(|p| p.index.as_ref()) else {
        return;
    };
    let tape = tape::Tape::new(
        index,
        state.items.iter().map(|it| it.seg),
        &state.paths,
        state.fast_review,
    );
    if tape.ids.is_empty() {
        return;
    }
    let from = state
        .cursor
        .as_deref()
        .and_then(|c| tape.position_of(c))
        .unwrap_or(0);
    ctx.player.set_rate(1.0);
    ctx.player.open_sequence(tape.shots.clone(), None);
    ctx.player.seek(tape.start_of(from));
    ctx.player.play();
    state.mode = Mode::Tape(tape);
}

/// Moves the tape's playhead `delta` shots on, to the start of that shot.
fn tape_step(ctx: &mut PageCtx<'_>, state: &mut State, delta: isize) {
    let Mode::Tape(t) = &mut state.mode else {
        return;
    };
    let Some(last) = t.ids.len().checked_sub(1) else {
        return;
    };
    let next = t.last.unwrap_or(0).saturating_add_signed(delta).min(last);
    ctx.player.seek(t.start_of(next));
    ctx.player.set_rate(t.rate_for(next));
    t.last = Some(next);
    if let Some(id) = t.ids.get(next).cloned() {
        focus_on(ctx, state, id);
    }
}

/// Starts comparing: the first selected shot (or the focused one) is the
/// select, the second selected (or the next result) the candidate.
fn enter_compare(ctx: &mut PageCtx<'_>, state: &mut State) {
    let order = result_ids(state);
    let selected = selected_ids(ctx.selection);
    let Some(select) = selected
        .first()
        .cloned()
        .or_else(|| state.cursor.clone())
        .or_else(|| order.first().cloned())
    else {
        return;
    };
    let candidate = selected
        .get(1)
        .filter(|c| **c != select)
        .cloned()
        .or_else(|| compare::next_candidate(&order, &select, &select, 0));
    let Some(candidate) = candidate else {
        return;
    };
    state.mode = Mode::Compare(compare::Compare { select, candidate });
    show_compare(ctx, state);
}

/// Steps the candidate `delta` results on.
fn move_candidate(ctx: &mut PageCtx<'_>, state: &mut State, delta: isize) {
    let order = result_ids(state);
    if let Mode::Compare(c) = &mut state.mode
        && let Some(next) = compare::next_candidate(&order, &c.select, &c.candidate, delta)
    {
        c.candidate = next;
    }
    show_compare(ctx, state);
}

/// Selects the select and the candidate (so `R` mentions both) and shows
/// the candidate in the viewer.
fn show_compare(ctx: &mut PageCtx<'_>, state: &mut State) {
    let Mode::Compare(c) = &state.mode else {
        return;
    };
    let (select, candidate) = (c.select.clone(), c.candidate.clone());
    set_selection(ctx, vec![select, candidate.clone()]);
    state.cursor = Some(candidate.clone());
    state.reveal = true;
    open_id(ctx, state, &candidate);
}

/// Starts a survey of the selection, or, with fewer than two selected, of
/// the focused shot's group.
fn enter_survey(ctx: &mut PageCtx<'_>, state: &mut State) {
    let selected = selected_ids(ctx.selection);
    let survey = if selected.len() >= 2 {
        let focus = state
            .cursor
            .as_deref()
            .and_then(|c| selected.iter().position(|x| x == c))
            .unwrap_or(0);
        let group = selected
            .first()
            .and_then(|id| survey::group_of(&state.groups, id));
        survey::Survey::new(selected, focus, group)
    } else {
        let from = state
            .cursor
            .clone()
            .or_else(|| selected.first().cloned())
            .and_then(|id| survey::group_of(&state.groups, &id));
        let group = from
            .filter(|g| state.groups.get(*g).is_some_and(|m| m.len() >= 2))
            .or_else(|| survey::next_group(&state.groups, from, 1))
            .or_else(|| survey::next_group(&state.groups, None, 1));
        let Some((g, members)) = group.and_then(|g| state.groups.get(g).map(|m| (g, m))) else {
            return;
        };
        let ids: Vec<String> = members.iter().take(survey::MAX).cloned().collect();
        let focus = state
            .cursor
            .as_deref()
            .and_then(|c| ids.iter().position(|x| x == c))
            .unwrap_or(0);
        survey::Survey::new(ids, focus, Some(g))
    };
    if survey.ids.len() < 2 {
        return;
    }
    state.mode = Mode::Survey(survey);
    show_survey(ctx, state);
}

/// Selects what is left of the survey and shows the focused tile.
fn show_survey(ctx: &mut PageCtx<'_>, state: &mut State) {
    let Mode::Survey(s) = &state.mode else {
        return;
    };
    let ids = s.ids.clone();
    let focused = s.focused().map(str::to_owned);
    set_selection(ctx, ids);
    if let Some(id) = focused {
        state.cursor = Some(id.clone());
        state.reveal = true;
        open_id(ctx, state, &id);
    }
}

/// Back to browsing: the tape stops (its shot stays in the viewer as a
/// clip), Compare leaves the select selected, Survey what is left.
fn leave(ctx: &mut PageCtx<'_>, state: &mut State) {
    match std::mem::take(&mut state.mode) {
        Mode::Browse | Mode::Survey(_) => {}
        Mode::Tape(t) => {
            if t.is_loaded(ctx.player) {
                ctx.player.pause();
                ctx.player.set_rate(1.0);
                if let Some(id) = state.cursor.clone() {
                    open_id(ctx, state, &id);
                }
            }
        }
        Mode::Compare(c) => focus_on(ctx, state, c.select),
    }
    state.reveal = true;
}

/// Who a verdict, rating or reference applies to: in Compare the
/// candidate, in Survey the focused tile; otherwise the selection, or the
/// cursor when nothing is selected.
fn targets(ctx: &PageCtx<'_>, state: &State) -> Vec<String> {
    match &state.mode {
        Mode::Compare(c) => return vec![c.candidate.clone()],
        Mode::Survey(s) => return s.focused().map(str::to_owned).into_iter().collect(),
        Mode::Browse | Mode::Tape(_) => {}
    }
    let ids = selected_ids(ctx.selection);
    if ids.is_empty() {
        state.cursor.iter().cloned().collect()
    } else {
        ids
    }
}

/// Saves the marks and invalidates the results.
fn marks_changed(ctx: &PageCtx<'_>, state: &mut State) {
    state.marks_rev += 1;
    if let Some(project) = ctx.project.as_ref() {
        state.error = state
            .marks
            .save(&project.work)
            .err()
            .map(|e| format!("saving review marks: {e:#}"));
    }
}

/// The clip of segment `seg`.
fn clip_of(ctx: &PageCtx<'_>, seg: usize) -> Option<String> {
    ctx.project
        .as_ref()?
        .index
        .as_ref()?
        .segments
        .get(seg)
        .map(|s| s.clip.clone())
}

/// Replaces the app-wide selection with these segments.
fn set_selection(ctx: &mut PageCtx<'_>, ids: Vec<String>) {
    *ctx.selection = ids
        .into_iter()
        .map(|id| Reference::new(Target::Segment { id }))
        .collect();
}

/// A click on result `item`: select it alone, toggle it (Ctrl), or select
/// the range from the anchor (Shift). The viewer loads it.
fn click(ctx: &mut PageCtx<'_>, state: &mut State, item: usize, mods: egui::Modifiers) {
    let Some(id) = id_at(state, item) else {
        return;
    };
    let mut ids = selected_ids(ctx.selection);
    if mods.shift {
        let from = state
            .anchor
            .as_deref()
            .and_then(|a| position_of(state, a))
            .unwrap_or(item);
        let (lo, hi) = (from.min(item), from.max(item));
        let range: Vec<String> = (lo..=hi).filter_map(|i| id_at(state, i)).collect();
        if mods.command {
            for r in range {
                if !ids.contains(&r) {
                    ids.push(r);
                }
            }
        } else {
            ids = range;
        }
    } else if mods.command {
        if let Some(k) = ids.iter().position(|x| *x == id) {
            ids.remove(k);
        } else {
            ids.push(id.clone());
        }
        state.anchor = Some(id.clone());
    } else {
        ids = vec![id.clone()];
        state.anchor = Some(id.clone());
    }
    set_selection(ctx, ids);
    state.cursor = Some(id);
    open(ctx, state, item);
}

/// Moves the cursor by `delta` results, selecting (or extending to) it.
fn step(ctx: &mut PageCtx<'_>, state: &mut State, delta: isize, extend: bool) {
    let n = state.items.len();
    if n == 0 {
        return;
    }
    let next = match cursor_pos(state) {
        Some(pos) => pos.saturating_add_signed(delta).min(n - 1),
        None if delta < 0 => n - 1,
        None => 0,
    };
    let mods = if extend {
        egui::Modifiers::SHIFT
    } else {
        egui::Modifiers::NONE
    };
    if extend && state.anchor.is_none() {
        state.anchor.clone_from(&state.cursor);
    }
    click(ctx, state, next, mods);
    state.reveal = true;
}

/// Loads result `item` into the viewer; on the tape, moves the playhead
/// to it instead.
fn open(ctx: &mut PageCtx<'_>, state: &mut State, item: usize) {
    let Some(it) = state.items.get(item) else {
        return;
    };
    let seg = it.seg;
    if let Mode::Tape(t) = &mut state.mode
        && let Some(id) = state.ids.get(seg)
        && let Some(k) = t.position_of(id)
        && t.is_loaded(ctx.player)
    {
        ctx.player.seek(t.start_of(k));
        ctx.player.set_rate(t.rate_for(k));
        t.last = Some(k);
        return;
    }
    open_seg(ctx, state, seg);
}

/// Loads segment `id` into the viewer.
fn open_id(ctx: &mut PageCtx<'_>, state: &State, id: &str) {
    if let Some(seg) = state.ids.iter().position(|x| x == id) {
        open_seg(ctx, state, seg);
    }
}

/// Loads segment `seg` (a position in `Index::segments`) into the viewer.
fn open_seg(ctx: &mut PageCtx<'_>, state: &State, seg: usize) {
    let Some(seg) = ctx
        .project
        .as_ref()
        .and_then(|p| p.index.as_ref())
        .and_then(|i| i.segments.get(seg))
    else {
        return;
    };
    if let Some(path) = state.paths.get(&seg.clip) {
        ctx.player.open_clip(
            path,
            f64::from(seg.seg.start),
            f64::from(seg.seg.end),
            seg.seg.cx,
        );
    }
}

/// Writes the view options to `ave.toml`.
fn save_options(ctx: &mut PageCtx<'_>, state: &mut State) {
    let Some(project) = ctx.project.as_mut() else {
        return;
    };
    project
        .settings
        .pages
        .insert("review".to_owned(), state.opts.to_table());
    if let Err(e) = project.save_settings() {
        state.error = Some(format!("saving view options: {e:#}"));
    }
}

/// The path of a clip's source file, for the painters.
pub(super) fn source<'a>(paths: &'a HashMap<String, PathBuf>, clip: &str) -> Option<&'a Path> {
    paths.get(clip).map(PathBuf::as_path)
}

/// What the grid and the list hand back.
pub(super) struct Drawn {
    /// Each drawn result's response, by result position.
    pub(super) responses: Vec<(usize, egui::Response)>,
    /// The result being hover-scrubbed and the source time under the pointer.
    pub(super) hover_scrub: Option<(usize, f32)>,
    /// Scroll offset after drawing.
    pub(super) scroll_y: f32,
    /// Height of the visible area.
    pub(super) view_h: f32,
    /// Results per row.
    pub(super) columns: usize,
    /// Stack badges clicked (result positions).
    pub(super) toggles: Vec<usize>,
    /// A column header asked for this order.
    pub(super) sort: Option<Sort>,
}

/// The scroll offset that brings rows `top..bottom` (points) into a view of
/// height `view_h` currently scrolled to `y`, or `None` when already in view.
pub(super) fn reveal_offset(top: f32, bottom: f32, (y, view_h): (f32, f32)) -> Option<f32> {
    if view_h <= 0.0 {
        return Some(top);
    }
    if top < y {
        Some(top)
    } else if bottom > y + view_h {
        Some(bottom - view_h)
    } else {
        None
    }
}

/// One line of text from `pos` (left-centre), cut with an ellipsis at
/// `max_w`. Returns its width.
pub(super) fn text_line(
    painter: &egui::Painter,
    pos: egui::Pos2,
    text: &str,
    size: f32,
    color: egui::Color32,
    max_w: f32,
) -> f32 {
    text_in(painter, pos, text, theme::font(size), color, max_w)
}

/// [`text_line`] in a given font (Medium for names, tabular for numbers).
pub(super) fn text_in(
    painter: &egui::Painter,
    pos: egui::Pos2,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
    max_w: f32,
) -> f32 {
    if max_w <= 4.0 {
        return 0.0;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max_w);
    let galley = painter.layout_job(job);
    let w = galley.size().x;
    let top = pos.y - galley.size().y / 2.0;
    painter.galley(egui::pos2(pos.x, top), galley, color);
    w
}

/// The colours of the metric bars: fire, LED, moving light, focus.
pub(super) fn metric_colors(theme: &Palette) -> [egui::Color32; 4] {
    [
        theme::rgb(theme.warning),
        theme::rgb(theme.series(5, 8)),
        theme::rgb(theme.success),
        theme::rgb(theme.text_muted),
    ]
}

/// A segment's four bar values, 0..1: fire, LED, moving light, focus.
pub(super) fn bar_values(night: &Night<'_>, s: &crate::index::Scored) -> [f32; 4] {
    [
        night.metric(s, search::Metric::Fire),
        night.metric(s, search::Metric::Led),
        night.metric(s, search::Metric::Move),
        (night.metric(s, search::Metric::Focus) / 1.4).clamp(0.0, 1.0),
    ]
}

/// The hover card: caption, why it matched, the numbers.
pub(super) fn tooltip(ui: &mut egui::Ui, paint: &Paint<'_>, item: &Item) {
    let Some(s) = paint.night.index.segments.get(item.seg) else {
        return;
    };
    ui.set_max_width(320.0);
    ui.label(egui::RichText::new(&s.id).strong());
    if let Some(c) = &s.caption {
        ui.label(&c.summary);
        let who: Vec<String> = c
            .people
            .iter()
            .map(|p| {
                if p.prop.is_empty() || p.prop == "none" {
                    p.appearance.clone()
                } else {
                    format!("{} [{}]", p.appearance, p.prop)
                }
            })
            .collect();
        if !who.is_empty() {
            ui.label(egui::RichText::new(format!("who: {}", who.join("; "))).weak());
        }
        ui.label(
            egui::RichText::new(format!(
                "{} · {} · {}",
                c.effective_moment(),
                c.framing,
                c.setting
            ))
            .weak(),
        );
    }
    let person = paint.night.person(s);
    if person != s.clip {
        ui.label(egui::RichText::new(format!("person: {person}")).weak());
    }
    if !item.reason.is_empty() {
        ui.label(
            egui::RichText::new(format!("match {:.2}: {}", item.score, item.reason))
                .color(theme::rgb(paint.theme.accent)),
        );
    }
    let v = bar_values(paint.night, s);
    measures(ui, paint.theme, v);
    ui.label(
        egui::RichText::new(format!(
            "{:.1}–{:.1}s · {} · fire {:.2} · led {:.2} · move {:.2} · focus {:.2} · q {:.2}",
            s.seg.start,
            s.seg.end,
            s.seg.camera.label(),
            v[0],
            v[1],
            v[2],
            paint.night.metric(s, search::Metric::Focus),
            s.q
        ))
        .monospace()
        .size(11.0),
    );
    if !s.flags.is_empty() {
        ui.label(
            egui::RichText::new(format!("flags: {}", s.flags.join(" ")))
                .color(theme::rgb(paint.theme.warning)),
        );
    }
}

/// The four measures as small labelled bars (fire, LED, moving light,
/// focus): the hover card's at-a-glance row, kept off the cards so they
/// stay calm.
fn measures(ui: &mut egui::Ui, theme: &Palette, values: [f32; 4]) {
    let colors = metric_colors(theme);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_SM;
        for ((name, v), c) in ["fire", "led", "move", "focus"]
            .into_iter()
            .zip(values)
            .zip(colors)
        {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 24.0), egui::Sense::hover());
            let painter = ui.painter();
            painter.text(
                rect.left_top(),
                egui::Align2::LEFT_TOP,
                name,
                theme::font(theme::SMALL_SIZE),
                theme::rgb(theme.text_muted),
            );
            let track = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.bottom() - 4.0),
                egui::vec2(rect.width(), 3.0),
            );
            painter.rect_filled(
                track,
                egui::CornerRadius::same(1),
                theme::ink(theme, theme::INK_10),
            );
            painter.rect_filled(
                egui::Rect::from_min_size(
                    track.min,
                    egui::vec2(track.width() * v.clamp(0.0, 1.0), track.height()),
                ),
                egui::CornerRadius::same(1),
                c,
            );
        }
    });
}

/// The width left in `ui` that is actually on screen: a wide header row
/// can stretch the pane's layout past its clip edge.
pub(super) fn visible_width(ui: &egui::Ui) -> f32 {
    ui.available_width()
        .min(ui.clip_rect().right() - ui.cursor().left())
        .max(0.0)
}

/// The room a vertical scroll bar takes.
pub(super) fn scroll_bar_width(ui: &egui::Ui) -> f32 {
    let s = &ui.spacing().scroll;
    s.bar_width + s.bar_inner_margin + s.bar_outer_margin
}

/// A play icon on a glass disc, centred at `c`, `r` across the disc's
/// radius: the shot the tape is on.
pub(super) fn play_mark(
    painter: &egui::Painter,
    theme: &Palette,
    c: egui::Pos2,
    r: f32,
    color: egui::Color32,
) {
    painter.circle(c, r, theme::glass_fill(theme), theme::ink_line(theme));
    let size = (r * 1.1).round();
    icons::paint(
        painter,
        egui::Rect::from_center_size(c + egui::vec2(1.0, 0.0), egui::Vec2::splat(size)),
        icons::PLAY,
        size,
        color,
    );
}

/// The mark badge: its colour and icon.
pub(super) fn verdict_style(theme: &Palette, v: Verdict) -> (egui::Color32, char) {
    match v {
        Verdict::Pick => (theme::rgb(theme.success), icons::CHECK),
        Verdict::Reject => (theme::rgb(theme.danger), icons::X),
    }
}

/// A small glass plate (Selects' card badges: the glass recipe, 2 px
/// corners, 4 px side padding) anchored at `pos`, holding an optional icon
/// in its colour and an optional label in white 12 px Medium. Returns its
/// rect.
pub(super) fn glass_badge(
    painter: &egui::Painter,
    theme: &Palette,
    pos: egui::Pos2,
    anchor: egui::Align2,
    icon: Option<(char, egui::Color32)>,
    label: &str,
) -> egui::Rect {
    let font = theme::medium(painter.ctx(), theme::BODY_SIZE);
    let galley = (!label.is_empty())
        .then(|| painter.layout_no_wrap(label.to_owned(), font, egui::Color32::WHITE));
    let icon_w = if icon.is_some() {
        icons::SMALL - 2.0
    } else {
        0.0
    };
    let gap = if icon.is_some() && galley.is_some() {
        2.0
    } else {
        0.0
    };
    let text_w = galley.as_ref().map_or(0.0, |g| g.size().x);
    let size = egui::vec2(icon_w + gap + text_w + SPACE_XS * 2.0, SPACE_LG + 2.0);
    let rect = anchor.anchor_size(pos, size);
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(theme::RADIUS_XS),
        theme::glass_fill(theme),
    );
    let mut x = rect.left() + SPACE_XS;
    if let Some((glyph, color)) = icon {
        icons::paint(
            painter,
            egui::Rect::from_min_size(
                egui::pos2(x, rect.center().y - icon_w / 2.0),
                egui::Vec2::splat(icon_w),
            ),
            glyph,
            icon_w,
            color,
        );
        x += icon_w + gap;
    }
    if let Some(galley) = galley {
        let y = rect.center().y - galley.size().y / 2.0;
        painter.galley(egui::pos2(x, y), galley, egui::Color32::WHITE);
    }
    rect
}

/// A segment's length for a badge: `4.2s`, or `1:07` from a minute up.
pub(super) fn duration(seconds: f32) -> String {
    let s = seconds.max(0.0);
    if s < 60.0 {
        format!("{s:.1}s")
    } else {
        let whole = s.round() as u32;
        format!("{}:{:02}", whole / 60, whole % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::{Options, Sort, View, reveal_offset};

    #[test]
    fn options_round_trip_and_survive_junk() -> anyhow::Result<()> {
        let o = Options {
            sort: Sort::Fire,
            view: View::List,
            group: true,
            cutoff: 0.25,
            scrub: false,
            card: 200.0,
        };
        let back = Options::from_table(Some(&o.to_table()));
        assert_eq!(back, o, "round trip");
        let junk: toml::Table = toml::from_str("sort = 3\ncutoff = \"high\"\ncard = 9999.0")?;
        let d = Options::from_table(Some(&junk));
        assert_eq!(d.sort, Sort::Best, "a bad sort keeps the default");
        assert!(d.card <= super::CARD_MAX, "card size clamped");
        Ok(())
    }

    #[test]
    fn reveal_scrolls_only_when_out_of_view() {
        assert_eq!(reveal_offset(100.0, 150.0, (0.0, 400.0)), None, "visible");
        assert_eq!(
            reveal_offset(500.0, 550.0, (0.0, 400.0)),
            Some(150.0),
            "below"
        );
        assert_eq!(
            reveal_offset(10.0, 60.0, (100.0, 400.0)),
            Some(10.0),
            "above"
        );
    }

    /// The review modes, driven through the real app by key presses in
    /// egui_kittest's headless harness.
    mod modes {
        use super::super::{Kind, Mode, State, result_ids, selected_ids};
        use crate::analyze::segments::Segment;
        use crate::analyze::{ClipIndex, Sample};
        use crate::gui::app::{Action, App, Page};
        use crate::gui::player::Source;
        use crate::index::{Index, Scored};
        use crate::media::Probe;
        use crate::project::Project;
        use crate::search::marks::Verdict;
        use egui::{Key, Modifiers};
        use egui_kittest::Harness;

        /// Clips A (3 shots), B (2) and C (1), best first.
        fn index() -> Index {
            let probe = Probe {
                duration: 60.0,
                width: 1920,
                height: 1080,
                fps: [24000, 1001],
                vcodec: "h264".into(),
                has_audio: false,
            };
            let mut clips = Vec::new();
            let mut segments = Vec::new();
            let mut q = 0.95;
            for (clip, n) in [("A", 3), ("B", 2), ("C", 1)] {
                clips.push(ClipIndex {
                    id: clip.into(),
                    path: format!("{clip}.mov"),
                    stamp: (0, 0),
                    probe: probe.clone(),
                    samples: vec![Sample::default()],
                    segments: Vec::new(),
                    hist: Vec::new(),
                    version: crate::analyze::ANALYSIS_VERSION,
                });
                for k in 0..n {
                    let t = k as f32 * 10.0;
                    segments.push(Scored {
                        id: format!("{clip}/{}", k + 1),
                        clip: clip.into(),
                        seg: Segment {
                            start: t,
                            end: t + 2.0 + k as f32 * 4.0,
                            cx: 0.5,
                            sharp: 1.0,
                            ..Segment::default()
                        },
                        subject: "fire".into(),
                        q,
                        ..Default::default()
                    });
                    q -= 0.1;
                }
            }
            Index {
                source: String::new(),
                sharp_ref: 1.0,
                clips,
                segments,
                ..Default::default()
            }
        }

        /// The app on the Shots tab of a night holding [`index`].
        fn harness(work: &std::path::Path) -> anyhow::Result<Harness<'static, Option<App>>> {
            let mut project = Project::open(work)?;
            project.index = Some(index());
            let mut project = Some(project);
            let mut harness = Harness::builder()
                .with_size(egui::vec2(1600.0, 900.0))
                .with_accessibility_check(false)
                .build_ui_state(
                    move |ui, app: &mut Option<App>| {
                        let app = app.get_or_insert_with(|| {
                            let mut app =
                                App::new(ui.ctx(), crate::config::Config::default(), None);
                            app.project = project.take();
                            app.apply(ui.ctx(), Action::Go(Page::Review));
                            app
                        });
                        app.show(ui);
                    },
                    None,
                );
            harness.run_steps(4);
            Ok(harness)
        }

        fn press(h: &mut Harness<'_, Option<App>>, key: Key) {
            h.key_press(key);
            h.run_steps(3);
        }

        fn press_with(h: &mut Harness<'_, Option<App>>, modifiers: Modifiers, key: Key) {
            h.key_press_modifiers(modifiers, key);
            h.run_steps(3);
        }

        fn app<'a>(h: &'a Harness<'_, Option<App>>) -> anyhow::Result<&'a App> {
            h.state()
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("the app was never built"))
        }

        fn review<'a>(h: &'a Harness<'_, Option<App>>) -> anyhow::Result<&'a State> {
            Ok(&app(h)?.pages.review)
        }

        fn selection(h: &Harness<'_, Option<App>>) -> anyhow::Result<Vec<String>> {
            Ok(selected_ids(&app(h)?.selection))
        }

        #[test]
        fn tape_plays_the_results_and_the_selection_follows() -> anyhow::Result<()> {
            let dir = tempfile::tempdir()?;
            let mut h = harness(dir.path())?;
            let order = result_ids(review(&h)?);
            assert_eq!(order.len(), 6, "every shot is a result: {order:?}");
            press(&mut h, Key::ArrowRight);
            press(&mut h, Key::T);
            let a = app(&h)?;
            assert_eq!(a.pages.review.mode.kind(), Kind::Tape, "T starts the tape");
            let Source::Sequence { shots, .. } = a.player.source() else {
                anyhow::bail!("the viewer holds {:?}", a.player.source());
            };
            assert_eq!(shots.len(), 6, "the whole result set is on the tape");
            assert!(a.player.playing(), "the tape plays");
            assert_eq!(selection(&h)?, order[..1], "the first shot is selected");

            press_with(&mut h, Modifiers::SHIFT, Key::L);
            let Mode::Tape(t) = &review(&h)?.mode else {
                anyhow::bail!("left the tape");
            };
            assert!(t.fast, "Shift+L turns Fast Review on");

            press(&mut h, Key::P);
            let r = review(&h)?;
            assert_eq!(
                r.marks.get(&order[0]).verdict,
                Some(Verdict::Pick),
                "P picks the playing shot"
            );
            assert_eq!(
                selection(&h)?,
                order[1..2],
                "and the tape moves on, the selection with it"
            );
            let Mode::Tape(t) = &r.mode else {
                anyhow::bail!("P left the tape");
            };
            assert_eq!(t.playing(), Some(order[1].as_str()), "the playing card");

            press(&mut h, Key::R);
            let staged: Vec<String> = selected_ids(&app(&h)?.refs);
            assert_eq!(staged, order[1..2], "R still mentions the selection");

            press(&mut h, Key::Escape);
            let a = app(&h)?;
            assert_eq!(a.pages.review.mode.kind(), Kind::Browse, "Esc leaves");
            assert!(!a.player.playing(), "and stops the tape");
            Ok(())
        }

        #[test]
        fn compare_steps_the_candidate_and_promotes_it() -> anyhow::Result<()> {
            let dir = tempfile::tempdir()?;
            let mut h = harness(dir.path())?;
            let order = result_ids(review(&h)?);
            press(&mut h, Key::ArrowRight);
            press(&mut h, Key::C);
            let Mode::Compare(c) = &review(&h)?.mode else {
                anyhow::bail!("C did not compare");
            };
            assert_eq!(
                (c.select.as_str(), c.candidate.as_str()),
                (order[0].as_str(), order[1].as_str()),
                "the focused shot against the next"
            );
            assert_eq!(selection(&h)?, order[..2], "both are selected");

            press(&mut h, Key::ArrowRight);
            press(&mut h, Key::X);
            let r = review(&h)?;
            assert_eq!(
                r.marks.get(&order[2]).verdict,
                Some(Verdict::Reject),
                "X rejects the candidate"
            );
            let Mode::Compare(c) = &r.mode else {
                anyhow::bail!("X left compare");
            };
            assert_eq!(c.candidate, order[3], "and moves the candidate on");

            press(&mut h, Key::ArrowUp);
            let Mode::Compare(c) = &review(&h)?.mode else {
                anyhow::bail!("Up left compare");
            };
            assert_eq!(c.select, order[3], "Up promotes the candidate");
            assert_eq!(c.candidate, order[4], "the next result is the candidate");

            press(&mut h, Key::Escape);
            assert_eq!(review(&h)?.mode.kind(), Kind::Browse, "Esc leaves");
            assert_eq!(selection(&h)?, order[3..4], "the select stays selected");
            Ok(())
        }

        #[test]
        fn survey_drops_shots_and_goes_on_to_the_next_group() -> anyhow::Result<()> {
            let dir = tempfile::tempdir()?;
            let mut h = harness(dir.path())?;
            press(&mut h, Key::ArrowRight);
            press(&mut h, Key::G);
            let Mode::Survey(s) = &review(&h)?.mode else {
                anyhow::bail!("G did not survey");
            };
            assert_eq!(s.ids, ["A/1", "A/2", "A/3"], "the focused shot's group");
            assert_eq!(selection(&h)?, s.ids, "the tiles are the selection");

            press(&mut h, Key::X);
            let r = review(&h)?;
            let Mode::Survey(s) = &r.mode else {
                anyhow::bail!("X left the survey");
            };
            assert_eq!(s.ids, ["A/2", "A/3"], "X drops the focused tile");
            assert_eq!(r.marks.get("A/1").verdict, None, "without rejecting it");
            assert_eq!(selection(&h)?, ["A/2", "A/3"], "the selection shrinks too");

            press(&mut h, Key::CloseBracket);
            let Mode::Survey(s) = &review(&h)?.mode else {
                anyhow::bail!("] left the survey");
            };
            assert_eq!(s.ids, ["B/1", "B/2"], "] loads the next group");

            press(&mut h, Key::OpenBracket);
            let Mode::Survey(s) = &review(&h)?.mode else {
                anyhow::bail!("[ left the survey");
            };
            assert_eq!(s.ids, ["A/1", "A/2", "A/3"], "[ goes back");

            press_with(&mut h, Modifiers::SHIFT, Key::R);
            assert!(app(&h)?.ref_popup, "Shift+R still offers the intents");
            Ok(())
        }
    }
}

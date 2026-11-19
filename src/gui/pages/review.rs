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
//! Marks are saved to `<work>/review.toml` (see [`crate::search::marks`]).
//! View options persist in `ave.toml` under `[pages.review]`.

mod grid;
mod list;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::PageCtx;
use crate::gui::app::Action;
use crate::gui::ui::theme::{self, SPACE_SM, SPACE_XS};
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
            card: 150.0,
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
            ui.add_space(SPACE_SM);
            crate::gui::ui::widgets::muted(
                ui,
                ctx.theme,
                "No shot log yet. Run Index on the Home screen, then come back here to cull.",
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
        let paint = Paint {
            theme: ctx.theme,
            night: &night,
            items: &state.items,
            paths: &state.paths,
            selected: &selected,
            cursor: state.cursor.as_deref(),
            scrub: state.opts.scrub,
        };
        if state.items.is_empty() {
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
                && let Some(item) = state.items.get(pos.0)
                && let Some(s) = index.segments.get(item.seg)
                && state.cursor.as_deref() == Some(s.id.as_str())
            {
                // Scrubbing the selected card drives the viewer too.
                ctx.player.pause();
                ctx.player.seek(f64::from(pos.1 - s.seg.start));
            }
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

/// The header: count, order, view, grouping, cut-off, smart searches and
/// the query's chips. Returns whether a view option changed.
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
    let small = |text: &str| egui::RichText::new(text).size(12.0);
    ui.spacing_mut().item_spacing = egui::vec2(SPACE_SM, SPACE_XS);
    ui.spacing_mut().interact_size.y = 20.0;
    ui.spacing_mut().button_padding = egui::vec2(6.0, 1.0);
    ui.horizontal(|ui| {
        let shown = state.items.len();
        let stacks = state
            .items
            .iter()
            .filter(|i| matches!(i.stack, Stack::Top { .. }))
            .count();
        ui.label(
            egui::RichText::new(format!("{} shots", state.matched))
                .size(12.0)
                .strong()
                .color(theme::rgb(theme.text)),
        );
        let mut detail = if query.is_empty() {
            String::new()
        } else {
            format!("of {total}")
        };
        if state.matched != shown && !state.opts.group {
            detail.push_str(&format!(" · {} above cut-off", shown));
        }
        if state.opts.group && stacks > 0 {
            detail.push_str(&format!(" · {stacks} stacks"));
        }

        if picks + rejects > 0 {
            detail.push_str(&format!(" · {picks} picked · {rejects} rejected"));
        }
        ui.label(small(detail.trim_start_matches(" · ")).color(theme::rgb(theme.text_muted)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (view, label, tip) in [
                (View::List, "☰", "List view"),
                (View::Grid, "▦", "Grid view"),
            ] {
                if ui
                    .selectable_label(state.opts.view == view, small(label))
                    .on_hover_text(tip)
                    .clicked()
                {
                    state.opts.view = view;
                }
            }
            if ui
                .selectable_label(state.opts.group, small("Stacks"))
                .on_hover_text("Group each clip's shots into one stack")
                .clicked()
            {
                state.opts.group = !state.opts.group;
            }
        });
    });
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("review-sort")
            .selected_text(small(if state.opts.sort == Sort::Best && !query.ranks() {
                "Best (quality)"
            } else {
                state.opts.sort.label()
            }))
            .width(96.0)
            .show_ui(ui, |ui| {
                for s in Sort::ALL {
                    ui.selectable_value(&mut state.opts.sort, s, s.label());
                }
            })
            .response
            .on_hover_text("Order");
        ui.label(small("min").color(theme::rgb(theme.text_muted)));
        ui.add(
            egui::Slider::new(&mut state.opts.cutoff, 0.0..=0.95)
                .step_by(0.05)
                .fixed_decimals(2)
                .trailing_fill(true),
        )
        .on_hover_text("Hide results scoring below this (quality when nothing is searched)");
        ui.menu_button(small("Smart ▾"), |ui| {
            for (label, q) in SMART {
                if ui.button(*label).on_hover_text(*q).clicked() {
                    dos.push(Do::Search((*q).to_owned()));
                    ui.close();
                }
            }
            ui.separator();
            if state.opts.view == View::Grid {
                ui.add(
                    egui::Slider::new(&mut state.opts.card, CARD_MIN..=CARD_MAX)
                        .text("card size")
                        .show_value(false),
                );
            }
            ui.checkbox(&mut state.opts.scrub, "Hover scrub");
        });
    });
    if !query.is_empty() {
        chips(ui, theme, query, dos);
    }
    if let Some(err) = &state.error {
        ui.label(small(err).color(theme::rgb(theme.warning)));
    }
    ui.add_space(SPACE_XS);
    state.opts != before
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

/// The query's tokens as removable chips.
fn chips(ui: &mut egui::Ui, theme: &Palette, query: &Query, dos: &mut Vec<Do>) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(SPACE_XS, SPACE_XS);
        for (i, c) in query.clauses.iter().enumerate() {
            let text = format!("{}  ✕", c.label());
            let galley =
                ui.painter()
                    .layout_no_wrap(text, theme::font(11.0), theme::rgb(theme.text));
            let size = galley.size() + egui::vec2(12.0, 4.0);
            let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
            let fill = match (c.negate, c.is_text(), resp.hovered()) {
                (_, _, true) => theme.hover,
                (true, _, false) => theme.raised,
                (false, true, false) => theme.panel,
                (false, false, false) => theme.accent_soft,
            };
            let stroke = if c.negate { theme.danger } else { theme.border };
            ui.painter().rect(
                rect,
                egui::CornerRadius::same(3),
                theme::rgb(fill),
                egui::Stroke::new(1.0, theme::rgb(stroke)),
                egui::StrokeKind::Inside,
            );
            ui.painter().galley(
                rect.min + egui::vec2(6.0, 2.0),
                galley,
                theme::rgb(theme.text),
            );
            if resp.on_hover_text("Remove").clicked() {
                dos.push(Do::Search(query.without(i)));
            }
        }
    });
}

/// No results: say why and offer a way out.
fn empty(ui: &mut egui::Ui, theme: &Palette, query: &Query, matched: usize, dos: &mut Vec<Do>) {
    ui.add_space(SPACE_SM);
    let text = if matched > 0 {
        format!("{matched} shots match, all below the cut-off.")
    } else if query.is_empty() {
        "The shot log is empty.".to_owned()
    } else {
        "No shots match.".to_owned()
    };
    crate::gui::ui::widgets::muted(ui, theme, &text);
    if !query.is_empty() && ui.small_button("Clear search").clicked() {
        dos.push(Do::Search(String::new()));
    }
}

/// Reads the tab's keys.
fn keys(ui: &egui::Ui, state: &State, dos: &mut Vec<Do>) {
    use egui::Key;
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
                let label = if r == 0 {
                    "No rating".to_owned()
                } else {
                    "★".repeat(usize::from(r))
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
                step(ctx, state, 1, false);
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
    }
}

/// Who a verdict, rating or reference applies to: the selection, or the
/// cursor when nothing is selected.
fn targets(ctx: &PageCtx<'_>, state: &State) -> Vec<String> {
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

/// Loads result `item` into the viewer.
fn open(ctx: &mut PageCtx<'_>, state: &State, item: usize) {
    let Some(it) = state.items.get(item) else {
        return;
    };
    let Some(seg) = ctx
        .project
        .as_ref()
        .and_then(|p| p.index.as_ref())
        .and_then(|i| i.segments.get(it.seg))
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
    if max_w <= 4.0 {
        return 0.0;
    }
    let mut job =
        egui::text::LayoutJob::simple_singleline(text.to_owned(), theme::font(size), color);
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

/// The mark badge: fill colour and glyph.
pub(super) fn verdict_style(theme: &Palette, v: Verdict) -> (egui::Color32, &'static str) {
    match v {
        Verdict::Pick => (theme::rgb(theme.success), "✓"),
        Verdict::Reject => (theme::rgb(theme.danger), "✕"),
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
}

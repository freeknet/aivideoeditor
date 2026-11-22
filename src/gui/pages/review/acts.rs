// ============================================================================
// Module:       gui::pages::review::acts
// Description:  The Shots tab grouped by act: results in recording order
//               under a full-width header per set of the night, folded on
//               click, laid out as rows of two fixed heights.
//
// Dependencies: egui; super (Item, Paint), crate::{index, search}
// ============================================================================

//! Group: Acts.
//!
//! The index splits a night into sets, its acts (see
//! [`crate::analyze::sets`]). With the Shots tab grouped by act the
//! results come in recording order, and each act opens with a header row
//! across the whole pane: `Set 3 · Indoor stage · LED · 20:35–20:42 · 4
//! clips`. Clicking the header folds the act away; its filter button
//! scopes the search to it (`set:3`).
//!
//! The grid and the list stay virtualised: [`layout`] turns the acts into
//! rows of two kinds, a header row and a row of up to `cols` results, each
//! kind of a fixed height, so the rows in view are found by a binary
//! search over their tops and only those are drawn.

use std::collections::{BTreeSet, HashMap};
use std::ops::Range;

use egui::{CornerRadius, Rect, Sense, pos2, vec2};

use super::{Item, Paint};
use crate::gui::ui::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::gui::ui::{icons, widgets};
use crate::index::Index;
use crate::search;

/// One act as the browser shows it.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::gui::pages) struct Act {
    /// The set's number, from 1; 0 for results whose clip is in no set.
    pub(super) n: usize,
    /// The set's derived title (`Indoor stage · LED`).
    pub(super) title: String,
    /// `20:35–20:42`, or empty when the set has no times.
    pub(super) span: String,
    /// Clips in the set.
    pub(super) clips: usize,
    /// Why the set starts where it does (`26 min gap`).
    pub(super) why: String,
    /// Results in the act, folded or not.
    pub(super) shots: usize,
    /// The act's results among the shown items; empty when folded.
    pub(super) items: Range<usize>,
    /// Folded away by a click on its header.
    pub(super) folded: bool,
}

impl Act {
    /// `Set 3 · Indoor stage · LED · 20:35–20:42 · 4 clips`.
    pub(super) fn label(&self) -> String {
        let mut parts = vec![if self.n == 0 {
            "Other clips".to_owned()
        } else {
            format!("Set {}", self.n)
        }];
        if !self.title.is_empty() {
            parts.push(self.title.clone());
        }
        if !self.span.is_empty() {
            parts.push(self.span.clone());
        }
        parts.push(match self.clips {
            1 => "1 clip".to_owned(),
            n => format!("{n} clips"),
        });
        parts.join(" · ")
    }

    /// [`Act::label`] with the time first, for a pane too narrow for it
    /// all: `Set 3 · 20:35–20:42 · Indoor stage · LED · 4 clips`.
    pub(super) fn compact(&self) -> String {
        if self.span.is_empty() {
            return self.label();
        }
        let label = Self {
            span: String::new(),
            ..self.clone()
        }
        .label();
        match label.split_once(" · ") {
            Some((head, rest)) => format!("{head} · {} · {rest}", self.span),
            None => format!("{label} · {}", self.span),
        }
    }
}

/// Each clip's set number and position in it (recording order).
fn positions(index: &Index) -> HashMap<&str, (usize, usize)> {
    index
        .sets
        .iter()
        .flat_map(|set| {
            set.clips
                .iter()
                .enumerate()
                .map(move |(k, c)| (c.as_str(), (set.n, k)))
        })
        .collect()
}

/// Orders hits by recording time: set by set, each set's clips in the
/// order they were shot, each clip's segments in time. Clips in no set go
/// last, by name; a night without sets is simply in clip-then-time order.
pub(super) fn order(hits: &mut [search::Hit], index: &Index) {
    let at = positions(index);
    let key = |h: &search::Hit| {
        let place = index
            .segments
            .get(h.seg)
            .and_then(|s| at.get(s.clip.as_str()).copied())
            .unwrap_or((usize::MAX, 0));
        (place, h.seg)
    };
    hits.sort_by_key(key);
}

/// Splits ordered results into acts: each run of results from one set
/// becomes an [`Act`]. A folded act keeps its count but drops its results
/// from the returned items. A night without sets has no acts and keeps
/// every item.
pub(super) fn split(
    items: Vec<Item>,
    index: &Index,
    folded: &BTreeSet<usize>,
) -> (Vec<Item>, Vec<Act>) {
    if index.sets.is_empty() {
        return (items, Vec::new());
    }
    let at = positions(index);
    let set_of = |it: &Item| {
        index
            .segments
            .get(it.seg)
            .and_then(|s| at.get(s.clip.as_str()))
            .map_or(0, |(n, _)| *n)
    };
    let mut shown = Vec::with_capacity(items.len());
    let mut acts: Vec<Act> = Vec::new();
    for it in items {
        let n = set_of(&it);
        if acts.last().is_none_or(|a| a.n != n) {
            let set = index.sets.iter().find(|s| s.n == n);
            acts.push(Act {
                n,
                title: set.map_or_else(String::new, |s| s.title.clone()),
                span: set.map_or_else(String::new, |s| s.span().replace('-', "–")),
                clips: set.map_or(0, |s| s.clips.len()),
                why: set.map_or_else(String::new, |s| s.why.clone()),
                shots: 0,
                items: shown.len()..shown.len(),
                folded: folded.contains(&n),
            });
        }
        let Some(act) = acts.last_mut() else {
            continue;
        };
        act.shots += 1;
        if !act.folded {
            shown.push(it);
            act.items.end = shown.len();
        }
    }
    if acts.iter().any(|a| a.n == 0) {
        // Clips the sets do not name: count them for their header.
        let mut clips: Vec<&str> = Vec::new();
        for s in index
            .segments
            .iter()
            .filter(|s| !at.contains_key(s.clip.as_str()))
        {
            if !clips.contains(&s.clip.as_str()) {
                clips.push(s.clip.as_str());
            }
        }
        for a in acts.iter_mut().filter(|a| a.n == 0) {
            a.clips = clips.len();
        }
    }
    (shown, acts)
}

/// One row of the grouped grid or list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Row {
    /// Act `k`'s header (an index into the acts).
    Header(usize),
    /// Results `start..end` side by side.
    Tiles {
        /// First result.
        start: usize,
        /// One past the last.
        end: usize,
    },
}

/// The fixed heights the rows are laid out with, points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Metrics {
    /// A header row.
    pub(super) header: f32,
    /// Space under a header row.
    pub(super) header_gap: f32,
    /// Extra space above a header that is not the first row.
    pub(super) header_lead: f32,
    /// A row of results.
    pub(super) tiles: f32,
    /// Space under a row of results.
    pub(super) tiles_gap: f32,
}

/// The rows, where each starts and how tall it is.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Layout {
    /// The rows, top to bottom.
    pub(super) rows: Vec<Row>,
    /// Each row's top, points from the top of the content.
    tops: Vec<f32>,
    /// Each row's height.
    heights: Vec<f32>,
    /// The content's height.
    pub(super) total: f32,
}

/// The rows for `n` results `cols` to a row: a header per act and its
/// results under it, or (no acts) the results alone.
pub(super) fn rows(acts: &[Act], n: usize, cols: usize) -> Vec<Row> {
    let cols = cols.max(1);
    let tiles = |range: Range<usize>, out: &mut Vec<Row>| {
        let mut start = range.start;
        while start < range.end {
            let end = (start + cols).min(range.end);
            out.push(Row::Tiles { start, end });
            start = end;
        }
    };
    let mut out = Vec::new();
    if acts.is_empty() {
        tiles(0..n, &mut out);
    } else {
        for (k, act) in acts.iter().enumerate() {
            out.push(Row::Header(k));
            tiles(act.items.start..act.items.end.min(n), &mut out);
        }
    }
    out
}

/// Lays out [`rows`] with fixed heights per row kind.
pub(super) fn layout(acts: &[Act], n: usize, cols: usize, m: Metrics) -> Layout {
    let rows = rows(acts, n, cols);
    let mut tops = Vec::with_capacity(rows.len());
    let mut heights = Vec::with_capacity(rows.len());
    let mut y = 0.0;
    for (k, row) in rows.iter().enumerate() {
        let (h, gap) = match row {
            Row::Header(_) => {
                if k > 0 {
                    y += m.header_lead;
                }
                (m.header, m.header_gap)
            }
            Row::Tiles { .. } => (m.tiles, m.tiles_gap),
        };
        tops.push(y);
        heights.push(h);
        y += h + gap;
    }
    Layout {
        rows,
        tops,
        heights,
        total: y,
    }
}

impl Layout {
    /// Row `r`'s top and height.
    pub(super) fn span(&self, r: usize) -> (f32, f32) {
        (
            self.tops.get(r).copied().unwrap_or(0.0),
            self.heights.get(r).copied().unwrap_or(0.0),
        )
    }

    /// The rows that overlap `y0..y1` (content points).
    pub(super) fn visible(&self, y0: f32, y1: f32) -> Range<usize> {
        // The first row ending at or below y0: bottoms only grow.
        let (mut first, mut hi) = (0, self.tops.len());
        while first < hi {
            let mid = first + (hi - first) / 2;
            let (top, h) = self.span(mid);
            if top + h < y0 {
                first = mid + 1;
            } else {
                hi = mid;
            }
        }
        let last = self.tops.partition_point(|t| *t < y1);
        first..last.max(first)
    }

    /// The row result `item` is in.
    pub(super) fn row_of(&self, item: usize) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| matches!(r, Row::Tiles { start, end } if (*start..*end).contains(&item)))
    }
}

/// The result `rows` rows up (negative) or down from `item`, in the same
/// column where the row is long enough, else its last; header rows are
/// stepped over. `None` when there is no such row.
pub(super) fn vertical(rows: &[Row], item: usize, delta: isize) -> Option<usize> {
    let r = rows
        .iter()
        .position(|r| matches!(r, Row::Tiles { start, end } if (*start..*end).contains(&item)))?;
    let Row::Tiles { start, .. } = rows.get(r)? else {
        return None;
    };
    let col = item - start;
    let mut left = delta.unsigned_abs();
    let mut k = r;
    while left > 0 {
        k = if delta < 0 { k.checked_sub(1)? } else { k + 1 };
        match rows.get(k)? {
            Row::Tiles { start, end } => {
                left -= 1;
                if left == 0 {
                    return Some((start + col).min(end - 1));
                }
            }
            Row::Header(_) => {}
        }
    }
    None
}

/// What a header row hands back.
pub(super) struct Header {
    /// The header was clicked: fold or unfold the act.
    pub(super) fold: bool,
    /// Its filter button was clicked: scope the search to the act.
    pub(super) scope: bool,
}

/// Paints act `act`'s header across `rect`: a chevron, the set's number in
/// 12 px Medium and the rest of its label muted, the result count and a
/// filter button at the right end. Hover is an ink wash.
pub(super) fn header(ui: &mut egui::Ui, paint: &Paint<'_>, act: &Act, rect: Rect) -> Header {
    let theme = paint.theme;
    let label = act.label();
    let id = egui::Id::unique(("review-act", act.n));
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    let painter = ui.painter_at(rect);
    if ui.rect_contains_pointer(rect) {
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_MD),
            theme::ink(theme, theme::INK_HOVER),
        );
    }
    let cy = rect.center().y;
    let mut x = rect.left() + SPACE_XS;
    let chevron = if act.folded {
        icons::CHEVRON_RIGHT
    } else {
        icons::CHEVRON_DOWN
    };
    icons::paint(
        &painter,
        Rect::from_center_size(
            pos2(x + icons::SMALL / 2.0, cy),
            vec2(icons::SMALL, icons::SMALL),
        ),
        chevron,
        icons::SMALL,
        theme::rgb(theme.text_muted),
    );
    x += icons::SMALL + SPACE_XS + 2.0;

    // The filter button shows while the pointer is on the row; the count
    // only where the whole label fits beside it, so a narrow pane keeps
    // the act's name and time rather than a number.
    let hovered = ui.rect_contains_pointer(rect);
    let button = theme::COMPACT_HEIGHT;
    let scope_rect = Rect::from_center_size(
        pos2(rect.right() - SPACE_XS - button / 2.0, cy),
        vec2(button, button),
    );
    let right = if act.n > 0 && hovered {
        scope_rect.left() - SPACE_XS
    } else {
        rect.right() - SPACE_SM
    };
    let count = match act.shots {
        1 => "1 shot".to_owned(),
        n => format!("{n} shots"),
    };
    let split = |text: &str| -> (String, String) {
        text.split_once(" · ")
            .map_or((text.to_owned(), String::new()), |(h, r)| {
                (h.to_owned(), format!(" · {r}"))
            })
    };
    let (mut head, mut rest) = split(&label);
    let head_font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    let rest_font = theme::font(theme::BODY_SIZE);
    let count_font = theme::tabular(ui.ctx(), theme::BODY_SIZE);
    let width = |text: &str, font: &egui::FontId| {
        painter
            .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
            .size()
            .x
    };
    let label_w = width(&head, &head_font) + width(&rest, &rest_font);
    let count_w = width(&count, &count_font);
    if x + label_w > right {
        // Too narrow for it all: the time goes next to the number, where
        // the cut cannot reach it.
        (head, rest) = split(&act.compact());
    }
    let room = if x + label_w + SPACE_MD + count_w <= right {
        super::text_in(
            &painter,
            pos2(right - count_w, cy),
            &count,
            count_font,
            theme::rgb(theme.text_faint),
            count_w + 2.0,
        );
        right - count_w - SPACE_MD - x
    } else {
        right - x
    };
    let w = super::text_in(
        &painter,
        pos2(x, cy),
        &head,
        head_font,
        theme::rgb(theme.text),
        room,
    );
    if !rest.is_empty() {
        super::text_in(
            &painter,
            pos2(x + w, cy),
            &rest,
            rest_font,
            theme::rgb(theme.text_muted),
            room - w,
        );
    }
    painter.hline(
        rect.x_range().shrink(SPACE_XS),
        rect.bottom() - 0.5,
        theme::ink_line(theme),
    );

    let scope = act.n > 0 && {
        let scope_label = format!("Only this act (set:{})", act.n);
        let b = ui.interact(scope_rect, id.with("scope"), Sense::click());
        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &scope_label));
        if hovered {
            let fill = if b.hovered() {
                theme::ink(theme, theme::INK_12)
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter()
                .rect_filled(scope_rect, CornerRadius::same(theme::RADIUS), fill);
            icons::paint(
                ui.painter(),
                scope_rect,
                icons::LIST_FILTER,
                icons::SMALL,
                theme::rgb(if b.hovered() {
                    theme.text
                } else {
                    theme.text_muted
                }),
            );
        }
        widgets::tip(b, theme, &scope_label, "").clicked()
    };
    let mut tip = format!("{label} · {count}");
    if !act.why.is_empty() {
        tip.push_str(&format!(" ({})", act.why));
    }
    tip.push_str(if act.folded {
        "\nClick to unfold"
    } else {
        "\nClick to fold"
    });
    let fold = !scope && widgets::tip(response, theme, &tip, "").clicked();
    Header { fold, scope }
}

/// The query with its act scoped to `n`: any other `set:` token goes.
pub(super) fn scoped(search: &str, n: usize) -> String {
    let query = search::Query::parse(search);
    let mut text = search.trim().to_owned();
    if let Some(i) = query
        .clauses
        .iter()
        .position(|c| matches!(c.term, search::Term::Set(_)) && !c.negate)
    {
        text = query.without(i);
    }
    format!("{} set:{n}", text.trim()).trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{Act, Metrics, Row, layout, rows, scoped, vertical};

    fn act(n: usize, items: std::ops::Range<usize>) -> Act {
        Act {
            n,
            title: "Indoor stage · LED".into(),
            span: "20:35–20:42".into(),
            clips: 4,
            why: "26 min gap".into(),
            shots: items.len(),
            items,
            folded: false,
        }
    }

    const M: Metrics = Metrics {
        header: 28.0,
        header_gap: 8.0,
        header_lead: 4.0,
        tiles: 100.0,
        tiles_gap: 16.0,
    };

    #[test]
    fn headers_open_each_act_and_rows_break_at_act_ends() -> anyhow::Result<()> {
        let acts = [act(1, 0..5), act(2, 5..7)];
        let r = rows(&acts, 7, 3);
        assert_eq!(
            r,
            [
                Row::Header(0),
                Row::Tiles { start: 0, end: 3 },
                Row::Tiles { start: 3, end: 5 },
                Row::Header(1),
                Row::Tiles { start: 5, end: 7 },
            ],
            "a header before each act; an act's last row is short rather than run into the next"
        );
        assert_eq!(
            rows(&[], 7, 3).len(),
            3,
            "with no acts the results are plain rows"
        );
        let mut folded = act(2, 5..5);
        folded.folded = true;
        assert_eq!(
            rows(&[act(1, 0..5), folded], 5, 3).last(),
            Some(&Row::Header(1)),
            "a folded act is its header alone"
        );
        Ok(())
    }

    #[test]
    fn layout_places_rows_by_kind_and_finds_the_visible_ones() -> anyhow::Result<()> {
        let acts = [act(1, 0..5), act(2, 5..7)];
        let l = layout(&acts, 7, 3, M);
        assert_eq!(l.span(0), (0.0, 28.0), "the first header at the top");
        assert_eq!(
            l.span(1),
            (36.0, 100.0),
            "tiles under the header and its gap"
        );
        assert_eq!(l.span(2), (152.0, 100.0), "the next tile row a gap lower");
        assert_eq!(
            l.span(3),
            (272.0, 28.0),
            "a later header gets its lead above it"
        );
        assert_eq!(l.total, 272.0 + 28.0 + 8.0 + 100.0 + 16.0, "content height");
        assert_eq!(l.visible(0.0, 30.0), 0..1, "only the first header");
        assert_eq!(l.visible(130.0, 280.0), 1..4, "rows overlapping the window");
        assert_eq!(l.visible(1000.0, 1200.0), 5..5, "nothing below the end");
        assert_eq!(l.row_of(6), Some(4), "result 6 is in the last row");
        assert_eq!(l.row_of(9), None, "no such result");
        Ok(())
    }

    #[test]
    fn up_and_down_step_over_headers_and_keep_the_column() -> anyhow::Result<()> {
        let acts = [act(1, 0..5), act(2, 5..9)];
        let r = rows(&acts, 9, 3);
        assert_eq!(vertical(&r, 1, 1), Some(4), "down within the act");
        assert_eq!(
            vertical(&r, 4, 1),
            Some(6),
            "down over a header, same column"
        );
        assert_eq!(
            vertical(&r, 2, 1),
            Some(4),
            "a short row below: its last result"
        );
        assert_eq!(vertical(&r, 7, -1), Some(4), "up over a header");
        assert_eq!(vertical(&r, 0, -1), None, "nothing above the first row");
        assert_eq!(vertical(&r, 8, 1), None, "nothing below the last row");
        Ok(())
    }

    #[test]
    fn labels_and_scoping() -> anyhow::Result<()> {
        assert_eq!(
            act(3, 0..2).label(),
            "Set 3 · Indoor stage · LED · 20:35–20:42 · 4 clips",
            "the header's text"
        );
        assert_eq!(
            act(3, 0..2).compact(),
            "Set 3 · 20:35–20:42 · Indoor stage · LED · 4 clips",
            "narrow: the time first"
        );
        assert_eq!(scoped("", 3), "set:3", "an empty query");
        assert_eq!(
            scoped("fire set:2", 3),
            "fire set:3",
            "replaces another act"
        );
        assert_eq!(scoped(" poi ", 1), "poi set:1", "appends");
        Ok(())
    }
}

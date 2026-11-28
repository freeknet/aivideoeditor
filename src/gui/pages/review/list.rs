// ============================================================================
// Module:       gui::pages::review::list
// Description:  The Shots tab's list: dense, virtualised rows of the shot-log
//               numbers, columns dropped by priority when the pane is narrow.
//
// Dependencies: egui, egui_material3; super (Paint, Item, Drawn, Sort),
//               crate::gui::thumbs
// ============================================================================

//! The list view: one row per result with the numbers the shot log prints.
//! Columns that do not fit are dropped, least useful first; the caption
//! takes whatever width is left. Clicking a sortable header sorts by it.
//!
//! Rows are MaterialList's virtualised rows, 28 px each (an act's header
//! is one too): only those in view are laid out. Each paints Selects' list
//! row over its item: a 16 px thumbnail, the name in 14 px and the numbers
//! in 12 px tabular, muted.

use crate::gui::ui::hint::Hint as _;
use std::cell::RefCell;

use egui::{CornerRadius, Rect, Sense, StrokeKind, pos2, vec2};
use egui_material3::{ListItem, MaterialList};

use super::acts::{self, Row};
use super::{Drawn, Item, Paint, Sort, Stack};
use crate::gui::thumbs::Thumbs;
use crate::gui::ui::icons;
use crate::gui::ui::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::search::Metric;
use crate::search::marks::Verdict;

/// Row height (Selects' `h-7`).
const ROW_H: f32 = theme::CONTROL_HEIGHT;

/// A row's inner padding at each end.
const PAD_X: f32 = SPACE_XS + 2.0;

/// Padding inside a cell, before its text.
const CELL_PAD: f32 = SPACE_XS;

/// Height of the leading thumbnail.
const THUMB_H: f32 = 16.0;

/// The narrowest the caption column gets before other columns go.
const CAPTION_MIN: f32 = 80.0;

/// A list column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Col {
    Thumb,
    Id,
    Time,
    Camera,
    Exp,
    Focus,
    Fire,
    Led,
    Move,
    Q,
    Flags,
}

impl Col {
    /// Every column, left to right.
    const ALL: [Self; 11] = [
        Self::Thumb,
        Self::Id,
        Self::Time,
        Self::Camera,
        Self::Exp,
        Self::Focus,
        Self::Fire,
        Self::Led,
        Self::Move,
        Self::Q,
        Self::Flags,
    ];

    /// Header text.
    fn title(self) -> &'static str {
        match self {
            Self::Thumb => "",
            Self::Id => "Shot",
            Self::Time => "Time",
            Self::Camera => "Camera",
            Self::Exp => "Exp",
            Self::Focus => "Focus",
            Self::Fire => "Fire",
            Self::Led => "LED",
            Self::Move => "Move",
            Self::Q => "Q",
            Self::Flags => "Flags",
        }
    }

    /// Width, points.
    fn width(self) -> f32 {
        match self {
            Self::Thumb => 40.0,
            Self::Id => 96.0,
            Self::Time => 66.0,
            Self::Camera => 56.0,
            Self::Flags => 72.0,
            Self::Exp | Self::Focus | Self::Fire | Self::Led | Self::Move => 40.0,
            Self::Q => 34.0,
        }
    }

    /// Lower keeps its place longer when space runs out.
    fn priority(self) -> u8 {
        match self {
            Self::Thumb | Self::Id => 0,
            Self::Q => 1,
            Self::Fire => 2,
            Self::Flags => 3,
            Self::Led => 4,
            Self::Move => 5,
            Self::Time => 6,
            Self::Focus => 7,
            Self::Camera => 8,
            Self::Exp => 9,
        }
    }

    /// The order clicking the header picks.
    fn sort(self) -> Option<Sort> {
        match self {
            Self::Id | Self::Time => Some(Sort::Time),
            Self::Fire => Some(Sort::Fire),
            Self::Move => Some(Sort::Motion),
            Self::Q => Some(Sort::Quality),
            Self::Thumb | Self::Camera | Self::Exp | Self::Focus | Self::Led | Self::Flags => None,
        }
    }
}

/// The columns that fit in `width`, left to right.
fn fitting(width: f32) -> Vec<Col> {
    let mut by_priority = Col::ALL.to_vec();
    by_priority.sort_by_key(|c| c.priority());
    let mut used = CAPTION_MIN;
    let mut keep = Vec::new();
    for c in by_priority {
        if used + c.width() <= width || c.priority() == 0 {
            used += c.width();
            keep.push(c);
        }
    }
    Col::ALL.into_iter().filter(|c| keep.contains(c)).collect()
}

/// Draws the list.
pub(super) fn draw(
    ui: &mut egui::Ui,
    paint: &Paint<'_>,
    thumbs: &mut Thumbs,
    reveal: Option<usize>,
    scroll: (f32, f32),
) -> Drawn {
    let theme = paint.theme;
    let bar = super::scroll_bar_width(ui);
    let full_w = super::visible_width(ui);
    let inner_w = (full_w - bar).max(120.0);
    // Fire and LED columns only for a template that measures them.
    let bars = paint.night.bars;
    let cols: Vec<Col> = fitting(inner_w - PAD_X * 2.0)
        .into_iter()
        .filter(|c| match c {
            Col::Fire => bars.contains(&crate::search::Metric::Fire),
            Col::Led => bars.contains(&crate::search::Metric::Led),
            Col::Thumb
            | Col::Id
            | Col::Time
            | Col::Camera
            | Col::Exp
            | Col::Focus
            | Col::Move
            | Col::Q
            | Col::Flags => true,
        })
        .collect();
    let mut sort = None;

    // Column header: 28 px, a hairline under it.
    let (head, _) = ui.allocate_exact_size(vec2(full_w, ROW_H), Sense::hover());
    let mut x = head.left() + PAD_X;
    let title_font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    for c in &cols {
        let r = Rect::from_min_size(pos2(x, head.top()), vec2(c.width(), head.height()));
        let mut color = theme.text_muted;
        if let Some(order) = c.sort() {
            let resp = ui
                .interact(
                    r,
                    egui::Id::unique(("review-col", c.title())),
                    Sense::click(),
                )
                .hint(format!("Sort by {}", order.label().to_lowercase()));
            if resp.hovered() {
                color = theme.text;
            }
            if resp.clicked() {
                sort = Some(order);
            }
        }
        super::text_in(
            ui.painter(),
            pos2(r.left() + CELL_PAD, r.center().y),
            c.title(),
            title_font.clone(),
            theme::rgb(color),
            c.width() - CELL_PAD,
        );
        x += c.width();
    }
    super::text_in(
        ui.painter(),
        pos2(x + CELL_PAD, head.center().y),
        "Caption",
        title_font,
        theme::rgb(theme.text_muted),
        CAPTION_MIN,
    );
    ui.painter()
        .hline(head.x_range(), head.bottom() - 0.5, theme::ink_line(theme));

    let n = paint.items.len();
    // Every row (an act's header or a result) is one 28 px list row, so
    // the list can be MaterialList's virtualised rows: uniform, no gaps.
    let layout = acts::layout(
        paint.acts,
        n,
        1,
        acts::Metrics {
            header: ROW_H,
            header_gap: 0.0,
            header_lead: 0.0,
            tiles: ROW_H,
            tiles_gap: 0.0,
        },
    );
    let mut area = egui::ScrollArea::vertical()
        .id_salt("review-list")
        .auto_shrink([false, false]);
    if let Some(r) = reveal.and_then(|i| layout.row_of(i)) {
        let (top, h) = layout.span(r);
        if let Some(off) = super::reveal_offset(top, top + h, scroll) {
            area = area.vertical_scroll_offset(off);
        }
    }
    let thumbs = RefCell::new(thumbs);
    let headers = RefCell::new((Vec::new(), None));
    let mut responses = Vec::new();
    ui.add_space(SPACE_XS);
    let out = area.show_viewport(ui, |ui, viewport| {
        ui.set_width(inner_w);
        let (thumbs, headers, cols) = (&thumbs, &headers, cols.as_slice());
        let shown = MaterialList::new()
            .id(egui::Id::unique("review-list-rows"))
            .dividers(false)
            .show_rows(ui, viewport, layout.rows.len(), ROW_H, |k| {
                list_row(paint, layout.rows.get(k).copied(), thumbs, headers, cols)
            });
        for (k, response) in shown.rows {
            if let Some(Row::Tiles { start, .. }) = layout.rows.get(k)
                && let Some(item) = paint.items.get(*start)
            {
                responses.push((*start, super::tooltip_on(response, paint, item)));
            }
        }
    });
    let (folds, scope) = headers.into_inner();
    Drawn {
        responses,
        hover_scrub: None,
        scroll_y: out.state.clamped_offset().y,
        view_h: out.inner_rect.height(),
        columns: 1,
        toggles: Vec::new(),
        refers: Vec::new(),
        sort,
        folds,
        scope,
    }
}

/// What an act header clicked this frame: the acts to fold or unfold, and
/// the act to scope the search to.
type Headers = RefCell<(Vec<usize>, Option<usize>)>;

/// List row `kind`: an act's header, or a result painted by [`row`] over a
/// Material list item (which gives it its hover, selection and focus).
fn list_row<'a>(
    paint: &'a Paint<'a>,
    kind: Option<Row>,
    thumbs: &'a RefCell<&mut Thumbs>,
    headers: &'a Headers,
    cols: &'a [Col],
) -> ListItem<'a> {
    let theme = paint.theme;
    match kind {
        Some(Row::Header(a)) => {
            let Some(act) = paint.acts.get(a) else {
                return ListItem::new("").enabled(false);
            };
            ListItem::new(act.label()).content_ui(move |ui, state| {
                let head = acts::header(ui, paint, act, state.rect);
                let mut headers = headers.borrow_mut();
                if head.fold {
                    headers.0.push(act.n);
                }
                if head.scope {
                    headers.1 = Some(act.n);
                }
            })
        }
        Some(Row::Tiles { start, .. }) => {
            let (Some(item), Some(s)) = (
                paint.items.get(start),
                paint
                    .items
                    .get(start)
                    .and_then(|item| paint.night.index.segments.get(item.seg)),
            ) else {
                return ListItem::new("").enabled(false);
            };
            ListItem::new(s.id.clone())
                .dense(true)
                .selected(paint.selected.contains(&s.id))
                .selected_tile_color(theme::selected_row(theme))
                .selected_color(theme::rgb(theme.text))
                .content_ui(move |ui, state| {
                    row(ui, paint, &mut thumbs.borrow_mut(), item, state.rect, cols);
                })
        }
        None => ListItem::new("").enabled(false),
    }
}
/// Paints one result's cells over its list row `rect`: Selects' list row,
/// a 16 px thumbnail, the name in 14 px and the numbers in 12 px tabular.
fn row(
    ui: &mut egui::Ui,
    paint: &Paint<'_>,
    thumbs: &mut Thumbs,
    item: &Item,
    rect: Rect,
    cols: &[Col],
) {
    let theme = paint.theme;
    let Some(s) = paint.night.index.segments.get(item.seg) else {
        return;
    };
    let mark = paint.night.marks.get(&s.id);
    let painter = ui.painter_at(rect);
    let rejected = mark.verdict == Some(Verdict::Reject);
    let ink = if rejected {
        theme::rgb(theme.text_muted)
    } else {
        theme::rgb(theme.text)
    };
    let muted = theme::rgb(theme.text_muted);
    let numbers = theme::tabular(ui.ctx(), theme::BODY_SIZE);
    let cy = rect.center().y;
    let values = super::bar_values(paint.night, s);
    let mut x = rect.left() + PAD_X;
    for c in cols {
        let w = c.width();
        let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(w, ROW_H));
        let left = pos2(cell.left() + CELL_PAD, cy);
        let number = |text: &str, color: egui::Color32| {
            super::text_in(&painter, left, text, numbers.clone(), color, w - CELL_PAD);
        };
        match c {
            Col::Thumb => {
                let thumb = Rect::from_min_size(
                    pos2(cell.left() + CELL_PAD, cy - THUMB_H / 2.0),
                    vec2(THUMB_H * 16.0 / 9.0, THUMB_H),
                );
                match super::source(paint.paths, &s.clip) {
                    Some(path) => thumbs.paint(
                        ui,
                        thumb,
                        path,
                        f64::from((s.seg.start + s.seg.end) / 2.0),
                        s.seg.cx,
                        theme::rgb(theme.raised),
                    ),
                    None => {
                        painter.rect_filled(
                            thumb,
                            CornerRadius::same(theme::RADIUS_XS),
                            theme::rgb(theme.raised),
                        );
                    }
                }
                if rejected {
                    painter.rect_filled(
                        thumb,
                        CornerRadius::same(theme::RADIUS_XS),
                        egui::Color32::from_black_alpha(150),
                    );
                }
                painter.rect_stroke(
                    thumb,
                    CornerRadius::same(theme::RADIUS_XS),
                    theme::ink_line(theme),
                    StrokeKind::Inside,
                );
                if paint.playing == Some(s.id.as_str()) {
                    super::play_mark(
                        &painter,
                        theme,
                        thumb.center(),
                        THUMB_H / 2.0,
                        theme::rgb(theme.success),
                    );
                }
            }
            Col::Id => {
                let mut x0 = cell.left() + CELL_PAD;
                if item.stack == Stack::Member {
                    x0 += SPACE_SM;
                }
                if let Some(v) = mark.verdict {
                    let (color, glyph) = super::verdict_style(theme, v);
                    icons::paint(
                        &painter,
                        Rect::from_min_size(
                            pos2(x0, cy - icons::SMALL / 2.0),
                            vec2(icons::SMALL, icons::SMALL),
                        ),
                        glyph,
                        icons::SMALL,
                        color,
                    );
                    x0 += icons::SMALL + SPACE_XS;
                }
                let tw = super::name_in(
                    &painter,
                    pos2(x0, cy),
                    &s.id,
                    theme::font(theme::LARGE_SIZE),
                    ink,
                    cell.right() - x0 - 2.0,
                );
                let mut after = x0 + tw + SPACE_XS;
                if let Stack::Top { count, open } = item.stack
                    && after + 20.0 < cell.right()
                {
                    let label = if open {
                        "−".to_owned()
                    } else {
                        format!("+{}", count - 1)
                    };
                    after += super::text_in(
                        &painter,
                        pos2(after, cy),
                        &label,
                        numbers.clone(),
                        muted,
                        cell.right() - after,
                    ) + SPACE_XS;
                }
                if mark.rating > 0 && after + 22.0 < cell.right() {
                    let star = Rect::from_min_size(
                        pos2(after, cy - SPACE_SM * 0.75),
                        vec2(SPACE_MD, SPACE_MD),
                    );
                    icons::paint(
                        &painter,
                        star,
                        icons::STAR,
                        SPACE_MD,
                        theme::rgb(theme.warning),
                    );
                    super::text_in(
                        &painter,
                        pos2(star.right() + 2.0, cy),
                        &mark.rating.to_string(),
                        numbers.clone(),
                        muted,
                        cell.right() - star.right() - 2.0,
                    );
                }
            }
            Col::Time => number(&format!("{:.1}–{:.1}", s.seg.start, s.seg.end), muted),
            Col::Camera => {
                super::text_line(
                    &painter,
                    left,
                    s.seg.camera.label(),
                    theme::BODY_SIZE,
                    muted,
                    w - CELL_PAD,
                );
            }
            Col::Exp => number(&format!("{:.2}", s.seg.p95), muted),
            Col::Focus => {
                let f = paint.night.metric(s, Metric::Focus);
                let color = if f < 0.6 {
                    theme::rgb(theme.warning)
                } else {
                    muted
                };
                number(&format!("{f:.2}"), color);
            }
            Col::Fire | Col::Led | Col::Move => {
                let k = match c {
                    Col::Fire => 0,
                    Col::Led => 1,
                    Col::Thumb
                    | Col::Id
                    | Col::Time
                    | Col::Camera
                    | Col::Exp
                    | Col::Focus
                    | Col::Move
                    | Col::Q
                    | Col::Flags => 2,
                };
                let v = values.get(k).copied().unwrap_or(0.0);
                number(&format!("{v:.2}"), muted);
            }
            Col::Q => number(&format!("{:.2}", s.q), super::grid::q_color(theme, s.q)),
            Col::Flags => {
                super::text_line(
                    &painter,
                    left,
                    &s.flags.join(" "),
                    theme::SMALL_SIZE,
                    theme::rgb(theme.warning),
                    w - CELL_PAD,
                );
            }
        }
        x += w;
    }
    let text = if item.reason.is_empty() {
        s.caption
            .as_ref()
            .map_or("", |c| c.summary.as_str())
            .to_owned()
    } else {
        item.reason.clone()
    };
    super::text_line(
        &painter,
        pos2(x + CELL_PAD, cy),
        &text,
        theme::BODY_SIZE,
        muted,
        rect.right() - x - CELL_PAD - PAD_X,
    );
}

#[cfg(test)]
mod tests {
    use super::{CAPTION_MIN, Col, fitting};

    #[test]
    fn narrow_panes_drop_the_least_useful_columns_first() {
        let wide = fitting(2000.0);
        assert_eq!(wide.len(), Col::ALL.len(), "everything fits: {wide:?}");
        let narrow =
            fitting(Col::Thumb.width() + Col::Id.width() + Col::Q.width() + CAPTION_MIN + 10.0);
        assert_eq!(
            narrow,
            vec![Col::Thumb, Col::Id, Col::Q],
            "thumb, id and quality stay"
        );
        assert!(
            fitting(10.0).contains(&Col::Id),
            "the id never goes, however narrow"
        );
    }
}

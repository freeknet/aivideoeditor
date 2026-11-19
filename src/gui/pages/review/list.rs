// ============================================================================
// Module:       gui::pages::review::list
// Description:  The Shots tab's list: dense, virtualised rows of the shot-log
//               numbers, columns dropped by priority when the pane is narrow.
//
// Dependencies: egui; super (Paint, Item, Drawn, Sort), crate::gui::thumbs
// ============================================================================

//! The list view: one row per result with the numbers the shot log prints.
//! Columns that do not fit are dropped, least useful first; the caption
//! takes whatever width is left. Clicking a sortable header sorts by it.

use egui::{Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use super::{Drawn, Item, Paint, Sort, Stack};
use crate::gui::thumbs::Thumbs;
use crate::gui::ui::theme;
use crate::search::Metric;
use crate::search::marks::Verdict;

/// Row height.
const ROW_H: f32 = 30.0;

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
            Self::Id => "shot",
            Self::Time => "time",
            Self::Camera => "camera",
            Self::Exp => "exp",
            Self::Focus => "focus",
            Self::Fire => "fire",
            Self::Led => "led",
            Self::Move => "move",
            Self::Q => "q",
            Self::Flags => "flags",
        }
    }

    /// Width, points.
    fn width(self) -> f32 {
        match self {
            Self::Thumb => 54.0,
            Self::Id => 92.0,
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
    let s = &ui.spacing().scroll;
    let bar = s.bar_width + s.bar_inner_margin + s.bar_outer_margin;
    let inner_w = (ui.available_width() - bar).max(120.0);
    let cols = fitting(inner_w);
    let mut sort = None;

    // Header.
    let (head, _) = ui.allocate_exact_size(vec2(inner_w, 18.0), Sense::hover());
    let mut x = head.left();
    for c in &cols {
        let r = Rect::from_min_size(pos2(x, head.top()), vec2(c.width(), head.height()));
        let mut color = theme.text_faint;
        if let Some(order) = c.sort() {
            let resp = ui
                .interact(
                    r,
                    egui::Id::unique(("review-col", c.title())),
                    Sense::click(),
                )
                .on_hover_text(format!("Sort by {}", order.label().to_lowercase()));
            if resp.hovered() {
                color = theme.text;
            }
            if resp.clicked() {
                sort = Some(order);
            }
        }
        super::text_line(
            ui.painter(),
            pos2(r.left() + 3.0, r.center().y),
            c.title(),
            10.5,
            theme::rgb(color),
            c.width() - 4.0,
        );
        x += c.width();
    }
    super::text_line(
        ui.painter(),
        pos2(x + 3.0, head.center().y),
        "caption",
        10.5,
        theme::rgb(theme.text_faint),
        CAPTION_MIN,
    );
    ui.painter().hline(
        head.x_range(),
        head.bottom(),
        Stroke::new(1.0, theme::rgb(theme.border)),
    );

    let n = paint.items.len();
    let mut area = egui::ScrollArea::vertical()
        .id_salt("review-list")
        .auto_shrink([false, false]);
    if let Some(r) = reveal {
        let top = r as f32 * ROW_H;
        if let Some(off) = super::reveal_offset(top, top + ROW_H, scroll) {
            area = area.vertical_scroll_offset(off);
        }
    }
    let mut responses = Vec::new();
    ui.spacing_mut().item_spacing.y = 0.0;
    let out = area.show_rows(ui, ROW_H, n, |ui, range| {
        for i in range {
            let (rect, _) = ui.allocate_exact_size(vec2(inner_w, ROW_H), Sense::hover());
            if let Some(item) = paint.items.get(i) {
                responses.push((i, row(ui, paint, thumbs, item, rect, &cols, i)));
            }
        }
    });
    Drawn {
        responses,
        hover_scrub: None,
        scroll_y: out.state.clamped_offset().y,
        view_h: out.inner_rect.height(),
        columns: 1,
        toggles: Vec::new(),
        sort,
    }
}

/// Paints one row.
fn row(
    ui: &mut egui::Ui,
    paint: &Paint<'_>,
    thumbs: &mut Thumbs,
    item: &Item,
    rect: Rect,
    cols: &[Col],
    i: usize,
) -> egui::Response {
    let theme = paint.theme;
    let Some(s) = paint.night.index.segments.get(item.seg) else {
        return ui.interact(
            rect,
            egui::Id::unique(("review-row-missing", i)),
            Sense::hover(),
        );
    };
    let response = ui.interact(
        rect,
        egui::Id::unique(("review-row", &s.id)),
        Sense::click(),
    );
    let selected = paint.selected.contains(&s.id);
    let hovered = response.hovered();
    let mark = paint.night.marks.get(&s.id);
    let painter = ui.painter_at(rect);
    let fill = if selected {
        Some(theme.selection)
    } else if hovered {
        Some(theme.hover)
    } else if i % 2 == 1 {
        Some(theme.raised)
    } else {
        None
    };
    if let Some(f) = fill {
        painter.rect_filled(rect, CornerRadius::ZERO, theme::rgb(f));
    }
    let rejected = mark.verdict == Some(Verdict::Reject);
    let ink = if rejected {
        theme::rgb(theme.text_faint)
    } else {
        theme::rgb(theme.text)
    };
    let muted = theme::rgb(theme.text_muted);
    let cy = rect.center().y;
    let size = 11.0;
    let values = super::bar_values(paint.night, s);
    let colors = super::metric_colors(theme);
    let mut x = rect.left();
    for c in cols {
        let w = c.width();
        let cell = Rect::from_min_size(pos2(x, rect.top()), vec2(w, ROW_H));
        let left = pos2(cell.left() + 3.0, cy);
        match c {
            Col::Thumb => {
                let thumb = Rect::from_center_size(cell.center(), vec2(48.0, 27.0));
                match super::source(paint.paths, &s.clip) {
                    Some(path) => thumbs.paint(
                        ui,
                        thumb,
                        path,
                        f64::from((s.seg.start + s.seg.end) / 2.0),
                        s.seg.cx,
                        theme::rgb(theme.app),
                    ),
                    None => {
                        painter.rect_filled(thumb, CornerRadius::ZERO, theme::rgb(theme.app));
                    }
                }
                if rejected {
                    painter.rect_filled(
                        thumb,
                        CornerRadius::ZERO,
                        egui::Color32::from_black_alpha(150),
                    );
                }
            }
            Col::Id => {
                let mut x0 = cell.left() + 3.0;
                if let Some(v) = mark.verdict {
                    let (color, glyph) = super::verdict_style(theme, v);
                    painter.circle_filled(pos2(x0 + 5.0, cy), 5.0, color);
                    painter.text(
                        pos2(x0 + 5.0, cy),
                        Align2::CENTER_CENTER,
                        glyph,
                        theme::font(8.0),
                        egui::Color32::BLACK,
                    );
                    x0 += 13.0;
                }
                let label = match item.stack {
                    Stack::Top { count, open } => {
                        format!("{} {}{}", s.id, if open { "−" } else { "+" }, count - 1)
                    }
                    Stack::Member => format!("↳ {}", s.id),
                    Stack::Alone => s.id.clone(),
                };
                let tw = super::text_line(
                    &painter,
                    pos2(x0, cy),
                    &label,
                    size,
                    ink,
                    cell.right() - x0 - 2.0,
                );
                if mark.rating > 0 && x0 + tw + 14.0 < cell.right() {
                    super::text_line(
                        &painter,
                        pos2(x0 + tw + 3.0, cy),
                        &format!("{}★", mark.rating),
                        9.5,
                        theme::rgb(theme.warning),
                        cell.right() - x0 - tw - 3.0,
                    );
                }
            }
            Col::Time => {
                super::text_line(
                    &painter,
                    left,
                    &format!("{:.1}–{:.1}", s.seg.start, s.seg.end),
                    size,
                    muted,
                    w - 4.0,
                );
            }
            Col::Camera => {
                super::text_line(&painter, left, s.seg.camera.label(), size, muted, w - 4.0);
            }
            Col::Exp => {
                super::text_line(
                    &painter,
                    left,
                    &format!("{:.2}", s.seg.p95),
                    size,
                    muted,
                    w - 4.0,
                );
            }
            Col::Focus => {
                let f = paint.night.metric(s, Metric::Focus);
                let color = if f < 0.6 {
                    theme::rgb(theme.warning)
                } else {
                    muted
                };
                super::text_line(&painter, left, &format!("{f:.2}"), size, color, w - 4.0);
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
                let color = colors.get(k).copied().unwrap_or(muted);
                let bar = Rect::from_min_size(
                    pos2(cell.left() + 3.0, rect.bottom() - 6.0),
                    vec2((w - 8.0) * v.clamp(0.0, 1.0), 2.0),
                );
                painter.rect_filled(bar, CornerRadius::ZERO, color);
                super::text_line(
                    &painter,
                    pos2(left.x, cy - 2.0),
                    &format!("{v:.2}"),
                    size,
                    ink,
                    w - 4.0,
                );
            }
            Col::Q => {
                super::text_line(
                    &painter,
                    left,
                    &format!("{:.2}", s.q),
                    size,
                    super::grid::q_color(theme, s.q),
                    w - 4.0,
                );
            }
            Col::Flags => {
                super::text_line(
                    &painter,
                    left,
                    &s.flags.join(" "),
                    10.0,
                    theme::rgb(theme.warning),
                    w - 4.0,
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
    let color = if item.reason.is_empty() {
        muted
    } else {
        theme::rgb(theme.accent)
    };
    super::text_line(
        &painter,
        pos2(x + 3.0, cy),
        &text,
        size,
        color,
        rect.right() - x - 6.0,
    );
    if selected {
        painter.rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new(1.0, theme::rgb(theme.accent)),
            StrokeKind::Inside,
        );
    }
    // The tape is playing this one: a bar down the left edge.
    if paint.playing == Some(s.id.as_str()) {
        painter.rect_filled(
            Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
            CornerRadius::ZERO,
            theme::rgb(theme.success),
        );
    }
    response.on_hover_ui(|ui| super::tooltip(ui, paint, item))
}

#[cfg(test)]
mod tests {
    use super::{CAPTION_MIN, Col, fitting};

    #[test]
    fn narrow_panes_drop_the_least_useful_columns_first() {
        let wide = fitting(2000.0);
        assert_eq!(wide.len(), Col::ALL.len(), "everything fits: {wide:?}");
        let narrow = fitting(54.0 + 92.0 + 34.0 + CAPTION_MIN + 10.0);
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

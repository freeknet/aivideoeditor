// ============================================================================
// Module:       gui::pages::review::grid
// Description:  The Shots tab's grid: virtualised rows of thumbnail cards with
//               hover scrub, metric bars and mark / flag / stack badges.
//
// Dependencies: egui; super (Paint, Item, Drawn), crate::gui::thumbs
// ============================================================================

//! Thumbnail cards, painted by hand so a few hundred stay cheap: only the
//! rows in view are drawn, and each card is one interaction rectangle.

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};

use super::{Drawn, Item, Paint, Stack};
use crate::gui::thumbs::Thumbs;
use crate::gui::ui::theme;

/// Gap between cards.
const GAP: f32 = 6.0;

/// The info strip under a thumbnail.
const INFO_H: f32 = 38.0;

/// Positions a hover scrub snaps to across a card, so scrubbing reuses
/// decoded frames.
const SCRUB_STEPS: f32 = 10.0;

/// Draws the grid.
pub(super) fn draw(
    ui: &mut egui::Ui,
    paint: &Paint<'_>,
    thumbs: &mut Thumbs,
    card_w: f32,
    reveal: Option<usize>,
    scroll: (f32, f32),
) -> Drawn {
    let s = &ui.spacing().scroll;
    let bar = s.bar_width + s.bar_inner_margin + s.bar_outer_margin;
    let inner_w = (ui.available_width() - bar).max(60.0);
    let cols = (((inner_w + GAP) / (card_w + GAP)).floor() as usize).max(1);
    let w = (inner_w - GAP * (cols as f32 - 1.0)) / cols as f32;
    let thumb_h = (w * 9.0 / 16.0).round();
    let row_h = thumb_h + INFO_H;
    let n = paint.items.len();
    let rows = n.div_ceil(cols);
    let mut area = egui::ScrollArea::vertical()
        .id_salt("review-grid")
        .auto_shrink([false, false]);
    if let Some(r) = reveal {
        let top = (r / cols) as f32 * (row_h + GAP);
        if let Some(off) = super::reveal_offset(top, top + row_h, scroll) {
            area = area.vertical_scroll_offset(off);
        }
    }
    let mut responses = Vec::new();
    let mut toggles = Vec::new();
    let mut hover_scrub = None;
    ui.spacing_mut().item_spacing.y = GAP;
    let out = area.show_rows(ui, row_h, rows, |ui, range| {
        for row in range {
            let (rect, _) = ui.allocate_exact_size(vec2(inner_w, row_h), Sense::hover());
            for c in 0..cols {
                let i = row * cols + c;
                let Some(item) = paint.items.get(i) else {
                    break;
                };
                let r =
                    Rect::from_min_size(rect.min + vec2(c as f32 * (w + GAP), 0.0), vec2(w, row_h));
                let drawn = card(ui, paint, thumbs, item, r, thumb_h);
                if let Some(t) = drawn.scrub {
                    hover_scrub = Some((i, t));
                }
                if drawn.toggled {
                    toggles.push(i);
                }
                responses.push((i, drawn.response));
            }
        }
    });
    Drawn {
        responses,
        hover_scrub,
        scroll_y: out.state.clamped_offset().y,
        view_h: out.inner_rect.height(),
        columns: cols,
        toggles,
        sort: None,
    }
}

/// What one card hands back.
struct Card {
    response: egui::Response,
    /// Source time under the pointer while scrubbing.
    scrub: Option<f32>,
    /// The stack badge was clicked.
    toggled: bool,
}

/// Paints one card into `rect`.
fn card(
    ui: &mut egui::Ui,
    paint: &Paint<'_>,
    thumbs: &mut Thumbs,
    item: &Item,
    rect: Rect,
    thumb_h: f32,
) -> Card {
    let theme = paint.theme;
    let Some(s) = paint.night.index.segments.get(item.seg) else {
        let response = ui.interact(
            rect,
            egui::Id::unique(("review-card-missing", item.seg)),
            Sense::hover(),
        );
        return Card {
            response,
            scrub: None,
            toggled: false,
        };
    };
    let id = egui::Id::unique(("review-card", &s.id));
    let response = ui.interact(rect, id, Sense::click());
    let selected = paint.selected.contains(&s.id);
    let focused = paint.cursor == Some(s.id.as_str());
    let hovered = response.hovered();
    let mark = paint.night.marks.get(&s.id);
    let painter = ui.painter_at(rect.expand(4.0));
    let thumb = Rect::from_min_size(rect.min, vec2(rect.width(), thumb_h));

    // A closed stack shows the cards behind it.
    if let Stack::Top { open: false, .. } = item.stack {
        for k in [2.0, 1.0] {
            painter.rect_stroke(
                thumb.translate(vec2(2.0 * k, -2.0 * k)),
                CornerRadius::same(2),
                Stroke::new(1.0, theme::rgb(theme.border_strong)),
                StrokeKind::Inside,
            );
        }
    }

    let fill = if selected {
        theme.selection
    } else if hovered {
        theme.hover
    } else {
        theme.raised
    };
    painter.rect_filled(rect, CornerRadius::same(2), theme::rgb(fill));

    // Thumbnail, scrubbed under the pointer.
    let (t0, t1) = (s.seg.start, s.seg.end);
    let mut t = (t0 + t1) / 2.0;
    let mut scrub = None;
    let mut scrub_x = None;
    if paint.scrub
        && hovered
        && let Some(p) = response.hover_pos()
        && thumb.contains(p)
    {
        let frac = ((p.x - thumb.left()) / thumb.width()).clamp(0.0, 0.999);
        let snapped = ((frac * SCRUB_STEPS).floor() + 0.5) / SCRUB_STEPS;
        t = t0 + (t1 - t0) * snapped;
        scrub = Some(t);
        scrub_x = Some(p.x);
    }
    match super::source(paint.paths, &s.clip) {
        Some(path) => thumbs.paint(
            ui,
            thumb,
            path,
            f64::from(t),
            s.seg.cx,
            theme::rgb(theme.app),
        ),
        None => {
            painter.rect_filled(thumb, CornerRadius::ZERO, theme::rgb(theme.app));
        }
    }
    if mark.verdict == Some(crate::search::marks::Verdict::Reject) {
        painter.rect_filled(thumb, CornerRadius::ZERO, Color32::from_black_alpha(150));
    }
    let scrim = Color32::from_black_alpha(170);
    let tiny = 10.0;
    if let Some(x) = scrub_x {
        let p = (x - thumb.left()) / thumb.width();
        painter.rect_filled(
            Rect::from_min_size(
                pos2(thumb.left(), thumb.bottom() - 2.0),
                vec2(thumb.width() * p, 2.0),
            ),
            CornerRadius::ZERO,
            theme::rgb(theme.accent),
        );
    }

    // Duration, bottom right.
    let dur = format!("{:.1}s", t1 - t0);
    badge(
        &painter,
        thumb.right_bottom() + vec2(-3.0, -4.0),
        Align2::RIGHT_BOTTOM,
        &dur,
        tiny,
        Color32::WHITE,
        scrim,
    );

    // Flags, bottom left.
    let mut x = thumb.left() + 3.0;
    for f in s.flags.iter().take(3) {
        let w = badge(
            &painter,
            pos2(x, thumb.bottom() - 4.0),
            Align2::LEFT_BOTTOM,
            f,
            tiny - 1.0,
            theme::rgb(theme.warning),
            scrim,
        );
        x += w + 3.0;
        if x > thumb.right() - 40.0 {
            break;
        }
    }

    // Verdict, top left.
    if let Some(v) = mark.verdict {
        let (color, glyph) = super::verdict_style(theme, v);
        let c = thumb.left_top() + vec2(10.0, 10.0);
        painter.circle_filled(c, 7.0, color);
        painter.text(
            c,
            Align2::CENTER_CENTER,
            glyph,
            theme::font(10.0),
            Color32::BLACK,
        );
    }

    // Rating and stack, top right.
    let mut right = thumb.right() - 3.0;
    let mut toggled = false;
    if let Stack::Top { count, open } = item.stack {
        let label = if open {
            "−".to_owned()
        } else {
            format!("+{}", count - 1)
        };
        let w = badge(
            &painter,
            pos2(right, thumb.top() + 3.0),
            Align2::RIGHT_TOP,
            &label,
            tiny + 1.0,
            Color32::WHITE,
            theme::translucent(theme.accent_soft, 230),
        );
        let hit = Rect::from_min_max(
            pos2(right - w - 2.0, thumb.top()),
            pos2(right + 2.0, thumb.top() + 20.0),
        );
        let b = ui
            .interact(hit, id.with("stack"), Sense::click())
            .on_hover_text(if open {
                "Collapse this clip's shots"
            } else {
                "Show this clip's other shots"
            });
        toggled = b.clicked();
        right -= w + 4.0;
    }
    if mark.rating > 0 {
        let stars = "★".repeat(usize::from(mark.rating));
        badge(
            &painter,
            pos2(right, thumb.top() + 3.0),
            Align2::RIGHT_TOP,
            &stars,
            tiny,
            theme::rgb(theme.warning),
            scrim,
        );
    }

    // Info strip.
    let info = Rect::from_min_max(pos2(rect.left(), thumb.bottom()), rect.max);
    let pad = 5.0;
    let line1 = info.top() + 8.0;
    let q = format!("{:.2}", s.q);
    let qw = super::text_line(
        &painter,
        pos2(info.right() - pad - 26.0, line1),
        &q,
        11.0,
        q_color(theme, s.q),
        26.0,
    );
    let _ = qw;
    let name = if item.stack == Stack::Member {
        format!("  ↳ {}", s.id)
    } else {
        s.id.clone()
    };
    super::text_line(
        &painter,
        pos2(info.left() + pad, line1),
        &name,
        11.0,
        theme::rgb(theme.text),
        info.width() - pad * 2.0 - 30.0,
    );
    // Metric bars.
    let bars_y = info.top() + 16.0;
    let colors = super::metric_colors(theme);
    let values = super::bar_values(paint.night, s);
    let bw = (info.width() - pad * 2.0 - 3.0 * 3.0) / 4.0;
    for (k, (v, c)) in values.iter().zip(colors.iter()).enumerate() {
        let x0 = info.left() + pad + k as f32 * (bw + 3.0);
        let track = Rect::from_min_size(pos2(x0, bars_y), vec2(bw, 3.0));
        painter.rect_filled(track, CornerRadius::ZERO, theme::rgb(theme.hover));
        painter.rect_filled(
            Rect::from_min_size(track.min, vec2(bw * v.clamp(0.0, 1.0), 3.0)),
            CornerRadius::ZERO,
            *c,
        );
    }
    // Caption (or, while searching, why it matched).
    let line2 = info.top() + 29.0;
    let text = if item.reason.is_empty() {
        s.caption.as_ref().map_or_else(
            || format!("{} · {}", s.subject, s.seg.camera.label()),
            |c| c.summary.clone(),
        )
    } else {
        item.reason.clone()
    };
    let color = if item.reason.is_empty() {
        theme.text_muted
    } else {
        theme.accent
    };
    super::text_line(
        &painter,
        pos2(info.left() + pad, line2),
        &text,
        10.5,
        theme::rgb(color),
        info.width() - pad * 2.0,
    );

    // Border: accent when selected, stronger under the pointer.
    let (w, c) = if selected {
        (2.0, theme.accent)
    } else if focused || hovered {
        (1.0, theme.border_strong)
    } else {
        (1.0, theme.border)
    };
    painter.rect_stroke(
        rect,
        CornerRadius::same(2),
        Stroke::new(w, theme::rgb(c)),
        StrokeKind::Inside,
    );
    let response = response.on_hover_ui(|ui| super::tooltip(ui, paint, item));
    Card {
        response,
        scrub,
        toggled,
    }
}

/// A small label on a dark plate anchored at `pos`. Returns its width.
fn badge(
    painter: &egui::Painter,
    pos: egui::Pos2,
    anchor: Align2,
    text: &str,
    size: f32,
    color: Color32,
    plate: Color32,
) -> f32 {
    let galley = painter.layout_no_wrap(text.to_owned(), theme::font(size), color);
    let size = galley.size() + Vec2::new(6.0, 2.0);
    let rect = anchor.anchor_size(pos, size);
    painter.rect_filled(rect, CornerRadius::same(2), plate);
    painter.galley(rect.min + vec2(3.0, 1.0), galley, color);
    size.x
}

/// Quality in a colour: good green, middling plain, poor red.
pub(super) fn q_color(theme: &crate::theme::Palette, q: f32) -> Color32 {
    if q >= 0.6 {
        theme::rgb(theme.success)
    } else if q >= 0.3 {
        theme::rgb(theme.text)
    } else {
        theme::rgb(theme.danger)
    }
}

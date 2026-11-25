// ============================================================================
// Module:       gui::pages::review::grid
// Description:  The Shots tab's grid: virtualised rows of Selects-style media
//               cards with hover scrub and glass mark / rating / stack badges.
//
// Dependencies: egui; super (Paint, Item, Drawn), crate::gui::thumbs
// ============================================================================

//! Thumbnail cards, painted by hand so a few hundred stay cheap: only the
//! rows in view are drawn, and each card is one interaction rectangle.
//!
//! A card is Selects' media card: a 16:9 thumbnail with 2 px corners, a
//! glass duration badge in its corner, the name under it in 12 px Medium
//! and one muted line of caption. Selection is a 2 px outline 2 px clear
//! of the thumbnail, in the key colour while the tab has the keyboard and
//! grey otherwise; hover is an ink wash. The measures live in the hover
//! card, not on the card.
//!
//! Grouped by act, a header row across the grid opens each act (see
//! [`acts`]); rows break at an act's end.

use crate::gui::ui::hint::Hint as _;
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use super::acts::{self, Row};
use super::{Drawn, Item, Paint, Stack};
use crate::gui::thumbs::Thumbs;
use crate::gui::ui::icons;
use crate::gui::ui::theme::{self, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::search::marks::Verdict;

/// Gap between columns (Selects' `gap-x-3`).
const GAP_X: f32 = SPACE_MD;

/// Gap between rows (Selects' `gap-y-4`).
const GAP_Y: f32 = SPACE_LG;

/// Room kept round the grid for the selection outline (2 px clear of the
/// thumbnail, 2 px wide).
const OUTLINE_ROOM: f32 = SPACE_XS;

/// Height of the name line (12 px at a 1.25 line height).
const NAME_H: f32 = 15.0;

/// Height of the caption line.
const CAPTION_H: f32 = 14.0;

/// The text under a thumbnail: a 4 px gap, the name, the caption.
const INFO_H: f32 = SPACE_XS + NAME_H + 2.0 + CAPTION_H;

/// Positions a hover scrub snaps to across a card, so scrubbing reuses
/// decoded frames.
const SCRUB_STEPS: f32 = 10.0;

/// Columns for a grid `inner` points wide whose cards are at most `card`
/// wide: Selects' `max(1, ceil((width + gap) / (card + gap)))`, so cards
/// never grow past `card` and stretch to fill the row.
fn columns(inner: f32, card: f32) -> usize {
    (((inner + GAP_X) / (card + GAP_X)).ceil() as usize).max(1)
}

/// Draws the grid.
pub(super) fn draw(
    ui: &mut egui::Ui,
    paint: &Paint<'_>,
    thumbs: &mut Thumbs,
    card_w: f32,
    reveal: Option<usize>,
    scroll: (f32, f32),
) -> Drawn {
    let bar = super::scroll_bar_width(ui);
    let inner_w = (super::visible_width(ui) - bar - OUTLINE_ROOM * 2.0).max(60.0);
    let cols = columns(inner_w, card_w);
    let w = (inner_w - GAP_X * (cols as f32 - 1.0)) / cols as f32;
    let thumb_h = (w * 9.0 / 16.0).round();
    let row_h = thumb_h + INFO_H;
    let n = paint.items.len();
    let layout = acts::layout(
        paint.acts,
        n,
        cols,
        acts::Metrics {
            header: theme::CONTROL_HEIGHT,
            header_gap: SPACE_SM,
            header_lead: SPACE_XS,
            tiles: row_h,
            tiles_gap: GAP_Y,
        },
    );
    let mut area = egui::ScrollArea::vertical()
        .id_salt("review-grid")
        .auto_shrink([false, false]);
    if let Some(r) = reveal.and_then(|i| layout.row_of(i)) {
        let (top, h) = layout.span(r);
        if let Some(off) = super::reveal_offset(top, top + h, scroll) {
            area = area.vertical_scroll_offset(off);
        }
    }
    let mut responses = Vec::new();
    let mut toggles = Vec::new();
    let mut refers = Vec::new();
    let mut folds = Vec::new();
    let mut scope = None;
    let mut hover_scrub = None;
    ui.add_space(SPACE_SM);
    let full_w = inner_w + OUTLINE_ROOM * 2.0;
    let out = area.show_viewport(ui, |ui, viewport| {
        let origin = ui.max_rect().min;
        ui.set_min_size(vec2(full_w, layout.total));
        for k in layout.visible(viewport.min.y, viewport.max.y) {
            let (top, h) = layout.span(k);
            let rect = Rect::from_min_size(origin + vec2(0.0, top), vec2(full_w, h));
            match layout.rows.get(k) {
                Some(Row::Header(a)) => {
                    let Some(act) = paint.acts.get(*a) else {
                        continue;
                    };
                    let head = acts::header(ui, paint, act, rect.shrink2(vec2(OUTLINE_ROOM, 0.0)));
                    if head.fold {
                        folds.push(act.n);
                    }
                    if head.scope {
                        scope = Some(act.n);
                    }
                }
                Some(Row::Tiles { start, end }) => {
                    for i in *start..*end {
                        let Some(item) = paint.items.get(i) else {
                            break;
                        };
                        let c = i - start;
                        let r = Rect::from_min_size(
                            rect.min + vec2(OUTLINE_ROOM + c as f32 * (w + GAP_X), 0.0),
                            vec2(w, row_h),
                        );
                        let drawn = card(ui, paint, thumbs, item, r, thumb_h);
                        if let Some(t) = drawn.scrub {
                            hover_scrub = Some((i, t));
                        }
                        if drawn.toggled {
                            toggles.push(i);
                        }
                        if drawn.refer {
                            refers.push(i);
                        }
                        responses.push((i, drawn.response));
                    }
                }
                None => {}
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
        refers,
        sort: None,
        folds,
        scope,
    }
}

/// What one card hands back.
struct Card {
    response: egui::Response,
    /// Source time under the pointer while scrubbing.
    scrub: Option<f32>,
    /// The stack badge was clicked.
    toggled: bool,
    /// The @ badge was clicked.
    refer: bool,
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
            refer: false,
        };
    };
    let id = egui::Id::unique(("review-card", &s.id));
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &s.id));
    let selected = paint.selected.contains(&s.id);
    let focused = paint.cursor == Some(s.id.as_str());
    let hovered = response.hovered();
    let mark = paint.night.marks.get(&s.id);
    if hovered {
        let flags = if s.flags.is_empty() {
            String::new()
        } else {
            format!(" · flagged: {}", s.flags.join(", "))
        };
        crate::gui::ui::hint::set(
            ui.ctx(),
            &format!(
                "{} · {:.1} s from {:.1} s · quality {:.2} (focus, exposure, steadiness){flags} · move across it to scrub, double-click to play, right-click for more",
                s.id,
                s.seg.end - s.seg.start,
                s.seg.start,
                s.q
            ),
            "",
        );
    }
    let painter = ui.painter_at(rect.expand(OUTLINE_ROOM + 2.0));
    let thumb = Rect::from_min_size(rect.min, vec2(rect.width(), thumb_h));
    let radius = CornerRadius::same(theme::RADIUS_XS);

    // Hover: an ink wash under the whole card.
    if hovered {
        painter.rect_filled(
            rect.expand(OUTLINE_ROOM),
            CornerRadius::same(theme::RADIUS),
            theme::ink(theme, theme::INK_HOVER),
        );
    }

    // A closed stack shows the cards behind it.
    if let Stack::Top { open: false, .. } = item.stack {
        for k in [2.0, 1.0] {
            painter.rect_filled(
                thumb
                    .translate(vec2(0.0, -2.0 * k))
                    .shrink2(vec2(4.0 * k, 0.0)),
                radius,
                theme::ink(
                    theme,
                    if k > 1.5 {
                        theme::INK_08
                    } else {
                        theme::INK_15
                    },
                ),
            );
        }
    }

    // Thumbnail, scrubbed under the pointer.
    painter.rect_filled(thumb, radius, theme::rgb(theme.raised));
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
            theme::rgb(theme.raised),
        ),
        None => {
            icons::paint(
                &painter,
                thumb,
                icons::FILM,
                icons::SIZE + 4.0,
                theme::rgb(theme.text_muted),
            );
        }
    }
    if mark.verdict == Some(Verdict::Reject) {
        painter.rect_filled(thumb, radius, Color32::from_black_alpha(150));
    }
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

    // Badges, all glass, 4 px in from the corners.
    let inset = SPACE_XS;
    // Over the card at all, badges included (the badge under the pointer is
    // what egui calls hovered then, not the card).
    let over = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|p| rect.contains(p));
    let (toggled, refer) = badges(ui, &painter, paint, item, s, thumb, inset, id, over);

    // Name, with the quality at the far end, then one muted line.
    let name_y = thumb.bottom() + SPACE_XS + NAME_H / 2.0;
    let q = format!("{:.2}", s.q);
    let q_font = theme::tabular(ui.ctx(), theme::SMALL_SIZE);
    let q_w = painter
        .layout_no_wrap(q.clone(), q_font.clone(), Color32::PLACEHOLDER)
        .size()
        .x;
    super::text_in(
        &painter,
        pos2(rect.right() - 2.0 - q_w, name_y),
        &q,
        q_font,
        theme::rgb(theme.text_faint),
        q_w + 2.0,
    );
    let indent = if item.stack == Stack::Member {
        SPACE_SM + 2.0
    } else {
        2.0
    };
    let rejected = mark.verdict == Some(Verdict::Reject);
    super::name_in(
        &painter,
        pos2(rect.left() + indent, name_y),
        &s.id,
        theme::medium(ui.ctx(), theme::BODY_SIZE),
        theme::rgb(if rejected {
            theme.text_muted
        } else {
            theme.text
        }),
        rect.width() - indent - q_w - SPACE_SM,
    );
    let caption_y = name_y + NAME_H / 2.0 + 2.0 + CAPTION_H / 2.0;
    let text = if item.reason.is_empty() {
        s.caption
            .as_ref()
            .map_or_else(|| super::kind_line(paint.night, s), |c| c.summary.clone())
    } else {
        item.reason.clone()
    };
    super::text_line(
        &painter,
        pos2(rect.left() + indent, caption_y),
        &text,
        theme::SMALL_SIZE,
        theme::rgb(theme.text_muted),
        rect.width() - indent - 2.0,
    );

    // Selection: 2 px, 2 px clear of the thumbnail.
    let ring = thumb.expand(2.0);
    if paint.playing == Some(s.id.as_str()) {
        painter.rect_stroke(
            ring,
            CornerRadius::same(theme::RADIUS_XS + 2),
            Stroke::new(2.0, theme::rgb(theme.success)),
            StrokeKind::Outside,
        );
        super::play_mark(
            &painter,
            theme,
            thumb.center(),
            12.0,
            theme::rgb(theme.success),
        );
    } else if selected {
        let color = if paint.focused {
            theme::rgb(theme.accent)
        } else {
            theme::rgb(theme.text_muted)
        };
        painter.rect_stroke(
            ring,
            CornerRadius::same(theme::RADIUS_XS + 2),
            Stroke::new(2.0, color),
            StrokeKind::Outside,
        );
    } else if focused {
        painter.rect_stroke(
            ring,
            CornerRadius::same(theme::RADIUS_XS + 2),
            Stroke::new(2.0, theme::ink(theme, theme::INK_20)),
            StrokeKind::Outside,
        );
    }
    let response = response.on_hover_ui(|ui| super::tooltip(ui, paint, item));
    Card {
        response,
        scrub,
        toggled,
        refer,
    }
}

/// The glass badges on a thumbnail: the mark top left, the stack and the
/// rating top right, flags bottom left, the length bottom right. Returns
/// whether the stack badge was clicked.
#[expect(
    clippy::too_many_arguments,
    reason = "one card's painting context, split out of `card` only for length"
)]
fn badges(
    ui: &egui::Ui,
    painter: &egui::Painter,
    paint: &Paint<'_>,
    item: &Item,
    s: &crate::index::Scored,
    thumb: Rect,
    inset: f32,
    id: egui::Id,
    hovered: bool,
) -> (bool, bool) {
    let theme = paint.theme;
    let mark = paint.night.marks.get(&s.id);
    super::glass_badge(
        painter,
        theme,
        thumb.right_bottom() + vec2(-inset, -inset),
        Align2::RIGHT_BOTTOM,
        None,
        &super::duration(s.seg.end - s.seg.start),
    );
    if !s.flags.is_empty() {
        super::glass_badge(
            painter,
            theme,
            thumb.left_bottom() + vec2(inset, -inset),
            Align2::LEFT_BOTTOM,
            Some((icons::TRIANGLE_ALERT, theme::rgb(theme.warning))),
            "",
        );
    }
    if let Some(v) = mark.verdict {
        let (color, glyph) = super::verdict_style(theme, v);
        super::glass_badge(
            painter,
            theme,
            thumb.left_top() + vec2(inset, inset),
            Align2::LEFT_TOP,
            Some((glyph, color)),
            "",
        );
    }
    let mut right = thumb.right() - inset;
    let mut toggled = false;
    if let Stack::Top { count, open } = item.stack {
        let label = if open {
            String::new()
        } else {
            (count - 1).to_string()
        };
        let glyph = if open { icons::MINUS } else { icons::LAYERS };
        let r = super::glass_badge(
            painter,
            theme,
            pos2(right, thumb.top() + inset),
            Align2::RIGHT_TOP,
            Some((glyph, Color32::WHITE)),
            &label,
        );
        let b = ui
            .interact(r.expand(2.0), id.with("stack"), Sense::click())
            .hint(if open {
                "Collapse this clip's shots"
            } else {
                "Show this clip's other shots"
            });
        toggled = b.clicked();
        right = r.left() - SPACE_XS;
    }
    if mark.rating > 0 {
        super::glass_badge(
            painter,
            theme,
            pos2(right, thumb.top() + inset),
            Align2::RIGHT_TOP,
            Some((icons::STAR, theme::rgb(theme.warning))),
            &mark.rating.to_string(),
        );
    }
    // On hover: @ adds this shot to the chat message, in one click.
    let mut refer = false;
    if hovered {
        let r = super::glass_badge(
            painter,
            theme,
            pos2(right, thumb.top() + inset),
            Align2::RIGHT_TOP,
            Some((icons::AT_SIGN, Color32::WHITE)),
            "",
        );
        let b = ui.interact(r.expand(2.0), id.with("refer"), Sense::click());
        b.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Add to chat"));
        refer = b
            .hint("Add this shot to the chat message (or select shots and press R)")
            .clicked();
    }
    (toggled, refer)
}

/// Quality in a colour: good plain, middling muted, poor red.
pub(super) fn q_color(theme: &crate::theme::Palette, q: f32) -> Color32 {
    if q >= 0.6 {
        theme::rgb(theme.text)
    } else if q >= 0.3 {
        theme::rgb(theme.text_muted)
    } else {
        theme::rgb(theme.danger)
    }
}

#[cfg(test)]
mod tests {
    use super::columns;

    #[test]
    fn cards_never_grow_past_their_width() -> anyhow::Result<()> {
        assert_eq!(columns(100.0, 160.0), 1, "a narrow pane holds one card");
        assert_eq!(columns(160.0, 160.0), 1, "exactly one card");
        assert_eq!(columns(161.0, 160.0), 2, "a point more splits the row");
        let n = columns(700.0, 160.0);
        let w = (700.0 - 12.0 * (n as f32 - 1.0)) / n as f32;
        assert!(w <= 160.0, "cards stay at most 160 wide: {w}");
        Ok(())
    }
}

// ============================================================================
// Module:       gui::pages::people::view
// Description:  Painting for the People tab: person cards, segment rows, the
//               small type and chips they share.
//
// Dependencies: egui; crate::gui::{thumbs, ui::{theme, icons}}, crate::index
// ============================================================================

//! How the People tab looks, in the Shots tab's language (Selects' media
//! cards and list rows): a person is a 16:9 thumbnail of frames cropped on
//! the action, the name under it in 12 px Medium and one muted line of
//! counts; a segment is a 28 px row with a 16 px thumbnail and its caption.
//! Selection is a 2 px outline round the thumbnail (cards) or the `#525252`
//! row fill (rows); hover is an ink wash.

use crate::gui::ui::hint::Hint as _;
use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::gui::thumbs::Thumbs;
use crate::gui::ui::theme::{self, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::gui::ui::{icons, widgets};
use crate::index::Index;
use crate::theme::Palette;

use super::model::{Group, Model, STRIP};

/// Small body text.
pub(super) const SMALL: f32 = theme::BODY_SIZE;

/// Meta text (counts, times).
pub(super) const META: f32 = theme::SMALL_SIZE;

/// Widest a person card gets (Selects' card grid).
pub(super) const CARD_MAX: f32 = 160.0;

/// Gap between card columns.
pub(super) const GAP_X: f32 = SPACE_MD;

/// Gap between card rows.
pub(super) const GAP_Y: f32 = SPACE_LG;

/// The text under a card's thumbnail: a 4 px gap, the name (15), 2 px,
/// the counts (14).
const INFO_H: f32 = SPACE_XS + 15.0 + 2.0 + 14.0;

/// Height of a segment row (Selects' list row).
const ROW: f32 = theme::CONTROL_HEIGHT;

/// Height of a row's thumbnail.
const THUMB_H: f32 = 16.0;

/// Text in a size and a palette colour.
pub(super) fn text(s: impl Into<String>, size: f32, color: Color32) -> egui::RichText {
    egui::RichText::new(s).font(theme::font(size)).color(color)
}

/// Columns and card width for a grid `inner` points wide: as many columns
/// as keep every card at most [`CARD_MAX`] wide.
pub(super) fn columns(inner: f32) -> (usize, f32) {
    let n = (((inner + GAP_X) / (CARD_MAX + GAP_X)).ceil() as usize).max(1);
    let width = ((inner - GAP_X * (n as f32 - 1.0)) / n as f32).max(1.0);
    (n, width)
}

/// Where a card's name line is, for the inline rename field over it.
pub(super) fn name_rect(card: Rect) -> Rect {
    let thumb_h = (card.width() * 9.0 / 16.0).round();
    Rect::from_min_size(
        pos2(card.left(), card.top() + thumb_h + 2.0),
        vec2(card.width(), 20.0),
    )
}

/// Paints segment `i`'s middle frame into `rect`, cropped on its action
/// centre, when the rect is on screen.
pub(super) fn frame(
    ui: &Ui,
    thumbs: &mut Thumbs,
    palette: &Palette,
    rect: Rect,
    index: &Index,
    model: &Model,
    i: usize,
) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let Some(seg) = index.segments.get(i) else {
        return;
    };
    let mid = f64::from(seg.seg.start + seg.seg.end) / 2.0;
    thumbs.paint(
        ui,
        rect,
        model.path(&seg.clip),
        mid,
        seg.seg.cx,
        theme::rgb(palette.raised),
    );
}

/// How a card is drawn.
pub(super) struct CardLook {
    /// Selected (the key outline).
    pub(super) selected: bool,
    /// Its segments are what the detail pane shows.
    pub(super) focused: bool,
    /// A drag could land here.
    pub(super) droppable: bool,
}

/// A person card: a 16:9 thumbnail holding a strip of frames, a glass
/// segment count, the name and the counts. The response takes clicks, the
/// context menu and segment drops.
pub(super) fn card(
    ui: &mut Ui,
    thumbs: &mut Thumbs,
    palette: &Palette,
    group: &Group,
    index: Option<&Index>,
    model: &Model,
    look: &CardLook,
) -> egui::Response {
    let width = ui.available_width();
    let thumb_h = (width * 9.0 / 16.0).round();
    let (rect, response) = ui.allocate_exact_size(vec2(width, thumb_h + INFO_H), Sense::click());
    let dragging = egui::DragAndDrop::has_payload_of_type::<super::DragSegs>(ui.ctx());
    let drop_hover = look.droppable && dragging && response.contains_pointer();
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter_at(rect.expand(SPACE_XS + 2.0));
    let hovered = response.hovered() && !drop_hover;
    if drop_hover {
        painter.rect(
            rect.expand(SPACE_XS),
            CornerRadius::same(theme::RADIUS),
            theme::translucent(palette.accent, 26),
            Stroke::new(1.0, theme::rgb(palette.accent)),
            egui::StrokeKind::Inside,
        );
    } else if hovered {
        painter.rect_filled(
            rect.expand(SPACE_XS),
            CornerRadius::same(theme::RADIUS),
            theme::ink(palette, theme::INK_HOVER),
        );
    }

    // The thumbnail: up to four frames side by side, cropped on each
    // segment's action.
    let thumb = Rect::from_min_size(rect.min, vec2(width, thumb_h));
    let radius = CornerRadius::same(theme::RADIUS_XS);
    painter.rect_filled(thumb, radius, theme::rgb(palette.raised));
    let shown = group.strip.len().clamp(1, STRIP);
    let gap = 1.0;
    let cell_w = (thumb.width() - (shown as f32 - 1.0) * gap) / shown as f32;
    for k in 0..shown {
        let r = Rect::from_min_size(
            pos2(thumb.left() + k as f32 * (cell_w + gap), thumb.top()),
            vec2(cell_w, thumb_h),
        );
        if let (Some(&i), Some(index)) = (group.strip.get(k), index) {
            frame(ui, thumbs, palette, r, index, model, i);
        } else {
            icons::paint(
                &painter,
                r,
                icons::USER,
                icons::SIZE + 4.0,
                theme::rgb(palette.text_muted),
            );
        }
    }
    count_badge(&painter, palette, thumb, group.segs);

    // Selection: 2 px, 2 px clear of the thumbnail.
    if look.selected || look.focused {
        let color = if look.selected {
            theme::rgb(palette.accent)
        } else {
            theme::rgb(palette.text_muted)
        };
        painter.rect_stroke(
            thumb.expand(2.0),
            CornerRadius::same(theme::RADIUS_XS + 2),
            Stroke::new(2.0, color),
            egui::StrokeKind::Outside,
        );
    }

    // Name, then the counts.
    let ink = theme::rgb(palette.text);
    let muted = theme::rgb(palette.text_muted);
    let name_font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    let y = thumb.bottom() + SPACE_XS;
    let unassigned = group.name.is_none();
    let name = if unassigned {
        "Unassigned".to_owned()
    } else {
        group.label().to_owned()
    };
    painter.text(
        pos2(rect.left() + 2.0, y),
        Align2::LEFT_TOP,
        elide(ui, &name, name_font.clone(), width - 4.0),
        name_font,
        if unassigned { muted } else { ink },
    );
    let counts = format!(
        "{} clip{} · {} seg{}",
        group.clips.len(),
        if group.clips.len() == 1 { "" } else { "s" },
        group.segs,
        if group.segs == 1 { "" } else { "s" }
    );
    painter.text(
        pos2(rect.left() + 2.0, y + 17.0),
        Align2::LEFT_TOP,
        elide(ui, &counts, theme::font(META), width - 4.0),
        theme::font(META),
        muted,
    );
    let props = props_line(group);
    if props.is_empty() {
        response
    } else {
        response.hint(props)
    }
}

/// The segment count on a card, a glass badge in the thumbnail's corner.
fn count_badge(painter: &egui::Painter, palette: &Palette, thumb: Rect, segs: usize) {
    let label = segs.to_string();
    let font = theme::medium(painter.ctx(), theme::BODY_SIZE);
    let galley = painter.layout_no_wrap(label, font, Color32::WHITE);
    let size = vec2(galley.size().x + SPACE_XS * 2.0, SPACE_LG + 2.0);
    let rect =
        Align2::RIGHT_BOTTOM.anchor_size(thumb.right_bottom() - vec2(SPACE_XS, SPACE_XS), size);
    painter.rect_filled(
        rect,
        CornerRadius::same(theme::RADIUS_XS),
        theme::glass_fill(palette),
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

/// `fans 12 · staff 3`: the props a person was seen with.
pub(super) fn props_line(group: &Group) -> String {
    group
        .props
        .iter()
        .take(3)
        .map(|(p, n)| format!("{p} {n}"))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// `text` cut with an ellipsis to fit `width`.
pub(super) fn elide(ui: &Ui, text: &str, font: egui::FontId, width: f32) -> String {
    let fits = |s: &str| {
        ui.painter()
            .layout_no_wrap(s.to_owned(), font.clone(), Color32::PLACEHOLDER)
            .size()
            .x
            <= width
    };
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let candidate: String = chars.iter().take(mid).collect::<String>() + "…";
        if fits(&candidate) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars.iter().take(lo).collect::<String>() + "…"
}

/// How a segment row is drawn.
pub(super) struct RowLook {
    /// Picked (part of the selection).
    pub(super) picked: bool,
    /// Loaded in the viewer.
    pub(super) playing: bool,
}

/// A segment row, as Selects' list rows: 28 px, 8 px corners, a 16 px
/// thumbnail, the caption in 14 px and the length in 12 px tabular, muted.
/// The id, times, quality, flags and performer are in its tooltip.
pub(super) fn segment_row(
    ui: &mut Ui,
    thumbs: &mut Thumbs,
    palette: &Palette,
    index: &Index,
    model: &Model,
    i: usize,
    look: &RowLook,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, ROW), Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let Some(seg) = index.segments.get(i) else {
        return response;
    };
    let radius = CornerRadius::same(theme::RADIUS_MD);
    if look.picked {
        ui.painter()
            .rect_filled(rect, radius, theme::selected_row(palette));
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, radius, theme::rgb(palette.raised));
    }
    let pad = SPACE_XS + 2.0;
    let thumb = Rect::from_min_size(
        pos2(rect.left() + pad, rect.center().y - THUMB_H / 2.0),
        vec2(THUMB_H * 16.0 / 9.0, THUMB_H),
    );
    frame(ui, thumbs, palette, thumb, index, model, i);
    ui.painter().rect_stroke(
        thumb,
        CornerRadius::same(theme::RADIUS_XS),
        if look.playing {
            Stroke::new(1.5, theme::rgb(palette.accent))
        } else {
            theme::ink_line(palette)
        },
        egui::StrokeKind::Outside,
    );
    let ink = theme::rgb(palette.text);
    let muted = theme::rgb(palette.text_muted);
    let length = format!("{:.1}s", seg.seg.end - seg.seg.start);
    let numbers = theme::tabular(ui.ctx(), theme::BODY_SIZE);
    let length_w = ui
        .painter()
        .layout_no_wrap(length.clone(), numbers.clone(), muted)
        .size()
        .x;
    ui.painter().text(
        pos2(rect.right() - pad, rect.center().y),
        Align2::RIGHT_CENTER,
        length,
        numbers,
        muted,
    );
    let x = thumb.right() + SPACE_SM;
    let w = (rect.right() - pad - length_w - SPACE_SM - x).max(10.0);
    let summary = seg
        .caption
        .as_ref()
        .map_or_else(|| seg.id.clone(), |c| c.summary.clone());
    let font = theme::font(theme::LARGE_SIZE);
    ui.painter().text(
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        elide(ui, &summary, font.clone(), w),
        font,
        ink,
    );
    widgets::rich_tip(response, &seg.id, |ui| {
        ui.set_max_width(300.0);
        ui.label(
            egui::RichText::new(&seg.id)
                .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                .color(ink),
        );
        if let Some(c) = &seg.caption {
            ui.label(egui::RichText::new(&c.summary).color(ink));
            if let Some(p) = c.people.first() {
                ui.label(
                    egui::RichText::new(format!("{} [{}]", p.appearance, p.prop)).color(muted),
                );
            }
        }
        let mut numbers = format!("{:.1}–{:.1}s · q {:.2}", seg.seg.start, seg.seg.end, seg.q);
        if !seg.flags.is_empty() {
            numbers.push_str(" · ");
            numbers.push_str(&seg.flags.join(" "));
        }
        ui.label(egui::RichText::new(numbers).color(muted));
    })
}

/// A clip header inside the detail list: Selects' section label (12 px,
/// muted), and a drag handle for the clip's segments.
pub(super) fn clip_header(ui: &mut Ui, palette: &Palette, label: &str) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, theme::COMPACT_HEIGHT), Sense::click_and_drag());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS),
            theme::ink(palette, theme::INK_HOVER),
        );
    }
    ui.painter().text(
        rect.left_center() + vec2(SPACE_XS + 2.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        theme::medium(ui.ctx(), theme::BODY_SIZE),
        theme::rgb(palette.text_muted),
    );
    response
}

/// A floating "Move N segments" label under the pointer while dragging.
pub(super) fn drag_label(ui: &Ui, palette: &Palette, n: usize) {
    let Some(pos) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::unique("people-drag"),
    ));
    let label = format!(
        "Move {n} segment{} to a person",
        if n == 1 { "" } else { "s" }
    );
    let galley = painter.layout_no_wrap(label, theme::font(SMALL), theme::rgb(palette.text));
    let rect = Rect::from_min_size(
        pos + vec2(14.0, 10.0),
        galley.size() + vec2(SPACE_MD * 2.0, SPACE_SM),
    );
    painter.rect(
        rect,
        CornerRadius::same(theme::RADIUS_MD),
        theme::popover_fill(palette),
        theme::ink_line(palette),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        rect.min + vec2(SPACE_MD, SPACE_XS),
        galley,
        theme::rgb(palette.text),
    );
}

#[cfg(test)]
mod tests {
    use super::{CARD_MAX, GAP_X, columns};

    #[test]
    fn person_cards_stay_at_most_card_max_wide() -> anyhow::Result<()> {
        for inner in [80.0, 160.0, 250.0, 333.0, 700.0, 1200.0] {
            let (n, w) = columns(inner);
            assert!(w <= CARD_MAX + 0.01, "{inner}: {w} wide");
            let used = w * n as f32 + GAP_X * (n as f32 - 1.0);
            assert!(
                (used - inner).abs() < 0.5,
                "{inner}: the row is filled ({used})"
            );
        }
        Ok(())
    }
}

// ============================================================================
// Module:       gui::pages::people::view
// Description:  Painting for the People tab: person cards, segment rows, the
//               small type and chips they share.
//
// Dependencies: egui; crate::gui::{thumbs, ui::theme}, crate::index
// ============================================================================

//! How the People tab looks: flat cards with a strip of frames cropped on
//! the action, rows of segments with their caption, small neutral type, and
//! the thumbnails carrying the colour.

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use crate::gui::thumbs::Thumbs;
use crate::gui::ui::theme;
use crate::index::Index;
use crate::theme::Palette;

use super::model::{Group, Model, STRIP};

/// Small body text.
pub(super) const SMALL: f32 = 12.0;

/// Meta text (counts, times).
pub(super) const META: f32 = 11.0;

/// Height of a segment row.
pub(super) const ROW: f32 = 54.0;

/// Text in the small size and a palette colour.
pub(super) fn text(s: impl Into<String>, size: f32, color: Color32) -> egui::RichText {
    egui::RichText::new(s).font(theme::font(size)).color(color)
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
        theme::rgb(palette.hover),
    );
}

/// How a card is drawn.
pub(super) struct CardLook {
    /// Selected (accent border).
    pub(super) selected: bool,
    /// Its segments are what the detail pane shows.
    pub(super) focused: bool,
    /// A drag could land here.
    pub(super) droppable: bool,
}

/// A person card: a strip of frames, the name, counts and props. The
/// response takes clicks, the context menu and segment drops.
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
    let cell = ((width - 2.0 * theme::SPACE_XS) / STRIP as f32).max(8.0);
    let strip_h = (cell * 1.25).min(96.0);
    let height = strip_h + 50.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let dragging = egui::DragAndDrop::has_payload_of_type::<super::DragSegs>(ui.ctx());
    let drop_hover = look.droppable && dragging && response.contains_pointer();
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let hovered = response.hovered() || drop_hover;
    let fill = if drop_hover {
        theme::translucent(palette.accent, 60)
    } else if look.focused {
        theme::rgb(palette.selection)
    } else if hovered {
        theme::rgb(palette.hover)
    } else {
        theme::rgb(palette.raised)
    };
    let stroke = if look.selected || drop_hover {
        Stroke::new(1.5, theme::rgb(palette.accent))
    } else {
        Stroke::new(1.0, theme::rgb(palette.border))
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );

    // The strip: up to four frames, cropped on each segment's action.
    let inner = rect.shrink(theme::SPACE_XS);
    let cell_w = (inner.width() - (STRIP as f32 - 1.0) * 2.0) / STRIP as f32;
    for k in 0..STRIP {
        let r = Rect::from_min_size(
            pos2(inner.left() + k as f32 * (cell_w + 2.0), inner.top()),
            vec2(cell_w, strip_h - theme::SPACE_XS),
        );
        match (group.strip.get(k), index) {
            (Some(&i), Some(index)) => frame(ui, thumbs, palette, r, index, model, i),
            _ => {
                ui.painter()
                    .rect_filled(r, CornerRadius::same(2), theme::rgb(palette.panel));
            }
        }
    }

    // Name, counts, props.
    let ink = theme::rgb(palette.text);
    let muted = theme::rgb(palette.text_muted);
    let y = inner.top() + strip_h;
    let name = if group.name.is_none() {
        format!("Unassigned · {}", group.clips.len())
    } else {
        group.label().to_owned()
    };
    ui.painter().text(
        pos2(inner.left() + 2.0, y),
        Align2::LEFT_TOP,
        elide(ui, &name, theme::font(SMALL), inner.width() - 4.0),
        theme::font(SMALL),
        if group.name.is_none() { muted } else { ink },
    );
    let counts = format!(
        "{} clip{} · {} seg{}",
        group.clips.len(),
        if group.clips.len() == 1 { "" } else { "s" },
        group.segs,
        if group.segs == 1 { "" } else { "s" }
    );
    ui.painter().text(
        pos2(inner.left() + 2.0, y + 16.0),
        Align2::LEFT_TOP,
        counts,
        theme::font(META),
        muted,
    );
    let props = props_line(group);
    if !props.is_empty() {
        ui.painter().text(
            pos2(inner.left() + 2.0, y + 30.0),
            Align2::LEFT_TOP,
            elide(ui, &props, theme::font(META), inner.width() - 4.0),
            theme::font(META),
            theme::rgb(palette.accent),
        );
    }
    response
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

/// A segment row: frame, id and times, quality, caption, main performer.
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
    let fill = if look.picked {
        theme::rgb(palette.selection)
    } else if response.hovered() {
        theme::rgb(palette.hover)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(3), fill);
    if look.picked {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.min, vec2(theme::SELECTION_BAR, rect.height())),
            CornerRadius::same(1),
            theme::rgb(palette.accent),
        );
    }
    let thumb_h = ROW - 6.0;
    let thumb = Rect::from_min_size(
        rect.min + vec2(theme::SPACE_XS + 2.0, 3.0),
        vec2(thumb_h * 16.0 / 9.0, thumb_h),
    );
    frame(ui, thumbs, palette, thumb, index, model, i);
    if look.playing {
        ui.painter().rect_stroke(
            thumb,
            CornerRadius::same(2),
            Stroke::new(1.5, theme::rgb(palette.accent)),
            egui::StrokeKind::Outside,
        );
    }
    let x = thumb.right() + theme::SPACE_SM;
    let w = (rect.right() - x - theme::SPACE_XS).max(10.0);
    let ink = theme::rgb(palette.text);
    let muted = theme::rgb(palette.text_muted);
    let mut head = format!(
        "{}  {:.1}–{:.1}s  q{:.2}",
        seg.id, seg.seg.start, seg.seg.end, seg.q
    );
    if !seg.flags.is_empty() {
        head.push_str("  ");
        head.push_str(&seg.flags.join(" "));
    }
    ui.painter().text(
        pos2(x, rect.top() + 4.0),
        Align2::LEFT_TOP,
        elide(ui, &head, theme::font(META), w),
        theme::font(META),
        muted,
    );
    let (summary, who) = seg.caption.as_ref().map_or_else(
        || ("(no caption)".to_owned(), String::new()),
        |c| {
            let who = c
                .people
                .first()
                .map(|p| format!("{} [{}]", p.appearance, p.prop))
                .unwrap_or_default();
            (c.summary.clone(), who)
        },
    );
    ui.painter().text(
        pos2(x, rect.top() + 19.0),
        Align2::LEFT_TOP,
        elide(ui, &summary, theme::font(SMALL), w),
        theme::font(SMALL),
        ink,
    );
    if !who.is_empty() {
        ui.painter().text(
            pos2(x, rect.top() + 35.0),
            Align2::LEFT_TOP,
            elide(ui, &who, theme::font(META), w),
            theme::font(META),
            muted,
        );
    }
    response
}

/// A clip header inside the detail list: id, segment count, drag handle.
pub(super) fn clip_header(ui: &mut Ui, palette: &Palette, label: &str) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 20.0), Sense::click_and_drag());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), theme::rgb(palette.hover));
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(theme::SPACE_XS, 0.0),
        Align2::LEFT_CENTER,
        label,
        theme::font(META),
        theme::rgb(palette.text_muted),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        Stroke::new(1.0, theme::rgb(palette.border)),
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
        "Move {n} segment{} → drop on a person",
        if n == 1 { "" } else { "s" }
    );
    let galley = painter.layout_no_wrap(label, theme::font(META), theme::rgb(palette.text));
    let rect = Rect::from_min_size(pos + vec2(14.0, 10.0), galley.size() + vec2(12.0, 6.0));
    painter.rect(
        rect,
        CornerRadius::same(3),
        theme::rgb(palette.raised),
        Stroke::new(1.0, theme::rgb(palette.accent)),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(6.0, 3.0), galley, theme::rgb(palette.text));
}

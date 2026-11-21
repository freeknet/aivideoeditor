// ============================================================================
// Module:       gui::pages::inspector::rows
// Description:  The inspector's property rows, as Selects' side panel lays
//               them out: a 12 px muted label, the 12 px value beside it,
//               sections under a hairline with a small SemiBold heading.
//
// Dependencies: egui; crate::gui::ui::theme, crate::theme::Palette
// ============================================================================

//! Property rows, the way Selects' Adjust panel is laid out: sections
//! separated by a hairline, each opened by a 32 px heading in 12 px
//! SemiBold; rows of a muted label column of fixed width and the value
//! beside it, wrapping in the rest. Links are the foreground colour,
//! underlined under the pointer: lime is not a link colour here.

use egui::{Color32, RichText, Sense, Ui};

use crate::gui::ui::theme;
use crate::theme::Palette;

/// Body size in the inspector: values and labels (`text-xs`).
pub(super) const SMALL: f32 = theme::BODY_SIZE;

/// Meta size: subtitles, captions under a row.
pub(super) const META: f32 = theme::SMALL_SIZE;

/// Width of the label column.
const LABEL: f32 = 88.0;

/// Height of a section's heading row (Selects' group heading).
const HEADING: f32 = 32.0;

/// Small text in a colour.
pub(super) fn text(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).font(theme::font(size)).color(color)
}

/// What the inspector is showing: 14 px Medium, with a muted line under it
/// (pass `""` for none).
pub(super) fn title(ui: &mut Ui, palette: &Palette, title: &str, subtitle: &str) {
    ui.add_space(theme::SPACE_SM);
    ui.add(
        egui::Label::new(
            RichText::new(title)
                .font(theme::medium(ui.ctx(), theme::TITLE_SIZE))
                .color(theme::rgb(palette.text)),
        )
        .wrap(),
    );
    if !subtitle.is_empty() {
        ui.add(egui::Label::new(text(subtitle, SMALL, theme::rgb(palette.text_muted))).wrap());
    }
}

/// A section: a hairline across the panel, then the title in 12 px
/// SemiBold on a 32 px row.
pub(super) fn section(ui: &mut Ui, palette: &Palette, title: &str) {
    section_with(ui, palette, title, "");
}

/// [`section`] with a muted note (a count) at the right of the heading.
pub(super) fn section_with(ui: &mut Ui, palette: &Palette, title: &str, note: &str) {
    ui.add_space(theme::SPACE_SM);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, HEADING), Sense::hover());
    // The hairline runs out to the panel's edges, as Selects' dividers do.
    let clip = ui.clip_rect();
    ui.painter().hline(
        clip.left()..=clip.right(),
        rect.top() + 0.5,
        theme::ink_line(palette),
    );
    ui.painter().text(
        egui::pos2(rect.left(), rect.center().y + 2.0),
        egui::Align2::LEFT_CENTER,
        title,
        theme::semibold(ui.ctx(), theme::BODY_SIZE),
        theme::rgb(palette.text),
    );
    if !note.is_empty() {
        ui.painter().text(
            egui::pos2(rect.right() - theme::SPACE_SM, rect.center().y + 2.0),
            egui::Align2::RIGHT_CENTER,
            note,
            theme::tabular(ui.ctx(), theme::BODY_SIZE),
            theme::rgb(palette.text_muted),
        );
    }
}

/// The label column of a row.
fn label(ui: &mut Ui, palette: &Palette, label: &str, align: egui::Align) {
    ui.allocate_ui_with_layout(
        egui::vec2(LABEL, theme::LARGE_SIZE + 2.0),
        egui::Layout::left_to_right(align),
        |ui| {
            ui.set_width(LABEL);
            ui.add(
                egui::Label::new(text(label, SMALL, theme::rgb(palette.text_muted)))
                    .wrap_mode(egui::TextWrapMode::Truncate),
            );
        },
    );
}

/// One property row with the value in the text colour.
pub(super) fn row(ui: &mut Ui, palette: &Palette, label: &str, value: impl Into<String>) {
    row_colored(ui, palette, label, value, theme::rgb(palette.text));
}

/// One property row with the value in `color`.
pub(super) fn row_colored(
    ui: &mut Ui,
    palette: &Palette,
    name: &str,
    value: impl Into<String>,
    color: Color32,
) {
    let value = value.into();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
        label(ui, palette, name, egui::Align::Min);
        ui.add(egui::Label::new(text(value, SMALL, color)).wrap());
    });
}

/// A row whose value is a link; returns whether it was clicked.
pub(super) fn row_link(ui: &mut Ui, palette: &Palette, name: &str, value: &str) -> bool {
    let mut clicked = false;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
        label(ui, palette, name, egui::Align::Min);
        clicked = link(ui, palette, value).clicked();
    });
    clicked
}

/// A link: foreground text, underlined and a pointing hand under the
/// pointer.
pub(super) fn link(ui: &mut Ui, palette: &Palette, value: &str) -> egui::Response {
    let ink = theme::rgb(palette.text);
    let response = ui.add(
        egui::Label::new(text(value, SMALL, ink))
            .sense(Sense::click())
            .wrap(),
    );
    if response.hovered() {
        let r = response.rect;
        ui.painter()
            .hline(r.x_range(), r.bottom() - 1.0, egui::Stroke::new(1.0, ink));
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A row with a small bar for a 0..1 value and the number beside it.
pub(super) fn row_meter(ui: &mut Ui, palette: &Palette, name: &str, value: f32, shown: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
        label(ui, palette, name, egui::Align::Center);
        let width = (ui.available_width() - 72.0).clamp(30.0, 140.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 4.0), Sense::hover());
        let painter = ui.painter();
        let radius = egui::CornerRadius::same(2);
        painter.rect_filled(rect, radius, theme::ink(palette, theme::INK_10));
        let filled = egui::Rect::from_min_size(
            rect.min,
            egui::vec2(rect.width() * value.clamp(0.0, 1.0), rect.height()),
        );
        painter.rect_filled(filled, radius, theme::rgb(palette.text_muted));
        ui.label(
            RichText::new(shown)
                .font(theme::tabular(ui.ctx(), SMALL))
                .color(theme::rgb(palette.text)),
        );
    });
}

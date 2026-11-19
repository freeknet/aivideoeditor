// ============================================================================
// Module:       gui::pages::inspector::rows
// Description:  The inspector's property rows: label left, value right, small
//               neutral type, section headers with a hairline.
//
// Dependencies: egui; crate::gui::ui::theme, crate::theme::Palette
// ============================================================================

//! Property rows, the way pro editors' inspectors are laid out: a muted
//! label column of fixed width, the value beside it wrapping in the rest.

use egui::{Color32, RichText, Ui};

use crate::gui::ui::theme;
use crate::theme::Palette;

/// Body size in the inspector.
pub(super) const SMALL: f32 = 12.0;

/// Label and meta size.
pub(super) const META: f32 = 11.0;

/// Width of the label column.
const LABEL: f32 = 78.0;

/// Small text in a colour.
pub(super) fn text(s: impl Into<String>, size: f32, color: Color32) -> RichText {
    RichText::new(s).font(theme::font(size)).color(color)
}

/// A section header: small caps-ish muted title over a hairline.
pub(super) fn section(ui: &mut Ui, palette: &Palette, title: &str) {
    ui.add_space(theme::SPACE_SM);
    ui.label(
        text(
            title.to_uppercase(),
            META - 1.0,
            theme::rgb(palette.text_muted),
        )
        .strong(),
    );
    let rect = ui.available_rect_before_wrap();
    ui.painter().hline(
        rect.x_range(),
        rect.top(),
        egui::Stroke::new(1.0, theme::rgb(palette.border)),
    );
    ui.add_space(theme::SPACE_XS);
}

/// One property row with the value in the text colour.
pub(super) fn row(ui: &mut Ui, palette: &Palette, label: &str, value: impl Into<String>) {
    row_colored(ui, palette, label, value, theme::rgb(palette.text));
}

/// One property row with the value in `color`.
pub(super) fn row_colored(
    ui: &mut Ui,
    palette: &Palette,
    label: &str,
    value: impl Into<String>,
    color: Color32,
) {
    let value = value.into();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
        ui.allocate_ui_with_layout(
            egui::vec2(LABEL, 14.0),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                ui.set_width(LABEL);
                ui.label(text(label, META, theme::rgb(palette.text_muted)));
            },
        );
        ui.add(egui::Label::new(text(value, SMALL, color)).wrap());
    });
}

/// A row whose value is a link; returns whether it was clicked.
pub(super) fn row_link(ui: &mut Ui, palette: &Palette, label: &str, value: &str) -> bool {
    let mut clicked = false;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
        ui.allocate_ui_with_layout(
            egui::vec2(LABEL, 14.0),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                ui.set_width(LABEL);
                ui.label(text(label, META, theme::rgb(palette.text_muted)));
            },
        );
        clicked = ui
            .link(text(value, SMALL, theme::rgb(palette.accent)))
            .clicked();
    });
    clicked
}

/// A row with a small bar for a 0..1 value and the number beside it.
pub(super) fn row_meter(ui: &mut Ui, palette: &Palette, label: &str, value: f32, shown: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
        ui.allocate_ui_with_layout(
            egui::vec2(LABEL, 14.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_width(LABEL);
                ui.label(text(label, META, theme::rgb(palette.text_muted)));
            },
        );
        let width = (ui.available_width() - 44.0).clamp(30.0, 140.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 5.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, egui::CornerRadius::same(2), theme::rgb(palette.hover));
        let filled = egui::Rect::from_min_size(
            rect.min,
            egui::vec2(rect.width() * value.clamp(0.0, 1.0), rect.height()),
        );
        painter.rect_filled(
            filled,
            egui::CornerRadius::same(2),
            theme::rgb(palette.accent),
        );
        ui.label(text(shown, SMALL, theme::rgb(palette.text)));
    });
}

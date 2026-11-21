// ============================================================================
// Module:       gui::pages::night::parts
// Description:  Small drawing pieces the Home pages and the export panel
//               share: the centred column, section heads, form rows, path
//               boxes, the run progress bar, notices and status marks.
//
// Dependencies: egui; crate::gui::ui::{icons, theme, widgets}
// ============================================================================

//! Shared pieces of the Home pages (start, wizard, processing, night page)
//! and the export panel.
//!
//! They are compositions of the design system's tokens and components
//! (`docs/DESIGN.md`), kept here rather than in `widgets` because only
//! these pages use them: a centred column, a section with a hairline
//! above it, a shadcn form row (12 px muted label, control beside it), a
//! read-only path shown as a field, Selects' run-list progress bar (8 px,
//! ink on ink), a sonner-style notice, and the step marks (spinner, check,
//! cross) of a run.

use std::path::Path;

use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, Ui, Vec2};

use crate::gui::ui::{icons, theme, widgets};
use crate::theme::Palette;

/// The width of a form row's label column.
pub(crate) const LABEL_WIDTH: f32 = 132.0;

/// Lays `add` out in a column at most `max` wide, centred in the space.
pub(crate) fn centred<R>(ui: &mut Ui, max: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let available = ui.available_width();
    let width = available.min(max);
    let side = ((available - width) / 2.0).max(0.0);
    let spacing = ui.spacing().item_spacing;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(side);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = spacing;
            ui.set_width(width);
            add(ui)
        })
        .inner
    })
    .inner
}

/// A quiet label over a group: 12 px Medium, muted.
pub(crate) fn label(ui: &mut Ui, theme: &Palette, text: &str) -> Response {
    ui.label(
        egui::RichText::new(text)
            .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
            .color(theme::rgb(theme.text_muted)),
    )
}

/// A section: a hairline across the column, the title in 14 px Medium and
/// one muted sentence under it.
pub(crate) fn section(ui: &mut Ui, theme: &Palette, title: &str, description: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, theme::ink_line(theme));
    ui.add_space(theme::SPACE_LG);
    ui.label(
        egui::RichText::new(title)
            .font(theme::medium(ui.ctx(), theme::TITLE_SIZE))
            .color(theme::rgb(theme.text)),
    );
    if !description.is_empty() {
        ui.add_space(2.0);
        muted(ui, theme, description);
    }
    ui.add_space(theme::SPACE_MD);
}

/// One muted, wrapping line of 12 px text.
pub(crate) fn muted(ui: &mut Ui, theme: &Palette, text: &str) -> Response {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(theme::font(theme::BODY_SIZE))
                .color(theme::rgb(theme.text_muted)),
        )
        .wrap(),
    )
}

/// A form row: `label` in 12 px muted text in a [`LABEL_WIDTH`] column,
/// then whatever `add` lays out, on one 28 px line.
pub(crate) fn form_row<R>(
    ui: &mut Ui,
    theme: &Palette,
    label: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        ui.set_min_height(theme::CONTROL_HEIGHT);
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(LABEL_WIDTH, theme::CONTROL_HEIGHT),
            Sense::hover(),
        );
        ui.painter().text(
            rect.left_center(),
            Align2::LEFT_CENTER,
            label,
            theme::medium(ui.ctx(), theme::BODY_SIZE),
            theme::rgb(theme.text_muted),
        );
        add(ui)
    })
    .inner
}

/// `path`'s text cut from the left so it fits `width` at 12 px.
fn fit_path(ui: &Ui, path: &str, width: f32) -> String {
    let font = theme::font(theme::BODY_SIZE);
    let fits = |text: &str| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER)
            .size()
            .x
            <= width
    };
    if fits(path) {
        return path.to_owned();
    }
    let chars: Vec<char> = path.chars().collect();
    let mut skip = 1;
    while skip < chars.len() {
        let tail: String = chars.iter().skip(skip).collect();
        let text = format!("\u{2026}{tail}");
        if fits(&text) {
            return text;
        }
        skip += 1;
    }
    String::new()
}

/// A path shown as a read-only field, `width` wide: the input surface and
/// outline, a leading icon, the path cut from the left to fit (or
/// `placeholder` in muted text), amber when the file is gone. The full
/// path is the tooltip.
pub(crate) fn path_box(
    ui: &mut Ui,
    theme: &Palette,
    glyph: char,
    path: Option<&Path>,
    placeholder: &str,
    width: f32,
) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, theme::CONTROL_HEIGHT), Sense::hover());
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(theme::RADIUS_MD),
        theme::ink(theme, theme::INK_03),
        theme::ink_line(theme),
        egui::StrokeKind::Inside,
    );
    let icon_rect = Rect::from_center_size(
        egui::pos2(
            rect.left() + theme::SPACE_SM + icons::SMALL / 2.0,
            rect.center().y,
        ),
        Vec2::splat(icons::SMALL),
    );
    let missing = path.is_some_and(|p| !p.exists());
    let ink = if missing {
        theme::rgb(theme.warning)
    } else {
        theme::rgb(theme.text_muted)
    };
    icons::paint(painter, icon_rect, glyph, icons::SMALL, ink);
    let x = icon_rect.right() + theme::SPACE_SM;
    let room = (rect.right() - theme::SPACE_SM - x).max(0.0);
    let (text, color) = match path {
        Some(p) => {
            let shown = p.display().to_string();
            let shown = if missing {
                format!("{shown} (missing)")
            } else {
                shown
            };
            (
                fit_path(ui, &shown, room),
                if missing {
                    theme::rgb(theme.warning)
                } else {
                    theme::rgb(theme.text)
                },
            )
        }
        None => (
            fit_path(ui, placeholder, room),
            theme::rgb(theme.text_muted).gamma_multiply(0.8),
        ),
    };
    ui.painter().text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        theme::font(theme::BODY_SIZE),
        color,
    );
    match path {
        Some(p) => response.on_hover_text(p.display().to_string()),
        None => response,
    }
}

/// Selects' run progress bar: `fraction` of the width, 8 px tall, fully
/// rounded, ink 80 % on an ink 10 % track (white, not the key colour). It
/// settles rather than jumps.
pub(crate) fn progress_bar(ui: &mut Ui, theme: &Palette, id: egui::Id, fraction: f32) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 8.0), Sense::hover());
    paint_progress(ui, theme, rect, id, fraction);
    response
}

/// [`progress_bar`] painted into `rect`.
pub(crate) fn paint_progress(ui: &Ui, theme: &Palette, rect: Rect, id: egui::Id, fraction: f32) {
    let shown = widgets::level(ui.ctx(), id, fraction.clamp(0.0, 1.0));
    let radius = CornerRadius::same((rect.height() / 2.0).round() as u8);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::ink(theme, theme::INK_10));
    if shown > 0.0 {
        let filled = Rect::from_min_size(
            rect.min,
            Vec2::new((rect.width() * shown).max(rect.height()), rect.height()),
        );
        painter.rect_filled(filled, radius, theme::translucent(theme.text, 204));
    }
}

/// How a [`notice`] reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Something happened.
    Info,
    /// Done, ready.
    Success,
    /// Needs a look.
    Warning,
    /// Failed.
    Danger,
}

/// A notice in the column, as Selects' toasts: the panel surface, a
/// hairline, 8 px corners, an icon in its colour, 12 px text, and a
/// dismiss button. Returns whether it was dismissed.
pub(crate) fn notice(ui: &mut Ui, theme: &Palette, kind: Kind, text: &str) -> bool {
    let (glyph, color) = match kind {
        Kind::Info => (icons::INFO, theme.text_muted),
        Kind::Success => (icons::CIRCLE_CHECK, theme.success),
        Kind::Warning => (icons::TRIANGLE_ALERT, theme.warning),
        Kind::Danger => (icons::CIRCLE_ALERT, theme.danger),
    };
    let mut dismissed = false;
    egui::Frame::new()
        .fill(theme::rgb(theme.panel))
        .stroke(theme::ink_line(theme))
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(egui::Margin {
            left: theme::as_margin(theme::SPACE_MD),
            right: theme::as_margin(theme::SPACE_XS),
            top: theme::as_margin(theme::SPACE_XS),
            bottom: theme::as_margin(theme::SPACE_XS),
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
                icons::icon(ui, glyph, icons::SIZE, theme::rgb(color));
                let room =
                    (ui.available_width() - theme::CONTROL_HEIGHT - theme::SPACE_SM).max(0.0);
                ui.allocate_ui(Vec2::new(room, theme::CONTROL_HEIGHT), |ui| {
                    ui.set_min_height(theme::CONTROL_HEIGHT);
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text)
                                .font(theme::font(theme::BODY_SIZE))
                                .color(theme::rgb(theme.text)),
                        )
                        .wrap(),
                    );
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    dismissed = widgets::icon_btn(ui, theme, icons::X, "Dismiss", "").clicked();
                });
            });
        });
    dismissed
}

/// One line of status in its colour, led by its icon: a warning under a
/// form, a failure in a list.
pub(crate) fn status_line(ui: &mut Ui, theme: &Palette, kind: Kind, text: &str) -> Response {
    let (glyph, color) = match kind {
        Kind::Info => (icons::INFO, theme.text_muted),
        Kind::Success => (icons::CIRCLE_CHECK, theme.success),
        Kind::Warning => (icons::TRIANGLE_ALERT, theme.warning),
        Kind::Danger => (icons::CIRCLE_ALERT, theme.danger),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
        icons::icon(ui, glyph, icons::SMALL, theme::rgb(color));
        ui.add(
            egui::Label::new(
                egui::RichText::new(text)
                    .font(theme::font(theme::BODY_SIZE))
                    .color(theme::rgb(color)),
            )
            .wrap(),
        );
    })
    .response
}

/// The mark in front of a step or a clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// Waiting: a faint empty circle.
    Queued,
    /// Running: a spinner.
    Running,
    /// Done: a check in a circle.
    Done,
    /// Failed: a cross in a circle.
    Failed,
    /// Stopped or partial: an alert.
    Stopped,
    /// Did not run: a faint dash.
    Skipped,
}

/// Paints `mark` centred in `rect` (a spinner for [`Mark::Running`]).
pub(crate) fn mark(ui: &mut Ui, theme: &Palette, rect: Rect, mark: Mark) {
    let size = rect.width().min(rect.height()).min(icons::SIZE);
    let (glyph, color) = match mark {
        Mark::Running => {
            // Painted, not placed: placing would move the layout's cursor.
            egui::Spinner::new()
                .size(size - 2.0)
                .color(theme::rgb(theme.text_muted))
                .paint_at(
                    ui,
                    Rect::from_center_size(rect.center(), Vec2::splat(size - 2.0)),
                );
            ui.ctx().request_repaint();
            return;
        }
        Mark::Queued => (icons::CIRCLE, theme.text_faint),
        Mark::Done => (icons::CIRCLE_CHECK, theme.success),
        Mark::Failed => (icons::CIRCLE_X, theme.danger),
        Mark::Stopped => (icons::CIRCLE_ALERT, theme.warning),
        Mark::Skipped => (icons::MINUS, theme.text_faint),
    };
    icons::paint(ui.painter(), rect, glyph, size, theme::rgb(color));
}

/// A video frame at `rect`, with Selects' thumbnail edge: 2 px corners and
/// an ink 10 % hairline.
pub(crate) fn thumb(
    thumbs: &mut crate::gui::thumbs::Thumbs,
    ui: &Ui,
    theme: &Palette,
    rect: Rect,
    frame: Option<(&Path, f64)>,
) {
    match frame {
        Some((path, t)) => thumbs.paint(ui, rect, path, t, 0.5, theme::rgb(theme.raised)),
        None => {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(theme::RADIUS_XS),
                theme::rgb(theme.raised),
            );
            icons::paint(
                ui.painter(),
                rect,
                icons::FILM,
                icons::SMALL,
                theme::rgb(theme.text_faint),
            );
        }
    }
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS_XS),
        Stroke::new(1.0, theme::ink(theme, theme::INK_10)),
        egui::StrokeKind::Inside,
    );
}

/// The fill behind a list row: the selected row colour, or ink 8 % under
/// the pointer (faded in), on 8 px corners.
pub(crate) fn row_fill(ui: &Ui, theme: &Palette, response: &Response, selected: bool) {
    let rect = response.rect;
    if selected {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_MD),
            theme::selected_row(theme),
        );
        return;
    }
    let lift = widgets::hover_t(ui, response.id.with("row"), response.hovered());
    if lift > 0.0 {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_MD),
            theme::ink(theme, (f32::from(theme::INK_HOVER) * lift).round() as u8),
        );
    }
}

/// A clock for elapsed and remaining time: "1:05", "12:40", "1:02:03".
pub(crate) fn clock(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::clock;

    #[test]
    fn clocks_read_as_minutes_and_hours() {
        assert_eq!(clock(65.0), "1:05", "minutes and seconds");
        assert_eq!(clock(3723.0), "1:02:03", "hours when there are any");
        assert_eq!(clock(-4.0), "0:00", "never negative");
    }
}

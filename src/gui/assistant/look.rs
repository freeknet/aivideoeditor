// ============================================================================
// Module:       gui::assistant::look
// Description:  The chat panel's own pieces, drawn to Selects' chat spec: the
//               composer's mode switch and scope pills, activity rows, the
//               hover meta row, notice rows, and mentions as coloured text.
//
// Dependencies: egui; crate::gui::ui::{icons, theme, widgets}; super::complete
// ============================================================================

//! Drawing helpers for the assistant panel.
//!
//! The values are Selects' (`docs/research/07-selects-visual-chat-project.md`
//! §3): tool and thinking activity is low-contrast 12 px text with a 12 px
//! chevron, not a card; the composer's Chat / Comments switch blends into
//! the composer box; mentions are lime text with no chip around them. The
//! generic components (buttons, menus, tooltips) come from
//! [`crate::gui::ui::widgets`]; only what is particular to the chat lives
//! here.

use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, CornerRadius, Rect, Response, Sense, Stroke, Ui, Vec2};

use super::complete;
use crate::color::Rgb;
use crate::gui::ui::theme::{self, SPACE_SM, SPACE_XS};
use crate::gui::ui::{icons, widgets};
use crate::theme::Palette;

/// Selects' `tone-500` (`#737373`): activity labels, chevrons.
fn tone_500(theme: &Palette) -> Color32 {
    theme::ring(theme)
}

/// [`tone_500`] as a palette colour, for blending.
fn ring_rgb(theme: &Palette) -> Rgb {
    theme.text_muted.lerp(theme.panel, 0.36)
}

/// Selects' `tone-600` (`#525252`): timings, the meta row.
fn tone_600(theme: &Palette) -> Color32 {
    theme::selected_row(theme)
}

/// Selects' `tone-200` (`#e5e5e5`): the assistant's prose, hovered rows.
pub(super) fn tone_200(theme: &Palette) -> Color32 {
    theme::rgb(theme::primary_fill(theme))
}

/// `text` laid out in `size` px `color`, with every reference token in the
/// key colour: the composer's mirror and the sent bubble.
pub(super) fn mention_job(
    theme: &Palette,
    text: &str,
    size: f32,
    color: Color32,
    wrap: f32,
) -> LayoutJob {
    let font = theme::font(size);
    let plain = TextFormat::simple(font.clone(), color);
    let lime = TextFormat::simple(font, theme::rgb(theme.accent));
    let mut job = LayoutJob::default();
    let mut at = 0;
    for span in complete::mention_spans(text) {
        if let Some(before) = text.get(at..span.start) {
            job.append(before, 0.0, plain.clone());
        }
        if let Some(token) = text.get(span.clone()) {
            job.append(token, 0.0, lime.clone());
        }
        at = span.end;
    }
    if let Some(rest) = text.get(at..) {
        job.append(rest, 0.0, plain);
    }
    job.wrap.max_width = wrap;
    job
}

/// The composer's Chat / Comments switch: a track that blends into the
/// composer box, 2 px inner padding, segments 6 px round in 12 px Medium;
/// the open one ink 12 % and foreground, the others muted. Returns whether
/// the choice changed.
pub(super) fn mode_switch<T: PartialEq + Copy>(
    ui: &mut Ui,
    theme: &Palette,
    current: &mut T,
    items: &[(T, &str)],
) -> bool {
    let font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    let pad = 2.0;
    let galleys: Vec<_> = items
        .iter()
        .map(|&(_, label)| {
            ui.painter()
                .layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER)
        })
        .collect();
    let widths: Vec<f32> = galleys
        .iter()
        .map(|g| g.size().x + SPACE_SM * 2.0)
        .collect();
    let height = theme::COMPACT_HEIGHT;
    let total = widths.iter().sum::<f32>() + pad * 2.0;
    let (track, _) = ui.allocate_exact_size(Vec2::new(total, height + pad * 2.0), Sense::hover());
    let mut changed = false;
    let mut x = track.left() + pad;
    for ((&(value, label), galley), width) in items.iter().zip(galleys).zip(widths) {
        let rect = Rect::from_min_size(egui::pos2(x, track.top() + pad), Vec2::new(width, height));
        x += width;
        let id = ui.scope_id().with(("mode", label));
        let response = ui.interact(rect, id, Sense::click());
        let on = *current == value;
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, on, label));
        let lift = widgets::hover_t(ui, id, response.hovered());
        if on {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(theme::RADIUS),
                theme::selected_fill(theme),
            );
        }
        let ink = if on {
            theme::rgb(theme.text)
        } else {
            widgets::blend(theme.text_muted, theme::primary_fill(theme), lift)
        };
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, ink);
        widgets::focus_ring(ui, theme, &response);
        if response.clicked() && !on {
            *current = value;
            changed = true;
        }
    }
    changed
}

/// A scope choice in the composer: Material's filter chip (compact, 24 px),
/// selected with its check when `on`. The caller decides what a click
/// does, so the chip's own toggle works on a copy.
pub(super) fn scope_pill(ui: &mut Ui, _theme: &Palette, label: &str, on: bool) -> Response {
    let mut shown = on;
    let response = egui_material3::MaterialChip::filter(label, &mut shown)
        .small()
        .show(ui)
        .response;
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
    response
}
/// A quick action in the empty chat: Material's suggestion chip (compact,
/// 24 px), its outline and state layer from the theme.
pub(super) fn suggestion(ui: &mut Ui, _theme: &Palette, label: &str) -> Response {
    let response = egui_material3::MaterialChip::suggestion(label)
        .small()
        .show(ui)
        .response;
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    response
}
/// How an activity row's expanded text is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Body {
    /// The model's reasoning: italic.
    Thinking,
    /// A tool's output: monospaced.
    Output,
}

/// One thinking or tool row, as Selects' shared activity row.
#[derive(Debug)]
pub(super) struct Activity<'a> {
    /// What happened ("Read the plan", "Thinking").
    pub(super) label: &'a str,
    /// Still running: the label shimmers.
    pub(super) live: bool,
    /// It failed: a red alert icon leads and the output is red.
    pub(super) failed: bool,
    /// Right-aligned detail: elapsed time, a step count.
    pub(super) right: Option<String>,
    /// What expanding it shows, and how.
    pub(super) body: Option<(&'a str, Body)>,
}

/// A thinking or tool row: a full-width 12 px line in `tone-500`, a 12 px
/// chevron after the label when it expands, timings in 10 px on the right;
/// opened, the raw text sits under it indented 24 px on the window colour.
pub(super) fn activity(ui: &mut Ui, theme: &Palette, id: impl egui::AsIdSalt, row: &Activity<'_>) {
    let id = ui.make_persistent_id(id);
    let expandable = row.body.is_some();
    let mut open: bool = ui.data(|d| d.get_temp(id)).unwrap_or(false);
    let height = theme::COMPACT_HEIGHT;
    let width = ui.available_width();
    let sense = if expandable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    if expandable {
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, open, row.label));
    }
    let hovered = expandable && response.hovered();
    let painter = ui.painter();
    if hovered {
        painter.rect_filled(
            rect,
            CornerRadius::same(4),
            theme::rgb(theme.raised).gamma_multiply(0.4),
        );
    }
    let mut x = rect.left() + SPACE_XS;
    if row.failed {
        let slot = Rect::from_min_size(egui::pos2(x, rect.center().y - 6.0), Vec2::splat(12.0));
        icons::paint(
            painter,
            slot,
            icons::CIRCLE_ALERT,
            12.0,
            theme::rgb(theme.danger),
        );
        x += 12.0 + SPACE_SM;
    }
    let ink = if row.live {
        let t = (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
        widgets::blend(ring_rgb(theme), theme.text, t)
    } else if hovered {
        tone_200(theme)
    } else {
        tone_500(theme)
    };
    let right = row.right.as_ref().map(|text| {
        painter.layout_no_wrap(
            text.clone(),
            theme::tabular(ui.ctx(), theme::TINY_SIZE),
            tone_600(theme),
        )
    });
    let right_w = right.as_ref().map_or(0.0, |g| g.size().x + SPACE_SM);
    let chevron = if expandable { 12.0 + SPACE_XS } else { 0.0 };
    let room = (rect.right() - SPACE_XS - right_w - chevron - x).max(0.0);
    let mut job =
        LayoutJob::simple_singleline(row.label.to_owned(), theme::font(theme::BODY_SIZE), ink);
    job.wrap.max_width = room;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = painter.layout_job(job);
    let label_w = galley.size().x;
    painter.galley(
        egui::pos2(x, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    if expandable {
        let slot = Rect::from_min_size(
            egui::pos2(x + label_w + SPACE_XS, rect.center().y - 6.0),
            Vec2::splat(12.0),
        );
        let glyph = if open {
            icons::CHEVRON_DOWN
        } else {
            icons::CHEVRON_RIGHT
        };
        icons::paint(painter, slot, glyph, 12.0, tone_500(theme));
    }
    if let Some(galley) = right {
        painter.galley(
            egui::pos2(
                rect.right() - SPACE_XS - galley.size().x,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            tone_600(theme),
        );
    }
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    if let (true, Some((text, kind))) = (open, row.body) {
        ui.add_space(SPACE_XS);
        ui.horizontal(|ui| {
            ui.add_space(theme::SPACE_XL);
            egui::Frame::new()
                .fill(theme::rgb(theme.app))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(theme::margin(SPACE_SM))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let color = if row.failed {
                        theme::rgb(theme.danger)
                    } else {
                        theme::rgb(theme.text_muted)
                    };
                    let mut text = egui::RichText::new(text).color(color);
                    text = match kind {
                        Body::Thinking => text.size(theme::SMALL_SIZE).italics(),
                        Body::Output => text.font(theme::mono(theme::SMALL_SIZE)),
                    };
                    ui.add(egui::Label::new(text).wrap());
                });
        });
    }
}

/// The meta row under a message: its time and a copy button in 10 px
/// `tone-600`, shown only while the pointer is over the message (`over`).
/// The row's space is kept either way so the list does not jump.
pub(super) fn meta_row(
    ui: &mut Ui,
    theme: &Palette,
    over: Rect,
    right_aligned: bool,
    note: &str,
    copy: &str,
) {
    let height = SPACE_LG_META;
    let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
    let shown = ui.rect_contains_pointer(over.union(row));
    let ink = tone_600(theme);
    let galley = ui.painter().layout_no_wrap(
        note.to_owned(),
        theme::tabular(ui.ctx(), theme::TINY_SIZE),
        ink,
    );
    let icon = 12.0;
    let total = galley.size().x + SPACE_XS + icon;
    let left = if right_aligned {
        row.right() - total
    } else {
        row.left() + SPACE_XS
    };
    let copy_rect = Rect::from_min_size(
        egui::pos2(
            left + galley.size().x + SPACE_XS,
            row.center().y - icon / 2.0,
        ),
        Vec2::splat(icon),
    );
    let id = ui.make_persistent_id(("copy", row.min.x as i32, row.min.y as i32));
    let response = ui.interact(copy_rect.expand(2.0), id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Copy message"));
    if shown {
        let hover = response.hovered();
        ui.painter().galley(
            egui::pos2(left, row.center().y - galley.size().y / 2.0),
            galley,
            ink,
        );
        icons::paint(
            ui.painter(),
            copy_rect,
            icons::COPY,
            icon,
            if hover { theme::rgb(theme.text) } else { ink },
        );
    }
    if response.clicked() {
        ui.ctx().copy_text(copy.to_owned());
    }
}

/// The meta row's height.
const SPACE_LG_META: f32 = 16.0;

/// How a notice row reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tone {
    /// Something failed.
    Error,
    /// Sending is blocked, or something needs a look.
    Warning,
    /// A note.
    Hint,
}

/// A notice row at the top of the composer box: at least 32 px, 8 x 12
/// padding, 12 px text in its tone, a hairline under it. `trailing` draws
/// controls at its right end (a dismiss button, a retry).
/// In a narrow chat the controls go on a line of their own under the text,
/// so a couple of buttons cannot squeeze the text into a column of one
/// word per line.
pub(super) fn notice_row(
    ui: &mut Ui,
    theme: &Palette,
    text: &str,
    tone: Tone,
    trailing: impl FnOnce(&mut Ui),
) {
    // Below this width the controls go under the text.
    const STACK_BELOW: f32 = 420.0;
    let (glyph, color) = match tone {
        Tone::Error => (icons::CIRCLE_ALERT, theme::rgb(theme.danger)),
        Tone::Warning => (icons::TRIANGLE_ALERT, theme::rgb(theme.warning)),
        Tone::Hint => (icons::INFO, theme::rgb(theme.text_muted)),
    };
    let frame = egui::Frame::new()
        .inner_margin(theme::margin_xy(theme::SPACE_MD, SPACE_SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(theme::SPACE_LG);
            let label = |ui: &mut Ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(text)
                            .size(theme::BODY_SIZE)
                            .color(color),
                    )
                    .wrap(),
                );
            };
            if ui.available_width() < STACK_BELOW {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_SM;
                    icons::icon(ui, glyph, icons::SMALL, color);
                    label(ui);
                });
                // Top-aligned, so an empty `trailing` takes no height.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), trailing);
                return;
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_SM;
                icons::icon(ui, glyph, icons::SMALL, color);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    trailing(ui);
                    ui.with_layout(
                        egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
                        label,
                    );
                });
            });
        });
    let rect = frame.response.rect;
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, theme::rgb(theme.raised)),
    );
}

/// A 32 px ghost trigger with a label and a chevron (the model picker):
/// 12 px Medium muted, `secondary` under the pointer or while `open`.
pub(super) fn picker(
    ui: &mut Ui,
    theme: &Palette,
    label: &str,
    name: &str,
    open: bool,
) -> Response {
    let font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    let max = (ui.available_width() - 48.0).max(48.0);
    let mut job = LayoutJob::simple_singleline(label.to_owned(), font, Color32::PLACEHOLDER);
    job.wrap.max_width = max - SPACE_XS - 12.0 - theme::SPACE_MD * 2.0;
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.painter().layout_job(job);
    let size = Vec2::new(
        galley.size().x + SPACE_XS + 12.0 + theme::SPACE_MD * 2.0 - SPACE_XS,
        32.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, name));
    let lift = widgets::hover_t(ui, response.id, response.hovered() || open);
    if lift > 0.0 {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(4),
            theme::rgb(theme.raised).gamma_multiply(lift),
        );
    }
    let ink = widgets::blend(theme.text_muted, theme.text, lift);
    let x = rect.left() + theme::SPACE_MD - 2.0;
    let text_h = galley.size().y;
    let text_w = galley.size().x;
    ui.painter()
        .galley(egui::pos2(x, rect.center().y - text_h / 2.0), galley, ink);
    let slot = Rect::from_min_size(
        egui::pos2(x + text_w + SPACE_XS, rect.center().y - 6.0),
        Vec2::splat(12.0),
    );
    icons::paint(ui.painter(), slot, icons::CHEVRON_DOWN, 12.0, ink);
    widgets::focus_ring(ui, theme, &response);
    response
}

/// The icon for an autocomplete row, by the kind of thing its token names.
pub(super) fn token_icon(token: &str) -> char {
    let kind = token
        .trim_start_matches(['+', '!', '>', '~', '*'])
        .trim_start_matches('@')
        .split(':')
        .next()
        .unwrap_or_default();
    match kind {
        "shot" => icons::FILM,
        "person" => icons::USER,
        "bars" => icons::MUSIC,
        "plan" => icons::LAYERS,
        "seg" | "clip" | "frame" => icons::VIDEO,
        _ => icons::AT_SIGN,
    }
}

/// One autocomplete row: 32 px, a 16 px icon, the label in 14 px, the
/// detail right-aligned in 12 px muted; `accent` when `active`.
pub(super) fn completion_row(
    ui: &mut Ui,
    theme: &Palette,
    glyph: char,
    label: &str,
    detail: &str,
    active: bool,
) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::MenuItem, true, active, label));
    let painter = ui.painter();
    if active {
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS),
            theme::accent_fill(theme),
        );
    }
    let slot = Rect::from_min_size(
        egui::pos2(rect.left() + SPACE_SM, rect.center().y - icons::SIZE / 2.0),
        Vec2::splat(icons::SIZE),
    );
    icons::paint(
        painter,
        slot,
        glyph,
        icons::SIZE,
        theme::rgb(theme.text_muted),
    );
    let label_x = slot.right() + SPACE_SM;
    let label_galley = painter.layout_no_wrap(
        label.to_owned(),
        theme::font(theme::LARGE_SIZE),
        theme::rgb(theme.text),
    );
    let label_w = label_galley.size().x;
    painter.galley(
        egui::pos2(label_x, rect.center().y - label_galley.size().y / 2.0),
        label_galley,
        theme::rgb(theme.text),
    );
    let room = rect.right() - SPACE_SM - (label_x + label_w + SPACE_SM);
    if room > 24.0 && !detail.is_empty() {
        let mut job = LayoutJob::simple_singleline(
            detail.to_owned(),
            theme::font(theme::BODY_SIZE),
            theme::rgb(theme.text_muted),
        );
        job.wrap.max_width = room;
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        let galley = painter.layout_job(job);
        painter.galley(
            egui::pos2(
                rect.right() - SPACE_SM - galley.size().x,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            theme::rgb(theme.text_muted),
        );
    }
    response
}

/// Paints `text` centred in `rect` (a numbered option's circle).
pub(super) fn centred(ui: &Ui, rect: Rect, text: &str, size: f32, color: Color32) {
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        theme::font(size),
        color,
    );
}

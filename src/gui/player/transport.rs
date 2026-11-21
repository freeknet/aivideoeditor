// ============================================================================
// Module:       gui::player::transport
// Description:  The transport under the viewer, as Selects draws it: a 4 px
//               scrub bar on its top edge, play/skip/loop as ghost icon
//               buttons, the shuttle speed, a centred timecode in fixed
//               digit cells, and a slot on the right for settings.
//
// Dependencies: egui; super (Player), super::clock,
//               crate::gui::ui::{icons, theme, widgets}, crate::theme
// ============================================================================

//! The transport bar.
//!
//! ```text
//! ════════════════════════════════════════════════════════════  4 px scrub
//! [⏮][▶][⏭] 1x        00:00:03:12 / 00:00:12:00        [⟲][⚙]   48 px
//! ```
//!
//! The scrub bar has no knob: a `#404040` track, the played part white,
//! 6 px under the pointer. It shows the marked I/O range as a lighter band
//! with key-coloured edges and a cut's shot boundaries as hairline gaps;
//! dragging scrubs from thumbnails, releasing lands the decoder on the
//! exact frame. Its hit area reaches 20 px up into the picture, so it adds
//! no height. The timecode is laid out a digit per fixed cell, because egui
//! cannot switch on Inter's tabular figures and a running clock must not
//! jitter. There is no volume slider in the row (Selects has none): it
//! lives in the settings menu the caller puts on the right.

use egui::{Align, Color32, CornerRadius, Layout, Rect, Sense, Ui, Vec2, pos2, vec2};

use super::{Player, clock};
use crate::gui::ui::{icons, theme, widgets};
use crate::theme::Palette;

/// Height of the transport row (Selects' `h-12`).
pub(crate) const HEIGHT: f32 = 48.0;

/// The scrub bar's hit area, reaching up into the picture.
const SCRUB_HIT: f32 = 20.0;

/// The scrub bar's thickness, and under the pointer.
const SCRUB: f32 = 4.0;
const SCRUB_HOT: f32 = 6.0;

/// The closest two shot boundaries are drawn on the scrub bar, points; a
/// cut whose shots average under twice this apart draws none.
const MIN_TICK_GAP: f32 = 6.0;

/// The timecode's size (`text-sm`).
const TIMECODE_SIZE: f32 = 14.0;

/// Shuttle speeds offered by the speed menu.
const SPEEDS: [f64; 4] = [1.0, 2.0, 4.0, 8.0];

/// How far a bar may reach past `ui`'s edge into the panel's inset.
fn bleed(gap: f32) -> f32 {
    gap.clamp(0.0, theme::PANEL_INSET)
}

/// The height [`bar`] takes from `ui` when it is the last thing in it:
/// the row sits on the panel's floor, so the part of it that falls in the
/// panel's bottom inset is not taken from the content.
pub(crate) fn reserved(ui: &Ui) -> f32 {
    let gap = ui.clip_rect().bottom() - ui.available_rect_before_wrap().bottom();
    HEIGHT - bleed(gap)
}

/// Draws the bar at the bottom of what is left of `ui`: the scrub bar on
/// its top edge, transport on the left, the timecode centred, and `right`
/// laid out right to left at the far end (after the loop toggle).
pub(crate) fn bar(
    p: &mut Player,
    ui: &mut Ui,
    theme: &Palette,
    right: impl FnOnce(&mut Ui, &mut Player),
) {
    let fps = p.fps();
    let available = ui.available_rect_before_wrap();
    let (_, _) = ui.allocate_exact_size(vec2(available.width(), reserved(ui)), Sense::hover());
    // Bleed across the panel's inset, as a header does, so the bar runs
    // edge to edge and the row sits on the panel's floor.
    let clip = ui.clip_rect();
    let left = available.left() - bleed(available.left() - clip.left());
    let right_edge = available.right() + bleed(clip.right() - available.right());
    let top = available.top();
    let strip = Rect::from_min_max(pos2(left, top), pos2(right_edge, top + HEIGHT));
    let row = Rect::from_min_max(
        pos2(strip.left() + theme::SPACE_SM, strip.top()),
        pos2(strip.right() - theme::SPACE_SM, strip.top() + HEIGHT),
    );

    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(Layout::left_to_right(Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 2.0;
    let playing = p.playing();
    let starts = p.seg_starts();
    let duration = p.duration();
    // The timecode is centred on the row whatever the side groups hold; a
    // narrow viewer (the 9:16 column) sheds the skip buttons and the speed,
    // then the length, rather than draw them under it.
    let now = clock::timecode(p.position(), fps);
    let full = format!("{now} / {}", clock::timecode(duration, fps));
    let painter = ui.painter_at(row);
    let fit = Fit::choose(row.width(), timecode_width(&painter, &full));
    if fit == Fit::Full
        && button(
            &mut bar,
            theme,
            icons::SKIP_BACK,
            "Previous shot or start",
            "",
            false,
        )
        .clicked()
    {
        p.seek(skip_target(p.position(), &starts, duration, -1.0));
    }
    let (glyph, label) = if playing {
        (icons::PAUSE, "Pause")
    } else {
        (icons::PLAY, "Play")
    };
    if play_button(&mut bar, theme, glyph, label).clicked() {
        p.set_rate(1.0);
        p.toggle();
    }
    if fit == Fit::Full
        && button(
            &mut bar,
            theme,
            icons::SKIP_FORWARD,
            "Next shot or end",
            "",
            false,
        )
        .clicked()
    {
        p.seek(skip_target(p.position(), &starts, duration, 1.0));
    }
    if fit != Fit::Minimal {
        speed(&mut bar, theme, p);
    }
    bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        right(ui, p);
        let looping = p.looping();
        if button(ui, theme, icons::REPEAT, "Loop", "Ctrl+L", looping).clicked() {
            p.set_looping(!looping);
        }
    });

    let text = if fit == Fit::Minimal { now } else { full };
    let shown = paint_timecode(&painter, row.center(), &text, theme::rgb(theme.text_muted));
    let id = ui.scope_id().with("transport-timecode");
    let response = ui.interact(shown, id, Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
    let _ = response.on_hover_text("Playhead position / length");

    scrub_bar(p, ui, theme, strip, fps);
}

/// How much of the row fits beside a centred timecode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fit {
    /// Skip, play, skip and speed on the left; `now / length`.
    Full,
    /// Play and speed; `now / length`.
    Compact,
    /// Play only; `now`.
    Minimal,
}

impl Fit {
    /// The most that fits a row `width` wide with a `timecode` that wide.
    fn choose(width: f32, timecode: f32) -> Self {
        // The right group: loop and one settings button.
        const RIGHT: f32 = theme::BAR_CONTROL * 2.0 + 2.0;
        // The left group, full and compact (speed counted at its minimum).
        const FULL: f32 = theme::BAR_CONTROL * 4.0 + 6.0;
        const COMPACT: f32 = theme::BAR_CONTROL * 2.0 + 2.0;
        let side = (width - timecode) / 2.0 - theme::SPACE_SM;
        if side >= FULL.max(RIGHT) {
            Self::Full
        } else if side >= COMPACT.max(RIGHT) {
            Self::Compact
        } else {
            Self::Minimal
        }
    }
}

/// A transport button: 36 px square, 8 px corners, a 16 px icon muted until
/// hovered, an ink-10 % hover; `on` shows the neutral selected fill and a
/// full-strength icon (a toggle that is on).
pub(crate) fn button(
    ui: &mut Ui,
    theme: &Palette,
    glyph: char,
    label: &str,
    keys: &str,
    on: bool,
) -> egui::Response {
    let rest = if on { theme.text } else { theme.text_muted };
    face(ui, theme, glyph, label, keys, on, rest, theme.text)
}

/// The play/pause button: a transport button whose icon is always the
/// foreground colour.
fn play_button(ui: &mut Ui, theme: &Palette, glyph: char, label: &str) -> egui::Response {
    face(
        ui, theme, glyph, label, "Space", false, theme.text, theme.text,
    )
}

/// Paints a transport button with the icon going from `rest` to `hot`.
#[expect(
    clippy::too_many_arguments,
    reason = "a button's look is these eight things; a struct for one private call site would only rename them"
)]
fn face(
    ui: &mut Ui,
    theme: &Palette,
    glyph: char,
    label: &str,
    keys: &str,
    on: bool,
    rest: crate::color::Rgb,
    hot: crate::color::Rgb,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(theme::BAR_CONTROL), Sense::click());
    response
        .widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on, label));
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    let alpha = if on {
        f32::from(theme::INK_12)
    } else {
        f32::from(theme::INK_10) * lift
    };
    if alpha > 0.0 {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_MD),
            theme::ink(theme, alpha.round() as u8),
        );
    }
    icons::paint(
        ui.painter(),
        rect,
        glyph,
        icons::SIZE,
        widgets::blend(rest, hot, lift),
    );
    widgets::focus_ring(ui, theme, &response);
    widgets::tip(response, theme, label, keys)
}

/// The shuttle speed: `1x` in 12 px muted text, opening the speeds; J/K/L
/// change it from the keyboard.
fn speed(ui: &mut Ui, theme: &Palette, p: &mut Player) {
    let text = speed_label(p.rate(), p.playing());
    let font = theme::font(theme::BODY_SIZE);
    let galley = ui
        .painter()
        .layout_no_wrap(text.clone(), font, Color32::PLACEHOLDER);
    let width = (galley.size().x + theme::SPACE_SM * 2.0).max(theme::BAR_CONTROL);
    let (rect, response) = ui.allocate_exact_size(vec2(width, theme::BAR_CONTROL), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Speed {text}"))
    });
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    if lift > 0.0 {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_MD),
            theme::ink(theme, (f32::from(theme::INK_10) * lift).round() as u8),
        );
    }
    let shuttling = text != "1x";
    let rest = if shuttling {
        theme.text
    } else {
        theme.text_muted
    };
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        widgets::blend(rest, theme.text, lift),
    );
    let response = widgets::tip(response, theme, "Speed", "J / K / L");
    let _ = egui::Popup::menu(&response)
        .width(120.0)
        .gap(theme::SPACE_XS)
        .align(egui::RectAlign::TOP_START)
        .show(|ui| {
            for rate in SPEEDS {
                let current = p.playing() && (p.rate() - rate).abs() < 1e-9;
                if widgets::menu_item(ui, theme, None, &format!("{rate:.0}x"), "", current)
                    .clicked()
                {
                    p.set_rate(rate);
                    p.play();
                    ui.close();
                }
            }
        });
}

/// The shuttle speed as Selects writes it: `1x` at normal play or paused,
/// else the speed with a minus for reverse (`2x`, `-8x`).
pub(crate) fn speed_label(rate: f64, playing: bool) -> String {
    if !playing || (rate - 1.0).abs() < 1e-9 {
        return "1x".to_owned();
    }
    let x = format!("{:.2}", rate.abs());
    let n = x.trim_end_matches('0').trim_end_matches('.');
    if rate < 0.0 {
        format!("-{n}x")
    } else {
        format!("{n}x")
    }
}

/// Where the skip buttons go from `pos`: the previous or next shot start
/// (`dir` negative or positive), else the start or the end. Going back
/// from just after a boundary skips to the one before it, as players do,
/// so a double press walks back shot by shot.
pub(crate) fn skip_target(pos: f64, starts: &[f64], duration: f64, dir: f64) -> f64 {
    // Within this of a boundary counts as on it.
    const NEAR: f64 = 0.25;
    if dir < 0.0 {
        starts
            .iter()
            .copied()
            .filter(|&t| t < pos - NEAR)
            .fold(0.0, f64::max)
    } else {
        starts
            .iter()
            .copied()
            .filter(|&t| t > pos + 1e-6)
            .fold(duration, f64::min)
    }
}

/// How wide [`paint_timecode`] lays `text` out.
fn timecode_width(painter: &egui::Painter, text: &str) -> f32 {
    let font = theme::tabular(painter.ctx(), TIMECODE_SIZE);
    let width = |c: char| {
        painter
            .layout_no_wrap(c.to_string(), font.clone(), Color32::PLACEHOLDER)
            .size()
            .x
    };
    let cell = ('0'..='9').map(width).fold(0.0, f32::max);
    text.chars()
        .map(|c| if c.is_ascii_digit() { cell } else { width(c) })
        .sum()
}

/// Paints `text` centred on `centre` in 14 px Inter, each digit in a cell
/// as wide as the widest digit so a running clock does not jitter; returns
/// the rect it covered.
pub(crate) fn paint_timecode(
    painter: &egui::Painter,
    centre: egui::Pos2,
    text: &str,
    color: Color32,
) -> Rect {
    let font = theme::tabular(painter.ctx(), TIMECODE_SIZE);
    let glyphs: Vec<_> = text
        .chars()
        .map(|c| painter.layout_no_wrap(c.to_string(), font.clone(), color))
        .collect();
    let cell = ('0'..='9')
        .map(|d| {
            painter
                .layout_no_wrap(d.to_string(), font.clone(), color)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let widths: Vec<f32> = text
        .chars()
        .zip(&glyphs)
        .map(|(c, g)| if c.is_ascii_digit() { cell } else { g.size().x })
        .collect();
    let total: f32 = widths.iter().sum();
    let height = glyphs.first().map_or(TIMECODE_SIZE, |g| g.size().y);
    let mut x = (centre.x - total / 2.0).round();
    let y = (centre.y - height / 2.0).round();
    for (g, w) in glyphs.into_iter().zip(widths) {
        let gx = x + (w - g.size().x) / 2.0;
        painter.galley(pos2(gx, y), g, color);
        x += w;
    }
    Rect::from_center_size(centre, vec2(total, height))
}

/// The scrub bar on the top edge of `strip`; dragging scrubs.
fn scrub_bar(p: &mut Player, ui: &mut Ui, theme: &Palette, strip: Rect, fps: f64) {
    let hit = Rect::from_min_max(
        pos2(strip.left(), strip.top() - SCRUB_HIT),
        pos2(strip.right(), strip.top()),
    );
    let id = ui.scope_id().with("transport-scrub");
    let response = ui.interact(hit, id, Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, "Playhead"));
    let d = p.duration().max(1e-3);
    let x_of = |t: f64| hit.left() + (t / d).clamp(0.0, 1.0) as f32 * hit.width();
    let t_of = |x: f32| f64::from(((x - hit.left()) / hit.width()).clamp(0.0, 1.0)) * d;
    if response.drag_started() || response.is_pointer_button_down_on() {
        p.set_scrubbing(true);
    }
    if (response.dragged() || response.clicked() || response.is_pointer_button_down_on())
        && let Some(at) = response.interact_pointer_pos()
    {
        p.seek(t_of(at.x));
    }
    if response.drag_stopped() || (!response.is_pointer_button_down_on() && !response.dragged()) {
        p.set_scrubbing(false);
    }
    let hot = widgets::hover_t(ui, id, response.hovered() || response.dragged());
    let thick = SCRUB + (SCRUB_HOT - SCRUB) * hot;
    let track = Rect::from_min_max(pos2(hit.left(), hit.bottom() - thick), hit.right_bottom());
    let painter = ui.painter_at(hit);
    painter.rect_filled(track, CornerRadius::ZERO, theme::accent_fill(theme));
    let x = x_of(p.position());
    painter.rect_filled(
        Rect::from_min_max(track.min, pos2(x, track.max.y)),
        CornerRadius::ZERO,
        theme::rgb(theme.text),
    );
    // The marked range: a lighter band over the bar, key-coloured edges.
    let (mi, mo) = p.marks();
    if let Some((i, o)) = p.marked() {
        let band = Rect::from_x_y_ranges(x_of(i)..=x_of(o), track.y_range());
        painter.rect_filled(
            band,
            CornerRadius::ZERO,
            theme::translucent(theme.accent, 90),
        );
    }
    let key = theme::rgb(theme.accent);
    for t in [mi, mo].into_iter().flatten() {
        let mx = x_of(t).round();
        painter.rect_filled(
            Rect::from_x_y_ranges((mx - 1.0)..=(mx + 1.0), track.y_range()),
            CornerRadius::ZERO,
            key,
        );
    }
    // A cut's shot boundaries: 1 px gaps in the panel colour, only where
    // they stand apart, and none at all for a tape of hundreds of shots
    // (they would only dash the bar).
    let gap = theme::rgb(theme.panel);
    let starts = p.seg_starts();
    let sparse = starts.len() as f32 * MIN_TICK_GAP * 2.0 <= track.width();
    let mut last = f32::NEG_INFINITY;
    for t in starts.into_iter().filter(|_| sparse) {
        let sx = x_of(t).round();
        if sx - last >= MIN_TICK_GAP {
            painter.vline(sx, track.y_range(), egui::Stroke::new(1.0, gap));
            last = sx;
        }
    }
    if response.hovered()
        && !response.dragged()
        && let Some(at) = response.hover_pos()
    {
        let t = t_of(at.x);
        painter.vline(
            at.x.round(),
            track.y_range(),
            egui::Stroke::new(1.0, theme::rgb(theme.text_muted)),
        );
        let _ = response.on_hover_text_at_pointer(clock::timecode(t, fps));
    }
}

/// A volume button opening [`volume_rows`]: the right-hand slot for a
/// transport with no settings of its own.
pub(crate) fn volume_menu(ui: &mut Ui, theme: &Palette, p: &mut Player) {
    let glyph = if p.muted() {
        icons::VOLUME_X
    } else {
        icons::VOLUME
    };
    let response = button(ui, theme, glyph, "Volume", "", false);
    let _ = egui::Popup::menu(&response)
        .width(220.0)
        .gap(theme::SPACE_XS)
        .align(egui::RectAlign::TOP_END)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| volume_rows(ui, theme, p));
}

/// The volume controls for a settings menu: a mute row with a check and a
/// slider cell. Changes go straight to the player.
pub(crate) fn volume_rows(ui: &mut Ui, theme: &Palette, p: &mut Player) {
    let muted = p.muted();
    let mut volume = p.volume();
    let glyph = if muted {
        icons::VOLUME_X
    } else {
        icons::VOLUME
    };
    if widgets::menu_item(ui, theme, Some(glyph), "Mute", "", muted).clicked() {
        p.set_volume(volume, !muted);
    }
    if slider_cell(ui, theme, "Volume", &mut volume) {
        p.set_volume(volume, false);
    }
}

/// Selects' slider cell: a 32 px field (6 px corners, ink-8 % fill), the
/// label on the left, the percentage on the right, the filled part
/// foreground at 10 % (15 % hovered, 20 % dragging). Returns whether
/// `value` (0..1) changed.
pub(crate) fn slider_cell(ui: &mut Ui, theme: &Palette, label: &str, value: &mut f32) -> bool {
    let width = ui.available_width().max(160.0);
    let (rect, response) = ui.allocate_exact_size(
        vec2(width, theme::CONTROL_HEIGHT + theme::SPACE_XS),
        Sense::click_and_drag(),
    );
    response.widget_info(|| egui::WidgetInfo::slider(true, f64::from(*value), label));
    let mut changed = false;
    if (response.dragged() || response.clicked())
        && let Some(at) = response.interact_pointer_pos()
    {
        let next = ((at.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        if (next - *value).abs() > f32::EPSILON {
            *value = next;
            changed = true;
        }
    }
    let radius = CornerRadius::same(theme::RADIUS);
    let painter = ui.painter();
    painter.rect_filled(rect, radius, theme::ink(theme, theme::INK_08));
    let alpha = if response.dragged() {
        theme::INK_20
    } else if response.hovered() {
        theme::INK_15
    } else {
        theme::INK_10
    };
    let filled = Rect::from_min_max(
        rect.min,
        pos2(
            rect.left() + rect.width() * value.clamp(0.0, 1.0),
            rect.max.y,
        ),
    );
    painter.rect_filled(filled, radius, theme::ink(theme, alpha));
    let inset = theme::SPACE_SM + 2.0;
    let font = theme::font(theme::BODY_SIZE);
    painter.text(
        pos2(rect.left() + inset, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font.clone(),
        theme::rgb(theme.text),
    );
    painter.text(
        pos2(rect.right() - inset, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{:.0} %", *value * 100.0),
        theme::tabular(ui.ctx(), theme::BODY_SIZE),
        theme::rgb(theme.text_muted),
    );
    changed
}

#[cfg(test)]
mod tests {
    use super::{Fit, skip_target, speed_label};

    #[test]
    fn a_narrow_row_sheds_controls_before_the_timecode() -> anyhow::Result<()> {
        assert_eq!(Fit::choose(900.0, 190.0), Fit::Full, "a wide viewer");
        assert_eq!(Fit::choose(420.0, 190.0), Fit::Compact, "the 9:16 column");
        assert_eq!(Fit::choose(300.0, 190.0), Fit::Minimal, "very narrow");
        Ok(())
    }

    #[test]
    fn the_speed_reads_like_selects() -> anyhow::Result<()> {
        assert_eq!(speed_label(1.0, true), "1x", "normal play");
        assert_eq!(speed_label(4.0, false), "1x", "paused");
        assert_eq!(speed_label(2.0, true), "2x", "fast forward");
        assert_eq!(speed_label(-8.0, true), "-8x", "fast reverse");
        assert_eq!(speed_label(-1.0, true), "-1x", "reverse");
        assert_eq!(speed_label(1.5, true), "1.5x", "fast review");
        assert_eq!(speed_label(1.04, true), "1.04x", "never a bare 1.0x");
        Ok(())
    }

    #[test]
    fn skipping_walks_shot_starts_then_the_ends() -> anyhow::Result<()> {
        let starts = [2.0, 5.0, 9.0];
        assert!(
            (skip_target(3.0, &starts, 12.0, 1.0) - 5.0).abs() < 1e-9,
            "next"
        );
        assert!(
            (skip_target(9.5, &starts, 12.0, 1.0) - 12.0).abs() < 1e-9,
            "end"
        );
        assert!(
            (skip_target(6.0, &starts, 12.0, -1.0) - 5.0).abs() < 1e-9,
            "back"
        );
        assert!(
            (skip_target(5.1, &starts, 12.0, -1.0) - 2.0).abs() < 1e-9,
            "just past a start goes to the one before"
        );
        assert!(skip_target(1.0, &starts, 12.0, -1.0).abs() < 1e-9, "start");
        assert!(
            (skip_target(4.0, &[], 12.0, 1.0) - 12.0).abs() < 1e-9,
            "a clip skips to its end"
        );
        Ok(())
    }
}

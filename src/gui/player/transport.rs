// ============================================================================
// Module:       gui::player::transport
// Description:  The transport bar under the viewer: play/pause, shuttle
//               speed, timecode, a scrub bar with the I/O range, loop and
//               volume, all painted (no glyph-font dependence).
//
// Dependencies: egui; super (Player), super::clock
// ============================================================================

//! The transport bar.
//!
//! One row: `[▶] [×2] 00:00:03:12 [──────●──────] 00:00:12:00 [⟲] [🔊 ▬▬]`.
//! The scrub bar shows the marked I/O range as a band, shot boundaries of a
//! cut as ticks, and the playhead as a knob; dragging it scrubs from
//! thumbnails, releasing lands the decoder on the exact frame.

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2};

use super::{Player, clock};

/// Colours for the bar.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Style {
    /// Played portion, playhead, active toggles.
    pub(crate) accent: Color32,
    /// Primary text and icons.
    pub(crate) text: Color32,
    /// Secondary text, idle icons.
    pub(crate) muted: Color32,
    /// The unplayed track.
    pub(crate) track: Color32,
    /// Button hover fill.
    pub(crate) hover: Color32,
}

impl Style {
    /// A style from the current egui visuals.
    pub(crate) fn from_visuals(v: &egui::Visuals) -> Self {
        Self {
            accent: v.selection.bg_fill,
            text: v.strong_text_color(),
            muted: v.weak_text_color(),
            track: v.widgets.inactive.bg_fill,
            hover: v.widgets.hovered.bg_fill,
        }
    }
}

/// Row height.
const HEIGHT: f32 = 28.0;

/// Draws the bar.
pub(crate) fn bar(p: &mut Player, ui: &mut Ui, style: &Style) {
    let fps = p.fps();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let playing = p.playing();
        let play = icon_button(ui, style, false, |painter, r, c| {
            if playing {
                paint_pause(painter, r, c);
            } else {
                paint_play(painter, r, c);
            }
        })
        .on_hover_text("Play / pause (Space)");
        if play.clicked() {
            p.set_rate(1.0);
            p.toggle();
        }
        let speed = speed_label(p.rate(), playing);
        ui.add_sized(
            vec2(34.0, HEIGHT),
            egui::Label::new(
                egui::RichText::new(speed)
                    .monospace()
                    .size(11.0)
                    .color(style.accent),
            ),
        )
        .on_hover_text("Shuttle: J / K / L, repeat for ×2 ×4 ×8");
        timecode_label(ui, style.text, &clock::timecode(p.position(), fps));
        // Everything right of the scrub bar has a fixed width.
        let right = 92.0 + 6.0 + HEIGHT + 6.0 + HEIGHT + 6.0 + 64.0 + 8.0;
        let width = (ui.available_width() - right).max(60.0);
        scrub_bar(p, ui, style, width, fps);
        timecode_label(ui, style.muted, &clock::timecode(p.duration(), fps));
        let looping = p.looping();
        let lp = icon_button(ui, style, looping, paint_loop).on_hover_text("Loop (Ctrl+L)");
        if lp.clicked() {
            p.set_looping(!looping);
        }
        let muted = p.muted();
        let volume = p.volume();
        let level = if muted { 0.0 } else { volume };
        let mute = icon_button(ui, style, false, move |painter, r, c| {
            paint_speaker(painter, r, c, level);
        })
        .on_hover_text(if muted { "Unmute" } else { "Mute" });
        if mute.clicked() {
            p.set_volume(volume, !muted);
        }
        if let Some(v) = volume_slider(ui, style, volume, muted) {
            p.set_volume(v, false);
        }
    });
}

/// The shuttle indicator: blank at ×1 forward, else direction and speed.
pub(crate) fn speed_label(rate: f64, playing: bool) -> String {
    if !playing || (rate - 1.0).abs() < 1e-9 {
        return String::new();
    }
    let x = rate.abs();
    let n = if (x - x.round()).abs() < 1e-9 {
        format!("{x:.0}")
    } else {
        format!("{x:.1}")
    };
    if rate < 0.0 {
        format!("«×{n}")
    } else {
        format!("×{n}»")
    }
}

/// A fixed-width monospace timecode.
fn timecode_label(ui: &mut Ui, color: Color32, text: &str) {
    ui.add_sized(
        vec2(92.0, HEIGHT),
        egui::Label::new(
            egui::RichText::new(text)
                .monospace()
                .size(12.0)
                .color(color),
        ),
    );
}

/// A square button whose face is painted by `face(painter, rect, colour)`.
fn icon_button(
    ui: &mut Ui,
    style: &Style,
    active: bool,
    face: impl FnOnce(&egui::Painter, Rect, Color32),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(HEIGHT), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, CornerRadius::same(4), style.hover);
    }
    let color = if active {
        style.accent
    } else if response.hovered() {
        style.text
    } else {
        style.muted
    };
    face(painter, rect.shrink(8.0), color);
    response
}

fn paint_play(painter: &egui::Painter, r: Rect, c: Color32) {
    let pts = vec![
        pos2(r.left() + 1.0, r.top()),
        pos2(r.right(), r.center().y),
        pos2(r.left() + 1.0, r.bottom()),
    ];
    painter.add(egui::Shape::convex_polygon(pts, c, Stroke::NONE));
}

fn paint_pause(painter: &egui::Painter, r: Rect, c: Color32) {
    let w = r.width() * 0.32;
    painter.rect_filled(
        Rect::from_min_size(r.min, vec2(w, r.height())),
        CornerRadius::same(1),
        c,
    );
    painter.rect_filled(
        Rect::from_min_size(pos2(r.right() - w, r.top()), vec2(w, r.height())),
        CornerRadius::same(1),
        c,
    );
}

fn paint_loop(painter: &egui::Painter, r: Rect, c: Color32) {
    let centre = r.center();
    let radius = r.width() * 0.45;
    let pts: Vec<Pos2> = (0..=20)
        .map(|k| {
            let a = 0.35 + k as f32 / 20.0 * (std::f32::consts::TAU - 0.9);
            centre + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    let tip = pts.last().copied().unwrap_or(centre);
    painter.add(egui::Shape::line(pts, Stroke::new(1.6, c)));
    painter.add(egui::Shape::convex_polygon(
        vec![
            tip + vec2(-3.5, -3.0),
            tip + vec2(3.0, -1.0),
            tip + vec2(-1.0, 3.5),
        ],
        c,
        Stroke::NONE,
    ));
}

fn paint_speaker(painter: &egui::Painter, r: Rect, c: Color32, level: f32) {
    let h = r.height();
    let body = vec![
        pos2(r.left(), r.top() + h * 0.33),
        pos2(r.left() + h * 0.3, r.top() + h * 0.33),
        pos2(r.left() + h * 0.62, r.top()),
        pos2(r.left() + h * 0.62, r.bottom()),
        pos2(r.left() + h * 0.3, r.bottom() - h * 0.33),
        pos2(r.left(), r.bottom() - h * 0.33),
    ];
    painter.add(egui::Shape::convex_polygon(body, c, Stroke::NONE));
    let centre = pos2(r.left() + h * 0.62, r.center().y);
    if level <= 0.0 {
        let x = r.right() - h * 0.2;
        let y = r.center().y;
        let d = h * 0.18;
        painter.line_segment(
            [pos2(x - d, y - d), pos2(x + d, y + d)],
            Stroke::new(1.4, c),
        );
        painter.line_segment(
            [pos2(x - d, y + d), pos2(x + d, y - d)],
            Stroke::new(1.4, c),
        );
        return;
    }
    let waves = if level > 0.5 { 2 } else { 1 };
    for w in 0..waves {
        let radius = h * (0.25 + 0.2 * w as f32);
        let pts: Vec<Pos2> = (0..=10)
            .map(|k| {
                let a = -0.9 + k as f32 / 10.0 * 1.8;
                centre + vec2(a.cos(), a.sin()) * radius
            })
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(1.3, c)));
    }
}

/// The scrub bar; dragging scrubs.
fn scrub_bar(p: &mut Player, ui: &mut Ui, style: &Style, width: f32, fps: f64) {
    let (rect, response) = ui.allocate_exact_size(vec2(width, HEIGHT), Sense::click_and_drag());
    let d = p.duration().max(1e-3);
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 8.0, 4.0));
    let x_of = |t: f64| track.left() + (t / d).clamp(0.0, 1.0) as f32 * track.width();
    let t_of = |x: f32| f64::from(((x - track.left()) / track.width()).clamp(0.0, 1.0)) * d;
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
    let painter = ui.painter();
    painter.rect_filled(track, CornerRadius::same(2), style.track);
    // The marked range, as a band taller than the track.
    let (mi, mo) = p.marks();
    if let Some((i, o)) = p.marked() {
        let band = Rect::from_min_max(
            pos2(x_of(i), rect.center().y - 7.0),
            pos2(x_of(o), rect.center().y + 7.0),
        );
        painter.rect_filled(
            band,
            CornerRadius::same(2),
            style.accent.gamma_multiply(0.28),
        );
    }
    for (mark, dir) in [(mi, 1.0_f32), (mo, -1.0)] {
        if let Some(t) = mark {
            let x = x_of(t);
            let y0 = rect.center().y - 8.0;
            let y1 = rect.center().y + 8.0;
            painter.line_segment([pos2(x, y0), pos2(x, y1)], Stroke::new(1.5, style.accent));
            painter.line_segment(
                [pos2(x, y0), pos2(x + 4.0 * dir, y0)],
                Stroke::new(1.5, style.accent),
            );
            painter.line_segment(
                [pos2(x, y1), pos2(x + 4.0 * dir, y1)],
                Stroke::new(1.5, style.accent),
            );
        }
    }
    // Shot boundaries of a cut.
    for t in p.seg_starts() {
        let x = x_of(t);
        painter.line_segment(
            [pos2(x, track.top() - 3.0), pos2(x, track.bottom() + 3.0)],
            Stroke::new(1.0, style.muted.gamma_multiply(0.7)),
        );
    }
    let x = x_of(p.position());
    let played = Rect::from_min_max(track.min, pos2(x, track.max.y));
    painter.rect_filled(played, CornerRadius::same(2), style.accent);
    let hot = response.hovered() || response.dragged();
    painter.circle_filled(
        pos2(x, rect.center().y),
        if hot { 6.0 } else { 4.5 },
        style.text,
    );
    if response.hovered()
        && !response.dragged()
        && let Some(at) = response.hover_pos()
    {
        let t = t_of(at.x);
        painter.line_segment(
            [
                pos2(at.x, track.top() - 4.0),
                pos2(at.x, track.bottom() + 4.0),
            ],
            Stroke::new(1.0, style.muted),
        );
        response.on_hover_text_at_pointer(clock::timecode(t, fps));
    }
}

/// The volume bar; returns a new volume when clicked or dragged.
fn volume_slider(ui: &mut Ui, style: &Style, volume: f32, muted: bool) -> Option<f32> {
    let (rect, response) = ui.allocate_exact_size(vec2(64.0, HEIGHT), Sense::click_and_drag());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width() - 6.0, 3.0));
    let painter = ui.painter();
    painter.rect_filled(track, CornerRadius::same(2), style.track);
    let level = if muted { 0.0 } else { volume };
    let x = track.left() + level * track.width();
    painter.rect_filled(
        Rect::from_min_max(track.min, pos2(x, track.max.y)),
        CornerRadius::same(2),
        if muted { style.muted } else { style.text },
    );
    painter.circle_filled(pos2(x, rect.center().y), 4.0, style.text);
    let changed = response.dragged() || response.clicked();
    let at = response.interact_pointer_pos()?;
    changed.then(|| ((at.x - track.left()) / track.width()).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::speed_label;

    #[test]
    fn the_speed_shows_only_off_normal_play() -> anyhow::Result<()> {
        assert_eq!(speed_label(1.0, true), "", "normal play is blank");
        assert_eq!(speed_label(4.0, false), "", "paused is blank");
        assert_eq!(speed_label(2.0, true), "×2»", "fast forward");
        assert_eq!(speed_label(-8.0, true), "«×8", "fast reverse");
        assert_eq!(speed_label(-1.0, true), "«×1", "reverse");
        Ok(())
    }
}

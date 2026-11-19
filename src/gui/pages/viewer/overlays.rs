// ============================================================================
// Module:       gui::pages::viewer::overlays
// Description:  Guides drawn over the viewer's frame: title/action safe
//               areas, the TikTok/Reels interface zones, the info card.
//
// Dependencies: egui
// ============================================================================

//! Overlay geometry and painting.
//!
//! The geometry is pure (fractions of the output frame) so it is tested on
//! its own; the painters only map it onto the frame's screen rect.
//!
//! Safe areas follow SMPTE ST 2046-1: action safe is the central 93 %,
//! title safe the central 90 %. The 9:16 interface zones are the union of
//! where TikTok and Instagram Reels put their own furniture over a vertical
//! video: the status/tabs strip at the top, caption, audio and account at
//! the bottom, and the like/comment/share column on the right.

use egui::{Color32, CornerRadius, Rect, Stroke, pos2};

/// Action-safe fraction of each side.
pub(crate) const ACTION_SAFE: f32 = 0.93;
/// Title-safe fraction of each side.
pub(crate) const TITLE_SAFE: f32 = 0.90;

/// A rectangle as fractions of the frame: `[x0, y0, x1, y1]`.
pub(crate) type Frac = [f32; 4];

/// The central `fraction` of a frame.
pub(crate) fn centred(fraction: f32) -> Frac {
    let m = (1.0 - fraction) / 2.0;
    [m, m, 1.0 - m, 1.0 - m]
}

/// Where vertical-video apps draw over the picture (9:16 only), with a
/// label for each.
pub(crate) fn social_zones() -> [(Frac, &'static str); 3] {
    [
        ([0.0, 0.0, 1.0, 0.11], "status · tabs"),
        ([0.0, 0.78, 1.0, 1.0], "caption · audio · account"),
        ([0.86, 0.42, 1.0, 0.78], "actions"),
    ]
}

/// Maps a fraction rect onto `frame`.
pub(crate) fn place(frame: Rect, f: Frac) -> Rect {
    Rect::from_min_max(
        pos2(
            frame.left() + f[0] * frame.width(),
            frame.top() + f[1] * frame.height(),
        ),
        pos2(
            frame.left() + f[2] * frame.width(),
            frame.top() + f[3] * frame.height(),
        ),
    )
}

/// Draws the action- and title-safe outlines.
pub(crate) fn paint_safe(painter: &egui::Painter, frame: Rect) {
    for (fraction, alpha, label) in [(ACTION_SAFE, 70, "action"), (TITLE_SAFE, 110, "title")] {
        let r = place(frame, centred(fraction));
        let stroke = Stroke::new(1.0, Color32::from_white_alpha(alpha));
        painter.rect_stroke(r, CornerRadius::ZERO, stroke, egui::StrokeKind::Inside);
        painter.text(
            r.left_top() + egui::vec2(4.0, 2.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(9.0),
            Color32::from_white_alpha(alpha),
        );
    }
    // Centre cross.
    let c = frame.center();
    let stroke = Stroke::new(1.0, Color32::from_white_alpha(60));
    painter.line_segment([c - egui::vec2(8.0, 0.0), c + egui::vec2(8.0, 0.0)], stroke);
    painter.line_segment([c - egui::vec2(0.0, 8.0), c + egui::vec2(0.0, 8.0)], stroke);
}

/// Draws the vertical-video interface zones as faint fills.
pub(crate) fn paint_social(painter: &egui::Painter, frame: Rect) {
    for (f, label) in social_zones() {
        let r = place(frame, f);
        painter.rect_filled(
            r,
            CornerRadius::ZERO,
            Color32::from_rgba_unmultiplied(255, 70, 90, 34),
        );
        painter.rect_stroke(
            r,
            CornerRadius::ZERO,
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 110, 120, 90)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(9.0),
            Color32::from_white_alpha(150),
        );
    }
}

/// Draws a small card of `lines` in the frame's top-left corner.
pub(crate) fn paint_info(painter: &egui::Painter, frame: Rect, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let font = egui::FontId::monospace(11.0);
    let galleys: Vec<_> = lines
        .iter()
        .map(|l| painter.layout_no_wrap(l.clone(), font.clone(), Color32::from_gray(230)))
        .collect();
    let w = galleys.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let line_h = galleys.first().map_or(13.0, |g| g.size().y);
    let card = Rect::from_min_size(
        frame.left_top() + egui::vec2(8.0, 8.0),
        egui::vec2(w + 14.0, line_h * galleys.len() as f32 + 10.0),
    );
    painter.rect_filled(card, CornerRadius::same(4), Color32::from_black_alpha(170));
    let mut y = card.top() + 5.0;
    for g in galleys {
        painter.galley(pos2(card.left() + 7.0, y), g, Color32::WHITE);
        y += line_h;
    }
}

#[cfg(test)]
mod tests {
    use super::{TITLE_SAFE, centred, place, social_zones};

    #[test]
    fn safe_areas_are_centred() -> anyhow::Result<()> {
        let f = centred(TITLE_SAFE);
        assert!((f[0] - 0.05).abs() < 1e-6, "5 % margin: {f:?}");
        assert!((f[2] - 0.95).abs() < 1e-6, "symmetric: {f:?}");
        let frame = egui::Rect::from_min_size(egui::pos2(100.0, 0.0), egui::vec2(200.0, 400.0));
        let r = place(frame, f);
        assert!(
            (r.left() - 110.0).abs() < 1e-3,
            "placed in the frame: {r:?}"
        );
        assert!((r.height() - 360.0).abs() < 1e-3, "90 % tall: {r:?}");
        Ok(())
    }

    #[test]
    fn social_zones_stay_inside_the_frame() -> anyhow::Result<()> {
        for (f, label) in social_zones() {
            assert!(
                f.iter().all(|v| (0.0..=1.0).contains(v)) && f[0] < f[2] && f[1] < f[3],
                "{label} is a proper rect inside the frame: {f:?}"
            );
        }
        Ok(())
    }
}

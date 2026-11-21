// ============================================================================
// Module:       gui::pages::review::survey
// Description:  Survey mode: the selected shots tiled n-up; drop the losers
//               until one remains, then go on to the next group.
//
// Dependencies: egui; super (Paint, Do), crate::gui::thumbs
// ============================================================================

//! Lightroom's Survey, for three or more contenders, on `G` (`N` is the
//! editor's snapping key).
//!
//! The tiles are the selection: dropping one (`X`, or its close button) takes it out
//! of the selection, so what is left is what `R` mentions in chat. Arrows
//! move the focus, which the viewer shows. Lightroom has no key for "the
//! next set of similar shots" and its users fake one with mouse macros;
//! here `]` / `[` load the next / previous *group* (one clip's results, at
//! most [`MAX`] of them) without leaving the mode.

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use super::{Do, Paint};
use crate::gui::thumbs::Thumbs;
use crate::gui::ui::icons;
use crate::gui::ui::theme::{self, SPACE_SM, SPACE_XS};

/// Most shots a group brings into the survey.
pub(super) const MAX: usize = 6;

/// Narrowest tile.
const TILE_MIN: f32 = 110.0;

/// Gap between tiles.
const GAP: f32 = theme::SPACE_MD;

/// The id line under a tile's thumbnail.
const LABEL_H: f32 = SPACE_XS + 15.0;

/// The shots being surveyed.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Survey {
    /// Segment ids, in result order.
    pub(super) ids: Vec<String>,
    /// The focused tile.
    pub(super) focus: usize,
    /// The group the shots came from, when they came from one.
    pub(super) group: Option<usize>,
    /// Tiles per row as last drawn (Up / Down move by a row).
    pub(super) columns: usize,
}

impl Survey {
    /// A survey of `ids` focused on `focus`.
    pub(super) fn new(ids: Vec<String>, focus: usize, group: Option<usize>) -> Self {
        let focus = focus.min(ids.len().saturating_sub(1));
        Self {
            ids,
            focus,
            group,
            columns: 1,
        }
    }

    /// The focused segment id.
    pub(super) fn focused(&self) -> Option<&str> {
        self.ids.get(self.focus).map(String::as_str)
    }

    /// Drops tile `at`, unless it is the last one. Returns whether it went.
    pub(super) fn remove(&mut self, at: usize) -> bool {
        if self.ids.len() <= 1 || at >= self.ids.len() {
            return false;
        }
        self.ids.remove(at);
        if self.focus > at || self.focus >= self.ids.len() {
            self.focus = self.focus.saturating_sub(1);
        }
        true
    }

    /// Moves the focus by `delta` tiles, stopping at the ends.
    pub(super) fn step(&mut self, delta: isize) {
        let last = self.ids.len().saturating_sub(1);
        self.focus = self.focus.saturating_add_signed(delta).min(last);
    }
}

/// The group `id` belongs to.
pub(super) fn group_of(groups: &[Vec<String>], id: &str) -> Option<usize> {
    groups.iter().position(|g| g.iter().any(|x| x == id))
}

/// The next group from `from` in direction `dir` with at least two shots
/// (the first or last such group when `from` is `None`).
pub(super) fn next_group(groups: &[Vec<String>], from: Option<usize>, dir: isize) -> Option<usize> {
    let worth = |k: &usize| groups.get(*k).is_some_and(|g| g.len() >= 2);
    match (from, dir >= 0) {
        (Some(f), true) => (f + 1..groups.len()).find(worth),
        (Some(f), false) => (0..f).rev().find(worth),
        (None, true) => (0..groups.len()).find(worth),
        (None, false) => (0..groups.len()).rev().find(worth),
    }
}

/// Draws the tiles. Returns how many fit in a row.
pub(super) fn draw(
    ui: &mut egui::Ui,
    paint: &Paint<'_>,
    thumbs: &mut Thumbs,
    survey: &Survey,
    dos: &mut Vec<Do>,
) -> usize {
    let theme = paint.theme;
    let n = survey.ids.len();
    let room = SPACE_XS;
    let w = (super::visible_width(ui) - super::scroll_bar_width(ui) - room * 2.0).max(60.0);
    let fit = (((w + GAP) / (TILE_MIN + GAP)).floor() as usize).max(1);
    // Two shots read best side by side, four as a square.
    let cols = if n <= 4 { fit.min(2) } else { fit.min(3) };
    let tile_w = (w - GAP * (cols as f32 - 1.0)) / cols as f32;
    let thumb_h = (tile_w * 9.0 / 16.0).round();
    let tile_h = thumb_h + LABEL_H;
    ui.add_space(SPACE_SM);
    if n == 1 {
        let keys = super::keys_of("review-next-group", paint.preset);
        ui.horizontal(|ui| {
            ui.add_space(room);
            icons::icon(
                ui,
                icons::CIRCLE_CHECK,
                icons::SMALL,
                theme::rgb(theme.success),
            );
            ui.label(
                egui::RichText::new("The keeper")
                    .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(theme.text)),
            )
            .on_hover_text(format!("P picks it; {keys} goes on to the next group"));
        });
        ui.add_space(SPACE_SM);
    }
    egui::ScrollArea::vertical()
        .id_salt("review-survey")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in 0..n.div_ceil(cols) {
                let (rect, _) =
                    ui.allocate_exact_size(vec2(w + room * 2.0, tile_h), Sense::hover());
                for c in 0..cols {
                    let i = row * cols + c;
                    let Some(id) = survey.ids.get(i) else {
                        break;
                    };
                    let r = Rect::from_min_size(
                        rect.min + vec2(room + c as f32 * (tile_w + GAP), 0.0),
                        vec2(tile_w, tile_h),
                    );
                    tile(ui, paint, thumbs, id, i, i == survey.focus, n > 1, r, dos);
                }
                ui.add_space(SPACE_SM);
            }
        });
    cols
}

/// One tile: the thumbnail (hover to scrub), a close button to drop it,
/// its id and quality.
#[expect(
    clippy::too_many_arguments,
    reason = "a tile is drawn from these and nothing else; a struct would only rename them"
)]
fn tile(
    ui: &mut egui::Ui,
    paint: &Paint<'_>,
    thumbs: &mut Thumbs,
    id: &str,
    i: usize,
    focused: bool,
    removable: bool,
    rect: Rect,
    dos: &mut Vec<Do>,
) {
    let theme = paint.theme;
    let thumb = Rect::from_min_size(rect.min, vec2(rect.width(), rect.height() - LABEL_H));
    let radius = CornerRadius::same(theme::RADIUS_XS);
    let response = ui.interact(
        rect,
        egui::Id::unique(("review-survey", id)),
        Sense::click(),
    );
    let painter = ui.painter_at(rect.expand(SPACE_XS + 2.0));
    if response.hovered() {
        painter.rect_filled(
            rect.expand(SPACE_XS),
            CornerRadius::same(theme::RADIUS),
            theme::ink(theme, theme::INK_HOVER),
        );
    }
    painter.rect_filled(thumb, radius, theme::rgb(theme.raised));
    let Some(s) = super::segment(paint.night, id) else {
        return;
    };
    let (t0, t1) = (s.seg.start, s.seg.end);
    let t = match response.hover_pos().filter(|p| thumb.contains(*p)) {
        Some(p) => t0 + (t1 - t0) * ((p.x - thumb.left()) / thumb.width()).clamp(0.0, 0.999),
        None => (t0 + t1) / 2.0,
    };
    if let Some(path) = super::source(paint.paths, &s.clip) {
        thumbs.paint(
            ui,
            thumb,
            path,
            f64::from(t),
            s.seg.cx,
            theme::rgb(theme.raised),
        );
    }
    if let Some(v) = paint.night.marks.get(&s.id).verdict {
        let (color, glyph) = super::verdict_style(theme, v);
        super::glass_badge(
            &painter,
            theme,
            thumb.left_top() + vec2(SPACE_XS, SPACE_XS),
            Align2::LEFT_TOP,
            Some((glyph, color)),
            "",
        );
    }
    super::glass_badge(
        &painter,
        theme,
        thumb.right_bottom() + vec2(-SPACE_XS, -SPACE_XS),
        Align2::RIGHT_BOTTOM,
        None,
        &super::duration(t1 - t0),
    );
    let mut dropped = false;
    if removable {
        let close = Rect::from_min_size(
            pos2(thumb.right() - SPACE_XS - 20.0, thumb.top() + SPACE_XS),
            vec2(20.0, 20.0),
        );
        let hit = ui
            .interact(
                close,
                egui::Id::unique(("review-survey-drop", id)),
                Sense::click(),
            )
            .on_hover_text(format!(
                "Drop it from the survey ({})",
                super::keys_of("review-survey-remove", paint.preset)
            ));
        let plate = if hit.hovered() {
            theme::translucent(theme.danger, 220)
        } else {
            theme::glass_fill(theme)
        };
        painter.rect_filled(close, CornerRadius::same(theme::RADIUS), plate);
        icons::paint(&painter, close, icons::X, icons::SMALL, Color32::WHITE);
        if hit.clicked() {
            dos.push(Do::SurveyRemove(Some(i)));
            dropped = true;
        }
    }
    let y = thumb.bottom() + SPACE_XS + 7.5;
    let q = format!("{:.2}", s.q);
    let q_font = theme::tabular(ui.ctx(), theme::SMALL_SIZE);
    super::text_in(
        &painter,
        pos2(rect.right() - 30.0, y),
        &q,
        q_font,
        theme::rgb(theme.text_faint),
        30.0,
    );
    super::text_in(
        &painter,
        pos2(rect.left() + 2.0, y),
        &s.id,
        theme::medium(ui.ctx(), theme::BODY_SIZE),
        theme::rgb(theme.text),
        rect.width() - 40.0,
    );
    if focused {
        painter.rect_stroke(
            thumb.expand(2.0),
            CornerRadius::same(theme::RADIUS_XS + 2),
            Stroke::new(2.0, theme::rgb(theme.accent)),
            StrokeKind::Outside,
        );
    }
    if response.clicked() && !dropped {
        dos.push(Do::SurveyFocus(i));
    }
}

#[cfg(test)]
mod tests {
    use super::{Survey, group_of, next_group};

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn dropping_tiles_stops_at_one_and_keeps_the_focus_sane() {
        let mut s = Survey::new(ids(&["a", "b", "c"]), 2, None);
        assert!(s.remove(2), "drops the focused last tile");
        assert_eq!(s.focused(), Some("b"), "focus falls back");
        assert!(s.remove(0), "drops one before the focus");
        assert_eq!(s.focused(), Some("b"), "focus stays on the same shot");
        assert!(!s.remove(0), "the last one stays");
        assert_eq!(s.ids, ids(&["b"]), "the keeper");
    }

    #[test]
    fn next_group_skips_singletons_both_ways() {
        let groups = vec![ids(&["a/1", "a/2"]), ids(&["b/1"]), ids(&["c/1", "c/2"])];
        assert_eq!(next_group(&groups, Some(0), 1), Some(2), "b has one shot");
        assert_eq!(next_group(&groups, Some(2), -1), Some(0), "and back");
        assert_eq!(next_group(&groups, Some(2), 1), None, "no more");
        assert_eq!(next_group(&groups, None, 1), Some(0), "from nowhere: first");
        assert_eq!(group_of(&groups, "c/2"), Some(2), "found by member");
    }
}

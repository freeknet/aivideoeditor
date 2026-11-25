// ============================================================================
// Module:       gui::pages::review::compare
// Description:  Compare mode: the select and a candidate two-up, with their
//               shot-log numbers and captions side by side.
//
// Dependencies: egui; super (Paint, Do), crate::gui::thumbs
// ============================================================================

//! Lightroom's Compare, for two contenders.
//!
//! The *select* is the shot to beat; the *candidate* is the one weighed
//! against it. Left / Right step the candidate through the results, Up
//! makes it the select (and the next result the candidate), and `P` / `X`
//! / `U` mark the candidate and move on. The numbers that differ are the
//! point: the better value of each measurement is drawn in green.

use crate::gui::ui::hint::Hint as _;
use egui::{Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, vec2};

use super::{Do, Paint};
use crate::gui::thumbs::Thumbs;
use crate::gui::ui::theme::{self, SPACE_SM, SPACE_XS};
use crate::index::Scored;
use crate::search::Metric;

/// The two shots being compared (segment ids).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Compare {
    /// The shot to beat.
    pub(super) select: String,
    /// The shot weighed against it.
    pub(super) candidate: String,
}

/// Panes sit side by side once each can be this wide.
const SIDE_BY_SIDE: f32 = 110.0;

/// The candidate `delta` results on from `from` in `order`, never the
/// select. When `from` is not among the results, the result after the
/// select (or the last one) is taken instead. `None` when the select is
/// the only result.
pub(super) fn next_candidate(
    order: &[String],
    select: &str,
    from: &str,
    delta: isize,
) -> Option<String> {
    let others: Vec<&String> = order.iter().filter(|id| *id != select).collect();
    let last = others.len().checked_sub(1)?;
    let at = match others.iter().position(|id| *id == from) {
        Some(p) => p.saturating_add_signed(delta).min(last),
        None => order
            .iter()
            .position(|id| id == select)
            .unwrap_or(0)
            .min(last),
    };
    others.get(at).map(|id| (*id).clone())
}

/// Draws the two-up and the numbers.
pub(super) fn draw(
    ui: &mut egui::Ui,
    paint: &Paint<'_>,
    thumbs: &mut Thumbs,
    cmp: &Compare,
    dos: &mut Vec<Do>,
) {
    let select = super::segment(paint.night, &cmp.select);
    let candidate = super::segment(paint.night, &cmp.candidate);
    egui::ScrollArea::vertical()
        .id_salt("review-compare")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let w = super::visible_width(ui);
            let side = w >= SIDE_BY_SIDE * 2.0 + SPACE_SM;
            let pane_w = if side { (w - SPACE_SM) / 2.0 } else { w };
            let panes = [
                ("Select", select, &cmp.select, false),
                ("Candidate", candidate, &cmp.candidate, true),
            ];
            if side {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_SM;
                    for (role, seg, id, focus) in panes {
                        ui.vertical(|ui| {
                            ui.set_width(pane_w);
                            pane(ui, paint, thumbs, role, seg, id, focus, pane_w, dos);
                        });
                    }
                });
            } else {
                for (role, seg, id, focus) in panes {
                    pane(ui, paint, thumbs, role, seg, id, focus, pane_w, dos);
                    ui.add_space(SPACE_XS);
                }
            }
            ui.add_space(SPACE_SM);
            if let (Some(a), Some(b)) = (select, candidate) {
                numbers(ui, paint, a, b);
            }
        });
}

/// One side: role, thumbnail, id and caption. Clicking it shows it in the
/// viewer.
#[expect(
    clippy::too_many_arguments,
    reason = "a pane is drawn from these and nothing else; a struct would only rename them"
)]
fn pane(
    ui: &mut egui::Ui,
    paint: &Paint<'_>,
    thumbs: &mut Thumbs,
    role: &str,
    seg: Option<&Scored>,
    id: &str,
    focus: bool,
    width: f32,
    dos: &mut Vec<Do>,
) {
    let theme = paint.theme;
    let role_color = if focus { theme.text } else { theme.text_muted };
    ui.label(
        egui::RichText::new(role)
            .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
            .color(theme::rgb(role_color)),
    );
    let room = SPACE_XS;
    let (outer, response) = ui.allocate_exact_size(
        vec2(
            width,
            ((width - room * 2.0) * 9.0 / 16.0).round() + room * 2.0,
        ),
        Sense::click(),
    );
    let rect = outer.shrink(room);
    let radius = CornerRadius::same(theme::RADIUS_XS);
    ui.painter()
        .rect_filled(rect, radius, theme::rgb(theme.raised));
    let Some(s) = seg else {
        ui.label(
            egui::RichText::new(format!("{id} is not in the shot log"))
                .color(theme::rgb(theme.text_muted)),
        );
        return;
    };
    let t = f64::from((s.seg.start + s.seg.end) / 2.0);
    if let Some(path) = super::source(paint.paths, &s.clip) {
        thumbs.paint(ui, rect, path, t, s.seg.cx, theme::rgb(theme.raised));
    }
    let painter = ui.painter_at(outer);
    if let Some(v) = paint.night.marks.get(&s.id).verdict {
        let (color, glyph) = super::verdict_style(theme, v);
        super::glass_badge(
            &painter,
            theme,
            rect.left_top() + vec2(SPACE_XS, SPACE_XS),
            Align2::LEFT_TOP,
            Some((glyph, color)),
            "",
        );
    }
    super::glass_badge(
        &painter,
        theme,
        rect.right_bottom() - vec2(SPACE_XS, SPACE_XS),
        Align2::RIGHT_BOTTOM,
        None,
        &super::duration(s.seg.end - s.seg.start),
    );
    if focus {
        painter.rect_stroke(
            rect.expand(2.0),
            CornerRadius::same(theme::RADIUS_XS + 2),
            Stroke::new(2.0, theme::rgb(theme.accent)),
            StrokeKind::Outside,
        );
    }
    if response.hint("Show in the viewer").clicked() {
        dos.push(Do::Preview(s.id.clone()));
    }
    let line = Rect::from_min_size(ui.cursor().min, vec2(width, 16.0));
    ui.allocate_rect(line, Sense::hover());
    super::text_in(
        ui.painter(),
        line.left_center() + vec2(room, 0.0),
        &s.id,
        theme::medium(ui.ctx(), theme::BODY_SIZE),
        theme::rgb(theme.text),
        width - room,
    );
    let caption = s.caption.as_ref().map_or_else(
        || format!("{} · {}", s.subject, s.seg.camera.label()),
        |c| c.summary.clone(),
    );
    ui.add(
        egui::Label::new(
            egui::RichText::new(caption)
                .size(theme::SMALL_SIZE)
                .color(theme::rgb(theme.text_muted)),
        )
        .wrap(),
    );
}

/// The shot-log numbers side by side, the better of each in green.
fn numbers(ui: &mut egui::Ui, paint: &Paint<'_>, a: &Scored, b: &Scored) {
    let theme = paint.theme;
    let night = paint.night;
    let len = |s: &Scored| s.seg.end - s.seg.start;
    // (label, select, candidate, higher is better)
    let rows = [
        ("quality", a.q, b.q, true),
        (
            "fire",
            night.metric(a, Metric::Fire),
            night.metric(b, Metric::Fire),
            true,
        ),
        (
            "LED",
            night.metric(a, Metric::Led),
            night.metric(b, Metric::Led),
            true,
        ),
        (
            "moving light",
            night.metric(a, Metric::Move),
            night.metric(b, Metric::Move),
            true,
        ),
        (
            "focus",
            night.metric(a, Metric::Focus),
            night.metric(b, Metric::Focus),
            true,
        ),
        // Length is not better or worse, only different.
        ("length s", len(a), len(b), false),
    ];
    let small = |text: String, color| {
        egui::RichText::new(text)
            .size(theme::BODY_SIZE)
            .color(color)
    };
    let muted = theme::rgb(theme.text_muted);
    let plain = theme::rgb(theme.text);
    let better = theme::rgb(theme.success);
    let tabular = theme::tabular(ui.ctx(), theme::BODY_SIZE);
    egui::Grid::new("review-compare-numbers")
        .num_columns(3)
        .spacing(vec2(SPACE_SM, 2.0))
        .show(ui, |ui| {
            ui.label(small(String::new(), muted));
            ui.label(small("select".to_owned(), muted));
            ui.label(small("candidate".to_owned(), plain));
            ui.end_row();
            for (label, va, vb, ranked) in rows {
                let color = |me: f32, other: f32| {
                    if ranked && me > other + 0.005 {
                        better
                    } else {
                        plain
                    }
                };
                ui.label(small(label.to_owned(), muted));
                ui.label(small(format!("{va:.2}"), color(va, vb)).font(tabular.clone()));
                ui.label(small(format!("{vb:.2}"), color(vb, va)).font(tabular.clone()));
                ui.end_row();
            }
            let moment = |s: &Scored| {
                s.caption.as_ref().map_or_else(String::new, |c| {
                    format!("{} · {}", c.effective_moment(), c.framing)
                })
            };
            let words = [
                (
                    "camera",
                    a.seg.camera.label().to_owned(),
                    b.seg.camera.label().to_owned(),
                ),
                ("moment", moment(a), moment(b)),
                ("person", night.person(a), night.person(b)),
                ("flags", a.flags.join(" "), b.flags.join(" ")),
            ];
            for (label, va, vb) in words {
                if va.is_empty() && vb.is_empty() {
                    continue;
                }
                let color = if label == "flags" {
                    theme::rgb(theme.warning)
                } else {
                    plain
                };
                ui.label(small(label.to_owned(), muted));
                ui.add(egui::Label::new(small(va, color)).wrap());
                ui.add(egui::Label::new(small(vb, color)).wrap());
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::next_candidate;

    fn order() -> Vec<String> {
        ["a", "b", "c", "d"].map(String::from).to_vec()
    }

    #[test]
    fn the_candidate_steps_around_the_select() {
        let o = order();
        assert_eq!(
            next_candidate(&o, "b", "b", 0).as_deref(),
            Some("c"),
            "first candidate: the result after the select"
        );
        assert_eq!(
            next_candidate(&o, "b", "a", 1).as_deref(),
            Some("c"),
            "stepping skips the select"
        );
        assert_eq!(
            next_candidate(&o, "b", "d", 1).as_deref(),
            Some("d"),
            "stops at the end"
        );
        assert_eq!(
            next_candidate(&o, "d", "d", 0).as_deref(),
            Some("c"),
            "a select at the end takes the one before"
        );
        assert_eq!(
            next_candidate(&["a".to_owned()], "a", "a", 1),
            None,
            "nothing to compare with"
        );
    }
}

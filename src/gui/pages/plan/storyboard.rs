// ============================================================================
// Module:       gui::pages::plan::storyboard
// Description:  The storyboard lane above V1: one card per shot (thumbnail,
//               beats, the plan's note) under a band naming the song section,
//               on the timeline's own time axis so it scrolls with it.
//
// Dependencies: egui; crate::gui::pages::plan
// ============================================================================

//! The storyboard row.
//!
//! The research's storyboard view, kept in the timeline instead of a page
//! of its own: the plan read as a board, each card saying *why* the shot is
//! there (its `note`), grouped under the song section (intro, build, drop,
//! …) it plays over. It shares the tracks' mapping from seconds to x, so
//! zooming and scrolling move it with V1. Clicking a card selects the shot
//! and seeks to it, like clicking it on V1.

use std::collections::BTreeSet;

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::timeline::Map;
use super::{Io, Shown, State};
use crate::gui::ui::theme;

/// Height of the section band at the top of the lane.
const BAND_H: f32 = 13.0;

/// Song sections overlapping the plan, as (name, start s, end s) relative to
/// the plan's start.
pub(super) fn sections(s: &Shown<'_>) -> Vec<(String, f32, f32)> {
    let origin = s.map.beat_time(s.plan.start_bar, 0);
    s.map
        .sections
        .iter()
        .filter(|sec| sec.to_bar >= s.plan.start_bar && sec.from_bar < s.plan.end_bar)
        .map(|sec| {
            let a = s.map.beat_time(sec.from_bar.max(s.plan.start_bar), 0) - origin;
            let b = s.map.beat_time((sec.to_bar + 1).min(s.plan.end_bar), 0) - origin;
            (sec.name.clone(), a, b)
        })
        .collect()
}

/// Draws the lane into `rect`.
pub(super) fn lane(
    state: &mut State,
    s: &Shown<'_>,
    io: &mut Io<'_>,
    ui: &mut Ui,
    rect: Rect,
    m: Map,
) {
    let t = s.theme;
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, theme::rgb(t.panel));
    let band = Rect::from_min_size(rect.min, vec2(rect.width(), BAND_H));
    let cards = Rect::from_min_max(pos2(rect.left(), band.bottom() + 1.0), rect.max);

    // Section bands and dividers.
    for (n, (name, a, b)) in sections(s).iter().enumerate() {
        let (x0, x1) = (m.x(*a), m.x(*b));
        if x1 < rect.left() || x0 > rect.right() {
            continue;
        }
        let r = Rect::from_min_max(pos2(x0, band.top()), pos2(x1, band.bottom()));
        let fill = if n % 2 == 0 {
            theme::translucent(t.accent, 34)
        } else {
            theme::translucent(t.accent, 18)
        };
        painter.rect_filled(r.shrink2(vec2(0.5, 0.0)), CornerRadius::ZERO, fill);
        let clip = painter.with_clip_rect(r.intersect(rect));
        clip.text(
            pos2(r.left().max(rect.left()) + 4.0, r.center().y),
            Align2::LEFT_CENTER,
            name.to_uppercase(),
            FontId::proportional(9.0),
            theme::rgb(t.text_muted),
        );
        painter.vline(
            x0,
            band.top()..=rect.bottom(),
            Stroke::new(1.0, theme::rgb(t.border_strong)),
        );
    }

    // Cards.
    let ratio = super::aspect_ratio(s.plan.aspect);
    let mut clicked = None;
    for (k, (shot, &(start, dur))) in s.plan.shots.iter().zip(&s.timing).enumerate() {
        let (x0, x1) = (m.x(start), m.x(start + dur));
        if x1 < rect.left() || x0 > rect.right() {
            continue;
        }
        let card = Rect::from_min_max(
            pos2(x0 + 1.5, cards.top() + 1.0),
            pos2(x1 - 1.5, cards.bottom() - 2.0),
        );
        if card.width() < 4.0 {
            continue;
        }
        let resp = ui
            .interact(card, egui::Id::unique("plan_story").with(k), Sense::click())
            .on_hover_text(if shot.note.is_empty() {
                format!("Shot {} · {} · no note", k + 1, shot.clip)
            } else {
                format!("Shot {} · {}\n{}", k + 1, shot.clip, shot.note)
            });
        if resp.clicked() {
            clicked = Some(k);
        }
        let selected = s.sel.contains(&k);
        painter.rect(
            card,
            CornerRadius::same(3),
            theme::rgb(if selected { t.selection } else { t.raised }),
            Stroke::new(
                if selected { 1.5 } else { 1.0 },
                theme::rgb(if selected {
                    t.accent
                } else if resp.hovered() {
                    t.border_strong
                } else {
                    t.border
                }),
            ),
            egui::StrokeKind::Inside,
        );
        let th_h = card.height() - 6.0;
        let th_w = (th_h * ratio).min(card.width() - 6.0);
        let mut text_left = card.left() + 4.0;
        if th_w >= 10.0 {
            let th = Rect::from_min_size(card.min + vec2(3.0, 3.0), vec2(th_w, th_h));
            if th.left() >= rect.left() && th.right() <= rect.right() {
                if let Some(path) = s.clip_path(&shot.clip) {
                    io.thumbs.paint(
                        ui,
                        th,
                        &path,
                        f64::from(shot.t_in),
                        shot.cx,
                        theme::rgb(t.hover),
                    );
                }
                text_left = th.right() + 4.0;
            }
        }
        let text = Rect::from_min_max(pos2(text_left, card.top() + 3.0), card.max - vec2(3.0, 2.0));
        if text.width() > 16.0 {
            let tp = painter.with_clip_rect(text.intersect(rect));
            let ink = theme::rgb(if selected { t.selection_text } else { t.text });
            tp.text(
                text.min,
                Align2::LEFT_TOP,
                format!("#{} · {}b", k + 1, shot.beats),
                FontId::proportional(10.0),
                ink,
            );
            if !shot.note.is_empty() {
                let galley = ui.painter().layout(
                    shot.note.clone(),
                    FontId::proportional(9.5),
                    theme::rgb(t.text_muted),
                    text.width().max(8.0),
                );
                tp.galley(text.min + vec2(0.0, 13.0), galley, Color32::PLACEHOLDER);
            }
        }
    }
    if let Some(k) = clicked {
        io.select(&s.name, &BTreeSet::from([k]));
        state.anchor = Some(k);
        if let Some(&(a, _)) = s.timing.get(k) {
            io.seek(s, a);
        }
    }
}

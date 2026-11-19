// ============================================================================
// Module:       gui::pages::plan::timeline
// Description:  The tracks: tool strip, track headers, ruler (seconds and
//               bars), V1 shots, A1 waveform, drop flag, playhead, scrollbar
//               and zoom, and the pointer edits on them.
//
// Dependencies: egui; crate::gui::pages::plan
// ============================================================================

//! Drawing and pointer handling for the tracks.
//!
//! Geometry is one mapping, `x = left + (t - scroll) · pps`, with `t` in
//! seconds from the plan's start bar. The beat grid comes from the music
//! map, so bars and beats sit where the song has them even when the tempo
//! drifts. Detail follows zoom: bar numbers thin out to every 2nd, 4th, 8th
//! bar, and beat ticks appear only when they are far enough apart to read.

use std::collections::BTreeSet;

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2,
};

use super::{Drag, Io, Shown, State, Tool, edit, waveform::Wave};
use crate::gui::ui::theme;
use crate::refs::{Modifier, Reference, Target};

/// Width of the tool strip.
const TOOLS_W: f32 = 26.0;
/// Width of the track headers.
const HEADS_W: f32 = 56.0;
/// Height of the ruler (bars, drops, seconds, then the comment pins).
const RULER_H: f32 = 40.0;
/// Where the seconds row of the ruler ends, from its top.
const SECONDS_BOTTOM: f32 = 30.0;
/// Height of the storyboard lane when shown.
const STORY_H: f32 = 64.0;
/// Pointer distance that grabs a comment pin.
const PIN_GRAB: f32 = 5.0;
/// Height of the scrollbar row.
const SCROLL_H: f32 = 16.0;
/// Width of the zoom controls at the bottom right.
const ZOOM_W: f32 = 92.0;
/// Pointer distance that grabs an edit point.
const EDGE_GRAB: f32 = 4.0;

/// What is under the pointer on V1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    /// The edit point before shot `n`.
    Edge(usize),
    /// Shot `n`.
    Shot(usize),
    /// The end card.
    Card,
    /// Nothing.
    None,
}

/// The horizontal mapping.
#[derive(Clone, Copy, Debug)]
pub(super) struct Map {
    /// Screen x of the lanes' left edge.
    left: f32,
    /// Seconds at the left edge.
    scroll: f32,
    /// Pixels per second.
    pps: f32,
}

impl Map {
    /// Screen x of `t` seconds.
    pub(super) fn x(self, t: f32) -> f32 {
        self.left + (t - self.scroll) * self.pps
    }
    /// Seconds at screen x.
    fn t(self, x: f32) -> f32 {
        self.scroll + (x - self.left) / self.pps
    }
}

/// The colour a shot's subject is tagged with.
fn subject_color(subject: &str) -> Color32 {
    match subject {
        "fire" => Color32::from_rgb(232, 128, 52),
        "led" => Color32::from_rgb(158, 118, 240),
        "white" => Color32::from_rgb(200, 210, 225),
        "silks" => Color32::from_rgb(64, 190, 178),
        _ => Color32::from_rgb(120, 124, 132),
    }
}

/// `m:ss` or `m:ss.s`.
pub(super) fn timecode(t: f32, fine: bool) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0).floor();
    let s = t - m * 60.0;
    if fine {
        format!("{m:.0}:{s:04.1}")
    } else {
        format!("{m:.0}:{:02.0}", s.floor())
    }
}

/// A small painted toggle; returns its response.
fn mini(
    ui: &mut Ui,
    s: &Shown<'_>,
    rect: Rect,
    id: egui::Id,
    label: &str,
    on: bool,
    tip: &str,
) -> egui::Response {
    let r = ui.interact(rect, id, Sense::click()).on_hover_text(tip);
    let t = s.theme;
    let fill = if on {
        theme::rgb(t.accent_soft)
    } else if r.hovered() {
        theme::rgb(t.hover)
    } else {
        theme::rgb(t.raised)
    };
    let p = ui.painter();
    p.rect(
        rect,
        CornerRadius::same(3),
        fill,
        Stroke::new(1.0, theme::rgb(t.border)),
        egui::StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(10.0),
        theme::rgb(if on { t.text } else { t.text_muted }),
    );
    r
}

/// Draws the tracks into `rect` and handles the pointer on them.
pub(super) fn tracks(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, ui: &mut Ui, rect: Rect) {
    let t = s.theme;
    let id = egui::Id::unique("plan_tracks");
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, theme::rgb(t.app));

    // Geometry.
    let lanes_left = rect.left() + TOOLS_W + HEADS_W;
    let story_h = if state.storyboard {
        // A third of the lanes at most, so V1 and A1 stay readable.
        let h = STORY_H.min((rect.height() - RULER_H - SCROLL_H) * 0.34);
        if h >= 30.0 { h } else { 0.0 }
    } else {
        0.0
    };
    let body = rect.height() - RULER_H - SCROLL_H - story_h;
    let v1_h = (body * 0.62).clamp(30.0, 110.0);
    let a1_h = (body - v1_h - 1.0).max(16.0);
    let ruler = Rect::from_min_max(
        pos2(lanes_left, rect.top()),
        pos2(rect.right(), rect.top() + RULER_H),
    );
    let story = Rect::from_min_size(
        pos2(lanes_left, ruler.bottom()),
        vec2(ruler.width(), story_h),
    );
    let v1 = Rect::from_min_size(pos2(lanes_left, story.bottom()), vec2(ruler.width(), v1_h));
    let a1 = Rect::from_min_size(
        pos2(lanes_left, v1.bottom() + 1.0),
        vec2(ruler.width(), a1_h),
    );
    let lanes = Rect::from_min_max(ruler.min, a1.max);
    let sb_row = Rect::from_min_max(
        pos2(lanes_left, a1.bottom()),
        pos2(rect.right(), rect.bottom()),
    );
    let sb = Rect::from_min_max(
        sb_row.min + vec2(0.0, 3.0),
        pos2(sb_row.right() - ZOOM_W, sb_row.bottom() - 3.0),
    );
    let total = s.total.max(0.1);

    // Zoom and scroll.
    let min_pps = lanes.width() / (total * 1.5);
    if state.view.fit {
        state.view.pps = lanes.width() / (total * 1.02);
        state.view.scroll = 0.0;
        state.view.fit = false;
    }
    if let Some(p) = ui.input(|i| i.pointer.hover_pos())
        && lanes.contains(p)
    {
        let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
        if (zoom - 1.0).abs() > 1e-4 {
            let at = Map {
                left: lanes.left(),
                scroll: state.view.scroll,
                pps: state.view.pps,
            }
            .t(p.x);
            state.view.pps = (state.view.pps * zoom).clamp(min_pps, 800.0);
            state.view.scroll = at - (p.x - lanes.left()) / state.view.pps;
            state.view.follow = false;
        } else if scroll.x != 0.0 || scroll.y != 0.0 {
            state.view.scroll -= (scroll.x + scroll.y) / state.view.pps;
            state.view.follow = false;
        }
    }
    state.view.pps = state.view.pps.clamp(min_pps, 800.0);
    let visible = lanes.width() / state.view.pps;
    if let Some((a, b)) = state.view.reveal.take()
        && (a < state.view.scroll || b > state.view.scroll + visible)
    {
        state.view.scroll = a - visible * 0.1;
    }
    // Follow the playhead while playing.
    let playhead = s.ours.then(|| io.player.position() as f32);
    if io.player.playing() && s.ours {
        if let Some(ph) = playhead
            && state.view.follow
            && (ph < state.view.scroll || ph > state.view.scroll + visible * 0.92)
        {
            state.view.scroll = ph - visible * 0.1;
        }
    } else {
        state.view.follow = true;
    }
    state.view.scroll = state.view.scroll.clamp(
        -visible * 0.05,
        (total - visible * 0.95).max(-visible * 0.05),
    );
    let m = Map {
        left: lanes.left(),
        scroll: state.view.scroll,
        pps: state.view.pps,
    };

    // Backgrounds.
    painter.rect_filled(ruler, CornerRadius::ZERO, theme::rgb(t.panel));
    for lane in [v1, a1] {
        painter.rect_filled(lane, CornerRadius::ZERO, theme::rgb(t.app));
    }
    let past_end = Rect::from_min_max(pos2(m.x(s.total).max(lanes.left()), v1.top()), a1.max);
    if past_end.width() > 0.0 {
        painter.rect_filled(past_end, CornerRadius::ZERO, Color32::from_black_alpha(70));
    }

    if story.height() > 20.0 {
        super::storyboard::lane(state, s, io, ui, story, m);
    }

    // Grid behind the lanes.
    let beat_px = s.grid.get(1).map_or(10.0, |b| b * m.pps);
    let lane_paint = painter.with_clip_rect(Rect::from_min_max(v1.min, a1.max));
    for (k, &bt) in s.grid.iter().enumerate() {
        let x = m.x(bt);
        if x < lanes.left() - 1.0 || x > lanes.right() + 1.0 {
            continue;
        }
        if k % 4 == 0 {
            lane_paint.vline(
                x,
                v1.top()..=a1.bottom(),
                Stroke::new(1.0, theme::translucent(t.border_strong, 140)),
            );
        } else if beat_px >= 9.0 {
            lane_paint.vline(
                x,
                v1.top()..=a1.bottom(),
                Stroke::new(1.0, theme::translucent(t.border, 90)),
            );
        }
    }

    // Pointer on V1.
    let v1_resp = ui.interact(v1, id.with("v1"), Sense::click_and_drag());
    let tool = state.tool;
    let hit_at = |p: Pos2| -> Hit {
        if !v1.contains(p) {
            return Hit::None;
        }
        if s.editable && tool == Tool::Select {
            for e in 1..s.plan.shots.len() {
                let x = m.x(s.timing[e].0);
                if (p.x - x).abs() <= EDGE_GRAB {
                    return Hit::Edge(e);
                }
            }
        }
        let tp = m.t(p.x);
        if tp >= s.card && tp < s.total {
            return Hit::Card;
        }
        s.timing
            .iter()
            .position(|&(a, d)| tp >= a && tp < a + d)
            .map_or(Hit::None, Hit::Shot)
    };
    let hover_hit = v1_resp.hover_pos().map_or(Hit::None, hit_at);
    let alt = ui.input(|i| i.modifiers.alt);
    let slipping = state.tool == Tool::Slip || alt;
    match (&state.drag, hover_hit) {
        (Some(Drag::Roll { .. }), _) | (None, Hit::Edge(_)) => {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        (Some(Drag::Slip { .. }), _) => ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn),
        (Some(Drag::Move { .. }), _) => ui.ctx().set_cursor_icon(CursorIcon::Grabbing),
        (None, Hit::Shot(_)) if s.editable && slipping => {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn)
        }
        (None, Hit::Shot(_)) if s.editable => ui.ctx().set_cursor_icon(CursorIcon::Grab),
        (Some(Drag::Frame { .. }), _) | (None, Hit::Shot(_) | Hit::Card | Hit::None) => {}
    }

    if v1_resp.drag_started() && s.editable {
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or_default();
        if let Some(plan) = state.plan.clone() {
            state.drag = match hit_at(origin) {
                Hit::Edge(e) => Some(Drag::Roll {
                    edit: e,
                    before: plan,
                }),
                Hit::Shot(k) if slipping => Some(Drag::Slip {
                    shot: k,
                    t0: plan.shots[k].t_in,
                    before: plan,
                }),
                Hit::Shot(k) => {
                    if !s.sel.contains(&k) {
                        io.select(&s.name, &BTreeSet::from([k]));
                    }
                    Some(Drag::Move { from: k, to: k })
                }
                Hit::Card | Hit::None => None,
            };
        }
    }
    if v1_resp.dragged()
        && let Some(p) = v1_resp.interact_pointer_pos()
    {
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or(p);
        match &mut state.drag {
            Some(Drag::Roll { edit: e, .. }) => {
                let e = *e;
                let b = edit::nearest_beat(&s.grid, m.t(p.x));
                let left = b.saturating_sub(s.starts[e - 1]);
                let changed = state
                    .plan
                    .as_mut()
                    .is_some_and(|plan| edit::roll(plan, e, left));
                if changed {
                    state.rev += 1;
                }
            }
            Some(Drag::Slip { shot, t0, .. }) => {
                let (k, t0) = (*shot, *t0);
                let dur = s.timing.get(k).map_or(0.0, |x| x.1);
                let len = s.plan.shots.get(k).and_then(|sh| s.clip_len(&sh.clip));
                let t_in = t0 - (p.x - origin.x) / m.pps;
                let changed = state
                    .plan
                    .as_mut()
                    .is_some_and(|plan| edit::slip(plan, k, t_in, dur, len));
                if changed {
                    state.rev += 1;
                }
            }
            Some(Drag::Move { to, .. }) => {
                let tp = m.t(p.x).clamp(0.0, s.card - 1e-3);
                if let Some(k) = s.timing.iter().position(|&(a, d)| tp >= a && tp < a + d) {
                    *to = k;
                }
            }
            Some(Drag::Frame { .. }) | None => {}
        }
    }
    if v1_resp.drag_stopped() {
        match state.drag.take() {
            Some(Drag::Roll { edit: e, before }) => {
                state.commit_drag(before, format!("roll {}|{}", e, e + 1));
            }
            Some(Drag::Slip { shot, before, .. }) => {
                state.commit_drag(before, format!("slip shot {}", shot + 1));
            }
            Some(Drag::Move { from, to }) => {
                if state.apply(format!("move shot {} to {}", from + 1, to + 1), |p| {
                    edit::reorder(p, from, to)
                }) {
                    io.select(&s.name, &BTreeSet::from([to]));
                }
            }
            other @ (Some(Drag::Frame { .. }) | None) => state.drag = other,
        }
    }
    if v1_resp.clicked() {
        let (shift, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
        match v1_resp.interact_pointer_pos().map_or(Hit::None, hit_at) {
            Hit::Shot(k) | Hit::Edge(k) => {
                let mut sel = s.sel.clone();
                if shift {
                    let a = state.anchor.unwrap_or(k);
                    sel = (a.min(k)..=a.max(k)).collect();
                } else if command {
                    if !sel.remove(&k) {
                        sel.insert(k);
                    }
                    state.anchor = Some(k);
                } else {
                    sel = BTreeSet::from([k]);
                    state.anchor = Some(k);
                }
                io.select(&s.name, &sel);
                if state
                    .alts
                    .as_ref()
                    .is_some_and(|a| Some(a.slot) != sel.first().copied() || sel.len() != 1)
                {
                    state.alts = None;
                }
            }
            Hit::Card | Hit::None => io.select(&s.name, &BTreeSet::new()),
        }
    }
    if v1_resp.double_clicked()
        && let Hit::Shot(k) = v1_resp.interact_pointer_pos().map_or(Hit::None, hit_at)
    {
        super::play_from(s, io, k);
    }
    if v1_resp.secondary_clicked()
        && let Hit::Shot(k) | Hit::Edge(k) =
            v1_resp.interact_pointer_pos().map_or(Hit::None, hit_at)
    {
        state.menu_shot = Some(k);
        if !s.sel.contains(&k) {
            io.select(&s.name, &BTreeSet::from([k]));
        }
    }

    // Shots.
    let clip_lanes = painter.with_clip_rect(lanes.intersect(rect));
    let v1_paint = painter.with_clip_rect(v1);
    let ratio = super::aspect_ratio(s.plan.aspect);
    let dim = state.solo_music;
    let dragging_from = match &state.drag {
        Some(Drag::Move { from, .. }) => Some(*from),
        Some(Drag::Roll { .. } | Drag::Slip { .. } | Drag::Frame { .. }) | None => None,
    };
    for (k, (shot, &(start, dur))) in s.plan.shots.iter().zip(&s.timing).enumerate() {
        let x0 = m.x(start);
        let x1 = m.x(start + dur);
        if x1 < lanes.left() || x0 > lanes.right() {
            continue;
        }
        let block = Rect::from_min_max(
            pos2(x0 + 0.5, v1.top() + 2.0),
            pos2(x1 - 0.5, v1.bottom() - 2.0),
        );
        let selected = s.sel.contains(&k);
        let problem = s.issues.contains_key(&k);
        let fill = if selected { t.selection } else { t.raised };
        v1_paint.rect_filled(block, CornerRadius::same(2), theme::rgb(fill));
        // Subject stripe on top.
        let stripe = Rect::from_min_max(block.min, pos2(block.right(), block.top() + 3.0));
        v1_paint.rect_filled(
            stripe,
            CornerRadius::ZERO,
            subject_color(s.subject(&shot.clip, shot.t_in)),
        );
        // Thumbnail at the in-point, framed for the plan's aspect.
        let th_h = block.height() - 4.0;
        let th_w = (th_h * ratio).min(block.width() - 4.0);
        let mut text_left = block.left() + 4.0;
        if !state.hide_thumbs && th_w >= 10.0 && th_h >= 14.0 {
            let th = Rect::from_min_size(
                pos2(block.left() + 2.0, block.top() + 3.0),
                vec2(th_w, th_h - 1.0),
            );
            if th.left() >= lanes.left() && th.right() <= lanes.right() {
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
        let text_rect = Rect::from_min_max(pos2(text_left, block.top() + 3.0), block.max);
        if text_rect.width() > 18.0 {
            let tp = v1_paint.with_clip_rect(text_rect.intersect(v1));
            let ink = theme::rgb(if selected { t.selection_text } else { t.text });
            tp.text(
                text_rect.min + vec2(0.0, 2.0),
                Align2::LEFT_TOP,
                &shot.clip,
                FontId::proportional(11.0),
                ink,
            );
            let mut line2 = format!("{}b", shot.beats);
            if !shot.who.is_empty() {
                line2.push_str(" · ");
                line2.push_str(&shot.who);
            }
            tp.text(
                text_rect.min + vec2(0.0, 16.0),
                Align2::LEFT_TOP,
                line2,
                FontId::proportional(10.0),
                theme::rgb(if selected {
                    t.selection_text
                } else {
                    t.text_muted
                }),
            );
            if text_rect.height() > 48.0 && !shot.note.is_empty() {
                tp.text(
                    text_rect.min + vec2(0.0, 30.0),
                    Align2::LEFT_TOP,
                    &shot.note,
                    FontId::proportional(9.5),
                    theme::rgb(t.text_faint),
                );
            }
            tp.text(
                pos2(text_rect.right() - 3.0, text_rect.top() + 2.0),
                Align2::RIGHT_TOP,
                (k + 1).to_string(),
                FontId::monospace(9.5),
                theme::rgb(t.text_faint),
            );
        }
        if problem {
            v1_paint.rect_stroke(
                block,
                CornerRadius::same(2),
                Stroke::new(1.0, theme::rgb(t.danger)),
                egui::StrokeKind::Inside,
            );
            let edge = Rect::from_min_max(pos2(block.left(), block.bottom() - 3.0), block.max);
            v1_paint.rect_filled(edge, CornerRadius::ZERO, theme::rgb(t.danger));
        }
        if selected {
            v1_paint.rect_stroke(
                block,
                CornerRadius::same(2),
                Stroke::new(2.0, theme::rgb(t.accent)),
                egui::StrokeKind::Inside,
            );
        } else if hover_hit == Hit::Shot(k) && state.drag.is_none() {
            v1_paint.rect_stroke(
                block,
                CornerRadius::same(2),
                Stroke::new(1.0, theme::rgb(t.border_strong)),
                egui::StrokeKind::Inside,
            );
        }
        if dim || dragging_from == Some(k) {
            v1_paint.rect_filled(block, CornerRadius::same(2), Color32::from_black_alpha(130));
        }
    }
    if let Hit::Edge(e) = hover_hit
        && state.drag.is_none()
    {
        let x = m.x(s.timing[e].0);
        v1_paint.vline(
            x,
            v1.top()..=v1.bottom(),
            Stroke::new(2.0, theme::rgb(t.accent)),
        );
    }
    if let Some(Drag::Roll { edit: e, .. }) = &state.drag
        && let Some(&(a, _)) = s.timing.get(*e)
    {
        let x = m.x(a);
        v1_paint.vline(
            x,
            v1.top()..=v1.bottom(),
            Stroke::new(2.0, theme::rgb(t.accent)),
        );
        let left = s.plan.shots.get(e - 1).map_or(0, |x| x.beats);
        let right = s.plan.shots.get(*e).map_or(0, |x| x.beats);
        v1_paint.text(
            pos2(x, v1.top() + 2.0),
            Align2::CENTER_TOP,
            format!(" {left} | {right} "),
            FontId::monospace(10.0),
            theme::rgb(t.accent),
        );
    }
    // Move: where it lands.
    if let Some(Drag::Move { from, to }) = &state.drag
        && from != to
        && let (Some(&(a, d)), Some(p)) = (s.timing.get(*to), v1_resp.interact_pointer_pos())
    {
        let slot = Rect::from_min_max(
            pos2(m.x(a), v1.top() + 1.0),
            pos2(m.x(a + d), v1.bottom() - 1.0),
        );
        v1_paint.rect_stroke(
            slot,
            CornerRadius::same(2),
            Stroke::new(2.0, theme::rgb(t.accent)),
            egui::StrokeKind::Inside,
        );
        if let Some(sh) = s.plan.shots.get(*from) {
            let ghost =
                Rect::from_center_size(pos2(p.x, v1.center().y), vec2(70.0, v1.height() * 0.6));
            v1_paint.rect_filled(
                ghost,
                CornerRadius::same(3),
                theme::translucent(t.accent, 90),
            );
            v1_paint.text(
                ghost.center(),
                Align2::CENTER_CENTER,
                &sh.clip,
                FontId::proportional(10.5),
                theme::rgb(t.text),
            );
        }
    }
    // Slip readout.
    if let Some(Drag::Slip { shot, .. }) = &state.drag
        && let (Some(sh), Some(&(a, _))) = (
            state.plan.as_ref().and_then(|p| p.shots.get(*shot)),
            s.timing.get(*shot),
        )
    {
        v1_paint.text(
            pos2(m.x(a) + 4.0, v1.bottom() - 4.0),
            Align2::LEFT_BOTTOM,
            format!("in {:.2}s", sh.t_in),
            FontId::monospace(10.0),
            theme::rgb(t.accent),
        );
    }

    // End card.
    let cx0 = m.x(s.card);
    let cx1 = m.x(s.total);
    if cx1 > lanes.left() && cx0 < lanes.right() {
        let block = Rect::from_min_max(
            pos2(cx0 + 0.5, v1.top() + 2.0),
            pos2(cx1 - 0.5, v1.bottom() - 2.0),
        );
        v1_paint.rect_filled(block, CornerRadius::same(2), theme::rgb(t.hover));
        let card_path = std::path::Path::new(&s.plan.end_card);
        let th_h = block.height() - 6.0;
        let th = Rect::from_min_size(
            block.min + vec2(3.0, 3.0),
            vec2((th_h * ratio).min(block.width() - 6.0), th_h),
        );
        let mut text_left = block.left() + 5.0;
        if !s.plan.end_card.is_empty()
            && card_path.exists()
            && th.width() > 10.0
            && th.left() >= lanes.left()
            && th.right() <= lanes.right()
        {
            io.thumbs
                .paint(ui, th, card_path, 0.0, 0.5, theme::rgb(t.raised));
            text_left = th.right() + 4.0;
        }
        let tp = v1_paint.with_clip_rect(block);
        tp.text(
            pos2(text_left, block.top() + 4.0),
            Align2::LEFT_TOP,
            "END CARD",
            FontId::proportional(10.5),
            theme::rgb(t.text_muted),
        );
        let file = card_path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "none set".to_owned());
        tp.text(
            pos2(text_left, block.top() + 18.0),
            Align2::LEFT_TOP,
            file,
            FontId::proportional(9.5),
            theme::rgb(t.text_faint),
        );
    }

    // Context menu.
    v1_resp.context_menu(|ui| {
        let Some(k) = state.menu_shot else {
            ui.close();
            return;
        };
        ui.set_min_width(190.0);
        ui.label(egui::RichText::new(format!("Shot {}", k + 1)).strong());
        if ui
            .add_enabled(s.editable, egui::Button::new("Alternatives…    A"))
            .clicked()
        {
            super::open_alternatives(state, s, k);
            ui.close();
        }
        if ui.button("Play from here").clicked() {
            super::play_from(s, io, k);
            ui.close();
        }
        if ui
            .add_enabled(s.editable, egui::Button::new("Comment on this shot"))
            .clicked()
        {
            let at = s.timing.get(k).map_or(0.0, |&(a, _)| a);
            super::comments::add_at(state, at);
            ui.close();
        }
        if let Some(sh) = s.plan.shots.get(k)
            && ui.button("Show source clip in viewer").clicked()
        {
            if let Some(path) = s.clip_path(&sh.clip) {
                let dur = s.timing.get(k).map_or(1.0, |x| x.1);
                io.player
                    .open_clip(&path, f64::from(sh.t_in), f64::from(sh.t_in + dur), sh.cx);
            }
            ui.close();
        }
        ui.menu_button("Reference in chat", |ui| {
            for modifier in Modifier::ALL {
                let label = if modifier.prefix().is_empty() {
                    modifier.label().to_owned()
                } else {
                    format!("{}  {}", modifier.prefix(), modifier.label())
                };
                if ui.button(label).clicked() {
                    let shots: Vec<usize> = if s.sel.contains(&k) {
                        s.sel.iter().copied().collect()
                    } else {
                        vec![k]
                    };
                    for n in shots {
                        io.actions.push(crate::gui::app::Action::AddRef(
                            Reference::new(Target::Shot {
                                plan: s.name.clone(),
                                index: n + 1,
                            })
                            .with(modifier),
                        ));
                    }
                    ui.close();
                }
            }
        });
        ui.separator();
        if ui
            .add_enabled(
                s.editable && s.plan.shots.len() > 1,
                egui::Button::new("Remove (beats to previous)    Del"),
            )
            .clicked()
        {
            if state.apply(format!("lift {}", k + 1), |p| edit::lift(p, k)) {
                io.select(&s.name, &BTreeSet::new());
                state.alts = None;
            }
            ui.close();
        }
    });
    // Tooltip.
    if state.drag.is_none()
        && let Hit::Shot(k) = hover_hit
        && let Some(sh) = s.plan.shots.get(k)
    {
        let dur = s.timing.get(k).map_or(0.0, |x| x.1);
        let issues = s.issues.get(&k).cloned().unwrap_or_default();
        v1_resp.on_hover_ui_at_pointer(|ui| {
            ui.set_max_width(320.0);
            ui.label(
                egui::RichText::new(format!("Shot {} · {} @ {:.2}s", k + 1, sh.clip, sh.t_in))
                    .strong(),
            );
            ui.label(format!(
                "{} beats · {dur:.2}s · cx {:.2}{}",
                sh.beats,
                sh.cx,
                if sh.who.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", sh.who)
                }
            ));
            if !sh.note.is_empty() {
                ui.label(egui::RichText::new(&sh.note).italics());
            }
            for msg in &issues {
                ui.label(egui::RichText::new(format!("• {msg}")).color(theme::rgb(t.danger)));
            }
        });
    }

    // A1: the waveform.
    let a1_resp = ui.interact(a1, id.with("a1"), Sense::click_and_drag());
    let a1_paint = painter.with_clip_rect(a1);
    let music_origin = s.music.as_ref().map_or(0.0, |(_, o)| *o as f32);
    let wave_color = if state.mute_music {
        theme::translucent(t.text_faint, 120)
    } else {
        theme::translucent(t.success, 150)
    };
    match s
        .music
        .as_ref()
        .map(|(path, _)| state.waves.get(ui.ctx(), path))
    {
        Some(Wave::Ready(peaks)) => {
            let mid = a1.center().y;
            let half = a1.height() * 0.42;
            let x_end = m.x(s.total).min(a1.right());
            let mut x = a1.left().max(m.x(0.0));
            while x < x_end {
                let ta = music_origin + m.t(x);
                let tb = music_origin + m.t(x + 1.0);
                if let Some((lo, hi)) = peaks.span(ta, tb) {
                    a1_paint.vline(
                        x + 0.5,
                        (mid - hi * half)..=(mid - lo * half).max(mid - hi * half + 1.0),
                        Stroke::new(1.0, wave_color),
                    );
                }
                x += 1.0;
            }
            a1_paint.hline(
                a1.left()..=x_end,
                mid,
                Stroke::new(1.0, theme::translucent(t.success, 60)),
            );
        }
        Some(Wave::Loading) => {
            a1_paint.text(
                a1.left_center() + vec2(8.0, 0.0),
                Align2::LEFT_CENTER,
                "reading the music…",
                FontId::proportional(10.5),
                theme::rgb(t.text_faint),
            );
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        Some(Wave::Failed(e)) => {
            a1_paint.text(
                a1.left_center() + vec2(8.0, 0.0),
                Align2::LEFT_CENTER,
                format!("no waveform: {e}"),
                FontId::proportional(10.5),
                theme::rgb(t.danger),
            );
        }
        None => {
            a1_paint.text(
                a1.left_center() + vec2(8.0, 0.0),
                Align2::LEFT_CENTER,
                "the plan names no music file",
                FontId::proportional(10.5),
                theme::rgb(t.text_faint),
            );
        }
    }
    let file = std::path::Path::new(&s.plan.music)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    a1_paint.text(
        pos2(a1.left().max(m.x(0.0)) + 4.0, a1.top() + 2.0),
        Align2::LEFT_TOP,
        file,
        FontId::proportional(9.5),
        theme::rgb(t.text_muted),
    );

    // Ruler.
    let ruler_resp = ui.interact(ruler, id.with("ruler"), Sense::click_and_drag());
    let show_resolved = state.show_resolved;
    let pin_at = |p: Pos2| -> Option<usize> {
        if p.y < ruler.top() + SECONDS_BOTTOM {
            return None;
        }
        s.plan
            .comments
            .iter()
            .enumerate()
            .filter(|(_, c)| show_resolved || !c.resolved)
            .map(|(i, c)| (i, (m.x(c.time) - p.x).abs()))
            .filter(|&(_, d)| d <= PIN_GRAB)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    let pin_clicked = ruler_resp
        .clicked()
        .then(|| ruler_resp.interact_pointer_pos().and_then(pin_at))
        .flatten();
    if let Some(i) = pin_clicked
        && let Some(c) = s.plan.comments.get(i)
    {
        io.seek(s, c.time.clamp(0.0, s.total));
        state.comment_sel = Some(i);
        state.show_comments = true;
    } else {
        for r in [&ruler_resp, &a1_resp] {
            if (r.clicked() || r.dragged())
                && let Some(p) = r.interact_pointer_pos()
            {
                io.seek(s, m.t(p.x).clamp(0.0, s.card));
            }
        }
    }
    let hover_pin = ruler_resp.hover_pos().and_then(pin_at);
    let rp = painter.with_clip_rect(ruler);
    rp.hline(
        ruler.left()..=ruler.right(),
        ruler.bottom() - 0.5,
        Stroke::new(1.0, theme::rgb(t.border)),
    );
    let bar_px = beat_px * 4.0;
    let step = [1_usize, 2, 4, 8, 16, 32]
        .into_iter()
        .find(|n| *n as f32 * bar_px >= 30.0)
        .unwrap_or(64);
    let bars = s.plan.end_bar.saturating_sub(s.plan.start_bar);
    for b in 0..=bars {
        let Some(&bt) = s.grid.get(b * 4) else { break };
        let x = m.x(bt);
        if x < ruler.left() - 30.0 || x > ruler.right() + 1.0 {
            continue;
        }
        let bar = s.plan.start_bar + b;
        if bar.is_multiple_of(step) || b == 0 {
            rp.vline(
                x,
                ruler.top()..=ruler.top() + 13.0,
                Stroke::new(1.0, theme::rgb(t.border_strong)),
            );
            rp.text(
                pos2(x + 3.0, ruler.top() + 1.0),
                Align2::LEFT_TOP,
                bar.to_string(),
                FontId::proportional(10.0),
                theme::rgb(t.text),
            );
        } else {
            rp.vline(
                x,
                ruler.top() + 8.0..=ruler.top() + 13.0,
                Stroke::new(1.0, theme::rgb(t.border_strong)),
            );
        }
    }
    if beat_px >= 6.0 {
        for (k, &bt) in s.grid.iter().enumerate() {
            if k % 4 != 0 {
                let x = m.x(bt);
                rp.vline(
                    x,
                    ruler.top() + 10.0..=ruler.top() + 13.0,
                    Stroke::new(1.0, theme::rgb(t.border)),
                );
            }
        }
    }
    let seconds_bottom = ruler.top() + SECONDS_BOTTOM;
    rp.hline(
        ruler.left()..=ruler.right(),
        seconds_bottom + 0.5,
        Stroke::new(1.0, theme::translucent(t.border, 120)),
    );
    let sec_step = [0.25_f32, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0]
        .into_iter()
        .find(|st| st * m.pps >= 54.0)
        .unwrap_or(120.0);
    let first = (m.t(ruler.left()) / sec_step).floor().max(0.0) as i64;
    let last = (m.t(ruler.right()) / sec_step).ceil() as i64;
    for n in first..=last {
        let ts = n as f32 * sec_step;
        if ts > s.total + 1e-3 {
            break;
        }
        let x = m.x(ts);
        rp.vline(
            x,
            seconds_bottom - 5.0..=seconds_bottom,
            Stroke::new(1.0, theme::rgb(t.text_faint)),
        );
        rp.text(
            pos2(x + 2.0, seconds_bottom - 2.0),
            Align2::LEFT_BOTTOM,
            timecode(ts, sec_step < 1.0),
            FontId::monospace(9.0),
            theme::rgb(t.text_faint),
        );
    }
    // Drops (the plan's, solid; the track's other candidates, hollow) and the card.
    let flag = |x: f32, color: Color32, solid: bool, label: &str| {
        if x < ruler.left() - 2.0 || x > ruler.right() + 2.0 {
            return;
        }
        let pts = vec![
            pos2(x, ruler.top() + 14.0),
            pos2(x + 9.0, ruler.top() + 17.5),
            pos2(x, ruler.top() + 21.0),
        ];
        if solid {
            rp.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
        } else {
            rp.add(egui::Shape::closed_line(pts, Stroke::new(1.0, color)));
        }
        rp.vline(
            x,
            ruler.top() + 13.0..=ruler.bottom(),
            Stroke::new(1.0, color),
        );
        if !label.is_empty() {
            rp.text(
                pos2(x + 11.0, ruler.top() + 17.5),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(9.0),
                color,
            );
        }
        lane_paint.vline(
            x,
            v1.top()..=a1.bottom(),
            Stroke::new(
                1.0,
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 70),
            ),
        );
    };
    let origin = s.map.beat_time(s.plan.start_bar, 0);
    for d in &s.map.drops {
        if d.bar != s.plan.drop_bar && d.bar >= s.plan.start_bar && d.bar <= s.plan.end_bar {
            flag(
                m.x(s.map.beat_time(d.bar, 0) - origin),
                theme::translucent(t.warning, 150),
                false,
                "",
            );
        }
    }
    flag(
        m.x(s.map.beat_time(s.plan.drop_bar, 0) - origin),
        theme::rgb(t.warning),
        true,
        "DROP",
    );
    flag(m.x(s.card), theme::rgb(t.text_muted), true, "CARD");

    // Comment pins, under the seconds.
    let pin_y = ruler.top() + SECONDS_BOTTOM + 1.0;
    for (i, c) in s.plan.comments.iter().enumerate() {
        if c.resolved && !state.show_resolved {
            continue;
        }
        let x = m.x(c.time);
        if x < ruler.left() - 6.0 || x > ruler.right() + 6.0 {
            continue;
        }
        let base = super::comments::color(&c.color);
        let ink = if c.resolved {
            base.gamma_multiply(0.35)
        } else {
            base
        };
        let head = Rect::from_min_size(pos2(x - 4.0, pin_y), vec2(8.0, 5.0));
        rp.rect_filled(head, CornerRadius::same(1), ink);
        rp.add(egui::Shape::convex_polygon(
            vec![
                pos2(x - 4.0, head.bottom()),
                pos2(x + 4.0, head.bottom()),
                pos2(x, head.bottom() + 3.5),
            ],
            ink,
            Stroke::NONE,
        ));
        if state.comment_sel == Some(i) || hover_pin == Some(i) {
            rp.rect_stroke(
                head.expand(1.5),
                CornerRadius::same(2),
                Stroke::new(1.0, theme::rgb(t.text)),
                egui::StrokeKind::Outside,
            );
        }
        lane_paint.vline(
            x,
            v1.top()..=a1.bottom(),
            Stroke::new(
                1.0,
                Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 60),
            ),
        );
    }
    if let Some(i) = hover_pin
        && let Some(c) = s.plan.comments.get(i)
    {
        let shot = super::comments::shot_at(&s.timing, c.time)
            .map_or_else(|| "end card".to_owned(), |k| format!("shot {}", k + 1));
        let note = if c.note.is_empty() {
            "(no note)".to_owned()
        } else {
            c.note.clone()
        };
        let head = format!(
            "{} · {shot} · {}{}",
            timecode(c.time, true),
            c.author,
            if c.resolved { " · resolved" } else { "" }
        );
        ruler_resp.clone().on_hover_ui_at_pointer(|ui| {
            ui.set_max_width(280.0);
            ui.label(egui::RichText::new(head).strong());
            ui.label(note);
        });
    }

    // Playhead.
    if let Some(ph) = playhead {
        let x = m.x(ph);
        if x >= lanes.left() && x <= lanes.right() {
            clip_lanes.vline(
                x,
                ruler.top()..=a1.bottom(),
                Stroke::new(1.5, theme::rgb(t.accent)),
            );
            clip_lanes.add(egui::Shape::convex_polygon(
                vec![
                    pos2(x - 6.0, ruler.top()),
                    pos2(x + 6.0, ruler.top()),
                    pos2(x, ruler.top() + 8.0),
                ],
                theme::rgb(t.accent),
                Stroke::NONE,
            ));
        }
    }

    // Scrollbar.
    painter.rect_filled(sb, CornerRadius::same(4), theme::rgb(t.panel));
    let span = total.max(visible);
    let thumb_w = (visible / span * sb.width()).clamp(24.0, sb.width());
    let thumb_x = sb.left() + (m.scroll.max(0.0) / span) * sb.width();
    let thumb = Rect::from_min_size(
        pos2(thumb_x.min(sb.right() - thumb_w), sb.top()),
        vec2(thumb_w, sb.height()),
    );
    let sb_resp = ui.interact(sb, id.with("scroll"), Sense::click_and_drag());
    painter.rect_filled(
        thumb,
        CornerRadius::same(4),
        theme::rgb(if sb_resp.hovered() || sb_resp.dragged() {
            t.border_strong
        } else {
            t.border
        }),
    );
    if sb_resp.dragged() {
        state.view.scroll += sb_resp.drag_delta().x / sb.width() * span;
        state.view.follow = false;
    } else if sb_resp.clicked()
        && let Some(p) = sb_resp.interact_pointer_pos()
    {
        state.view.scroll = (p.x - sb.left()) / sb.width() * span - visible / 2.0;
    }
    // Zoom controls.
    let zx = sb_row.right() - ZOOM_W + 6.0;
    let zy = sb_row.top() + 1.0;
    let bw = vec2(24.0, SCROLL_H - 2.0);
    let centre = m.t(lanes.center().x);
    if mini(
        ui,
        s,
        Rect::from_min_size(pos2(zx, zy), bw),
        id.with("zout"),
        "-",
        false,
        "Zoom out (Ctrl+wheel)",
    )
    .clicked()
    {
        state.view.pps = (state.view.pps / 1.5).max(min_pps);
        state.view.scroll = centre - lanes.width() / 2.0 / state.view.pps;
    }
    if mini(
        ui,
        s,
        Rect::from_min_size(pos2(zx + 27.0, zy), bw),
        id.with("zin"),
        "+",
        false,
        "Zoom in (Ctrl+wheel)",
    )
    .clicked()
    {
        state.view.pps = (state.view.pps * 1.5).min(800.0);
        state.view.scroll = centre - lanes.width() / 2.0 / state.view.pps;
    }
    if mini(
        ui,
        s,
        Rect::from_min_size(pos2(zx + 54.0, zy), vec2(30.0, bw.y)),
        id.with("zfit"),
        "Fit",
        false,
        "Fit the whole plan (Shift+Z)",
    )
    .clicked()
    {
        state.view.fit = true;
    }

    // Tool strip.
    let tools = Rect::from_min_max(rect.min, pos2(rect.left() + TOOLS_W, rect.bottom()));
    painter.rect_filled(tools, CornerRadius::ZERO, theme::rgb(t.panel));
    painter.vline(
        tools.right() - 0.5,
        tools.y_range(),
        Stroke::new(1.0, theme::rgb(t.border)),
    );
    for (n, (tool, tip)) in [
        (
            Tool::Select,
            "Select (V): click, drag to reorder, drag an edit to roll",
        ),
        (
            Tool::Slip,
            "Slip (T): drag a shot to move its in-point (or Alt-drag)",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let b = Rect::from_min_size(
            pos2(
                tools.left() + 3.0,
                rect.top() + RULER_H + 2.0 + n as f32 * 24.0,
            ),
            vec2(20.0, 20.0),
        );
        let on = state.tool == tool;
        if mini(ui, s, b, id.with(("tool", n)), "", on, tip).clicked() {
            state.tool = tool;
        }
        let c = b.center();
        let ink = theme::rgb(if on { t.text } else { t.text_muted });
        match tool {
            Tool::Select => {
                let pts = [
                    (-3.0, -6.0),
                    (4.0, 1.0),
                    (0.5, 1.2),
                    (2.5, 5.5),
                    (1.0, 6.2),
                    (-1.0, 2.0),
                    (-3.0, 4.0),
                ]
                .iter()
                .map(|&(dx, dy)| c + vec2(dx, dy))
                .collect();
                painter.add(egui::Shape::convex_polygon(pts, ink, Stroke::NONE));
            }
            Tool::Slip => {
                let st = Stroke::new(1.3, ink);
                painter.hline(c.x - 6.0..=c.x + 6.0, c.y, st);
                for dir in [-1.0_f32, 1.0] {
                    let tip = c + vec2(6.0 * dir, 0.0);
                    painter.line_segment([tip, tip + vec2(-3.0 * dir, -3.0)], st);
                    painter.line_segment([tip, tip + vec2(-3.0 * dir, 3.0)], st);
                }
                painter.vline(c.x - 2.0, c.y - 5.0..=c.y + 5.0, st);
                painter.vline(c.x + 2.0, c.y - 5.0..=c.y + 5.0, st);
            }
        }
    }

    // Track headers.
    let heads = Rect::from_min_max(
        pos2(tools.right(), rect.top()),
        pos2(lanes_left, rect.bottom()),
    );
    painter.rect_filled(heads, CornerRadius::ZERO, theme::rgb(t.panel));
    painter.vline(
        heads.right() - 0.5,
        heads.y_range(),
        Stroke::new(1.0, theme::rgb(t.border)),
    );
    painter.text(
        pos2(heads.left() + 6.0, ruler.center().y),
        Align2::LEFT_CENTER,
        super::aspect_key(s.plan.aspect),
        FontId::monospace(10.0),
        theme::rgb(t.text_muted),
    );
    let mut heads_rows = vec![(v1, "V1"), (a1, "A1")];
    if story.height() > 20.0 {
        heads_rows.insert(0, (story, "Board"));
    }
    for (lane, name) in heads_rows {
        let row = Rect::from_min_max(
            pos2(heads.left(), lane.top()),
            pos2(heads.right(), lane.bottom()),
        );
        painter.hline(
            row.x_range(),
            row.top() - 0.5,
            Stroke::new(1.0, theme::rgb(t.border)),
        );
        painter.text(
            row.left_top() + vec2(6.0, 4.0),
            Align2::LEFT_TOP,
            name,
            FontId::proportional(11.0),
            theme::rgb(t.text),
        );
    }
    let gy = v1.top() + 20.0;
    if v1.height() >= 40.0 {
        let show = !state.hide_thumbs;
        if mini(
            ui,
            s,
            Rect::from_min_size(pos2(heads.left() + 6.0, gy), vec2(20.0, 15.0)),
            id.with("thumbs"),
            "T",
            show,
            "Thumbnails on V1",
        )
        .clicked()
        {
            state.hide_thumbs = !state.hide_thumbs;
        }
    }
    let ay = (a1.top() + 18.0).min(a1.bottom() - 15.0);
    if mini(
        ui,
        s,
        Rect::from_min_size(pos2(heads.left() + 6.0, ay), vec2(20.0, 15.0)),
        id.with("mute"),
        "M",
        state.mute_music,
        "Mute A1 (dims the waveform; the viewer previews without sound)",
    )
    .clicked()
    {
        state.mute_music = !state.mute_music;
    }
    if mini(
        ui,
        s,
        Rect::from_min_size(pos2(heads.left() + 29.0, ay), vec2(20.0, 15.0)),
        id.with("solo"),
        "S",
        state.solo_music,
        "Solo A1 (dims V1 to read the music)",
    )
    .clicked()
    {
        state.solo_music = !state.solo_music;
    }
}

#[cfg(test)]
mod tests {
    use super::{Map, timecode};

    #[test]
    fn the_mapping_round_trips() -> anyhow::Result<()> {
        let m = Map {
            left: 100.0,
            scroll: 2.0,
            pps: 50.0,
        };
        assert!((m.x(2.0) - 100.0).abs() < 1e-4, "scroll at the left edge");
        assert!((m.t(m.x(7.3)) - 7.3).abs() < 1e-4, "round trip");
        Ok(())
    }

    #[test]
    fn timecodes_read_like_a_ruler() -> anyhow::Result<()> {
        assert_eq!(timecode(65.0, false), "1:05", "whole seconds");
        assert_eq!(timecode(5.5, true), "0:05.5", "fine");
        Ok(())
    }
}

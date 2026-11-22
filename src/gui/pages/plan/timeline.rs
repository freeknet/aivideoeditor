// ============================================================================
// Module:       gui::pages::plan::timeline
// Description:  The timeline canvas, painted the way Selects paints its own:
//               ruler and marker lane, track headers, V1 shots as filmstrip
//               clips, A1 waveform, playhead, scroll strip with zoom buttons
//               and the floating tool island, and the pointer edits on them.
//
// Dependencies: egui; crate::gui::pages::plan, crate::gui::ui::{icons,
//               theme, widgets, workspace}
// ============================================================================

//! Drawing and pointer handling for the tracks.
//!
//! Selects draws its timeline on one canvas with constants in code
//! (`docs/research/07-selects-visual-viewer-timeline.md` §3); so does this.
//! Every number below is that spec's: a 35 px ruler with ticks in its top
//! 16 px and clock labels at least 80 px apart, a marker lane under the
//! ticks for the drop, the end card and review comments (12 × 16 pins), a
//! 96 px track-header column, tracks 4 px apart, clips with 4 px corners
//! and a 1 px gap at their right end, a 12 × 10 playhead head, the tool
//! island 24 px from the right and 49 px from the bottom.
//!
//! Geometry is one mapping, `x = left + (t - scroll) · pps`, with `t` in
//! seconds from the plan's start bar. The ruler counts clock time; the
//! song's bars sit in the marker lane, so bars and beats stay where the
//! music has them even when the tempo drifts.

use std::collections::BTreeSet;

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2,
    vec2,
};

use super::{Drag, Io, Shown, State, Tool, edit, waveform::Wave};
use crate::gui::ui::{icons, theme, widgets};
use crate::refs::{Modifier, Reference, Target};

/// Width of the track-header column.
pub(super) const HEADER_W: f32 = 96.0;
/// Height of the ruler: ticks on top, the marker lane under them.
pub(super) const RULER_H: f32 = 35.0;
/// The tick zone at the top of the ruler; the marker lane starts here.
const TICKS_H: f32 = 16.0;
/// Height of a minor tick.
const MINOR_TICK: f32 = 8.0;
/// Closest two labelled (major) ticks may be.
const MAJOR_MIN_PX: f32 = 80.0;
/// Closest two minor ticks may be.
const MINOR_MIN_PX: f32 = 12.0;
/// Where a ruler label sits from its tick.
const LABEL_AT: Vec2 = Vec2::new(8.0, 2.0);
/// Ruler labels: white at 30 % over the canvas (`#5d5d5d`).
const LABEL_ALPHA: u8 = 77;
/// Space between tracks.
const TRACK_GAP: f32 = 4.0;
/// The line between the video and audio panes.
const PANE_LINE: f32 = 2.0;
/// Space above the first track when the panel is tall enough.
const TRACKS_TOP: f32 = 24.0;
/// Height of the storyboard lane when shown.
const STORY_H: f32 = 64.0;
/// Clip corners.
const CLIP_RADIUS: u8 = 4;
/// Where a clip's name sits from its top-left corner.
const NAME_AT: Vec2 = Vec2::new(6.0, 4.0);
/// A clip's name: Mona Sans 11 px.
const NAME_SIZE: f32 = 11.0;
/// A clip's name: white at 70 %.
const NAME_ALPHA: u8 = 179;
/// Room a clip's name needs beside its text: 6 px inset each side.
const NAME_ROOM: f32 = 12.0;
/// Below this width a clip is a sliver: no thumbnails, waveform or name.
const MIN_CLIP_W: f32 = 8.0;
/// Clips at least this wide show trim handles under the pointer.
const HANDLE_MIN_W: f32 = 40.0;
/// Height of the striped band on a shot that fails the check.
const BAD_BAND_H: f32 = 6.0;
/// The scroll strip under the tracks: 8 px padding round an 8 px thumb.
const STRIP_H: f32 = 24.0;
/// The scroll thumb's thickness.
const THUMB_H: f32 = 8.0;
/// The zoom buttons beside the scroll thumb.
const ZOOM_BUTTON: f32 = 20.0;
/// How much one zoom button press zooms.
const ZOOM_STEP: f32 = 1.5;
/// The closest zoom.
const MAX_PPS: f32 = 800.0;
/// The tool island's distance from the canvas's right edge.
const ISLAND_RIGHT: f32 = 24.0;
/// The tool island's distance from the canvas's bottom edge.
const ISLAND_BOTTOM: f32 = 49.0;
/// A tool button in the island.
const ISLAND_BUTTON: f32 = 32.0;
/// Room "fit" leaves at the right so the island does not cover the end.
const FIT_ROOM: f32 = ISLAND_RIGHT + ISLAND_BUTTON + 16.0;
/// The playhead's head: a downward triangle this wide…
const HEAD_W: f32 = 12.0;
/// …and this tall.
const HEAD_H: f32 = 10.0;
/// The head's fill: the playhead colour at 35 %.
const HEAD_FILL: f32 = 0.35;
/// Where the playhead's line starts, from the top of the ruler.
const LINE_FROM: f32 = 8.0;
/// A marker pin's width.
const PIN_W: f32 = 12.0;
/// A marker pin's height (a 12 px square tapering to a point).
const PIN_H: f32 = 16.0;
/// Pointer distance that grabs a pin.
const PIN_GRAB: f32 = 6.0;
/// A waveform bar's pitch: one peak bucket every 2 px, as Selects.
const BAR_STEP: f32 = 2.0;
/// A waveform bar's width inside its pitch, leaving a hairline gap.
const BAR_W: f32 = 1.5;
/// A waveform bar's opacity over the clip gradient, so a loud master
/// reads as a shape rather than a white wall.
const BAR_ALPHA: f32 = 0.55;
/// The red mark over a column whose source samples clipped.
const CLIP_MARK_H: f32 = 3.0;
/// Pointer distance that grabs an edit point.
const EDGE_GRAB: f32 = 4.0;

/// A clip's colours: Selects' `clipLabelColors`, a Tailwind 800 fill and
/// 700 border.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Label {
    /// The clip's body.
    pub(super) fill: Color32,
    /// Its 1 px outline.
    pub(super) border: Color32,
}

/// A video clip with no label.
const VIDEO_CLIP: Label = Label {
    fill: Color32::from_rgb(0x0E, 0x4B, 0x6F),
    border: Color32::from_rgb(0x07, 0x59, 0x85),
};
/// An audio clip.
const AUDIO_CLIP: Label = Label {
    fill: Color32::from_rgb(0x07, 0x59, 0x85),
    border: Color32::from_rgb(0x07, 0x59, 0x85),
};
/// A generated graphic (the end card).
pub(super) const GRAPHIC_CLIP: Label = Label {
    fill: Color32::from_rgb(0x4C, 0x1D, 0x95),
    border: Color32::from_rgb(0x7C, 0x3A, 0xED),
};
/// The orange label.
const ORANGE: Label = Label {
    fill: Color32::from_rgb(0x7C, 0x2D, 0x12),
    border: Color32::from_rgb(0x9A, 0x34, 0x12),
};
/// The violet label.
const VIOLET: Label = Label {
    fill: Color32::from_rgb(0x5B, 0x21, 0xB6),
    border: Color32::from_rgb(0x6D, 0x28, 0xD9),
};
/// The stone label.
const STONE: Label = Label {
    fill: Color32::from_rgb(0x44, 0x40, 0x3C),
    border: Color32::from_rgb(0x57, 0x53, 0x4E),
};
/// The green label.
const GREEN: Label = Label {
    fill: Color32::from_rgb(0x16, 0x65, 0x34),
    border: Color32::from_rgb(0x15, 0x80, 0x3D),
};

/// The label a shot's subject (from the shot log) is drawn in: fire
/// orange, LED violet, white stone, silks green; anything else the
/// default video blue.
fn subject_label(subject: &str) -> Label {
    match subject {
        "fire" => ORANGE,
        "led" => VIOLET,
        "white" => STONE,
        "silks" => GREEN,
        _ => VIDEO_CLIP,
    }
}

/// The label of `shot` on the shown plan.
pub(super) fn shot_label(s: &Shown<'_>, shot: &crate::plan::Shot) -> Label {
    subject_label(s.subject(&shot.clip, shot.t_in))
}

/// A marker colour and its brighter "active" form (selected, hovered or at
/// the playhead): Selects' `markerPaint`, dark theme.
pub(super) fn marker(name: &str) -> (Color32, Color32) {
    match name {
        "blue" => (
            Color32::from_rgb(0x60, 0xA5, 0xFA),
            Color32::from_rgb(0x93, 0xC5, 0xFD),
        ),
        "yellow" => (
            Color32::from_rgb(0xFA, 0xCC, 0x15),
            Color32::from_rgb(0xFD, 0xE0, 0x47),
        ),
        "red" => (
            Color32::from_rgb(0xF8, 0x71, 0x71),
            Color32::from_rgb(0xFC, 0xA5, 0xA5),
        ),
        _ => (
            Color32::from_rgb(0x4A, 0xDE, 0x80),
            Color32::from_rgb(0x86, 0xEF, 0xAC),
        ),
    }
}

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
    pub(super) left: f32,
    /// Seconds at the left edge.
    pub(super) scroll: f32,
    /// Pixels per second.
    pub(super) pps: f32,
}

impl Map {
    /// Screen x of `t` seconds.
    pub(super) fn x(self, t: f32) -> f32 {
        self.left + (t - self.scroll) * self.pps
    }
    /// Seconds at screen x.
    pub(super) fn t(self, x: f32) -> f32 {
        self.scroll + (x - self.left) / self.pps
    }
}

/// `m:ss` or `m:ss.s`: a moment as the comments list and tooltips say it.
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

/// A ruler label: clock time, `MM:SS`, or `HH:MM:SS` from an hour on.
pub(super) fn clock(t: f32, hours: bool) -> String {
    let whole = t.max(0.0).round() as u64;
    let (h, m, s) = (whole / 3600, whole / 60 % 60, whole % 60);
    if hours || h > 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// The labelled step: the first of Selects' steps at least
/// [`MAJOR_MIN_PX`] apart at `pps`.
pub(super) fn major_step(pps: f32) -> f32 {
    const STEPS: [f32; 15] = [
        1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 900.0, 1800.0, 3600.0, 7200.0,
        21600.0,
    ];
    STEPS
        .into_iter()
        .find(|step| step * pps >= MAJOR_MIN_PX)
        .unwrap_or(21600.0)
}

/// The minor step under `major`: the finest that divides it at least twice
/// and is at least [`MINOR_MIN_PX`] apart, or none.
pub(super) fn minor_step(major: f32, pps: f32) -> Option<f32> {
    const STEPS: [f32; 16] = [
        0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 2.5, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0,
        1800.0,
    ];
    STEPS.into_iter().find(|&step| {
        let n = major / step;
        step * pps >= MINOR_MIN_PX && n >= 2.0 - 1e-3 && (n - n.round()).abs() < 1e-3
    })
}

/// Paints the tick zone of a ruler over `ruler` (its top [`TICKS_H`]):
/// major ticks with clock labels, minor ticks at half height.
pub(super) fn paint_ticks(
    ui: &Ui,
    painter: &egui::Painter,
    t: &crate::theme::Palette,
    ruler: Rect,
    m: Map,
) {
    let major = major_step(m.pps);
    let minor = minor_step(major, m.pps);
    let line = theme::ink(t, theme::INK_LINE);
    let faint = line.gamma_multiply(0.8);
    let label = theme::ink(t, LABEL_ALPHA);
    let font = theme::tabular(ui.ctx(), theme::BODY_SIZE);
    let first = m.t(ruler.left()).max(0.0);
    let last = m.t(ruler.right());
    if let Some(step) = minor {
        let mut n = (first / step).floor() as i64;
        loop {
            let at = n as f32 * step;
            if at > last {
                break;
            }
            let x = m.x(at).round() + 0.5;
            if at >= 0.0 {
                painter.vline(
                    x,
                    ruler.top()..=ruler.top() + MINOR_TICK,
                    Stroke::new(1.0, faint),
                );
            }
            n += 1;
        }
    }
    let hours = last >= 3600.0 || major >= 3600.0;
    let mut n = (first / major).floor() as i64;
    loop {
        let at = n as f32 * major;
        if at > last + major {
            break;
        }
        if at >= 0.0 {
            let x = m.x(at).round() + 0.5;
            painter.vline(
                x,
                ruler.top()..=ruler.top() + TICKS_H,
                Stroke::new(1.0, line),
            );
            painter.text(
                pos2(x, ruler.top()) + LABEL_AT,
                Align2::LEFT_TOP,
                clock(at, hours),
                font.clone(),
                label,
            );
        }
        n += 1;
    }
}

/// A marker pin with its top at `top`, centred on `x`: a 12 px rounded
/// square tapering to a point 16 px down. Hollow pins are outlined.
pub(super) fn paint_pin(painter: &egui::Painter, x: f32, top: f32, color: Color32, hollow: bool) {
    let o = pos2(x.round() - PIN_W / 2.0, top);
    let r = 1.7;
    let points: Vec<Pos2> = [
        (r, 0.0),
        (PIN_W - r, 0.0),
        (PIN_W, r),
        (PIN_W, 11.0),
        (PIN_W / 2.0, PIN_H),
        (0.0, 11.0),
        (0.0, r),
    ]
    .iter()
    .map(|&(dx, dy)| o + vec2(dx, dy))
    .collect();
    if hollow {
        painter.add(Shape::closed_line(points, Stroke::new(1.0, color)));
    } else {
        painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    }
}

/// The playhead at `x`: its head at `top`, its line down to `bottom`.
pub(super) fn paint_playhead(
    painter: &egui::Painter,
    x: f32,
    top: f32,
    bottom: f32,
    color: Color32,
) {
    let x = x.round() + 0.5;
    let head = vec![
        pos2(x - HEAD_W / 2.0, top),
        pos2(x + HEAD_W / 2.0, top),
        pos2(x, top + HEAD_H),
    ];
    painter.add(Shape::convex_polygon(
        head.clone(),
        color.gamma_multiply(HEAD_FILL),
        Stroke::NONE,
    ));
    painter.add(Shape::closed_line(head, Stroke::new(1.0, color)));
    painter.vline(x, top + LINE_FROM..=bottom, Stroke::new(1.0, color));
}

/// A clip block as Selects paints it: the label fill, 4 px corners, and
/// `block` itself already carrying the 1 px gap at its right end.
pub(super) fn paint_clip(painter: &egui::Painter, block: Rect, label: Label) {
    painter.rect_filled(block, CornerRadius::same(CLIP_RADIUS), label.fill);
}

/// The 1 px outline over a clip, drawn after its thumbnails.
pub(super) fn paint_clip_border(painter: &egui::Painter, block: Rect, label: Label) {
    painter.rect_stroke(
        block,
        CornerRadius::same(CLIP_RADIUS),
        Stroke::new(1.0, label.border),
        egui::StrokeKind::Inside,
    );
}

/// A clip's name at its top-left, 11 px white at 70 % over a soft shadow,
/// clipped to the clip; `left` is where the visible part starts.
///
/// The name is drawn only when it fits: the visible part of the clip must
/// be at least the text's width plus [`NAME_ROOM`]. Failing that `short`
/// (the shot number) is drawn if it fits, else nothing, so short clips
/// never show a cut-off stub of a name.
pub(super) fn paint_name(
    painter: &egui::Painter,
    block: Rect,
    left: f32,
    text: &str,
    short: Option<&str>,
) {
    if block.width() < MIN_CLIP_W * 2.0 || block.height() < NAME_SIZE + 8.0 {
        return;
    }
    let area = Rect::from_min_max(pos2(block.left().max(left), block.top()), block.max);
    let font = theme::font(NAME_SIZE);
    let fits = |t: &str| {
        let galley = painter.layout_no_wrap(t.to_owned(), font.clone(), Color32::WHITE);
        name_fits(area.width(), galley.size().x)
    };
    let Some(shown) = [Some(text), short]
        .into_iter()
        .flatten()
        .find(|t| !t.is_empty() && fits(t))
    else {
        return;
    };
    let clip = painter.with_clip_rect(area.intersect(painter.clip_rect()));
    let at = pos2(area.left() + NAME_AT.x, block.top() + NAME_AT.y);
    clip.text(
        at + vec2(0.0, 1.0),
        Align2::LEFT_TOP,
        shown,
        font.clone(),
        Color32::from_black_alpha(217),
    );
    clip.text(
        at,
        Align2::LEFT_TOP,
        shown,
        font,
        Color32::from_white_alpha(NAME_ALPHA),
    );
}

/// Whether a name `text_w` wide fits a clip whose visible part is
/// `visible_w` wide: the text plus [`NAME_ROOM`] of inset.
fn name_fits(visible_w: f32, text_w: f32) -> bool {
    visible_w >= text_w + NAME_ROOM
}

/// The selection outline: 2 px, radius 3, 1 px inside the clip; the key
/// colour while the timeline has focus, white otherwise.
pub(super) fn paint_selected(painter: &egui::Painter, block: Rect, color: Color32) {
    painter.rect_stroke(
        block.shrink(1.0),
        CornerRadius::same(3),
        Stroke::new(2.0, color),
        egui::StrokeKind::Inside,
    );
}

/// The colour a selection is outlined in.
pub(super) fn selection_color(ui: &Ui, t: &crate::theme::Palette) -> Color32 {
    if crate::gui::ui::workspace::panel_focused(ui.ctx(), "timeline") {
        theme::rgb(t.accent)
    } else {
        theme::rgb(t.text)
    }
}

/// Thumbnails along a clip as a filmstrip: frames of `path` from `t_in`,
/// each as wide as the frame shape at the strip's height, anchored to the
/// source time so they do not swim while scrolling.
#[expect(
    clippy::too_many_arguments,
    reason = "a filmstrip is a clip, its source, its framing and the visible area"
)]
pub(super) fn filmstrip(
    ui: &mut Ui,
    io: &mut Io<'_>,
    strip: Rect,
    visible: Rect,
    path: &std::path::Path,
    t_in: f32,
    cx: f32,
    ratio: f32,
    pps: f32,
) {
    let area = strip.intersect(visible);
    if area.width() < 4.0 || area.height() < 8.0 {
        return;
    }
    let tile = (strip.height() * ratio).max(8.0);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
    child.set_clip_rect(area.intersect(ui.clip_rect()));
    let first = ((area.left() - strip.left()) / tile).floor().max(0.0) as usize;
    let last = ((area.right() - strip.left()) / tile).ceil() as usize;
    for i in first..last {
        let x0 = strip.left() + i as f32 * tile;
        let rect = Rect::from_min_size(pos2(x0, strip.top()), vec2(tile, strip.height()));
        // A quarter second is close enough, and lets neighbours share decodes.
        let t = t_in + ((x0 - strip.left()) / pps / 0.25).round() * 0.25;
        io.thumbs
            .paint(&child, rect, path, f64::from(t), cx, Color32::TRANSPARENT);
    }
}

/// Where everything on the canvas sits.
struct Layout {
    /// The ruler, right of the header column.
    ruler: Rect,
    /// The storyboard lane, when shown.
    story: Option<Rect>,
    /// The video track.
    v1: Rect,
    /// The audio track.
    a1: Rect,
    /// Everything right of the header column, above the scroll strip.
    lanes: Rect,
    /// The track-header column.
    heads: Rect,
    /// The scroll strip along the bottom.
    strip: Rect,
}

impl Layout {
    /// Lays the canvas out in `rect`.
    fn new(rect: Rect, storyboard: bool) -> Self {
        let strip = Rect::from_min_max(pos2(rect.left(), rect.bottom() - STRIP_H), rect.max);
        let left = rect.left() + HEADER_W;
        let ruler = Rect::from_min_max(
            pos2(left, rect.top()),
            pos2(rect.right(), rect.top() + RULER_H),
        );
        let avail = strip.top() - ruler.bottom();
        let pad = if avail > 220.0 {
            TRACKS_TOP
        } else {
            theme::SPACE_SM
        };
        let room = (avail - pad - theme::SPACE_SM).max(48.0);
        let story_h = if storyboard {
            let h = STORY_H.min(room * 0.3);
            if h >= 36.0 { h } else { 0.0 }
        } else {
            0.0
        };
        let tracks = room - story_h - if story_h > 0.0 { TRACK_GAP } else { 0.0 } - TRACK_GAP;
        let v1_h = (tracks * 0.56).clamp(28.0, 96.0);
        let a1_h = (tracks - v1_h).clamp(20.0, 72.0);
        let mut y = ruler.bottom() + pad;
        let story = (story_h > 0.0).then(|| {
            let r = Rect::from_min_size(pos2(left, y), vec2(ruler.width(), story_h));
            y += story_h + TRACK_GAP;
            r
        });
        let v1 = Rect::from_min_size(pos2(left, y), vec2(ruler.width(), v1_h));
        let a1 = Rect::from_min_size(
            pos2(left, v1.bottom() + TRACK_GAP),
            vec2(ruler.width(), a1_h),
        );
        Self {
            ruler,
            story,
            v1,
            a1,
            lanes: Rect::from_min_max(pos2(left, rect.top()), pos2(rect.right(), strip.top())),
            heads: Rect::from_min_max(rect.min, pos2(left, strip.top())),
            strip,
        }
    }
}

/// Draws the tracks into `rect` and handles the pointer on them.
pub(super) fn tracks(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, ui: &mut Ui, rect: Rect) {
    let t = s.theme;
    let id = egui::Id::unique("plan_tracks");
    let lay = Layout::new(rect, state.storyboard);
    let lanes = lay.lanes;
    let total = s.total.max(0.1);

    // Zoom and scroll.
    let view_w = (lanes.width() - FIT_ROOM).max(40.0);
    let min_pps = view_w / (total * 1.5);
    if state.view.fit {
        state.view.pps = view_w / total;
        state.view.scroll = 0.0;
        state.view.fit = false;
    }
    if let Some(p) = ui.input(|i| i.pointer.hover_pos())
        && lanes.contains(p)
    {
        let (zoom, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
        if (zoom - 1.0).abs() > 1e-4 {
            zoom_at(state, lanes.left(), p.x, zoom, min_pps);
        } else if scroll.x != 0.0 || scroll.y != 0.0 {
            state.view.scroll -= (scroll.x + scroll.y) / state.view.pps;
            state.view.follow = false;
        }
    }
    state.view.pps = state.view.pps.clamp(min_pps, MAX_PPS);
    let visible = lanes.width() / state.view.pps;
    if let Some((a, b)) = state.view.reveal.take()
        && (a < state.view.scroll || b > state.view.scroll + visible)
    {
        state.view.scroll = a - visible * 0.1;
    }
    // The playhead: this plan's position when the viewer plays it, else
    // where it was last (a quiet grey mark).
    if s.ours {
        state.view.parked = io.player.position() as f32;
    }
    let playhead = state.view.parked;
    if io.player.playing() && s.ours {
        if state.view.follow
            && (playhead < state.view.scroll || playhead > state.view.scroll + visible * 0.92)
        {
            state.view.scroll = playhead - visible * 0.1;
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

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, theme::rgb(t.panel));
    let lane_paint = painter.with_clip_rect(lanes);

    // The song's bars, faint behind the clips.
    for bar in (0..s.grid.len()).step_by(4) {
        let x = m.x(s.grid[bar]).round() + 0.5;
        if x >= lanes.left() && x <= lanes.right() {
            lane_paint.vline(
                x,
                lay.ruler.bottom()..=lanes.bottom(),
                Stroke::new(1.0, theme::ink(t, theme::INK_03)),
            );
        }
    }
    // In/out: the selected shots' span.
    let range = s
        .sel
        .first()
        .zip(s.sel.last())
        .and_then(|(&a, &b)| Some((s.timing.get(a)?.0, s.timing.get(b).map(|&(x, d)| x + d)?)));
    if let Some((a, b)) = range {
        let (x0, x1) = (m.x(a), m.x(b));
        lane_paint.rect_filled(
            Rect::from_min_max(pos2(x0, lay.ruler.top()), pos2(x1, lay.ruler.bottom())),
            CornerRadius::ZERO,
            theme::ink(t, 46),
        );
        lane_paint.rect_filled(
            Rect::from_min_max(pos2(x0, lay.ruler.bottom()), pos2(x1, lanes.bottom())),
            CornerRadius::ZERO,
            theme::ink(t, 10),
        );
        for x in [x0.round() + 0.5, x1.round() - 0.5] {
            lane_paint.vline(x, lay.ruler.y_range(), Stroke::new(1.0, theme::ink(t, 179)));
        }
    }

    if let Some(story) = lay.story {
        super::storyboard::lane(state, s, io, ui, story, m);
    }

    // The pane line between video and audio.
    painter.hline(
        rect.left()..=rect.right(),
        lay.v1.bottom() + TRACK_GAP / 2.0,
        Stroke::new(PANE_LINE, theme::ink(t, theme::INK_LINE)),
    );

    let body = Rect::from_min_max(pos2(lanes.left(), lay.ruler.bottom()), lanes.max);
    pointer_tools(state, ui, id, body, m, min_pps);
    video(state, s, io, ui, &lay, m);
    audio(state, s, io, ui, &lay, m);
    ruler(state, s, io, ui, &lay, m);

    // Playhead, over everything on the canvas.
    let x = m.x(playhead);
    if x >= lanes.left() && x <= lanes.right() {
        let color = if s.ours {
            theme::rgb(t.accent)
        } else {
            theme::rgb(t.text_muted)
        };
        paint_playhead(&lane_paint, x, lay.ruler.top(), lanes.bottom(), color);
    }

    headers(state, s, ui, &lay, id);
    scroll_strip(state, s, ui, &lay, id, total, min_pps);
    island(state, s, ui, rect, &lay, id);
}

/// Zooms by `factor` keeping the time under screen x `at` in place.
fn zoom_at(state: &mut State, left: f32, at: f32, factor: f32, min_pps: f32) {
    let m = Map {
        left,
        scroll: state.view.scroll,
        pps: state.view.pps,
    };
    let held = m.t(at);
    state.view.pps = (state.view.pps * factor).clamp(min_pps, MAX_PPS);
    state.view.scroll = held - (at - left) / state.view.pps;
    state.view.follow = false;
}

/// The zoom and hand tools: they act anywhere on the tracks.
fn pointer_tools(state: &mut State, ui: &mut Ui, id: egui::Id, body: Rect, m: Map, min_pps: f32) {
    let sense = match state.tool {
        Tool::Zoom | Tool::Hand => Sense::click_and_drag(),
        Tool::Select | Tool::Slip => Sense::hover(),
    };
    let resp = ui.interact(body, id.with("body"), sense);
    match state.tool {
        Tool::Hand => {
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(if resp.dragged() {
                    CursorIcon::Grabbing
                } else {
                    CursorIcon::Grab
                });
            }
            if resp.dragged() {
                state.view.scroll -= resp.drag_delta().x / m.pps;
                state.view.follow = false;
            }
        }
        Tool::Zoom => {
            let out = ui.input(|i| i.modifiers.alt);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(if out {
                    CursorIcon::ZoomOut
                } else {
                    CursorIcon::ZoomIn
                });
            }
            if resp.clicked()
                && let Some(p) = resp.interact_pointer_pos()
            {
                let factor = if out { 1.0 / ZOOM_STEP } else { ZOOM_STEP };
                zoom_at(state, m.left, p.x, factor, min_pps);
            }
        }
        Tool::Select | Tool::Slip => {}
    }
}

/// V1: the shots and the end card, and the pointer edits on them.
fn video(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, ui: &mut Ui, lay: &Layout, m: Map) {
    let t = s.theme;
    let v1 = lay.v1;
    let lanes = lay.lanes;
    let id = egui::Id::unique("plan_tracks");
    let editing_tool = matches!(state.tool, Tool::Select | Tool::Slip);
    let v1_resp = ui.interact(
        v1,
        id.with("v1"),
        if editing_tool {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
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
    let hover_hit = if editing_tool {
        v1_resp.hover_pos().map_or(Hit::None, hit_at)
    } else {
        Hit::None
    };
    let alt = ui.input(|i| i.modifiers.alt);
    let slipping = state.tool == Tool::Slip || alt;
    match (&state.drag, hover_hit) {
        (Some(Drag::Roll { .. }), _) | (None, Hit::Edge(_)) => {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }
        (Some(Drag::Slip { .. }), _) => ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn),
        (Some(Drag::Move { .. }), _) => ui.ctx().set_cursor_icon(CursorIcon::Grabbing),
        (None, Hit::Shot(_)) if s.editable && slipping => {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn);
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
                if state
                    .plan
                    .as_mut()
                    .is_some_and(|plan| edit::roll(plan, e, left))
                {
                    state.rev += 1;
                }
            }
            Some(Drag::Slip { shot, t0, .. }) => {
                let (k, t0) = (*shot, *t0);
                let dur = s.timing.get(k).map_or(0.0, |x| x.1);
                let len = s.plan.shots.get(k).and_then(|sh| s.clip_len(&sh.clip));
                let t_in = t0 - (p.x - origin.x) / m.pps;
                if state
                    .plan
                    .as_mut()
                    .is_some_and(|plan| edit::slip(plan, k, t_in, dur, len))
                {
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

    // The shots.
    let v1_paint = ui.painter().with_clip_rect(v1.intersect(lanes));
    let ratio = super::aspect_ratio(s.plan.aspect);
    let dim = state.solo_music;
    let outline = selection_color(ui, t);
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
            pos2(x0.floor(), v1.top()),
            pos2((x1.floor() - 1.0).max(x0.floor() + 1.0), v1.bottom()),
        );
        let label = shot_label(s, shot);
        paint_clip(&v1_paint, block, label);
        if !state.hide_thumbs
            && block.width() >= MIN_CLIP_W
            && let Some(path) = s.clip_path(&shot.clip)
        {
            filmstrip(
                ui,
                io,
                block.shrink(1.0),
                v1.intersect(lanes),
                &path,
                shot.t_in,
                shot.cx,
                ratio,
                m.pps,
            );
        }
        paint_clip_border(&v1_paint, block, label);
        if s.issues.contains_key(&k) {
            bad_band(&v1_paint, block, t);
        }
        if dim || dragging_from == Some(k) {
            v1_paint.rect_filled(
                block,
                CornerRadius::same(CLIP_RADIUS),
                theme::translucent(t.panel, 153),
            );
        }
        paint_name(
            &v1_paint,
            block,
            lanes.left(),
            &format!("{}  {}", k + 1, shot.clip),
            Some(&(k + 1).to_string()),
        );
        if s.sel.contains(&k) {
            paint_selected(&v1_paint, block, outline);
        } else if hover_hit == Hit::Shot(k) && state.drag.is_none() {
            handles(&v1_paint, block, s.editable && state.tool == Tool::Select);
        }
    }
    if let Hit::Edge(e) = hover_hit
        && state.drag.is_none()
    {
        let x = m.x(s.timing[e].0).round();
        v1_paint.vline(x, v1.y_range(), Stroke::new(2.0, theme::rgb(t.text)));
    }
    if let Some(Drag::Roll { edit: e, .. }) = &state.drag
        && let Some(&(a, _)) = s.timing.get(*e)
    {
        let x = m.x(a).round();
        v1_paint.vline(x, v1.y_range(), Stroke::new(2.0, theme::rgb(t.text)));
        let left = s.plan.shots.get(e - 1).map_or(0, |x| x.beats);
        let right = s.plan.shots.get(*e).map_or(0, |x| x.beats);
        chip(
            ui,
            t,
            pos2(x, v1.top() - 2.0),
            Align2::CENTER_BOTTOM,
            &format!("{left} | {right} beats"),
        );
    }
    // Move: where it lands, and the clip following the pointer.
    if let Some(Drag::Move { from, to }) = &state.drag
        && from != to
        && let (Some(&(a, d)), Some(p)) = (s.timing.get(*to), v1_resp.interact_pointer_pos())
    {
        let slot = Rect::from_min_max(pos2(m.x(a), v1.top()), pos2(m.x(a + d) - 1.0, v1.bottom()));
        paint_selected(&v1_paint, slot, theme::rgb(t.text));
        if let Some(sh) = s.plan.shots.get(*from) {
            chip(
                ui,
                t,
                pos2(p.x, v1.center().y),
                Align2::CENTER_CENTER,
                &format!("Shot from {}", sh.clip),
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
        chip(
            ui,
            t,
            pos2(m.x(a) + 4.0, v1.bottom() + 2.0),
            Align2::LEFT_TOP,
            &format!("in {:.2}s", sh.t_in),
        );
    }

    // The end card, a generated graphic.
    let (cx0, cx1) = (m.x(s.card), m.x(s.total));
    if cx1 > lanes.left() && cx0 < lanes.right() && cx1 - cx0 >= 2.0 {
        let block = Rect::from_min_max(
            pos2(cx0.floor(), v1.top()),
            pos2(cx1.floor() - 1.0, v1.bottom()),
        );
        paint_clip(&v1_paint, block, GRAPHIC_CLIP);
        let card_path = std::path::Path::new(&s.plan.end_card);
        if !s.plan.end_card.is_empty() && card_path.exists() && !state.hide_thumbs {
            let strip = block.shrink(1.0);
            let th = Rect::from_min_size(
                strip.min,
                vec2((strip.height() * ratio).min(strip.width()), strip.height()),
            );
            let area = th.intersect(v1.intersect(lanes));
            if area.width() > 4.0 {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
                child.set_clip_rect(area);
                io.thumbs
                    .paint(&child, th, card_path, 0.0, 0.5, Color32::TRANSPARENT);
            }
        }
        paint_clip_border(&v1_paint, block, GRAPHIC_CLIP);
        let file = card_path.file_name().map_or_else(
            || "none set".to_owned(),
            |f| f.to_string_lossy().into_owned(),
        );
        paint_name(
            &v1_paint,
            block,
            lanes.left(),
            &format!("End card  {file}"),
            Some("End card"),
        );
        if dim {
            v1_paint.rect_filled(
                block,
                CornerRadius::same(CLIP_RADIUS),
                theme::translucent(t.panel, 153),
            );
        }
    }

    context_menu(state, s, io, &v1_resp);
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
                    .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(t.text)),
            );
            ui.label(
                egui::RichText::new(format!(
                    "{} beats · {dur:.2}s · cx {:.2}{}",
                    sh.beats,
                    sh.cx,
                    if sh.who.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", sh.who)
                    }
                ))
                .color(theme::rgb(t.text_muted)),
            );
            if !sh.note.is_empty() {
                ui.label(egui::RichText::new(&sh.note).color(theme::rgb(t.text)));
            }
            for msg in &issues {
                ui.label(egui::RichText::new(msg).color(theme::rgb(t.danger)));
            }
        });
    }
}

/// The striped band along the bottom of a shot the check fails on: black
/// at 50 % with red diagonal stripes, 1.5 px wide and 8 px apart.
fn bad_band(painter: &egui::Painter, block: Rect, t: &crate::theme::Palette) {
    let band = Rect::from_min_max(
        pos2(block.left() + 1.0, block.bottom() - 1.0 - BAD_BAND_H),
        pos2(block.right() - 1.0, block.bottom() - 1.0),
    );
    if band.width() < 2.0 {
        return;
    }
    let p = painter.with_clip_rect(band.intersect(painter.clip_rect()));
    p.rect_filled(band, CornerRadius::ZERO, Color32::from_black_alpha(128));
    let red = theme::translucent(t.danger, 204);
    let mut x = band.left() - BAD_BAND_H;
    while x < band.right() {
        p.line_segment(
            [pos2(x, band.bottom()), pos2(x + BAD_BAND_H, band.top())],
            Stroke::new(1.5, red),
        );
        x += 8.0;
    }
}

/// Trim handles on a hovered clip: two 3 px pills inset 4 px from its
/// edges, black at 30 % with a white-30 % outline.
fn handles(painter: &egui::Painter, block: Rect, editable: bool) {
    if !editable || block.width() < HANDLE_MIN_W {
        return;
    }
    let h = (block.height() / 2.0).min(20.0);
    for x in [block.left() + 4.0, block.right() - 7.0] {
        let pill = Rect::from_min_size(pos2(x, block.center().y - h / 2.0), vec2(3.0, h));
        painter.rect(
            pill,
            CornerRadius::same(2),
            Color32::from_black_alpha(77),
            Stroke::new(1.0, Color32::from_white_alpha(77)),
            egui::StrokeKind::Inside,
        );
    }
}

/// A small floating readout (a drag's ghost, a roll's beats): popover fill,
/// a hairline, 11 px tabular text.
fn chip(ui: &Ui, t: &crate::theme::Palette, at: Pos2, align: Align2, text: &str) {
    let painter = ui.painter();
    let galley = painter.layout_no_wrap(
        text.to_owned(),
        theme::tabular(ui.ctx(), theme::SMALL_SIZE),
        theme::rgb(t.text),
    );
    let size = galley.size() + vec2(theme::SPACE_SM * 2.0, theme::SPACE_XS * 2.0);
    let rect = align.anchor_size(at, size);
    painter.rect(
        rect,
        CornerRadius::same(theme::RADIUS),
        theme::popover_fill(t),
        theme::ink_line(t),
        egui::StrokeKind::Inside,
    );
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::rgb(t.text),
    );
}

/// The menu on a shot.
fn context_menu(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, v1_resp: &egui::Response) {
    let t = s.theme;
    v1_resp.context_menu(|ui| {
        let Some(k) = state.menu_shot else {
            ui.close();
            return;
        };
        ui.set_min_width(220.0);
        widgets::menu_heading(ui, t, &format!("Shot {}", k + 1));
        if ui
            .add_enabled_ui(s.editable, |ui| {
                widgets::menu_item(ui, t, Some(icons::SPARKLES), "Alternatives", "A", false)
            })
            .inner
            .clicked()
        {
            super::open_alternatives(state, s, k);
            ui.close();
        }
        if widgets::menu_item(ui, t, Some(icons::PLAY), "Play from here", "", false).clicked() {
            super::play_from(s, io, k);
            ui.close();
        }
        if ui
            .add_enabled_ui(s.editable, |ui| {
                widgets::menu_item(
                    ui,
                    t,
                    Some(icons::MESSAGE_SQUARE),
                    "Comment on this shot",
                    "",
                    false,
                )
            })
            .inner
            .clicked()
        {
            let at = s.timing.get(k).map_or(0.0, |&(a, _)| a);
            super::comments::add_at(state, at);
            ui.close();
        }
        if let Some(sh) = s.plan.shots.get(k)
            && widgets::menu_item(
                ui,
                t,
                Some(icons::FILE_VIDEO),
                "Show source clip in viewer",
                "",
                false,
            )
            .clicked()
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
                if widgets::menu_item(ui, t, None, modifier.label(), modifier.prefix(), false)
                    .clicked()
                {
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
        widgets::menu_separator(ui, t);
        if ui
            .add_enabled_ui(s.editable && s.plan.shots.len() > 1, |ui| {
                widgets::menu_item(
                    ui,
                    t,
                    Some(icons::TRASH),
                    "Remove (beats to previous)",
                    "Del",
                    false,
                )
            })
            .inner
            .clicked()
        {
            if state.apply(format!("lift {}", k + 1), |p| edit::lift(p, k)) {
                io.select(&s.name, &BTreeSet::new());
                state.alts = None;
            }
            ui.close();
        }
    });
}

/// A1: the music as one audio clip, its waveform growing from the bottom.
fn audio(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, ui: &mut Ui, lay: &Layout, m: Map) {
    let t = s.theme;
    let a1 = lay.a1;
    let id = egui::Id::unique("plan_tracks");
    let sense = match state.tool {
        Tool::Select | Tool::Slip => Sense::click_and_drag(),
        Tool::Zoom | Tool::Hand => Sense::hover(),
    };
    let a1_resp = ui.interact(a1, id.with("a1"), sense);
    if (a1_resp.clicked() || a1_resp.dragged())
        && let Some(p) = a1_resp.interact_pointer_pos()
    {
        io.seek(s, m.t(p.x).clamp(0.0, s.card));
    }
    let paint = ui.painter().with_clip_rect(a1.intersect(lay.lanes));
    let x0 = m.x(0.0).floor();
    let x1 = m.x(s.total).floor() - 1.0;
    if x1 - x0 < 2.0 {
        return;
    }
    let block = Rect::from_min_max(pos2(x0, a1.top()), pos2(x1, a1.bottom()));
    let media = if state.mute_music { 0.4 } else { 1.0 };
    // The clip colour as a gradient, 40 % at the top to 70 % at the bottom.
    let inner = block.shrink(1.0);
    let base = AUDIO_CLIP.fill;
    let top = base.gamma_multiply(0.4 * media);
    let bottom = base.gamma_multiply(0.7 * media);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(inner.left_top(), top);
    mesh.colored_vertex(inner.right_top(), top);
    mesh.colored_vertex(inner.right_bottom(), bottom);
    mesh.colored_vertex(inner.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    paint.add(Shape::mesh(mesh));
    paint_clip_border(&paint, block, AUDIO_CLIP);

    let music_origin = s.cut.music.as_ref().map_or(0.0, |(_, o)| *o as f32);
    let note = |text: &str, color: Color32| {
        paint.text(
            pos2(
                block.left().max(lay.lanes.left()) + NAME_AT.x,
                a1.center().y,
            ),
            Align2::LEFT_CENTER,
            text,
            theme::font(NAME_SIZE),
            color,
        );
    };
    match s
        .cut
        .music
        .as_ref()
        .map(|(path, _)| state.waves.get(ui.ctx(), path))
    {
        Some(Wave::Ready(peaks)) => {
            let bars = Color32::from_white_alpha((255.0 * BAR_ALPHA * media) as u8);
            let red = theme::rgb(t.danger);
            // Full scale reaches the strip the clip's name sits in, not
            // over it, so the name stays legible over a loud master.
            let lane = (inner.height() - NAME_AT.y - NAME_SIZE - 4.0).max(inner.height() * 0.5);
            let from = inner.left().max(lay.lanes.left()).floor();
            let to = inner.right().min(lay.lanes.right());
            let mut x = from;
            while x < to {
                let ta = music_origin + m.t(x);
                let tb = music_origin + m.t(x + BAR_STEP);
                if let Some(level) = peaks.level(ta, tb)
                    && level > 1e-3
                {
                    let h = (level * lane).max(2.0);
                    paint.rect_filled(
                        Rect::from_min_max(
                            pos2(x, inner.bottom() - h),
                            pos2(x + BAR_W, inner.bottom()),
                        ),
                        CornerRadius::ZERO,
                        bars,
                    );
                }
                // Real clipping in the source samples, not loudness.
                if peaks.clipped(ta, tb) {
                    paint.rect_filled(
                        Rect::from_min_max(
                            pos2(x, inner.top()),
                            pos2(x + BAR_STEP, inner.top() + CLIP_MARK_H),
                        ),
                        CornerRadius::ZERO,
                        red,
                    );
                }
                x += BAR_STEP;
            }
        }
        Some(Wave::Loading) => {
            note("Reading the music…", theme::rgb(t.text_muted));
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        Some(Wave::Failed(e)) => note(&format!("No waveform: {e}"), theme::rgb(t.danger)),
        None => note("The plan names no music file", theme::rgb(t.text_muted)),
    }
    let file = std::path::Path::new(&s.plan.music)
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    paint_name(&paint, block, lay.lanes.left(), &file, None);
}

/// The ruler: clock ticks, then the marker lane with the song's bars, the
/// drop and the end card, and the marker pins; seeks on click and drag, a
/// click on a pin selects that marker and a double-click edits it.
fn ruler(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, ui: &mut Ui, lay: &Layout, m: Map) {
    let t = s.theme;
    let ruler = lay.ruler;
    let id = egui::Id::unique("plan_tracks");
    let resp = ui.interact(ruler, id.with("ruler"), Sense::click_and_drag());
    let marker_y = ruler.top() + TICKS_H;
    let show_resolved = state.show_resolved;
    let pin_at = |p: Pos2| -> Option<usize> {
        if p.y < marker_y {
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
    let pin_clicked = resp
        .clicked()
        .then(|| resp.interact_pointer_pos().and_then(pin_at))
        .flatten();
    let pin_double = resp
        .double_clicked()
        .then(|| resp.interact_pointer_pos().and_then(pin_at))
        .flatten();
    if let Some(i) = pin_double {
        super::comments::edit(state, s, io, i);
    } else if let Some(i) = pin_clicked
        && let Some(c) = s.plan.comments.get(i)
    {
        io.seek(s, c.time.clamp(0.0, s.total));
        super::comments::select(state, s, io, i);
        state.show_comments = true;
    } else if (resp.clicked() || resp.dragged())
        && let Some(p) = resp.interact_pointer_pos()
    {
        io.seek(s, m.t(p.x).clamp(0.0, s.card));
    }
    let hover_pin = resp.hover_pos().and_then(pin_at);
    let rp = ui.painter().with_clip_rect(ruler);
    let lane_paint = ui.painter().with_clip_rect(lay.lanes);
    paint_ticks(ui, &rp, t, ruler, m);

    // The song's bars, thinned so their numbers stay apart.
    let beat_px = s.grid.get(1).map_or(10.0, |b| b * m.pps);
    let step = [1_usize, 2, 4, 8, 16, 32]
        .into_iter()
        .find(|n| *n as f32 * beat_px * 4.0 >= 36.0)
        .unwrap_or(64);
    let bar_font = theme::tabular(ui.ctx(), theme::SMALL_SIZE);
    let faint = theme::ink(t, LABEL_ALPHA);
    let bars = s.plan.end_bar.saturating_sub(s.plan.start_bar);
    for b in 0..=bars {
        let Some(&bt) = s.grid.get(b * 4) else { break };
        let bar = s.plan.start_bar + b;
        if !bar.is_multiple_of(step) && b != 0 {
            continue;
        }
        let x = m.x(bt).round() + 0.5;
        rp.vline(
            x,
            marker_y..=marker_y + 4.0,
            Stroke::new(1.0, theme::ink(t, theme::INK_LINE)),
        );
        rp.text(
            pos2(x + 3.0, marker_y + 1.0),
            Align2::LEFT_TOP,
            bar.to_string(),
            bar_font.clone(),
            faint,
        );
    }

    // The drop (the plan's, solid; the track's other candidates, hollow)
    // and the end card, each with a line through the tracks.
    let origin = s.map.beat_time(s.plan.start_bar, 0);
    let flag = |x: f32, color: Color32, hollow: bool, label: &str| {
        if x < ruler.left() - PIN_W || x > ruler.right() + PIN_W {
            return;
        }
        paint_pin(&rp, x, marker_y, color, hollow);
        if !label.is_empty() {
            rp.text(
                pos2(x + PIN_W / 2.0 + 3.0, marker_y + 6.0),
                Align2::LEFT_CENTER,
                label,
                theme::medium(ui.ctx(), theme::SMALL_SIZE),
                color,
            );
        }
        lane_paint.vline(
            x.round() + 0.5,
            ruler.bottom()..=lay.lanes.bottom(),
            Stroke::new(1.0, color.gamma_multiply(0.3)),
        );
    };
    let (drop, _) = marker("yellow");
    for d in &s.map.drops {
        if d.bar != s.plan.drop_bar && d.bar >= s.plan.start_bar && d.bar <= s.plan.end_bar {
            flag(
                m.x(s.map.beat_time(d.bar, 0) - origin),
                drop.gamma_multiply(0.6),
                true,
                "",
            );
        }
    }
    flag(
        m.x(s.map.beat_time(s.plan.drop_bar, 0) - origin),
        drop,
        false,
        "Drop",
    );
    flag(m.x(s.card), theme::rgb(t.text_muted), false, "Card");

    // Comment pins.
    for (i, c) in s.plan.comments.iter().enumerate() {
        if c.resolved && !state.show_resolved {
            continue;
        }
        let x = m.x(c.time);
        if x < ruler.left() - PIN_W || x > ruler.right() + PIN_W {
            continue;
        }
        let (fill, active) = marker(&c.color);
        let lit = state.comment_sel == Some(i) || hover_pin == Some(i);
        let ink = match (c.resolved, lit) {
            (true, _) => fill.gamma_multiply(0.35),
            (false, true) => active,
            (false, false) => fill,
        };
        paint_pin(&rp, x, marker_y, ink, false);
        lane_paint.vline(
            x.round() + 0.5,
            ruler.bottom()..=lay.lanes.bottom(),
            Stroke::new(1.0, fill.gamma_multiply(0.25)),
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
        resp.on_hover_ui_at_pointer(|ui| {
            ui.set_max_width(280.0);
            ui.label(
                egui::RichText::new(head)
                    .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(t.text)),
            );
            ui.label(egui::RichText::new(note).color(theme::rgb(t.text_muted)));
        });
    }
}

/// The track-header column: labels and the track toggles.
fn headers(state: &mut State, s: &Shown<'_>, ui: &mut Ui, lay: &Layout, id: egui::Id) {
    let t = s.theme;
    let painter = ui.painter().with_clip_rect(lay.heads);
    let left = lay.heads.left();
    let muted = theme::rgb(t.text_muted);
    let label = |row: Rect, text: &str| {
        painter.text(
            pos2(left + 17.0, row.center().y),
            Align2::CENTER_CENTER,
            text,
            theme::tabular(ui.ctx(), theme::BODY_SIZE),
            muted,
        );
    };
    if let Some(story) = lay.story {
        icons::paint(
            &painter,
            Rect::from_center_size(
                pos2(left + 17.0, story.center().y),
                Vec2::splat(icons::SIZE),
            ),
            icons::LAYOUT_GRID,
            icons::SIZE,
            muted,
        );
    }
    label(lay.v1, "V1");
    label(lay.a1, "A1");
    let thumbs = toggle(
        ui,
        s,
        pos2(left + 48.0, lay.v1.center().y),
        id.with("thumbs"),
        if state.hide_thumbs {
            icons::EYE_OFF
        } else {
            icons::EYE
        },
        if state.hide_thumbs { 0.5 } else { 1.0 },
        if state.hide_thumbs {
            "Show thumbnails on V1"
        } else {
            "Hide thumbnails on V1"
        },
    );
    if thumbs.clicked() {
        state.hide_thumbs = !state.hide_thumbs;
    }
    if state.locked {
        let r = Rect::from_center_size(pos2(left + 70.0, lay.v1.center().y), Vec2::splat(20.0));
        icons::paint(&painter, r, icons::LOCK, icons::SIZE, muted);
        let _ = ui
            .interact(r, id.with("lock"), Sense::hover())
            .on_hover_text("Chat is editing this plan: read-only");
    }
    let mute = toggle(
        ui,
        s,
        pos2(left + 38.0, lay.a1.center().y),
        id.with("mute"),
        if state.mute_music {
            icons::VOLUME_X
        } else {
            icons::VOLUME
        },
        if state.mute_music { 0.5 } else { 1.0 },
        if state.mute_music {
            "Unmute A1"
        } else {
            "Mute A1 (dims the waveform)"
        },
    );
    if mute.clicked() {
        state.mute_music = !state.mute_music;
    }
    let solo = toggle(
        ui,
        s,
        pos2(left + 59.0, lay.a1.center().y),
        id.with("solo"),
        icons::HEADPHONES,
        if state.solo_music { 1.0 } else { 0.3 },
        if state.solo_music {
            "Unsolo A1"
        } else {
            "Solo A1 (dims V1 to read the music)"
        },
    );
    if solo.clicked() {
        state.solo_music = !state.solo_music;
    }
}

/// A track-header toggle: a 16 px icon at `alpha`, an ink-8 % square under
/// the pointer.
fn toggle(
    ui: &mut Ui,
    s: &Shown<'_>,
    center: Pos2,
    id: egui::Id,
    glyph: char,
    alpha: f32,
    tip: &str,
) -> egui::Response {
    let t = s.theme;
    let rect = Rect::from_center_size(center, Vec2::splat(20.0));
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), theme::ink_hover(t));
    }
    let ink = if resp.hovered() {
        theme::rgb(t.text)
    } else {
        theme::rgb(t.text_muted).gamma_multiply(alpha)
    };
    icons::paint(painter, rect, glyph, icons::SIZE, ink);
    widgets::tip(resp, t, tip, "")
}

/// The scroll strip: an 8 px thumb whose length is the zoom (drag its ends
/// to zoom, its body to scroll), then zoom to fit, out and in.
fn scroll_strip(
    state: &mut State,
    s: &Shown<'_>,
    ui: &mut Ui,
    lay: &Layout,
    id: egui::Id,
    total: f32,
    min_pps: f32,
) {
    let t = s.theme;
    let strip = lay.strip;
    let painter = ui.painter();
    painter.hline(strip.x_range(), strip.top() + 0.5, theme::ink_line(t));
    painter.vline(lay.lanes.left() - 0.5, strip.y_range(), theme::ink_line(t));
    let lanes = lay.lanes;
    let buttons_w = ZOOM_BUTTON * 3.0 + theme::SPACE_XS * 2.0;
    let track = Rect::from_min_max(
        pos2(
            lanes.left() + theme::SPACE_SM,
            strip.top() + theme::SPACE_SM,
        ),
        pos2(
            strip.right() - theme::SPACE_SM - buttons_w - theme::SPACE_SM,
            strip.top() + theme::SPACE_SM + THUMB_H,
        ),
    );
    if track.width() > 24.0 {
        let visible = lanes.width() / state.view.pps;
        let span = total.max(visible + state.view.scroll.max(0.0));
        let scale = track.width() / span;
        let x0 = track.left() + state.view.scroll.max(0.0) * scale;
        let w = (visible * scale).clamp(THUMB_H * 3.0, track.width());
        let thumb = Rect::from_min_size(
            pos2(x0.min(track.right() - w), track.top()),
            vec2(w, THUMB_H),
        );
        let body = ui.interact(
            thumb.shrink2(vec2(THUMB_H, 0.0)),
            id.with("thumb"),
            Sense::drag(),
        );
        let lead = ui.interact(
            Rect::from_min_size(thumb.min, vec2(THUMB_H, THUMB_H)).expand(2.0),
            id.with("thumb-lead"),
            Sense::drag(),
        );
        let tail = ui.interact(
            Rect::from_min_size(
                pos2(thumb.right() - THUMB_H, thumb.top()),
                vec2(THUMB_H, THUMB_H),
            )
            .expand(2.0),
            id.with("thumb-tail"),
            Sense::drag(),
        );
        let track_resp = ui.interact(
            track.expand2(vec2(0.0, 4.0)),
            id.with("track"),
            Sense::click(),
        );
        let hot = body.hovered() || body.dragged() || lead.hovered() || tail.hovered();
        painter.rect_filled(
            thumb,
            CornerRadius::same(theme::RADIUS_LG),
            if hot {
                theme::rgb(t.text_muted).gamma_multiply(0.5)
            } else {
                theme::accent_fill(t)
            },
        );
        for (r, x) in [
            (&lead, thumb.left() + THUMB_H / 2.0),
            (&tail, thumb.right() - THUMB_H / 2.0),
        ] {
            if r.hovered() || r.dragged() {
                painter.circle_filled(
                    pos2(x, thumb.center().y),
                    THUMB_H / 2.0,
                    theme::rgb(t.text_muted),
                );
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
        }
        if body.hovered() || body.dragged() {
            ui.ctx().set_cursor_icon(if body.dragged() {
                CursorIcon::Grabbing
            } else {
                CursorIcon::Grab
            });
        }
        if body.dragged() {
            state.view.scroll += body.drag_delta().x / scale;
            state.view.follow = false;
        } else if lead.dragged() || tail.dragged() {
            // Moving an end changes how much is visible: that is the zoom.
            let (a, b) = (state.view.scroll, state.view.scroll + visible);
            let (a, b) = if lead.dragged() {
                ((a + lead.drag_delta().x / scale).min(b - 0.1), b)
            } else {
                (a, (b + tail.drag_delta().x / scale).max(a + 0.1))
            };
            state.view.pps = (lanes.width() / (b - a)).clamp(min_pps, MAX_PPS);
            state.view.scroll = a;
            state.view.follow = false;
        } else if track_resp.clicked()
            && let Some(p) = track_resp.interact_pointer_pos()
        {
            state.view.scroll = (p.x - track.left()) / scale - visible / 2.0;
            state.view.follow = false;
        }
    }
    // Zoom to fit, out, in.
    let mut x = strip.right() - theme::SPACE_SM - buttons_w;
    let cy = strip.top() + STRIP_H / 2.0;
    let centre = Map {
        left: lanes.left(),
        scroll: state.view.scroll,
        pps: state.view.pps,
    }
    .t(lanes.center().x);
    for (glyph, label, keys) in [
        (icons::MAXIMIZE_2, "Zoom to fit", "Shift+Z"),
        (icons::MINUS, "Zoom out", "Ctrl+Wheel"),
        (icons::PLUS, "Zoom in", "Ctrl+Wheel"),
    ] {
        let rect = Rect::from_min_size(pos2(x, cy - ZOOM_BUTTON / 2.0), Vec2::splat(ZOOM_BUTTON));
        x += ZOOM_BUTTON + theme::SPACE_XS;
        let resp = ui.interact(rect, id.with(label), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
        if resp.hovered() {
            painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), theme::ink_hover(t));
        }
        let ink = if resp.hovered() {
            theme::rgb(t.text)
        } else {
            theme::rgb(t.text).gamma_multiply(0.7)
        };
        icons::paint(painter, rect, glyph, icons::SIZE, ink);
        let resp = widgets::tip(resp, t, label, keys);
        if resp.clicked() {
            match glyph {
                icons::MAXIMIZE_2 => state.view.fit = true,
                icons::MINUS => {
                    state.view.pps = (state.view.pps / ZOOM_STEP).max(min_pps);
                    state.view.scroll = centre - lanes.width() / 2.0 / state.view.pps;
                }
                _ => {
                    state.view.pps = (state.view.pps * ZOOM_STEP).min(MAX_PPS);
                    state.view.scroll = centre - lanes.width() / 2.0 / state.view.pps;
                }
            }
        }
    }
}

/// The floating tool island: a glass column of 32 px tool buttons 24 px
/// from the canvas's right edge and 49 px from its bottom; the active tool
/// keeps an ink-10 % fill.
fn island(state: &mut State, s: &Shown<'_>, ui: &mut Ui, rect: Rect, lay: &Layout, id: egui::Id) {
    let t = s.theme;
    let tools = [
        (
            Tool::Select,
            icons::POINTER,
            "Selection: click, drag to reorder, drag an edit to roll",
            "tool-select",
        ),
        (
            Tool::Slip,
            icons::MOVE_HORIZONTAL,
            "Slip: drag a shot to move its in-point (or Alt-drag)",
            "tool-slip",
        ),
        (
            Tool::Zoom,
            icons::ZOOM_IN,
            "Zoom: click to zoom in, Alt-click to zoom out",
            "tool-zoom",
        ),
        (Tool::Hand, icons::HAND, "Hand: drag to scroll", "tool-hand"),
    ];
    let n = tools.len() as f32;
    let pad = theme::SPACE_XS;
    let long = n * ISLAND_BUTTON + (n - 1.0) + pad * 2.0;
    let short = ISLAND_BUTTON + pad * 2.0;
    let bottom = rect.bottom() - ISLAND_BOTTOM;
    let right = rect.right() - ISLAND_RIGHT;
    // A column when the tracks are tall enough, else a row.
    let vertical = bottom - lay.ruler.bottom() >= long;
    let size = if vertical {
        vec2(short, long)
    } else {
        vec2(long, short)
    };
    let frame = Rect::from_min_max(pos2(right - size.x, bottom - size.y), pos2(right, bottom));
    let painter = ui.painter();
    painter.rect(
        frame,
        CornerRadius::same(theme::RADIUS_ISLAND),
        theme::glass_fill(t),
        theme::ink_line(t),
        egui::StrokeKind::Inside,
    );
    let _ = ui.interact(frame, id.with("island"), Sense::hover());
    for (k, (tool, glyph, label, command)) in tools.into_iter().enumerate() {
        // The keys come from the table, so a preset's keys show here too.
        let keys = crate::gui::ui::keymap::chord(command, s.preset)
            .map(crate::gui::ui::keymap::format_chord)
            .unwrap_or_default();
        let offset = k as f32 * (ISLAND_BUTTON + 1.0);
        let min = if vertical {
            frame.min + vec2(pad, pad + offset)
        } else {
            frame.min + vec2(pad + offset, pad)
        };
        let r = Rect::from_min_size(min, Vec2::splat(ISLAND_BUTTON));
        let resp = ui.interact(r, id.with(("tool", k)), Sense::click());
        let on = state.tool == tool;
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
        if on || resp.hovered() {
            painter.rect_filled(
                r,
                CornerRadius::same(theme::RADIUS_MD),
                theme::ink(t, theme::INK_10),
            );
        }
        icons::paint(painter, r, glyph, icons::SIZE, theme::rgb(t.text));
        if widgets::tip(resp, t, label, &keys).clicked() {
            state.tool = tool;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Map, clock, major_step, minor_step, name_fits, timecode};

    #[test]
    fn a_name_needs_its_width_plus_twelve_px() -> anyhow::Result<()> {
        assert!(name_fits(52.0, 40.0), "exactly text + 12 fits");
        assert!(!name_fits(51.5, 40.0), "half a pixel short does not");
        assert!(!name_fits(20.0, 40.0), "a short clip never draws the name");
        Ok(())
    }

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
        assert_eq!(clock(65.0, false), "01:05", "the ruler's MM:SS");
        assert_eq!(clock(3725.0, false), "01:02:05", "hours once there are any");
        Ok(())
    }

    #[test]
    fn ruler_steps_keep_labels_and_ticks_apart() -> anyhow::Result<()> {
        assert!(
            (major_step(40.0) - 2.0).abs() < 1e-6,
            "2 s is the first ≥ 80 px at 40 px/s"
        );
        assert!((major_step(100.0) - 1.0).abs() < 1e-6, "1 s at 100 px/s");
        assert!((major_step(1.0) - 120.0).abs() < 1e-6, "2 min at 1 px/s");
        let minor = minor_step(2.0, 40.0).ok_or_else(|| anyhow::anyhow!("no minor step"))?;
        assert!(
            (minor - 0.5).abs() < 1e-6,
            "0.5 s ticks (20 px) under 2 s labels: {minor}"
        );
        assert!(
            minor_step(1.0, 12.0).is_none(),
            "nothing finer than a second fits at 12 px/s"
        );
        Ok(())
    }
}

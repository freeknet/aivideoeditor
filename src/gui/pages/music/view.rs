// ============================================================================
// Module:       gui::pages::music::view
// Description:  The Music page's two strips: the whole-track overview and the
//               zoomed beat-grid view, with their mouse gestures.
//
// Dependencies: egui; crate::{beats, refs}, crate::gui::{pages, player, ui}
// ============================================================================

//! The overview strip and the zoomed view.
//!
//! **Overview:** the whole track's waveform over coloured section bands,
//! drop flags, song-range brackets, the zoomed view's window and the
//! playhead. Click or drag to move the window.
//!
//! **Zoomed view** (FCP 12's beat grid): a ruler with bar numbers, a band
//! of section names, a band of song-range brackets, then the waveform with
//! the grid behind it: section starts as heavy lines, bars solid, beats
//! dashed, with detail following the zoom (beats appear when there is room
//! for them, bar numbers thin out when there is not).
//!
//! | Gesture | Where | Effect |
//! |---|---|---|
//! | click | lane | select the bar, seek there (Shift-click extends) |
//! | double-click | lane | play from there |
//! | drag | lane | select bars |
//! | drag a bar line | ruler | move the downbeat there (snaps to beats; Shift: free offset) |
//! | click / drag | ruler, off a line | scrub |
//! | wheel / Shift-wheel | anywhere | scroll |
//! | Ctrl-wheel / pinch | anywhere | zoom around the pointer |
//! | right-click | lane | reference in chat, drops, ranges, rename |

use crate::gui::ui::hint::Hint as _;
use egui::{Align2, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2};

use super::peaks::Peaks;
use super::{Drag, Edit, State, section_color};
use crate::beats::MusicMap;
use crate::gui::app::Action;
use crate::gui::pages::PageCtx;
use crate::gui::player::Source;
use crate::gui::ui::{theme, widgets};
use crate::refs::{Modifier, Reference, Target};
use crate::theme::Palette;

/// The deliverables a song range is usually cut for.
pub(super) const DELIVERABLES: [&str; 6] = [
    "reel-15", "reel-30", "reel-60", "feed-30", "wide-60", "recap",
];

/// A time window drawn across a rectangle.
#[derive(Clone, Copy, Debug)]
pub(super) struct Span {
    rect: Rect,
    t0: f32,
    t1: f32,
}

impl Span {
    /// The x of time `t`.
    fn x(&self, t: f32) -> f32 {
        self.rect.left() + (t - self.t0) / (self.t1 - self.t0).max(1e-6) * self.rect.width()
    }

    /// The time at `x`.
    fn t(&self, x: f32) -> f32 {
        self.t0 + (x - self.rect.left()) / self.rect.width().max(1.0) * (self.t1 - self.t0)
    }

    /// Seconds per pixel.
    fn per_px(&self) -> f32 {
        (self.t1 - self.t0) / self.rect.width().max(1.0)
    }
}

/// The playhead's time in the track, when the player holds this track.
pub(super) fn playhead(ctx: &PageCtx<'_>, map: &MusicMap) -> Option<f32> {
    match ctx.player.source() {
        Source::Audio { path, t0, .. } if same_track(path, &map.path) => {
            Some((t0 + ctx.player.position()) as f32)
        }
        Source::Audio { .. } | Source::Empty | Source::Clip { .. } | Source::Sequence { .. } => {
            None
        }
    }
}

/// Whether a player path is the map's track.
fn same_track(path: &std::path::Path, track: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").to_lowercase();
    norm(&path.to_string_lossy()) == norm(track)
}

/// Loads the track into the player (if it is not already) and seeks to `t`.
pub(super) fn seek(ctx: &mut PageCtx<'_>, map: &MusicMap, t: f32) {
    if playhead(ctx, map).is_none() {
        ctx.player.open_audio(
            std::path::Path::new(&map.path),
            0.0,
            f64::from(map.duration),
        );
    }
    ctx.player.seek(f64::from(t.max(0.0)));
}

/// The bar at `t`, clamped to the track's bars.
pub(super) fn bar_near(map: &MusicMap, t: f32) -> Option<usize> {
    let last = map.bars.len().checked_sub(1)?;
    Some(
        map.bar_at(t)
            .unwrap_or(if t < map.bars[0].t { 0 } else { last }),
    )
}

/// Makes bars `a..=b` the page's and the app's selection.
pub(super) fn select(ctx: &mut PageCtx<'_>, state: &mut State, a: usize, b: usize) {
    let (from, to) = (a.min(b), a.max(b));
    state.sel = Some((from, to));
    *ctx.selection = vec![Reference::new(Target::Bars { from, to })];
}

/// Draws the overview strip.
pub(super) fn overview(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    map: &MusicMap,
    peaks: Option<&Peaks>,
) {
    let theme = ctx.theme;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 58.0),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), theme::rgb(theme.raised));
    let span = Span {
        rect,
        t0: 0.0,
        t1: map.duration.max(1.0),
    };
    let wave = Rect::from_min_max(
        rect.min + Vec2::new(0.0, 12.0),
        rect.max - Vec2::new(0.0, 10.0),
    );
    for s in &map.sections {
        let Some(first) = map.bars.get(s.from_bar) else {
            continue;
        };
        let (a, b) = (first.t, map.bar_end(s.to_bar));
        let band = Rect::from_x_y_ranges(span.x(a)..=span.x(b), rect.y_range());
        let colour = section_color(theme, &s.name);
        painter.rect_filled(band, CornerRadius::ZERO, theme::translucent(colour, 34));
        painter.line_segment(
            [band.left_top(), band.left_bottom()],
            Stroke::new(1.0, theme::translucent(colour, 160)),
        );
        let label = painter.layout_no_wrap(s.name.clone(), theme::font(10.0), theme::rgb(colour));
        if label.size().x + 6.0 < band.width() {
            painter.galley(
                band.left_top() + Vec2::new(3.0, 0.0),
                label,
                Color32::PLACEHOLDER,
            );
        }
    }
    if let Some(peaks) = peaks {
        waveform(&painter, span, wave, peaks, theme::rgb(theme.text_muted));
    }
    drops(&painter, span, rect, map, theme, None);
    ranges(
        &painter,
        span,
        Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 9.0), rect.max),
        map,
        theme,
    );
    // The zoomed view's window.
    let win = Rect::from_x_y_ranges(
        span.x(state.view_t0)..=span.x(state.view_t0 + state.view_span),
        rect.y_range(),
    );
    painter.rect_stroke(
        win.intersect(rect),
        CornerRadius::same(2),
        Stroke::new(1.5, theme::rgb(theme.text)),
        StrokeKind::Inside,
    );
    if let Some(t) = playhead(ctx, map) {
        let x = span.x(t);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.5, theme::rgb(theme.accent)),
        );
    }
    if (response.clicked() || response.dragged())
        && let Some(p) = response.interact_pointer_pos()
    {
        state.view_t0 = span.t(p.x) - 0.5 * state.view_span;
        state.clamp_view(map.duration);
    }
    response.hint("Whole track: click or drag to move the zoomed view");
}

/// Draws the zoomed view and handles its gestures.
pub(super) fn zoomed(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    map: &MusicMap,
    peaks: Option<&Peaks>,
    edits: &mut Vec<Edit>,
) {
    let theme = ctx.theme;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), 190.0),
        Sense::click_and_drag(),
    );
    let ruler = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 18.0));
    let names = Rect::from_min_size(ruler.left_bottom(), Vec2::new(rect.width(), 15.0));
    let bands = Rect::from_min_size(names.left_bottom(), Vec2::new(rect.width(), 13.0));
    let lane = Rect::from_min_max(bands.left_bottom() + Vec2::new(0.0, 2.0), rect.max);

    // Wheel: scroll; Ctrl-wheel / pinch: zoom around the pointer.
    if response.hovered() {
        let (scroll, zoom, pointer) =
            ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta(), i.pointer.hover_pos()));
        let per_px = state.view_span / rect.width().max(1.0);
        if (zoom - 1.0).abs() > 1e-4 {
            let at = pointer.map_or(state.view_t0 + 0.5 * state.view_span, |p| {
                state.view_t0 + (p.x - rect.left()) * per_px
            });
            state.zoom(1.0 / zoom, at, map.duration);
        }
        let dx = scroll.x + scroll.y;
        if dx.abs() > 0.0 {
            state.view_t0 -= dx * per_px;
            state.clamp_view(map.duration);
            ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }
    let span = Span {
        rect,
        t0: state.view_t0,
        t1: state.view_t0 + state.view_span,
    };

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::same(4), theme::rgb(theme.raised));
    painter.rect_filled(ruler, CornerRadius::ZERO, theme::rgb(theme.panel));

    // Section band and heavy section lines.
    for s in &map.sections {
        let Some(first) = map.bars.get(s.from_bar) else {
            continue;
        };
        let (a, b) = (span.x(first.t), span.x(map.bar_end(s.to_bar)));
        if b < rect.left() || a > rect.right() {
            continue;
        }
        let colour = section_color(theme, &s.name);
        let band = Rect::from_x_y_ranges(a..=b, names.y_range());
        painter.rect_filled(band, CornerRadius::ZERO, theme::translucent(colour, 60));
        painter.rect_filled(
            Rect::from_x_y_ranges(a..=b, lane.y_range()),
            CornerRadius::ZERO,
            theme::translucent(colour, 14),
        );
        let text = format!("{}  {}–{}", s.name, s.from_bar, s.to_bar);
        painter.text(
            Pos2::new(a.max(rect.left()) + 4.0, names.center().y),
            Align2::LEFT_CENTER,
            text,
            theme::font(11.0),
            theme::rgb(theme.text),
        );
        painter.line_segment(
            [Pos2::new(a, names.top()), Pos2::new(a, rect.bottom())],
            Stroke::new(2.5, theme::rgb(colour)),
        );
    }

    if let Some((from, to)) = state.sel
        && let Some(first) = map.bars.get(from)
    {
        let r = Rect::from_x_y_ranges(span.x(first.t)..=span.x(map.bar_end(to)), lane.y_range());
        painter.rect_filled(
            r,
            CornerRadius::ZERO,
            theme::translucent(theme.selection, 120),
        );
    }

    if let Some(peaks) = peaks {
        waveform(
            &painter,
            span,
            lane.shrink2(Vec2::new(0.0, 4.0)),
            peaks,
            theme::rgb(theme.text_muted),
        );
    }
    if state.show_grid {
        grid(&painter, span, ruler, lane, map, theme);
    } else {
        painter.text(
            ruler.right_center() - Vec2::new(4.0, 0.0),
            Align2::RIGHT_CENTER,
            "grid hidden (0)",
            theme::font(10.0),
            theme::rgb(theme.text_faint),
        );
    }
    ranges(&painter, span, bands, map, theme);
    drops(
        &painter,
        span,
        Rect::from_x_y_ranges(rect.x_range(), ruler.top()..=lane.bottom()),
        map,
        theme,
        Some(lane.top() + 2.0),
    );

    let pointer = response.hover_pos();
    state.hover_bar = pointer
        .filter(|p| lane.contains(*p) || names.contains(*p) || bands.contains(*p))
        .and_then(|p| bar_near(map, span.t(p.x)));
    if let Some(p) = pointer {
        painter.line_segment(
            [
                Pos2::new(p.x, ruler.bottom()),
                Pos2::new(p.x, rect.bottom()),
            ],
            Stroke::new(1.0, theme::translucent(theme.text, 60)),
        );
    }

    gestures(ctx, state, ui, map, span, ruler, &response, edits);

    // The downbeat being dragged.
    if let Drag::Grid { bar } = state.drag
        && let Some(p) = response.interact_pointer_pos()
    {
        let shift = ui.input(|i| i.modifiers.shift);
        let t = if shift {
            span.t(p.x)
        } else {
            nearest_beat(map, span.t(p.x)).1
        };
        let x = span.x(t);
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(2.0, theme::rgb(theme.accent)),
        );
        painter.text(
            Pos2::new(x + 4.0, lane.top() + 2.0),
            Align2::LEFT_TOP,
            format!("bar {bar} here{}", if shift { " (free)" } else { "" }),
            theme::font(11.0),
            theme::rgb(theme.accent),
        );
    }

    if let Some(t) = playhead(ctx, map) {
        let x = span.x(t);
        if (rect.left()..=rect.right()).contains(&x) {
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(1.5, theme::rgb(theme.accent)),
            );
            painter.add(Shape::convex_polygon(
                vec![
                    Pos2::new(x - 5.0, rect.top()),
                    Pos2::new(x + 5.0, rect.top()),
                    Pos2::new(x, rect.top() + 7.0),
                ],
                theme::rgb(theme.accent),
                Stroke::NONE,
            ));
        }
        if ctx.player.playing() && (t > span.t1 - 0.05 * state.view_span || t < span.t0) {
            state.view_t0 = t - 0.1 * state.view_span;
            state.clamp_view(map.duration);
        }
    }
    painter.rect_stroke(
        rect,
        CornerRadius::same(4),
        Stroke::new(1.0, theme::rgb(theme.border)),
        StrokeKind::Inside,
    );
}

/// Clicks, drags and the context menu of the zoomed view.
#[expect(
    clippy::too_many_arguments,
    reason = "one call site; bundling the geometry would only rename the arguments"
)]
fn gestures(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &egui::Ui,
    map: &MusicMap,
    span: Span,
    ruler: Rect,
    response: &egui::Response,
    edits: &mut Vec<Edit>,
) {
    let shift = ui.input(|i| i.modifiers.shift);
    let press = response.interact_pointer_pos();
    if response.drag_started()
        && let Some(p) = press
    {
        state.drag = if ruler.contains(p) {
            match line_near(map, span, p.x) {
                Some(bar) => Drag::Grid { bar },
                None => Drag::Scrub,
            }
        } else {
            bar_near(map, span.t(p.x)).map_or(Drag::None, |from| Drag::Select { from })
        };
    }
    if response.dragged()
        && let Some(p) = press
    {
        match state.drag {
            Drag::Select { from } => {
                if let Some(to) = bar_near(map, span.t(p.x))
                    && state.sel != Some((from.min(to), from.max(to)))
                {
                    select(ctx, state, from, to);
                    state.anchor = Some(from);
                }
            }
            Drag::Scrub => seek(ctx, map, span.t(p.x)),
            Drag::Grid { .. } | Drag::None => {}
        }
    }
    if response.drag_stopped() {
        if let Drag::Grid { bar } = state.drag
            && let Some(p) = press
            && let Some(downbeat) = map.bars.get(bar)
        {
            let t = span.t(p.x);
            if shift {
                edits.push(Edit::Nudge(t - downbeat.t));
            } else {
                let (k, _) = nearest_beat(map, t);
                let delta = k as i64 - (bar * 4) as i64;
                if let Ok(delta) = i32::try_from(delta)
                    && delta != 0
                {
                    edits.push(Edit::ShiftDownbeat(delta));
                }
            }
        }
        state.drag = Drag::None;
    }
    if response.clicked()
        && let Some(p) = press
    {
        let t = span.t(p.x);
        if !ruler.contains(p)
            && let Some(bar) = bar_near(map, t)
        {
            match (shift, state.anchor) {
                (true, Some(anchor)) => select(ctx, state, anchor, bar),
                (true, None) | (false, _) => {
                    select(ctx, state, bar, bar);
                    state.anchor = Some(bar);
                }
            }
        }
        seek(ctx, map, t);
    }
    if response.double_clicked()
        && let Some(p) = press
    {
        seek(ctx, map, span.t(p.x));
        ctx.player.play();
    }
    if response.secondary_clicked()
        && let Some(p) = press
    {
        state.menu_t = span.t(p.x);
        if let Some(bar) = bar_near(map, state.menu_t)
            && !state.sel.is_some_and(|(a, b)| bar >= a && bar <= b)
        {
            select(ctx, state, bar, bar);
            state.anchor = Some(bar);
        }
    }
    widgets::context_menu(response, |ui| menu(ctx, state, ui, map, edits));
}

/// The right-click menu over the zoomed view.
fn menu(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    ui: &mut egui::Ui,
    map: &MusicMap,
    edits: &mut Vec<Edit>,
) {
    let Some((from, to)) = state.sel else {
        ui.label("No bars here");
        return;
    };
    let bar = bar_near(map, state.menu_t).unwrap_or(from);
    let t0 = map.bars.get(from).map_or(0.0, |b| b.t);
    let t1 = map.bar_end(to);
    ui.label(
        egui::RichText::new(if from == to {
            format!("bar {from} · {:.2}s", t0)
        } else {
            format!("bars {from}–{to} · {:.1}s", t1 - t0)
        })
        .strong(),
    );
    ui.separator();
    if ui.button("Play from here").clicked() {
        seek(ctx, map, state.menu_t);
        ctx.player.play();
        ui.close();
    }
    widgets::submenu(ui, "Reference in chat", |ui| {
        for m in Modifier::ALL {
            if ui.button(m.label()).clicked() {
                ctx.actions.push(Action::AddRef(
                    Reference::new(Target::Bars { from, to }).with(m),
                ));
                ui.close();
            }
        }
    });
    ui.separator();
    let is_drop = map.drops.iter().position(|d| d.bar == bar);
    let label = match is_drop {
        Some(_) => format!("Unmark drop at bar {bar}   D"),
        None => format!("Mark drop at bar {bar}   D"),
    };
    if ui.button(label).clicked() {
        edits.push(Edit::ToggleDrop(bar));
        ui.close();
    }
    if is_drop.is_some_and(|i| i > 0)
        && ui.button(format!("Make bar {bar} the main drop")).clicked()
    {
        edits.push(Edit::PrimaryDrop(bar));
        ui.close();
    }
    widgets::submenu(ui, "Use as song range for", |ui| {
        for name in DELIVERABLES {
            if ui.button(name).clicked() {
                edits.push(Edit::SetRange(name.to_owned(), from, to));
                ui.close();
            }
        }
    });
    if let Some(i) = map.section_at(bar)
        && let Some(s) = map.sections.get(i)
        && ui.button(format!("Rename section “{}”…", s.name)).clicked()
    {
        state.rename = Some((i, s.name.clone()));
        state.focus_rename = true;
        ui.close();
    }
    if !map.grid_is_detected() {
        ui.separator();
        if ui.button("Reset grid to detected").clicked() {
            edits.push(Edit::ResetGrid);
            ui.close();
        }
    }
}

/// The beat nearest `t`: its index and time.
pub(super) fn nearest_beat(map: &MusicMap, t: f32) -> (usize, f32) {
    let i = map.beats.partition_point(|b| *b < t);
    let candidates = [
        i.saturating_sub(1),
        i.min(map.beats.len().saturating_sub(1)),
    ];
    candidates
        .into_iter()
        .filter_map(|k| map.beats.get(k).map(|b| (k, *b)))
        .min_by(|a, b| (a.1 - t).abs().total_cmp(&(b.1 - t).abs()))
        .unwrap_or((0, t))
}

/// The bar whose line is within a few pixels of `x`.
fn line_near(map: &MusicMap, span: Span, x: f32) -> Option<usize> {
    let t = span.t(x);
    let i = map.bars.partition_point(|b| b.t < t);
    [i.saturating_sub(1), i]
        .into_iter()
        .filter(|k| *k < map.bars.len())
        .map(|k| (k, (span.x(map.bars[k].t) - x).abs()))
        .filter(|(_, d)| *d <= 7.0)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(k, _)| k)
}

/// The waveform as one min/max line per pixel column.
fn waveform(painter: &egui::Painter, span: Span, rect: Rect, peaks: &Peaks, colour: Color32) {
    let mid = rect.center().y;
    let half = 0.5 * rect.height() / peaks.top.max(1e-6);
    let per_px = span.per_px();
    let left = rect.left().max(span.rect.left());
    let mut x = left;
    while x < rect.right() {
        let t = span.t(x);
        if let Some((lo, hi)) = peaks.range(t, t + per_px) {
            let (a, b) = (mid - hi * half, mid - lo * half);
            painter.line_segment(
                [Pos2::new(x + 0.5, a), Pos2::new(x + 0.5, b.max(a + 1.0))],
                Stroke::new(1.0, colour),
            );
        }
        x += 1.0;
    }
}

/// Bars solid with numbers, beats dashed, detail by zoom.
fn grid(
    painter: &egui::Painter,
    span: Span,
    ruler: Rect,
    lane: Rect,
    map: &MusicMap,
    theme: &Palette,
) {
    let px_beat = map.period() / span.per_px();
    let px_bar = 4.0 * px_beat;
    // Label every 1, 2, 4, 8… bars so numbers sit at least 30 px apart.
    let mut every = 1;
    while (every as f32) * px_bar < 30.0 && every < 1024 {
        every *= 2;
    }
    // Lines: every bar when there is room, else every phrase.
    let mut line_every = 1;
    while (line_every as f32) * px_bar < 5.0 && line_every < 1024 {
        line_every *= 2;
    }
    let bar_stroke = Stroke::new(1.0, theme::translucent(theme.text, 90));
    let phrase_stroke = Stroke::new(1.0, theme::translucent(theme.text, 150));
    let beat_stroke = Stroke::new(1.0, theme::translucent(theme.text, 45));
    let first = map
        .bars
        .partition_point(|b| b.t < span.t0)
        .saturating_sub(1);
    for (k, bar) in map.bars.iter().enumerate().skip(first) {
        let x = span.x(bar.t);
        if x > span.rect.right() {
            break;
        }
        if px_beat >= 7.0 {
            for b in 1..4 {
                let bt = map.beat_time(k, b);
                if bt >= map.bar_end(k) {
                    break;
                }
                let bx = span.x(bt);
                painter.extend(Shape::dashed_line(
                    &[Pos2::new(bx, lane.top()), Pos2::new(bx, lane.bottom())],
                    beat_stroke,
                    3.0,
                    3.0,
                ));
                painter.line_segment(
                    [
                        Pos2::new(bx, ruler.bottom() - 3.0),
                        Pos2::new(bx, ruler.bottom()),
                    ],
                    beat_stroke,
                );
            }
        }
        if k % line_every == 0 {
            let stroke = if k % 8 == 0 {
                phrase_stroke
            } else {
                bar_stroke
            };
            painter.line_segment(
                [Pos2::new(x, lane.top()), Pos2::new(x, lane.bottom())],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(x, ruler.bottom() - 7.0),
                    Pos2::new(x, ruler.bottom()),
                ],
                stroke,
            );
        }
        if k % every == 0 {
            painter.text(
                Pos2::new(x + 3.0, ruler.top() + 1.0),
                Align2::LEFT_TOP,
                k.to_string(),
                theme::font(10.0),
                theme::rgb(if k % 8 == 0 {
                    theme.text
                } else {
                    theme.text_muted
                }),
            );
        }
    }
}

/// Drop flags: filled for the user's, outlined for detected ones; the
/// main drop is labelled DROP. `zoomed` is where the flags sit in the
/// zoomed view (with a line through the whole view); without it (the
/// overview) only the main drop is labelled, below the section names.
fn drops(
    painter: &egui::Painter,
    span: Span,
    rect: Rect,
    map: &MusicMap,
    theme: &Palette,
    zoomed: Option<f32>,
) {
    let accent = theme::rgb(theme.accent);
    for (i, d) in map.drops.iter().enumerate() {
        let x = span.x(d.t);
        if x < span.rect.left() - 40.0 || x > span.rect.right() {
            continue;
        }
        let top = zoomed.unwrap_or(rect.top() + 12.0);
        if zoomed.is_some() {
            painter.line_segment(
                [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                Stroke::new(if i == 0 { 2.0 } else { 1.0 }, accent),
            );
        } else if i > 0 {
            let flag = vec![
                Pos2::new(x, top),
                Pos2::new(x + 6.0, top + 3.0),
                Pos2::new(x, top + 6.0),
            ];
            let fill = if d.user { accent } else { Color32::TRANSPARENT };
            painter.add(Shape::convex_polygon(flag, fill, Stroke::new(1.0, accent)));
            continue;
        }
        let flag = vec![
            Pos2::new(x, top),
            Pos2::new(x + 9.0, top + 4.0),
            Pos2::new(x, top + 8.0),
        ];
        let fill = if d.user { accent } else { Color32::TRANSPARENT };
        painter.add(Shape::convex_polygon(flag, fill, Stroke::new(1.2, accent)));
        let label = if i == 0 {
            "DROP".to_owned()
        } else {
            format!("drop {}", i + 1)
        };
        painter.text(
            Pos2::new(x + 11.0, top),
            Align2::LEFT_TOP,
            label,
            theme::font(if i == 0 { 10.5 } else { 9.5 }),
            accent,
        );
    }
}

/// Song ranges as labelled brackets in `band`.
fn ranges(painter: &egui::Painter, span: Span, band: Rect, map: &MusicMap, theme: &Palette) {
    let n = map.ranges.len();
    for (i, r) in map.ranges.iter().enumerate() {
        let Some(first) = map.bars.get(r.from_bar) else {
            continue;
        };
        let (a, b) = (span.x(first.t), span.x(map.bar_end(r.to_bar)));
        if b < span.rect.left() || a > span.rect.right() {
            continue;
        }
        let colour = theme::rgb(theme.series(i, n.max(4)));
        let rows = 2.0;
        let row_h = band.height() / rows;
        let y = band.top() + (i % 2) as f32 * row_h + row_h * 0.5;
        let stroke = Stroke::new(1.5, colour);
        painter.line_segment([Pos2::new(a, y), Pos2::new(b, y)], stroke);
        painter.line_segment(
            [
                Pos2::new(a, y - row_h * 0.45),
                Pos2::new(a, y + row_h * 0.45),
            ],
            stroke,
        );
        painter.line_segment(
            [
                Pos2::new(b, y - row_h * 0.45),
                Pos2::new(b, y + row_h * 0.45),
            ],
            stroke,
        );
        let text = painter.layout_no_wrap(r.name.clone(), theme::font(9.5), colour);
        let w = text.size().x;
        if w + 8.0 < b - a {
            let at = Pos2::new(a.max(span.rect.left()) + 4.0, y - text.size().y * 0.5);
            let bg =
                Rect::from_min_size(at - Vec2::new(2.0, 0.0), text.size() + Vec2::new(4.0, 0.0));
            painter.rect_filled(bg, CornerRadius::same(2), theme::rgb(theme.raised));
            painter.galley(at, text, Color32::PLACEHOLDER);
        }
    }
}

// ============================================================================
// Module:       gui::pages::plan::compare
// Description:  A/B compare of two plans (or a plan and one of its saved
//               versions): both timelines stacked on one beat grid, the
//               shots that differ highlighted, a per-shot diff list, play A
//               or B in the viewer, and "adopt B" through the version path.
//
// Dependencies: egui; crate::{plan, beats, project}, crate::gui::pages::plan
// ============================================================================

//! The compare view (Frame.io's "Compare Versions", for plans).
//!
//! **A** is the open plan as the timeline shows it; **B** is another plan
//! file or a saved version of this one, picked from the "Compare plans" menu in
//! the tab strip (`Shift+C` takes the previous version, or the next plan
//! when there is no history). The view is read-only.
//!
//! Both plans are laid on **one beat grid**: x is music time, so a shot
//! sits over the same bars in both lanes even when the plans start on
//! different bars, and a moved shot is visibly somewhere else.
//!
//! [`diff`] pairs the shots: the same moment of the same clip (in-points
//! within [`SAME_MOMENT`]) in both plans is one shot. A pair on the same
//! beat is unchanged, or *changed* when its length, in-point or crop moved;
//! a pair on different beats has *moved*. What is left over is *replaced*
//! when A and B each have a leftover shot starting on the same beat, else
//! *removed* (only in A) or *added* (only in B). Each row carries both
//! shots' notes, which say why the shot is there.
//!
//! "Play A / Play B" (`B`) loads that side's cut, end card included, into
//! the viewer at the same point of the music. "Adopt B" replaces the open
//! plan with B as one undoable edit and saves it at once as a new version
//! (`adopt <B>`), the way every timeline edit is saved.

use std::collections::BTreeMap;

use egui::{Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::{Cut, Io, Shown, State, history, sequence};
use crate::gui::ui::theme;
use crate::plan::{Plan, Shot};
use crate::project::Project;
use crate::theme::Palette;

/// Two in-points of one clip closer than this, in seconds, are the same
/// moment: a slip, not a different shot.
const SAME_MOMENT: f32 = 1.0;
/// Widest the diff list gets.
const LIST_W: f32 = 380.0;
/// Width of the lane labels.
const LABEL_W: f32 = 82.0;
/// Height of the bar ruler over the lanes.
const RULER_H: f32 = 18.0;
/// Height of the toolbar row.
const BAR_H: f32 = 26.0;

/// One side of a compare.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Side {
    /// The open plan.
    #[default]
    A,
    /// The plan it is compared with.
    B,
}

impl Side {
    /// The other side.
    fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

/// A compare in progress.
#[derive(Debug)]
pub(super) struct Compare {
    /// What B is, e.g. `reel-15` or `reel-30 v3`.
    pub(super) label: String,
    /// B.
    pub(super) plan: Plan,
    /// Which side the viewer plays.
    pub(super) play: Side,
    /// The diff row last clicked.
    pub(super) row: Option<usize>,
}

/// How a shot differs between A and B.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// Only in B.
    Added,
    /// Only in A.
    Removed,
    /// A different shot in the same slot.
    Replaced,
    /// The same shot on a different beat.
    Moved,
    /// The same shot on the same beat, trimmed, slipped or reframed.
    Changed,
}

impl Kind {
    /// The word the list shows.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Replaced => "replaced",
            Self::Moved => "moved",
            Self::Changed => "changed",
        }
    }

    /// Its highlight colour.
    fn color(self, t: &Palette) -> Color32 {
        match self {
            Self::Added => theme::rgb(t.success),
            Self::Removed => theme::rgb(t.danger),
            Self::Replaced => theme::rgb(t.warning),
            Self::Moved => theme::rgb(t.accent),
            Self::Changed => theme::rgb(t.text_muted),
        }
    }
}

/// One row of the diff.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Change {
    /// How it differs.
    pub(super) kind: Kind,
    /// The shot in A (0-based), when A has it.
    pub(super) a: Option<usize>,
    /// The shot in B (0-based), when B has it.
    pub(super) b: Option<usize>,
    /// What changed, in a few words.
    pub(super) detail: String,
}

/// The beat (counted from the song's first bar) each shot starts on.
fn abs_starts(plan: &Plan) -> Vec<usize> {
    let mut beat = plan.start_bar * 4;
    plan.shots
        .iter()
        .map(|s| {
            let at = beat;
            beat += s.beats;
            at
        })
        .collect()
}

/// Whether two shots are the same moment of the same clip.
fn same_moment(x: &Shot, y: &Shot) -> bool {
    x.clip == y.clip && (x.t_in - y.t_in).abs() < SAME_MOMENT
}

/// A shot as the list names it: `MVI_7925 @ 12.0s`.
fn describe(s: &Shot) -> String {
    format!("{} @ {:.1}s", s.clip, s.t_in)
}

/// What differs within one shot kept in both plans.
fn tweaks(x: &Shot, y: &Shot) -> Vec<String> {
    let mut out = Vec::new();
    if x.beats != y.beats {
        out.push(format!("{} → {} beats", x.beats, y.beats));
    }
    if (x.t_in - y.t_in).abs() > 0.01 {
        out.push(format!("in {:+.2}s", y.t_in - x.t_in));
    }
    if (x.cx - y.cx).abs() > 0.01 {
        out.push(format!("crop {:.2} → {:.2}", x.cx, y.cx));
    }
    out
}

/// The per-shot differences from A to B, in the order they play (by the
/// beat they start on).
pub(super) fn diff(a: &Plan, b: &Plan) -> Vec<Change> {
    let (sa, sb) = (abs_starts(a), abs_starts(b));
    let mut pair: Vec<Option<usize>> = vec![None; a.shots.len()];
    let mut used = vec![false; b.shots.len()];
    // The same shot on the same beat first, then the same shot anywhere
    // (nearest beat), so a repeat of a clip pairs with the right copy.
    for exact in [true, false] {
        for (i, x) in a.shots.iter().enumerate() {
            if pair[i].is_some() {
                continue;
            }
            let best = b
                .shots
                .iter()
                .enumerate()
                .filter(|&(j, y)| !used[j] && same_moment(x, y) && (!exact || sb[j] == sa[i]))
                .min_by_key(|&(j, _)| sb[j].abs_diff(sa[i]))
                .map(|(j, _)| j);
            if let Some(j) = best {
                pair[i] = Some(j);
                used[j] = true;
            }
        }
    }
    let mut out: Vec<(usize, Change)> = Vec::new();
    for (i, x) in a.shots.iter().enumerate() {
        let Some(j) = pair[i] else {
            continue;
        };
        let Some(y) = b.shots.get(j) else {
            continue;
        };
        let mut words = tweaks(x, y);
        if sa[i] != sb[j] {
            words.insert(0, format!("slot {} → {}", i + 1, j + 1));
            out.push((
                sb[j],
                Change {
                    kind: Kind::Moved,
                    a: Some(i),
                    b: Some(j),
                    detail: format!("{} · {}", describe(y), words.join(", ")),
                },
            ));
        } else if !words.is_empty() {
            out.push((
                sb[j],
                Change {
                    kind: Kind::Changed,
                    a: Some(i),
                    b: Some(j),
                    detail: format!("{} · {}", describe(y), words.join(", ")),
                },
            ));
        }
    }
    // Leftovers: a slot both sides refilled is a replacement.
    let mut b_left: Vec<usize> = (0..b.shots.len()).filter(|&j| !used[j]).collect();
    for (i, x) in a.shots.iter().enumerate() {
        if pair[i].is_some() {
            continue;
        }
        let slot = b_left.iter().position(|&j| sb[j] == sa[i]);
        match slot.map(|k| b_left.remove(k)) {
            Some(j) => {
                let y_text = b.shots.get(j).map(describe).unwrap_or_default();
                out.push((
                    sa[i],
                    Change {
                        kind: Kind::Replaced,
                        a: Some(i),
                        b: Some(j),
                        detail: format!("{} → {y_text}", describe(x)),
                    },
                ));
            }
            None => out.push((
                sa[i],
                Change {
                    kind: Kind::Removed,
                    a: Some(i),
                    b: None,
                    detail: describe(x),
                },
            )),
        }
    }
    for j in b_left {
        if let Some(y) = b.shots.get(j) {
            out.push((
                sb[j],
                Change {
                    kind: Kind::Added,
                    a: None,
                    b: Some(j),
                    detail: describe(y),
                },
            ));
        }
    }
    out.sort_by_key(|(beat, c)| (*beat, c.a.unwrap_or(usize::MAX)));
    out.into_iter().map(|(_, c)| c).collect()
}

/// What differs about the plans as a whole (shape, bars, music, card).
pub(super) fn plan_changes(a: &Plan, b: &Plan) -> Vec<String> {
    let mut out = Vec::new();
    if a.aspect != b.aspect {
        out.push(format!(
            "aspect {} → {}",
            super::aspect_key(a.aspect),
            super::aspect_key(b.aspect)
        ));
    }
    let bars = |p: &Plan| format!("{}–{}", p.start_bar, p.end_bar);
    if (a.start_bar, a.end_bar) != (b.start_bar, b.end_bar) {
        out.push(format!("bars {} → {}", bars(a), bars(b)));
    }
    if a.drop_bar != b.drop_bar {
        out.push(format!("drop bar {} → {}", a.drop_bar, b.drop_bar));
    }
    if a.card_bar != b.card_bar || a.end_card != b.end_card {
        out.push("end card differs".to_owned());
    }
    if a.music != b.music {
        out.push("different music".to_owned());
    }
    out
}

/// Each side's shots that differ, with how.
fn marks(changes: &[Change]) -> (BTreeMap<usize, Kind>, BTreeMap<usize, Kind>) {
    let mut a = BTreeMap::new();
    let mut b = BTreeMap::new();
    for c in changes {
        if let Some(i) = c.a {
            a.insert(i, c.kind);
        }
        if let Some(j) = c.b {
            b.insert(j, c.kind);
        }
    }
    (a, b)
}

/// Starts comparing the open plan with `plan`.
pub(super) fn open(state: &mut State, label: String, plan: Plan) {
    state.viewing = None;
    state.alts = None;
    state.drag = None;
    state.compare = Some(Compare {
        label,
        plan,
        play: Side::A,
        row: None,
    });
}

/// The B `Shift+C` takes: the version before the latest, or else the next
/// plan in the tab order.
pub(super) fn default_choice(state: &State, project: &Project) -> Option<(String, Plan)> {
    let name = state.name.as_deref()?;
    if state.versions.len() >= 2
        && let Some(v) = state.versions.get(state.versions.len() - 2)
        && let Ok(plan) = history::load(v)
    {
        return Some((format!("{name} v{}", v.n), plan));
    }
    let stems = super::tabs::stems(project);
    let at = stems.iter().position(|s| s == name).unwrap_or(0);
    stems
        .iter()
        .cycle()
        .skip(at + 1)
        .take(stems.len())
        .filter(|s| s.as_str() != name)
        .find_map(|s| {
            Plan::load(&history::live_path(&project.work, s))
                .ok()
                .map(|p| (s.clone(), p))
        })
}

/// The "Compare" menu in the tab strip.
pub(super) fn menu(state: &mut State, project: &Project, ui: &mut Ui) {
    let Some(name) = state.name.clone() else {
        return;
    };
    let mut pick: Option<(String, anyhow::Result<Plan>)> = None;
    let mut close = false;
    ui.menu_button("Compare plans", |ui| {
        ui.set_min_width(260.0);
        if state.compare.is_some() && ui.button("Close compare").clicked() {
            close = true;
            ui.close();
        }
        let latest = state.versions.last().map(|v| v.n);
        let older: Vec<&history::Version> = state
            .versions
            .iter()
            .rev()
            .filter(|v| Some(v.n) != latest)
            .collect();
        ui.label(
            egui::RichText::new(format!("Versions of {name}"))
                .size(11.0)
                .weak(),
        );
        if older.is_empty() {
            ui.label(egui::RichText::new("no earlier versions").size(11.0).weak());
        }
        for v in older {
            let text = format!(
                "v{} · {} · {}",
                v.n,
                v.saved.trim_end_matches(" UTC"),
                if v.change.is_empty() {
                    "save"
                } else {
                    &v.change
                }
            );
            if ui.button(text).clicked() {
                pick = Some((format!("{name} v{}", v.n), history::load(v)));
                ui.close();
            }
        }
        ui.separator();
        ui.label(egui::RichText::new("Other plans").size(11.0).weak());
        for stem in super::tabs::stems(project) {
            if stem == name {
                continue;
            }
            if ui.button(format!("Plan {stem}")).clicked() {
                let plan = Plan::load(&history::live_path(&project.work, &stem));
                pick = Some((stem, plan));
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Compare this plan with a saved version or another plan (Shift+C)");
    if close {
        state.compare = None;
    }
    match pick {
        Some((label, Ok(plan))) => open(state, label, plan),
        Some((label, Err(e))) => state.status = format!("{label}: {e:#}"),
        None => {}
    }
}

/// One lane's plan as the view needs it.
struct Lane<'a> {
    /// Which side.
    side: Side,
    /// Its name as the label says it.
    name: String,
    /// The plan.
    plan: &'a Plan,
    /// Start and length of each shot, seconds from its start bar.
    timing: Vec<(f32, f32)>,
    /// Its start bar, seconds from the view's left edge.
    offset: f32,
    /// The end card's span, seconds from its start bar.
    card: (f32, f32),
    /// As the viewer plays it.
    cut: Cut,
    /// Its shots that differ.
    marks: BTreeMap<usize, Kind>,
}

impl Lane<'_> {
    /// Where shot `k` starts, seconds from the view's left edge.
    fn shot_at(&self, k: usize) -> Option<f32> {
        self.timing.get(k).map(|&(a, _)| self.offset + a)
    }
}

/// Loads `lane`'s cut and puts the playhead at `t` (view seconds).
fn cue(io: &mut Io<'_>, lane: &Lane<'_>, t: f32) {
    lane.cut.open(io.player);
    io.player.seek(f64::from((t - lane.offset).max(0.0)));
}

/// Switches the viewer to `side` at the same point of the music.
fn play_side(play: &mut Side, io: &mut Io<'_>, lanes: &[Lane<'_>; 2], side: Side) {
    let (from, to) = match side {
        Side::A => (&lanes[1], &lanes[0]),
        Side::B => (&lanes[0], &lanes[1]),
    };
    let here = if from.cut.is_loaded(io.player) {
        from.offset + io.player.position() as f32
    } else if to.cut.is_loaded(io.player) {
        to.offset + io.player.position() as f32
    } else {
        to.offset
    };
    let playing = io.player.playing();
    *play = side;
    cue(io, to, here);
    if playing {
        io.player.play();
    }
}

/// Replaces the open plan with B, saved at once as a new version.
fn adopt(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>) {
    let Some(c) = state.compare.take() else {
        return;
    };
    let mut b = c.plan;
    b.name = s.name.clone();
    if state.apply(format!("adopt {}", c.label), |p| {
        *p = b;
        true
    }) {
        state.save(s.project);
        if let Some(plan) = state.plan.as_ref() {
            sequence(plan, s.project, s.map).open(io.player);
        }
        state.status = format!("adopted {} · {}", c.label, state.status);
    }
}

/// The compare view, in place of the tracks.
pub(super) fn view(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, ui: &mut Ui, rect: Rect) {
    let t = s.theme;
    let Some(c) = state.compare.as_mut() else {
        return;
    };
    let changes = diff(&s.plan, &c.plan);
    let whole_plan = plan_changes(&s.plan, &c.plan);
    let (marks_a, marks_b) = marks(&changes);
    let origin_a = s.map.beat_time(s.plan.start_bar, 0);
    let origin_b = s.map.beat_time(c.plan.start_bar, 0);
    let t0 = origin_a.min(origin_b);
    let a_name = match &state.viewing {
        Some(v) => format!("{} v{}", s.name, v.version.n),
        None => s.name.clone(),
    };
    let lanes = [
        Lane {
            side: Side::A,
            name: a_name,
            plan: &s.plan,
            timing: s.timing.clone(),
            offset: origin_a - t0,
            card: (s.card, s.total),
            cut: s.cut.clone(),
            marks: marks_a,
        },
        Lane {
            side: Side::B,
            name: c.label.clone(),
            plan: &c.plan,
            timing: c.plan.timing(s.map),
            offset: origin_b - t0,
            card: c.plan.card_span(s.map),
            cut: sequence(&c.plan, s.project, s.map),
            marks: marks_b,
        },
    ];
    let span = lanes
        .iter()
        .map(|l| l.offset + l.card.1)
        .fold(0.1_f32, f32::max);

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, theme::rgb(t.app));

    // Toolbar.
    let bar = Rect::from_min_size(rect.min, vec2(rect.width(), BAR_H));
    let mut do_adopt = false;
    let mut do_close = false;
    let mut switch_to = None;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(vec2(4.0, 2.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.label(
                egui::RichText::new("COMPARE")
                    .size(10.5)
                    .strong()
                    .color(theme::rgb(t.text_muted)),
            );
            for (side, label) in [(Side::A, "Play A"), (Side::B, "Play B")] {
                if ui
                    .selectable_label(c.play == side, label)
                    .on_hover_text("Play this side in the viewer, at the same point (B)")
                    .clicked()
                {
                    switch_to = Some(side);
                }
            }
            ui.separator();
            let n = changes.len() + whole_plan.len();
            ui.label(
                egui::RichText::new(if n == 0 {
                    "no differences".to_owned()
                } else {
                    format!(
                        "{} shot difference{}",
                        changes.len(),
                        if changes.len() == 1 { "" } else { "s" }
                    )
                })
                .size(11.5)
                .color(theme::rgb(t.text)),
            );
            if !whole_plan.is_empty() {
                ui.label(
                    egui::RichText::new(whole_plan.join(" · "))
                        .size(11.0)
                        .color(theme::rgb(t.warning)),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("Close")
                    .on_hover_text("Back to the timeline (Esc)")
                    .clicked()
                {
                    do_close = true;
                }
                if ui
                    .add_enabled(s.editable && n > 0, egui::Button::new("Adopt B"))
                    .on_hover_text(format!(
                        "Make {} the open plan {} (saved as a new version; Undo takes it back)",
                        c.label, s.name
                    ))
                    .clicked()
                {
                    do_adopt = true;
                }
            });
        },
    );

    // Geometry: lanes on the left, the list on the right.
    let body = Rect::from_min_max(pos2(rect.left(), bar.bottom() + 2.0), rect.max);
    let list_w = LIST_W.min(body.width() * 0.42);
    let list = Rect::from_min_max(pos2(body.right() - list_w, body.top()), body.max);
    let tracks = Rect::from_min_max(body.min, pos2(list.left() - 4.0, body.bottom()));
    let lanes_left = tracks.left() + LABEL_W;
    let lanes_w = (tracks.right() - lanes_left).max(10.0);
    let pps = lanes_w / (span * 1.02);
    let x = |sec: f32| lanes_left + sec * pps;
    let ruler = Rect::from_min_max(
        pos2(lanes_left, tracks.top()),
        pos2(tracks.right(), tracks.top() + RULER_H),
    );
    let lane_h = ((tracks.height() - RULER_H - 6.0) / 2.0).clamp(24.0, 110.0);
    let lane_rects = [0.0, 1.0].map(|k: f32| {
        Rect::from_min_size(
            pos2(tracks.left(), ruler.bottom() + 2.0 + k * (lane_h + 2.0)),
            vec2(tracks.width(), lane_h),
        )
    });

    // Ruler and grid: the song's bars, shared by both lanes.
    painter.rect_filled(ruler, CornerRadius::ZERO, theme::rgb(t.panel));
    let first_bar = s.plan.start_bar.min(c.plan.start_bar);
    let last_bar = s.plan.end_bar.max(c.plan.end_bar);
    let bar_px = s.map.period() * 4.0 * pps;
    let every = [1usize, 2, 4, 8, 16]
        .into_iter()
        .find(|n| bar_px * *n as f32 >= 28.0)
        .unwrap_or(32);
    let grid_bottom = lane_rects[1].bottom();
    for bar_n in first_bar..=last_bar {
        let bx = x(s.map.beat_time(bar_n, 0) - t0);
        painter.vline(
            bx,
            ruler.bottom()..=grid_bottom,
            Stroke::new(1.0, theme::translucent(t.border_strong, 120)),
        );
        if (bar_n - first_bar) % every == 0 {
            painter.text(
                pos2(bx + 2.0, ruler.center().y),
                Align2::LEFT_CENTER,
                bar_n.to_string(),
                FontId::monospace(9.5),
                theme::rgb(t.text_muted),
            );
        }
    }

    // Lanes.
    let mut clicked_shot: Option<(Side, usize)> = None;
    for (lane, lr) in lanes.iter().zip(lane_rects) {
        let head = Rect::from_min_size(lr.min, vec2(LABEL_W - 4.0, lr.height()));
        painter.rect_filled(head, CornerRadius::same(2), theme::rgb(t.panel));
        let side_text = match lane.side {
            Side::A => "A",
            Side::B => "B",
        };
        painter.text(
            head.left_top() + vec2(6.0, 4.0),
            Align2::LEFT_TOP,
            side_text,
            FontId::proportional(13.0),
            theme::rgb(if c.play == lane.side {
                t.accent
            } else {
                t.text
            }),
        );
        painter.with_clip_rect(head).text(
            head.left_top() + vec2(6.0, 22.0),
            Align2::LEFT_TOP,
            &lane.name,
            FontId::proportional(10.0),
            theme::rgb(t.text_muted),
        );
        let lane_rect = Rect::from_min_max(pos2(lanes_left, lr.top()), lr.max);
        let lp = painter.with_clip_rect(lane_rect);
        let ratio = super::aspect_ratio(lane.plan.aspect);
        for (k, (shot, &(start, dur))) in lane.plan.shots.iter().zip(&lane.timing).enumerate() {
            let x0 = x(lane.offset + start);
            let x1 = x(lane.offset + start + dur);
            let block = Rect::from_min_max(
                pos2(x0 + 0.5, lr.top() + 2.0),
                pos2(x1 - 0.5, lr.bottom() - 2.0),
            );
            let mark = lane.marks.get(&k).copied();
            lp.rect_filled(block, CornerRadius::same(2), theme::rgb(t.raised));
            let th_h = block.height() - 6.0;
            let th = Rect::from_min_size(
                block.min + vec2(2.0, 3.0),
                vec2((th_h * ratio).min(block.width() - 4.0), th_h),
            );
            let mut text_left = block.left() + 4.0;
            if th.width() >= 10.0 && th_h >= 14.0 {
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
                text_left = th.right() + 3.0;
            }
            let text = Rect::from_min_max(pos2(text_left, block.top() + 2.0), block.max);
            if text.width() > 16.0 {
                let tp = lp.with_clip_rect(text.intersect(lane_rect));
                tp.text(
                    text.min,
                    Align2::LEFT_TOP,
                    format!("{} {}", k + 1, shot.clip),
                    FontId::proportional(10.5),
                    theme::rgb(t.text),
                );
                if let Some(kind) = mark {
                    tp.text(
                        text.min + vec2(0.0, 14.0),
                        Align2::LEFT_TOP,
                        kind.label(),
                        FontId::proportional(9.5),
                        kind.color(t),
                    );
                }
            }
            match mark {
                Some(kind) => {
                    lp.rect_filled(
                        block,
                        CornerRadius::same(2),
                        kind.color(t).gamma_multiply(0.18),
                    );
                    lp.rect_stroke(
                        block,
                        CornerRadius::same(2),
                        Stroke::new(2.0, kind.color(t)),
                        egui::StrokeKind::Inside,
                    );
                }
                None => {
                    // Unchanged: pushed back so the differences read.
                    lp.rect_filled(block, CornerRadius::same(2), Color32::from_black_alpha(90));
                }
            }
            let selected = c
                .row
                .and_then(|r| changes.get(r))
                .is_some_and(|ch| match lane.side {
                    Side::A => ch.a == Some(k),
                    Side::B => ch.b == Some(k),
                });
            if selected {
                lp.rect_stroke(
                    block,
                    CornerRadius::same(2),
                    Stroke::new(1.5, theme::rgb(t.text)),
                    egui::StrokeKind::Outside,
                );
            }
            let id = egui::Id::unique("plan_compare_shot").with((side_text, k));
            let resp = ui.interact(block.intersect(lane_rect), id, Sense::click());
            let name = format!(
                "{side_text} shot {} {}{}",
                k + 1,
                shot.clip,
                mark.map_or(String::new(), |m| format!(" ({})", m.label()))
            );
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
            if resp.clicked() {
                clicked_shot = Some((lane.side, k));
            }
            let tip_note = shot.note.clone();
            resp.on_hover_ui(|ui| {
                ui.label(egui::RichText::new(describe(shot)).strong());
                ui.label(format!("{} beats · {dur:.2}s", shot.beats));
                if let Some(kind) = mark {
                    ui.label(egui::RichText::new(kind.label()).color(kind.color(t)));
                }
                if !tip_note.is_empty() {
                    ui.label(egui::RichText::new(tip_note).italics());
                }
            });
        }
        // The end card, as the render holds it.
        let (c0, c1) = lane.card;
        if c1 > c0 {
            let block = Rect::from_min_max(
                pos2(x(lane.offset + c0) + 0.5, lr.top() + 2.0),
                pos2(x(lane.offset + c1) - 0.5, lr.bottom() - 2.0),
            );
            lp.rect_filled(block, CornerRadius::same(2), theme::rgb(t.hover));
            lp.with_clip_rect(block.intersect(lane_rect)).text(
                block.left_top() + vec2(4.0, 3.0),
                Align2::LEFT_TOP,
                "END CARD",
                FontId::proportional(9.5),
                theme::rgb(t.text_muted),
            );
        }
        // The playhead, on the side the viewer plays.
        if lane.cut.is_loaded(io.player) {
            let px = x(lane.offset + io.player.position() as f32);
            painter.vline(
                px,
                ruler.top()..=grid_bottom,
                Stroke::new(1.5, theme::rgb(t.accent)),
            );
            painter.text(
                pos2(px, ruler.top()),
                Align2::CENTER_TOP,
                "▼",
                FontId::proportional(9.0),
                theme::rgb(t.accent),
            );
        }
    }

    // The diff list.
    painter.rect_filled(list, CornerRadius::same(2), theme::rgb(t.panel));
    let mut clicked_row = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(list.shrink(6.0)), |ui| {
        ui.label(
            egui::RichText::new(format!("A {}  →  B {}", s.name, c.label))
                .size(11.0)
                .strong()
                .color(theme::rgb(t.text)),
        );
        egui::ScrollArea::vertical()
            .id_salt("plan_compare_list")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if changes.is_empty() {
                    ui.label(
                        egui::RichText::new("Every shot is the same in both.")
                            .size(11.0)
                            .color(theme::rgb(t.text_muted)),
                    );
                }
                for (r, ch) in changes.iter().enumerate() {
                    let slot = match (ch.a, ch.b) {
                        (_, Some(j)) => format!("B{}", j + 1),
                        (Some(i), None) => format!("A{}", i + 1),
                        (None, None) => String::new(),
                    };
                    let head = egui::RichText::new(format!(
                        "{slot:<4} {:<8} {}",
                        ch.kind.label(),
                        ch.detail
                    ))
                    .size(11.0)
                    .color(ch.kind.color(t));
                    let resp = ui.selectable_label(c.row == Some(r), head);
                    if resp.clicked() {
                        clicked_row = Some(r);
                    }
                    let note = |p: &Plan, k: Option<usize>| {
                        k.and_then(|k| p.shots.get(k))
                            .map(|sh| sh.note.clone())
                            .filter(|n| !n.is_empty())
                    };
                    for (side, n) in [("A", note(&s.plan, ch.a)), ("B", note(&c.plan, ch.b))] {
                        if let Some(n) = n {
                            ui.label(
                                egui::RichText::new(format!("   {side}: {n}"))
                                    .size(10.0)
                                    .italics()
                                    .color(theme::rgb(t.text_faint)),
                            );
                        }
                    }
                }
            });
    });
    ui.allocate_rect(rect, Sense::hover());

    // What the clicks do.
    if let Some(side) = switch_to {
        play_side(&mut c.play, io, &lanes, side);
    }
    if let Some((side, k)) = clicked_shot {
        let lane = match side {
            Side::A => &lanes[0],
            Side::B => &lanes[1],
        };
        c.row = changes.iter().position(|ch| match side {
            Side::A => ch.a == Some(k),
            Side::B => ch.b == Some(k),
        });
        c.play = side;
        if let Some(at) = lane.shot_at(k) {
            cue(io, lane, at);
        }
    }
    if let Some(r) = clicked_row
        && let Some(ch) = changes.get(r)
    {
        c.row = Some(r);
        // Cue the played side at the change; a shot only on the other side
        // is cued at its point of the music.
        let (mine, theirs) = match c.play {
            Side::A => ((&lanes[0], ch.a), (&lanes[1], ch.b)),
            Side::B => ((&lanes[1], ch.b), (&lanes[0], ch.a)),
        };
        let at = mine
            .1
            .and_then(|k| mine.0.shot_at(k))
            .or_else(|| theirs.1.and_then(|k| theirs.0.shot_at(k)));
        if let Some(at) = at {
            cue(io, mine.0, at);
        }
    }
    drop(lanes);
    if do_close {
        state.compare = None;
    } else if do_adopt {
        adopt(state, s, io);
    }
}

/// The compare view's keys: `B` plays the other side, `Shift+C` / `Esc`
/// close it, Space cues the played side so the viewer's Space plays it.
pub(super) fn keys(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, ui: &Ui) {
    use egui::Key;
    let plain = |key: Key| ui.input(|i| i.key_pressed(key) && i.modifiers.is_none());
    let fired = |id: &str| ui.input(|i| crate::gui::ui::keymap::pressed(i, id, s.preset));
    let shift_c = fired("plan-compare");
    if plain(Key::Escape) || shift_c {
        state.compare = None;
        return;
    }
    let Some(c) = state.compare.as_mut() else {
        return;
    };
    let b_cut = sequence(&c.plan, s.project, s.map);
    let (side, cut) = match c.play {
        Side::A => (Side::A, s.cut.clone()),
        Side::B => (Side::B, b_cut.clone()),
    };
    if fired("plan-compare-side") {
        let origin_a = s.map.beat_time(s.plan.start_bar, 0);
        let origin_b = s.map.beat_time(c.plan.start_bar, 0);
        let t0 = origin_a.min(origin_b);
        let lane = |side: Side, cut: Cut, offset: f32| Lane {
            side,
            name: String::new(),
            plan: &s.plan,
            timing: Vec::new(),
            offset,
            card: (0.0, 0.0),
            cut,
            marks: BTreeMap::new(),
        };
        let lanes = [
            lane(Side::A, s.cut.clone(), origin_a - t0),
            lane(Side::B, b_cut, origin_b - t0),
        ];
        play_side(&mut c.play, io, &lanes, side.other());
    } else if plain(Key::Space) && !cut.is_loaded(io.player) {
        cut.open(io.player);
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, diff, plan_changes};
    use crate::plan::{Aspect, Plan, Rules, Shot};

    fn shot(clip: &str, t_in: f32, beats: usize, note: &str) -> Shot {
        Shot {
            clip: clip.into(),
            t_in,
            beats,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: note.into(),
        }
    }

    fn plan(shots: Vec<Shot>) -> Plan {
        Plan {
            name: "reel".into(),
            aspect: Aspect::Vertical,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 1,
            card_bar: 4,
            end_bar: 5,
            end_card: String::new(),
            rules: Rules::default(),
            finish: Default::default(),
            shots,
            comments: Vec::new(),
        }
    }

    #[test]
    fn the_diff_names_each_kind_of_change() -> anyhow::Result<()> {
        let a = plan(vec![
            shot("A", 1.0, 4, "opener"),
            shot("B", 2.0, 4, "build"),
            shot("C", 3.0, 4, "drop"),
            shot("D", 4.0, 4, "outro"),
        ]);
        let b = plan(vec![
            shot("A", 1.3, 4, "opener, slipped"),
            shot("C", 3.0, 4, "drop earlier"),
            shot("E", 9.0, 4, "new build"),
            shot("F", 5.0, 2, "tag"),
            shot("G", 6.0, 2, "tag 2"),
        ]);
        let got: Vec<(Kind, Option<usize>, Option<usize>)> =
            diff(&a, &b).iter().map(|c| (c.kind, c.a, c.b)).collect();
        assert_eq!(
            got,
            vec![
                (Kind::Changed, Some(0), Some(0)),
                (Kind::Removed, Some(1), None),
                (Kind::Moved, Some(2), Some(1)),
                (Kind::Added, None, Some(2)),
                (Kind::Replaced, Some(3), Some(3)),
                (Kind::Added, None, Some(4)),
            ],
            "slip, removal, move, additions, a replacement in a refilled slot"
        );
        let changes = diff(&a, &b);
        assert!(
            changes
                .first()
                .is_some_and(|c| c.detail.contains("in +0.30s")),
            "a slip says by how much: {changes:?}"
        );
        assert!(
            changes
                .iter()
                .any(|c| c.kind == Kind::Moved && c.detail.contains("slot 3 → 2")),
            "a move says from where to where: {changes:?}"
        );
        Ok(())
    }

    #[test]
    fn the_same_plan_has_no_differences() -> anyhow::Result<()> {
        let a = plan(vec![shot("A", 1.0, 8, ""), shot("B", 2.0, 8, "")]);
        assert!(diff(&a, &a.clone()).is_empty(), "nothing to list");
        assert!(plan_changes(&a, &a.clone()).is_empty(), "nor plan-wide");
        let mut wide = a.clone();
        wide.aspect = Aspect::Wide;
        wide.end_bar = 6;
        assert_eq!(
            plan_changes(&a, &wide),
            vec!["aspect 9x16 → 16x9".to_owned(), "bars 0–5 → 0–6".to_owned()],
            "shape and length"
        );
        Ok(())
    }

    #[test]
    fn a_removal_and_an_addition_in_different_slots_stay_apart() -> anyhow::Result<()> {
        let a = plan(vec![shot("A", 1.0, 8, ""), shot("B", 2.0, 8, "")]);
        let b = plan(vec![shot("A", 1.0, 4, ""), shot("C", 2.0, 12, "")]);
        let got: Vec<Kind> = diff(&a, &b).iter().map(|c| c.kind).collect();
        assert_eq!(
            got,
            vec![Kind::Changed, Kind::Added, Kind::Removed],
            "B's C starts on beat 4, A's B on beat 8: not the same slot"
        );
        Ok(())
    }
}

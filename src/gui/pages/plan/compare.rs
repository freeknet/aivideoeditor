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
//! file or a saved version of this one, picked from the "Compare cuts" menu in
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

use egui::{Align2, Color32, CornerRadius, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::{Cut, Io, Shown, State, history, sequence, timeline};
use crate::gui::ui::{icons, theme, widgets};
use crate::plan::{Plan, Shot};
use crate::project::Project;
use crate::theme::Palette;

/// Two in-points of one clip closer than this, in seconds, are the same
/// moment: a slip, not a different shot.
const SAME_MOMENT: f32 = 1.0;
/// Widest the diff list gets.
const LIST_W: f32 = 380.0;
/// Height of the toolbar row.
const BAR_H: f32 = 40.0;
/// Space between the two lanes.
const LANE_GAP: f32 = 4.0;
/// Room for the kind badge in a diff row.
const KIND_W: f32 = 76.0;
/// Room for the slot (`B12`) in a diff row.
const SLOT_W: f32 = 36.0;

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
            Self::Moved => timeline::marker("blue").0,
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

/// What the "Compare cuts" menu offers, read before it is drawn.
#[derive(Clone, Debug)]
pub(super) struct MenuInfo {
    /// The open plan.
    name: String,
    /// Its versions, oldest first.
    versions: Vec<history::Version>,
    /// A compare is on show.
    open: bool,
    /// Every plan of the night.
    stems: Vec<String>,
}

/// What was picked in the menu.
#[derive(Clone, Debug)]
pub(super) enum Pick {
    /// End the compare.
    Close,
    /// Compare with a saved version of the open plan.
    Version(history::Version),
    /// Compare with another plan.
    Plan(String),
}

/// What the menu needs, when a plan is open.
pub(super) fn menu_info(state: &State, project: &Project) -> Option<MenuInfo> {
    Some(MenuInfo {
        name: state.name.clone()?,
        versions: state.versions.clone(),
        open: state.compare.is_some(),
        stems: super::tabs::stems(project),
    })
}

/// The "Compare cuts" button in the timeline's header and its menu.
pub(super) fn menu(ui: &mut Ui, t: &Palette, info: &MenuInfo) -> Option<Pick> {
    let resp = widgets::icon_toggle(ui, t, icons::COMPARE, "Compare cuts", "Shift+C", info.open);
    let mut pick = None;
    let _ = egui::Popup::menu(&resp)
        .width(300.0)
        .gap(theme::SPACE_XS)
        .show(|ui| {
            if info.open {
                if widgets::menu_item(ui, t, Some(icons::X), "Close compare", "Esc", false)
                    .clicked()
                {
                    pick = Some(Pick::Close);
                    ui.close();
                }
                widgets::menu_separator(ui, t);
            }
            let latest = info.versions.last().map(|v| v.n);
            let older: Vec<&history::Version> = info
                .versions
                .iter()
                .rev()
                .filter(|v| Some(v.n) != latest)
                .collect();
            ui.horizontal(|ui| {
                ui.add_space(theme::SPACE_SM);
                widgets::menu_heading(ui, t, &format!("Versions of {}", info.name));
            });
            if older.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(theme::SPACE_SM);
                    ui.label(egui::RichText::new("No earlier versions").color(theme::ink(t, 102)));
                });
            }
            egui::ScrollArea::vertical()
                .id_salt("plan_compare_versions")
                .max_height(240.0)
                .show(ui, |ui| {
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
                        if widgets::menu_item(ui, t, Some(icons::HISTORY), &text, "", false)
                            .clicked()
                        {
                            pick = Some(Pick::Version(v.clone()));
                            ui.close();
                        }
                    }
                });
            widgets::menu_separator(ui, t);
            ui.horizontal(|ui| {
                ui.add_space(theme::SPACE_SM);
                widgets::menu_heading(ui, t, "Other cuts");
            });
            for stem in &info.stems {
                if *stem == info.name {
                    continue;
                }
                if widgets::menu_item(ui, t, Some(icons::FILM), &format!("Plan {stem}"), "", false)
                    .clicked()
                {
                    pick = Some(Pick::Plan(stem.clone()));
                    ui.close();
                }
            }
        });
    pick
}

/// Acts on what was picked in the menu.
pub(super) fn take(state: &mut State, project: &Project, pick: Pick) {
    let loaded = match pick {
        Pick::Close => {
            state.compare = None;
            return;
        }
        Pick::Version(v) => {
            let name = state.name.clone().unwrap_or_default();
            (format!("{name} v{}", v.n), history::load(&v))
        }
        Pick::Plan(stem) => {
            let plan = Plan::load(&history::live_path(&project.work, &stem));
            (stem, plan)
        }
    };
    match loaded {
        (label, Ok(plan)) => open(state, label, plan),
        (label, Err(e)) => state.status = format!("{label}: {e:#}"),
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
        // Markers are the review of this plan: they stay.
        b.comments = std::mem::take(&mut p.comments);
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
    painter.rect_filled(rect, CornerRadius::ZERO, theme::rgb(t.panel));

    // Toolbar.
    let bar = Rect::from_min_size(rect.min, vec2(rect.width(), BAR_H));
    painter.hline(bar.x_range(), bar.bottom() - 0.5, theme::ink_line(t));
    let mut do_adopt = false;
    let mut do_close = false;
    let mut switch_to = None;
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(bar.shrink2(vec2(theme::SPACE_SM, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
            let mut play = c.play;
            if widgets::segmented(
                ui,
                t,
                &mut play,
                &[(Side::A, "Play A"), (Side::B, "Play B")],
            ) {
                switch_to = Some(play);
            }
            let n = changes.len() + whole_plan.len();
            ui.label(
                egui::RichText::new(if n == 0 {
                    "No differences".to_owned()
                } else {
                    format!(
                        "{} shot difference{}",
                        changes.len(),
                        if changes.len() == 1 { "" } else { "s" }
                    )
                })
                .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                .color(theme::rgb(t.text)),
            );
            for w in &whole_plan {
                widgets::pill(ui, t, w, widgets::Tone::Warning);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if widgets::icon_btn(ui, t, icons::X, "Close compare", "Esc").clicked() {
                    do_close = true;
                }
                ui.add_space(theme::SPACE_XS);
                let adopt = ui
                    .add_enabled_ui(s.editable && n > 0, |ui| {
                        widgets::button_with(
                            ui,
                            t,
                            widgets::ButtonKind::Primary,
                            Some(icons::CHECK),
                            "Adopt B",
                        )
                    })
                    .inner;
                let adopt = widgets::tip(
                    adopt,
                    t,
                    &format!(
                        "Make {} the open plan {} (saved as a new version; Undo takes it back)",
                        c.label, s.name
                    ),
                    "",
                );
                if adopt.clicked() {
                    do_adopt = true;
                }
            });
        },
    );

    // Geometry: lanes on the left, the list on the right.
    let body = Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max);
    let list_w = LIST_W.min(body.width() * 0.42);
    let list = Rect::from_min_max(pos2(body.right() - list_w, body.top()), body.max);
    let tracks = Rect::from_min_max(body.min, pos2(list.left(), body.bottom()));
    let lanes_left = tracks.left() + timeline::HEADER_W;
    let lanes_w = (tracks.right() - lanes_left - theme::SPACE_LG).max(10.0);
    let m = timeline::Map {
        left: lanes_left,
        scroll: 0.0,
        pps: lanes_w / span,
    };
    let x = |sec: f32| m.x(sec);
    let ruler = Rect::from_min_max(
        pos2(lanes_left, tracks.top()),
        pos2(tracks.right(), tracks.top() + timeline::RULER_H),
    );
    let lane_h = ((tracks.height() - timeline::RULER_H - theme::SPACE_XL - LANE_GAP) / 2.0)
        .clamp(24.0, 96.0);
    let lane_rects = [0.0, 1.0].map(|k: f32| {
        Rect::from_min_size(
            pos2(
                tracks.left(),
                ruler.bottom() + theme::SPACE_MD + k * (lane_h + LANE_GAP),
            ),
            vec2(tracks.width(), lane_h),
        )
    });
    let grid_bottom = tracks.bottom();
    let canvas = painter.with_clip_rect(Rect::from_min_max(
        pos2(lanes_left, tracks.top()),
        tracks.max,
    ));

    // Ruler: clock time from the earlier start, the song's bars under it.
    timeline::paint_ticks(ui, &canvas, t, ruler, m);
    let first_bar = s.plan.start_bar.min(c.plan.start_bar);
    let last_bar = s.plan.end_bar.max(c.plan.end_bar);
    let bar_px = s.map.period() * 4.0 * m.pps;
    let every = [1usize, 2, 4, 8, 16]
        .into_iter()
        .find(|n| bar_px * *n as f32 >= 36.0)
        .unwrap_or(32);
    let bar_font = theme::tabular(ui.ctx(), theme::SMALL_SIZE);
    for bar_n in first_bar..=last_bar {
        let bx = x(s.map.beat_time(bar_n, 0) - t0).round() + 0.5;
        canvas.vline(
            bx,
            ruler.bottom()..=grid_bottom,
            Stroke::new(1.0, theme::ink(t, theme::INK_03)),
        );
        if (bar_n - first_bar) % every == 0 {
            canvas.text(
                pos2(bx + 3.0, ruler.top() + 17.0),
                Align2::LEFT_TOP,
                bar_n.to_string(),
                bar_font.clone(),
                theme::ink(t, 77),
            );
        }
    }

    // Lanes.
    let outline = timeline::selection_color(ui, t);
    let mut clicked_shot: Option<(Side, usize)> = None;
    for (lane, lr) in lanes.iter().zip(lane_rects) {
        let side_text = match lane.side {
            Side::A => "A",
            Side::B => "B",
        };
        let head = Rect::from_min_size(lr.min, vec2(timeline::HEADER_W, lr.height()));
        let playing = c.play == lane.side;
        painter.text(
            pos2(head.left() + 17.0, head.center().y),
            Align2::CENTER_CENTER,
            side_text,
            theme::tabular(ui.ctx(), theme::BODY_SIZE),
            theme::rgb(if playing { t.text } else { t.text_muted }),
        );
        painter.with_clip_rect(head.shrink2(vec2(2.0, 0.0))).text(
            pos2(head.left() + 30.0, head.center().y),
            Align2::LEFT_CENTER,
            &lane.name,
            theme::font(theme::SMALL_SIZE),
            theme::rgb(t.text_muted),
        );
        let lane_rect = Rect::from_min_max(pos2(lanes_left, lr.top()), lr.max);
        let lp = painter.with_clip_rect(lane_rect);
        let ratio = super::aspect_ratio(lane.plan.aspect);
        for (k, (shot, &(start, dur))) in lane.plan.shots.iter().zip(&lane.timing).enumerate() {
            let x0 = x(lane.offset + start).floor();
            let x1 = x(lane.offset + start + dur).floor() - 1.0;
            let block = Rect::from_min_max(pos2(x0, lr.top()), pos2(x1.max(x0 + 1.0), lr.bottom()));
            let mark = lane.marks.get(&k).copied();
            let label = timeline::shot_label(s, shot);
            timeline::paint_clip(&lp, block, label);
            if let Some(path) = s.clip_path(&shot.clip) {
                timeline::filmstrip(
                    ui,
                    io,
                    block.shrink(1.0),
                    lane_rect,
                    &path,
                    shot.t_in,
                    shot.cx,
                    ratio,
                    m.pps,
                );
            }
            timeline::paint_clip_border(&lp, block, label);
            if mark.is_none() {
                // Unchanged: pushed back so the differences read.
                lp.rect_filled(
                    block,
                    CornerRadius::same(4),
                    theme::translucent(t.panel, 153),
                );
            }
            let name = match mark {
                Some(kind) => format!("{}  {} · {}", k + 1, shot.clip, kind.label()),
                None => format!("{}  {}", k + 1, shot.clip),
            };
            timeline::paint_name(
                &lp,
                block,
                lane_rect.left(),
                &name,
                Some(&(k + 1).to_string()),
            );
            let selected = c
                .row
                .and_then(|r| changes.get(r))
                .is_some_and(|ch| match lane.side {
                    Side::A => ch.a == Some(k),
                    Side::B => ch.b == Some(k),
                });
            if selected {
                timeline::paint_selected(&lp, block, outline);
            } else if let Some(kind) = mark {
                timeline::paint_selected(&lp, block, kind.color(t));
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
                ui.label(
                    egui::RichText::new(describe(shot))
                        .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                        .color(theme::rgb(t.text)),
                );
                ui.label(
                    egui::RichText::new(format!("{} beats · {dur:.2}s", shot.beats))
                        .color(theme::rgb(t.text_muted)),
                );
                if let Some(kind) = mark {
                    ui.label(egui::RichText::new(kind.label()).color(kind.color(t)));
                }
                if !tip_note.is_empty() {
                    ui.label(egui::RichText::new(tip_note).color(theme::rgb(t.text)));
                }
            });
        }
        // The end card, as the render holds it.
        let (c0, c1) = lane.card;
        if c1 > c0 {
            let block = Rect::from_min_max(
                pos2(x(lane.offset + c0).floor(), lr.top()),
                pos2(x(lane.offset + c1).floor() - 1.0, lr.bottom()),
            );
            timeline::paint_clip(&lp, block, timeline::GRAPHIC_CLIP);
            timeline::paint_clip_border(&lp, block, timeline::GRAPHIC_CLIP);
            timeline::paint_name(&lp, block, lane_rect.left(), "End card", None);
        }
    }
    // The playhead, on the side the viewer plays.
    for lane in &lanes {
        if lane.cut.is_loaded(io.player) {
            let px = x(lane.offset + io.player.position() as f32);
            timeline::paint_playhead(&canvas, px, ruler.top(), grid_bottom, theme::rgb(t.accent));
        }
    }

    // The diff list.
    painter.vline(list.left() + 0.5, list.y_range(), theme::ink_line(t));
    let mut clicked_row = None;
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(list.shrink2(vec2(theme::SPACE_SM, theme::SPACE_SM))),
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(
                egui::RichText::new(format!("A {}  →  B {}", s.name, c.label))
                    .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(t.text)),
            );
            ui.add_space(theme::SPACE_XS);
            egui::ScrollArea::vertical()
                .id_salt("plan_compare_list")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if changes.is_empty() {
                        ui.label(
                            egui::RichText::new("Every shot is the same in both.")
                                .color(theme::rgb(t.text_muted)),
                        );
                    }
                    for (r, ch) in changes.iter().enumerate() {
                        let note = |p: &Plan, k: Option<usize>| {
                            k.and_then(|k| p.shots.get(k))
                                .map(|sh| sh.note.clone())
                                .filter(|n| !n.is_empty())
                        };
                        let notes = [("A", note(&s.plan, ch.a)), ("B", note(&c.plan, ch.b))];
                        if diff_row(ui, t, ch, c.row == Some(r), &notes).clicked() {
                            clicked_row = Some(r);
                        }
                    }
                });
        },
    );
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

/// One row of the diff list: the kind as a tinted badge, the slot and what
/// changed, then each side's note; ink 8 % under the pointer, the selected
/// fill when picked.
fn diff_row(
    ui: &mut Ui,
    t: &Palette,
    ch: &Change,
    selected: bool,
    notes: &[(&str, Option<String>); 2],
) -> egui::Response {
    let slot = match (ch.a, ch.b) {
        (_, Some(j)) => format!("B{}", j + 1),
        (Some(i), None) => format!("A{}", i + 1),
        (None, None) => String::new(),
    };
    let width = ui.available_width();
    let text_w = width - KIND_W - SLOT_W - theme::SPACE_SM * 2.0;
    let detail = ui.painter().layout(
        ch.detail.clone(),
        theme::font(theme::BODY_SIZE),
        theme::rgb(t.text),
        text_w.max(40.0),
    );
    let note_galleys: Vec<_> = notes
        .iter()
        .filter_map(|(side, n)| {
            n.as_ref().map(|n| {
                ui.painter().layout(
                    format!("{side}: {n}"),
                    theme::font(theme::SMALL_SIZE),
                    theme::rgb(t.text_muted),
                    text_w.max(40.0),
                )
            })
        })
        .collect();
    let height = theme::SPACE_SM
        + detail.size().y.max(20.0)
        + note_galleys.iter().map(|g| g.size().y + 2.0).sum::<f32>()
        + theme::SPACE_SM;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let mut label = format!("{slot} {} {}", ch.kind.label(), ch.detail);
    for (side, n) in notes {
        if let Some(n) = n {
            label.push_str(&format!(" · {side}: {n}"));
        }
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &label));
    let painter = ui.painter();
    if selected {
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_MD),
            theme::selected_fill(t),
        );
    } else if resp.hovered() {
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_MD),
            theme::ink_hover(t),
        );
    }
    let top = rect.top() + theme::SPACE_SM;
    let kind = ch.kind.color(t);
    let badge = ui.painter().layout_no_wrap(
        ch.kind.label().to_owned(),
        theme::medium(ui.ctx(), theme::SMALL_SIZE),
        kind,
    );
    let pill = Rect::from_min_size(
        pos2(rect.left() + theme::SPACE_SM, top),
        vec2(badge.size().x + theme::SPACE_SM + theme::SPACE_XS, 20.0),
    );
    painter.rect_filled(
        pill,
        CornerRadius::same(theme::RADIUS),
        kind.gamma_multiply(0.15),
    );
    painter.galley(pill.center() - badge.size() / 2.0, badge, kind);
    painter.text(
        pos2(rect.left() + theme::SPACE_SM + KIND_W, top + 10.0),
        Align2::LEFT_CENTER,
        slot,
        theme::tabular(ui.ctx(), theme::BODY_SIZE),
        theme::rgb(t.text_muted),
    );
    let text_x = rect.left() + theme::SPACE_SM + KIND_W + SLOT_W;
    let mut y = top + (20.0 - detail.size().y.min(20.0)) / 2.0;
    let detail_h = detail.size().y.max(20.0);
    painter.galley(pos2(text_x, y), detail, theme::rgb(t.text));
    y = top + detail_h + 2.0;
    for g in note_galleys {
        let h = g.size().y;
        painter.galley(pos2(text_x, y), g, theme::rgb(t.text_muted));
        y += h + 2.0;
    }
    resp
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
            accepted: Vec::new(),
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

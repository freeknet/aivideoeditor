// ============================================================================
// Module:       gui::pages::inspector::shot_list
// Description:  The inspector's Shot list tab, Selects' Transcript for a cut:
//               every shot of the open plan in timeline order, grouped by
//               song section, with its time, bar, segment, caption, note and
//               check problems; click to select and seek, keys to walk it.
//
// Dependencies: egui; crate::{plan, beats, index, refs, project},
//               crate::gui::{player, ui}
// ============================================================================

//! The cut read as text.
//!
//! Selects' right panel has a Transcript tab: the edit as words, a cursor
//! on the word under the playhead. ave's footage has no words, so its
//! transcript is the plan itself, the way `shotlog.md` reads: one row per
//! shot, in order, under the song section it plays over (intro, build,
//! drop, …), each saying *what* is on screen (the caption line from the
//! index) and *why* it is there (the plan's `note`), with `plan::check`'s
//! problems on that shot as a quiet icon.
//!
//! The open plan is the one the timeline has open (its `current` in the
//! night's settings), else the first plan file. Clicking a row selects the
//! shot app-wide and seeks the viewer to it; while the viewer plays the
//! plan, the row under the playhead carries a lime marker.
//!
//! Keys, while the inspector is the focused panel: Up / Down walk the
//! rows, Enter plays from the selected one. Delete / Backspace lift the
//! selected shots through the timeline's undoable, versioned edit path
//! (`Action::LiftShots`), whether or not the timeline is open; the
//! workspace's `R` mentions them.

use std::path::PathBuf;

use egui::{Align, Sense, Ui};

use super::PageCtx;
use super::rows::{self, SMALL, text};
use super::usage::{self, Plans};
use crate::beats::MusicMap;
use crate::gui::app::Action;
use crate::gui::player::{EndCard, Player, SeqShot, Source};
use crate::gui::ui::{icons, theme, widgets};
use crate::index::Index;
use crate::plan::{Plan, Problem};
use crate::project::Project;
use crate::refs::{Reference, Target};
use crate::theme::Palette;

/// Width of the shot-number column.
const NUMBER_W: f32 = 24.0;

/// The tab's own state.
#[derive(Debug, Default)]
pub(super) struct State {
    /// The plan `problems` were checked for, by name and content.
    checked: Option<(String, Plan)>,
    /// `plan::check` on it.
    problems: Vec<Problem>,
    /// The row under the playhead last frame (0-based), to scroll to it
    /// once when it changes.
    active: Option<usize>,
}

/// One shot as the list shows it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Line {
    /// Shot index, 0-based.
    pub(super) k: usize,
    /// Seconds from the plan's start.
    pub(super) start: f32,
    /// Bar the shot starts in.
    pub(super) bar: usize,
    /// Beat within that bar, from 1.
    pub(super) beat: usize,
    /// The source segment's id (`MVI_7925/3`), or the clip when the index
    /// does not know it.
    pub(super) seg: String,
    /// The segment's caption line, when captioned.
    pub(super) caption: String,
    /// The plan's note.
    pub(super) note: String,
    /// `plan::check`'s problems on this shot.
    pub(super) problems: Vec<String>,
    /// The song section the shot starts in (`""` when unmapped).
    pub(super) section: String,
}

/// What a click or key asked for, applied after drawing.
enum Op {
    /// Select shot `k` and seek to it.
    Select(usize),
    /// Select shot `k` and play from it.
    Play(usize),
}

/// The plan the list reads: the timeline's open one, else the first.
fn open_plan(project: &Project, plans: &Plans) -> Option<String> {
    let current = project
        .settings
        .pages
        .get("plan")
        .and_then(|t| t.get("current"))
        .and_then(|v| v.as_str())
        .filter(|name| plans.get(name).is_some());
    current
        .map(str::to_owned)
        .or_else(|| plans.names().into_iter().next())
}

/// `MM:SS.s`.
pub(super) fn clock(secs: f32) -> String {
    let tenths = (secs.max(0.0) * 10.0).round() as u64;
    format!(
        "{:02}:{:02}.{}",
        tenths / 600,
        tenths % 600 / 10,
        tenths % 10
    )
}

/// The rows of `plan`, in timeline order.
pub(super) fn lines(
    plan: &Plan,
    index: Option<&Index>,
    map: Option<&MusicMap>,
    problems: &[Problem],
) -> Vec<Line> {
    let starts: Vec<f32> = match map {
        Some(m) => plan.timing(m).into_iter().map(|(t, _)| t).collect(),
        None => usage::shot_lengths(plan, None)
            .iter()
            .scan(0.0, |at, len| {
                let t = *at;
                *at += len;
                Some(t)
            })
            .collect(),
    };
    let mut beats_before = 0;
    plan.shots
        .iter()
        .enumerate()
        .map(|(k, shot)| {
            let bar = plan.start_bar + beats_before / 4;
            let beat = beats_before % 4 + 1;
            beats_before += shot.beats;
            let seg = index.and_then(|i| {
                let mut in_clip = i.segments.iter().filter(|s| s.clip == shot.clip);
                if shot.seg.is_empty() {
                    in_clip.find(|s| (s.seg.start..s.seg.end).contains(&shot.t_in))
                } else {
                    i.segments.iter().find(|s| s.id == shot.seg)
                }
            });
            let section = map
                .and_then(|m| {
                    m.sections
                        .iter()
                        .find(|s| s.from_bar <= bar && bar <= s.to_bar)
                })
                .map(|s| s.name.clone())
                .unwrap_or_default();
            Line {
                k,
                start: starts.get(k).copied().unwrap_or(0.0),
                bar,
                beat,
                seg: seg.map_or_else(
                    || {
                        if shot.seg.is_empty() {
                            format!("{} @ {:.1}s", shot.clip, shot.t_in)
                        } else {
                            shot.seg.clone()
                        }
                    },
                    |s| s.id.clone(),
                ),
                caption: seg
                    .and_then(|s| s.caption.as_ref())
                    .map(|c| c.summary.clone())
                    .unwrap_or_default(),
                note: shot.note.clone(),
                problems: problems
                    .iter()
                    .filter(|p| p.shot == k + 1)
                    .map(|p| p.message.clone())
                    .collect(),
                section,
            }
        })
        .collect()
}

/// Runs of consecutive lines in the same section: (name, first, end).
pub(super) fn groups(lines: &[Line]) -> Vec<(String, usize, usize)> {
    let mut out: Vec<(String, usize, usize)> = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        match out.last_mut() {
            Some((name, _, end)) if *name == line.section => *end = n + 1,
            Some(_) | None => out.push((line.section.clone(), n, n + 1)),
        }
    }
    out
}

/// The plan as the viewer plays it (the timeline's own recipe): shots back
/// to back over the music, then the end card for the span the render
/// holds it.
fn cut(
    plan: &Plan,
    project: &Project,
    map: &MusicMap,
) -> (Vec<SeqShot>, Option<(PathBuf, f64)>, Option<EndCard>) {
    let shots = plan
        .shots
        .iter()
        .zip(plan.timing(map))
        .map(|(s, (_, dur))| SeqShot {
            path: project
                .index
                .as_ref()
                .and_then(|i| i.clip(&s.clip))
                .map(|c| PathBuf::from(&c.path))
                .unwrap_or_default(),
            t_in: f64::from(s.t_in),
            dur: f64::from(dur),
            cx: s.cx,
        })
        .collect();
    let music = (!plan.music.is_empty()).then(|| {
        (
            PathBuf::from(&plan.music),
            f64::from(map.beat_time(plan.start_bar, 0)),
        )
    });
    let (from, to) = plan.card_span(map);
    let card = (!plan.end_card.is_empty() && to > from).then(|| EndCard {
        path: PathBuf::from(&plan.end_card),
        dur: f64::from(to - from),
    });
    (shots, music, card)
}

/// Whether the viewer holds exactly these shots as a sequence.
fn holds(player: &Player, shots: &[SeqShot]) -> bool {
    matches!(player.source(), Source::Sequence { shots: s, .. } if s.as_slice() == shots)
}

/// Draws the Shot list tab.
pub(super) fn draw(ctx: &mut PageCtx<'_>, plans: &mut Plans, state: &mut State, ui: &mut Ui) {
    let palette: &Palette = ctx.theme;
    let Some(project) = ctx.project.as_ref() else {
        widgets::empty_state(
            ui,
            palette,
            icons::FOLDER_OPEN,
            "No night open",
            "Open a night to read its plans as shot lists.",
            |_| {},
        );
        return;
    };
    plans.refresh(&project.plans);
    let Some((name, plan)) =
        open_plan(project, plans).and_then(|n| plans.get(&n).map(|p| (n, p.clone())))
    else {
        widgets::empty_state(
            ui,
            palette,
            icons::CLAPPERBOARD,
            "No plan open",
            "Draft a plan on the timeline to read it here, shot by shot.",
            |_| {},
        );
        return;
    };
    if plan.shots.is_empty() {
        widgets::empty_state(
            ui,
            palette,
            icons::LIST,
            "No shots yet",
            &format!("{name} has no shots. Add some on the timeline."),
            |_| {},
        );
        return;
    }

    // Problems, re-checked only when the plan changes.
    let fresh = state
        .checked
        .as_ref()
        .is_none_or(|(n, p)| *n != name || *p != plan);
    if fresh {
        state.problems = match (project.index.as_ref(), project.music.as_ref()) {
            (Some(index), Some(map)) => crate::plan::check(&plan, index, map, &project.people),
            (None, _) | (_, None) => Vec::new(),
        };
        state.checked = Some((name.clone(), plan.clone()));
    }
    let map = project.music.as_ref();
    let lines = lines(&plan, project.index.as_ref(), map, &state.problems);
    let cut = map.map(|m| cut(&plan, project, m));
    let ours = cut
        .as_ref()
        .is_some_and(|(shots, _, _)| holds(ctx.player, shots));
    let playing = ours && ctx.player.playing();
    let active = ours
        .then(|| ctx.player.sequence_shot().map(|(k, _)| k))
        .flatten();
    let selected: Option<usize> = ctx.selection.iter().find_map(|r| match &r.target {
        Target::Shot { plan: p, index } if *p == name && *index >= 1 => Some(index - 1),
        Target::Shot { .. }
        | Target::Segment { .. }
        | Target::Range { .. }
        | Target::Frame { .. }
        | Target::Bars { .. }
        | Target::Person { .. }
        | Target::Plan { .. } => None,
    });
    let is_selected = |k: usize| {
        ctx.selection.iter().any(|r| {
            r.target
                == Target::Shot {
                    plan: name.clone(),
                    index: k + 1,
                }
        })
    };

    // Delete / Backspace lift the selected shots whenever the list has a
    // selection, open timeline or not: the app routes it through the
    // timeline's undoable, versioned lift. (An open timeline reads the key
    // first and clears the selection, so a shot is never lifted twice.)
    let lift: Vec<usize> = (0..lines.len()).filter(|&k| is_selected(k)).collect();
    if !lift.is_empty() && ctx.keys_free(ui) {
        use egui::{Key, Modifiers};
        let delete = ui.input_mut(|i| {
            let del = i.consume_key(Modifiers::NONE, Key::Delete);
            i.consume_key(Modifiers::NONE, Key::Backspace) || del
        });
        if delete {
            ctx.actions.push(Action::LiftShots {
                plan: name.clone(),
                shots: lift,
            });
        }
    }

    // Keys, while the inspector is the panel last pressed in.
    let mut ops = Vec::new();
    let mut moved = false;
    if ctx.keys_free(ui) && crate::gui::ui::workspace::panel_focused(ui.ctx(), "inspector") {
        use egui::{Key, Modifiers};
        let (down, up, enter) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::Enter),
            )
        });
        let last = lines.len() - 1;
        if down {
            ops.push(Op::Select(selected.map_or(0, |k| (k + 1).min(last))));
            moved = true;
        }
        if up {
            ops.push(Op::Select(selected.map_or(0, |k| k.saturating_sub(1))));
            moved = true;
        }
        if enter {
            ops.push(Op::Play(selected.or(active).unwrap_or(0)));
        }
    }
    let scroll_to = if moved {
        ops.last().map(|op| match op {
            Op::Select(k) | Op::Play(k) => *k,
        })
    } else if playing && active != state.active {
        active
    } else {
        None
    };
    state.active = active;

    let mut actions: Vec<Action> = Vec::new();
    let total: f32 = cut.as_ref().map_or(0.0, |(shots, _, _)| {
        shots.iter().map(|s| s.dur as f32).sum()
    });
    egui::ScrollArea::vertical()
        .id_salt("inspector-shot-list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 0,
                    right: 0,
                    top: 0,
                    bottom: theme::as_margin(theme::SPACE_LG),
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.style_mut().interaction.selectable_labels = false;
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(
                            theme::as_margin(theme::SPACE_SM),
                            0,
                        ))
                        .show(ui, |ui| {
                            let shots = plan.shots.len();
                            let subtitle = if total > 0.0 {
                                format!("{shots} shots · {}", clock(total))
                            } else {
                                format!("{shots} shots")
                            };
                            rows::title(ui, palette, &name, &subtitle);
                        });
                    for (section, from, to) in groups(&lines) {
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(
                                theme::as_margin(theme::SPACE_SM),
                                0,
                            ))
                            .show(ui, |ui| {
                                let title = if section.is_empty() {
                                    "Shots"
                                } else {
                                    section.as_str()
                                };
                                let count = to - from;
                                let note = if count == 1 {
                                    "1 shot".to_owned()
                                } else {
                                    format!("{count} shots")
                                };
                                rows::section_with(ui, palette, title, &note);
                            });
                        for line in lines.get(from..to).unwrap_or_default() {
                            let reference = Reference::new(Target::Shot {
                                plan: name.clone(),
                                index: line.k + 1,
                            });
                            let marked = playing && active == Some(line.k);
                            let resp = row(ui, palette, line, is_selected(line.k), marked);
                            if scroll_to == Some(line.k) {
                                resp.scroll_to_me(Some(Align::Center));
                            }
                            if resp.clicked() {
                                ops.push(Op::Select(line.k));
                            }
                            if resp.double_clicked() {
                                ops.push(Op::Play(line.k));
                            }
                            resp.context_menu(|ui| {
                                widgets::mention_menu(ui, &mut actions, &reference);
                            });
                        }
                    }
                });
        });
    ctx.actions.extend(actions);

    for op in ops {
        let (k, play) = match op {
            Op::Select(k) => (k, false),
            Op::Play(k) => (k, true),
        };
        let Some(line) = lines.get(k) else {
            continue;
        };
        *ctx.selection = vec![Reference::new(Target::Shot {
            plan: name.clone(),
            index: k + 1,
        })];
        match &cut {
            Some((shots, music, card)) => {
                if !holds(ctx.player, shots) {
                    ctx.player
                        .open_cut(shots.clone(), music.clone(), card.clone());
                }
                ctx.player.seek(f64::from(line.start));
            }
            None => {
                // No music map: show the shot's footage on its own.
                let Some(shot) = plan.shots.get(k) else {
                    continue;
                };
                let len = usage::shot_lengths(&plan, None)
                    .get(k)
                    .copied()
                    .unwrap_or(0.0);
                let path = project
                    .index
                    .as_ref()
                    .and_then(|i| i.clip(&shot.clip))
                    .map(|c| PathBuf::from(&c.path))
                    .unwrap_or_default();
                ctx.player.open_clip(
                    &path,
                    f64::from(shot.t_in),
                    f64::from(shot.t_in + len),
                    shot.cx,
                );
            }
        }
        if play {
            ctx.player.play();
        }
    }
}

/// One shot's row; returns its click response (labelled `Shot <n>`).
fn row(ui: &mut Ui, p: &Palette, line: &Line, selected: bool, marked: bool) -> egui::Response {
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = egui::Frame::new()
        .inner_margin(egui::Margin {
            left: theme::as_margin(theme::SPACE_SM),
            right: theme::as_margin(theme::SPACE_SM),
            top: 3,
            bottom: 3,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = egui::vec2(theme::SPACE_SM, 2.0);
            let muted = theme::rgb(p.text_muted);
            let number = if marked { theme::rgb(p.accent) } else { muted };
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(NUMBER_W, SMALL + 4.0),
                    egui::Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_width(NUMBER_W);
                        ui.label(
                            egui::RichText::new((line.k + 1).to_string())
                                .font(theme::tabular(ui.ctx(), SMALL))
                                .color(number),
                        );
                    },
                );
                ui.label(
                    egui::RichText::new(clock(line.start))
                        .font(theme::tabular(ui.ctx(), SMALL))
                        .color(if marked { number } else { theme::rgb(p.text) }),
                );
                let bar = if line.beat == 1 {
                    format!("bar {}", line.bar)
                } else {
                    format!("bar {}.{}", line.bar, line.beat)
                };
                ui.label(
                    egui::RichText::new(bar)
                        .font(theme::tabular(ui.ctx(), SMALL))
                        .color(muted),
                );
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if !line.problems.is_empty() {
                        ui.label(
                            egui::RichText::new(icons::TRIANGLE_ALERT.to_string())
                                .font(theme::font(icons::SMALL))
                                .color(theme::rgb(p.warning)),
                        );
                    }
                    ui.add(
                        egui::Label::new(text(&line.seg, SMALL, muted))
                            .wrap_mode(egui::TextWrapMode::Truncate),
                    );
                });
            });
            let indent = NUMBER_W + theme::SPACE_SM;
            if !line.note.is_empty() {
                ui.horizontal_top(|ui| {
                    ui.add_space(indent);
                    ui.add(egui::Label::new(text(&line.note, SMALL, theme::rgb(p.text))).wrap());
                });
            }
            if !line.caption.is_empty() {
                ui.horizontal_top(|ui| {
                    ui.add_space(indent);
                    ui.add(egui::Label::new(text(&line.caption, SMALL, muted)).wrap());
                });
            }
        });
    let rect = inner.response.rect;
    let resp = ui.interact(
        rect,
        ui.scope_id().with(("shot-list-row", line.k)),
        Sense::click(),
    );
    let n = line.k + 1;
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Shot {n}")));
    let fill = if selected {
        theme::selected_fill(p)
    } else if resp.hovered() {
        theme::ink(p, theme::INK_08)
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().set(
        bg,
        egui::Shape::rect_filled(rect, egui::CornerRadius::same(theme::RADIUS), fill),
    );
    if marked {
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height()));
        ui.painter()
            .rect_filled(bar, egui::CornerRadius::same(1), theme::rgb(p.accent));
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if line.problems.is_empty() {
        resp
    } else {
        widgets::tip(resp, p, &line.problems.join("\n"), "")
    }
}

#[cfg(test)]
mod tests {
    use super::{Line, clock, groups, lines};
    use crate::beats::{Bar, MusicMap, Section};
    use crate::plan::{Aspect, Plan, Problem, Rules, Shot};

    fn shot(clip: &str, note: &str) -> Shot {
        Shot {
            clip: clip.into(),
            t_in: 1.0,
            beats: 4,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: note.into(),
        }
    }

    fn map() -> MusicMap {
        MusicMap {
            path: "m.wav".into(),
            duration: 60.0,
            bpm: 120.0,
            beats: (0..120).map(|i| i as f32 * 0.5).collect(),
            bars: (0..30)
                .map(|i| Bar {
                    t: i as f32 * 2.0,
                    rms: 1.0,
                    low: 1.0,
                })
                .collect(),
            sections: vec![
                Section {
                    name: "intro".into(),
                    from_bar: 0,
                    to_bar: 1,
                },
                Section {
                    name: "drop".into(),
                    from_bar: 2,
                    to_bar: 5,
                },
            ],
            ..MusicMap::default()
        }
    }

    fn plan() -> Plan {
        Plan {
            name: "reel".into(),
            aspect: Aspect::Vertical,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 2,
            card_bar: 4,
            end_bar: 5,
            end_card: String::new(),
            rules: Rules::default(),
            finish: Default::default(),
            shots: vec![
                shot("A", "opener"),
                shot("B", "builds"),
                shot("C", "hero fire on the drop"),
                shot("D", "closer"),
            ],
            comments: Vec::new(),
        }
    }

    #[test]
    fn clock_reads_minutes_seconds_tenths() -> anyhow::Result<()> {
        assert_eq!(clock(0.0), "00:00.0", "zero");
        assert_eq!(clock(4.04), "00:04.0", "seconds");
        assert_eq!(clock(59.96), "01:00.0", "rounds up into the minute");
        assert_eq!(clock(75.25), "01:15.3", "minutes");
        Ok(())
    }

    #[test]
    fn lines_follow_the_plan_with_bars_sections_and_problems() -> anyhow::Result<()> {
        let problems = vec![
            Problem {
                shot: 3,
                message: "`C` again".into(),
            },
            Problem {
                shot: 0,
                message: "whole plan".into(),
            },
        ];
        let got = lines(&plan(), None, Some(&map()), &problems);
        let notes: Vec<&str> = got.iter().map(|l| l.note.as_str()).collect();
        assert_eq!(
            notes,
            vec!["opener", "builds", "hero fire on the drop", "closer"],
            "timeline order"
        );
        let at: Vec<(f32, usize)> = got.iter().map(|l| (l.start, l.bar)).collect();
        assert_eq!(
            at,
            vec![(0.0, 0), (2.0, 1), (4.0, 2), (6.0, 3)],
            "a bar per shot at 120 bpm"
        );
        let problems: Vec<usize> = got.iter().map(|l| l.problems.len()).collect();
        assert_eq!(
            problems,
            vec![0, 0, 1, 0],
            "only shot 3's own problem; whole-plan ones stay off the rows"
        );
        assert_eq!(
            got.first().map(|l| l.seg.as_str()),
            Some("A @ 1.0s"),
            "without an index the clip and in-point stand in for the segment"
        );
        Ok(())
    }

    #[test]
    fn groups_are_runs_of_one_section() -> anyhow::Result<()> {
        let got: Vec<Line> = lines(&plan(), None, Some(&map()), &[]);
        assert_eq!(
            groups(&got),
            vec![("intro".to_owned(), 0, 2), ("drop".to_owned(), 2, 4)],
            "two chapters, like a transcript's"
        );
        let unmapped = lines(&plan(), None, None, &[]);
        assert_eq!(
            groups(&unmapped),
            vec![(String::new(), 0, 4)],
            "no music map: one group"
        );
        Ok(())
    }

    /// A night with the music map above and `plan()` saved as `reel`; no
    /// index (the list and the viewer need none).
    fn night() -> anyhow::Result<(tempfile::TempDir, std::path::PathBuf)> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-12").join("_ave");
        std::fs::create_dir_all(work.join("plans"))?;
        map().save(&work)?;
        plan().save(&work.join("plans").join("reel.toml"))?;
        Ok((dir, work))
    }

    type Harness = egui_kittest::Harness<'static, Option<crate::gui::app::App>>;

    /// The app on `work` with the Shot list tab open, headless.
    fn harness(work: std::path::PathBuf) -> anyhow::Result<Harness> {
        use crate::gui::app::{Action, App, Page};
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1680.0, 1000.0))
            .with_accessibility_check(false)
            .build_ui_state(
                move |ui, app: &mut Option<App>| {
                    let app = app.get_or_insert_with(|| {
                        let mut app = App::new(
                            ui.ctx(),
                            crate::config::Config::default(),
                            Some(work.clone()),
                        );
                        app.apply(ui.ctx(), Action::Go(Page::Plan));
                        app
                    });
                    app.show(ui);
                },
                None,
            );
        h.run_steps(6);
        h.query_by_label("Shot list")
            .ok_or_else(|| anyhow::anyhow!("no Shot list tab"))?
            .click();
        h.run_steps(3);
        Ok(h)
    }

    fn app(h: &Harness) -> anyhow::Result<&crate::gui::app::App> {
        h.state().as_ref().ok_or_else(|| anyhow::anyhow!("no app"))
    }

    fn shot_ref(index: usize) -> crate::refs::Reference {
        crate::refs::Reference::new(crate::refs::Target::Shot {
            plan: "reel".into(),
            index,
        })
    }

    #[test]
    fn the_tab_lists_every_shot_in_timeline_order() -> anyhow::Result<()> {
        use egui_kittest::kittest::Queryable;
        let (_dir, work) = night()?;
        let h = harness(work)?;
        let mut ys = Vec::new();
        for n in 1..=4 {
            let label = format!("Shot {n}");
            let row = h
                .query_by_label(&label)
                .ok_or_else(|| anyhow::anyhow!("no row for shot {n}"))?;
            ys.push(row.rect().top());
        }
        assert!(
            ys.windows(2).all(|w| w[0] < w[1]),
            "rows run down in shot order: {ys:?}"
        );
        assert!(
            h.query_all_by_label_contains("hero fire on the drop")
                .next()
                .is_some(),
            "each row carries the plan's note"
        );
        Ok(())
    }

    #[test]
    fn clicking_a_row_selects_the_shot_and_seeks_to_it() -> anyhow::Result<()> {
        use egui_kittest::kittest::Queryable;
        let (_dir, work) = night()?;
        let mut h = harness(work)?;
        h.query_by_label("Shot 3")
            .ok_or_else(|| anyhow::anyhow!("no row for shot 3"))?
            .click();
        h.run_steps(3);
        let a = app(&h)?;
        assert_eq!(a.selection, vec![shot_ref(3)], "the shot is the selection");
        assert!(
            (a.player.position() - 4.0).abs() < 1e-6,
            "the viewer sits at shot 3's start, 4 s in: {}",
            a.player.position()
        );
        h.key_press(egui::Key::ArrowDown);
        h.run_steps(3);
        let a = app(&h)?;
        assert_eq!(a.selection, vec![shot_ref(4)], "Down walks to the next row");
        assert!(
            (a.player.position() - 6.0).abs() < 1e-6,
            "and seeks with it: {}",
            a.player.position()
        );
        h.key_press(egui::Key::ArrowUp);
        h.run_steps(3);
        assert_eq!(app(&h)?.selection, vec![shot_ref(3)], "Up walks back");
        Ok(())
    }

    #[test]
    fn enter_plays_and_delete_lifts_through_the_timeline() -> anyhow::Result<()> {
        use egui_kittest::kittest::Queryable;
        let (_dir, work) = night()?;
        let mut h = harness(work.clone())?;
        h.query_by_label("Shot 2")
            .ok_or_else(|| anyhow::anyhow!("no row for shot 2"))?
            .click();
        h.run_steps(3);
        h.key_press(egui::Key::Enter);
        h.run_steps(1);
        assert!(app(&h)?.player.playing(), "Enter plays from the row");
        if let Some(a) = h.state_mut().as_mut() {
            a.player.pause();
        }
        h.key_press(egui::Key::Delete);
        h.run_steps(3);
        assert!(
            app(&h)?.selection.is_empty(),
            "the timeline lifted the selected shot and cleared the selection"
        );
        // The timeline saves its edit a moment later, as a new version.
        std::thread::sleep(std::time::Duration::from_millis(1500));
        h.run_steps(3);
        let live = crate::plan::Plan::load(&work.join("plans").join("reel.toml"))?;
        let clips: Vec<&str> = live.shots.iter().map(|s| s.clip.as_str()).collect();
        assert_eq!(clips, vec!["A", "C", "D"], "shot 2 is gone");
        assert_eq!(
            live.shots.first().map(|s| s.beats),
            Some(8),
            "its beats went to the shot before, as the timeline's lift does"
        );
        Ok(())
    }

    /// Regression: Delete in the Shot list did nothing while the timeline
    /// was folded, because only the timeline read the key.
    #[test]
    fn delete_lifts_with_the_timeline_folded() -> anyhow::Result<()> {
        use crate::gui::ui::layout::Region;
        use egui_kittest::kittest::Queryable;
        let (_dir, work) = night()?;
        let mut h = harness(work.clone())?;
        if let Some(a) = h.state_mut().as_mut() {
            a.layout.set_open(Region::Timeline, false);
        }
        h.run_steps(3);
        assert!(
            !app(&h)?.layout.is_open(Region::Timeline),
            "the timeline is folded"
        );
        let versions = || -> usize {
            std::fs::read_dir(work.join("plans").join(".history").join("reel"))
                .map_or(0, |d| d.filter_map(Result::ok).count())
        };
        let before = versions();
        h.query_by_label("Shot 2")
            .ok_or_else(|| anyhow::anyhow!("no row for shot 2"))?
            .click();
        h.run_steps(3);
        assert_eq!(app(&h)?.selection, vec![shot_ref(2)], "shot 2 is selected");
        h.key_press(egui::Key::Delete);
        h.run_steps(3);
        assert!(
            app(&h)?.selection.is_empty(),
            "the lift cleared the selection"
        );
        let live = crate::plan::Plan::load(&work.join("plans").join("reel.toml"))?;
        let clips: Vec<&str> = live.shots.iter().map(|s| s.clip.as_str()).collect();
        assert_eq!(clips, vec!["A", "C", "D"], "shot 2 is gone, saved at once");
        assert_eq!(
            live.shots.first().map(|s| s.beats),
            Some(8),
            "its beats went to the shot before, as the timeline's lift does"
        );
        assert!(
            versions() > before,
            "the edit was saved as a new version ({before} before)"
        );
        h.query_by_label("Shot 3")
            .ok_or_else(|| anyhow::anyhow!("no row for shot 3"))?
            .click();
        h.run_steps(3);
        h.key_press(egui::Key::Backspace);
        h.run_steps(3);
        let live = crate::plan::Plan::load(&work.join("plans").join("reel.toml"))?;
        let clips: Vec<&str> = live.shots.iter().map(|s| s.clip.as_str()).collect();
        assert_eq!(clips, vec!["A", "C"], "Backspace lifts too");
        Ok(())
    }
}

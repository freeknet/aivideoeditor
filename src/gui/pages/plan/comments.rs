// ============================================================================
// Module:       gui::pages::plan::comments
// Description:  Markers (review comments) on the open plan: the list beside
//               the tracks (seek, edit, recolour, resolve, delete), adding
//               one at the playhead, jumping between them, and the batch
//               message that hands them to the chat.
//
// Dependencies: egui; crate::{plan, refs}, crate::gui::pages::plan
// ============================================================================

//! Markers and comments, the way Selects reviews a cut.
//!
//! Selects has one concept here and so does ave: a marker is a moment of
//! the cut with a colour, and a marker with a note *is* a review comment.
//! Each is a `[[comment]]` in the plan's TOML (`[[marker]]` is read too;
//! [`crate::plan::Comment`]): a time on the cut, a note, a colour, who left
//! it and whether it is dealt with. `M` (or `F`) drops one at the playhead
//! and focuses its text; `Shift+M` and `Alt+M` go to the next and previous
//! one; the ruler shows each as a coloured pin (click to select and seek,
//! double-click to edit, `Delete` removes the selected one); this list
//! seeks on click, edits on double-click, recolours from the swatches of
//! the selected row, resolves and deletes. **Send to chat** writes every
//! open comment as one message, each with its timecode and the shot under
//! it as an
//! `@shot:<plan>#<n>` reference, and asks the assistant for a plan covering
//! all of them that waits for the go-ahead, so the review turns into one
//! proposal rather than a string of edits.

use std::collections::BTreeSet;

use egui::{Color32, CornerRadius, Rect, Sense, Ui, UiBuilder, vec2};

use super::{Io, Shown, State};
use crate::gui::app::Action;
use crate::gui::ui::{icons, theme, widgets};
use crate::plan::{COMMENT_COLORS, Comment, Plan};
use crate::refs::{Reference, Target};

/// Why an edit control on the timeline is disabled: an older version is
/// open, or the plan is locked.
pub(super) const LOCKED: &str = "Viewing an older version, or the cut is locked";

/// A labelled change to the plan, applied after the list is drawn.
type Edit = (String, Box<dyn FnOnce(&mut Plan) -> bool>);

/// The marker colour for a comment colour name (Selects' marker paint).
pub(super) fn color(name: &str) -> Color32 {
    super::timeline::marker(name).0
}

/// The colour after `name` in the cycle.
fn next_color(name: &str) -> &'static str {
    let at = COMMENT_COLORS.iter().position(|c| *c == name).unwrap_or(0);
    COMMENT_COLORS
        .get((at + 1) % COMMENT_COLORS.len())
        .copied()
        .unwrap_or("green")
}

/// The shot (0-based) playing at `t` seconds, if any (not the end card).
pub(super) fn shot_at(timing: &[(f32, f32)], t: f32) -> Option<usize> {
    timing.iter().position(|&(a, d)| t >= a && t < a + d)
}

/// Adds an empty comment by "you" at `t`; returns its index.
pub(super) fn add(plan: &mut Plan, t: f32) -> usize {
    plan.comments.push(Comment {
        time: t.max(0.0),
        note: String::new(),
        color: COMMENT_COLORS[0].to_owned(),
        author: "you".to_owned(),
        resolved: false,
    });
    plan.comments.len() - 1
}

/// The marker to go to from `t` seconds: the first one after it (or the
/// last one before it), skipping resolved ones unless they are shown.
/// Markers within a hundredth of a second of `t` count as "here".
pub(super) fn jump_target(
    comments: &[Comment],
    t: f32,
    forward: bool,
    show_resolved: bool,
) -> Option<usize> {
    const HERE: f32 = 0.01;
    let mut shown = by_time(comments)
        .into_iter()
        .filter(|&i| show_resolved || !comments[i].resolved);
    if forward {
        shown.find(|&i| comments[i].time > t + HERE)
    } else {
        shown.rfind(|&i| comments[i].time < t - HERE)
    }
}

/// The playhead in plan seconds (the selected shot's start when the viewer
/// is not on this plan).
fn playhead(s: &Shown<'_>, io: &Io<'_>) -> f32 {
    if s.ours {
        io.player.position() as f32
    } else {
        s.single()
            .and_then(|k| s.timing.get(k))
            .map_or(0.0, |&(a, _)| a)
    }
    .clamp(0.0, s.total)
}

/// Seeks to the next (or previous) marker and selects it.
pub(super) fn jump(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, forward: bool) {
    let comments = &s.plan.comments;
    let Some(i) = jump_target(comments, playhead(s, io), forward, state.show_resolved) else {
        state.status = format!("no {} marker", if forward { "later" } else { "earlier" });
        return;
    };
    if let Some(c) = comments.get(i) {
        io.seek(s, c.time.clamp(0.0, s.total));
    }
    select(state, s, io, i);
}

/// Makes marker `i` the selection (shots deselected, so `Delete` takes it).
pub(super) fn select(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, i: usize) {
    state.comment_sel = Some(i);
    io.select(&s.name, &BTreeSet::new());
}

/// Deletes the selected marker; whether there was one.
pub(super) fn delete_selected(state: &mut State) -> bool {
    let Some(i) = state.comment_sel.take() else {
        return false;
    };
    state.comment_edit = None;
    state.comment_focus = None;
    state.apply("delete marker", |p| {
        if i < p.comments.len() {
            p.comments.remove(i);
            true
        } else {
            false
        }
    })
}

/// Opens marker `i` for editing: the list shows it selected, with its
/// colour swatches and its note in a text box.
pub(super) fn edit(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, i: usize) {
    let Some(c) = s.plan.comments.get(i) else {
        return;
    };
    select(state, s, io, i);
    state.show_comments = true;
    if s.editable {
        state.comment_edit = Some((i, c.note.clone()));
        state.comment_focus = Some(i);
    }
}

/// Indices of the comments in time order.
pub(super) fn by_time(comments: &[Comment]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..comments.len()).collect();
    order.sort_by(|&a, &b| comments[a].time.total_cmp(&comments[b].time));
    order
}

/// The message "Send comments to chat" hands the assistant: every open
/// comment in time order with its timecode and the shot under it, then the
/// instruction to propose one plan and wait. `None` when nothing is open.
pub(super) fn chat_text(name: &str, comments: &[Comment], timing: &[(f32, f32)]) -> Option<String> {
    let open: Vec<usize> = by_time(comments)
        .into_iter()
        .filter(|&i| !comments[i].resolved)
        .collect();
    if open.is_empty() {
        return None;
    }
    let mut text = format!("Review comments on plan `{name}` ({} open):\n", open.len());
    for (n, &i) in open.iter().enumerate() {
        let c = &comments[i];
        let at = super::timeline::timecode(c.time, true);
        let under = match shot_at(timing, c.time) {
            Some(k) => Reference::new(Target::Shot {
                plan: name.to_owned(),
                index: k + 1,
            })
            .to_string(),
            None => format!(
                "{} (end card)",
                Reference::new(Target::Plan {
                    name: name.to_owned()
                })
            ),
        };
        let note = if c.note.trim().is_empty() {
            "(no note: look at this moment)"
        } else {
            c.note.trim()
        };
        text.push_str(&format!("{}. [{at}] {under}: {note}\n", n + 1));
    }
    text.push_str(
        "\nPropose one plan that covers every comment above, saying which change \
         answers which comment number. Do not edit the plan until I approve it.",
    );
    Some(text)
}

/// The comments list, drawn into `rect` beside the tracks.
pub(super) fn panel(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, ui: &mut Ui, rect: Rect) {
    let t = s.theme;
    ui.painter()
        .vline(rect.left() + 0.5, rect.y_range(), theme::ink_line(t));
    let comments = &s.plan.comments;
    let open = comments.iter().filter(|c| !c.resolved).count();
    let mut seek = None;
    let mut edits: Vec<Edit> = Vec::new();
    let inner = Rect::from_min_max(
        rect.min + vec2(theme::SPACE_SM + 1.0, theme::SPACE_XS),
        rect.max - vec2(theme::SPACE_SM, theme::SPACE_XS),
    );
    ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
        ui.spacing_mut().item_spacing = vec2(theme::SPACE_SM, 2.0);
        ui.horizontal(|ui| {
            ui.set_min_height(theme::CONTROL_HEIGHT);
            ui.label(
                egui::RichText::new("Comments")
                    .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(t.text)),
            );
            widgets::pill(ui, t, &open.to_string(), widgets::Tone::Neutral);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let send = ui
                    .add_enabled_ui(open > 0, |ui| {
                        widgets::icon_btn(
                            ui,
                            t,
                            icons::SEND,
                            "Send to chat: every open comment as one message, asking for a plan that waits for your go-ahead",
                            "",
                        )
                    })
                    .inner;
                let send = widgets::why_disabled(send, "No open comments to send");
                if send.clicked()
                    && let Some(text) = chat_text(&s.name, comments, &s.timing)
                {
                    io.actions.push(Action::ChatInbox(text));
                    state.status = format!("sent {open} comments to chat");
                }
                let add = ui
                    .add_enabled_ui(s.editable, |ui| {
                        widgets::icon_btn(ui, t, icons::PLUS, "Add a marker at the playhead", "M")
                    })
                    .inner;
                let add = widgets::why_disabled(add, LOCKED);
                if add.clicked() {
                    add_at_playhead(state, s, io);
                }
            });
        });
        let resolved = comments.len() - open;
        if resolved > 0 {
            widgets::switch(
                ui,
                t,
                &mut state.show_resolved,
                &format!("Show {resolved} resolved"),
            );
        }
        ui.add_space(theme::SPACE_XS);
        if comments.is_empty() {
            widgets::empty_state(
                ui,
                t,
                icons::MESSAGE_SQUARE,
                "No comments",
                "Press M over the timeline to leave one at the playhead.",
                |_| {},
            );
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for i in by_time(comments) {
                    let c = &comments[i];
                    if c.resolved && !state.show_resolved {
                        continue;
                    }
                    item(state, s, ui, i, c, &mut seek, &mut edits);
                }
            });
    });
    for (label, f) in edits {
        state.apply(label, f);
    }
    if let Some(at) = seek {
        io.seek(s, at);
    }
}

/// The edit that paints marker `i` `color`.
fn recolour(i: usize, color: String) -> Edit {
    (
        format!("marker colour {color}"),
        Box::new(move |p: &mut Plan| {
            p.comments.get_mut(i).is_some_and(|c| {
                c.color = color;
                true
            })
        }),
    )
}

/// The selected row's colour picker: one dot per marker colour, the
/// current one ringed; a click repaints the marker.
fn swatches(ui: &mut Ui, s: &Shown<'_>, i: usize, c: &Comment, edits: &mut Vec<Edit>) {
    let t = s.theme;
    ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
    for name in COMMENT_COLORS {
        let (dot, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        let (fill, active) = super::timeline::marker(name);
        let current = c.color == name;
        let hot = resp.hovered();
        ui.painter()
            .circle_filled(dot.center(), 5.0, if hot { active } else { fill });
        if current {
            ui.painter().circle_stroke(
                dot.center(),
                6.5,
                egui::Stroke::new(1.0, theme::rgb(t.text)),
            );
        }
        let tip = format!("Paint it {name}");
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::RadioButton, true, current, &tip)
        });
        if widgets::tip(resp, t, &tip, "").clicked() && !current {
            edits.push(recolour(i, name.to_owned()));
        }
    }
    ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
}

/// One comment row: its colour, time, shot and author, resolve and delete,
/// then the note; ink 8 % under the pointer, the selected fill when picked.
fn item(
    state: &mut State,
    s: &Shown<'_>,
    ui: &mut Ui,
    i: usize,
    c: &Comment,
    seek: &mut Option<f32>,
    edits: &mut Vec<Edit>,
) {
    let t = s.theme;
    let selected = state.comment_sel == Some(i);
    let id = egui::Id::unique("plan_comment_row").with(i);
    let hovered = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let fill = if selected {
        theme::selected_fill(t)
    } else if hovered {
        theme::ink_hover(t)
    } else {
        Color32::TRANSPARENT
    };
    let frame = egui::Frame::new()
        .fill(fill)
        .corner_radius(CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(theme::margin_xy(theme::SPACE_SM, theme::SPACE_XS))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
                if selected && s.editable {
                    swatches(ui, s, i, c, edits);
                } else {
                    let (dot, resp) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::click());
                    let ink = color(&c.color);
                    let ink = if c.resolved {
                        ink.gamma_multiply(0.35)
                    } else {
                        ink
                    };
                    ui.painter().circle_filled(dot.center(), 5.0, ink);
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            s.editable,
                            "Change the colour",
                        )
                    });
                    if widgets::tip(resp, t, "Change the colour", "").clicked() && s.editable {
                        let next = next_color(&c.color).to_owned();
                        edits.push(recolour(i, next));
                    }
                }
                let at = super::timeline::timecode(c.time, true);
                let time = ui.add(
                    egui::Label::new(
                        egui::RichText::new(&at)
                            .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                            .color(theme::rgb(t.text)),
                    )
                    .sense(Sense::click()),
                );
                if widgets::tip(time, t, "Seek here", "").clicked() {
                    *seek = Some(c.time);
                    state.comment_sel = Some(i);
                }
                let shot = shot_at(&s.timing, c.time)
                    .map_or_else(|| "card".to_owned(), |k| format!("shot {}", k + 1));
                ui.label(
                    egui::RichText::new(format!("{shot} · {}", c.author))
                        .font(theme::font(theme::SMALL_SIZE))
                        .color(theme::rgb(t.text_muted)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let delete = ui
                        .add_enabled_ui(s.editable, |ui| {
                            widgets::icon_toggle_sized(
                                ui,
                                t,
                                icons::TRASH,
                                "Delete",
                                "",
                                false,
                                theme::COMPACT_HEIGHT,
                            )
                        })
                        .inner;
                    let delete = widgets::why_disabled(delete, LOCKED);
                    if delete.clicked() {
                        state.comment_edit = None;
                        state.comment_sel = None;
                        edits.push((
                            "delete comment".to_owned(),
                            Box::new(move |p: &mut Plan| {
                                if i < p.comments.len() {
                                    p.comments.remove(i);
                                    true
                                } else {
                                    false
                                }
                            }),
                        ));
                    }
                    let (glyph, label) = if c.resolved {
                        (icons::ROTATE_CCW, "Reopen")
                    } else {
                        (icons::CHECK, "Resolve")
                    };
                    let resolve = ui
                        .add_enabled_ui(s.editable, |ui| {
                            widgets::icon_toggle_sized(
                                ui,
                                t,
                                glyph,
                                label,
                                "",
                                false,
                                theme::COMPACT_HEIGHT,
                            )
                        })
                        .inner;
                    let resolve = widgets::why_disabled(resolve, LOCKED);
                    if resolve.clicked() {
                        let done = !c.resolved;
                        edits.push((
                            format!("{} comment", if done { "resolve" } else { "reopen" }),
                            Box::new(move |p: &mut Plan| {
                                p.comments.get_mut(i).is_some_and(|c| {
                                    c.resolved = done;
                                    true
                                })
                            }),
                        ));
                    }
                });
            });
            // The note: a label, or a text box while editing.
            let editing = matches!(&state.comment_edit, Some((n, _)) if *n == i);
            if editing && s.editable {
                let focus = state.comment_focus == Some(i);
                let Some((_, buf)) = state.comment_edit.as_mut() else {
                    return;
                };
                let resp = widgets::input(buf)
                    .label("Comment")
                    .placeholder("What should change here")
                    .multiline(true)
                    .min_lines(2)
                    .fill_width(true)
                    .show(ui)
                    .response;
                if focus {
                    resp.request_focus();
                    state.comment_focus = None;
                }
                let (enter, escape) = ui.input(|inp| {
                    (
                        inp.key_pressed(egui::Key::Enter) && !inp.modifiers.shift,
                        inp.key_pressed(egui::Key::Escape),
                    )
                });
                if escape {
                    state.comment_edit = None;
                } else if resp.lost_focus() || (resp.has_focus() && enter) {
                    let note = buf.trim().to_owned();
                    state.comment_edit = None;
                    if note != c.note {
                        edits.push((
                            "comment note".to_owned(),
                            Box::new(move |p: &mut Plan| {
                                p.comments.get_mut(i).is_some_and(|c| {
                                    c.note = note;
                                    true
                                })
                            }),
                        ));
                    }
                }
            } else {
                let (text, ink) = if c.note.is_empty() {
                    ("No note", theme::ink(t, 102))
                } else {
                    (
                        c.note.as_str(),
                        theme::rgb(if c.resolved { t.text_muted } else { t.text }),
                    )
                };
                let mut rich = egui::RichText::new(text)
                    .font(theme::font(theme::BODY_SIZE))
                    .color(ink);
                if c.resolved {
                    rich = rich.strikethrough();
                }
                let resp = ui.add(egui::Label::new(rich).wrap().sense(Sense::click()));
                let resp = widgets::tip(resp, t, "Click to seek, double-click to edit", "");
                if resp.clicked() {
                    *seek = Some(c.time);
                    state.comment_sel = Some(i);
                }
                if resp.double_clicked() && s.editable {
                    state.comment_edit = Some((i, c.note.clone()));
                    state.comment_focus = Some(i);
                }
            }
        });
    let now = frame.response.hovered() || ui.rect_contains_pointer(frame.response.rect);
    if now != hovered {
        ui.data_mut(|d| d.insert_temp(id, now));
        ui.ctx().request_repaint();
    }
}

/// Adds a comment at the playhead (or the selected shot's start when the
/// viewer is not on this plan), opens the list and focuses its text.
pub(super) fn add_at_playhead(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>) {
    let t = playhead(s, io);
    io.select(&s.name, &BTreeSet::new());
    add_at(state, t);
}

/// Adds a comment at `t` seconds, opens the list and focuses its text.
pub(super) fn add_at(state: &mut State, t: f32) {
    let mut at = None;
    if state.apply("add comment", |p| {
        at = Some(add(p, t));
        true
    }) && let Some(i) = at
    {
        state.show_comments = true;
        state.comment_sel = Some(i);
        state.comment_edit = Some((i, String::new()));
        state.comment_focus = Some(i);
    }
}

#[cfg(test)]
mod tests {
    use super::{add, by_time, chat_text, jump_target, next_color, shot_at};
    use crate::plan::{Aspect, Comment, Plan, Rules};

    fn plan() -> Plan {
        Plan {
            name: "reel-30".into(),
            aspect: Aspect::Vertical,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 1,
            card_bar: 2,
            end_bar: 3,
            end_card: String::new(),
            rules: Rules::default(),
            finish: Default::default(),
            shots: Vec::new(),
            comments: Vec::new(),
            accepted: Vec::new(),
        }
    }

    #[test]
    fn comments_round_trip_through_the_plan_file() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("reel-30.toml");
        let mut p = plan();
        let i = add(&mut p, 4.25);
        p.comments[i].note = "too long, cut on the snare".into();
        p.comments[i].color = "yellow".into();
        p.comments.push(Comment {
            time: 1.0,
            note: "ok".into(),
            color: "blue".into(),
            author: "assistant".into(),
            resolved: true,
        });
        p.save(&path)?;
        let text = std::fs::read_to_string(&path)?;
        assert!(
            text.contains("[[comment]]"),
            "stored as [[comment]]: {text}"
        );
        assert_eq!(Plan::load(&path)?, p, "comments survive a save and load");

        // A plan without comments writes no [[comment]] and reads back empty.
        let bare = plan();
        bare.save(&path)?;
        assert!(
            !std::fs::read_to_string(&path)?.contains("comment"),
            "no empty comment table"
        );
        let minimal = "time = 2.0\n";
        let c: Comment = toml::from_str(minimal)?;
        assert_eq!(
            (c.color.as_str(), c.author.as_str(), c.resolved),
            ("green", "you", false),
            "defaults for a hand-written comment"
        );
        Ok(())
    }

    #[test]
    fn marker_tables_read_as_comments_and_survive_edits() -> anyhow::Result<()> {
        let text = "name = \"reel-30\"\naspect = \"9x16\"\nmusic = \"m.wav\"\nstart_bar = 0\n\
                    drop_bar = 1\ncard_bar = 2\nend_bar = 3\nshot = []\n\n\
                    [[marker]]\ntime = 4.0\nnote = \"flash frame\"\ncolor = \"yellow\"\n";
        let mut p: Plan = toml::from_str(text)?;
        assert_eq!(p.comments.len(), 1, "[[marker]] is read as a marker");
        assert_eq!(
            (p.comments[0].note.as_str(), p.comments[0].color.as_str()),
            ("flash frame", "yellow"),
            "with its note and colour"
        );
        // An edit to the shots leaves the markers where they are.
        p.shots.push(crate::plan::Shot {
            clip: "A".into(),
            t_in: 1.0,
            beats: 4,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: String::new(),
        });
        let back: Plan = toml::from_str(&toml::to_string_pretty(&p)?)?;
        assert_eq!(back, p, "saved as [[comment]] and read back the same");
        Ok(())
    }

    #[test]
    fn jumps_go_to_the_next_and_previous_marker() -> anyhow::Result<()> {
        let mut p = plan();
        add(&mut p, 5.0);
        add(&mut p, 1.0);
        let r = add(&mut p, 3.0);
        p.comments[r].resolved = true;
        let next = |t, show| jump_target(&p.comments, t, true, show);
        let prev = |t, show| jump_target(&p.comments, t, false, show);
        assert_eq!(next(0.0, false), Some(1), "first marker after the start");
        assert_eq!(next(1.0, false), Some(0), "the resolved one is skipped");
        assert_eq!(next(1.0, true), Some(2), "unless resolved ones are shown");
        assert_eq!(next(5.0, false), None, "nothing after the last");
        assert_eq!(
            prev(5.0, false),
            Some(1),
            "the one at the playhead is not 'previous'"
        );
        assert_eq!(prev(0.5, false), None, "nothing before the first");
        Ok(())
    }

    #[test]
    fn comments_sort_by_time_and_colours_cycle() -> anyhow::Result<()> {
        let mut p = plan();
        add(&mut p, 5.0);
        add(&mut p, 1.0);
        add(&mut p, 3.0);
        assert_eq!(by_time(&p.comments), vec![1, 2, 0], "time order");
        assert_eq!(next_color("green"), "blue", "cycle");
        assert_eq!(next_color("red"), "green", "wraps");
        Ok(())
    }

    #[test]
    fn the_chat_message_lists_open_comments_with_shot_refs() -> anyhow::Result<()> {
        let timing = [(0.0, 2.0), (2.0, 3.0)];
        assert_eq!(shot_at(&timing, 2.5), Some(1), "second shot");
        assert_eq!(shot_at(&timing, 9.0), None, "past the shots");
        let mut p = plan();
        let a = add(&mut p, 2.5);
        p.comments[a].note = "swap for a fire shot".into();
        let b = add(&mut p, 0.5);
        p.comments[b].note = "  hold longer ".into();
        let c = add(&mut p, 1.0);
        p.comments[c].resolved = true;
        add(&mut p, 6.0);
        let text = chat_text("reel-30", &p.comments, &timing)
            .ok_or_else(|| anyhow::anyhow!("open comments give a message"))?;
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.first().copied(),
            Some("Review comments on plan `reel-30` (3 open):"),
            "header counts open ones: {text}"
        );
        assert_eq!(
            lines.get(1).copied(),
            Some("1. [0:00.5] @shot:reel-30#1: hold longer"),
            "time order, trimmed note, shot reference"
        );
        assert_eq!(
            lines.get(2).copied(),
            Some("2. [0:02.5] @shot:reel-30#2: swap for a fire shot"),
            "resolved one skipped"
        );
        assert!(
            lines
                .get(3)
                .is_some_and(|l| l.starts_with("3. [0:06.0] @plan:reel-30 (end card)")),
            "past the shots points at the plan: {text}"
        );
        assert!(
            text.contains("Do not edit the plan until I approve it."),
            "asks for a plan and to wait"
        );
        p.comments.iter_mut().for_each(|c| c.resolved = true);
        assert!(
            chat_text("reel-30", &p.comments, &timing).is_none(),
            "nothing open, nothing to send"
        );
        Ok(())
    }
}

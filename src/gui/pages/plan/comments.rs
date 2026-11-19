// ============================================================================
// Module:       gui::pages::plan::comments
// Description:  Review comments on the open plan: the list beside the tracks
//               (seek, edit, resolve, delete), adding one at the playhead,
//               and the batch message that hands them to the chat.
//
// Dependencies: egui; crate::{plan, refs}, crate::gui::pages::plan
// ============================================================================

//! Comments, the way Selects reviews a cut.
//!
//! A comment is a `[[comment]]` in the plan's TOML ([`crate::plan::Comment`]):
//! a time on the cut, a note, a colour, who left it and whether it is dealt
//! with. `F` drops one at the playhead and focuses its text; the ruler shows
//! each as a coloured pin; this list seeks on click, edits on double-click,
//! resolves and deletes. **Send to chat** writes every open comment as one
//! message, each with its timecode and the shot under it as an
//! `@shot:<plan>#<n>` reference, and asks the assistant for a plan covering
//! all of them that waits for the go-ahead, so the review turns into one
//! proposal rather than a string of edits.

use egui::{Color32, CornerRadius, FontId, Rect, Sense, Stroke, Ui, UiBuilder, vec2};

use super::{Io, Shown, State};
use crate::gui::app::Action;
use crate::gui::ui::theme;
use crate::plan::{COMMENT_COLORS, Comment, Plan};
use crate::refs::{Reference, Target};

/// A labelled change to the plan, applied after the list is drawn.
type Edit = (String, Box<dyn FnOnce(&mut Plan) -> bool>);

/// The marker colour for a comment colour name.
pub(super) fn color(name: &str) -> Color32 {
    match name {
        "blue" => Color32::from_rgb(84, 152, 240),
        "yellow" => Color32::from_rgb(236, 196, 64),
        "red" => Color32::from_rgb(232, 84, 84),
        _ => Color32::from_rgb(92, 192, 112),
    }
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
    ui.painter().rect(
        rect,
        CornerRadius::same(3),
        theme::rgb(t.panel),
        Stroke::new(1.0, theme::rgb(t.border)),
        egui::StrokeKind::Inside,
    );
    let comments = &s.plan.comments;
    let open = comments.iter().filter(|c| !c.resolved).count();
    let mut seek = None;
    let mut edits: Vec<Edit> = Vec::new();
    ui.scope_builder(UiBuilder::new().max_rect(rect.shrink(6.0)), |ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("COMMENTS {open}"))
                    .size(10.5)
                    .strong()
                    .color(theme::rgb(t.text_muted)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let send = ui
                    .add_enabled(open > 0, egui::Button::new("Send to chat").small())
                    .on_hover_text(
                        "Hand every open comment to the assistant as one message, \
                         asking for a plan that covers them and waits for your go-ahead",
                    );
                if send.clicked()
                    && let Some(text) = chat_text(&s.name, comments, &s.timing)
                {
                    io.actions.push(Action::ChatInbox(text));
                    state.status = format!("sent {open} comments to chat");
                }
                if ui
                    .add_enabled(s.editable, egui::Button::new("Add  F").small())
                    .on_hover_text("A comment at the playhead")
                    .clicked()
                {
                    add_at_playhead(state, s, io);
                }
            });
        });
        let resolved = comments.len() - open;
        if resolved > 0 {
            ui.checkbox(
                &mut state.show_resolved,
                egui::RichText::new(format!("Show {resolved} resolved")).size(11.0),
            );
        }
        ui.separator();
        if comments.is_empty() {
            ui.label(
                egui::RichText::new(
                    "No comments. Press F over the timeline to leave one at the playhead.",
                )
                .size(11.0)
                .color(theme::rgb(t.text_faint)),
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

/// One comment row.
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
    egui::Frame::new()
        .fill(theme::rgb(if selected { t.raised } else { t.panel }))
        .stroke(Stroke::new(
            1.0,
            theme::rgb(if selected { t.border_strong } else { t.border }),
        ))
        .corner_radius(CornerRadius::same(3))
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (dot, resp) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::click());
                let ink = color(&c.color);
                let ink = if c.resolved {
                    ink.gamma_multiply(0.35)
                } else {
                    ink
                };
                ui.painter().circle_filled(dot.center(), 4.5, ink);
                if resp.on_hover_text("Change the colour").clicked() && s.editable {
                    let next = next_color(&c.color).to_owned();
                    edits.push((
                        format!("comment colour {next}"),
                        Box::new(move |p: &mut Plan| {
                            p.comments.get_mut(i).is_some_and(|c| {
                                c.color = next;
                                true
                            })
                        }),
                    ));
                }
                let at = super::timeline::timecode(c.time, true);
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(&at)
                                .monospace()
                                .size(10.5)
                                .color(theme::rgb(t.accent)),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Seek here")
                    .clicked()
                {
                    *seek = Some(c.time);
                    state.comment_sel = Some(i);
                }
                let shot = shot_at(&s.timing, c.time)
                    .map_or_else(|| "card".to_owned(), |k| format!("#{}", k + 1));
                ui.label(
                    egui::RichText::new(format!("{shot} · {}", c.author))
                        .size(10.0)
                        .color(theme::rgb(t.text_faint)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(s.editable, egui::Button::new("Delete").small())
                        .clicked()
                    {
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
                    let label = if c.resolved { "Reopen" } else { "Resolve" };
                    if ui
                        .add_enabled(s.editable, egui::Button::new(label).small())
                        .clicked()
                    {
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
                let resp = ui.add(
                    egui::TextEdit::multiline(buf)
                        .hint_text("what should change here")
                        .font(FontId::proportional(11.5))
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
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
                    ("(no note)", t.text_faint)
                } else {
                    (
                        c.note.as_str(),
                        if c.resolved { t.text_muted } else { t.text },
                    )
                };
                let mut rich = egui::RichText::new(text).size(11.5).color(theme::rgb(ink));
                if c.resolved {
                    rich = rich.strikethrough();
                }
                let resp = ui
                    .add(egui::Label::new(rich).wrap().sense(Sense::click()))
                    .on_hover_text("Click to seek · double-click to edit");
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
}

/// Adds a comment at the playhead (or the selected shot's start when the
/// viewer is not on this plan), opens the list and focuses its text.
pub(super) fn add_at_playhead(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>) {
    let t = if s.ours {
        io.player.position() as f32
    } else {
        s.single()
            .and_then(|k| s.timing.get(k))
            .map_or(0.0, |&(a, _)| a)
    }
    .clamp(0.0, s.total);
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
    use super::{add, by_time, chat_text, next_color, shot_at};
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

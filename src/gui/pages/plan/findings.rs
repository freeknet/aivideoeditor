// ============================================================================
// Module:       gui::pages::plan::findings
// Description:  The check's findings list: every finding with what it means
//               and what can be done about it there and then (Fix, Ask chat,
//               Accept), plus Fix all.
//
// Dependencies: egui; crate::{fix, plan}, crate::gui::pages::plan,
//               crate::gui::ui::{icons, theme, widgets}
// ============================================================================

//! Findings that can be solved from where they are shown.
//!
//! The check bar's "Findings" button (or a click on its soft or hard count)
//! opens a popover listing every finding, hard first. Each row names where
//! the finding is and what is wrong; a hard one also says, in plain words,
//! what it means and what to do ([`crate::fix::advice`]). Under it:
//!
//! - **Fix** applies the best automatic fix ([`crate::fix::fixes`]): it
//!   clears the finding, adds no hard one, and its tooltip says what it
//!   does. It is one undoable edit, saved as a new version like any other.
//!   Fixes are worked out one finding a frame while the list is open, so a
//!   long list never stalls the window.
//! - **Ask chat** puts a prompt naming the shots (`@shot:<plan>#N`) and the
//!   finding in the chat composer, so the assistant proposes a fix card
//!   scoped to those shots. Nothing is sent.
//! - **Accept** (soft findings) records the finding in the plan's
//!   `accepted` list: it stops counting and shows dimmed with an Undo. It
//!   comes back when the shots it is about change.
//!
//! **Fix all** applies every automatic fix in turn, re-checking after each
//! ([`crate::fix::fix_all`]), as one edit.

use std::collections::BTreeSet;

use egui::{Popup, PopupCloseBehavior, Response, Ui};

use super::{Io, Shown, State};
use crate::fix::Fix;
use crate::gui::app::Action;
use crate::gui::ui::{icons, theme, widgets};
use crate::plan::{Finding, Memory};

/// Width of the popover.
const WIDTH: f32 = 520.0;
/// Longest finding line before it is cut short.
const LINE_CHARS: usize = 64;

/// The automatic fixes for the shown plan's findings.
#[derive(Debug, Default)]
pub(super) struct Fixes {
    /// Plan revision they were worked out for.
    rev: u64,
    /// The best fix of each finding, in the check's order: `None` while not
    /// worked out yet, empty when there is none.
    each: Vec<Option<Vec<Fix>>>,
    /// Other nights' plans, read once per revision.
    memory: Option<Memory>,
}

/// What the person asked for in the list.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Act {
    /// Select the finding's shot (or the shots around its cut).
    Select(Finding),
    /// Apply the best fix of finding `n` (index into the check's findings).
    Fix(usize),
    /// Hand finding `n` to the chat.
    Ask(usize),
    /// Accept finding `n` (`true`) or take the acceptance back.
    Accept(usize, bool),
    /// Apply every automatic fix.
    FixAll,
}

/// The fix state for this revision, reset when the plan changed.
fn fixes_for<'a>(state: &'a mut State, s: &Shown<'_>) -> &'a mut Fixes {
    let n = state.problems.len();
    let fresh = state
        .fixes
        .as_ref()
        .is_none_or(|f| f.rev != s.rev || f.each.len() != n);
    if fresh {
        state.fixes = Some(Fixes {
            rev: s.rev,
            each: vec![None; n],
            memory: None,
        });
    }
    state.fixes.get_or_insert_with(Fixes::default)
}

/// Works out the fix of the next finding that has none yet; whether any
/// are still to do.
fn work_one(state: &mut State, s: &Shown<'_>) -> bool {
    let Some(index) = s.project.index.as_ref() else {
        return false;
    };
    if !s.editable {
        return false;
    }
    let problems = state.problems.clone();
    let cache = fixes_for(state, s);
    let Some(n) = cache.each.iter().position(Option::is_none) else {
        return false;
    };
    let memory = cache
        .memory
        .get_or_insert_with(|| Memory::for_index(index))
        .clone();
    let night = crate::fix::Night {
        index,
        map: s.map,
        people: &s.project.people,
        memory: &memory,
    };
    let found = match problems.get(n) {
        Some(f) if !s.plan.is_accepted(f) => crate::fix::fixes(&s.plan, f, night, 1),
        Some(_) | None => Vec::new(),
    };
    if let Some(slot) = cache.each.get_mut(n) {
        *slot = Some(found);
    }
    cache.each.iter().any(Option::is_none)
}

/// `text` cut to about `n` characters with an ellipsis.
fn short(text: &str, n: usize) -> String {
    if text.chars().count() <= n {
        return text.to_owned();
    }
    let cut: String = text.chars().take(n).collect();
    format!("{}\u{2026}", cut.trim_end())
}

/// The popover under `anchor` (the Findings button); `toggle` opens or
/// closes it from elsewhere (a click on a count). Returns what was asked.
pub(super) fn popover(
    state: &mut State,
    s: &Shown<'_>,
    anchor: &Response,
    toggle: bool,
) -> Option<Act> {
    let id = Popup::default_response_id(anchor);
    if toggle {
        Popup::toggle_id(&anchor.ctx, id);
    }
    if Popup::is_id_open(&anchor.ctx, id) && work_one(state, s) {
        anchor.ctx.request_repaint();
    }
    let problems = state.problems.clone();
    let mut order: Vec<usize> = (0..problems.len()).collect();
    order.sort_by_key(|&n| problems.get(n).map(|f| f.severity));
    let each: Vec<Option<Vec<Fix>>> = state
        .fixes
        .as_ref()
        .filter(|f| f.rev == s.rev && f.each.len() == problems.len())
        .map(|f| f.each.clone())
        .unwrap_or_else(|| vec![None; problems.len()]);
    let mut act = None;
    // The response is not needed: drawing it is the point.
    let _ = Popup::menu(anchor)
        .id(id)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(WIDTH)
        .gap(theme::SPACE_XS)
        .show(|ui| {
            act = list(ui, s, &problems, &order, &each);
            if matches!(act, Some(Act::Select(_))) {
                ui.close();
            }
        });
    act
}

/// The list itself: a heading with Fix all, then one block per finding.
fn list(
    ui: &mut Ui,
    s: &Shown<'_>,
    problems: &[Finding],
    order: &[usize],
    each: &[Option<Vec<Fix>>],
) -> Option<Act> {
    let t = s.theme;
    let mut act = None;
    let open = problems.iter().filter(|f| !s.plan.is_accepted(f)).count();
    let fixable = order.iter().any(|&n| {
        problems.get(n).is_some_and(|f| !s.plan.is_accepted(f))
            && each
                .get(n)
                .is_some_and(|e| e.as_ref().is_none_or(|v| !v.is_empty()))
    });
    ui.horizontal(|ui| {
        ui.set_min_height(theme::CONTROL_HEIGHT);
        widgets::menu_heading(
            ui,
            t,
            &match open {
                1 => "1 finding to look at".to_owned(),
                n => format!("{n} findings to look at"),
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let all = ui
                .add_enabled_ui(s.editable && fixable, |ui| {
                    widgets::button_with(
                        ui,
                        t,
                        widgets::ButtonKind::Outline,
                        Some(icons::WAND),
                        "Fix all",
                    )
                })
                .inner;
            let tip = if s.editable {
                "Apply every automatic fix, one at a time, re-checking after each (one undo)"
            } else {
                "Fixes change the cut being edited, not an old version or a locked cut"
            };
            if widgets::tip(all, t, tip, "")
                .on_disabled_hover_text(tip)
                .clicked()
            {
                act = Some(Act::FixAll);
            }
        });
    });
    widgets::menu_separator(ui, t);
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (row, &n) in order.iter().enumerate() {
                let Some(f) = problems.get(n) else {
                    continue;
                };
                if row > 0 {
                    widgets::menu_separator(ui, t);
                }
                if let Some(a) = item(ui, s, f, n, each.get(n).cloned().flatten()) {
                    act = Some(a);
                }
            }
        });
    act
}

/// One finding: its line, what it means (hard ones), and its actions.
fn item(ui: &mut Ui, s: &Shown<'_>, f: &Finding, n: usize, fix: Option<Vec<Fix>>) -> Option<Act> {
    let t = s.theme;
    let accepted = s.plan.is_accepted(f);
    let mut act = None;
    let glyph = if f.is_hard() {
        icons::CIRCLE_X
    } else {
        icons::TRIANGLE_ALERT
    };
    let line = format!("{} \u{b7} {}", f.place(), f.message);
    let resp = ui
        .scope(|ui| {
            if accepted {
                ui.multiply_opacity(0.5);
            }
            widgets::menu_item(ui, t, Some(glyph), &short(&line, LINE_CHARS), "", false)
        })
        .inner;
    let what = if f.seam.is_some() {
        "Select the shots around this cut"
    } else if f.shot.is_some() {
        "Select this shot"
    } else {
        "The whole cut"
    };
    let tip = format!(
        "{line}\n\n{} ({}): {}",
        what,
        f.id,
        crate::fix::advice(f.id)
    );
    if widgets::tip(resp, t, &tip, "").clicked() {
        act = Some(Act::Select(f.clone()));
    }
    let indent = theme::SPACE_SM + icons::SMALL + theme::SPACE_SM;
    if f.is_hard() {
        ui.horizontal(|ui| {
            ui.add_space(indent);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(crate::fix::advice(f.id))
                        .font(theme::font(theme::BODY_SIZE))
                        .color(theme::rgb(t.text_muted)),
                )
                .wrap(),
            );
        });
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
        ui.add_space(indent - theme::SPACE_XS);
        if accepted {
            widgets::pill(ui, t, "accepted", widgets::Tone::Neutral);
            let undo = widgets::button_with(
                ui,
                t,
                widgets::ButtonKind::Ghost,
                Some(icons::UNDO),
                "Undo",
            );
            if widgets::tip(undo, t, "Count this finding again", "").clicked() {
                act = Some(Act::Accept(n, false));
            }
            return;
        }
        let best = fix.as_ref().and_then(|v| v.first());
        let (enabled, tip) = match (&fix, best) {
            _ if !s.editable => (
                false,
                "Fixes change the cut being edited, not an old version or a locked cut".to_owned(),
            ),
            (_, Some(b)) => (true, b.preview()),
            (None, _) => (false, "Working out a fix\u{2026}".to_owned()),
            (Some(_), None) if crate::fix::fixable(f.id) => (
                false,
                "No automatic fix keeps the rules here: ask the chat, or edit it by hand".to_owned(),
            ),
            (Some(_), None) => (
                false,
                format!("No automatic fix: {}", crate::fix::advice(f.id)),
            ),
        };
        let button = ui
            .add_enabled_ui(enabled, |ui| {
                widgets::button_with(
                    ui,
                    t,
                    widgets::ButtonKind::Outline,
                    Some(icons::WAND),
                    "Fix",
                )
            })
            .inner;
        if widgets::tip(button, t, &tip, "")
            .on_disabled_hover_text(&tip)
            .clicked()
        {
            act = Some(Act::Fix(n));
        }
        let ask = widgets::button_with(
            ui,
            t,
            widgets::ButtonKind::Ghost,
            Some(icons::MESSAGE_SQUARE),
            "Ask chat",
        );
        if widgets::tip(
            ask,
            t,
            "Put a prompt about this finding in the chat, scoped to its shots (not sent)",
            "",
        )
        .clicked()
        {
            act = Some(Act::Ask(n));
        }
        if !f.is_hard() {
            let accept = ui
                .add_enabled_ui(s.editable, |ui| {
                    widgets::button_with(
                        ui,
                        t,
                        widgets::ButtonKind::Ghost,
                        Some(icons::CHECK),
                        "Accept",
                    )
                })
                .inner;
            if widgets::tip(
                accept,
                t,
                "Keep it as it is: it stops counting for this cut, and comes back if these shots change",
                "",
            )
            .clicked()
            {
                act = Some(Act::Accept(n, true));
            }
        }
    });
    act
}

/// The shots (1-based) a finding is about.
fn shots_of(f: &Finding) -> Vec<usize> {
    match (f.seam, f.shot) {
        (Some(n), _) => vec![n, n + 1],
        (None, Some(n)) => vec![n],
        (None, None) => Vec::new(),
    }
}

/// The chat prompt for finding `f` of plan `plan`: where it is, the shots
/// as references (so the chat's scope follows them), what is wrong, and a
/// request for a fix card.
pub(super) fn prompt(plan: &str, f: &Finding) -> String {
    let refs: Vec<String> = shots_of(f)
        .into_iter()
        .map(|n| format!("@shot:{plan}#{n}"))
        .collect();
    // References stand alone between spaces, or the composer cannot read them.
    let at = match refs.as_slice() {
        [a, b] => format!("In {plan}, the cut between {a} and {b}"),
        [a] => format!("In {plan}, {a}"),
        _ => format!("The {plan} cut"),
    };
    format!(
        "{at} has a {} {} finding: {}. Propose a fix as an edit I can apply, changing only {}.",
        f.severity.as_str(),
        f.id,
        f.message,
        if refs.is_empty() {
            "what it needs"
        } else {
            "these shots"
        }
    )
}

/// Does what the list asked.
pub(super) fn act(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, act: Act) {
    let problems = state.problems.clone();
    match act {
        Act::Select(f) => super::strips::show_finding(state, s, io, &f),
        Act::Fix(n) => {
            let (Some(f), Some(fix)) = (
                problems.get(n),
                state
                    .fixes
                    .as_ref()
                    .filter(|x| x.rev == s.rev)
                    .and_then(|x| x.each.get(n).cloned().flatten())
                    .and_then(|v| v.into_iter().next()),
            ) else {
                return;
            };
            let plan = fix.plan.clone();
            if state.apply(format!("fix {} at {}", f.id, f.place()), |p| {
                *p = plan;
                true
            }) {
                state.status = format!("fixed: {}", fix.label);
                let shots: BTreeSet<usize> = shots_of(f)
                    .into_iter()
                    .filter_map(|k| k.checked_sub(1))
                    .filter(|&k| k < s.plan.shots.len())
                    .collect();
                io.select(&s.name, &shots);
            }
        }
        Act::Ask(n) => {
            let Some(f) = problems.get(n) else {
                return;
            };
            io.actions.push(Action::ChatInbox(prompt(&s.name, f)));
            super::strips::show_finding(state, s, io, f);
            state.status = format!("asked the chat about {}", f.place());
        }
        Act::Accept(n, on) => {
            let Some(f) = problems.get(n) else {
                return;
            };
            let verb = if on { "accept" } else { "unaccept" };
            state.apply(format!("{verb} {} at {}", f.id, f.place()), |p| {
                p.set_accepted(f, on)
            });
        }
        Act::FixAll => {
            let Some(index) = s.project.index.as_ref() else {
                return;
            };
            let memory = Memory::for_index(index);
            let night = crate::fix::Night {
                index,
                map: s.map,
                people: &s.project.people,
                memory: &memory,
            };
            let (fixed, done) = crate::fix::fix_all(&s.plan, night);
            if done.is_empty() {
                state.status = "nothing to fix automatically".to_owned();
                return;
            }
            let label = match done.len() {
                1 => "fix 1 finding".to_owned(),
                k => format!("fix {k} findings"),
            };
            if state.apply(label.clone(), |p| {
                *p = fixed;
                true
            }) {
                state.status = format!(
                    "{label}: {}",
                    done.iter()
                        .map(|a| a.fix.label.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::prompt;
    use crate::plan::{Finding, Severity};

    #[test]
    fn the_chat_prompt_names_the_shots_and_the_finding() -> anyhow::Result<()> {
        let f = Finding {
            id: "colour-jump",
            severity: Severity::Soft,
            shot: Some(4),
            seam: Some(3),
            message: "mid-tone colour moves 0.43 across the cut".into(),
        };
        let text = prompt("reel-15", &f);
        assert!(
            text.starts_with("In reel-15, the cut between @shot:reel-15#3 and @shot:reel-15#4 has a soft colour-jump finding: mid-tone colour moves 0.43")
                && text.ends_with("changing only these shots."),
            "{text}"
        );
        let refs = crate::refs::Reference::scan(&text);
        assert_eq!(refs.len(), 2, "both shots become references: {refs:?}");
        let plan = Finding {
            id: "off-drop",
            severity: Severity::Hard,
            shot: None,
            seam: None,
            message: "no shot starts exactly on the drop (bar 9)".into(),
        };
        assert!(
            prompt("reel-15", &plan).starts_with("The reel-15 cut has a hard off-drop finding"),
            "a plan finding names no shot"
        );
        Ok(())
    }
}

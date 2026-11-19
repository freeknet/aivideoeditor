// ============================================================================
// Module:       gui::pages::plan::tabs
// Description:  Plans as timeline tabs: one per plan file, with an unsaved
//               dot and a failed-check count, close / reopen, rename,
//               duplicate as a new plan, and "+" to draft one.
//
// Dependencies: egui; crate::{plan, project, refs}, crate::gui::pages::plan
// ============================================================================

//! The tab strip above the tracks.
//!
//! Selects keeps every timeline ("draft") as a tab and lands an agent's edit
//! as a new tab, so before/after is switching tabs. Here a tab is a plan
//! file, `plans/<name>.toml`: the strip lists `project.plans` (re-read once
//! a second, so a plan the assistant or `ave plan` writes appears by
//! itself), minus the ones the user closed (kept in the night's settings
//! as `pages.plan.hidden`; closing hides, it never deletes).
//!
//! A tab carries a dot when the plan has changes the user has not seen:
//! an unsaved edit on the open tab, or a file that changed on disk since
//! the tab was last open. It carries a cross and a count when `plan::check`
//! fails on it, computed in the background only when the file changes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result};
use egui::{Align2, CornerRadius, FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

use super::{State, history};
use crate::gui::app::Action;
use crate::gui::player::Player;
use crate::gui::ui::theme;
use crate::plan::Plan;
use crate::project::Project;
use crate::refs::{Reference, Target};
use crate::theme::Palette;

/// What a tab shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Tab {
    /// The plan's name (file stem).
    pub(super) name: String,
    /// The open plan.
    pub(super) current: bool,
    /// Unsaved, or changed since last seen.
    pub(super) dirty: bool,
    /// Problems `plan::check` finds (0 when it passes).
    pub(super) errors: usize,
}

/// What is known about a plan file between frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Info {
    /// The file's mtime when last looked at.
    pub(super) stamp: Option<SystemTime>,
    /// Its mtime when the user last had it open.
    pub(super) seen: Option<SystemTime>,
    /// Problems the check found at `stamp`.
    pub(super) errors: usize,
}

/// The rename / duplicate form.
#[derive(Debug)]
pub(super) struct Form {
    /// Renaming, or duplicating.
    kind: FormKind,
    /// The new name.
    name: String,
    /// Why the last attempt was refused.
    error: Option<String>,
}

/// What the form does.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FormKind {
    /// Renames the plan.
    Rename(String),
    /// Copies the plan under a new name.
    Duplicate(String),
}

/// The tabs to draw: every plan not closed, plus the open one always.
pub(super) fn model(
    stems: &[String],
    hidden: &BTreeSet<String>,
    current: Option<&str>,
    current_dirty: bool,
    current_errors: usize,
    info: &BTreeMap<String, Info>,
) -> Vec<Tab> {
    stems
        .iter()
        .filter(|s| Some(s.as_str()) == current || !hidden.contains(*s))
        .map(|s| {
            let is_current = Some(s.as_str()) == current;
            let known = info.get(s);
            Tab {
                name: s.clone(),
                current: is_current,
                dirty: if is_current {
                    current_dirty
                } else {
                    known.is_some_and(|i| i.stamp.is_some() && i.stamp != i.seen)
                },
                errors: if is_current {
                    current_errors
                } else {
                    known.map_or(0, |i| i.errors)
                },
            }
        })
        .collect()
}

/// `<base>-copy`, or `<base>-copy-2`, … the first not in `taken`.
pub(super) fn free_name(base: &str, taken: &[String]) -> String {
    let first = format!("{base}-copy");
    if !taken.contains(&first) {
        return first;
    }
    (2..)
        .map(|n| format!("{base}-copy-{n}"))
        .find(|n| !taken.contains(n))
        .unwrap_or(first)
}

/// The plans of the night, by file stem.
pub(super) fn stems(project: &Project) -> Vec<String> {
    project
        .plans
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect()
}

/// The closed tabs, from the night's settings.
pub(super) fn hidden(project: &Project) -> BTreeSet<String> {
    project
        .settings
        .pages
        .get(super::PAGE_KEY)
        .and_then(|t| t.get("hidden"))
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Stores the closed tabs.
fn set_hidden(project: &mut Project, hidden: &BTreeSet<String>) {
    let table = project
        .settings
        .pages
        .entry(super::PAGE_KEY.to_owned())
        .or_default();
    table.insert(
        "hidden".to_owned(),
        toml::Value::Array(
            hidden
                .iter()
                .map(|h| toml::Value::String(h.clone()))
                .collect(),
        ),
    );
    let _ = project.save_settings();
}

/// Renames plan `old` to `new`: the live file, its history, and the plan's
/// own `name`. Refuses to overwrite.
pub(super) fn rename_plan(work: &Path, old: &str, new: &str) -> Result<()> {
    let from = history::live_path(work, old);
    let to = history::live_path(work, new);
    anyhow::ensure!(!to.exists(), "a plan called {new} exists");
    let mut plan = Plan::load(&from)?;
    std::fs::rename(&from, &to).with_context(|| format!("renaming {}", from.display()))?;
    let (hist_from, hist_to) = (history::dir(work, old), history::dir(work, new));
    if hist_from.is_dir() && !hist_to.exists() {
        std::fs::rename(&hist_from, &hist_to)
            .with_context(|| format!("moving {}", hist_from.display()))?;
    }
    if plan.name != new {
        plan.name = new.to_owned();
        history::save(
            work,
            new,
            &plan,
            &format!("renamed from {old}"),
            history::now(),
        )?;
    }
    Ok(())
}

/// Copies plan `src` (as `plan`) to a new plan `new`, with its own history
/// starting at "duplicated from".
pub(super) fn duplicate_plan(work: &Path, src: &str, plan: &Plan, new: &str) -> Result<()> {
    anyhow::ensure!(
        !history::live_path(work, new).exists(),
        "a plan called {new} exists"
    );
    let mut copy = plan.clone();
    copy.name = new.to_owned();
    history::save(
        work,
        new,
        &copy,
        &format!("duplicated from {src}"),
        history::now(),
    )?;
    Ok(())
}

/// Re-reads the plan list, and each changed file's check count.
pub(super) fn refresh(state: &mut State, project: &mut Project) {
    project.reload_plans();
    let names = stems(project);
    let first = !state.tabs_scanned;
    state.tabs_scanned = true;
    state.tab_info.retain(|k, _| names.contains(k));
    for name in &names {
        let stamp = super::mtime(&history::live_path(&project.work, name));
        let info = state.tab_info.entry(name.clone()).or_insert_with(|| Info {
            stamp: None,
            seen: if first { stamp } else { None },
            errors: 0,
        });
        if info.stamp != stamp {
            info.stamp = stamp;
            info.errors = match (
                Plan::load(&history::live_path(&project.work, name)),
                project.index.as_ref(),
                project.music.as_ref(),
            ) {
                (Ok(plan), Some(index), Some(map)) => {
                    crate::plan::check(&plan, index, map, &project.people).len()
                }
                (Ok(_), _, _) => 0,
                (Err(_), _, _) => 1,
            };
        }
        if state.name.as_deref() == Some(name.as_str()) {
            info.seen = stamp;
        }
    }
}

/// Opens `name` and the viewer on it.
fn switch(state: &mut State, project: &mut Project, player: &mut Player, name: &str) {
    if state.name.as_deref() != Some(name) {
        state.open(project, name);
        super::open_in_player(state, project, player);
    }
}

/// The tab strip.
pub(super) fn strip(
    state: &mut State,
    project: &mut Project,
    player: &mut Player,
    palette: &Palette,
    actions: &mut Vec<Action>,
    ui: &mut Ui,
) {
    let t = palette;
    let names = stems(project);
    let mut closed = hidden(project);
    let current_errors = if state.viewing.is_none() {
        state.problems.len()
    } else {
        0
    };
    let tabs = model(
        &names,
        &closed,
        state.name.as_deref(),
        state.dirty.is_some(),
        current_errors,
        &state.tab_info,
    );
    let mut open = None;
    let mut close = Vec::new();
    let mut close_others = None;
    let mut reopen = None;
    let h = 24.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.label(
            egui::RichText::new("TIMELINE")
                .size(10.5)
                .strong()
                .color(theme::rgb(t.text_muted)),
        );
        ui.add_space(6.0);
        for tab in &tabs {
            let font = FontId::proportional(11.5);
            let galley = ui.painter().layout_no_wrap(
                tab.name.clone(),
                font.clone(),
                theme::rgb(if tab.current { t.text } else { t.text_muted }),
            );
            let count = (tab.errors > 0).then(|| tab.errors.to_string());
            let count_w = count.as_ref().map_or(0.0, |c| 14.0 + 6.5 * c.len() as f32);
            let w = 10.0 + galley.size().x + 10.0 + count_w + 16.0;
            let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
            let close_rect = Rect::from_center_size(
                pos2(rect.right() - 10.0, rect.center().y),
                vec2(14.0, 14.0),
            );
            let close_resp = ui
                .interact(
                    close_rect,
                    egui::Id::unique("plan_tab_close").with(&tab.name),
                    Sense::click(),
                )
                .on_hover_text("Close the tab (the plan file stays)");
            let hovered = resp.hovered() || close_resp.hovered();
            let p = ui.painter();
            let fill = if tab.current {
                theme::rgb(t.raised)
            } else if hovered {
                theme::rgb(t.hover)
            } else {
                theme::rgb(t.panel)
            };
            p.rect(
                rect,
                CornerRadius {
                    nw: 4,
                    ne: 4,
                    sw: 0,
                    se: 0,
                },
                fill,
                Stroke::new(
                    1.0,
                    theme::rgb(if tab.current {
                        t.border_strong
                    } else {
                        t.border
                    }),
                ),
                egui::StrokeKind::Inside,
            );
            if tab.current {
                p.hline(
                    rect.x_range(),
                    rect.top() + 1.0,
                    Stroke::new(2.0, theme::rgb(t.accent)),
                );
            }
            let mut x = rect.left() + 10.0;
            p.galley(
                pos2(x, rect.center().y - galley.size().y / 2.0),
                galley.clone(),
                theme::rgb(t.text),
            );
            x += galley.size().x + 5.0;
            if tab.dirty {
                p.circle_filled(pos2(x + 2.0, rect.center().y), 3.0, theme::rgb(t.warning));
            }
            x += 8.0;
            if let Some(c) = &count {
                let red = theme::rgb(t.danger);
                let cx = pos2(x + 4.0, rect.center().y);
                let st = Stroke::new(1.5, red);
                p.line_segment([cx + vec2(-3.0, -3.0), cx + vec2(3.0, 3.0)], st);
                p.line_segment([cx + vec2(-3.0, 3.0), cx + vec2(3.0, -3.0)], st);
                p.text(
                    pos2(x + 10.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    c,
                    FontId::proportional(10.5),
                    red,
                );
            }
            if hovered || tab.current {
                let ink = theme::rgb(if close_resp.hovered() {
                    t.text
                } else {
                    t.text_faint
                });
                let c = close_rect.center();
                let st = Stroke::new(1.2, ink);
                p.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], st);
                p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], st);
            }
            let tip = format!(
                "plans/{}.toml{}{}",
                tab.name,
                if tab.dirty {
                    if tab.current {
                        " · unsaved changes"
                    } else {
                        " · changed since you last opened it"
                    }
                } else {
                    ""
                },
                if tab.errors > 0 {
                    format!(" · check fails ({} problems)", tab.errors)
                } else {
                    String::new()
                }
            );
            let resp = resp.on_hover_text(tip);
            if close_resp.clicked() || resp.middle_clicked() {
                close.push(tab.name.clone());
            } else if resp.clicked() {
                open = Some(tab.name.clone());
            }
            resp.context_menu(|ui| {
                ui.set_min_width(180.0);
                if ui.button("Rename…").clicked() {
                    state.tab_form = Some(Form {
                        kind: FormKind::Rename(tab.name.clone()),
                        name: tab.name.clone(),
                        error: None,
                    });
                    ui.close();
                }
                if ui.button("Duplicate as new plan…").clicked() {
                    state.tab_form = Some(Form {
                        kind: FormKind::Duplicate(tab.name.clone()),
                        name: free_name(&tab.name, &names),
                        error: None,
                    });
                    ui.close();
                }
                if ui.button("Reference in chat").clicked() {
                    actions.push(Action::AddRef(Reference::new(Target::Plan {
                        name: tab.name.clone(),
                    })));
                    ui.close();
                }
                ui.separator();
                if ui.button("Close tab").clicked() {
                    close.push(tab.name.clone());
                    ui.close();
                }
                if ui
                    .add_enabled(tabs.len() > 1, egui::Button::new("Close other tabs"))
                    .clicked()
                {
                    close_others = Some(tab.name.clone());
                    ui.close();
                }
            });
        }
        ui.add_space(4.0);
        let busy = state.pending_job.is_some();
        if ui
            .add_enabled(!busy, egui::Button::new("+").small())
            .on_hover_text("Draft a new plan on the beat grid from the shot log")
            .clicked()
        {
            super::open_draft_form(state, project);
        }
        if let Some((_, name)) = &state.pending_job {
            ui.spinner();
            ui.label(
                egui::RichText::new(format!("drafting {name}…"))
                    .size(11.0)
                    .color(theme::rgb(t.text_muted)),
            );
        }
        let reopenable: Vec<&String> = names
            .iter()
            .filter(|n| closed.contains(*n) && Some(n.as_str()) != state.name.as_deref())
            .collect();
        if !reopenable.is_empty() {
            ui.menu_button(
                egui::RichText::new(format!("{} closed", reopenable.len()))
                    .size(11.0)
                    .color(theme::rgb(t.text_muted)),
                |ui| {
                    for n in reopenable {
                        if ui.button(n).clicked() {
                            reopen = Some(n.clone());
                            ui.close();
                        }
                    }
                },
            )
            .response
            .on_hover_text("Closed tabs: pick one to reopen it");
        }
    });

    if let Some(keep) = close_others {
        closed.extend(names.iter().filter(|n| **n != keep).cloned());
        set_hidden(project, &closed);
        switch(state, project, player, &keep);
    }
    for name in close {
        // The open tab: move to a neighbour first; the last one stays.
        let visible: Vec<&Tab> = tabs.iter().filter(|t| !closed.contains(&t.name)).collect();
        if visible.len() <= 1 {
            state.status = "the last tab stays open".to_owned();
            continue;
        }
        if state.name.as_deref() == Some(name.as_str()) {
            let at = visible.iter().position(|t| t.name == name).unwrap_or(0);
            let next = visible
                .get(at + 1)
                .or_else(|| at.checked_sub(1).and_then(|p| visible.get(p)))
                .map(|t| t.name.clone());
            if let Some(next) = next {
                switch(state, project, player, &next);
            }
        }
        closed.insert(name);
        set_hidden(project, &closed);
    }
    if let Some(name) = reopen {
        closed.remove(&name);
        set_hidden(project, &closed);
        switch(state, project, player, &name);
    }
    if let Some(name) = open {
        switch(state, project, player, &name);
    }
}

/// The rename / duplicate window.
pub(super) fn form_window(
    state: &mut State,
    project: &mut Project,
    player: &mut Player,
    palette: &Palette,
    ctx: &egui::Context,
) {
    let Some(form) = state.tab_form.as_mut() else {
        return;
    };
    let title = match &form.kind {
        FormKind::Rename(old) => format!("Rename {old}"),
        FormKind::Duplicate(src) => format!("Duplicate {src} as a new plan"),
    };
    let mut open = true;
    let mut go = false;
    let mut cancel = false;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                let r = ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(200.0));
                if !r.has_focus() && form.error.is_none() && !r.lost_focus() {
                    r.request_focus();
                }
            });
            if let Some(e) = &form.error {
                ui.label(
                    egui::RichText::new(e)
                        .size(11.0)
                        .color(theme::rgb(palette.danger)),
                );
            }
            ui.horizontal(|ui| {
                let label = match form.kind {
                    FormKind::Rename(_) => "Rename",
                    FormKind::Duplicate(_) => "Duplicate",
                };
                if ui.button(label).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if go {
        let name = form.name.trim().to_owned();
        let kind = form.kind.clone();
        if !super::valid_name(&name) {
            form.error = Some("Use letters, digits, - and _ (e.g. reel-30).".to_owned());
            return;
        }
        if history::live_path(&project.work, &name).exists() {
            form.error = Some(format!("A plan called {name} exists."));
            return;
        }
        // Save the open plan first so the file carries every edit.
        if state.dirty.is_some() {
            state.save(project);
        }
        let result = match &kind {
            FormKind::Rename(old) => rename_plan(&project.work, old, &name),
            FormKind::Duplicate(src) => {
                let plan = if state.name.as_deref() == Some(src.as_str())
                    && let Some(p) = state.plan.clone()
                {
                    Ok(p)
                } else {
                    Plan::load(&history::live_path(&project.work, src))
                };
                plan.and_then(|p| duplicate_plan(&project.work, src, &p, &name))
            }
        };
        match result {
            Ok(()) => {
                state.tab_form = None;
                project.reload_plans();
                let mut closed = hidden(project);
                if let FormKind::Rename(old) = &kind
                    && closed.remove(old)
                {
                    set_hidden(project, &closed);
                }
                let reopen = match &kind {
                    FormKind::Rename(old) => state.name.as_deref() == Some(old.as_str()),
                    FormKind::Duplicate(_) => true,
                };
                if reopen {
                    state.name = None;
                    switch(state, project, player, &name);
                }
                state.status = match &kind {
                    FormKind::Rename(old) => format!("renamed {old} to {name}"),
                    FormKind::Duplicate(src) => format!("duplicated {src} as {name}"),
                };
            }
            Err(e) => {
                if let Some(form) = state.tab_form.as_mut() {
                    form.error = Some(format!("{e:#}"));
                }
            }
        }
        return;
    }
    if !open || cancel || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.tab_form = None;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::{Duration, SystemTime};

    use super::{Info, Tab, duplicate_plan, free_name, model, rename_plan};
    use crate::gui::pages::plan::history;
    use crate::plan::{Aspect, Plan, Rules};

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn tabs_list_open_plans_with_dirt_and_check_counts() -> anyhow::Result<()> {
        let stems = names(&["feed-30", "reel-15", "reel-30", "wide-60"]);
        let hidden: BTreeSet<String> = ["reel-15", "wide-60"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let t1 = t0 + Duration::from_secs(5);
        let mut info = BTreeMap::new();
        info.insert(
            "feed-30".to_owned(),
            Info {
                stamp: Some(t1),
                seen: Some(t0),
                errors: 3,
            },
        );
        info.insert(
            "reel-30".to_owned(),
            Info {
                stamp: Some(t0),
                seen: Some(t0),
                errors: 9,
            },
        );
        let tabs = model(&stems, &hidden, Some("wide-60"), true, 0, &info);
        assert_eq!(
            tabs,
            vec![
                Tab {
                    name: "feed-30".into(),
                    current: false,
                    dirty: true,
                    errors: 3
                },
                Tab {
                    name: "reel-30".into(),
                    current: false,
                    dirty: false,
                    errors: 9
                },
                Tab {
                    name: "wide-60".into(),
                    current: true,
                    dirty: true,
                    errors: 0
                },
            ],
            "closed tabs hidden except the open one; dirt from disk or from unsaved edits"
        );
        // A plan that appears after the first scan has never been seen.
        info.insert(
            "new".to_owned(),
            Info {
                stamp: Some(t1),
                seen: None,
                errors: 0,
            },
        );
        let tabs = model(&names(&["new"]), &BTreeSet::new(), None, false, 0, &info);
        assert!(
            tabs.first().is_some_and(|t| t.dirty),
            "a new plan file shows a dot"
        );
        Ok(())
    }

    #[test]
    fn duplicate_names_count_up() -> anyhow::Result<()> {
        assert_eq!(
            free_name("reel-30", &names(&["reel-30"])),
            "reel-30-copy",
            "first"
        );
        assert_eq!(
            free_name(
                "reel-30",
                &names(&["reel-30", "reel-30-copy", "reel-30-copy-2"])
            ),
            "reel-30-copy-3",
            "next free"
        );
        Ok(())
    }

    fn plan(name: &str) -> Plan {
        Plan {
            name: name.into(),
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
    fn rename_and_duplicate_move_files_and_history() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path();
        history::save(work, "reel-30", &plan("reel-30"), "draft", 1)?;
        duplicate_plan(work, "reel-30", &plan("reel-30"), "reel-30-copy")?;
        let copy = Plan::load(&history::live_path(work, "reel-30-copy"))?;
        assert_eq!(copy.name, "reel-30-copy", "the copy carries its own name");
        assert_eq!(
            history::list(work, "reel-30-copy")
                .first()
                .map(|v| v.change.clone()),
            Some("duplicated from reel-30".to_owned()),
            "and its own history"
        );
        assert!(
            duplicate_plan(work, "reel-30", &plan("reel-30"), "reel-30-copy").is_err(),
            "never overwrites"
        );

        rename_plan(work, "reel-30", "reel-best")?;
        assert!(
            !history::live_path(work, "reel-30").exists(),
            "old file gone"
        );
        let renamed = Plan::load(&history::live_path(work, "reel-best"))?;
        assert_eq!(renamed.name, "reel-best", "name field follows");
        let versions = history::list(work, "reel-best");
        assert_eq!(
            versions.len(),
            2,
            "history moved, plus the rename: {versions:?}"
        );
        assert!(
            rename_plan(work, "reel-best", "reel-30-copy").is_err(),
            "rename refuses to overwrite"
        );
        Ok(())
    }
}

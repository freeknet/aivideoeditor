// ============================================================================
// Module:       gui::pages::plan::tabs
// Description:  Plans as timeline tabs in the panel header: one per plan
//               file, with an unsaved dot and a failed-check count, close /
//               reopen, rename, duplicate, and "+" to draft or reopen one.
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
use egui::{Align2, CornerRadius, Rect, Sense, Ui, Vec2, pos2, vec2};

use super::{State, history};
use crate::gui::app::Action;
use crate::gui::player::Player;
use crate::gui::ui::{icons, theme, widgets};
use crate::plan::Plan;
use crate::project::Project;
use crate::refs::{Reference, Target};
use crate::theme::Palette;

/// A tab's padding on the left (before the close cross).
const PAD_LEFT: f32 = 6.0;
/// A tab's padding on the right.
const PAD_RIGHT: f32 = 10.0;
/// Space between a tab's parts.
const GAP: f32 = 6.0;
/// The close cross.
const CLOSE: f32 = 14.0;

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
    /// Whether any of them is hard (else they are all soft warnings).
    pub(super) hard: bool,
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
    /// Whether any of them is hard.
    pub(super) hard: bool,
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
    current_errors: (usize, bool),
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
                    current_errors.0
                } else {
                    known.map_or(0, |i| i.errors)
                },
                hard: if is_current {
                    current_errors.1
                } else {
                    known.is_some_and(|i| i.hard)
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
    if let Err(error) = project.save_settings() {
        log::warn!("could not save the project settings: {error:#}");
    }
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
            hard: false,
        });
        if info.stamp != stamp {
            info.stamp = stamp;
            (info.errors, info.hard) = match (
                Plan::load(&history::live_path(&project.work, name)),
                project.index.as_ref(),
                project.music.as_ref(),
            ) {
                (Ok(plan), Some(index), Some(map)) => {
                    let found = crate::plan::check(&plan, index, map, &project.people);
                    (
                        found.len() - crate::plan::accepted_count(&plan, &found),
                        found.iter().any(crate::plan::Finding::is_hard),
                    )
                }
                (Ok(_), _, _) => (0, false),
                // A plan that does not load is an error.
                (Err(_), _, _) => (1, true),
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

/// The tab strip, inside the panel header: one 28 px pill per plan (close
/// cross first, the film glyph, the name, an unsaved dot, a failed-check
/// count), then `+` for a new plan or a closed one.
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
    // Accepted soft findings do not count.
    let any_hard = state.problems.iter().any(crate::plan::Finding::is_hard);
    let current_errors = match (&state.viewing, &state.plan) {
        (None, Some(plan)) => (
            state.problems.len() - crate::plan::accepted_count(plan, &state.problems),
            any_hard,
        ),
        (None, None) => (state.problems.len(), any_hard),
        (Some(_), _) => (0, false),
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
    for tab in &tabs {
        let font = theme::medium(ui.ctx(), theme::BODY_SIZE);
        let galley =
            ui.painter()
                .layout_no_wrap(tab.name.clone(), font, egui::Color32::PLACEHOLDER);
        let count = (tab.errors > 0).then(|| tab.errors.to_string());
        let count_galley = count.as_ref().map(|c| {
            ui.painter().layout_no_wrap(
                c.clone(),
                theme::tabular(ui.ctx(), theme::SMALL_SIZE),
                theme::rgb(t.danger),
            )
        });
        let count_w = count_galley
            .as_ref()
            .map_or(0.0, |g| GAP + icons::SMALL - 2.0 + 2.0 + g.size().x);
        let dot_w = if tab.dirty { GAP + 6.0 } else { 0.0 };
        let w = PAD_LEFT
            + CLOSE
            + GAP
            + icons::SIZE
            + GAP
            + galley.size().x
            + dot_w
            + count_w
            + PAD_RIGHT;
        let (rect, resp) = ui.allocate_exact_size(vec2(w, theme::CONTROL_HEIGHT), Sense::click());
        resp.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Tab, true, tab.current, &tab.name)
        });
        let close_rect = Rect::from_min_size(
            pos2(
                rect.left() + PAD_LEFT - 2.0,
                rect.center().y - (CLOSE + 4.0) / 2.0,
            ),
            vec2(CLOSE + 4.0, CLOSE + 4.0),
        );
        let close_resp = ui.interact(
            close_rect,
            egui::Id::unique("plan_tab_close").with(&tab.name),
            Sense::click(),
        );
        close_resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Close {}", tab.name))
        });
        let close_resp = widgets::tip(close_resp, t, "Close the tab (the cut is kept)", "");
        let hovered = resp.hovered() || close_resp.hovered();
        let p = ui.painter();
        let fill = if tab.current {
            Some(theme::selected_fill(t))
        } else if hovered {
            Some(theme::rgb(t.raised))
        } else {
            None
        };
        if let Some(fill) = fill {
            p.rect_filled(rect, CornerRadius::same(theme::RADIUS_MD), fill);
        }
        let ink = theme::rgb(if tab.current { t.text } else { t.text_muted });
        // The close cross: always there, 30 % until the pointer is on it.
        if close_resp.hovered() {
            p.rect_filled(close_rect, CornerRadius::same(4), theme::ink_hover(t));
        }
        icons::paint(
            p,
            close_rect,
            icons::X,
            CLOSE,
            theme::rgb(t.text_muted).gamma_multiply(if close_resp.hovered() { 1.0 } else { 0.3 }),
        );
        let mut x = rect.left() + PAD_LEFT + CLOSE + GAP;
        icons::paint(
            p,
            Rect::from_min_size(
                pos2(x, rect.center().y - icons::SIZE / 2.0),
                Vec2::splat(icons::SIZE),
            ),
            icons::FILM,
            icons::SIZE,
            ink,
        );
        x += icons::SIZE + GAP;
        let gw = galley.size().x;
        p.galley(
            pos2(x, rect.center().y - galley.size().y / 2.0),
            galley,
            ink,
        );
        x += gw;
        if tab.dirty {
            p.circle_filled(
                pos2(x + GAP + 3.0, rect.center().y),
                3.0,
                theme::rgb(t.accent),
            );
            x += dot_w;
        }
        if let Some(g) = count_galley {
            x += GAP;
            // Hard findings are errors (red cross); soft ones only warn.
            let (glyph, tone) = if tab.hard {
                (icons::CIRCLE_X, theme::rgb(t.danger))
            } else {
                (icons::TRIANGLE_ALERT, theme::rgb(t.warning))
            };
            icons::paint(
                p,
                Rect::from_min_size(
                    pos2(x, rect.center().y - (icons::SMALL - 2.0) / 2.0),
                    Vec2::splat(icons::SMALL - 2.0),
                ),
                glyph,
                icons::SMALL - 2.0,
                tone,
            );
            x += icons::SMALL;
            p.galley(pos2(x, rect.center().y - g.size().y / 2.0), g, tone);
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
            if tab.errors > 0 && tab.hard {
                format!(" · check fails ({} findings)", tab.errors)
            } else if tab.errors > 0 {
                format!(" · {} warning(s), still renders", tab.errors)
            } else {
                String::new()
            }
        );
        let resp = widgets::tip(resp, t, &tip, "");
        if close_resp.clicked() || resp.middle_clicked() {
            close.push(tab.name.clone());
        } else if resp.clicked() {
            open = Some(tab.name.clone());
        }
        widgets::context_menu(&resp, |ui| {
            ui.set_min_width(200.0);
            if widgets::menu_item(ui, t, Some(icons::PENCIL), "Rename…", "", false).clicked() {
                state.tab_form = Some(Form {
                    kind: FormKind::Rename(tab.name.clone()),
                    name: tab.name.clone(),
                    error: None,
                });
                ui.close();
            }
            if widgets::menu_item(
                ui,
                t,
                Some(icons::COPY),
                "Duplicate as a new cut…",
                "",
                false,
            )
            .clicked()
            {
                state.tab_form = Some(Form {
                    kind: FormKind::Duplicate(tab.name.clone()),
                    name: free_name(&tab.name, &names),
                    error: None,
                });
                ui.close();
            }
            if widgets::menu_item(ui, t, Some(icons::AT_SIGN), "Reference in chat", "", false)
                .clicked()
            {
                actions.push(Action::AddRef(Reference::new(Target::Plan {
                    name: tab.name.clone(),
                })));
                ui.close();
            }
            widgets::menu_separator(ui, t);
            if widgets::menu_item(ui, t, Some(icons::X), "Close tab", "", false).clicked() {
                close.push(tab.name.clone());
                ui.close();
            }
            let others = ui
                .add_enabled_ui(tabs.len() > 1, |ui| {
                    widgets::menu_item(ui, t, None, "Close other tabs", "", false)
                })
                .inner;
            if widgets::why_disabled(others, "This is the only open tab").clicked() {
                close_others = Some(tab.name.clone());
                ui.close();
            }
        });
    }
    // "+": a new plan, or one of the closed tabs.
    let busy = state.pending_job.is_some();
    let (rect, plus) = ui.allocate_exact_size(Vec2::splat(theme::CONTROL_HEIGHT), Sense::click());
    plus.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, "New cut, or reopen one")
    });
    let open_menu = widgets::menu_is_open(ui.ctx(), widgets::menu_id(&plus));
    if plus.hovered() || open_menu {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_MD),
            theme::ink(t, theme::INK_10),
        );
    }
    icons::paint(
        ui.painter(),
        rect,
        icons::PLUS,
        icons::SIZE,
        if plus.hovered() || open_menu {
            theme::rgb(t.text_muted)
        } else {
            theme::ring(t)
        },
    );
    let plus = widgets::tip(plus, t, "New cut, or reopen a closed tab", "");
    let reopenable: Vec<&String> = names
        .iter()
        .filter(|n| closed.contains(*n) && Some(n.as_str()) != state.name.as_deref())
        .collect();
    let mut draft = false;
    // The response is not needed: drawing it is the point.
    let _ = widgets::menu(&plus)
        .width(220.0)
        .gap(theme::SPACE_XS)
        .show(|ui| {
            let new = ui
                .add_enabled_ui(!busy, |ui| {
                    widgets::menu_item(ui, t, Some(icons::PLUS), "New cut…", "", false)
                })
                .inner;
            if widgets::why_disabled(new, "Wait for the cut being drafted").clicked() {
                draft = true;
                ui.close();
            }
            widgets::menu_separator(ui, t);
            if reopenable.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(theme::SPACE_SM);
                    ui.label(egui::RichText::new("All cuts open").color(theme::rgb(t.text_muted)));
                });
            }
            for n in &reopenable {
                if widgets::menu_item(ui, t, Some(icons::FILM), n, "", false).clicked() {
                    reopen = Some((*n).clone());
                    ui.close();
                }
            }
        });
    if draft {
        super::open_draft_form(state, project);
    }

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
                let r = widgets::input(&mut form.name)
                    .label("Name")
                    .width(200.0)
                    .show(ui)
                    .response;
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
                if widgets::key_button(ui, palette, label).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    go = true;
                }
                if widgets::ghost_button(ui, palette, "Cancel").clicked() {
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
                hard: false,
            },
        );
        info.insert(
            "reel-30".to_owned(),
            Info {
                stamp: Some(t0),
                seen: Some(t0),
                errors: 9,
                hard: true,
            },
        );
        let tabs = model(&stems, &hidden, Some("wide-60"), true, (0, false), &info);
        assert_eq!(
            tabs,
            vec![
                Tab {
                    name: "feed-30".into(),
                    current: false,
                    dirty: true,
                    errors: 3,
                    hard: false
                },
                Tab {
                    name: "reel-30".into(),
                    current: false,
                    dirty: false,
                    errors: 9,
                    hard: true
                },
                Tab {
                    name: "wide-60".into(),
                    current: true,
                    dirty: true,
                    errors: 0,
                    hard: false
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
                hard: false,
            },
        );
        let tabs = model(
            &names(&["new"]),
            &BTreeSet::new(),
            None,
            false,
            (0, false),
            &info,
        );
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
            accepted: Vec::new(),
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

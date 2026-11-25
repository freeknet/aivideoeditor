// ============================================================================
// Module:       gui::pages::inspector::cut
// Description:  With nothing selected: the open cut at a glance (shape, length,
//               the rules it keeps, what the chat message points at).
//
// Dependencies: egui; super::rows; crate::{plan, project, refs}
// ============================================================================

//! The open cut, when nothing is selected.
//!
//! Instead of an empty panel, the inspector shows the cut the editor has
//! open: its shape, length and shot count, the rules it was drafted under
//! (saying so when the drafter loosened the project's to fit the footage),
//! and the shots the chat message being written points at, with a way to
//! clear them.

use std::path::PathBuf;
use std::time::SystemTime;

use super::rows;
use crate::gui::pages::PageCtx;
use crate::gui::ui::{icons, theme, widgets};
use crate::plan::Plan;
use crate::refs::Target;

/// The open cut, read once per change of its file.
#[derive(Debug, Default)]
pub(super) struct Cache {
    loaded: Option<(PathBuf, Option<SystemTime>, Plan)>,
}

impl Cache {
    /// The plan at `path`, read again only when the file changed.
    fn plan(&mut self, path: &PathBuf) -> Option<&Plan> {
        let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let fresh = self
            .loaded
            .as_ref()
            .is_some_and(|(p, m, _)| p == path && *m == mtime);
        if !fresh {
            self.loaded = Plan::load(path)
                .ok()
                .map(|plan| (path.clone(), mtime, plan));
        }
        self.loaded.as_ref().map(|(_, _, plan)| plan)
    }
}

/// The open cut's file: the one the editor last showed, if it still exists.
fn open_cut(ctx: &PageCtx<'_>) -> Option<(String, PathBuf)> {
    let project = ctx.project.as_ref()?;
    let name = project
        .settings
        .pages
        .get("plan")
        .and_then(|t| t.get("current"))
        .and_then(|v| v.as_str())?
        .to_owned();
    let path = project
        .plans
        .iter()
        .find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy() == name))?
        .clone();
    Some((name, path))
}

/// Draws the open cut's summary; `false` when there is no open cut (the
/// caller shows its empty state).
pub(super) fn draw(ctx: &mut PageCtx<'_>, cache: &mut Cache, ui: &mut egui::Ui) -> bool {
    let Some((name, path)) = open_cut(ctx) else {
        return false;
    };
    let Some(plan) = cache.plan(&path).cloned() else {
        return false;
    };
    let palette = ctx.theme;
    let length = ctx
        .project
        .as_ref()
        .and_then(|p| p.music.as_ref())
        .map(|map| map.beat_time(plan.end_bar, 0) - map.beat_time(plan.start_bar, 0));
    let shape = crate::recipe::aspect_key(plan.aspect);
    let mut subtitle = format!("{shape} · {} shots", plan.shots.len());
    if let Some(len) = length {
        subtitle.push_str(&format!(
            " · {}:{:02}",
            (len / 60.0) as u32,
            (len % 60.0) as u32
        ));
    }
    if plan.music.is_empty() {
        subtitle.push_str(" · no music");
    }
    egui::ScrollArea::vertical()
        .id_salt("inspector-cut")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            rows::title(ui, palette, &name, &subtitle);
            ui.label(rows::text(
                "Select a shot, a segment or a person to see everything known about it.",
                rows::SMALL,
                theme::rgb(palette.text_muted),
            ));

            // What the chat message points at in this cut.
            let flagged: Vec<usize> = ctx
                .refs
                .iter()
                .filter_map(|r| match &r.target {
                    Target::Shot { plan: p, index } if *p == name => Some(*index),
                    Target::Shot { .. }
                    | Target::Segment { .. }
                    | Target::Range { .. }
                    | Target::Frame { .. }
                    | Target::Bars { .. }
                    | Target::Person { .. }
                    | Target::Plan { .. } => None,
                })
                .collect();
            rows::section(ui, palette, "In the chat message");
            if flagged.is_empty() {
                ui.label(rows::text(
                    "Nothing yet. Select shots and press R, or use Ask the chat in the bar under the timeline.",
                    rows::SMALL,
                    theme::rgb(palette.text_muted),
                ));
            } else {
                let list: Vec<String> = flagged.iter().map(|n| format!("shot {n}")).collect();
                rows::row(ui, palette, "Shots", list.join(", "));
                if widgets::button_with(
                    ui,
                    palette,
                    widgets::ButtonKind::Ghost,
                    Some(icons::X),
                    "Clear them",
                )
                .clicked()
                {
                    ctx.refs.retain(|r| {
                        !matches!(&r.target, Target::Shot { plan: p, .. } if *p == name)
                    });
                }
            }

            // The rules it was drafted under.
            rows::section(ui, palette, "Rules it keeps");
            let r = &plan.rules;
            rows::row(ui, palette, "Same person", format!("{} shots apart", r.person_gap));
            rows::row(ui, palette, "Same clip", format!("{:.0} s apart", r.same_clip_gap));
            if !plan.music.is_empty() {
                let drop = if r.drop_subject.is_empty() {
                    "any shot".to_owned()
                } else {
                    r.drop_subject.clone()
                };
                rows::row(ui, palette, "On the drop", drop);
            }
            let project_rules = ctx
                .project
                .as_ref()
                .map(|p| crate::recipe::Recipe::cached(&p.work).rules);
            if project_rules.is_some_and(|pr| pr.person_gap > r.person_gap || pr.same_clip_gap > r.same_clip_gap) {
                ui.label(rows::text(
                    "Looser than the project's rules: the drafter eased them to fit the footage.",
                    rows::SMALL,
                    theme::rgb(palette.warning),
                ));
            }
        });
    true
}

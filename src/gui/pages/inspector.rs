// ============================================================================
// Module:       gui::pages::inspector
// Description:  The inspector, as Selects' right side panel: an Inspector |
//               Facts tab header, then everything known about the selection
//               (numbers, caption, flags, who, where it is used), or the
//               exact facts the chat would receive for it.
//
// Dependencies: egui; crate::{refs, gui::pages, gui::ui}
// ============================================================================

//! The inspector, docked on the workspace's right.
//!
//! The 40 px header holds two tab pills. **Inspector** shows whatever
//! `ctx.selection` holds, whichever region selected it:
//!
//! - a **segment**, **range** or **frame**: a frame cropped where the action
//!   is, the shot-log measurements, every flag with the threshold that set
//!   it, the full caption, who the people map says it is, which plan shots
//!   use it, and the clip's segments as a strip to walk along;
//! - a **shot**: the plan's fields (clip, in, beats, crop, who, note) and
//!   where it sits on the beat grid, then its footage as above;
//! - **bars**: their times and per-bar energy, and any drop among them;
//! - a **person**: their best frames, props, entries and the shots that
//!   show them;
//! - a **plan**: its frame, bars and shot list;
//! - several things: a compact list, one line each; click one to inspect it.
//!
//! Links (a person, a shot, a segment in the strip) replace the selection,
//! so every other region follows. **Facts** shows the exact facts the chat
//! would receive for the selection. With nothing selected, either tab is a
//! centred empty state.

mod details;
mod rows;
mod usage;

use super::PageCtx;
use crate::gui::ui::{icons, theme, widgets};
use crate::refs::{Reference, Target};

use details::{Op, View};
use rows::{META, SMALL, text};

/// The inspector's views, the tabs in its header.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    /// Everything known about the selection.
    #[default]
    Inspector,
    /// The facts the chat receives for it.
    Facts,
}

/// The inspector's own state.
#[derive(Debug, Default)]
pub(crate) struct State {
    plans: usage::Plans,
    tab: Tab,
}

/// Draws the inspector into its panel.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let count = ctx.selection.len();
    widgets::panel_bar(
        ui,
        ctx.theme,
        |ui| {
            widgets::tab_strip(
                ui,
                ctx.theme,
                &mut state.tab,
                &[(Tab::Inspector, "Inspector"), (Tab::Facts, "Facts")],
            );
        },
        |ui| {
            if count > 1 {
                ui.add_space(theme::SPACE_XS);
                let _ = widgets::pill(
                    ui,
                    ctx.theme,
                    &format!("{count} selected"),
                    widgets::Tone::Neutral,
                );
            }
        },
    );
    if ctx.selection.is_empty() {
        widgets::empty_state(
            ui,
            ctx.theme,
            icons::SLIDERS,
            "Nothing selected",
            "Select a shot, segment, bar or person to see everything known about it.",
            |_| {},
        );
        return;
    }
    let Some(project) = ctx.project.as_ref() else {
        widgets::empty_state(
            ui,
            ctx.theme,
            icons::FOLDER_OPEN,
            "No night open",
            "Open a night to inspect its footage.",
            |_| {},
        );
        return;
    };
    state.plans.refresh(&project.plans);
    let selection = ctx.selection.clone();
    let mut ops = Vec::new();
    let view = View {
        palette: ctx.theme,
        project,
        plans: &state.plans,
    };
    let tab = state.tab;
    egui::ScrollArea::vertical()
        .id_salt("inspector")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // Selects' rows sit 16 px in from the panel's left edge and
            // 8 px from its right: the panel's inset gives the first 8.
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: theme::as_margin(theme::SPACE_SM),
                    right: 0,
                    top: 0,
                    bottom: theme::as_margin(theme::SPACE_LG),
                })
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = theme::SPACE_XS;
                    match (tab, selection.as_slice()) {
                        (Tab::Inspector, [one]) => single(&view, ui, ctx.thumbs, one, &mut ops),
                        (Tab::Inspector, many) => list(&view, ui, many, &mut ops),
                        (Tab::Facts, items) => facts(&view, ui, items),
                    }
                });
        });
    for op in ops {
        match op {
            Op::Select(reference) => *ctx.selection = vec![reference],
            Op::Open(path, t0, t1, cx) => ctx.player.open_clip(&path, t0, t1, cx),
        }
    }
}

/// One selected item, in full.
fn single(
    view: &View<'_>,
    ui: &mut egui::Ui,
    thumbs: &mut crate::gui::thumbs::Thumbs,
    item: &Reference,
    ops: &mut Vec<Op>,
) {
    ui.add_space(theme::SPACE_SM);
    match &item.target {
        Target::Segment { id } => {
            let seg = view
                .project
                .index
                .as_ref()
                .and_then(|i| i.segments.iter().find(|s| s.id == *id));
            match seg {
                Some(seg) => details::segment(view, ui, thumbs, seg, ops),
                None => {
                    rows::row_colored(
                        ui,
                        view.palette,
                        "",
                        format!("No segment {id} in the index."),
                        theme::rgb(view.palette.text_muted),
                    );
                }
            }
        }
        Target::Range { clip, t0, t1 } => details::range(view, ui, thumbs, clip, (*t0, *t1), ops),
        Target::Frame { clip, t } => details::range(view, ui, thumbs, clip, (*t, *t), ops),
        Target::Shot { plan, index } => details::shot(view, ui, thumbs, plan, *index, ops),
        Target::Bars { from, to } => details::bars(view, ui, *from, *to),
        Target::Person { name } => details::person(view, ui, thumbs, name, ops),
        Target::Plan { name } => details::plan(view, ui, name, ops),
    }
}

/// What kind of thing a reference points at, for the list.
fn kind(target: &Target) -> &'static str {
    match target {
        Target::Shot { .. } => "Shot",
        Target::Segment { .. } => "Segment",
        Target::Range { .. } => "Range",
        Target::Frame { .. } => "Frame",
        Target::Bars { .. } => "Bars",
        Target::Person { .. } => "Person",
        Target::Plan { .. } => "Plan",
    }
}

/// Several selected items: one compact entry each; click one to inspect it.
fn list(view: &View<'_>, ui: &mut egui::Ui, items: &[Reference], ops: &mut Vec<Op>) {
    let p = view.palette;
    rows::section_with(ui, p, "Selection", &items.len().to_string());
    for item in items {
        let line = match crate::refs::resolve(item, view.project) {
            Ok(r) => r.facts,
            Err(e) => e,
        };
        let first = line.split("; ").next().unwrap_or_default().to_owned();
        if rows::row_link(ui, p, kind(&item.target), &item.short()) {
            ops.push(Op::Select(item.clone()));
        }
        ui.horizontal_top(|ui| {
            ui.add_space(88.0 + theme::SPACE_SM);
            ui.add(egui::Label::new(text(first, META, theme::rgb(p.text_muted))).wrap());
        });
        ui.add_space(2.0);
    }
}

/// The facts the chat receives for the selection.
fn facts(view: &View<'_>, ui: &mut egui::Ui, items: &[Reference]) {
    let p = view.palette;
    for item in items {
        rows::section(ui, p, &item.to_string());
        let facts = crate::refs::resolve(item, view.project).map_or_else(|e| e, |r| r.facts);
        for fact in facts.split("; ") {
            ui.add(egui::Label::new(text(fact, SMALL, theme::rgb(p.text_muted))).wrap());
        }
    }
}

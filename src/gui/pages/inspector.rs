// ============================================================================
// Module:       gui::pages::inspector
// Description:  The inspector: everything known about the selection (numbers,
//               caption, flags, who, where it is used), and its settings.
//
// Dependencies: egui; crate::{refs, gui::pages}
// ============================================================================

//! The inspector, docked on the workspace's right.
//!
//! It shows whatever `ctx.selection` holds, whichever region selected it:
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
//! so every other region follows. At the foot, the exact facts the chat
//! would receive for the selection.

mod details;
mod rows;
mod usage;

use super::PageCtx;
use crate::gui::ui::{theme, widgets};
use crate::refs::{Reference, Target};

use details::{Op, View};
use rows::{META, SMALL, text};

/// The inspector's own state.
#[derive(Debug, Default)]
pub(crate) struct State {
    plans: usage::Plans,
}

/// Draws the inspector into its panel.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    widgets::title(ui, ctx.theme, "Inspector");
    if ctx.selection.is_empty() {
        widgets::muted(
            ui,
            ctx.theme,
            "Select a shot, segment, bar or person to see everything known about it.",
        );
        return;
    }
    let Some(project) = ctx.project.as_ref() else {
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
    egui::ScrollArea::vertical()
        .id_salt("inspector")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            match selection.as_slice() {
                [one] => single(&view, ui, ctx.thumbs, one, &mut ops),
                many => list(&view, ui, many, &mut ops),
            }
            facts(&view, ui, &selection);
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
                    widgets::muted(ui, view.palette, &format!("No segment {id} in the index."));
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

/// Several selected items: one compact line each.
fn list(view: &View<'_>, ui: &mut egui::Ui, items: &[Reference], ops: &mut Vec<Op>) {
    let p = view.palette;
    ui.label(text(
        format!("{} selected · click one to inspect it", items.len()),
        META,
        theme::rgb(p.text_muted),
    ));
    ui.add_space(theme::SPACE_XS);
    for item in items {
        let kind = match &item.target {
            Target::Shot { .. } => "shot",
            Target::Segment { .. } => "segment",
            Target::Range { .. } => "range",
            Target::Frame { .. } => "frame",
            Target::Bars { .. } => "bars",
            Target::Person { .. } => "person",
            Target::Plan { .. } => "plan",
        };
        let line = match crate::refs::resolve(item, view.project) {
            Ok(r) => r.facts,
            Err(e) => e,
        };
        let first = line.split("; ").next().unwrap_or_default().to_owned();
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
            ui.label(text(kind, META - 1.0, theme::rgb(p.text_faint)));
            if ui
                .link(text(item.short(), SMALL, theme::rgb(p.accent)))
                .clicked()
            {
                ops.push(Op::Select(item.clone()));
            }
        });
        ui.add(egui::Label::new(text(first, META, theme::rgb(p.text_muted))).wrap());
        ui.add_space(2.0);
    }
}

/// The facts the chat receives for the selection, collapsed by default.
fn facts(view: &View<'_>, ui: &mut egui::Ui, items: &[Reference]) {
    ui.add_space(theme::SPACE_SM);
    egui::CollapsingHeader::new(text(
        "Facts sent to chat",
        META,
        theme::rgb(view.palette.text_muted),
    ))
    .id_salt("inspector-facts")
    .default_open(false)
    .show(ui, |ui| {
        for item in items {
            let facts = crate::refs::resolve(item, view.project).map_or_else(|e| e, |r| r.facts);
            ui.label(text(item.to_string(), META, theme::rgb(view.palette.text)));
            ui.add(egui::Label::new(text(facts, META, theme::rgb(view.palette.text_muted))).wrap());
            ui.add_space(theme::SPACE_XS);
        }
    });
}

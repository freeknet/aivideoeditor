// ============================================================================
// Module:       gui::pages::people
// Description:  People: performer groups, the confirm queue, merge/split/rename
//               and manual clip ranges.
//
// Dependencies: egui; crate::{people, refs}, crate::gui::pages (PageCtx)
// ============================================================================

//! The People tab: the people map (`people.toml`) as a grid of performers.
//!
//! Left (or top, when the browser is narrow): one card per person, plus
//! Unassigned for performer footage nobody is mapped to, each with frames
//! cropped on the action, counts and the props the captions saw. Right: the
//! focused person's clips and segments with their captions. Everything is
//! an edit of the map, saved at once and undoable:
//!
//! | Gesture | Effect |
//! |---|---|
//! | click / Ctrl-click / Shift-click | select people or segments |
//! | `Enter` / `F2`, double-click a name | rename inline |
//! | `Ctrl+M` | merge the selected people into the first |
//! | `Ctrl+Shift+M` | split the picked segments off as a new person |
//! | drag segments (or a clip header) onto a card | move them to that person |
//! | right-click | move to ▸, reference in chat ▸, avoid in plans |
//! | `Space` (pointer over the tab) | the confirm queue: `Y` / `N` / `S` |
//! | `Ctrl+Z` / `Ctrl+Shift+Z` | undo / redo |
//!
//! Moving part of a clip writes `clip@from-to` ranges, exactly what the
//! command line's people step accepts; "Assign range…" writes one by hand.

mod edit;
mod model;
mod queue;
mod view;

use egui::{Key, Modifiers, Ui};

use super::PageCtx;
use crate::gui::app::Action;
use crate::gui::ui::{keymap, theme, widgets};
use crate::refs::{Modifier, Reference, Target};

use model::{Group, Order};
use queue::{Answer, Queue};
use view::{META, SMALL, text};

/// The drag payload: segments (indices into `index.segments`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DragSegs(pub(crate) Vec<usize>);

/// Which card the detail pane shows.
#[derive(Clone, Debug, Default, PartialEq)]
enum Focus {
    /// Nothing yet.
    #[default]
    None,
    /// A named person.
    Person(String),
    /// The Unassigned pile.
    Unassigned,
}

impl Focus {
    /// The focused group's name, `None` for Unassigned.
    fn name(&self) -> Option<Option<&str>> {
        match self {
            Self::None => None,
            Self::Person(n) => Some(Some(n.as_str())),
            Self::Unassigned => Some(None),
        }
    }
}

/// A dialog over the tab.
#[derive(Clone, Debug, PartialEq)]
enum Dialog {
    /// Name a new person for these segments (split, or move to new).
    NewPerson {
        /// Segments to move.
        segs: Vec<usize>,
        /// The name being typed.
        name: String,
    },
    /// Map a time range of a clip to a person by hand.
    AssignRange {
        /// Clip id.
        clip: String,
        /// Start, seconds.
        from: f32,
        /// End, seconds.
        to: f32,
        /// Person.
        name: String,
    },
}

/// Something the user asked for this frame, applied after drawing.
#[derive(Clone, Debug, PartialEq)]
enum Op {
    /// Click on a person card.
    Person(Option<String>, Modifiers),
    /// Click on a segment row.
    Segment(usize, Modifiers),
    /// Start renaming a person.
    StartRename(String),
    /// Finish renaming.
    Rename(String, String),
    /// Merge these people into the first.
    Merge(Vec<String>),
    /// Move segments to a person (`None`: nobody).
    Move(Vec<usize>, Option<String>),
    /// Remove a person from the map.
    Forget(String),
    /// Open a dialog.
    Open(Dialog),
    /// Assign a range from the dialog.
    AssignRange(String, f32, f32, String),
    /// Stage a reference for the chat.
    Ref(Reference),
    /// Undo the last edit.
    Undo,
    /// Redo.
    Redo,
    /// Start the confirm queue.
    StartQueue,
    /// Answer the queue's question.
    Answer(Answer),
    /// File the queue's clip under another guess.
    AnswerAs(String),
    /// Stop the queue.
    StopQueue,
    /// Draft people.toml from the captions.
    Draft,
}

/// The page's own state, kept across frames and page switches.
#[derive(Debug, Default)]
pub(crate) struct State {
    cache: model::Cache,
    order: Order,
    filter: String,
    selected: Vec<String>,
    anchor: Option<String>,
    focus: Focus,
    picked: Vec<usize>,
    seg_anchor: Option<usize>,
    renaming: Option<(String, String)>,
    dialog: Option<Dialog>,
    history: edit::History,
    queue: Option<Queue>,
    asked: Option<String>,
    status: String,
}

/// Draws the page into the browser.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    if ctx.project.is_none() {
        super::no_project(ctx, ui);
        return;
    }
    let hot = ui.rect_contains_pointer(ui.max_rect());
    let mut ops = Vec::new();
    keys(ctx, state, ui, hot, &mut ops);
    toolbar(ctx, state, ui, &mut ops);
    if state.queue.is_some() {
        confirm_queue(ctx, state, ui, &mut ops);
    } else {
        body(ctx, state, ui, &mut ops);
    }
    dialog(ctx, state, ui, &mut ops);
    for op in ops {
        apply(ctx, state, op);
    }
}

/// The tab's shortcuts.
fn keys(ctx: &PageCtx<'_>, state: &mut State, ui: &Ui, hot: bool, ops: &mut Vec<Op>) {
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        if state.dialog.is_some() {
            state.dialog = None;
        } else if state.renaming.is_some() {
            state.renaming = None;
        } else if state.queue.is_some() {
            ops.push(Op::StopQueue);
        }
    }
    if !ctx.keys_free(ui) || !hot {
        return;
    }
    let preset = ctx.keymap;
    ui.input_mut(|i| {
        let mut take = |id| keymap::consume(i, id, preset);
        if state.queue.is_some() {
            for (id, answer) in [
                ("people-yes", Answer::Yes),
                ("people-no", Answer::No),
                ("people-skip", Answer::Skip),
            ] {
                if take(id) {
                    ops.push(Op::Answer(answer));
                }
            }
            return;
        }
        if take("people-new-person") {
            if !state.picked.is_empty() {
                ops.push(Op::Open(Dialog::NewPerson {
                    segs: state.picked.clone(),
                    name: String::new(),
                }));
            }
        } else if take("people-merge") && state.selected.len() > 1 {
            ops.push(Op::Merge(state.selected.clone()));
        }
        if take("people-redo") || take("people-redo-alt") {
            ops.push(Op::Redo);
        } else if take("people-undo") {
            ops.push(Op::Undo);
        }
        if take("people-queue") {
            ops.push(Op::StartQueue);
        }
        if (take("people-rename") || take("people-rename-alt"))
            && let Focus::Person(name) = &state.focus
        {
            ops.push(Op::StartRename(name.clone()));
        }
    });
}

/// The strip above the grid: queue, range, undo, order, filter.
fn toolbar(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui, ops: &mut Vec<Op>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
        let queue_on = state.queue.is_some();
        if ui
            .selectable_label(queue_on, text("Confirm", SMALL, theme::rgb(ctx.theme.text)))
            .on_hover_text("Confirm queue (Space): one doubtful clip at a time, Y / N / S")
            .clicked()
        {
            ops.push(if queue_on {
                Op::StopQueue
            } else {
                Op::StartQueue
            });
        }
        if ui
            .button(text("Assign range…", SMALL, theme::rgb(ctx.theme.text)))
            .on_hover_text("Map clip@from-to to a person")
            .clicked()
        {
            ops.push(Op::Open(range_dialog(ctx, state)));
        }
        let undo = state
            .history
            .last()
            .map_or_else(|| "Nothing to undo".to_owned(), |l| format!("Undo {l}"));
        if ui
            .add_enabled(
                state.history.last().is_some(),
                egui::Button::new(text("↶", SMALL, theme::rgb(ctx.theme.text))),
            )
            .on_hover_text(format!("{undo} (Ctrl+Z)"))
            .clicked()
        {
            ops.push(Op::Undo);
        }
        egui::ComboBox::from_id_salt("people-order")
            .selected_text(text(state.order.label(), META, theme::rgb(ctx.theme.text)))
            .width(84.0)
            .show_ui(ui, |ui| {
                for o in Order::ALL {
                    ui.selectable_value(&mut state.order, o, o.label());
                }
            })
            .response
            .on_hover_text("Order people by");
        ui.add(
            egui::TextEdit::singleline(&mut state.filter)
                .hint_text("Filter people")
                .font(theme::font(SMALL))
                .desired_width(ui.available_width()),
        );
    });
    if !state.status.is_empty() {
        ui.label(text(
            state.status.as_str(),
            META,
            theme::rgb(ctx.theme.text_muted),
        ));
    }
    ui.add_space(theme::SPACE_XS);
}

/// The grid and the detail pane, side by side or stacked.
fn body(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui, ops: &mut Vec<Op>) {
    let empty = ctx
        .project
        .as_ref()
        .is_some_and(|p| p.people.names.is_empty());
    let captioned = ctx.project.as_ref().is_some_and(|p| {
        p.index
            .as_ref()
            .is_some_and(|i| i.segments.iter().any(|s| s.caption.is_some()))
    });
    if empty && captioned {
        widgets::muted(
            ui,
            ctx.theme,
            "No people.toml yet. Draft one from the captions, then rename and fix groups here.",
        );
        if widgets::button(
            ui,
            ctx.theme,
            "Draft from captions",
            widgets::Emphasis::Primary,
        )
        .clicked()
        {
            ops.push(Op::Draft);
        }
        ui.add_space(theme::SPACE_SM);
    }
    let wide = ui.available_width() >= 600.0;
    let panel = if wide {
        egui::Panel::left("people-grid")
            .resizable(true)
            .default_size(ui.available_width() * 0.42)
            .size_range(170.0..=ui.available_width() * 0.7)
    } else {
        egui::Panel::top("people-grid")
            .resizable(true)
            .default_size(ui.available_height() * 0.45)
            .size_range(120.0..=ui.available_height() * 0.8)
    };
    panel
        .frame(egui::Frame::new().inner_margin(theme::margin_xy(0.0, 0.0)))
        .show(ui, |ui| grid(ctx, state, ui, ops));
    egui::CentralPanel::default()
        .frame(egui::Frame::new().inner_margin(theme::margin_xy(theme::SPACE_XS, 0.0)))
        .show(ui, |ui| detail(ctx, state, ui, ops));
    if egui::DragAndDrop::has_payload_of_type::<DragSegs>(ui.ctx())
        && let Some(drag) = egui::DragAndDrop::payload::<DragSegs>(ui.ctx())
    {
        view::drag_label(ui, ctx.theme, drag.0.len());
    }
}

/// The person cards.
fn grid(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui, ops: &mut Vec<Op>) {
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    let model = state.cache.get(project, state.order);
    let index = project.index.as_ref();
    let filter = state.filter.to_lowercase();
    let shown: Vec<&Group> = model
        .groups
        .iter()
        .filter(|g| {
            filter.is_empty()
                || g.label().to_lowercase().contains(&filter)
                || g.sample.to_lowercase().contains(&filter)
                || g.props.iter().any(|(p, _)| p.contains(filter.as_str()))
        })
        .chain((!model.unassigned.clips.is_empty()).then_some(&model.unassigned))
        .collect();
    let names: Vec<String> = model.groups.iter().filter_map(|g| g.name.clone()).collect();
    egui::ScrollArea::vertical()
        .id_salt("people-cards")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (columns, width) =
                widgets::grid_layout(ui.available_width(), 150.0, theme::SPACE_XS, shown.len());
            for row in shown.chunks(columns.max(1)) {
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
                    for group in row {
                        let column = egui::Layout::top_down(egui::Align::Min);
                        ui.allocate_ui_with_layout(egui::vec2(width, 0.0), column, |ui| {
                            ui.set_width(width);
                            let look = view::CardLook {
                                selected: group
                                    .name
                                    .as_ref()
                                    .is_some_and(|n| state.selected.contains(n)),
                                focused: state.focus.name() == Some(group.name.as_deref()),
                                droppable: true,
                            };
                            let card =
                                view::card(ui, ctx.thumbs, ctx.theme, group, index, model, &look);
                            let cx = CardCtx {
                                palette: ctx.theme,
                                selected: &state.selected,
                                names: &names,
                            };
                            card_actions(&cx, &mut state.renaming, ui, group, &card, ops);
                        });
                    }
                });
                ui.add_space(theme::SPACE_XS);
            }
            if shown.is_empty() {
                widgets::muted(ui, ctx.theme, "Nobody matches.");
            }
        });
}

/// What a card's actions need from the page.
struct CardCtx<'a> {
    palette: &'a crate::theme::Palette,
    selected: &'a [String],
    names: &'a [String],
}

/// Clicks, drops, renaming and the context menu of one card.
fn card_actions(
    cx: &CardCtx<'_>,
    renaming: &mut Option<(String, String)>,
    ui: &mut Ui,
    group: &Group,
    response: &egui::Response,
    ops: &mut Vec<Op>,
) {
    let names = cx.names;
    if let Some(drag) = response.dnd_release_payload::<DragSegs>() {
        ops.push(Op::Move(drag.0.clone(), group.name.clone()));
    }
    if response.clicked() {
        ops.push(Op::Person(group.name.clone(), ui.input(|i| i.modifiers)));
    }
    if response.double_clicked()
        && let Some(name) = &group.name
    {
        ops.push(Op::StartRename(name.clone()));
    }
    // Inline rename over the card's name line.
    let mut cancel = false;
    if let (Some(name), Some((old, buffer))) = (&group.name, renaming.as_mut())
        && name == old
    {
        let rect = egui::Rect::from_min_size(
            response.rect.left_bottom() - egui::vec2(-theme::SPACE_XS, 50.0),
            egui::vec2(response.rect.width() - 2.0 * theme::SPACE_XS, 20.0),
        );
        let edit = ui
            .put(
                rect,
                egui::TextEdit::singleline(buffer)
                    .font(theme::font(SMALL))
                    .id(egui::Id::unique("people-rename")),
            )
            .accessible_name(format!("Rename {old}"));
        if !edit.has_focus() && !edit.lost_focus() {
            edit.request_focus();
        }
        if edit.lost_focus() {
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                ops.push(Op::Rename(old.clone(), buffer.clone()));
            } else {
                cancel = true;
            }
        }
    }
    if cancel {
        *renaming = None;
    }
    let palette = cx.palette;
    response.context_menu(|ui| {
        let Some(name) = group.name.clone() else {
            ui.label(text(
                "Performer footage nobody is mapped to",
                META,
                theme::rgb(palette.text_muted),
            ));
            if ui.button("Confirm these one by one (Space)").clicked() {
                ops.push(Op::StartQueue);
            }
            return;
        };
        if ui.button("Rename (Enter)").clicked() {
            ops.push(Op::StartRename(name.clone()));
        }
        let others: Vec<String> = cx
            .selected
            .iter()
            .filter(|n| **n != name)
            .cloned()
            .collect();
        if !others.is_empty()
            && ui
                .button(format!(
                    "Merge {} selected into {name} (Ctrl+M)",
                    others.len()
                ))
                .clicked()
        {
            let mut all = vec![name.clone()];
            all.extend(others);
            ops.push(Op::Merge(all));
        }
        ui.menu_button("Merge into ▸", |ui| {
            egui::ScrollArea::vertical()
                .max_height(320.0)
                .show(ui, |ui| {
                    for target in names.iter().filter(|n| **n != name) {
                        if ui.button(target).clicked() {
                            ops.push(Op::Merge(vec![target.clone(), name.clone()]));
                        }
                    }
                });
        });
        ui.separator();
        ui.menu_button("Reference in chat ▸", |ui| {
            for m in Modifier::ALL {
                if ui.button(m.label()).clicked() {
                    ops.push(Op::Ref(
                        Reference::new(Target::Person { name: name.clone() }).with(m),
                    ));
                }
            }
        });
        if ui.button("Avoid this person in plans").clicked() {
            ops.push(Op::Ref(
                Reference::new(Target::Person { name: name.clone() }).with(Modifier::Avoid),
            ));
        }
        ui.separator();
        if ui
            .button("Remove from the map (footage becomes unassigned)")
            .clicked()
        {
            ops.push(Op::Forget(name));
        }
    });
}

/// The focused person's clips and segments.
fn detail(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui, ops: &mut Vec<Op>) {
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    let Some(index) = project.index.as_ref() else {
        widgets::muted(ui, ctx.theme, "Index the footage to see who is in it.");
        return;
    };
    let model = state.cache.get(project, state.order);
    let Some(focus) = state.focus.name() else {
        widgets::muted(
            ui,
            ctx.theme,
            "Select a person to see their clips. Drag segments onto a card to move them; Space starts the confirm queue.",
        );
        return;
    };
    let Some(group) = model.group(focus) else {
        widgets::muted(ui, ctx.theme, "That person is no longer in the map.");
        return;
    };
    let ink = theme::rgb(ctx.theme.text);
    let muted = theme::rgb(ctx.theme.text_muted);

    // Header.
    ui.horizontal(|ui| {
        ui.label(text(group.label(), theme::TITLE_SIZE - 2.0, ink).strong());
        if let Some(name) = &group.name
            && ui
                .small_button(text("Rename", META, muted))
                .on_hover_text("Rename (Enter)")
                .clicked()
        {
            ops.push(Op::StartRename(name.clone()));
        }
    });
    let mut meta = format!("{} clips · {} segments", group.clips.len(), group.segs);
    let props = view::props_line(group);
    if !props.is_empty() {
        meta.push_str(" · ");
        meta.push_str(&props);
    }
    ui.label(text(meta, META, muted));
    if !group.sample.is_empty() {
        ui.label(text(format!("“{}”", group.sample), META, muted).italics());
    }
    if state.selected.len() > 1 {
        ui.horizontal(|ui| {
            ui.label(text(
                format!("{} people selected", state.selected.len()),
                META,
                ink,
            ));
            if ui
                .small_button(text(
                    format!(
                        "Merge into {}",
                        state.selected.first().map_or("", String::as_str)
                    ),
                    META,
                    ink,
                ))
                .on_hover_text("Ctrl+M")
                .clicked()
            {
                ops.push(Op::Merge(state.selected.clone()));
            }
        });
    }
    if !state.picked.is_empty() {
        ui.horizontal(|ui| {
            ui.label(text(format!("{} picked", state.picked.len()), META, ink));
            if ui
                .small_button(text("Split off… (Ctrl+Shift+M)", META, ink))
                .clicked()
            {
                ops.push(Op::Open(Dialog::NewPerson {
                    segs: state.picked.clone(),
                    name: String::new(),
                }));
            }
        });
    }
    ui.add_space(theme::SPACE_XS);

    let order: Vec<usize> = group.all_segs().collect();
    let names: Vec<String> = model.groups.iter().filter_map(|g| g.name.clone()).collect();
    let playing = match ctx.player.source() {
        crate::gui::player::Source::Clip { path, t0, .. } => Some((path.clone(), *t0)),
        crate::gui::player::Source::Empty
        | crate::gui::player::Source::Sequence { .. }
        | crate::gui::player::Source::Audio { .. } => None,
    };
    egui::ScrollArea::vertical()
        .id_salt("people-detail")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for row in &group.clips {
                let caption = format!(
                    "{} · {} segment{}",
                    row.clip,
                    row.segs.len(),
                    if row.segs.len() == 1 { "" } else { "s" }
                );
                let header = view::clip_header(ui, ctx.theme, &caption)
                    .on_hover_text("Drag onto a person to move this clip's segments");
                header.dnd_set_drag_payload(DragSegs(row.segs.clone()));
                if header.clicked() {
                    state.picked.clone_from(&row.segs);
                }
                for &i in &row.segs {
                    let look = view::RowLook {
                        picked: state.picked.contains(&i),
                        playing: playing.as_ref().is_some_and(|(p, t0)| {
                            index.segments.get(i).is_some_and(|s| {
                                p.as_path() == row.path.as_path()
                                    && (f64::from(s.seg.start) - t0).abs() < 0.01
                            })
                        }),
                    };
                    let response =
                        view::segment_row(ui, ctx.thumbs, ctx.theme, index, model, i, &look);
                    let payload = if state.picked.contains(&i) {
                        state.picked.clone()
                    } else {
                        vec![i]
                    };
                    response.dnd_set_drag_payload(DragSegs(payload.clone()));
                    if response.clicked() {
                        ops.push(Op::Segment(i, ui.input(|inp| inp.modifiers)));
                    }
                    response.context_menu(|ui| {
                        segment_menu(ui, group, &names, &payload, index, ops);
                    });
                }
                ui.add_space(theme::SPACE_XS);
            }
            if order.is_empty() {
                widgets::muted(ui, ctx.theme, "No indexed footage under this name.");
            }
        });
}

/// The right-click menu of a segment (or the picked segments).
fn segment_menu(
    ui: &mut Ui,
    group: &Group,
    names: &[String],
    segs: &[usize],
    index: &crate::index::Index,
    ops: &mut Vec<Op>,
) {
    let n = segs.len();
    ui.label(text(
        format!("{n} segment{}", if n == 1 { "" } else { "s" }),
        META,
        ui.visuals().weak_text_color(),
    ));
    ui.menu_button("Move to ▸", |ui| {
        egui::ScrollArea::vertical()
            .max_height(360.0)
            .show(ui, |ui| {
                if ui.button("New person…").clicked() {
                    ops.push(Op::Open(Dialog::NewPerson {
                        segs: segs.to_vec(),
                        name: String::new(),
                    }));
                }
                if group.name.is_some() && ui.button("Unassigned").clicked() {
                    ops.push(Op::Move(segs.to_vec(), None));
                }
                ui.separator();
                for name in names.iter().filter(|n| Some(*n) != group.name.as_ref()) {
                    if ui.button(name).clicked() {
                        ops.push(Op::Move(segs.to_vec(), Some(name.clone())));
                    }
                }
            });
    });
    if ui
        .button("Split off as a new person… (Ctrl+Shift+M)")
        .clicked()
    {
        ops.push(Op::Open(Dialog::NewPerson {
            segs: segs.to_vec(),
            name: String::new(),
        }));
    }
    ui.separator();
    ui.menu_button("Reference in chat ▸", |ui| {
        for m in Modifier::ALL {
            if ui.button(m.label()).clicked() {
                for &i in segs {
                    if let Some(s) = index.segments.get(i) {
                        ops.push(Op::Ref(
                            Reference::new(Target::Segment { id: s.id.clone() }).with(m),
                        ));
                    }
                }
            }
        }
    });
}

/// The confirm queue screen.
fn confirm_queue(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut Ui, ops: &mut Vec<Op>) {
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    let Some(index) = project.index.as_ref() else {
        ops.push(Op::StopQueue);
        return;
    };
    let model = state.cache.get(project, state.order);
    let Some(queue) = state.queue.as_mut() else {
        return;
    };
    let ink = theme::rgb(ctx.theme.text);
    let muted = theme::rgb(ctx.theme.text_muted);
    let Some(question) = queue.question(&project.people, model) else {
        ui.add_space(theme::SPACE_MD);
        ui.label(text("Queue done", theme::TITLE_SIZE, ink).strong());
        ui.label(text(
            format!(
                "{} answered, {} filed. Nothing else has a plausible guess.",
                queue.answered, queue.confirmed
            ),
            SMALL,
            muted,
        ));
        if widgets::button(ui, ctx.theme, "Back to people", widgets::Emphasis::Primary).clicked() {
            ops.push(Op::StopQueue);
        }
        return;
    };
    // Show the clip in the viewer when the question changes.
    if state.asked.as_deref() != Some(question.item.clip.as_str()) {
        state.asked = Some(question.item.clip.clone());
        if let Some(&first) = model
            .clip_segs
            .get(&question.item.clip)
            .and_then(|s| s.first())
            && let Some(seg) = index.segments.get(first)
        {
            let end = model
                .clip_segs
                .get(&question.item.clip)
                .and_then(|s| s.last())
                .and_then(|&i| index.segments.get(i))
                .map_or(seg.seg.end, |s| s.seg.end);
            ctx.player.open_clip(
                model.path(&question.item.clip),
                f64::from(seg.seg.start),
                f64::from(end),
                seg.seg.cx,
            );
        }
    }

    ui.label(text(
        format!(
            "Confirm · {} left · {} answered · Esc to stop",
            queue.left(),
            queue.answered
        ),
        META,
        muted,
    ));
    ui.add_space(theme::SPACE_XS);
    let palette = ctx.theme;
    let strip = |thumbs: &mut crate::gui::thumbs::Thumbs, ui: &mut Ui, segs: &[usize]| {
        let width = ui.available_width();
        let cell = ((width - 3.0 * 2.0) / 4.0).max(10.0);
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(width, cell * 1.25), egui::Sense::hover());
        for (k, &i) in segs.iter().take(4).enumerate() {
            let r = egui::Rect::from_min_size(
                rect.min + egui::vec2(k as f32 * (cell + 2.0), 0.0),
                egui::vec2(cell, cell * 1.25),
            );
            view::frame(ui, thumbs, palette, r, index, model, i);
        }
    };
    // The clip in question.
    let clip_segs: Vec<usize> = model
        .clip_segs
        .get(&question.item.clip)
        .cloned()
        .unwrap_or_default();
    let mut best: Vec<usize> = clip_segs.clone();
    best.sort_by(|a, b| {
        let q = |i: &usize| index.segments.get(*i).map_or(0.0, |s| s.q);
        q(b).total_cmp(&q(a))
    });
    strip(ctx.thumbs, ui, &best);
    let profile = model.profiles.iter().find(|p| p.clip == question.item.clip);
    ui.label(text(
        format!(
            "{} · {}",
            question.item.clip,
            question
                .item
                .current
                .as_deref()
                .map_or_else(|| "unassigned".to_owned(), |c| format!("filed as {c}"))
        ),
        META,
        muted,
    ));
    if let Some(p) = profile {
        ui.label(text(p.sample.as_str(), SMALL, ink));
    }
    ui.add_space(theme::SPACE_SM);
    ui.label(
        text(
            format!("Is this {}?", question.guess.0),
            theme::TITLE_SIZE,
            ink,
        )
        .strong(),
    );
    ui.label(text(
        format!(
            "likeness {:.0}% (caption words, prop, recorded nearby)",
            question.guess.1 * 100.0
        ),
        META,
        muted,
    ));
    if let Some(guess) = model.group(Some(&question.guess.0)) {
        strip(ctx.thumbs, ui, &guess.strip);
        if !guess.sample.is_empty() {
            ui.label(text(format!("“{}”", guess.sample), META, muted).italics());
        }
    }
    ui.add_space(theme::SPACE_SM);
    ui.horizontal(|ui| {
        if widgets::button(ui, ctx.theme, "Yes  Y", widgets::Emphasis::Primary).clicked() {
            ops.push(Op::Answer(Answer::Yes));
        }
        if widgets::button(ui, ctx.theme, "No  N", widgets::Emphasis::Secondary).clicked() {
            ops.push(Op::Answer(Answer::No));
        }
        if widgets::button(ui, ctx.theme, "Not sure  S", widgets::Emphasis::Secondary).clicked() {
            ops.push(Op::Answer(Answer::Skip));
        }
    });
    if !question.others.is_empty() {
        ui.add_space(theme::SPACE_XS);
        ui.horizontal_wrapped(|ui| {
            ui.label(text("Or it is:", META, muted));
            for (name, score) in &question.others {
                if ui
                    .small_button(text(format!("{name} {:.0}%", score * 100.0), META, ink))
                    .clicked()
                {
                    ops.push(Op::AnswerAs(name.clone()));
                }
            }
        });
    }
}

/// The open dialog, if any.
fn dialog(ctx: &mut PageCtx<'_>, state: &mut State, ui: &Ui, ops: &mut Vec<Op>) {
    let Some(dialog) = state.dialog.as_mut() else {
        return;
    };
    let clips: Vec<String> = ctx
        .project
        .as_ref()
        .and_then(|p| p.index.as_ref())
        .map(|i| i.clips.iter().map(|c| c.id.clone()).collect())
        .unwrap_or_default();
    let names: Vec<String> = ctx
        .project
        .as_ref()
        .map(|p| {
            p.people
                .all_names()
                .into_iter()
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut close = false;
    let modal = egui::Modal::new(egui::Id::unique("people-dialog")).show(ui.ctx(), |ui| {
        ui.set_width(300.0);
        match dialog {
            Dialog::NewPerson { segs, name } => {
                widgets::title(ui, ctx.theme, "New person");
                widgets::muted(
                    ui,
                    ctx.theme,
                    &format!(
                        "Move {} segment{} to a new name.",
                        segs.len(),
                        if segs.len() == 1 { "" } else { "s" }
                    ),
                );
                let edit = ui
                    .add(
                        egui::TextEdit::singleline(name)
                            .hint_text("e.g. white-mask")
                            .desired_width(f32::INFINITY),
                    )
                    .accessible_name("New person's name");
                if !edit.has_focus() && !edit.lost_focus() {
                    edit.request_focus();
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.horizontal(|ui| {
                    let ok = widgets::button(ui, ctx.theme, "Create", widgets::Emphasis::Primary)
                        .clicked();
                    if (ok || enter) && !name.trim().is_empty() {
                        ops.push(Op::Move(segs.clone(), Some(name.trim().to_owned())));
                        close = true;
                    }
                    if widgets::button(ui, ctx.theme, "Cancel", widgets::Emphasis::Secondary)
                        .clicked()
                    {
                        close = true;
                    }
                });
            }
            Dialog::AssignRange {
                clip,
                from,
                to,
                name,
            } => {
                widgets::title(ui, ctx.theme, "Assign range");
                widgets::muted(
                    ui,
                    ctx.theme,
                    "Writes clip@from-to = name: shots starting in that range show this person.",
                );
                egui::Grid::new("people-range")
                    .num_columns(2)
                    .spacing([theme::SPACE_SM, theme::SPACE_XS])
                    .show(ui, |ui| {
                        let label = ui.label("Clip");
                        egui::ComboBox::from_id_salt("people-range-clip")
                            .selected_text(clip.as_str())
                            .width(180.0)
                            .show_ui(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(300.0)
                                    .show(ui, |ui| {
                                        for c in &clips {
                                            ui.selectable_value(clip, c.clone(), c);
                                        }
                                    });
                            })
                            .response
                            .labelled_by(label.id);
                        ui.end_row();
                        let label = ui.label("From (s)");
                        ui.add(egui::DragValue::new(from).speed(0.1).range(0.0..=f32::MAX))
                            .labelled_by(label.id);
                        ui.end_row();
                        let label = ui.label("To (s)");
                        ui.add(egui::DragValue::new(to).speed(0.1).range(0.0..=f32::MAX))
                            .labelled_by(label.id);
                        ui.end_row();
                        let label = ui.label("Person");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(name).desired_width(130.0))
                                .labelled_by(label.id);
                            egui::ComboBox::from_id_salt("people-range-name")
                                .selected_text("▾")
                                .width(24.0)
                                .show_ui(ui, |ui| {
                                    egui::ScrollArea::vertical()
                                        .max_height(300.0)
                                        .show(ui, |ui| {
                                            for n in &names {
                                                ui.selectable_value(name, n.clone(), n);
                                            }
                                        });
                                })
                                .response
                                .accessible_name("Pick a known person");
                        });
                        ui.end_row();
                    });
                let valid = !clip.is_empty() && *to > *from && !name.trim().is_empty();
                ui.horizontal(|ui| {
                    let assign = ui.add_enabled(valid, egui::Button::new("Assign"));
                    if assign.clicked() {
                        ops.push(Op::AssignRange(
                            clip.clone(),
                            *from,
                            *to,
                            name.trim().to_owned(),
                        ));
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
                ui.label(text(
                    crate::people::range_key(clip, *from, *to),
                    META,
                    theme::rgb(ctx.theme.text_muted),
                ));
            }
        }
    });
    if close || modal.should_close() {
        state.dialog = None;
    }
}

/// The Assign range dialog, filled from the picked segment or the viewer.
fn range_dialog(ctx: &PageCtx<'_>, state: &State) -> Dialog {
    let index = ctx.project.as_ref().and_then(|p| p.index.as_ref());
    let picked = state
        .picked
        .first()
        .and_then(|&i| index.and_then(|x| x.segments.get(i)));
    let name = match &state.focus {
        Focus::Person(n) => n.clone(),
        Focus::None | Focus::Unassigned => String::new(),
    };
    match picked {
        Some(seg) => Dialog::AssignRange {
            clip: seg.clip.clone(),
            from: seg.seg.start,
            to: seg.seg.end,
            name,
        },
        None => Dialog::AssignRange {
            clip: index
                .and_then(|i| i.clips.first())
                .map(|c| c.id.clone())
                .unwrap_or_default(),
            from: 0.0,
            to: 1.0,
            name,
        },
    }
}

/// Applies one op.
fn apply(ctx: &mut PageCtx<'_>, state: &mut State, op: Op) {
    match op {
        Op::Person(name, mods) => select_person(ctx, state, name, mods),
        Op::Segment(i, mods) => select_segment(ctx, state, i, mods),
        Op::StartRename(name) => {
            state.renaming = Some((name.clone(), name));
        }
        Op::Rename(from, to) => {
            state.renaming = None;
            let to = to.trim().to_owned();
            if to.is_empty() || to == from {
                return;
            }
            let merging = ctx
                .project
                .as_ref()
                .is_some_and(|p| p.people.all_names().contains(to.as_str()));
            let label = if merging {
                format!("Rename {from} → {to} (merged)")
            } else {
                format!("Rename {from} → {to}")
            };
            edit(ctx, state, &label, |p| {
                p.rename(&from, &to);
            });
            rename_selection(ctx, state, &from, &to);
        }
        Op::Merge(names) => {
            let Some((into, rest)) = names.split_first() else {
                return;
            };
            if rest.is_empty() {
                return;
            }
            let label = format!("Merge {} into {into}", rest.join(", "));
            let into = into.clone();
            let rest = rest.to_vec();
            edit(ctx, state, &label, |p| {
                p.merge(&rest, &into);
            });
            state.selected = vec![into.clone()];
            state.focus = Focus::Person(into.clone());
            *ctx.selection = vec![Reference::new(Target::Person { name: into })];
        }
        Op::Move(segs, to) => move_segments(ctx, state, &segs, to),
        Op::Forget(name) => {
            edit(ctx, state, &format!("Remove {name}"), |p| {
                p.forget(&name);
            });
            state.selected.retain(|n| *n != name);
            if state.focus == Focus::Person(name) {
                state.focus = Focus::Unassigned;
            }
        }
        Op::Open(dialog) => {
            let dialog = match dialog {
                Dialog::NewPerson { segs, name } if name.is_empty() => {
                    let base = match &state.focus {
                        Focus::Person(n) => n.clone(),
                        Focus::None | Focus::Unassigned => "person".to_owned(),
                    };
                    let name = ctx
                        .project
                        .as_ref()
                        .map(|p| edit::fresh_name(&p.people, &base))
                        .unwrap_or(base);
                    Dialog::NewPerson { segs, name }
                }
                other @ (Dialog::NewPerson { .. } | Dialog::AssignRange { .. }) => other,
            };
            state.dialog = Some(dialog);
        }
        Op::AssignRange(clip, from, to, name) => {
            let label = format!(
                "Assign {} to {name}",
                crate::people::range_key(&clip, from, to)
            );
            let spans = ctx.project.as_ref().and_then(|p| {
                let index = p.index.as_ref()?;
                Some(state.cache.get(p, state.order).spans(&clip, index))
            });
            let spans = spans.unwrap_or_default();
            edit(ctx, state, &label, |p| {
                p.assign_range(&clip, from, to, &name);
                p.tidy(&clip, &spans);
            });
        }
        Op::Ref(reference) => ctx.actions.push(Action::AddRef(reference)),
        Op::Undo => {
            if let Some(project) = ctx.project.as_mut() {
                state.status = state.history.undo(project);
            }
        }
        Op::Redo => {
            if let Some(project) = ctx.project.as_mut() {
                state.status = state.history.redo(project);
            }
        }
        Op::StartQueue => {
            if let Some(project) = ctx.project.as_ref() {
                let model = state.cache.get(project, state.order);
                let queue = Queue::build(&project.people, model);
                state.status = format!("{} clips to confirm", queue.left());
                state.queue = Some(queue);
                state.asked = None;
            }
        }
        Op::Answer(answer) => answer_queue(ctx, state, answer, None),
        Op::AnswerAs(name) => answer_queue(ctx, state, Answer::Yes, Some(name)),
        Op::StopQueue => {
            if let Some(q) = state.queue.take() {
                state.status = format!("Confirmed {} of {} answered", q.confirmed, q.answered);
            }
            state.asked = None;
        }
        Op::Draft => {
            if let Some(project) = ctx.project.as_ref() {
                ctx.jobs.spawn(
                    "Draft people",
                    crate::commands::Cmd::People {
                        work: project.work.clone(),
                        force: false,
                    },
                );
                state.status = "Drafting people.toml from the captions…".to_owned();
            }
        }
    }
}

/// Runs `change` over the map, saves, reloads, and reports in the status.
fn edit(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    label: &str,
    change: impl FnOnce(&mut crate::people::People),
) {
    if let Some(project) = ctx.project.as_mut() {
        state.status = state.history.apply(project, label, change);
    }
}

/// Moves segments to a person and keeps the view on where they went.
fn move_segments(ctx: &mut PageCtx<'_>, state: &mut State, segs: &[usize], to: Option<String>) {
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    let Some(index) = project.index.as_ref() else {
        return;
    };
    let model = state.cache.get(project, state.order);
    let mut next = project.people.clone();
    edit::move_segments(&mut next, model, index, segs, to.as_deref());
    let label = format!(
        "Move {} segment{} to {}",
        segs.len(),
        if segs.len() == 1 { "" } else { "s" },
        to.as_deref().unwrap_or("Unassigned")
    );
    edit(ctx, state, &label, |p| *p = next);
    state.picked.clear();
}

/// Answers the queue's question (optionally filing under another name).
fn answer_queue(ctx: &mut PageCtx<'_>, state: &mut State, answer: Answer, name: Option<String>) {
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    let model = state.cache.get(project, state.order);
    let Some(queue) = state.queue.as_mut() else {
        return;
    };
    let Some(question) = queue.question(&project.people, model) else {
        return;
    };
    let filed = queue.answer(&question, answer);
    let Some(to) = name.or(filed) else {
        return;
    };
    // File the clip's segments that are unassigned or under the doubted name.
    let segs: Vec<usize> = model
        .clip_segs
        .get(&question.item.clip)
        .map(|all| {
            all.iter()
                .copied()
                .filter(|&i| {
                    project
                        .index
                        .as_ref()
                        .and_then(|x| x.segments.get(i))
                        .is_some_and(|s| {
                            let now = project.people.entry_at(&s.clip, s.seg.start);
                            now.is_none()
                                || now == Some(s.clip.as_str())
                                || now == question.item.current.as_deref()
                        })
                })
                .collect()
        })
        .unwrap_or_default();
    move_segments(ctx, state, &segs, Some(to));
}

/// Selects a person card.
fn select_person(ctx: &mut PageCtx<'_>, state: &mut State, name: Option<String>, mods: Modifiers) {
    state.picked.clear();
    state.seg_anchor = None;
    let Some(name) = name else {
        state.selected.clear();
        state.focus = Focus::Unassigned;
        ctx.selection.clear();
        return;
    };
    if mods.command {
        if let Some(pos) = state.selected.iter().position(|n| *n == name) {
            state.selected.remove(pos);
        } else {
            state.selected.push(name.clone());
        }
    } else if mods.shift
        && let Some(anchor) = state.anchor.clone()
        && let Some(project) = ctx.project.as_ref()
    {
        let model = state.cache.get(project, state.order);
        let order: Vec<&str> = model
            .groups
            .iter()
            .filter_map(|g| g.name.as_deref())
            .collect();
        let a = order.iter().position(|n| *n == anchor);
        let b = order.iter().position(|n| *n == name);
        if let (Some(a), Some(b)) = (a, b) {
            let (lo, hi) = (a.min(b), a.max(b));
            state.selected = order
                .iter()
                .skip(lo)
                .take(hi - lo + 1)
                .map(|n| (*n).to_owned())
                .collect();
        }
    } else {
        state.selected = vec![name.clone()];
    }
    if !mods.shift {
        state.anchor = Some(name.clone());
    }
    state.focus = Focus::Person(name);
    *ctx.selection = state
        .selected
        .iter()
        .map(|n| Reference::new(Target::Person { name: n.clone() }))
        .collect();
}

/// Selects a segment row and loads it in the viewer.
fn select_segment(ctx: &mut PageCtx<'_>, state: &mut State, i: usize, mods: Modifiers) {
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    let Some(index) = project.index.as_ref() else {
        return;
    };
    let model = state.cache.get(project, state.order);
    if mods.command {
        if let Some(pos) = state.picked.iter().position(|&j| j == i) {
            state.picked.remove(pos);
        } else {
            state.picked.push(i);
        }
    } else if mods.shift
        && let Some(anchor) = state.seg_anchor
        && let Some(group) = state.focus.name().and_then(|f| model.group(f))
    {
        let order: Vec<usize> = group.all_segs().collect();
        let a = order.iter().position(|&j| j == anchor);
        let b = order.iter().position(|&j| j == i);
        if let (Some(a), Some(b)) = (a, b) {
            let (lo, hi) = (a.min(b), a.max(b));
            state.picked = order.into_iter().skip(lo).take(hi - lo + 1).collect();
        }
    } else {
        state.picked = vec![i];
    }
    if !mods.shift {
        state.seg_anchor = Some(i);
    }
    if let Some(seg) = index.segments.get(i) {
        ctx.player.open_clip(
            model.path(&seg.clip),
            f64::from(seg.seg.start),
            f64::from(seg.seg.end),
            seg.seg.cx,
        );
    }
    *ctx.selection = state
        .picked
        .iter()
        .filter_map(|&j| index.segments.get(j))
        .map(|s| Reference::new(Target::Segment { id: s.id.clone() }))
        .collect();
}

/// Keeps the selection pointing at a renamed person.
fn rename_selection(ctx: &mut PageCtx<'_>, state: &mut State, from: &str, to: &str) {
    for n in &mut state.selected {
        if n == from {
            to.clone_into(n);
        }
    }
    state.selected.dedup();
    if state.focus == Focus::Person(from.to_owned()) {
        state.focus = Focus::Person(to.to_owned());
    }
    for r in ctx.selection.iter_mut() {
        if let Target::Person { name } = &mut r.target
            && name == from
        {
            to.clone_into(name);
        }
    }
}

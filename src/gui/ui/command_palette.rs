// ============================================================================
// Module:       gui::ui::command_palette
// Description:  The command palette: Ctrl+K, type to filter, arrows to move,
//               Enter to run, Escape to close.
//
// Dependencies: egui (Modal, TextEdit); crate::command_palette (matching),
//               super::{theme, widgets}, crate::gui::app
// ============================================================================

//! The command palette.
//!
//! Every [`Action`] the app has, reachable by typing. The list is built
//! fresh each time the palette is drawn from the app's own state, so a new
//! theme or page shows up here without anyone remembering to add it, and
//! running an entry pushes the same `Action` a button or shortcut would.

use super::theme::{SPACE_MD, SPACE_SM};
use super::widgets;
use crate::command_palette::Command;
use crate::gui::app::{Action, App};
use egui::{Key, Ui};

/// The palette's width.
const WIDTH: f32 = 520.0;

/// How many matches are shown at once.
const VISIBLE: usize = 8;

/// The palette's state between frames.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// Whether it is showing.
    pub open: bool,
    /// What has been typed.
    pub query: String,
    /// The highlighted match, by position in the filtered list.
    pub selected: usize,
}

impl State {
    /// Opens the palette with an empty query.
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
    }
}

/// Every command the palette offers, built from the app's current state:
/// each runnable row of the command table with its keys under the preset
/// in force, then the settings and themes.
#[must_use]
pub fn commands(app: &App) -> Vec<Command<Action>> {
    let mut commands: Vec<Command<Action>> = super::keymap::COMMANDS
        .iter()
        .filter(|command| command.id != "palette")
        .filter_map(|command| {
            let run = command.run?;
            Some(Command::new(command.label, command.hint(app.keymap), run()))
        })
        .collect();
    commands.push(Command::new(
        if app.reduce_motion {
            "Turn animations on"
        } else {
            "Reduce motion"
        },
        "Settings",
        Action::ToggleMotion,
    ));
    for theme in app.catalog.themes() {
        commands.push(Command::new(
            format!("Switch theme to {}", theme.name),
            "Theme",
            Action::SetTheme(theme.id.clone()),
        ));
    }
    for preset in super::keymap::Preset::ALL {
        commands.push(Command::new(
            format!("Use {} keyboard shortcuts", preset.label()),
            "Keyboard",
            Action::SetKeymap(preset),
        ));
    }
    commands
}

/// Draws the palette while it is open; Ctrl+K opens it from anywhere.
pub fn draw(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    let chord = super::keymap::chord("palette", app.keymap);
    let toggle = chord.is_some_and(|chord| ctx.input_mut(|input| input.consume_shortcut(&chord)));
    if toggle && !app.settings_open {
        if app.commands.open {
            app.commands.open = false;
        } else {
            app.commands.open();
        }
    }
    if !app.commands.open {
        return;
    }
    let all = commands(app);
    let matches = crate::command_palette::filter(&all, &app.commands.query);
    let (up, down, enter) = ctx.input_mut(|input| {
        (
            input.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            input.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            input.consume_key(egui::Modifiers::NONE, Key::Enter),
        )
    });
    let state = &mut app.commands;
    let last = matches.len().saturating_sub(1);
    if down {
        state.selected = (state.selected + 1).min(last);
    }
    if up {
        state.selected = state.selected.saturating_sub(1);
    }
    state.selected = state.selected.min(last);

    let theme = app.theme.clone();
    let id = egui::Id::unique("command-palette");
    let mut chosen = None;
    let area =
        egui::Modal::default_area(id).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 96.0));
    let response = egui::Modal::new(id)
        .area(area)
        .frame(egui::Frame::NONE)
        .backdrop_color(super::theme::translucent(theme.app, 140))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(super::theme::rgb(theme.raised))
                .stroke(egui::Stroke::new(1.0, super::theme::rgb(theme.border)))
                .corner_radius(egui::CornerRadius::same(super::theme::RADIUS_LG))
                .inner_margin(super::theme::margin(SPACE_MD))
                .show(ui, |ui| {
                    ui.set_width(WIDTH);
                    let field = egui_material3::MaterialInput::outlined(&mut state.query)
                        .label("Command")
                        .placeholder("Type a command")
                        .density(-1)
                        .fill_width(true)
                        .show(ui)
                        .response;
                    field.request_focus();
                    if field.changed() {
                        state.selected = 0;
                    }
                    ui.add_space(SPACE_SM);
                    if matches.is_empty() {
                        widgets::muted(ui, &theme, "No command matches.");
                    }
                    let first = state.selected.saturating_sub(VISIBLE - 1);
                    let shown: Vec<(usize, &Command<Action>)> = matches
                        .iter()
                        .copied()
                        .enumerate()
                        .skip(first)
                        .take(VISIBLE)
                        .collect();
                    let clicked = rows(ui, &shown, state.selected);
                    if let Some((_, command)) = clicked.and_then(|i| shown.get(i)) {
                        chosen = Some(command.action.clone());
                    }
                });
        });
    if enter {
        chosen = chosen.or_else(|| {
            matches
                .get(state.selected)
                .map(|command| command.action.clone())
        });
    }
    if let Some(action) = chosen {
        state.open = false;
        actions.push(action);
    } else if response.should_close() {
        state.open = false;
    }
}

/// The matches on screen as a Material 3 list: each its title, its hint
/// as trailing text, the highlighted one (`selected`, an index into all
/// the matches) with the list's selected fill. Returns the position in
/// `shown` of the row clicked, if one was. Up, Down and Enter are read by
/// the palette before the list sees them, so they move the highlight
/// whatever has focus.
fn rows(ui: &mut Ui, shown: &[(usize, &Command<Action>)], selected: usize) -> Option<usize> {
    use egui_material3::{ListItem, MaterialList};
    let list = shown.iter().fold(
        MaterialList::new()
            .id(ui.make_persistent_id("command-palette-rows"))
            .dividers(false)
            .fill_width(true),
        |list, (index, command)| {
            list.item(
                ListItem::new(command.title.clone())
                    .trailing_text(command.hint.clone())
                    .dense(true)
                    .row_height(super::theme::CONTROL_HEIGHT)
                    .selected(*index == selected),
            )
        },
    );
    list.show(ui).items.iter().position(egui::Response::clicked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::app::Page;

    #[test]
    fn every_page_is_reachable_from_the_palette() {
        let ctx = egui::Context::default();
        let app = App::new(&ctx, crate::config::Config::default(), None);
        let all = commands(&app);
        for page in Page::ALL {
            assert!(
                all.iter().any(|command| command.action == Action::Go(page)),
                "{page:?} has no command"
            );
        }
    }
}

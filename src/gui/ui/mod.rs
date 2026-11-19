// ============================================================================
// Module:       gui::ui
// Description:  The one draw entry point, the frame's layout order, the pages,
//               and the keyboard shortcuts.
//
// Dependencies: egui; every sibling module; crate::gui::app
// ============================================================================

//! Drawing one frame.
//!
//! [`draw`] runs in full every frame and lays the window out in a fixed
//! order, because egui's panels claim space in the order they are shown:
//!
//! 1. **The top bar** and **the status bar**, full width.
//! 2. **The chat** (left), the height between them, or its rail.
//! 3. **The timeline** along the foot of what is left, then the browser,
//!    inspector and export beside the viewer, which takes the rest.
//! 4. **Overlays**: settings, the command palette, the shortcut sheet.
//!
//! Drawing never changes the app directly: it returns [`Action`]s, which
//! [`App::apply`](crate::gui::app::App::apply) carries out after the frame.
//! A shortcut, a button and a menu entry for the same thing are therefore
//! the same code path.
//!
//! Shortcuts come from one table, [`keymap::COMMANDS`], and are read
//! before the page draws, and not at all while a modal is open, so a
//! keystroke aimed at a dialog cannot also act behind it.

pub mod chrome;
pub mod command_palette;
pub mod icon;
pub mod keymap;
pub mod layout;
pub mod modal;
pub mod motion;
pub mod settings;
pub mod shortcut_sheet;
pub mod theme;
pub mod topbar;
pub mod widgets;
pub mod workspace;

use crate::gui::app::{Action, App};
use egui::Ui;
use theme::{SPACE_LG, SPACE_MD, SPACE_SM};

/// What a control shows in its icon slot: an icon from the drawn set.
pub type Glyph = crate::icon::Icon;

/// The glyph for each purpose, so call sites name the purpose and not the
/// shape.
pub mod glyph {
    use super::Glyph;
    use crate::icon::Icon;

    /// The Night page.
    pub const NIGHT: Glyph = Icon::Folder;
    /// The Review page.
    pub const REVIEW: Glyph = Icon::Grid;
    /// The People page.
    pub const PEOPLE: Glyph = Icon::Sparkle;
    /// The Music page.
    pub const MUSIC: Glyph = Icon::Activity;
    /// The Plan page.
    pub const PLAN: Glyph = Icon::Layers;
    /// The Deliver page.
    pub const DELIVER: Glyph = Icon::Bolt;
    /// Informational notes.
    pub const ABOUT: Glyph = Icon::Info;
    /// Settings.
    pub const SETTINGS: Glyph = Icon::Settings;
    /// The sidebar toggle.
    pub const MENU: Glyph = Icon::Menu;
    /// Close.
    pub const CLOSE: Glyph = Icon::Close;
    /// The command palette.
    pub const COMMAND: Glyph = Icon::Command;
    /// Search.
    pub const SEARCH: Glyph = Icon::Search;
    /// A card: speed.
    pub const BOLT: Glyph = Icon::Bolt;
    /// A card: structure.
    pub const LAYERS: Glyph = Icon::Layers;
    /// A card: polish.
    pub const SPARKLE: Glyph = Icon::Sparkle;
    /// A card: motion.
    pub const ACTIVITY: Glyph = Icon::Activity;
    /// A card: storage.
    pub const FOLDER: Glyph = Icon::Folder;
    /// A card: you.
    pub const SUN: Glyph = Icon::Sun;
}

/// The size a glyph is drawn at in the sidebar and on buttons.
#[must_use]
pub const fn icon_size() -> f32 {
    icon::NAV
}

/// Paints `glyph` into a control's icon slot, in the control's ink.
pub fn paint_glyph(ui: &Ui, slot: &widgets::Slot, glyph: Glyph) {
    draw_glyph(ui.painter(), slot.icon, glyph, slot.ink);
}

/// Paints `glyph` into `rect` in `color`.
pub fn draw_glyph(painter: &egui::Painter, rect: egui::Rect, glyph: Glyph, color: egui::Color32) {
    icon::paint(painter, rect, glyph, color);
}

/// Draws one frame and returns what the user asked for.
pub fn draw(app: &mut App, ui: &mut Ui) -> Vec<Action> {
    let mut actions = Vec::new();
    // Installed before anything paints, so nothing is drawn under the
    // previous frame's theme.
    theme::apply(ui.ctx(), &app.theme);
    motion::set_reduced(app.reduce_motion);

    let blocked = app.settings_open || app.commands.open || app.shortcuts_open;
    if !blocked {
        shortcuts(ui, app.keymap, &mut actions);
    }

    workspace::draw(app, ui, &mut actions, blocked);

    settings::draw(app, ui.ctx(), &mut actions);
    command_palette::draw(app, ui.ctx(), &mut actions);
    shortcut_sheet::draw(app, ui.ctx(), &mut actions);
    actions
}

/// Reads the global keyboard shortcuts into actions, from the command
/// table under the preset in force. See [`keymap`].
fn shortcuts(ui: &Ui, preset: keymap::Preset, actions: &mut Vec<Action>) {
    // A shortcut must not fire while a text field has focus.
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    // While the palette is open this does not run, and the palette reads
    // its own key to close; with a text field focused, it reads it to open.
    let fired = ui.ctx().input_mut(|input| keymap::dispatch(input, preset));
    actions.extend(fired);
}

/// The about page: the mark, the version and the shortcuts.
pub fn about(app: &App, ui: &mut Ui) {
    let theme = &app.theme;
    crate::gui::icons::paint_brand(ui, 72.0);
    ui.add_space(SPACE_MD);
    widgets::title(ui, theme, crate::NAME);
    widgets::muted(
        ui,
        theme,
        &format!("{} Version {}.", crate::TAGLINE, env!("CARGO_PKG_VERSION")),
    );
    ui.add_space(SPACE_MD);
    widgets::title(ui, theme, "Keyboard");
    egui::Grid::new("shortcuts")
        .num_columns(2)
        .spacing([SPACE_LG, SPACE_SM])
        .show(ui, |ui| {
            for command in keymap::COMMANDS {
                let keys = command.hint(app.keymap);
                if keys.is_empty() {
                    continue;
                }
                ui.label(
                    egui::RichText::new(keys)
                        .monospace()
                        .color(theme::rgb(theme.accent)),
                );
                ui.label(egui::RichText::new(command.label).color(theme::rgb(theme.text_muted)));
                ui.end_row();
            }
        });
}

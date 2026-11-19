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
//! 1. **The header** (top) and **the status bar** (bottom), full width.
//! 2. **The sidebar** (left), the height between them.
//! 3. **The page**, in whatever is left.
//! 4. **Overlays**: the settings card and anything else modal.
//!
//! Drawing never changes the app directly: it returns [`Action`]s, which
//! [`App::apply`](crate::gui::app::App::apply) carries out after the frame.
//! A shortcut, a button and a menu entry for the same thing are therefore
//! the same code path.
//!
//! Shortcuts are read before the page draws, and not at all while a modal
//! is open, so a keystroke aimed at a dialog cannot also act behind it.

pub mod chrome;
pub mod command_palette;
pub mod icon;
pub mod modal;
pub mod motion;
pub mod settings;
pub mod theme;
pub mod widgets;

use crate::gui::app::{Action, App, Page};
use egui::{Key, Ui};
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

    let blocked = app.settings_open;
    let blocked = blocked || app.commands.open;
    if !blocked {
        shortcuts(ui, &mut actions);
    }

    chrome::header(app, ui, &mut actions);
    chrome::status_bar(app, ui);
    chrome::sidebar(app, ui, &mut actions);

    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::rgb(app.theme.app)))
        .show(ui, |ui| {
            enter_page(ui, app.page);
            let page = app.page;
            let crate::gui::app::App {
                theme,
                project,
                jobs,
                thumbs,
                player,
                pages,
                config,
                refs,
                ..
            } = app;
            let mut ctx = crate::gui::pages::PageCtx {
                theme,
                project,
                jobs,
                thumbs,
                player,
                config,
                refs,
                actions: &mut actions,
                blocked,
            };
            match page {
                Page::Night => crate::gui::pages::night::draw(&mut ctx, &mut pages.night, ui),
                Page::Review => crate::gui::pages::review::draw(&mut ctx, &mut pages.review, ui),
                Page::People => crate::gui::pages::people::draw(&mut ctx, &mut pages.people, ui),
                Page::Music => crate::gui::pages::music::draw(&mut ctx, &mut pages.music, ui),
                Page::Plan => {
                    crate::gui::pages::plan::draw(
                        &mut ctx,
                        &mut pages.plan,
                        &mut pages.assistant,
                        ui,
                    );
                }
                Page::Deliver => crate::gui::pages::deliver::draw(&mut ctx, &mut pages.deliver, ui),
            }
        });

    settings::draw(app, ui.ctx(), &mut actions);
    command_palette::draw(app, ui.ctx(), &mut actions);
    actions
}

/// Reads the keyboard shortcuts into actions. See [`SHORTCUTS`].
fn shortcuts(ui: &Ui, actions: &mut Vec<Action>) {
    // A shortcut must not fire while a text field has focus.
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    ui.ctx().input(|input| {
        let command = input.modifiers.command;
        if command && input.key_pressed(Key::Comma) {
            actions.push(Action::OpenSettings);
        }
        if command && input.key_pressed(Key::O) {
            actions.push(Action::PickProject);
        }
        if command && input.key_pressed(Key::B) {
            actions.push(Action::ToggleSidebar);
        }
        let numbers = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
        ];
        for (key, page) in numbers.into_iter().zip(Page::ALL) {
            if command && input.key_pressed(key) {
                actions.push(Action::Go(page));
            }
        }
    });
}

/// The shortcuts, as data, so the about page and the handler above cannot
/// drift apart without the test below noticing.
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+,", "Open settings"),
    ("Ctrl+B", "Collapse or expand the sidebar"),
    ("Ctrl+1…6", "Switch page"),
    ("Ctrl+O", "Open a night"),
    ("Ctrl+K", "Open the command palette"),
    ("Esc", "Close a dialog"),
];

/// Fades and lifts a page into place as it arrives, so a page change reads
/// as navigation rather than as a flicker. Left alone once finished: a
/// permanently non-opaque `Ui` costs a layer every frame for nothing.
fn enter_page(ui: &mut Ui, page: Page) {
    /// How far the content rises as it arrives.
    const RISE: f32 = 8.0;
    let ctx = ui.ctx().clone();
    // Keyed on the page, so arriving at another restarts the animation.
    let progress = widgets::arrival(&ctx, egui::Id::unique("page-enter"), egui::Id::unique(page));
    if progress >= 1.0 {
        return;
    }
    ui.set_opacity(progress);
    ui.add_space(RISE * (1.0 - progress));
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
            for (keys, what) in SHORTCUTS {
                ui.label(
                    egui::RichText::new(*keys)
                        .monospace()
                        .color(theme::rgb(theme.accent)),
                );
                ui.label(egui::RichText::new(*what).color(theme::rgb(theme.text_muted)));
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_a_numbered_shortcut() {
        assert_eq!(
            Page::ALL.len(),
            6,
            "update the Ctrl+1…6 shortcut and SHORTCUTS"
        );
    }

    #[test]
    fn every_shortcut_is_documented_once() {
        let mut keys: Vec<&str> = SHORTCUTS.iter().map(|(keys, _)| *keys).collect();
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "a shortcut is listed twice");
    }
}

// ============================================================================
// Module:       gui::pages
// Description:  The six pipeline pages, the context they are drawn with, and
//               the per-page state the app keeps for them.
//
// Dependencies: egui; crate::{project, jobs, config, theme}, crate::gui
// ============================================================================

//! The workspace's regions (see `gui::ui::workspace`): `night` is the Home
//! screen, `review` / `people` / `music` are the browser's tabs, `viewer`
//! the centre, `inspector` the right pane, `plan` the timeline at the foot,
//! `deliver` the export panel.
//!
//! Every page is a module with a `State` (kept across frames and page
//! switches) and `draw(ctx, state, ui)`. A page reaches everything shared
//! through [`PageCtx`]: the open project, background jobs, thumbnails, the
//! viewer, the theme, and the app-wide action list. Pages never reach into
//! one another; anything cross-page goes through an [`Action`] or through
//! the files the project reads (see `docs/GUI_CONTRACTS.md`).

pub(crate) mod deliver;
pub(crate) mod inspector;
pub(crate) mod music;
pub(crate) mod night;
pub(crate) mod people;
pub(crate) mod plan;
pub(crate) mod review;
pub(crate) mod viewer;

use crate::gui::app::Action;
use crate::gui::player::Player;
use crate::gui::thumbs::Thumbs;
use crate::gui::ui::{theme, widgets};
use crate::jobs::Jobs;
use crate::project::Project;
use crate::theme::Palette;

/// Everything a page may use while it draws.
pub(crate) struct PageCtx<'a> {
    /// The palette to draw in.
    pub(crate) theme: &'a Palette,
    /// The open night, if any.
    pub(crate) project: &'a mut Option<Project>,
    /// Background pipeline jobs.
    pub(crate) jobs: &'a mut Jobs,
    /// The thumbnail cache.
    pub(crate) thumbs: &'a mut Thumbs,
    /// The shared viewer.
    pub(crate) player: &'a mut Player,
    /// Saved preferences.
    pub(crate) config: &'a mut crate::config::Config,
    /// References staged for the assistant (read-only here; add with
    /// `Action::AddRef`, consume in the assistant).
    pub(crate) refs: &'a mut Vec<crate::refs::Reference>,
    /// What is selected app-wide, as references. Pages replace it when the
    /// user selects something; the inspector shows it; `R` stages it for chat.
    pub(crate) selection: &'a mut Vec<crate::refs::Reference>,
    /// The top bar's search text.
    pub(crate) search: &'a mut String,
    /// Messages handed to the chat (`Action::ChatInbox`); the assistant drains it.
    pub(crate) chat_inbox: &'a mut Vec<String>,
    /// App-wide actions to apply after the frame.
    pub(crate) actions: &'a mut Vec<Action>,
    /// Whether a modal is open (pages must not read shortcuts then).
    pub(crate) blocked: bool,
    /// The keyboard preset in force: regions read their keys with
    /// `keymap::chord(id, ctx.keymap)` (or `keymap::pressed`).
    pub(crate) keymap: crate::gui::ui::keymap::Preset,
    /// The page on show (the guide offers a different next step in the editor).
    pub(crate) page: crate::gui::app::Page,
}

impl PageCtx<'_> {
    /// Whether the page may read single-key shortcuts this frame: no modal
    /// and no focused text field.
    pub(crate) fn keys_free(&self, ui: &egui::Ui) -> bool {
        !self.blocked && !ui.ctx().egui_wants_keyboard_input()
    }
}

/// The state of every page.
#[derive(Debug, Default)]
pub(crate) struct States {
    /// Night page.
    pub(crate) night: night::State,
    /// Review page.
    pub(crate) review: review::State,
    /// People page.
    pub(crate) people: people::State,
    /// Music page.
    pub(crate) music: music::State,
    /// Plan page.
    pub(crate) plan: plan::State,
    /// Deliver page.
    pub(crate) deliver: deliver::State,
    /// The assistant panel.
    pub(crate) assistant: crate::gui::assistant::State,
    /// The inspector.
    pub(crate) inspector: inspector::State,
    /// The viewer.
    pub(crate) viewer: viewer::State,
}

/// A page heading and the line under it.
pub(crate) fn heading(ctx: &PageCtx<'_>, ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(
        egui::RichText::new(title)
            .font(theme::font(theme::HEADING_SIZE + 4.0))
            .color(theme::rgb(ctx.theme.text))
            .strong(),
    );
    widgets::muted(ui, ctx.theme, subtitle);
    ui.add_space(theme::SPACE_MD);
}

/// What a page shows when no night is open.
pub(crate) fn no_project(ctx: &mut PageCtx<'_>, ui: &mut egui::Ui) {
    heading(
        ctx,
        ui,
        "No project open",
        "Open a project: its footage folder, or the _ave folder inside it.",
    );
    if widgets::button(ui, ctx.theme, "Open project…", widgets::Emphasis::Primary).clicked() {
        ctx.actions.push(Action::PickProject);
    }
}

// ============================================================================
// Module:       gui::pages
// Description:  The six pipeline pages, the context they are drawn with, and
//               the per-page state the app keeps for them.
//
// Dependencies: egui; crate::{project, jobs, config, theme}, crate::gui
// ============================================================================

//! The pages, one per pipeline stage: Night → Review → People → Music →
//! Plan → Deliver.
//!
//! Every page is a module with a `State` (kept across frames and page
//! switches) and `draw(ctx, state, ui)`. A page reaches everything shared
//! through [`PageCtx`]: the open project, background jobs, thumbnails, the
//! viewer, the theme, and the app-wide action list. Pages never reach into
//! one another; anything cross-page goes through an [`Action`] or through
//! the files the project reads (see `docs/GUI_CONTRACTS.md`).

pub(crate) mod deliver;
pub(crate) mod music;
pub(crate) mod night;
pub(crate) mod people;
pub(crate) mod plan;
pub(crate) mod review;

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
    /// App-wide actions to apply after the frame.
    pub(crate) actions: &'a mut Vec<Action>,
    /// Whether a modal is open (pages must not read shortcuts then).
    pub(crate) blocked: bool,
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
        "No night open",
        "Open a night's folder (its footage folder or its _ave work folder) to start.",
    );
    if widgets::button(ui, ctx.theme, "Open night…", widgets::Emphasis::Primary).clicked() {
        ctx.actions.push(Action::PickProject);
    }
}

/// A page that has not been built yet.
pub(crate) fn placeholder(ctx: &mut PageCtx<'_>, ui: &mut egui::Ui, title: &str, what: &str) {
    egui::Frame::new()
        .inner_margin(theme::margin(theme::SPACE_XL))
        .show(ui, |ui| {
            if ctx.project.is_none() {
                no_project(ctx, ui);
                return;
            }
            heading(ctx, ui, title, what);
        });
}

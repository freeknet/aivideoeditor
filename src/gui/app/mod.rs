// ============================================================================
// Module:       gui::app
// Description:  The application state: which page is open, what is selected,
//               the active theme, and the actions that change them.
//
// Dependencies: egui (the Context the runtime hands over); crate::theme,
//               crate::gui::ui (drawing)
// ============================================================================

//! The application.
//!
//! [`App`] is driven by whichever runtime owns the window (`gui::run`). It
//! uses **egui only** — never eframe or winit types — so the runtime can be
//! swapped without the app noticing. A runtime does three things with it:
//!
//! 1. builds it once with [`App::new`];
//! 2. each frame, either calls [`App::ui`] with the `Context` from inside
//!    `Context::run_pass`, or [`App::show`] with the root `Ui` it was given;
//! 3. on a normal quit, calls [`App::on_exit`].
//!
//! Besides the shell (page, sidebar, theme), the app owns what every page
//! shares: the open night ([`crate::project::Project`]), background jobs,
//! the thumbnail cache, the viewer, and each page's own state.
//!
//! Every change of state a user can cause goes through [`Action`] and
//! [`App::apply`], so a shortcut, a button, the tray menu and the command
//! palette cannot drift apart in what "open settings" means.

use crate::gui::ui::{self, glyph};
use crate::theme::Palette;

/// A page of the app: one per pipeline stage, in sidebar order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Page {
    /// Open a night, run the pipeline, see what each step produced.
    #[default]
    Night,
    /// The shot log as a culling grid.
    Review,
    /// Who is in which clip.
    People,
    /// The track: beat grid, sections, drops.
    Music,
    /// The edit.
    Plan,
    /// Renders and hand-off.
    Deliver,
}

impl Page {
    /// Every page, in sidebar order.
    pub const ALL: [Self; 6] = [
        Self::Night,
        Self::Review,
        Self::People,
        Self::Music,
        Self::Plan,
        Self::Deliver,
    ];

    /// The name the page is shown under.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Night => "Night",
            Self::Review => "Review",
            Self::People => "People",
            Self::Music => "Music",
            Self::Plan => "Plan",
            Self::Deliver => "Deliver",
        }
    }

    /// The stable name the config persists. Never rename one.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Night => "night",
            Self::Review => "review",
            Self::People => "people",
            Self::Music => "music",
            Self::Plan => "plan",
            Self::Deliver => "deliver",
        }
    }

    /// The page with this stable name.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|page| page.key() == key)
    }

    /// The glyph the sidebar shows beside the label.
    #[must_use]
    pub fn glyph(self) -> ui::Glyph {
        match self {
            Self::Night => glyph::NIGHT,
            Self::Review => glyph::REVIEW,
            Self::People => glyph::PEOPLE,
            Self::Music => glyph::MUSIC,
            Self::Plan => glyph::PLAN,
            Self::Deliver => glyph::DELIVER,
        }
    }
}

/// The browser's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BrowserTab {
    /// The shot log: segments to find, judge and pick.
    #[default]
    Shots,
    /// Performers.
    People,
    /// The track and its beat grid.
    Music,
}

impl BrowserTab {
    /// Every tab, in order.
    pub const ALL: [Self; 3] = [Self::Shots, Self::People, Self::Music];

    /// The tab's label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Shots => "Shots",
            Self::People => "People",
            Self::Music => "Music",
        }
    }
}

/// Everything a user can do, from any input.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Open a page.
    Go(Page),
    /// Open the command palette.
    OpenCommands,
    /// Open the settings card.
    OpenSettings,
    /// Close the settings card.
    CloseSettings,
    /// Collapse or expand the sidebar.
    ToggleSidebar,
    /// Switch to the theme with this id.
    SetTheme(String),
    /// Switch animation off or back on.
    ToggleMotion,
    /// Ask for a folder and open it as a night.
    PickProject,
    /// Open this folder as a night (a footage folder or an `_ave` work folder).
    OpenProject(std::path::PathBuf),
    /// Re-read the open night from disk.
    ReloadProject,
    /// Close the open night.
    CloseProject,
    /// Stage a reference for the assistant composer (from any page).
    AddRef(crate::refs::Reference),
    /// Drop every staged reference.
    ClearRefs,
    /// Show or hide the chat panel.
    ToggleChat,
    /// Show or hide the inspector.
    ToggleInspector,
    /// Show or hide the export panel.
    ToggleExport,
    /// Bring the window to the front.
    Focus,
    /// Close the window and quit.
    Quit,
}

/// The application state.
pub struct App {
    /// The page on screen.
    pub page: Page,
    /// Whether the sidebar shows icons only.
    pub sidebar_collapsed: bool,
    /// Whether the settings card is open.
    pub settings_open: bool,
    /// The palette everything is drawn in.
    pub theme: Palette,
    /// Every theme the picker offers.
    pub catalog: crate::theme::Catalog,
    /// Whether animation is switched off.
    pub reduce_motion: bool,
    /// The preferences, kept current so [`App::on_exit`] can save them.
    pub config: crate::config::Config,
    /// The command palette's state.
    pub commands: ui::command_palette::State,
    /// The open night.
    pub(crate) project: Option<crate::project::Project>,
    /// Background pipeline jobs.
    pub(crate) jobs: crate::jobs::Jobs,
    /// The thumbnail cache.
    pub(crate) thumbs: crate::gui::thumbs::Thumbs,
    /// The shared viewer.
    pub(crate) player: crate::gui::player::Player,
    /// Every page's own state.
    pub(crate) pages: crate::gui::pages::States,
    /// The last error to show in the status bar.
    pub(crate) notice: Option<String>,
    /// References staged for the assistant, in the order they were added.
    pub(crate) refs: Vec<crate::refs::Reference>,
    /// What is selected anywhere (browser, timeline, inspector).
    pub(crate) selection: Vec<crate::refs::Reference>,
    /// The top bar's search text (the Shots tab filters by it).
    pub(crate) search: String,
    /// Whether the Home screen replaces the workspace.
    pub(crate) home: bool,
    /// Whether the export panel is docked.
    pub(crate) export_open: bool,
    /// Whether the chat panel is shown.
    pub(crate) chat_open: bool,
    /// Whether the inspector is shown.
    pub(crate) inspector_open: bool,
    /// The browser's tab.
    pub(crate) browser_tab: BrowserTab,
    /// Whether the "reference as…" chooser is open.
    pub(crate) ref_popup: bool,
}

impl App {
    /// Builds the app, before the first frame.
    #[must_use]
    pub fn new(
        ctx: &egui::Context,
        config: crate::config::Config,
        open: Option<std::path::PathBuf>,
    ) -> Self {
        if let Some(family) = config.font.as_deref() {
            // A missing family is not an error: the bundled faces stay.
            let _ = crate::gui::font::install(ctx, family);
        }
        let catalog = crate::theme::Catalog::load();
        let theme = catalog.get(config.theme.as_deref());
        let mut app = Self {
            page: config
                .page
                .as_deref()
                .and_then(Page::from_key)
                .unwrap_or_default(),
            sidebar_collapsed: config.sidebar_collapsed.unwrap_or(false),
            settings_open: false,
            theme,
            catalog,
            reduce_motion: config.reduce_motion.unwrap_or(false),
            config,
            commands: ui::command_palette::State::default(),
            project: None,
            jobs: crate::jobs::Jobs::default(),
            thumbs: crate::gui::thumbs::Thumbs::default(),
            player: crate::gui::player::Player::default(),
            pages: crate::gui::pages::States::default(),
            notice: None,
            refs: Vec::new(),
            selection: Vec::new(),
            search: String::new(),
            home: false,
            export_open: false,
            chat_open: true,
            inspector_open: true,
            browser_tab: BrowserTab::Shots,
            ref_popup: false,
        };
        app.jobs.set_context(ctx);
        let reopen = open.or_else(|| {
            app.config
                .last_project
                .as_ref()
                .map(std::path::PathBuf::from)
                .filter(|p| p.exists())
        });
        if let Some(path) = reopen {
            app.open_project(&path);
        }
        app
    }

    /// Draws one frame, given the `Context` inside `Context::run_pass`.
    ///
    /// Takes the pass's root `Ui` through `Context::root_ui`, so it must not
    /// be called from inside `Context::run_ui` or another `root_ui` call —
    /// a runtime that already holds the root `Ui` calls [`App::show`].
    pub fn ui(&mut self, ctx: &egui::Context) {
        ctx.root_ui(|ui| self.show(ui));
    }

    /// Draws one frame into the root `Ui`.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.remember_window_size(ui.ctx());
        if !self.jobs.poll().is_empty()
            && let Some(project) = &mut self.project
        {
            project.reload();
        }
        self.thumbs.pump(ui.ctx());
        let actions = ui::draw(self, ui);
        for action in actions {
            self.apply(ui.ctx(), action);
        }
    }

    /// Carries out one action.
    pub fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Go(page) => {
                self.page = page;
                self.home = page == Page::Night;
                match page {
                    Page::Review => self.browser_tab = BrowserTab::Shots,
                    Page::People => self.browser_tab = BrowserTab::People,
                    Page::Music => self.browser_tab = BrowserTab::Music,
                    Page::Deliver => self.export_open = true,
                    Page::Night | Page::Plan => {}
                }
            }
            Action::ToggleChat => self.chat_open = !self.chat_open,
            Action::ToggleInspector => self.inspector_open = !self.inspector_open,
            Action::ToggleExport => self.export_open = !self.export_open,
            Action::OpenSettings => self.settings_open = true,
            Action::CloseSettings => self.settings_open = false,
            Action::OpenCommands => self.commands.open(),
            Action::ToggleSidebar => self.sidebar_collapsed = !self.sidebar_collapsed,
            Action::SetTheme(id) => self.theme = self.catalog.get(Some(&id)),
            Action::ToggleMotion => self.reduce_motion = !self.reduce_motion,
            Action::Focus => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::PickProject => {
                let picked = rfd::FileDialog::new()
                    .set_title("Open a night (its footage folder or its _ave folder)")
                    .pick_folder();
                if let Some(path) = picked {
                    self.open_project(&path);
                }
            }
            Action::OpenProject(path) => self.open_project(&path),
            Action::AddRef(reference) => {
                if !self.refs.contains(&reference) {
                    self.refs.push(reference);
                }
                self.chat_open = true;
            }
            Action::ClearRefs => self.refs.clear(),
            Action::ReloadProject => {
                if let Some(project) = &mut self.project {
                    project.reload();
                }
            }
            Action::CloseProject => {
                self.project = None;
                self.thumbs.set_work(None);
                self.config.last_project = None;
            }
        }
        self.remember_choices();
    }

    /// Opens `path` as a night. A footage folder (one holding video files)
    /// opens its night's `_ave` work folder beside it, creating it.
    pub(crate) fn open_project(&mut self, path: &std::path::Path) {
        let work = crate::project::work_dir_for(path);
        match crate::project::Project::open(&work) {
            Ok(mut project) => {
                if project.settings.footage.is_none() && work != path {
                    project.settings.footage = Some(path.to_path_buf());
                    let _ = project.save_settings();
                }
                self.thumbs.set_work(Some(&work));
                let key = work.display().to_string();
                self.config.last_project = Some(key.clone());
                let recent = self.config.recent_projects.get_or_insert_with(Vec::new);
                recent.retain(|p| *p != key);
                recent.insert(0, key);
                recent.truncate(12);
                self.project = Some(project);
                self.notice = None;
            }
            Err(e) => self.notice = Some(format!("Could not open {}: {e:#}", path.display())),
        }
    }

    /// The colour the runtime clears the window to before egui paints: the
    /// theme's own background, so a resize never flashes white.
    #[must_use]
    pub fn clear_color(&self) -> [f32; 4] {
        egui::Rgba::from(ui::theme::rgb(self.theme.app)).to_array()
    }

    /// Called by the runtime on a normal quit.
    pub fn on_exit(&mut self) {
        // The window is already closing, so a failed save has nowhere to
        // be shown; the next start simply uses the previous preferences.
        let _ = crate::config::save(&self.config);
    }

    /// Copies what the user has chosen into the config, so it is saved.
    fn remember_choices(&mut self) {
        self.config.page = Some(self.page.key().to_string());
        self.config.sidebar_collapsed = Some(self.sidebar_collapsed);
        self.config.theme = Some(self.theme.id.clone());
        self.config.reduce_motion = Some(self.reduce_motion);
    }

    /// Keeps the restored window size in the config, so the next start
    /// opens the window the size it was left.
    fn remember_window_size(&mut self, ctx: &egui::Context) {
        let (rect, maximized) =
            ctx.input(|input| (input.viewport().inner_rect, input.viewport().maximized));
        if maximized == Some(true) {
            return;
        }
        let usable = |size: &egui::Vec2| {
            size.x.is_finite() && size.y.is_finite() && size.x > 0.0 && size.y > 0.0
        };
        if let Some(size) = rect.map(|rect| rect.size()).filter(usable) {
            self.config.window_size = Some([size.x, size.y]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh app, built the way the runtime builds it.
    fn app(ctx: &egui::Context) -> App {
        App::new(ctx, crate::config::Config::default(), None)
    }

    /// Runs `frames` headless frames, the way a runtime would.
    fn run(ctx: &egui::Context, app: &mut App, frames: usize) {
        for _ in 0..frames {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1180.0, 760.0),
                )),
                ..Default::default()
            };
            ctx.run_pass(input, |ctx| app.ui(ctx))
                .drop_without_applying_deltas();
        }
    }

    #[test]
    fn page_keys_are_unique_and_round_trip() {
        for page in Page::ALL {
            assert_eq!(Page::from_key(page.key()), Some(page), "{page:?}");
        }
        assert_eq!(Page::from_key("no-such-page"), None);
    }

    #[test]
    fn every_page_and_the_settings_card_draw_headless() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        for page in Page::ALL {
            app.apply(&ctx, Action::Go(page));
            run(&ctx, &mut app, 2);
            assert_eq!(app.page, page);
        }
        app.apply(&ctx, Action::OpenSettings);
        run(&ctx, &mut app, 2);
        assert!(app.settings_open, "settings stays open until dismissed");
    }

    #[test]
    fn toggling_the_sidebar_twice_restores_it() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let before = app.sidebar_collapsed;
        app.apply(&ctx, Action::ToggleSidebar);
        assert_ne!(app.sidebar_collapsed, before);
        app.apply(&ctx, Action::ToggleSidebar);
        assert_eq!(app.sidebar_collapsed, before);
    }

    #[test]
    fn switching_theme_changes_the_palette() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let other = app
            .catalog
            .themes()
            .iter()
            .find(|theme| theme.id != app.theme.id)
            .map(|theme| theme.id.clone())
            .ok_or_else(|| anyhow::anyhow!("the catalog has only one theme"))?;
        app.apply(&ctx, Action::SetTheme(other.clone()));
        assert_eq!(app.theme.id, other);
        run(&ctx, &mut app, 1);
        Ok(())
    }
}

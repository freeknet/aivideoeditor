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
            Self::Night => "Home",
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

/// A stage of a project, in the order they happen: what the guide strip
/// under the top bar shows, and what [`Action::RunStep`] asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// The clips the project is cut from.
    Footage,
    /// Indexing, captions and who's who.
    Analyse,
    /// The track and its beat map.
    Music,
    /// The drafted plans.
    FirstCut,
    /// Watching and changing the cut.
    Review,
    /// Rendering and delivering.
    Export,
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
    /// Set how many clips analysis reads at once.
    SetAnalysisJobs(usize),
    /// Set the folder ffmpeg is in (empty: find it on `PATH`).
    SetFfmpegDir(String),
    /// Set the captioning model's paths: llama.cpp's folder, the model file
    /// and its projector file (`None` leaves one as it is, `Some("")` clears it).
    SetCaptionPaths([Option<String>; 3]),
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
    /// Hand a message to the chat (e.g. a batch of timeline comments); the
    /// assistant shows it in its composer or sends it, and opens the panel.
    ChatInbox(String),
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
    /// Switch the workspace to a view mode.
    SetViewMode(ui::layout::ViewMode),
    /// Fold a panel to its rail, or unfold it.
    TogglePanel(ui::layout::Region),
    /// Unfold a panel and bring it forward (Selects' Shift+1…4).
    FocusRegion(ui::layout::Region),
    /// Mention the app-wide selection in chat (`R`).
    StageSelection,
    /// Ask which intent to mention the selection with (`Shift+R`).
    ChooseSelectionIntent,
    /// Show or hide the keyboard shortcut sheet (`?`).
    ToggleShortcuts,
    /// Switch the keyboard to a preset.
    SetKeymap(ui::keymap::Preset),
    /// Put the keyboard in the top bar's search box.
    FocusSearch,
    /// Lift shots (0-based) from a plan through the timeline's undoable,
    /// versioned edit path, whether or not the timeline is open (the
    /// inspector's Shot list sends it on Delete).
    LiftShots {
        /// The plan (file stem).
        plan: String,
        /// The shots, 0-based.
        shots: Vec<usize>,
    },
    /// Do the next thing for a stage of the project (analyse, map the
    /// music, draft the first cut…) from whatever screen asked; the guide
    /// strip runs it through Home's pipeline queue.
    RunStep(Stage),
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
    /// The view mode, and which panels are folded and how big, per mode.
    pub(crate) layout: ui::layout::Layout,
    /// The keyboard preset in force.
    pub(crate) keymap: ui::keymap::Preset,
    /// Whether the shortcut sheet is open.
    pub(crate) shortcuts_open: bool,
    /// Whether the search view is open under the search bar.
    pub(crate) search_open: bool,
    /// Set by [`Action::FocusSearch`]; the top bar takes it next frame.
    pub(crate) focus_search: bool,
    /// The top bar's recent searches, newest first.
    pub(crate) recent_searches: Vec<String>,
    /// The browser's tab.
    pub(crate) browser_tab: BrowserTab,
    /// Whether the "reference as…" chooser is open.
    pub(crate) ref_popup: bool,
    /// Messages handed to the chat by other regions, oldest first; the
    /// assistant takes them.
    pub(crate) chat_inbox: Vec<String>,
}

impl App {
    /// Builds the app, before the first frame.
    #[must_use]
    pub fn new(
        ctx: &egui::Context,
        config: crate::config::Config,
        open: Option<std::path::PathBuf>,
    ) -> Self {
        // A missing family is not an error: the bundled faces stay. The
        // symbol fallback is installed either way.
        let _ = crate::gui::font::setup(ctx, config.font.as_deref());
        let catalog = crate::theme::Catalog::load();
        let theme = catalog.get(config.theme.as_deref());
        // The ffmpeg and GPU checks each run ffmpeg once; done off the UI
        // thread now so the first screen to ask does not stall.
        let _ = std::thread::Builder::new()
            .name("ave-tool-checks".into())
            .spawn(|| {
                // Warming the cached answer; the value is read later.
                let _ = crate::media::ffmpeg_status();
                // Warming the cached answer; the value is read later.
                let _ = crate::media::gpu_decode();
            });
        crate::gui::pages::night::pipeline::set_analysis_jobs(
            config
                .analysis_jobs
                .unwrap_or(crate::gui::pages::night::pipeline::DEFAULT_ANALYSIS_JOBS),
        );
        let layout = ui::layout::Layout::from_config(&config);
        let keymap = config
            .keymap
            .as_deref()
            .and_then(ui::keymap::Preset::from_key)
            .unwrap_or_default();
        let recent_searches = config.recent_searches.clone().unwrap_or_default();
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
            layout,
            keymap,
            shortcuts_open: false,
            search_open: false,
            focus_search: false,
            recent_searches,
            browser_tab: BrowserTab::Shots,
            ref_popup: false,
            chat_inbox: Vec::new(),
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
                use ui::layout::Region;
                match page {
                    Page::Review => self.show_browser(BrowserTab::Shots),
                    Page::People => self.show_browser(BrowserTab::People),
                    Page::Music => self.show_browser(BrowserTab::Music),
                    Page::Deliver => self.export_open = true,
                    Page::Plan => self.layout.set_open(Region::Timeline, true),
                    Page::Night => {}
                }
            }
            Action::ChatInbox(text) => {
                self.chat_inbox.push(text);
                self.layout.set_open(ui::layout::Region::Chat, true);
            }
            Action::ToggleChat => self.layout.toggle(ui::layout::Region::Chat),
            Action::ToggleInspector => self.layout.toggle(ui::layout::Region::Inspector),
            Action::ToggleExport => self.export_open = !self.export_open,
            Action::SetViewMode(mode) => {
                // Choosing the mode already on screen puts it back to its
                // own arrangement, which is the way out of a mess.
                if self.layout.mode == mode {
                    self.layout.reset();
                }
                self.layout.mode = mode;
            }
            Action::TogglePanel(region) => self.layout.toggle(region),
            Action::FocusRegion(region) => {
                self.home = false;
                self.layout.set_open(region, true);
            }
            Action::StageSelection => {
                if !self.selection.is_empty() {
                    for item in self.selection.clone() {
                        if !self.refs.contains(&item) {
                            self.refs.push(item);
                        }
                    }
                    self.layout.set_open(ui::layout::Region::Chat, true);
                }
            }
            Action::ChooseSelectionIntent => self.ref_popup = !self.selection.is_empty(),
            Action::ToggleShortcuts => self.shortcuts_open = !self.shortcuts_open,
            Action::SetKeymap(preset) => self.keymap = preset,
            Action::FocusSearch => {
                self.focus_search = true;
                self.search_open = true;
            }
            Action::LiftShots { plan, shots } => {
                if let Some(project) = self.project.as_mut() {
                    crate::gui::pages::plan::lift_shots(
                        &mut self.pages.plan,
                        project,
                        &mut self.player,
                        &mut self.selection,
                        &plan,
                        &shots,
                    );
                }
            }
            Action::RunStep(stage) => self.pages.night.guide.requests.push(stage),
            Action::OpenSettings => self.settings_open = true,
            Action::CloseSettings => self.settings_open = false,
            Action::OpenCommands => self.commands.open(),
            Action::ToggleSidebar => self.sidebar_collapsed = !self.sidebar_collapsed,
            Action::SetTheme(id) => self.theme = self.catalog.get(Some(&id)),
            Action::ToggleMotion => self.reduce_motion = !self.reduce_motion,
            Action::SetAnalysisJobs(n) => {
                self.config.analysis_jobs = Some(n);
                crate::gui::pages::night::pipeline::set_analysis_jobs(n);
            }
            Action::SetFfmpegDir(dir) => {
                let dir = (!dir.trim().is_empty()).then_some(dir);
                crate::media::set_tool_dir(dir.as_ref().map(std::path::PathBuf::from));
                self.config.ffmpeg_dir = dir;
                if let Err(e) = crate::config::save(&self.config) {
                    self.notice = Some(format!("Could not save the ffmpeg folder: {e:#}"));
                }
            }
            Action::SetCaptionPaths([dir, model, mmproj]) => {
                let keep = |new: Option<String>, old: &mut Option<String>| {
                    if let Some(v) = new {
                        *old = (!v.trim().is_empty()).then_some(v);
                    }
                };
                keep(dir, &mut self.config.caption_llama_dir);
                keep(model, &mut self.config.caption_model);
                keep(mmproj, &mut self.config.caption_mmproj);
                // Captioning reads these from disk: saved now, not on exit.
                if let Err(e) = crate::config::save(&self.config) {
                    self.notice = Some(format!("Could not save the caption settings: {e:#}"));
                }
            }
            Action::Focus => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::PickProject => {
                let picked = rfd::FileDialog::new()
                    .set_title("Open a project (its footage folder or its _ave folder)")
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
                self.layout.set_open(ui::layout::Region::Chat, true);
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
        if work != path
            && !work.join("ave.toml").is_file()
            && let Some(why) = crate::project::footage_problem(path)
        {
            self.notice = Some(format!("Could not open {}: {why}", path.display()));
            return;
        }
        match crate::project::Project::open(&work) {
            Ok(mut project) => {
                if project.settings.footage.is_none() && work != path {
                    project.settings.footage = Some(path.to_path_buf());
                    if let Err(error) = project.save_settings() {
                        log::warn!("could not save the project settings: {error:#}");
                    }
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

    /// Opens the browser on `tab`.
    fn show_browser(&mut self, tab: BrowserTab) {
        self.browser_tab = tab;
        self.layout.set_open(ui::layout::Region::Browser, true);
    }

    /// Remembers a search the user ran, newest first, without repeats.
    pub(crate) fn remember_search(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        self.recent_searches.retain(|q| q != query);
        self.recent_searches.insert(0, query.to_owned());
        self.recent_searches.truncate(8);
        self.config.recent_searches = Some(self.recent_searches.clone());
    }

    /// Called by the runtime on a normal quit.
    pub fn on_exit(&mut self) {
        // Splits are dragged without an action, so they are copied here.
        self.remember_choices();
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
        self.config.keymap = Some(self.keymap.key().to_owned());
        self.layout.write(&mut self.config);
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
    fn every_view_mode_draws_headless_and_is_remembered() {
        use ui::layout::ViewMode;
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        for mode in ViewMode::ALL {
            app.apply(&ctx, Action::SetViewMode(mode));
            run(&ctx, &mut app, 2);
            assert_eq!(app.layout.mode, mode);
            assert_eq!(app.config.view_mode.as_deref(), Some(mode.key()));
        }
        app.apply(&ctx, Action::ToggleShortcuts);
        run(&ctx, &mut app, 2);
        assert!(app.shortcuts_open, "the sheet stays open until dismissed");
    }

    #[test]
    fn a_folded_panel_unfolds_when_something_is_sent_to_it() {
        use ui::layout::Region;
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.apply(&ctx, Action::TogglePanel(Region::Chat));
        assert!(!app.layout.is_open(Region::Chat), "folded");
        assert_eq!(
            app.config.folded_panels.as_deref(),
            Some(&["chat".to_owned()][..]),
            "the fold is saved"
        );
        app.apply(&ctx, Action::ChatInbox("hello".to_owned()));
        assert!(app.layout.is_open(Region::Chat), "a message opens the chat");
    }

    #[test]
    fn the_keymap_preset_is_saved_and_searches_are_remembered_once() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        app.apply(&ctx, Action::SetKeymap(ui::keymap::Preset::FinalCut));
        assert_eq!(app.config.keymap.as_deref(), Some("final-cut"));
        app.remember_search("fire>0.3");
        app.remember_search("dark");
        app.remember_search("fire>0.3");
        assert_eq!(
            app.recent_searches,
            ["fire>0.3", "dark"],
            "newest first, no repeats"
        );
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

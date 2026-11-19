// ============================================================================
// Module:       gui::pages::night
// Description:  Home: the start page (drop zone, first request, recent
//               nights), the get-started wizard, the processing page, and the
//               open night's pipeline, settings and recipe.
//
// Dependencies: egui, rfd; crate::{commands, jobs, project, recipe},
//               crate::gui::pages
// ============================================================================

//! The Home screen, after Selects' start and processing flow.
//!
//! - **No night open: the start page** (`start`): drop this week's footage
//!   folder (or pick it), type or pick a first request, or open a recent
//!   night from the grid.
//! - **A new night: the get-started wizard** (`wizard`): footage → music
//!   and end cards → deliverables → an approval card listing what will run
//!   with time estimates from measured throughput, and "Start analysis".
//! - **While it runs: the processing page** (`processing`): a bar per
//!   step, elapsed and remaining time, Stop, every clip's status, and "Open
//!   editor" as soon as indexing is done. The first request goes to the
//!   chat (`Action::ChatInbox`) once the analysis is queued, or straight
//!   away for a night that is already indexed.
//! - **The night page**: recent nights on the left (`recent`); the pipeline
//!   as a step bar (`pipeline`, one serial job queue shared with the
//!   wizard), the night's settings and recipe (`settings`), and a strip of
//!   the night's clips to skim.
//!
//! See `docs/research/05-selects-ui.md` §1 and §3 and
//! `docs/research/02-ui-and-workflow.md` §6 and §9.3.

pub(crate) mod pipeline;
pub(crate) mod processing;
pub(crate) mod recent;
pub(crate) mod scan;
pub(crate) mod settings;
pub(crate) mod start;
pub(crate) mod wizard;

use std::path::PathBuf;

use super::PageCtx;
use crate::gui::app::{Action, Page};
use crate::gui::ui::{theme, widgets};

/// Which Home view shows while a night is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum View {
    /// The night page.
    #[default]
    Night,
    /// The get-started wizard.
    Wizard,
    /// The processing page.
    Processing,
}

/// The page's own state.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The recent-nights column.
    recent: recent::State,
    /// The step bar and its job queue.
    pipeline: pipeline::State,
    /// The settings and recipe editor.
    settings: settings::State,
    /// The clip being previewed in the strip, by index.
    preview: Option<usize>,
    /// What the last new night carried over, or why it failed; shown once.
    notice: Option<String>,
    /// The start page.
    pub(crate) start: start::State,
    /// The wizard, while a night is being set up.
    pub(crate) wizard: Option<wizard::State>,
    /// The processing page.
    pub(crate) processing: processing::State,
    /// The view on screen while a night is open.
    pub(crate) view: View,
    /// A night opened from the start page, until it has opened.
    arriving: Option<PathBuf>,
}

impl State {
    /// Opens the wizard for the night at `work`.
    fn start_wizard(&mut self, work: PathBuf, previous: Option<PathBuf>, notes: Vec<String>) {
        self.wizard = Some(wizard::State::new(work, previous, notes));
        self.view = View::Wizard;
    }

    /// The first request, taken: it goes to the chat once.
    fn take_prompt(&mut self) -> Option<String> {
        let text = std::mem::take(&mut self.start.prompt);
        (!text.trim().is_empty()).then_some(text)
    }
}

/// Work the page does every frame, shown or not: starting the next queued
/// pipeline job when the one before it finishes.
pub(crate) fn tick(jobs: &mut crate::jobs::Jobs, state: &mut State) {
    state.pipeline.tick(jobs);
}

/// Draws the page into the central area.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    if ctx.project.is_none() {
        full_width(ctx, ui, "home-start", |ctx, ui| start_page(ctx, state, ui));
        return;
    }
    arrived(ctx, state);
    let work = ctx.project.as_ref().map(|p| p.work.clone());
    if state.view == View::Wizard && state.wizard.as_ref().map(|w| &w.work) == work.as_ref() {
        full_width(ctx, ui, "home-wizard", |ctx, ui| {
            wizard_page(ctx, state, ui)
        });
        return;
    }
    if state.view == View::Processing {
        full_width(ctx, ui, "home-processing", |ctx, ui| {
            egui::Frame::new()
                .inner_margin(theme::margin_xy(theme::SPACE_XL, theme::SPACE_LG))
                .show(ui, |ui| {
                    let outcome =
                        processing::draw(ctx, &mut state.processing, &mut state.pipeline, ui);
                    if outcome == Some(processing::Outcome::Back) {
                        state.view = View::Night;
                    }
                });
        });
        return;
    }
    let line = egui::Stroke::new(1.0, theme::rgb(ctx.theme.border));
    egui::Panel::left("home-recent")
        .resizable(true)
        .default_size(300.0)
        .size_range(240.0..=420.0)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(ctx.theme.panel))
                .stroke(line)
                .inner_margin(theme::margin(theme::SPACE_MD)),
        )
        .show(ui, |ui| {
            if let Some(footage) = recent::draw(ctx, &mut state.recent, ui) {
                new_night(ctx, state, &footage);
            }
        });
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::rgb(ctx.theme.app)))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("home-main")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(theme::margin_xy(theme::SPACE_XL, theme::SPACE_LG))
                        .show(ui, |ui| main(ctx, state, ui));
                });
        });
}

/// A full-width Home view in a vertical scroll area.
fn full_width(
    ctx: &mut PageCtx<'_>,
    ui: &mut egui::Ui,
    id: &str,
    add: impl FnOnce(&mut PageCtx<'_>, &mut egui::Ui),
) {
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(theme::rgb(ctx.theme.app)))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(id)
                .auto_shrink([false, false])
                .show(ui, |ui| add(ctx, ui));
        });
}

/// The start page, and what its choices set going.
fn start_page(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    if let Some(notice) = state.notice.clone() {
        egui::Frame::new()
            .inner_margin(theme::margin_xy(theme::SPACE_XL, theme::SPACE_SM))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(theme::rgb(ctx.theme.warning), &notice);
                    if ui.small_button("\u{2715}").clicked() {
                        state.notice = None;
                    }
                });
            });
    }
    match start::draw(ctx, &mut state.start, &mut state.recent, ui) {
        Some(start::Outcome::NewNight(footage)) => new_night(ctx, state, &footage),
        Some(start::Outcome::Open(work)) => {
            ctx.actions.push(Action::OpenProject(work.clone()));
            state.arriving = Some(work);
        }
        None => {}
    }
}

/// Sets a night up for `footage` from the last one and opens the wizard.
fn new_night(ctx: &mut PageCtx<'_>, state: &mut State, footage: &std::path::Path) {
    match recent::set_up(ctx, &state.recent, footage) {
        Ok(made) => {
            let from = made
                .previous
                .as_deref()
                .map(|p| format!(" from {}", crate::recipe::Names::for_work(p).night))
                .unwrap_or_default();
            state.notice = Some(if made.notes.is_empty() {
                format!("New night set up{from}.")
            } else {
                format!("New night set up{from}: {}.", made.notes.join(", "))
            });
            state.arriving = None;
            state.start_wizard(made.work, made.previous, made.notes);
        }
        Err(e) => state.notice = Some(format!("Could not set up the night: {e:#}")),
    }
}

/// When a night opened from the start page arrives: an indexed one goes
/// to the editor with the first request; one never analysed gets the
/// wizard.
fn arrived(ctx: &mut PageCtx<'_>, state: &mut State) {
    let Some(p) = ctx.project.as_ref() else {
        return;
    };
    if state.arriving.as_ref() != Some(&p.work) {
        return;
    }
    state.arriving = None;
    let work = p.work.clone();
    if p.index.is_none() {
        let previous = state.recent.last_night(Some(&work));
        state.start_wizard(work, previous, Vec::new());
    } else if let Some(prompt) = state.take_prompt() {
        ctx.actions.push(Action::ChatInbox(prompt));
        ctx.actions.push(Action::Go(Page::Review));
    } else {
        state.view = View::Night;
    }
}

/// The wizard, and what its outcome sets going.
fn wizard_page(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let Some(wizard) = state.wizard.as_mut() else {
        state.view = View::Night;
        return;
    };
    match wizard::draw(ctx, wizard, ui) {
        Some(wizard::Outcome::Start { jobs, plan }) => {
            let work = wizard.work.clone();
            state.pipeline.enqueue(ctx.jobs, jobs);
            state.processing.begin(work, plan);
            state.wizard = None;
            state.view = View::Processing;
            if let Some(prompt) = state.take_prompt() {
                ctx.actions.push(Action::ChatInbox(prompt));
            }
        }
        Some(wizard::Outcome::Close) => {
            state.wizard = None;
            state.view = View::Night;
        }
        None => {}
    }
}

/// The open night.
fn main(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    header(ctx, state, ui);
    banner(ctx, state, ui);
    state.settings.sync(ctx);
    pipeline::draw(ctx, &mut state.pipeline, &state.settings, ui);
    ui.add_space(theme::SPACE_XL);
    settings::draw(ctx, &mut state.settings, ui);
    ui.add_space(theme::SPACE_XL);
    clips(ctx, state, ui);
}

/// "Not analysed yet: get started", or "analysing: view progress".
fn banner(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let Some((work, indexed)) = ctx
        .project
        .as_ref()
        .map(|p| (p.work.clone(), p.index.is_some()))
    else {
        return;
    };
    let busy = state.pipeline.active();
    if indexed && !busy {
        return;
    }
    let theme = ctx.theme;
    egui::Frame::new()
        .fill(theme::rgb(theme.accent_soft))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(theme::margin(theme::SPACE_MD))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(if busy {
                        "Analysis is running."
                    } else {
                        "This night has not been analysed yet."
                    })
                    .color(theme::rgb(theme.text)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if busy {
                        if widgets::button(ui, theme, "View progress", widgets::Emphasis::Primary)
                            .clicked()
                        {
                            state.view = View::Processing;
                        }
                    } else if widgets::button(ui, theme, "Get started", widgets::Emphasis::Primary)
                        .on_hover_text("Footage, music, deliverables, then approve the analysis")
                        .clicked()
                    {
                        let previous = state.recent.last_night(Some(&work));
                        state.start_wizard(work.clone(), previous, Vec::new());
                    }
                });
            });
        });
    ui.add_space(theme::SPACE_LG);
}

/// The night's name, where it lives, and the way into the workspace.
fn header(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let Some((name, work)) = ctx.project.as_ref().map(|p| (p.name(), p.work.clone())) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(&name)
                    .font(theme::font(theme::HEADING_SIZE + 4.0))
                    .color(theme::rgb(ctx.theme.text))
                    .strong(),
            );
            widgets::muted(ui, ctx.theme, &work.display().to_string());
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, ctx.theme, "Open editor", widgets::Emphasis::Primary)
                .on_hover_text("The workspace: shots, viewer, timeline (Ctrl+2)")
                .clicked()
            {
                ctx.actions.push(Action::Go(Page::Review));
            }
            if widgets::button(ui, ctx.theme, "Export…", widgets::Emphasis::Secondary).clicked() {
                ctx.actions.push(Action::Go(Page::Deliver));
            }
            if widgets::button(ui, ctx.theme, "Reveal", widgets::Emphasis::Secondary)
                .on_hover_text("Show the work folder in Explorer")
                .clicked()
            {
                super::deliver::files::open_folder(&work);
            }
        });
    });
    if let Some(notice) = state.notice.clone() {
        ui.add_space(theme::SPACE_SM);
        let mut dismiss = false;
        egui::Frame::new()
            .fill(theme::rgb(ctx.theme.accent_soft))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS))
            .inner_margin(theme::margin(theme::SPACE_SM))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&notice).color(theme::rgb(ctx.theme.text)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        dismiss = ui.small_button("✕").clicked();
                    });
                });
            });
        if dismiss {
            state.notice = None;
        }
    }
    if let Some(p) = ctx.project.as_ref() {
        for w in &p.warnings {
            ui.colored_label(theme::rgb(ctx.theme.warning), w);
        }
    }
    ui.add_space(theme::SPACE_LG);
}

/// A strip of the night's clips: click one to skim it in the viewer.
fn clips(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let Some(index) = ctx.project.as_ref().and_then(|p| p.index.as_ref()) else {
        return;
    };
    let clips: Vec<(String, PathBuf, f64)> = index
        .clips
        .iter()
        .map(|c| (c.id.clone(), PathBuf::from(&c.path), c.probe.duration))
        .collect();
    widgets::title(ui, ctx.theme, &format!("Clips · {}", clips.len()));
    ui.add_space(theme::SPACE_XS);
    let placeholder = theme::rgb(ctx.theme.raised);
    egui::ScrollArea::horizontal()
        .id_salt("night-clips")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (i, (id, path, duration)) in clips.iter().enumerate() {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(128.0, 72.0), egui::Sense::click());
                    if ui.is_rect_visible(rect) {
                        ctx.thumbs
                            .paint(ui, rect, path, duration * 0.5, 0.5, placeholder);
                    }
                    if state.preview == Some(i) {
                        ui.painter().rect_stroke(
                            rect,
                            egui::CornerRadius::same(4),
                            egui::Stroke::new(2.0, theme::rgb(ctx.theme.accent)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if response.on_hover_text(id).clicked() {
                        state.preview = Some(i);
                        ctx.player.open_clip(path, 0.0, *duration, 0.5);
                        ctx.player.play();
                    }
                }
            });
        });
    let loaded = matches!(ctx.player.source(), crate::gui::player::Source::Clip { .. });
    if state.preview.is_some() && loaded {
        ui.add_space(theme::SPACE_SM);
        let width = ui.available_width().min(640.0);
        let background = theme::rgb(ctx.theme.raised);
        ctx.player.show(
            ui,
            ctx.thumbs,
            egui::vec2(width, width * 9.0 / 16.0),
            16.0 / 9.0,
            background,
        );
        ctx.player.transport(ui);
        if ctx.keys_free(ui) {
            ctx.player.handle_keys(ui);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::gui::app::{Action, App, Page};

    /// Runs `frames` headless frames at a desktop size.
    fn run(ctx: &egui::Context, app: &mut App, frames: usize) {
        for _ in 0..frames {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1500.0, 900.0),
                )),
                ..Default::default()
            };
            ctx.run_pass(input, |ctx| app.ui(ctx))
                .drop_without_applying_deltas();
        }
    }

    #[test]
    fn home_and_export_draw_an_open_night_without_writing_to_it() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-11").join("_ave");
        std::fs::create_dir_all(work.join("plans"))?;
        std::fs::write(work.join("ave.toml"), "footage = \"nowhere\"\n")?;
        std::fs::write(work.join("plans").join("reel-30.toml"), "not a plan")?;
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, crate::config::Config::default(), Some(work.clone()));
        app.apply(&ctx, Action::Go(Page::Night));
        run(&ctx, &mut app, 4);
        assert!(app.project.is_some(), "the night opened");
        app.apply(&ctx, Action::Go(Page::Deliver));
        run(&ctx, &mut app, 4);
        assert!(app.export_open, "export docked");
        assert!(
            !work.join(crate::recipe::FILE).exists(),
            "opening and drawing a night never writes its recipe"
        );
        Ok(())
    }

    /// A folder dropped on the window, as a test stands it in.
    #[derive(Debug)]
    struct Dropped(std::path::PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            Err("a folder".to_owned())
        }
    }

    /// Runs one frame with `path` dropped on the window.
    fn drop_on(ctx: &egui::Context, app: &mut App, path: &std::path::Path) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1500.0, 900.0),
            )),
            dropped_files: vec![std::sync::Arc::new(Dropped(path.to_path_buf()))],
            ..Default::default()
        };
        ctx.run_pass(input, |ctx| app.ui(ctx))
            .drop_without_applying_deltas();
    }

    #[test]
    fn a_dropped_folder_becomes_a_night_in_the_wizard() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let video = dir
            .path()
            .join("2026-11-18")
            .join("originals")
            .join("video");
        std::fs::create_dir_all(&video)?;
        std::fs::write(video.join("A.mov"), b"not really a video")?;
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, crate::config::Config::default(), None);
        app.apply(&ctx, Action::Go(Page::Night));
        run(&ctx, &mut app, 2);
        assert!(app.project.is_none(), "the start page, no night");
        app.pages.night.start.prompt = super::start::TEMPLATES[0].1.to_owned();
        drop_on(&ctx, &mut app, &video);
        run(&ctx, &mut app, 3);
        let work = dir.path().join("2026-11-18").join("_ave");
        assert_eq!(
            app.project.as_ref().map(|p| p.work.clone()),
            Some(work.clone()),
            "the dropped footage opened its night"
        );
        assert!(work.join("ave.toml").is_file(), "the night was set up");
        assert_eq!(
            app.pages.night.view,
            super::View::Wizard,
            "the wizard opened"
        );
        assert!(
            app.chat_inbox.is_empty(),
            "the first request waits until the analysis is approved"
        );
        for step in [
            super::wizard::Step::Music,
            super::wizard::Step::Deliverables,
            super::wizard::Step::Approve,
        ] {
            if let Some(w) = app.pages.night.wizard.as_mut() {
                w.step = step;
            }
            run(&ctx, &mut app, 3);
        }
        assert!(
            app.pages.night.wizard.is_some(),
            "drawing every step leaves the wizard open"
        );
        Ok(())
    }

    #[test]
    fn opening_an_indexed_night_hands_the_request_to_the_chat() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-11").join("_ave");
        crate::index::Index::default().save(&work)?;
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, crate::config::Config::default(), None);
        app.apply(&ctx, Action::Go(Page::Night));
        app.pages.night.start.prompt = "Cut a 60 s recap".to_owned();
        app.pages.night.arriving = Some(work.clone());
        app.apply(&ctx, Action::OpenProject(work));
        app.layout
            .set_open(crate::gui::ui::layout::Region::Chat, false);
        run(&ctx, &mut app, 2);
        // The chat may already have taken the message into its composer
        // (egui can run a second pass in the same frame); `ChatInbox` is
        // the only thing here that opens the chat panel.
        assert!(
            app.layout.is_open(crate::gui::ui::layout::Region::Chat),
            "the request went to the chat (inbox {:?})",
            app.chat_inbox
        );
        assert!(!app.home, "and the editor is open");
        assert!(app.pages.night.start.prompt.is_empty(), "sent once");
        Ok(())
    }

    #[test]
    fn the_processing_page_follows_a_run_and_stops_it() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-11").join("_ave");
        let video = dir.path().join("2026-11-11").join("video");
        std::fs::create_dir_all(&video)?;
        std::fs::create_dir_all(&work)?;
        std::fs::write(
            work.join("ave.toml"),
            format!("footage = {:?}\n", video.display().to_string()),
        )?;
        let ctx = egui::Context::default();
        let mut app = App::new(&ctx, crate::config::Config::default(), Some(work.clone()));
        app.apply(&ctx, Action::Go(Page::Night));
        let show = |label: &str| {
            (
                label.to_owned(),
                crate::commands::Cmd::Show {
                    work: work.clone(),
                    ids: Vec::new(),
                },
            )
        };
        let state = &mut app.pages.night;
        state
            .pipeline
            .enqueue(&app.jobs, vec![show("index"), show("caption")]);
        state.processing.begin(
            work.clone(),
            ["index", "caption"]
                .iter()
                .map(|l| super::wizard::Planned {
                    label: (*l).to_owned(),
                    title: (*l).to_owned(),
                    secs: 10.0,
                })
                .collect(),
        );
        state.view = super::View::Processing;
        run(&ctx, &mut app, 3);
        app.pages.night.pipeline.cancel(&mut app.jobs);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while app.pages.night.pipeline.active() && std::time::Instant::now() < deadline {
            run(&ctx, &mut app, 1);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            !app.pages.night.pipeline.active(),
            "stopping empties the queue"
        );
        assert!(
            app.jobs.list.iter().all(|j| j.label != "caption"),
            "the queued step never started"
        );
        Ok(())
    }

    /// Renders the start page, the wizard's approval card and the
    /// processing page of a copy of a real night to PNGs, when
    /// `AVE_HOME_SHOTS=<out dir>` and `AVE_SCREENSHOT_NIGHT=<a copy of a
    /// night's _ave>` are set; otherwise does nothing. The processing shot
    /// runs a real index on the copy (reading the footage, writing only the
    /// copy), with half the clip cache removed so there is work to show.
    #[test]
    fn home_screenshots_on_request() -> anyhow::Result<()> {
        let (Some(out), Some(night)) = (
            std::env::var_os("AVE_HOME_SHOTS").map(std::path::PathBuf::from),
            std::env::var_os("AVE_SCREENSHOT_NIGHT").map(std::path::PathBuf::from),
        ) else {
            return Ok(());
        };
        std::fs::create_dir_all(&out)?;
        let shoot = |name: &str,
                     setup: &dyn Fn(&mut App, &egui::Context),
                     wait: &dyn Fn(&App) -> bool|
         -> anyhow::Result<()> {
            let mut harness = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1680.0, 1000.0))
                .with_accessibility_check(false)
                .build_ui_state(
                    |ui, app: &mut Option<App>| {
                        let app = app.get_or_insert_with(|| {
                            let config = crate::config::Config {
                                recent_projects: Some(vec![night.display().to_string()]),
                                ..Default::default()
                            };
                            let mut app = App::new(ui.ctx(), config, None);
                            setup(&mut app, ui.ctx());
                            app
                        });
                        app.show(ui);
                    },
                    None,
                );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(240);
            let mut rounds = 0;
            while rounds < 40
                || (harness.state().as_ref().is_some_and(|a| !wait(a))
                    && std::time::Instant::now() < deadline)
            {
                harness.run_steps(10);
                std::thread::sleep(std::time::Duration::from_millis(100));
                rounds += 1;
            }
            let image = harness
                .render()
                .map_err(|e| anyhow::anyhow!("rendering {name}: {e}"))?;
            image.save(out.join(format!("{name}.png")))?;
            Ok(())
        };
        shoot(
            "home-start",
            &|app, ctx| {
                app.apply(ctx, Action::Go(Page::Night));
                app.pages.night.start.prompt = super::start::TEMPLATES[0].1.to_owned();
            },
            &|_| true,
        )?;
        shoot(
            "home-wizard",
            &|app, ctx| {
                app.apply(ctx, Action::OpenProject(night.clone()));
                app.apply(ctx, Action::Go(Page::Night));
                let mut w = super::wizard::State::new(night.clone(), None, Vec::new());
                w.step = super::wizard::Step::Approve;
                app.pages.night.wizard = Some(w);
                app.pages.night.view = super::View::Wizard;
            },
            &|_| true,
        )?;
        // Half the cache goes, so the index has clips to analyse.
        let mut cached: Vec<std::path::PathBuf> = std::fs::read_dir(night.join("clips"))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        cached.sort();
        for path in cached.iter().skip(cached.len() / 2) {
            std::fs::remove_file(path)?;
        }
        shoot(
            "home-processing",
            &|app, ctx| {
                app.apply(ctx, Action::OpenProject(night.clone()));
                app.apply(ctx, Action::Go(Page::Night));
                let footage = app
                    .project
                    .as_ref()
                    .and_then(|p| p.settings.footage.clone())
                    .unwrap_or_default();
                let jobs = vec![
                    super::pipeline::index_cmd(&night, &footage, false),
                    (
                        "people".to_owned(),
                        crate::commands::Cmd::Show {
                            work: night.clone(),
                            ids: Vec::new(),
                        },
                    ),
                ];
                let state = &mut app.pages.night;
                state.pipeline.enqueue(&app.jobs, jobs);
                state.processing.begin(
                    night.clone(),
                    vec![
                        super::wizard::Planned {
                            label: "index".to_owned(),
                            title: "Index".to_owned(),
                            secs: 60.0,
                        },
                        super::wizard::Planned {
                            label: "people".to_owned(),
                            title: "People".to_owned(),
                            secs: 2.0,
                        },
                    ],
                );
                state.view = super::View::Processing;
            },
            // Shoot mid-run: once a few clips have been analysed.
            &|app| {
                app.jobs
                    .list
                    .iter()
                    .find(|j| j.label == "index")
                    .is_some_and(|j| {
                        j.status != crate::jobs::Status::Running
                            || j.progress
                                .items
                                .values()
                                .filter(|i| i.secs.is_some())
                                .count()
                                >= 3
                    })
            },
        )?;
        Ok(())
    }
}

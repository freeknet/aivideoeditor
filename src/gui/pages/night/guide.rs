// ============================================================================
// Module:       gui::pages::night::guide
// Description:  The guide: one strip under the top bar on every screen of an
//               open project (Footage → Analyse → Music → First cut → Review
//               → Export) showing where the project stands, with the one
//               button that does the next thing.
//
// Dependencies: egui, rfd; crate::{project, recipe},
//               crate::gui::pages::night::{pipeline, parts}
// ============================================================================

//! The guide.
//!
//! Selects never leaves you wondering what to do next: the project moves
//! from import to analysis to a first cut to review and export, and the
//! screen always says which of those you are on. Here that is a strip under
//! the top bar, on Home and in the editor alike:
//!
//! - **Six stages**, each done, current, busy or still to come, read from
//!   what is on disk ([`facts`], [`marks`]), so work done in a terminal or
//!   by the chat counts the same as work done here. Clicking a stage goes
//!   to the screen that works on it.
//! - **One "next" button** for the first stage not done: pick the footage,
//!   set the project up, choose or map the music, draft the first cut,
//!   review it, export it. While something runs, the button gives way to
//!   what is running and how far it has got.
//!
//! Other screens reach the same steps through [`Action::RunStep`]: an empty
//! timeline's "Map the music" or the Shots browser's "Analyse footage" asks
//! for a stage, and the guide runs it on the next frame, through the one
//! pipeline queue on Home. No screen tells you to go and run something
//! somewhere else.
//!
//! When a set-up run (the wizard's "Start analysis") ends with a first cut
//! drafted and you are still watching its progress, the guide opens the
//! editor on it.

use crate::gui::app::{Action, Page, Stage};
use crate::gui::pages::PageCtx;
use crate::gui::ui::{icons, theme, widgets};
use crate::project::Project;
use crate::recipe::{self, Recipe};

use super::{View, parts, pipeline};

impl Stage {
    /// Every stage, in order.
    pub(crate) const ALL: [Self; 6] = [
        Self::Footage,
        Self::Analyse,
        Self::Music,
        Self::FirstCut,
        Self::Review,
        Self::Export,
    ];

    /// Its name on the strip.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Footage => "Footage",
            Self::Analyse => "Analyse",
            Self::Music => "Music",
            Self::FirstCut => "First cut",
            Self::Review => "Review",
            Self::Export => "Export",
        }
    }

    /// Whether a pipeline job label is this stage's work.
    fn owns(self, label: &str) -> bool {
        match self {
            Self::Footage | Self::Review => false,
            Self::Analyse => matches!(label, "index" | "proxy" | "caption" | "people"),
            Self::Music => label == "beats",
            Self::FirstCut => label.starts_with("plan "),
            Self::Export => label.starts_with("render ") || label.starts_with("otio "),
        }
    }
}

/// Where a stage stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// Finished.
    Done,
    /// The first one not finished: the next button works on it.
    Here,
    /// Its work is running or queued.
    Busy,
    /// After the current one.
    Later,
}

/// What the guide reads from the open project.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    /// Whether a footage folder is set.
    pub(crate) footage: bool,
    /// Clips indexed, if the shot log exists.
    pub(crate) indexed: Option<usize>,
    /// Segments captioned, of all segments.
    pub(crate) captioned: (usize, usize),
    /// Whether a track is chosen.
    pub(crate) track: bool,
    /// Whether the track is mapped (beats, bars, drops).
    pub(crate) mapped: bool,
    /// Plans in the project.
    pub(crate) plans: usize,
    /// Deliverables the recipe asks for.
    pub(crate) deliverables: usize,
    /// Of those, how many are in the output folder.
    pub(crate) delivered: usize,
    /// Heavy clips (4K, HEVC) still without a playback copy.
    pub(crate) proxies_missing: usize,
}

/// Reads [`Facts`] from `project`, with `recipe` for the deliverables.
pub(crate) fn facts(project: &Project, recipe: &Recipe) -> Facts {
    let names = recipe::Names::for_work(&project.work);
    let out = recipe.output_dir(&project.work, &names);
    let delivered = recipe
        .deliverables
        .iter()
        .filter(|d| {
            out.join(recipe.filename(d, recipe::MAIN_VERSION, &names))
                .is_file()
        })
        .count();
    Facts {
        footage: project.settings.footage.is_some(),
        indexed: project.index.as_ref().map(|i| i.clips.len()),
        captioned: project.index.as_ref().map_or((0, 0), |i| {
            (
                i.segments.iter().filter(|s| s.caption.is_some()).count(),
                i.segments.len(),
            )
        }),
        track: project.settings.track.is_some(),
        // Mapped means mapped for *this* track (a track changed since is
        // not), or going without music.
        mapped: project.no_music()
            || project
                .music
                .as_ref()
                .zip(project.settings.track.as_ref())
                .is_some_and(|(m, t)| std::path::Path::new(&m.path) == t.as_path()),
        plans: project.plans.len(),
        deliverables: recipe.deliverables.len(),
        delivered,
        proxies_missing: project
            .index
            .as_ref()
            .map_or(0, |i| crate::proxy::missing(&project.work, i).len()),
    }
}

/// Whether `stage` is finished, by the facts.
fn done(f: &Facts, stage: Stage) -> bool {
    match stage {
        Stage::Footage => f.footage,
        Stage::Analyse => f.indexed.is_some() && f.proxies_missing == 0,
        Stage::Music => f.mapped,
        Stage::FirstCut => f.plans > 0,
        Stage::Review => f.delivered > 0,
        Stage::Export => f.deliverables > 0 && f.delivered >= f.deliverables,
    }
}

/// Every stage's mark: done ones, the first one not done as [`Mark::Here`]
/// (or [`Mark::Busy`] while its work runs), the rest [`Mark::Later`]. A
/// later stage whose work is running is busy too.
pub(crate) fn marks(f: &Facts, busy: impl Fn(Stage) -> bool) -> Vec<(Stage, Mark)> {
    let mut here_found = false;
    Stage::ALL
        .into_iter()
        .map(|stage| {
            let mark = if busy(stage) {
                Mark::Busy
            } else if done(f, stage) && !here_found {
                Mark::Done
            } else if here_found {
                Mark::Later
            } else {
                Mark::Here
            };
            if matches!(mark, Mark::Here | Mark::Busy) && !done(f, stage) {
                here_found = true;
            }
            (stage, mark)
        })
        .collect()
}

/// The stage the next button works on: the first one not done.
pub(crate) fn current(f: &Facts) -> Option<Stage> {
    Stage::ALL.into_iter().find(|s| !done(f, *s))
}

/// What the next button says and does for `stage`.
pub(crate) fn next_label(f: &Facts, stage: Stage) -> &'static str {
    match stage {
        Stage::Footage => "Choose footage\u{2026}",
        Stage::Analyse if f.indexed.is_some() => "Prepare smooth playback",
        Stage::Analyse => "Set up and analyse",
        Stage::Music if f.track => "Map the music",
        Stage::Music => "Choose music\u{2026}",
        Stage::FirstCut => "Draft the first cut",
        Stage::Review => "Review the cut",
        Stage::Export => "Export",
    }
}

/// The page state the guide needs from Home.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// Stages other screens asked for (`Action::RunStep`), oldest first.
    pub(crate) requests: Vec<Stage>,
    /// A problem to show under the strip (a refused folder, say).
    notice: Option<String>,
    /// The facts, and when and for which project they were read.
    cached: Option<(std::path::PathBuf, std::time::Instant, Facts)>,
    /// The project whose missing playback copies were queued on their own,
    /// so they are queued once, not again every frame.
    auto_proxy: Option<std::path::PathBuf>,
}

/// Re-reads the facts at most every second (the output folder is a few
/// file checks; the rest is in memory).
fn read_facts(state: &mut State, project: &Project, recipe: &Recipe, force: bool) -> Facts {
    let fresh = state.cached.as_ref().is_some_and(|(w, at, _)| {
        *w == project.work && at.elapsed() < std::time::Duration::from_secs(1)
    });
    if !fresh || force {
        state.cached = Some((
            project.work.clone(),
            std::time::Instant::now(),
            facts(project, recipe),
        ));
    }
    state
        .cached
        .as_ref()
        .map(|(_, _, f)| f.clone())
        .unwrap_or_default()
}

/// Draws the strip and does what it, or another screen, asked for.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, home: &mut super::State, ui: &mut egui::Ui) {
    let Some(work) = ctx.project.as_ref().map(|p| p.work.clone()) else {
        return;
    };
    home.settings.sync(ctx);
    let recipe = home.settings.recipe();
    let requests = std::mem::take(&mut home.guide.requests);
    let Some(project) = ctx.project.as_ref() else {
        return;
    };
    let f = read_facts(&mut home.guide, project, &recipe, !requests.is_empty());
    for stage in requests {
        run(ctx, home, &recipe, &f, stage);
    }
    hand_over(ctx, home, &f, &work);
    // Heavy clips without playback copies get them without being asked:
    // playback stutters until they exist, and nobody should need to know
    // that to fix it. Once per project, and only when nothing else runs.
    if f.indexed.is_some()
        && f.proxies_missing > 0
        && home.guide.auto_proxy.as_ref() != Some(&work)
        && !home.pipeline.active()
        && ctx.jobs.running().next().is_none()
    {
        home.guide.auto_proxy = Some(work.clone());
        home.pipeline
            .enqueue(ctx.jobs, vec![pipeline::proxy_cmd(&work, false)]);
    }

    let busy_stage = |stage: Stage| {
        home.pipeline.queued().any(|l| stage.owns(l))
            || ctx.jobs.running().any(|j| stage.owns(&j.label))
    };
    let marks = marks(&f, busy_stage);
    let running = ctx.jobs.running().next().map(|j| {
        let p = &j.progress;
        let how = match (p.total, &p.stage) {
            (0, Some(stage)) => stage.clone(),
            (0, None) => "starting\u{2026}".to_owned(),
            (total, _) => format!("{}/{total}", p.done),
        };
        format!("{} \u{b7} {how}", job_title(&j.label))
    });
    let theme = ctx.theme;
    let mut clicked_stage = None;
    let mut next = None;
    let mut progress = false;
    ui.horizontal(|ui| {
        ui.set_min_height(theme::CONTROL_HEIGHT);
        ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
        for (i, (stage, mark)) in marks.iter().enumerate() {
            if i > 0 {
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(icons::SMALL, icons::SMALL),
                    egui::Sense::hover(),
                );
                icons::paint(
                    ui.painter(),
                    rect,
                    icons::CHEVRON_RIGHT,
                    icons::SMALL,
                    theme::rgb(theme.text_faint),
                );
            }
            if stage_chip(ui, theme, *stage, *mark).clicked() {
                clicked_stage = Some(*stage);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
            if let Some(line) = &running {
                progress = widgets::outline_button(ui, theme, "Progress")
                    .on_hover_text("What each step and clip has been through")
                    .clicked();
                ui.label(
                    egui::RichText::new(line)
                        .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                        .color(theme::rgb(theme.text_muted)),
                );
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(icons::SMALL), egui::Sense::hover());
                parts::mark(ui, theme, rect, parts::Mark::Running);
            } else if let Some(stage) = current(&f) {
                let label = next_label(&f, stage);
                let tip = next_tip(&f, stage);
                if widgets::button_with(
                    ui,
                    theme,
                    widgets::ButtonKind::Key,
                    Some(icons::SPARKLES),
                    label,
                )
                .on_hover_text(tip)
                .clicked()
                {
                    next = Some(stage);
                }
                ui.label(
                    egui::RichText::new("Next:")
                        .font(theme::font(theme::BODY_SIZE))
                        .color(theme::rgb(theme.text_muted)),
                );
            } else {
                if widgets::outline_button(ui, theme, "Export again\u{2026}").clicked() {
                    next = Some(Stage::Export);
                }
                parts::status_line(ui, theme, parts::Kind::Success, "Everything is delivered");
            }
        });
    });
    if let Some(why) = home.guide.notice.clone() {
        ui.add_space(theme::SPACE_XS);
        ui.horizontal(|ui| {
            parts::status_line(ui, theme, parts::Kind::Warning, &why);
            if widgets::ghost_button(ui, theme, "Dismiss").clicked() {
                home.guide.notice = None;
            }
        });
    }
    // A failed step is said where the person is, not only on Home.
    // One line here; the full reasons are on the progress page.
    let problem = home
        .pipeline
        .stopped()
        .map(|s| (short(s), true))
        .or_else(|| summary(home.pipeline.skipped_lines()).map(|s| (s, false)));
    if let Some((why, stopped)) = problem {
        ui.add_space(theme::SPACE_XS);
        ui.horizontal(|ui| {
            parts::status_line(ui, theme, parts::Kind::Warning, &why);
            if widgets::ghost_button(ui, theme, "Details").clicked() {
                home.view = View::Processing;
                ctx.actions.push(Action::Go(Page::Night));
            }
            if !stopped && widgets::ghost_button(ui, theme, "Dismiss").clicked() {
                home.pipeline.dismiss_skipped();
            }
        });
    }
    if progress {
        home.view = View::Processing;
        ctx.actions.push(Action::Go(Page::Night));
    }
    if let Some(stage) = clicked_stage {
        go_to(ctx, home, stage);
    }
    if let Some(stage) = next {
        run(ctx, home, &recipe, &f, stage);
    }
}

/// The skipped steps of a run as one short line: captions skipped, which
/// cuts were not drafted, anything else by count. `None` when nothing was.
pub(crate) fn summary(lines: &[String]) -> Option<String> {
    let captions = lines.iter().any(|l| l.starts_with("Captions were skipped"));
    let undrafted: Vec<&str> = lines
        .iter()
        .filter_map(|l| l.split_once(" was not drafted").map(|(name, _)| name))
        .collect();
    let loosened: Vec<&str> = lines
        .iter()
        .filter_map(|l| l.split_once(": rules loosened").map(|(name, _)| name))
        .collect();
    let other = lines.len() - usize::from(captions) - undrafted.len() - loosened.len();
    // Say why only when every failed draft failed for that reason.
    let footage = lines
        .iter()
        .filter(|l| l.contains(" was not drafted"))
        .all(|l| l.contains("not enough different footage"));
    let why = if footage { ": not enough footage" } else { "" };
    let mut parts = Vec::new();
    if captions {
        parts.push("Captions skipped (not set up)".to_owned());
    }
    match undrafted.as_slice() {
        [] => {}
        [one] => parts.push(format!("{one} not drafted{why}")),
        many => parts.push(format!(
            "{} cuts not drafted ({}){why}",
            many.len(),
            many.join(", ")
        )),
    }
    match loosened.as_slice() {
        [] => {}
        [one] => parts.push(format!(
            "{one} drafted with looser repeat rules to fit the footage"
        )),
        many => parts.push(format!(
            "{} cuts drafted with looser repeat rules ({})",
            many.len(),
            many.join(", ")
        )),
    }
    if other > 0 {
        parts.push(format!("{other} other step(s) skipped"));
    }
    (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
}

/// A long message cut to its first clause, for one line on the strip.
fn short(text: &str) -> String {
    const MAX: usize = 110;
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    let cut: String = text.chars().take(MAX).collect();
    format!("{}\u{2026}", cut.trim_end())
}

/// What a job label reads as on the strip.
fn job_title(label: &str) -> String {
    match label {
        "index" => "Analysing footage".to_owned(),
        "proxy" => "Preparing smooth playback".to_owned(),
        "caption" => "Captioning".to_owned(),
        "people" => "Finding who's who".to_owned(),
        "beats" => "Mapping the music".to_owned(),
        other => match other.split_once(' ') {
            Some(("plan", name)) => format!("Drafting {name}"),
            Some(("render", name)) => format!("Rendering {name}"),
            Some(("otio", name)) => format!("Exporting {name}"),
            _ => other.to_owned(),
        },
    }
}

/// The next button's tooltip.
fn next_tip(f: &Facts, stage: Stage) -> String {
    match stage {
        Stage::Footage => "Pick the folder your clips are in.".to_owned(),
        Stage::Analyse if f.indexed.is_some() => format!(
            "Make light playback copies of the {} heavy clip(s) (4K or HEVC), so the timeline plays smoothly. Renders still use the originals.",
            f.proxies_missing
        ),
        Stage::Analyse => {
            "Footage, music and what to make, then approve what runs. Everything after it runs on its own."
                .to_owned()
        }
        Stage::Music if f.track => "Find the track's beats, bars and drops, so cuts land on them.".to_owned(),
        Stage::Music => "Pick a track to cut to its beat; with none, cuts are paced every two seconds.".to_owned(),
        Stage::FirstCut => format!(
            "Draft {} on the beat grid from the shot log.",
            match f.deliverables {
                1 => "the deliverable".to_owned(),
                n => format!("all {n} deliverables"),
            }
        ),
        Stage::Review => "Open the timeline: watch the cut, change shots, or ask the chat for changes.".to_owned(),
        Stage::Export => "Render the cuts and copy them to the output folder.".to_owned(),
    }
}

/// One stage on the strip: its mark and name; returns the click.
fn stage_chip(
    ui: &mut egui::Ui,
    theme: &crate::theme::Palette,
    stage: Stage,
    mark: Mark,
) -> egui::Response {
    let font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    let galley =
        ui.painter()
            .layout_no_wrap(stage.title().to_owned(), font, egui::Color32::PLACEHOLDER);
    let pad = theme::SPACE_SM;
    let size = egui::vec2(
        icons::SMALL + theme::SPACE_XS + galley.size().x + pad * 2.0,
        theme::CONTROL_HEIGHT - 4.0,
    );
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::Button,
            true,
            matches!(mark, Mark::Here | Mark::Busy),
            stage.title(),
        )
    });
    let hover = widgets::hover_t(ui, response.id, response.hovered());
    let painter = ui.painter();
    if matches!(mark, Mark::Here | Mark::Busy) {
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(theme::RADIUS_MD),
            theme::selected_fill(theme),
        );
    } else if hover > 0.0 {
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(theme::RADIUS_MD),
            theme::ink(theme, theme::INK_03),
        );
    }
    let dot = egui::Rect::from_min_size(
        egui::pos2(rect.left() + pad, rect.center().y - icons::SMALL / 2.0),
        egui::Vec2::splat(icons::SMALL),
    );
    let ink = match mark {
        Mark::Done => theme::rgb(theme.text_muted),
        Mark::Here | Mark::Busy => theme::rgb(theme.text),
        Mark::Later => theme::rgb(theme.text_faint),
    };
    match mark {
        Mark::Done => parts::mark(ui, theme, dot, parts::Mark::Done),
        Mark::Busy => parts::mark(ui, theme, dot, parts::Mark::Running),
        Mark::Here => {
            ui.painter().circle_filled(
                dot.center(),
                dot.width() / 2.0 - 2.0,
                theme::rgb(theme.text),
            );
        }
        Mark::Later => parts::mark(ui, theme, dot, parts::Mark::Queued),
    }
    ui.painter().galley(
        egui::pos2(
            dot.right() + theme::SPACE_XS,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        ink,
    );
    response.on_hover_text(match stage {
        Stage::Footage => "The project's clips and settings",
        Stage::Analyse => "Analysis progress, clip by clip",
        Stage::Music => "The beat map",
        Stage::FirstCut | Stage::Review => "The timeline",
        Stage::Export => "Renders and deliverables",
    })
}

/// Goes to the screen that works on `stage`.
fn go_to(ctx: &mut PageCtx<'_>, home: &mut super::State, stage: Stage) {
    match stage {
        Stage::Footage => {
            home.view = View::Night;
            ctx.actions.push(Action::Go(Page::Night));
        }
        Stage::Analyse => {
            home.view = View::Processing;
            ctx.actions.push(Action::Go(Page::Night));
        }
        Stage::Music => ctx.actions.push(Action::Go(Page::Music)),
        Stage::FirstCut | Stage::Review => ctx.actions.push(Action::Go(Page::Plan)),
        Stage::Export => ctx.actions.push(Action::Go(Page::Deliver)),
    }
}

/// Does the next thing for `stage`: picks, sets up, queues or opens.
fn run(ctx: &mut PageCtx<'_>, home: &mut super::State, recipe: &Recipe, f: &Facts, stage: Stage) {
    home.guide.notice = None;
    let Some(work) = ctx.project.as_ref().map(|p| p.work.clone()) else {
        return;
    };
    match stage {
        Stage::Footage => {
            let Some(picked) = rfd::FileDialog::new()
                .set_title("The folder your clips are in")
                .pick_folder()
            else {
                return;
            };
            if let Some(why) = crate::project::footage_problem(&picked) {
                home.guide.notice = Some(why);
                return;
            }
            if let Some(p) = ctx.project.as_mut() {
                p.settings.footage = Some(picked);
                if let Err(e) = p.save_settings() {
                    home.guide.notice = Some(format!("Could not save the footage folder: {e:#}"));
                }
            }
        }
        Stage::Analyse => {
            if f.indexed.is_none() {
                // Never analysed: the set-up wizard asks what it needs
                // and starts everything at once.
                let previous = home.recent.last_night(Some(&work));
                home.start_wizard(work, previous, Vec::new());
                ctx.actions.push(Action::Go(Page::Night));
            } else if f.proxies_missing > 0 {
                home.pipeline
                    .enqueue(ctx.jobs, vec![pipeline::proxy_cmd(&work, false)]);
            } else if let Some(footage) = ctx
                .project
                .as_ref()
                .and_then(|p| p.settings.footage.clone())
            {
                let mut jobs = pipeline::index_jobs(&work, &footage, false);
                if f.captioned.0 < f.captioned.1 {
                    jobs.push(pipeline::caption_cmd(&work));
                }
                home.pipeline.enqueue(ctx.jobs, jobs);
            }
        }
        Stage::Music => {
            if !f.track {
                let Some(track) = rfd::FileDialog::new()
                    .set_title("The track the cut is timed to")
                    .add_filter("Audio", &["wav", "mp3", "flac", "m4a", "aac", "ogg"])
                    .pick_file()
                else {
                    return;
                };
                if let Some(p) = ctx.project.as_mut() {
                    p.settings.track = Some(track);
                    if let Err(e) = p.save_settings() {
                        home.guide.notice = Some(format!("Could not save the track: {e:#}"));
                        return;
                    }
                }
            }
            let Some(track) = ctx.project.as_ref().and_then(|p| p.settings.track.clone()) else {
                return;
            };
            home.pipeline.enqueue(
                ctx.jobs,
                vec![(
                    "beats".to_owned(),
                    crate::commands::Cmd::Beats {
                        track: Some(track),
                        none: false,
                        work: work.clone(),
                    },
                )],
            );
        }
        Stage::FirstCut => {
            if f.indexed.is_none() || !f.mapped {
                home.guide.notice = Some(
                    "The first cut needs the footage analysed, and the chosen track mapped."
                        .to_owned(),
                );
                return;
            }
            if recipe.deliverables.is_empty() {
                home.guide.notice = Some(
                    "Nothing to make yet: add a deliverable on the Home screen's recipe."
                        .to_owned(),
                );
                home.view = View::Night;
                ctx.actions.push(Action::Go(Page::Night));
                return;
            }
            // Only deliverables without a plan: a plan that exists may
            // hold hand edits, and redrafting it is the pipeline's
            // confirmed "Redraft", never a side effect of "next".
            let missing: Vec<String> = ctx
                .project
                .as_ref()
                .map(|p| {
                    let matched = recipe.match_plans(&p.plans);
                    recipe
                        .deliverables
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| {
                            !matched
                                .iter()
                                .any(|m| m.deliverable == *i && m.version == recipe::MAIN_VERSION)
                        })
                        .map(|(_, d)| d.id.clone())
                        .collect()
                })
                .unwrap_or_default();
            let jobs = pipeline::plan_cmds(ctx, recipe, &missing);
            home.pipeline.enqueue(ctx.jobs, jobs);
            ctx.actions.push(Action::Go(Page::Plan));
        }
        Stage::Review => ctx.actions.push(Action::Go(Page::Plan)),
        Stage::Export => ctx.actions.push(Action::Go(Page::Deliver)),
    }
}

/// After a set-up run that drafted the first cut, opens the editor on it,
/// once, if the person is still watching the run's progress.
fn hand_over(ctx: &mut PageCtx<'_>, home: &mut super::State, f: &Facts, work: &std::path::Path) {
    let Some(run) = home.processing.run.as_mut() else {
        return;
    };
    let drafts = run.steps.iter().any(|s| s.label.starts_with("plan "));
    let finished = !home.pipeline.active() && home.pipeline.stopped().is_none();
    if run.work != work || run.handed_over || !drafts || !finished || f.plans == 0 {
        return;
    }
    run.handed_over = true;
    if home.view == View::Processing {
        home.view = View::Night;
        ctx.actions.push(Action::Go(Page::Plan));
    }
}

#[cfg(test)]
mod tests {
    use super::{Facts, Mark, Stage, current, marks, next_label};

    /// A project with footage chosen and nothing else.
    fn fresh() -> Facts {
        Facts {
            footage: true,
            deliverables: 2,
            ..Facts::default()
        }
    }

    #[test]
    fn heavy_clips_without_playback_copies_keep_analysis_open() {
        let mut f = fresh();
        f.indexed = Some(4);
        f.proxies_missing = 2;
        assert_eq!(current(&f), Some(Stage::Analyse), "copies still to make");
        assert_eq!(next_label(&f, Stage::Analyse), "Prepare smooth playback");
        f.proxies_missing = 0;
        assert_ne!(current(&f), Some(Stage::Analyse), "made: analysis is done");
    }

    #[test]
    fn a_fresh_project_is_on_analysis_with_the_set_up_button() {
        let f = fresh();
        assert_eq!(current(&f), Some(Stage::Analyse), "footage is chosen");
        assert_eq!(next_label(&f, Stage::Analyse), "Set up and analyse");
        let m = marks(&f, |_| false);
        assert_eq!(m[0], (Stage::Footage, Mark::Done), "{m:?}");
        assert_eq!(m[1], (Stage::Analyse, Mark::Here), "{m:?}");
        assert!(
            m[2..].iter().all(|(_, mark)| *mark == Mark::Later),
            "nothing after the current stage is done or current: {m:?}"
        );
    }

    #[test]
    fn the_guide_walks_the_stages_in_order() {
        let mut f = Facts::default();
        assert_eq!(current(&f), Some(Stage::Footage), "no footage");
        f.footage = true;
        f.indexed = Some(12);
        assert_eq!(current(&f), Some(Stage::Music), "analysed");
        assert_eq!(next_label(&f, Stage::Music), "Choose music\u{2026}");
        f.track = true;
        assert_eq!(
            next_label(&f, Stage::Music),
            "Map the music",
            "a track, no map"
        );
        f.mapped = true;
        assert_eq!(current(&f), Some(Stage::FirstCut), "mapped");
        f.plans = 2;
        assert_eq!(current(&f), Some(Stage::Review), "drafted");
        f.deliverables = 2;
        f.delivered = 1;
        assert_eq!(current(&f), Some(Stage::Export), "one delivered");
        f.delivered = 2;
        assert_eq!(current(&f), None, "all delivered");
    }

    #[test]
    fn a_running_step_marks_its_stage_busy_and_holds_the_ones_after() {
        let mut f = fresh();
        f.indexed = Some(3);
        let m = marks(&f, |s| s == Stage::Music);
        assert_eq!(m[2], (Stage::Music, Mark::Busy), "{m:?}");
        assert_eq!(m[3], (Stage::FirstCut, Mark::Later), "{m:?}");
        assert_eq!(m[1], (Stage::Analyse, Mark::Done), "{m:?}");
    }

    #[test]
    fn skipped_steps_read_as_one_short_line() {
        let lines: Vec<String> = [
            "Captions were skipped (the vision server is not answering). The rest runs…",
            "reel-30 was not drafted: not enough different footage for a 30 s cut: by shot 13…",
            "recap was not drafted: not enough different footage for a 180 s cut: by shot 10…",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(
            super::summary(&lines).as_deref(),
            Some(
                "Captions skipped (not set up) \u{b7} 2 cuts not drafted (reel-30, recap): not enough footage"
            )
        );
        assert_eq!(
            super::summary(&lines[1..2]).as_deref(),
            Some("reel-30 not drafted: not enough footage"),
            "one cut is named"
        );
        assert_eq!(super::summary(&[]), None, "nothing skipped");
        let loose = vec![
            "recap: rules loosened to fit the footage: the same person 2 shots apart".to_owned(),
        ];
        assert_eq!(
            super::summary(&loose).as_deref(),
            Some("recap drafted with looser repeat rules to fit the footage"),
            "a loosened draft is a note, not a skipped step"
        );
    }

    #[test]
    fn job_labels_read_as_what_they_do() {
        assert_eq!(super::job_title("index"), "Analysing footage");
        assert_eq!(super::job_title("plan reel-30"), "Drafting reel-30");
        assert_eq!(super::job_title("render reel-30"), "Rendering reel-30");
        assert!(Stage::Analyse.owns("caption"), "captions are analysis");
        assert!(
            Stage::FirstCut.owns("plan reel-30"),
            "drafts are the first cut"
        );
        assert!(!Stage::Review.owns("plan reel-30"), "review runs nothing");
    }
}

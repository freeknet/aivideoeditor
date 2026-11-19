// ============================================================================
// Module:       gui::pages::night::pipeline
// Description:  The pipeline step bar (Index → Caption → Music → People →
//               Plans → Export), its serial job queue, "Run everything", and
//               the live log with progress.
//
// Dependencies: egui; crate::{commands, jobs, plan, recipe}, crate::gui::pages
// ============================================================================

//! The pipeline.
//!
//! Each step shows what it has produced ("81 clips · 417 segments · 37
//! flagged"), when it last ran (the modification time of what it wrote, so
//! a run from the terminal counts too) and a Run button.
//!
//! **One queue.** Every run, a single step or "Run everything", is appended
//! to one serial queue of `ctx.jobs` jobs; [`State::tick`] starts the next
//! when the one before it finishes and stops the queue on a failure. Two
//! steps writing the same shot log at once is not something a user can
//! cause.
//!
//! **Nothing hand-made is overwritten by surprise.** "Run everything" runs
//! only what is missing (no shot log, no captions, no music map, no
//! people.toml, plans that do not exist yet). Re-drafting people or plans
//! from their own step asks first, because those are the files a user edits.
//!
//! Progress: the job's parsed reports (`crate::jobs::JobProgress`) give
//! items done, total and time left; output without reports falls back to
//! any `N/M` in its lines, and indexing to counting the per-clip cache
//! files it writes.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::commands::Cmd;
use crate::gui::app::{Action, Page};
use crate::gui::pages::PageCtx;
use crate::gui::ui::{modal, theme, widgets};
use crate::jobs::{Jobs, Status};
use crate::recipe::{self, Recipe};

/// A pipeline step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// `ave index`.
    Index,
    /// `ave caption`.
    Caption,
    /// `ave beats`.
    Music,
    /// `ave people`.
    People,
    /// `ave plan` for every deliverable.
    Plans,
    /// Render and deliver (the export panel).
    Export,
}

impl Step {
    /// The step's name.
    fn title(self) -> &'static str {
        match self {
            Self::Index => "Index",
            Self::Caption => "Caption",
            Self::Music => "Music",
            Self::People => "People",
            Self::Plans => "Plans",
            Self::Export => "Export",
        }
    }

    /// Whether a job label belongs to this step.
    fn owns(self, label: &str) -> bool {
        match self {
            Self::Index => label == "index",
            Self::Caption => label == "caption",
            Self::Music => label == "beats",
            Self::People => label == "people",
            Self::Plans => label.starts_with("plan "),
            Self::Export => label.starts_with("render ") || label.starts_with("otio "),
        }
    }
}

/// Where a step stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Standing {
    /// Done.
    Done,
    /// Some of it done.
    Partial,
    /// Can run, has not.
    Ready,
    /// Waiting on an earlier step or a setting.
    Blocked,
    /// Running or queued.
    Busy,
}

/// The parsed `N/M` progress of a job, newest line first.
pub(crate) fn progress(lines: &[String]) -> Option<(u64, u64)> {
    lines.iter().rev().take(40).find_map(|line| {
        line.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
            .find_map(|word| {
                let (n, m) = word.split_once('/')?;
                let n: u64 = n.parse().ok()?;
                let m: u64 = m.trim_end_matches(':').parse().ok()?;
                (m > 0 && n <= m).then_some((n, m))
            })
    })
}

/// "3 min ago", "2 d ago": how long since `t`.
pub(crate) fn ago(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .map_or(0, |d| d.as_secs());
    match secs {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

/// Facts read from disk, refreshed every second or so.
#[derive(Debug, Default)]
struct Facts {
    /// When they were read, and for which night.
    read: Option<(PathBuf, Instant)>,
    /// Modification times: index, music, people, newest plan, newest render.
    index: Option<SystemTime>,
    /// `music.json`.
    music: Option<SystemTime>,
    /// `people.toml`.
    people: Option<SystemTime>,
    /// The newest plan.
    plans: Option<SystemTime>,
    /// The newest delivered file.
    export: Option<SystemTime>,
    /// Deliverables with a plan, and the ids of those without.
    missing_plans: Vec<String>,
    /// Deliverables' plans that fail their check.
    failing: Vec<String>,
    /// Deliverables in the output folder.
    delivered: usize,
    /// Videos in the footage folder.
    videos: Option<usize>,
    /// Clip cache files (`<work>/clips/*.json`) newer than the running
    /// index job's start.
    cached: usize,
}

/// Modification time of a path.
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The step bar's state.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// Jobs waiting to start, in order.
    queue: VecDeque<(String, Cmd)>,
    /// The queued job running now.
    current: Option<u64>,
    /// Jobs this queue run has started and the total it was asked for.
    started: usize,
    /// How many jobs the current run of the queue holds in all.
    total: usize,
    /// Why the queue stopped, if a job failed.
    stopped: Option<String>,
    /// The step whose log is shown.
    selected: Option<Step>,
    /// A re-run waiting for the user to confirm.
    confirm: Option<(String, Vec<(String, Cmd)>)>,
    /// Disk facts.
    facts: Facts,
}

impl State {
    /// Starts the next queued job when nothing queued is running. Stops the
    /// queue when a job failed.
    pub(crate) fn tick(&mut self, jobs: &mut Jobs) {
        if let Some(id) = self.current {
            match jobs.get(id).map(|j| j.status.clone()) {
                Some(Status::Running) => return,
                Some(Status::Done) | None => self.current = None,
                Some(Status::Failed(e)) => {
                    let label = jobs.get(id).map(|j| j.label.clone()).unwrap_or_default();
                    let first = e.lines().next().unwrap_or_default().to_owned();
                    self.stopped = Some(if crate::commands::is_cancelled(&e) {
                        format!("{label} cancelled; what it finished is kept")
                    } else {
                        format!("{label} failed: {first}")
                    });
                    self.queue.clear();
                    self.current = None;
                    return;
                }
            }
        }
        if let Some((label, cmd)) = self.queue.pop_front() {
            self.current = Some(jobs.spawn(label, cmd));
            self.started += 1;
        } else {
            self.started = 0;
            self.total = 0;
        }
    }

    /// Stops the run: drops what is queued and asks the running job to stop
    /// after the clip or segment in hand.
    pub(crate) fn cancel(&mut self, jobs: &mut Jobs) {
        self.queue.clear();
        if let Some(id) = self.current {
            jobs.cancel(id);
        }
    }

    /// Whether anything is queued or running.
    pub(crate) fn active(&self) -> bool {
        self.current.is_some() || !self.queue.is_empty()
    }

    /// Labels of the jobs waiting to start, in order.
    pub(crate) fn queued(&self) -> impl Iterator<Item = &str> {
        self.queue.iter().map(|(l, _)| l.as_str())
    }

    /// Why the queue stopped, if it did.
    pub(crate) fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// Appends jobs to the queue, skipping any already queued or running.
    pub(crate) fn enqueue(&mut self, jobs: &Jobs, list: Vec<(String, Cmd)>) {
        if self.current.is_none() && self.queue.is_empty() {
            self.started = 0;
            self.total = 0;
        }
        self.stopped = None;
        for (label, cmd) in list {
            let queued = self.queue.iter().any(|(l, _)| *l == label);
            let running = jobs.running().any(|j| j.label == label);
            if !queued && !running {
                self.queue.push_back((label, cmd));
                self.total += 1;
            }
        }
    }

    /// Whether a job for `step` is queued or running.
    fn busy(&self, jobs: &Jobs, step: Step) -> bool {
        self.queue.iter().any(|(l, _)| step.owns(l)) || jobs.running().any(|j| step.owns(&j.label))
    }

    /// Re-reads the disk facts when they are a second old or the night changed.
    fn refresh(&mut self, ctx: &PageCtx<'_>, recipe: &Recipe) {
        let Some(p) = ctx.project.as_ref() else {
            return;
        };
        let indexing = ctx
            .jobs
            .running()
            .find(|j| j.label == "index")
            .map(|j| j.started);
        let interval = if indexing.is_some() || self.current.is_some() {
            Duration::from_millis(700)
        } else {
            Duration::from_secs(2)
        };
        let fresh = self
            .facts
            .read
            .as_ref()
            .is_some_and(|(w, at)| *w == p.work && at.elapsed() < interval);
        if fresh {
            return;
        }
        let work = &p.work;
        let names = recipe::Names::for_work(work);
        let newest = |dir: PathBuf| {
            std::fs::read_dir(dir).ok().and_then(|e| {
                e.filter_map(Result::ok)
                    .filter_map(|e| e.metadata().ok()?.modified().ok())
                    .max()
            })
        };
        let matched = recipe.match_plans(&p.plans);
        let mut missing = Vec::new();
        let mut failing = Vec::new();
        for (i, d) in recipe.deliverables.iter().enumerate() {
            let main = matched
                .iter()
                .find(|m| m.deliverable == i && m.version == recipe::MAIN_VERSION);
            match main {
                None => missing.push(d.id.clone()),
                Some(m) => {
                    if let (Some(index), Some(music)) = (&p.index, &p.music)
                        && let Ok(plan) = crate::plan::Plan::load(&m.path)
                        && !crate::plan::check(&plan, index, music, &p.people).is_empty()
                    {
                        failing.push(d.id.clone());
                    }
                }
            }
        }
        let out = recipe.output_dir(work, &names);
        let delivered = recipe
            .deliverables
            .iter()
            .filter(|d| {
                out.join(recipe.filename(d, recipe::MAIN_VERSION, &names))
                    .is_file()
            })
            .count();
        let videos = p
            .settings
            .footage
            .as_deref()
            .and_then(|f| crate::index::list_videos(f).ok())
            .map(|v| v.len());
        let cached = indexing.map_or(0, |since| {
            let since = SystemTime::now()
                .checked_sub(since.elapsed())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            std::fs::read_dir(work.join("clips"))
                .map(|e| {
                    e.filter_map(Result::ok)
                        .filter(|e| {
                            e.metadata()
                                .and_then(|m| m.modified())
                                .is_ok_and(|t| t >= since)
                        })
                        .count()
                })
                .unwrap_or(0)
        });
        self.facts = Facts {
            read: Some((work.clone(), Instant::now())),
            index: modified(&work.join("index.json")),
            music: modified(&work.join("music.json")),
            people: modified(&work.join("people.toml")),
            plans: newest(work.join("plans")),
            export: newest(out),
            missing_plans: missing,
            failing,
            delivered,
            videos,
            cached,
        };
    }
}

/// One step as the bar shows it.
struct View {
    /// Which step.
    step: Step,
    /// Where it stands.
    standing: Standing,
    /// What it produced, or what it waits for.
    summary: String,
    /// When it last ran.
    when: Option<SystemTime>,
    /// What its button does.
    run: Run,
}

/// What a step's button does.
enum Run {
    /// Queue these jobs.
    Jobs(&'static str, Vec<(String, Cmd)>),
    /// Ask first, then queue.
    Confirm(&'static str, String, Vec<(String, Cmd)>),
    /// Open the export panel.
    Export,
    /// Nothing yet.
    None,
}

/// The index step.
pub(crate) fn index_cmd(work: &Path, footage: &Path, force: bool) -> (String, Cmd) {
    (
        "index".to_owned(),
        Cmd::Index {
            footage: footage.to_path_buf(),
            work: work.to_path_buf(),
            jobs: 4,
            force,
        },
    )
}

/// The caption step.
pub(crate) fn caption_cmd(work: &Path) -> (String, Cmd) {
    (
        "caption".to_owned(),
        Cmd::Caption {
            work: work.to_path_buf(),
            min_q: 0.15,
            redo: false,
            parallel: 3,
        },
    )
}

/// The plan jobs for these deliverables.
pub(crate) fn plan_cmds(ctx: &PageCtx<'_>, recipe: &Recipe, ids: &[String]) -> Vec<(String, Cmd)> {
    let Some(p) = ctx.project.as_ref() else {
        return Vec::new();
    };
    recipe
        .deliverables
        .iter()
        .filter(|d| ids.contains(&d.id))
        .map(|d| {
            (
                format!("plan {}", d.id),
                Recipe::plan_cmd(d, &p.work, &p.settings),
            )
        })
        .collect()
}

/// The steps with their standing, summary and action.
fn views(ctx: &PageCtx<'_>, state: &State, recipe: &Recipe) -> Vec<View> {
    let Some(p) = ctx.project.as_ref() else {
        return Vec::new();
    };
    let f = &state.facts;
    let work = p.work.clone();
    let mut out = Vec::new();

    // Index.
    let (standing, summary) = match (&p.index, f.videos) {
        (Some(i), videos) => {
            let flagged = i.segments.iter().filter(|s| !s.flags.is_empty()).count();
            let new = videos.filter(|v| *v > i.clips.len());
            let text = format!(
                "{} clips · {} segments · {flagged} flagged",
                i.clips.len(),
                i.segments.len()
            );
            match new {
                Some(v) => (
                    Standing::Partial,
                    format!("{text} · {} new in the folder", v - i.clips.len()),
                ),
                None => (Standing::Done, text),
            }
        }
        (None, Some(v)) => (Standing::Ready, format!("{v} clips to analyse")),
        (None, None) => (Standing::Blocked, "Set the footage folder".to_owned()),
    };
    let run = match &p.settings.footage {
        Some(footage) if p.index.is_some() => {
            Run::Jobs("Update", vec![index_cmd(&work, footage, false)])
        }
        Some(footage) => Run::Jobs("Run", vec![index_cmd(&work, footage, false)]),
        None => Run::None,
    };
    out.push(View {
        step: Step::Index,
        standing,
        summary,
        when: f.index,
        run,
    });

    // Caption.
    let captioned = p.index.as_ref().map(|i| {
        (
            i.segments.iter().filter(|s| s.caption.is_some()).count(),
            i.segments.len(),
        )
    });
    let (standing, summary, run) = match captioned {
        Some((0, n)) => (
            Standing::Ready,
            format!("{n} segments to caption"),
            Run::Jobs("Run", vec![caption_cmd(&work)]),
        ),
        Some((c, n)) => (
            Standing::Done,
            format!("{c} of {n} segments captioned"),
            Run::Jobs("Fill gaps", vec![caption_cmd(&work)]),
        ),
        None => (Standing::Blocked, "Index first".to_owned(), Run::None),
    };
    out.push(View {
        step: Step::Caption,
        standing,
        summary,
        when: captioned.filter(|(c, _)| *c > 0).and(f.index),
        run,
    });

    // Music.
    let beats = |t: &PathBuf| {
        (
            "beats".to_owned(),
            Cmd::Beats {
                track: t.clone(),
                work: work.clone(),
            },
        )
    };
    let track = p.settings.track.as_ref();
    let (standing, summary, run) = match (&p.music, track) {
        (Some(m), Some(t)) => (
            Standing::Done,
            format!(
                "{:.1} BPM · {} bars · {} drops",
                m.bpm,
                m.bars.len(),
                m.drops.len()
            ),
            Run::Jobs("Re-map", vec![beats(t)]),
        ),
        (Some(m), None) => (
            Standing::Done,
            format!("{:.1} BPM · {} bars", m.bpm, m.bars.len()),
            Run::None,
        ),
        (None, Some(t)) => (
            Standing::Ready,
            t.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            Run::Jobs("Run", vec![beats(t)]),
        ),
        (None, None) => (
            Standing::Blocked,
            "Set the music track".to_owned(),
            Run::None,
        ),
    };
    out.push(View {
        step: Step::Music,
        standing,
        summary,
        when: f.music,
        run,
    });

    // People.
    let people = |force: bool| {
        (
            "people".to_owned(),
            Cmd::People {
                work: work.clone(),
                force,
            },
        )
    };
    let named = p.people.names.len();
    let (standing, summary, run) = match (f.people.is_some(), p.index.is_some()) {
        (true, _) => (
            Standing::Done,
            format!("{named} clips named"),
            Run::Confirm(
                "Redraft",
                "Redraft people.toml from the captions? Names and ranges you edited are lost."
                    .to_owned(),
                vec![people(true)],
            ),
        ),
        (false, true) => (
            Standing::Ready,
            "Draft who's who from the captions".to_owned(),
            Run::Jobs("Run", vec![people(false)]),
        ),
        (false, false) => (Standing::Blocked, "Caption first".to_owned(), Run::None),
    };
    out.push(View {
        step: Step::People,
        standing,
        summary,
        when: f.people,
        run,
    });

    // Plans.
    let total = recipe.deliverables.len();
    let have = total.saturating_sub(f.missing_plans.len());
    let all: Vec<String> = recipe.deliverables.iter().map(|d| d.id.clone()).collect();
    let ready = p.index.is_some() && p.music.is_some();
    let (standing, summary, run) = if total == 0 {
        (
            Standing::Blocked,
            "The recipe has no deliverables".to_owned(),
            Run::None,
        )
    } else if !ready {
        (
            Standing::Blocked,
            "Needs the shot log and the music map".to_owned(),
            Run::None,
        )
    } else if have == 0 {
        (
            Standing::Ready,
            format!("{total} deliverables to draft"),
            Run::Jobs("Draft", plan_cmds(ctx, recipe, &all)),
        )
    } else {
        let failing = if f.failing.is_empty() {
            String::new()
        } else {
            format!(" · {} fail check", f.failing.len())
        };
        let standing = if have < total || !f.failing.is_empty() {
            Standing::Partial
        } else {
            Standing::Done
        };
        let run = if have < total {
            Run::Jobs("Draft missing", plan_cmds(ctx, recipe, &f.missing_plans))
        } else {
            Run::Confirm(
                "Redraft",
                format!(
                    "Redraft all {total} plans from the shot log? Shots and notes you edited by hand are lost."
                ),
                plan_cmds(ctx, recipe, &all),
            )
        };
        (standing, format!("{have} of {total} drafted{failing}"), run)
    };
    out.push(View {
        step: Step::Plans,
        standing,
        summary,
        when: f.plans,
        run,
    });

    // Export.
    let (standing, summary) = if total > 0 && f.delivered == total {
        (Standing::Done, format!("All {total} delivered"))
    } else if f.delivered > 0 {
        (
            Standing::Partial,
            format!("{} of {total} delivered", f.delivered),
        )
    } else if have > 0 {
        (Standing::Ready, "Render and deliver the plans".to_owned())
    } else {
        (Standing::Blocked, "Draft plans first".to_owned())
    };
    out.push(View {
        step: Step::Export,
        standing,
        summary,
        when: f.export.filter(|_| f.delivered > 0),
        run: Run::Export,
    });

    for view in &mut out {
        if state.busy(ctx.jobs, view.step) {
            view.standing = Standing::Busy;
        }
    }
    out
}

/// What "Run everything" would queue: whatever is missing, in order.
fn everything(
    ctx: &PageCtx<'_>,
    state: &State,
    recipe: &Recipe,
) -> (Vec<(String, Cmd)>, Vec<String>) {
    let Some(p) = ctx.project.as_ref() else {
        return (Vec::new(), vec!["no night open".to_owned()]);
    };
    let work = &p.work;
    let mut jobs = Vec::new();
    let mut skipped = Vec::new();
    let indexed = p.index.is_some();
    match &p.settings.footage {
        Some(footage) => {
            let new = state
                .facts
                .videos
                .zip(p.index.as_ref().map(|i| i.clips.len()))
                .is_some_and(|(v, c)| v > c);
            if !indexed || new {
                jobs.push(index_cmd(work, footage, false));
            }
        }
        None if !indexed => skipped.push("index: set the footage folder".to_owned()),
        None => {}
    }
    let captioned = p
        .index
        .as_ref()
        .is_some_and(|i| i.segments.iter().any(|s| s.caption.is_some()));
    if !captioned && (indexed || !jobs.is_empty()) {
        jobs.push(caption_cmd(work));
    }
    let mapped = p.music.is_some();
    match &p.settings.track {
        Some(track) if !mapped => jobs.push((
            "beats".to_owned(),
            Cmd::Beats {
                track: track.clone(),
                work: work.clone(),
            },
        )),
        None if !mapped => skipped.push("music: set the track".to_owned()),
        Some(_) | None => {}
    }
    let can_index = indexed || jobs.iter().any(|(l, _)| l == "index");
    if !work.join("people.toml").exists() && can_index {
        jobs.push((
            "people".to_owned(),
            Cmd::People {
                work: work.clone(),
                force: false,
            },
        ));
    }
    let has_music = mapped || jobs.iter().any(|(l, _)| l == "beats");
    if has_music && can_index {
        jobs.extend(plan_cmds(ctx, recipe, &state.facts.missing_plans));
    } else if !state.facts.missing_plans.is_empty() {
        skipped.push("plans: need the shot log and the music map".to_owned());
    }
    (jobs, skipped)
}

/// Draws the step bar, the queue and the log.
pub(crate) fn draw(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    settings: &super::settings::State,
    ui: &mut egui::Ui,
) {
    let recipe = settings.recipe();
    state.refresh(ctx, &recipe);
    let views = views(ctx, state, &recipe);
    let (all, skipped) = everything(ctx, state, &recipe);

    ui.horizontal(|ui| {
        widgets::title(ui, ctx.theme, "Pipeline");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let busy = state.current.is_some() || !state.queue.is_empty();
            let tip = if all.is_empty() {
                "Everything that can run has run. Re-run single steps below.".to_owned()
            } else {
                let labels: Vec<&str> = all.iter().map(|(l, _)| l.as_str()).collect();
                format!(
                    "Runs only what is missing, in order: {}. Nothing you edited is replaced.",
                    labels.join(" → ")
                )
            };
            let enabled = !all.is_empty() && !busy;
            let clicked = ui
                .add_enabled_ui(enabled, |ui| {
                    widgets::button(ui, ctx.theme, "Run everything", widgets::Emphasis::Primary)
                })
                .inner
                .on_hover_text(tip)
                .on_disabled_hover_text(if busy {
                    "The queue is running"
                } else {
                    "Everything that can run has run"
                })
                .clicked();
            if clicked {
                state.enqueue(ctx.jobs, all.clone());
                state.selected = None;
            }
            if !skipped.is_empty() {
                ui.label(
                    egui::RichText::new(format!("skips {}", skipped.join("; ")))
                        .small()
                        .color(theme::rgb(ctx.theme.text_faint)),
                );
            }
        });
    });
    ui.add_space(theme::SPACE_SM);
    bar(ctx, state, ui, views);
    ui.add_space(theme::SPACE_MD);
    queue_line(ctx, state, ui);
    log(ctx, state, ui);
    confirm(ctx, state, ui);
}

/// The six steps side by side, joined by chevrons.
fn bar(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui, views: Vec<View>) {
    let theme = ctx.theme;
    let gap = 18.0;
    let n = views.len().max(1) as f32;
    let width = ((ui.available_width() - gap * (n - 1.0)) / n).max(120.0);
    let height = 132.0;
    let mut clicked_run: Option<Run> = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let count = views.len();
        for (i, view) in views.into_iter().enumerate() {
            let selected = state.selected == Some(view.step);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
            let lift = widgets::hover_t(ui, response.id, response.hovered());
            let (accent, word) = match view.standing {
                Standing::Done => (theme.success, "done"),
                Standing::Partial => (theme.warning, "partly done"),
                Standing::Ready => (theme.accent, "ready"),
                Standing::Blocked => (theme.text_faint, "waiting"),
                Standing::Busy => (theme.accent, "running"),
            };
            let border = if selected {
                theme.accent
            } else {
                theme.border.lerp(theme.border_strong, lift)
            };
            let painter = ui.painter();
            painter.rect(
                rect,
                egui::CornerRadius::same(theme::RADIUS_LG),
                theme::rgb(theme.raised.lerp(theme.hover, lift * 0.5)),
                egui::Stroke::new(if selected { 2.0 } else { 1.0 }, theme::rgb(border)),
                egui::StrokeKind::Inside,
            );
            let stripe = egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(1.0, 1.0),
                egui::vec2(rect.width() - 2.0, 3.0),
            );
            painter.rect_filled(
                stripe,
                egui::CornerRadius {
                    nw: theme::RADIUS_LG,
                    ne: theme::RADIUS_LG,
                    sw: 0,
                    se: 0,
                },
                theme::rgb(accent),
            );
            let pad = theme::SPACE_MD;
            let x = rect.left() + pad;
            painter.text(
                egui::pos2(x, rect.top() + 14.0),
                egui::Align2::LEFT_TOP,
                format!("{}  {}", i + 1, view.step.title()),
                theme::font(theme::TITLE_SIZE),
                theme::rgb(theme.text),
            );
            painter.circle_filled(
                egui::pos2(x + 4.0, rect.top() + 44.0),
                4.0,
                theme::rgb(accent),
            );
            painter.text(
                egui::pos2(x + 13.0, rect.top() + 44.0),
                egui::Align2::LEFT_CENTER,
                word,
                theme::font(theme::BODY_SIZE - 2.0),
                theme::rgb(theme.text_muted),
            );
            let galley = painter.layout(
                view.summary.clone(),
                theme::font(theme::BODY_SIZE - 1.0),
                theme::rgb(theme.text),
                rect.width() - pad * 2.0,
            );
            painter.galley(
                egui::pos2(x, rect.top() + 56.0),
                galley,
                theme::rgb(theme.text),
            );
            if let Some(t) = view.when {
                painter.text(
                    egui::pos2(x, rect.bottom() - 46.0),
                    egui::Align2::LEFT_BOTTOM,
                    ago(t),
                    theme::font(theme::BODY_SIZE - 3.0),
                    theme::rgb(theme.text_faint),
                );
            }
            let button = egui::Rect::from_min_size(
                egui::pos2(x, rect.bottom() - pad - 26.0),
                egui::vec2(rect.width() - pad * 2.0, 26.0),
            );
            let busy = view.standing == Standing::Busy;
            let label = match &view.run {
                Run::Jobs(label, _) | Run::Confirm(label, _, _) => Some(*label),
                Run::Export => Some("Open export"),
                Run::None => None,
            };
            if let Some(label) = label {
                let label = if busy { "Running…" } else { label };
                let primary = matches!(view.standing, Standing::Ready) && !busy;
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
                let emphasis = if primary {
                    widgets::Emphasis::Primary
                } else {
                    widgets::Emphasis::Secondary
                };
                let pressed = child
                    .add_enabled_ui(!busy, |ui| widgets::button(ui, theme, label, emphasis))
                    .inner
                    .clicked();
                if pressed {
                    clicked_run = Some(view.run);
                    state.selected = Some(view.step);
                }
            }
            if response.clicked() {
                state.selected = if selected { None } else { Some(view.step) };
            }
            if i + 1 < count {
                let (gap_rect, _) =
                    ui.allocate_exact_size(egui::vec2(gap, height), egui::Sense::hover());
                ui.painter().text(
                    gap_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "›",
                    theme::font(theme::TITLE_SIZE),
                    theme::rgb(theme.text_faint),
                );
            }
        }
    });
    match clicked_run {
        Some(Run::Jobs(_, jobs)) => state.enqueue(ctx.jobs, jobs),
        Some(Run::Confirm(_, message, jobs)) => state.confirm = Some((message, jobs)),
        Some(Run::Export) => ctx.actions.push(Action::Go(Page::Deliver)),
        Some(Run::None) | None => {}
    }
}

/// The queue's progress, when it holds more than one job, or why it stopped.
fn queue_line(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    if let Some(why) = state.stopped.clone() {
        ui.horizontal(|ui| {
            ui.colored_label(
                theme::rgb(ctx.theme.danger),
                format!("Queue stopped: {why}"),
            );
            if ui.small_button("Dismiss").clicked() {
                state.stopped = None;
            }
        });
        ui.add_space(theme::SPACE_SM);
    }
    if state.total < 2 {
        return;
    }
    let done = state.started.saturating_sub(1);
    let label = state
        .current
        .and_then(|id| ctx.jobs.get(id))
        .map(|j| j.label.clone())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(format!(
            "Queue: {} of {} · {label}",
            state.started.min(state.total),
            state.total
        ));
        if !state.queue.is_empty() && ui.small_button("Clear the rest").clicked() {
            state.queue.clear();
            state.total = state.started;
        }
    });
    let id = egui::Id::unique("night-queue-meter");
    widgets::meter(
        ui,
        ctx.theme,
        id,
        done as f32 / state.total.max(1) as f32,
        ctx.theme.accent,
    );
    ui.add_space(theme::SPACE_SM);
}

/// The log of the selected step's latest job (or the queue's current one).
fn log(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let job = match state.selected {
        Some(step) => ctx.jobs.list.iter().rev().find(|j| step.owns(&j.label)),
        None => state
            .current
            .and_then(|id| ctx.jobs.get(id))
            .or_else(|| ctx.jobs.list.last()),
    };
    let Some(job) = job else {
        if let Some(step) = state.selected {
            widgets::muted(
                ui,
                ctx.theme,
                &format!("{} has not run in this session.", step.title()),
            );
        }
        return;
    };
    let theme = ctx.theme;
    let (status, color) = match &job.status {
        Status::Running => (
            format!("running for {:.0}s", job.started.elapsed().as_secs_f32()),
            theme.accent,
        ),
        Status::Done => (
            format!("done in {:.1}s", job.took.unwrap_or(0.0)),
            theme.success,
        ),
        Status::Failed(e) => (
            format!("failed: {}", e.lines().next().unwrap_or_default()),
            theme.danger,
        ),
    };
    let elapsed = job.started.elapsed().as_secs_f32();
    let reported = job.progress.fraction().map(|f| {
        let left = job
            .progress
            .remaining(elapsed)
            .map(|s| format!(" \u{b7} {} left", crate::project::duration_text(s)))
            .unwrap_or_default();
        (
            f,
            format!("{}/{}{left}", job.progress.done, job.progress.total),
        )
    });
    let fraction = match (&job.status, reported, progress(&job.lines)) {
        (Status::Running, Some(r), _) => Some(r),
        (Status::Running, None, Some((n, m))) => Some((n as f32 / m as f32, format!("{n}/{m}"))),
        (Status::Running, None, None) if job.label == "index" => {
            state.facts.videos.filter(|v| *v > 0).map(|v| {
                let n = state.facts.cached.min(v);
                (n as f32 / v as f32, format!("{n}/{v} clips analysed"))
            })
        }
        (Status::Running | Status::Done | Status::Failed(_), _, _) => None,
    };
    let retry =
        matches!(job.status, Status::Failed(_)).then(|| (job.label.clone(), job.cmd.clone()));
    let lines: Vec<String> = job.lines.iter().rev().take(14).rev().cloned().collect();
    let label = job.label.clone();
    let running = job.status == Status::Running;
    egui::Frame::new()
        .fill(theme::rgb(theme.panel))
        .stroke(egui::Stroke::new(1.0, theme::rgb(theme.border)))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin(theme::SPACE_MD))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if running {
                    ui.add(egui::Spinner::new().size(14.0));
                }
                ui.label(egui::RichText::new(format!("ave {label}")).strong());
                ui.label(egui::RichText::new(status).color(theme::rgb(color)));
                if let Some((label, cmd)) = retry.clone() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Retry").clicked() {
                            state.enqueue(ctx.jobs, vec![(label, cmd)]);
                        }
                    });
                }
            });
            if let Some((f, text)) = fraction {
                ui.add_space(theme::SPACE_XS);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .small()
                            .color(theme::rgb(theme.text_muted)),
                    );
                });
                widgets::meter(
                    ui,
                    theme,
                    egui::Id::unique(("night-progress", &label)),
                    f,
                    theme.accent,
                );
            }
            ui.add_space(theme::SPACE_XS);
            if lines.is_empty() {
                widgets::muted(ui, theme, if running { "Working…" } else { "No output." });
            }
            for line in &lines {
                ui.label(
                    egui::RichText::new(line)
                        .monospace()
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
            }
        });
}

/// The "are you sure" card for a re-run that replaces hand edits.
fn confirm(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let open = state.confirm.is_some();
    let message = state
        .confirm
        .as_ref()
        .map(|(m, _)| m.clone())
        .unwrap_or_default();
    let theme = ctx.theme;
    let outcome = modal::show(ui.ctx(), theme, "Replace your edits?", open, 420.0, |ui| {
        ui.label(&message);
        ui.add_space(theme::SPACE_LG);
        let mut choice = None;
        ui.horizontal(|ui| {
            if widgets::button(ui, theme, "Replace", widgets::Emphasis::Primary).clicked() {
                choice = Some(true);
            }
            if widgets::button(ui, theme, "Cancel", widgets::Emphasis::Secondary).clicked() {
                choice = Some(false);
            }
        });
        choice
    });
    match outcome.inner.flatten() {
        Some(true) => {
            if let Some((_, jobs)) = state.confirm.take() {
                state.enqueue(ctx.jobs, jobs);
            }
        }
        Some(false) => state.confirm = None,
        None if outcome.dismissed => state.confirm = None,
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::{Step, ago, progress};

    #[test]
    fn progress_is_read_from_the_newest_line_that_has_it() {
        let lines: Vec<String> = [
            "starting",
            "120/417 captioned, 300s",
            "130/417 captioned, 310s",
            "a path C:/x/y",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(progress(&lines), Some((130, 417)), "newest N/M");
        let none: Vec<String> = vec!["C:/footage/clips".to_owned(), "9/0".to_owned()];
        assert_eq!(progress(&none), None, "paths and N/0 are not progress");
    }

    #[test]
    fn job_labels_map_to_their_steps() {
        assert!(Step::Plans.owns("plan reel-30"), "plan jobs");
        assert!(Step::Export.owns("render reel-30"), "render jobs");
        assert!(Step::Music.owns("beats"), "beats is the music step");
        assert!(!Step::Index.owns("index-x"), "exact labels");
    }

    #[test]
    fn ages_read_like_people_say_them() {
        let now = std::time::SystemTime::now();
        assert_eq!(ago(now), "just now");
        let hour = std::time::Duration::from_secs(3 * 3600);
        assert_eq!(now.checked_sub(hour).map(ago).as_deref(), Some("3 h ago"));
    }
}

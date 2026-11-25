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

use crate::gui::ui::hint::Hint as _;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::commands::Cmd;
use crate::gui::app::{Action, Page};
use crate::gui::pages::PageCtx;
use crate::gui::ui::{icons, modal, theme, widgets};
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
    /// Steps that failed without stopping the queue (captions, one
    /// deliverable's draft), each with why.
    skipped: Vec<String>,
    /// The step whose log is shown.
    selected: Option<Step>,
    /// A re-run waiting for the user to confirm.
    confirm: Option<(String, Vec<(String, Cmd)>)>,
    /// Disk facts.
    facts: Facts,
}

impl State {
    /// Starts the next queued job when nothing queued is running. Stops the
    /// queue when a job failed, except where nothing after it depends on
    /// it: captions (every later step works without them, and the vision
    /// model is optional set-up) and one deliverable's draft (the others
    /// draft on their own). Those are noted in [`State::skipped`] and the
    /// queue goes on.
    pub(crate) fn tick(&mut self, jobs: &mut Jobs) {
        if let Some(id) = self.current {
            match jobs.get(id).map(|j| j.status.clone()) {
                Some(Status::Running) => return,
                Some(Status::Done) => {
                    // A draft that had to loosen the rules says so: worth a note.
                    if let Some(job) = jobs.get(id)
                        && let Some(name) = job.label.strip_prefix("plan ")
                        && let Some(line) =
                            job.lines.iter().find(|l| l.starts_with("rules loosened"))
                    {
                        self.skipped.push(format!("{name}: {line}"));
                    }
                    self.current = None;
                }
                None => self.current = None,
                Some(Status::Failed(e)) => {
                    let label = jobs.get(id).map(|j| j.label.clone()).unwrap_or_default();
                    let first = e.lines().next().unwrap_or_default().to_owned();
                    self.current = None;
                    let cancelled = crate::commands::is_cancelled(&e);
                    if let Some(name) = label.strip_prefix("plan ")
                        && !cancelled
                    {
                        self.skipped
                            .push(format!("{name} was not drafted: {first}"));
                    } else if label == "caption" && !cancelled {
                        self.skipped.push(format!(
                            "Captions were skipped ({first}). The rest runs without them; \
                             set captions up and run Caption on the Home screen to add them."
                        ));
                    } else {
                        self.stopped = Some(if cancelled {
                            format!("{label} cancelled; what it finished is kept")
                        } else {
                            format!("{label} failed: {first}")
                        });
                        self.queue.clear();
                        return;
                    }
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

    /// The steps the queue went on without, and why, one line each.
    pub(crate) fn skipped(&self) -> Option<String> {
        (!self.skipped.is_empty()).then(|| {
            self.skipped.join(
                "
",
            )
        })
    }

    /// The steps the queue went on without, one line each.
    pub(crate) fn skipped_lines(&self) -> &[String] {
        &self.skipped
    }

    /// Forgets the skipped steps (the person dismissed them).
    pub(crate) fn dismiss_skipped(&mut self) {
        self.skipped.clear();
    }

    /// Appends jobs to the queue, skipping any already queued or running.
    pub(crate) fn enqueue(&mut self, jobs: &Jobs, list: Vec<(String, Cmd)>) {
        if self.current.is_none() && self.queue.is_empty() {
            self.started = 0;
            self.total = 0;
            self.skipped.clear();
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
                        && crate::plan::hard_count(&crate::plan::check(
                            &plan, index, music, &p.people,
                        )) > 0
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
    /// A second, rarer action beside the button: its icon, its tooltip and
    /// what it does (re-analyse everything, caption again).
    more: Option<(char, &'static str, Run)>,
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

/// Clips analysis reads at once unless Settings says otherwise.
pub(crate) const DEFAULT_ANALYSIS_JOBS: usize = 4;

/// Clips analysis reads at once: a machine setting (Settings → Analysis),
/// held here so every place that queues an index run uses it.
static ANALYSIS_JOBS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(DEFAULT_ANALYSIS_JOBS);

/// Sets how many clips analysis reads at once (at least one).
pub(crate) fn set_analysis_jobs(n: usize) {
    ANALYSIS_JOBS.store(n.max(1), std::sync::atomic::Ordering::Relaxed);
}

/// How many clips analysis reads at once.
pub(crate) fn analysis_jobs() -> usize {
    ANALYSIS_JOBS.load(std::sync::atomic::Ordering::Relaxed)
}

/// The index step.
pub(crate) fn index_cmd(work: &Path, footage: &Path, force: bool) -> (String, Cmd) {
    (
        "index".to_owned(),
        Cmd::Index {
            footage: footage.to_path_buf(),
            work: work.to_path_buf(),
            jobs: analysis_jobs(),
            force,
        },
    )
}

/// The playback-copy step (see [`crate::proxy`]).
pub(crate) fn proxy_cmd(work: &Path, force: bool) -> (String, Cmd) {
    (
        "proxy".to_owned(),
        Cmd::Proxy {
            work: work.to_path_buf(),
            force,
        },
    )
}

/// Analysis as one run: the index, then playback copies of heavy clips.
pub(crate) fn index_jobs(work: &Path, footage: &Path, force: bool) -> Vec<(String, Cmd)> {
    vec![index_cmd(work, footage, force), proxy_cmd(work, false)]
}

/// Segments scoring below this are not worth a caption.
const CAPTION_MIN_Q: f32 = 0.15;

/// Caption requests in flight at once.
const CAPTION_PARALLEL: usize = 3;

/// The caption step.
pub(crate) fn caption_cmd(work: &Path) -> (String, Cmd) {
    (
        "caption".to_owned(),
        Cmd::Caption {
            work: work.to_path_buf(),
            min_q: CAPTION_MIN_Q,
            redo: false,
            parallel: CAPTION_PARALLEL,
        },
    )
}

/// The caption step over every segment, replacing captions there are.
fn recaption_cmd(work: &Path) -> (String, Cmd) {
    (
        "caption".to_owned(),
        Cmd::Caption {
            work: work.to_path_buf(),
            min_q: CAPTION_MIN_Q,
            redo: true,
            parallel: CAPTION_PARALLEL,
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
            // Segments the rules will avoid, not every note the analysis made.
            let avoid = &recipe.rules.reject_flags;
            let flagged = i
                .segments
                .iter()
                .filter(|s| s.flags.iter().any(|f| avoid.contains(f)))
                .count();
            let new = videos.filter(|v| *v > i.clips.len());
            let text = format!(
                "{} clips · {} segments · {flagged} to avoid",
                i.clips.len(),
                i.segments.len()
            );
            match new {
                Some(v) => (
                    Standing::Partial,
                    format!("{text} · {} new in the folder", v - i.clips.len()),
                ),
                None => match crate::proxy::missing(&p.work, i).len() {
                    0 => (Standing::Done, text),
                    n => (
                        Standing::Partial,
                        format!("{n} clips to copy for playback · {text}"),
                    ),
                },
            }
        }
        (None, Some(v)) => (Standing::Ready, format!("{v} clips to analyse")),
        (None, None) => (Standing::Blocked, "Set the footage folder".to_owned()),
    };
    let run = match &p.settings.footage {
        Some(footage) if p.index.is_some() => {
            Run::Jobs("Update", index_jobs(&work, footage, false))
        }
        Some(footage) => Run::Jobs("Run", index_jobs(&work, footage, false)),
        None => Run::None,
    };
    let more = p.settings.footage.as_ref().filter(|_| p.index.is_some()).map(|footage| {
        (
            icons::ROTATE_CCW,
            "Analyse every clip again from scratch",
            Run::Confirm(
                "Re-analyse all",
                "Analyse every clip again from scratch? This takes as long as the first time. Captions, people and cuts are kept.".to_owned(),
                index_jobs(&work, footage, true),
            ),
        )
    });
    out.push(View {
        step: Step::Index,
        standing,
        summary,
        when: f.index,
        run,
        more,
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
    let more = captioned.filter(|(c, _)| *c > 0).map(|_| {
        (
            icons::ROTATE_CCW,
            "Caption every segment again, replacing the captions there are",
            Run::Confirm(
                "Caption again",
                "Caption every segment again? The captions there are now are replaced.".to_owned(),
                vec![recaption_cmd(&work)],
            ),
        )
    });
    out.push(View {
        step: Step::Caption,
        standing,
        summary,
        when: captioned.filter(|(c, _)| *c > 0).and(f.index),
        run,
        more,
    });

    // Music.
    let beats = |t: &PathBuf| {
        (
            "beats".to_owned(),
            Cmd::Beats {
                track: Some(t.clone()),
                none: false,
                work: work.clone(),
            },
        )
    };
    let track = p.settings.track.as_ref();
    let (standing, summary, run) = match (&p.music, track) {
        _ if p.no_music() => (
            Standing::Done,
            "No music: paced every 2 s, the clips' own sound".to_owned(),
            Run::None,
        ),
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
            Standing::Ready,
            "Set the track in the settings below, or go without".to_owned(),
            Run::Jobs(
                "No music",
                vec![(
                    "beats".to_owned(),
                    Cmd::Beats {
                        track: None,
                        none: true,
                        work: work.clone(),
                    },
                )],
            ),
        ),
    };
    out.push(View {
        step: Step::Music,
        standing,
        summary,
        when: f.music,
        run,
        more: None,
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
        more: None,
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
            "Needs the footage analysed first".to_owned(),
            Run::None,
        )
    } else if have == 0 {
        (
            Standing::Ready,
            if total == 1 {
                "1 deliverable to draft".to_owned()
            } else {
                format!("{total} deliverables to draft")
            },
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
        more: None,
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
        (Standing::Ready, "Render and deliver the cuts".to_owned())
    } else {
        (Standing::Blocked, "Draft the cuts first".to_owned())
    };
    out.push(View {
        step: Step::Export,
        standing,
        summary,
        when: f.export.filter(|_| f.delivered > 0),
        run: Run::Export,
        more: None,
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
        return (Vec::new(), vec!["no project open".to_owned()]);
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
                jobs.extend(index_jobs(work, footage, false));
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
                track: Some(track.clone()),
                none: false,
                work: work.clone(),
            },
        )),
        None if !mapped => skipped.push("music: choose a track, or No music".to_owned()),
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
        ui.set_min_height(theme::CONTROL_HEIGHT);
        ui.label(
            egui::RichText::new("Pipeline")
                .font(theme::medium(ui.ctx(), theme::TITLE_SIZE))
                .color(theme::rgb(ctx.theme.text)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let busy = state.current.is_some() || !state.queue.is_empty();
            let mut tip = if all.is_empty() {
                "Everything that can run has run. Re-run single steps below.".to_owned()
            } else {
                let labels: Vec<&str> = all.iter().map(|(l, _)| l.as_str()).collect();
                format!(
                    "Runs only what is missing, in order: {}. Nothing you edited is replaced.",
                    labels.join(" → ")
                )
            };
            if !skipped.is_empty() {
                tip.push_str(&format!("\nSkips {}.", skipped.join("; ")));
            }
            let enabled = !all.is_empty() && !busy;
            let clicked = ui
                .add_enabled_ui(enabled, |ui| {
                    widgets::button_with(
                        ui,
                        ctx.theme,
                        widgets::ButtonKind::Outline,
                        Some(icons::PLAY),
                        "Run everything",
                    )
                })
                .inner
                .hint(tip)
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
        });
    });
    ui.add_space(theme::SPACE_MD);
    bar(ctx, state, ui, views);
    ui.add_space(theme::SPACE_MD);
    queue_line(ctx, state, ui);
    log(ctx, state, ui);
    confirm(ctx, state, ui);
}

/// The six steps side by side, joined by chevrons: calm cards (the panel
/// surface, a hairline, 10 px corners) with the step's number and name,
/// a badge for where it stands, what it produced, and its action.
fn bar(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui, views: Vec<View>) {
    // The chevrons' gutter and a card's height.
    const GAP: f32 = 18.0;
    const HEIGHT: f32 = 144.0;
    let theme = ctx.theme;
    let n = views.len().max(1) as f32;
    let width = ((ui.available_width() - GAP * (n - 1.0)) / n).max(120.0);
    let mut clicked_run: Option<Run> = None;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let count = views.len();
        for (i, view) in views.into_iter().enumerate() {
            let selected = state.selected == Some(view.step);
            let (rect, response) =
                ui.allocate_exact_size(egui::vec2(width, HEIGHT), egui::Sense::click());
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, true, selected, view.step.title())
            });
            if response.hovered() {
                let what = match view.step {
                    Step::Index => "measures every clip and makes playback copies of heavy ones",
                    Step::Caption => {
                        "describes each segment with the local vision model (optional)"
                    }
                    Step::Music => "maps the track's beats and drops, or paces cuts with no music",
                    Step::People => "groups the footage by who is in it, from the captions",
                    Step::Plans => "drafts a cut for each deliverable",
                    Step::Export => "renders the cuts into the output folder",
                };
                crate::gui::ui::hint::set(
                    ui.ctx(),
                    &format!(
                        "{}: {what}. Now: {}. Click for details.",
                        view.step.title(),
                        view.summary
                    ),
                    "",
                );
            }
            let lift = widgets::hover_t(ui, response.id, response.hovered());
            let (word, tone) = match view.standing {
                Standing::Done => ("done", widgets::Tone::Success),
                Standing::Partial => ("partly done", widgets::Tone::Warning),
                Standing::Ready => ("ready", widgets::Tone::Neutral),
                Standing::Blocked => ("waiting", widgets::Tone::Neutral),
                Standing::Busy => ("running", widgets::Tone::Neutral),
            };
            let border = if selected {
                egui::Stroke::new(1.0, theme::ring(theme))
            } else {
                egui::Stroke::new(
                    1.0,
                    theme::ink(
                        theme,
                        egui::lerp(f32::from(theme::INK_10)..=f32::from(theme::INK_20), lift)
                            .round() as u8,
                    ),
                )
            };
            let painter = ui.painter();
            painter.rect(
                rect,
                egui::CornerRadius::same(theme::RADIUS_LG),
                theme::rgb(theme.panel),
                border,
                egui::StrokeKind::Inside,
            );
            let pad = theme::SPACE_MD;
            let x = rect.left() + pad;
            let number =
                egui::Rect::from_min_size(egui::pos2(x, rect.top() + pad), egui::Vec2::splat(20.0));
            painter.circle_filled(number.center(), 10.0, theme::ink(theme, theme::INK_10));
            painter.text(
                number.center(),
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                theme::tabular(ui.ctx(), theme::SMALL_SIZE),
                theme::rgb(theme.text_muted),
            );
            painter.text(
                egui::pos2(number.right() + theme::SPACE_SM, number.center().y),
                egui::Align2::LEFT_CENTER,
                view.step.title(),
                theme::medium(ui.ctx(), theme::LARGE_SIZE),
                theme::rgb(theme.text),
            );
            let badge_y = number.bottom() + theme::SPACE_SM + 10.0;
            let badge_w = {
                // A badge is drawn from its right edge; measure it first.
                let galley = painter.layout_no_wrap(
                    word.to_owned(),
                    theme::medium(ui.ctx(), theme::SMALL_SIZE),
                    egui::Color32::PLACEHOLDER,
                );
                galley.size().x + theme::SPACE_SM + theme::SPACE_XS
            };
            super::recent::paint_badge(ui, theme, x + badge_w, badge_y, word, tone);
            let painter = ui.painter();
            if let Some(t) = view.when {
                painter.text(
                    egui::pos2(x + badge_w + theme::SPACE_SM, badge_y),
                    egui::Align2::LEFT_CENTER,
                    ago(t),
                    theme::font(theme::SMALL_SIZE),
                    theme::rgb(theme.text_faint),
                );
            }
            let summary = painter.layout(
                view.summary.clone(),
                theme::font(theme::BODY_SIZE),
                theme::rgb(theme.text_muted),
                rect.width() - pad * 2.0,
            );
            let summary_rect = egui::Rect::from_min_max(
                egui::pos2(x, badge_y + 14.0),
                egui::pos2(rect.right() - pad, badge_y + 14.0 + 32.0),
            );
            painter.with_clip_rect(summary_rect).galley(
                summary_rect.min,
                summary,
                theme::rgb(theme.text_muted),
            );
            let button = egui::Rect::from_min_size(
                egui::pos2(x, rect.bottom() - pad - theme::CONTROL_HEIGHT),
                egui::vec2(rect.width() - pad * 2.0, theme::CONTROL_HEIGHT),
            );
            let busy = view.standing == Standing::Busy;
            let label = match &view.run {
                Run::Jobs(label, _) | Run::Confirm(label, _, _) => Some(*label),
                Run::Export => Some("Open export"),
                Run::None => None,
            };
            if let Some(label) = label {
                let label = if busy { "Running\u{2026}" } else { label };
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(button)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                let pressed = child
                    .add_enabled_ui(!busy, |ui| widgets::outline_button(ui, theme, label))
                    .inner
                    .clicked();
                let more_pressed = view.more.as_ref().is_some_and(|(glyph, tip, _)| {
                    child
                        .add_enabled_ui(!busy, |ui| widgets::icon_btn(ui, theme, *glyph, tip, ""))
                        .inner
                        .clicked()
                });
                if more_pressed && let Some((_, _, more)) = view.more {
                    clicked_run = Some(more);
                    state.selected = Some(view.step);
                } else if pressed {
                    clicked_run = Some(view.run);
                    state.selected = Some(view.step);
                }
            }
            if response.clicked() {
                state.selected = if selected { None } else { Some(view.step) };
            }
            if i + 1 < count {
                let (gap_rect, _) =
                    ui.allocate_exact_size(egui::vec2(GAP, HEIGHT), egui::Sense::hover());
                icons::paint(
                    ui.painter(),
                    gap_rect,
                    icons::CHEVRON_RIGHT,
                    icons::SMALL,
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
    let theme = ctx.theme;
    if let Some(why) = state.stopped.clone() {
        if super::parts::notice(
            ui,
            theme,
            super::parts::Kind::Danger,
            &format!("Queue stopped: {why}"),
        ) {
            state.stopped = None;
        }
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
        ui.set_min_height(theme::CONTROL_HEIGHT);
        ui.label(
            egui::RichText::new(format!(
                "Queue: {} of {} \u{b7} {label}",
                state.started.min(state.total),
                state.total
            ))
            .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
            .color(theme::rgb(theme.text_muted)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !state.queue.is_empty()
                && widgets::ghost_button(ui, theme, "Clear the rest").clicked()
            {
                state.queue.clear();
                state.total = state.started;
            }
        });
    });
    super::parts::progress_bar(
        ui,
        theme,
        egui::Id::unique("night-queue-meter"),
        done as f32 / state.total.max(1) as f32,
    );
    ui.add_space(theme::SPACE_MD);
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
            theme.text_muted,
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
    let mark = match &job.status {
        Status::Running => super::parts::Mark::Running,
        Status::Done => super::parts::Mark::Done,
        Status::Failed(_) => super::parts::Mark::Failed,
    };
    egui::Frame::new()
        .fill(theme::rgb(theme.panel))
        .stroke(theme::ink_line(theme))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(theme::margin_xy(theme::SPACE_LG, theme::SPACE_MD))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.set_min_height(theme::CONTROL_HEIGHT);
                ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
                let rect = ui
                    .allocate_exact_size(egui::Vec2::splat(icons::SIZE), egui::Sense::hover())
                    .0;
                super::parts::mark(ui, theme, rect, mark);
                ui.label(
                    egui::RichText::new(format!("ave {label}"))
                        .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                        .color(theme::rgb(theme.text)),
                );
                ui.label(
                    egui::RichText::new(status)
                        .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                        .color(theme::rgb(color)),
                );
                if let Some((label, cmd)) = retry.clone() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::button_with(
                            ui,
                            theme,
                            widgets::ButtonKind::Outline,
                            Some(icons::ROTATE_CCW),
                            "Retry",
                        )
                        .clicked()
                        {
                            state.enqueue(ctx.jobs, vec![(label, cmd)]);
                        }
                    });
                }
            });
            if let Some((f, text)) = fraction {
                ui.add_space(theme::SPACE_XS);
                ui.label(
                    egui::RichText::new(text)
                        .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                        .color(theme::rgb(theme.text_muted)),
                );
                ui.add_space(theme::SPACE_XS);
                super::parts::progress_bar(
                    ui,
                    theme,
                    egui::Id::unique(("night-progress", &label)),
                    f,
                );
            }
            ui.add_space(theme::SPACE_SM);
            if lines.is_empty() {
                super::parts::muted(ui, theme, if running { "Working…" } else { "No output." });
            }
            for line in &lines {
                ui.label(
                    egui::RichText::new(line)
                        .font(theme::mono(theme::SMALL_SIZE))
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
    fn a_failed_caption_run_is_skipped_and_the_queue_goes_on() -> anyhow::Result<()> {
        // An empty work folder: captions fail (no shot log), and so does
        // the step after them, which must still be tried.
        let dir = tempfile::tempdir()?;
        let work = dir.path();
        let mut jobs = crate::jobs::Jobs::default();
        let mut state = super::State::default();
        state.enqueue(
            &jobs,
            vec![
                super::caption_cmd(work),
                (
                    "plan reel-15".to_owned(),
                    crate::commands::Cmd::Plan {
                        name: "reel-15".to_owned(),
                        work: work.to_path_buf(),
                        aspect: "9x16".to_owned(),
                        seconds: 15.0,
                        end_card: String::new(),
                        card_bars: 2,
                    },
                ),
                (
                    "people".to_owned(),
                    crate::commands::Cmd::People {
                        work: work.to_path_buf(),
                        force: false,
                    },
                ),
            ],
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        state.tick(&mut jobs);
        while state.active() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
            jobs.poll();
            state.tick(&mut jobs);
        }
        assert!(!state.active(), "the queue finished");
        let skipped = state.skipped().unwrap_or_default();
        assert!(
            skipped.contains("Captions were skipped"),
            "the caption failure is noted, not fatal: {skipped:?}"
        );
        assert!(
            skipped.contains("reel-15 was not drafted"),
            "one deliverable's failed draft is noted, not fatal: {skipped:?}"
        );
        assert!(
            state
                .stopped()
                .is_some_and(|s| s.starts_with("people failed")),
            "the step after captions ran, and its failure stops the queue: {:?}",
            state.stopped()
        );
        Ok(())
    }

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

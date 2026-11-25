// ============================================================================
// Module:       commands
// Description:  The pipeline subcommands (index, caption, beats, plan, render),
//               what each one does, and the progress reports and cancel flag
//               they share with whoever runs them.
//
// Dependencies: clap (derive), std::io for output; crate::project (status)
// ============================================================================

//! The pipeline subcommands, run headless from a terminal.
//!
//! The workflow, in order:
//!
//! 1. `ave index <footage> --work <dir>`: analyse every clip, write the shot log.
//! 2. `ave caption --work <dir>`: add vision-model captions to the shot log.
//! 3. `ave beats <track> --work <dir>`: tempo, bars and the drop of the music.
//! 4. `ave plan <edit> --work <dir>`: draft a beat-synced plan from the index.
//! 5. `ave check <plan>`: enforce the edit rules on a (possibly hand-edited) plan.
//! 6. `ave render <plan>` / `ave otio <plan>`: an mp4, or a timeline for an editor.
//!
//! Commands write to the `out` they are handed instead of reaching for
//! standard output themselves, so tests can read back what a user saw.
//!
//! **Progress.** A long step reports as it goes through a [`Progress`]: one
//! [`Report`] per clip or segment (`index 12/81 MVI_7925: done 3.2s eta
//! 40s`) and one per stage (`render: encoding`). The text form is what a
//! terminal shows (through the log) and what the GUI's job list parses back
//! ([`Report::parse`]), so there is one format, readable by both. The same
//! [`Progress`] carries a cancel flag that the engine loops check between
//! clips and segments; a cancelled step fails with [`CANCELLED`], keeping
//! everything it finished.
//!
//! Index and caption runs also record per-clip status and measured
//! throughput in `<work>/status.json` ([`crate::project::NightStatus`]), so
//! a reopened night shows what is done and estimates come from this
//! machine's real speed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The error text of a step stopped by its cancel flag.
pub(crate) const CANCELLED: &str = "cancelled";

/// Steps that report progress, as their reports name them.
const STEPS: [&str; 7] = [
    "index", "caption", "beats", "people", "plan", "render", "otio",
];

/// Where one item (a clip, a segment) of a step stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemState {
    /// Waiting its turn.
    Queued,
    /// Being worked on now.
    Started,
    /// Already done by an earlier run; nothing to do.
    Cached,
    /// Finished.
    Done,
    /// Gave up on it; the report carries the reason.
    Failed,
}

impl ItemState {
    /// The word in a report line.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Started => "started",
            Self::Cached => "cached",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    /// The state a report word names.
    fn from_word(word: &str) -> Option<Self> {
        [
            Self::Queued,
            Self::Started,
            Self::Cached,
            Self::Done,
            Self::Failed,
        ]
        .into_iter()
        .find(|s| s.word() == word)
    }

    /// Whether the item needs nothing more.
    pub(crate) fn finished(self) -> bool {
        match self {
            Self::Cached | Self::Done | Self::Failed => true,
            Self::Queued | Self::Started => false,
        }
    }
}

/// One progress report.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Report {
    /// A step moved to a new stage: `render: encoding`.
    Stage {
        /// The step (`beats`, `render`...).
        step: String,
        /// What it is doing now.
        text: String,
    },
    /// One item changed state: `index 12/81 MVI_7925: done 3.2s eta 40s`.
    Item {
        /// The step.
        step: String,
        /// Items finished so far, this one included when it just finished.
        n: usize,
        /// Items in the run.
        total: usize,
        /// The clip or segment id.
        item: String,
        /// Its new state.
        state: ItemState,
        /// Seconds this item took, once finished.
        secs: Option<f32>,
        /// Seconds the step expects to need for the rest.
        eta: Option<f32>,
        /// Why it failed.
        reason: Option<String>,
    },
}

/// Separates a failed item's report from its reason.
const REASON: &str = " \u{2014} ";

impl Report {
    /// A stage report.
    pub(crate) fn stage(step: &str, text: impl Into<String>) -> Self {
        Self::Stage {
            step: step.to_owned(),
            text: text.into(),
        }
    }

    /// An item report with no timing or reason yet.
    pub(crate) fn item(
        step: &str,
        n: usize,
        total: usize,
        item: impl Into<String>,
        state: ItemState,
    ) -> Self {
        Self::Item {
            step: step.to_owned(),
            n,
            total,
            item: item.into(),
            state,
            secs: None,
            eta: None,
            reason: None,
        }
    }

    /// With the seconds the item took.
    #[must_use]
    pub(crate) fn took(mut self, value: f32) -> Self {
        if let Self::Item { secs, .. } = &mut self {
            *secs = Some(value);
        }
        self
    }

    /// With the step's remaining-time estimate.
    #[must_use]
    pub(crate) fn remaining(mut self, value: Option<f32>) -> Self {
        if let Self::Item { eta, .. } = &mut self {
            *eta = value;
        }
        self
    }

    /// With the reason it failed (first line only: a report is one line).
    #[must_use]
    pub(crate) fn because(mut self, why: &str) -> Self {
        if let Self::Item { reason, .. } = &mut self {
            *reason = Some(why.lines().next().unwrap_or_default().trim().to_owned());
        }
        self
    }

    /// The one-line text form.
    pub(crate) fn line(&self) -> String {
        match self {
            Self::Stage { step, text } => format!("{step}: {text}"),
            Self::Item {
                step,
                n,
                total,
                item,
                state,
                secs,
                eta,
                reason,
            } => {
                let mut s = format!("{step} {n}/{total} {item}: {}", state.word());
                if let Some(secs) = secs {
                    s.push_str(&format!(" {secs:.1}s"));
                }
                if let Some(eta) = eta {
                    s.push_str(&format!(" eta {:.0}s", eta.max(0.0)));
                }
                if let Some(reason) = reason {
                    s.push_str(REASON);
                    s.push_str(reason);
                }
                s
            }
        }
    }

    /// Reads a report back from its text form; `None` for any other line.
    pub(crate) fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        let (step, rest) = line.split_once([' ', ':'])?;
        if !STEPS.contains(&step) {
            return None;
        }
        if let Some(text) = line.strip_prefix(step).and_then(|r| r.strip_prefix(": ")) {
            return Some(Self::stage(step, text));
        }
        let (count, rest) = rest.split_once(' ')?;
        let (n, total) = count.split_once('/')?;
        let (n, total): (usize, usize) = (n.parse().ok()?, total.parse().ok()?);
        let (head, reason) = match rest.split_once(REASON) {
            Some((h, r)) => (h, Some(r.to_owned())),
            None => (rest, None),
        };
        let (item, tail) = head.rsplit_once(": ")?;
        let mut words = tail.split_whitespace();
        let state = ItemState::from_word(words.next()?)?;
        let seconds = |w: &str| w.strip_suffix('s').and_then(|v| v.parse::<f32>().ok());
        let (mut secs, mut eta) = (None, None);
        while let Some(word) = words.next() {
            if word == "eta" {
                eta = words.next().and_then(seconds);
            } else if let Some(v) = seconds(word) {
                secs = Some(v);
            }
        }
        Some(Self::Item {
            step: step.to_owned(),
            n,
            total,
            item: item.to_owned(),
            state,
            secs,
            eta,
            reason,
        })
    }
}

/// Seconds left for `total` items when `done` of them took `elapsed`.
pub(crate) fn eta(elapsed: f32, done: usize, total: usize) -> Option<f32> {
    (done > 0 && total >= done).then(|| elapsed / done as f32 * (total - done) as f32)
}

/// Something that receives reports.
type Sink = Arc<dyn Fn(&Report) + Send + Sync>;

/// Where a step's reports go, and the flag that asks it to stop.
///
/// Cheap to clone; clones share the flag and the sinks, so the engine's
/// worker threads can each hold one.
#[derive(Clone, Default)]
pub(crate) struct Progress {
    /// Every receiver of reports.
    sinks: Vec<Sink>,
    /// Set to ask the step to stop.
    cancel: Arc<AtomicBool>,
}

impl std::fmt::Debug for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Progress")
            .field("sinks", &self.sinks.len())
            .field("cancelled", &self.cancelled())
            .finish()
    }
}

impl Progress {
    /// Reports go to the log, one line each: what a terminal shows.
    pub(crate) fn logged() -> Self {
        Self::default().tee(|r| log::info!("{}", r.line()))
    }

    /// Adds a receiver of reports.
    #[must_use]
    pub(crate) fn tee(mut self, sink: impl Fn(&Report) + Send + Sync + 'static) -> Self {
        self.sinks.push(Arc::new(sink));
        self
    }

    /// Sends a report to every receiver.
    pub(crate) fn report(&self, report: &Report) {
        for sink in &self.sinks {
            sink(report);
        }
    }

    /// Reports a stage.
    pub(crate) fn stage(&self, step: &str, text: impl Into<String>) {
        self.report(&Report::stage(step, text));
    }

    /// The flag that asks the step to stop; set it from any thread.
    pub(crate) fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Whether the step has been asked to stop.
    pub(crate) fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// An error when the step has been asked to stop: `progress.check()?`
    /// between items.
    pub(crate) fn check(&self) -> anyhow::Result<()> {
        if self.cancelled() {
            anyhow::bail!(CANCELLED);
        }
        Ok(())
    }
}

/// Whether an error chain is a step stopping on request.
pub(crate) fn is_cancelled(error: &str) -> bool {
    error.lines().next().is_some_and(|l| l.trim() == CANCELLED)
}

/// Records what index and caption runs report into `<work>/status.json`.
struct Recorder {
    /// The work folder.
    work: PathBuf,
    /// What has been recorded so far.
    inner: Mutex<Recorded>,
}

/// The recorder's running notes.
#[derive(Default)]
struct Recorded {
    /// The status file as loaded, updated per report.
    status: crate::project::NightStatus,
    /// First item started, for wall-clock speed.
    first: Option<Instant>,
    /// Last item finished.
    last: Option<Instant>,
    /// Clips analysed (not cached) this run.
    analysed: Vec<String>,
    /// Segments captioned this run.
    captioned: usize,
}

impl Recorder {
    /// Starts recording for `work`.
    fn new(work: &Path) -> Self {
        Self {
            work: work.to_path_buf(),
            inner: Mutex::new(Recorded {
                status: crate::project::NightStatus::load(work),
                ..Recorded::default()
            }),
        }
    }

    /// Notes one report.
    fn observe(&self, report: &Report) {
        let Report::Item {
            step,
            item,
            state,
            secs,
            reason,
            ..
        } = report
        else {
            return;
        };
        let Ok(mut r) = self.inner.lock() else {
            return;
        };
        let now = Instant::now();
        match *state {
            ItemState::Started => {
                r.first.get_or_insert(now);
            }
            ItemState::Done => {
                r.last = Some(now);
                if step == "index" {
                    r.analysed.push(item.clone());
                } else {
                    r.captioned += 1;
                }
            }
            ItemState::Queued | ItemState::Cached | ItemState::Failed => {}
        }
        r.status
            .observe(step, item, *state, *secs, reason.as_deref());
    }

    /// Works out throughput from this run and saves the status file.
    fn finish(&self) {
        let Ok(mut r) = self.inner.lock() else {
            return;
        };
        let wall = match (r.first, r.last) {
            (Some(a), Some(b)) if b > a => Some(b.duration_since(a).as_secs_f32()),
            (Some(_) | None, Some(_) | None) => None,
        };
        let analysed = std::mem::take(&mut r.analysed);
        if let Ok(index) = crate::index::Index::load(&self.work) {
            let source: f64 = index
                .clips
                .iter()
                .filter(|c| analysed.contains(&c.id))
                .map(|c| c.probe.duration)
                .sum();
            r.status
                .measure_index(&index, wall.filter(|_| !analysed.is_empty()), source);
        }
        if r.captioned >= 5
            && let Some(wall) = wall
        {
            r.status.throughput.caption_secs = Some(wall / (r.captioned - 1) as f32);
        }
        r.status.settle();
        if let Err(e) = r.status.save(&self.work) {
            log::warn!("status.json: {e:#}");
        }
    }
}

/// The subcommands.
#[derive(clap::Subcommand, Clone, Debug, PartialEq)]
pub(crate) enum Cmd {
    /// Analyse every video in a folder and write index.json + shotlog.md.
    Index {
        /// Folder of source clips.
        footage: PathBuf,
        /// Work folder for the index, plans and renders.
        #[arg(long)]
        work: PathBuf,
        /// Clips analysed at once.
        #[arg(long, default_value_t = 3)]
        jobs: usize,
        /// Re-analyse even when the cache is current.
        #[arg(long)]
        force: bool,
    },
    /// Caption indexed segments with the local vision model.
    Caption {
        /// Work folder (from `ave index`).
        #[arg(long)]
        work: PathBuf,
        /// Skip segments scoring below this quality.
        #[arg(long, default_value_t = 0.15)]
        min_q: f32,
        /// Caption again even where a caption exists.
        #[arg(long)]
        redo: bool,
        /// Requests in flight at once.
        #[arg(long, default_value_t = 3)]
        parallel: usize,
    },
    /// Map a music track: tempo, beats, bars, drops -> <work>/music.json
    /// (or, with --none, cut without music).
    Beats {
        /// The track.
        #[arg(required_unless_present = "none")]
        track: Option<PathBuf>,
        /// No music: cuts paced every two seconds, with the clips' own sound.
        #[arg(long, conflicts_with = "track")]
        none: bool,
        /// Work folder.
        #[arg(long)]
        work: PathBuf,
    },
    /// Make playback copies of heavy clips (4K, HEVC) -> <work>/proxies/, so the
    /// editor plays smoothly; renders still use the originals.
    Proxy {
        /// Work folder.
        #[arg(long)]
        work: PathBuf,
        /// Make every proxy again, even the up-to-date ones.
        #[arg(long)]
        force: bool,
    },
    /// Draft <work>/people.toml (who is in which clip) from the captions.
    People {
        /// Work folder.
        #[arg(long)]
        work: PathBuf,
        /// Overwrite an existing people.toml.
        #[arg(long)]
        force: bool,
    },
    /// Draft a beat-synced plan -> <work>/plans/<name>.toml.
    Plan {
        /// Output name, e.g. `reel-30`.
        name: String,
        /// Work folder.
        #[arg(long)]
        work: PathBuf,
        /// Frame shape: 9x16, 4x5 or 16x9.
        #[arg(long, default_value = "9x16")]
        aspect: String,
        /// Target length in seconds.
        #[arg(long, default_value_t = 30.0)]
        seconds: f32,
        /// End card image.
        #[arg(long, default_value = "")]
        end_card: String,
        /// Bars the end card holds.
        #[arg(long, default_value_t = 2)]
        card_bars: usize,
    },
    /// Check a plan against its rules; exits 1 only on a hard finding.
    ///
    /// Every finding prints as `<plan> <where> [<severity> <id>] <message>`, where
    /// <where> is `shot N`, `cut N|N+1` (a seam check) or `plan` (the whole
    /// plan), then a verdict line. Hard findings (the plan is invalid or the
    /// render would be wrong: beat-count, bar-order, past-track, off-drop,
    /// missing-clip, outside-clip, bad-span, crop-outside, empty-shot) fail the check and
    /// stop `ave render` without --force. Soft findings are taste rules
    /// bent (drop-subject, flash-shot, flagged, bad-moment, wide-action,
    /// same-stretch, repeat-performer, lookalike, reused, jump-cut,
    /// exposure-jump, colour-jump): reported, never fatal.
    Check {
        /// Plan file.
        plan: PathBuf,
        /// Work folder.
        #[arg(long)]
        work: PathBuf,
    },
    /// Render a plan to <work>/renders/<name>.mp4 (+ a 720p preview).
    Render {
        /// Plan file.
        plan: PathBuf,
        /// Work folder.
        #[arg(long)]
        work: PathBuf,
        /// Video encoder.
        #[arg(long, value_enum, default_value_t = crate::render::Encoder::X264)]
        encoder: crate::render::Encoder,
        /// Render even if the check has hard findings.
        #[arg(long)]
        force: bool,
    },
    /// Export a plan as an OpenTimelineIO timeline -> <work>/renders/<name>.otio.
    Otio {
        /// Plan file.
        plan: PathBuf,
        /// Work folder.
        #[arg(long)]
        work: PathBuf,
    },
    /// Serve the engine as MCP tools on stdio, for an agent (Claude Code).
    Mcp {
        /// Work folder (a night's _ave).
        #[arg(long)]
        work: PathBuf,
    },
    /// Print one segment's numbers and caption, or a shot-log summary.
    Show {
        /// Work folder.
        #[arg(long)]
        work: PathBuf,
        /// Segment ids (`MVI_7987/3`); none prints a summary.
        ids: Vec<String>,
    },
}

/// Runs the command `options` selects, reporting progress to the log.
pub(crate) fn execute(command: Cmd, out: &mut impl Write) -> anyhow::Result<()> {
    execute_with(command, out, &Progress::logged())
}

/// Runs a command, reporting to `progress` and stopping when it is
/// cancelled. Index and caption runs also update `<work>/status.json`.
pub(crate) fn execute_with(
    command: Cmd,
    out: &mut impl Write,
    progress: &Progress,
) -> anyhow::Result<()> {
    let recorder = match &command {
        Cmd::Index { work, .. } | Cmd::Caption { work, .. } => Some(Arc::new(Recorder::new(work))),
        Cmd::Beats { .. }
        | Cmd::Proxy { .. }
        | Cmd::People { .. }
        | Cmd::Plan { .. }
        | Cmd::Check { .. }
        | Cmd::Render { .. }
        | Cmd::Otio { .. }
        | Cmd::Mcp { .. }
        | Cmd::Show { .. } => None,
    };
    let progress = match &recorder {
        Some(r) => {
            let r = Arc::clone(r);
            progress.clone().tee(move |report| r.observe(report))
        }
        None => progress.clone(),
    };
    let result = run(command, out, &progress);
    if let Some(r) = recorder {
        r.finish();
    }
    result
}

/// Runs one command.
fn run(command: Cmd, out: &mut impl Write, progress: &Progress) -> anyhow::Result<()> {
    match command {
        Cmd::Index {
            footage,
            work,
            jobs,
            force,
        } => {
            let index = crate::index::run(&footage, &work, jobs, force, progress)?;
            writeln!(
                out,
                "{} clips, {} segments, {} skipped -> {}",
                index.clips.len(),
                index.segments.len(),
                index.skipped.len(),
                work.join("shotlog.md").display()
            )?;
        }
        Cmd::Caption {
            work,
            min_q,
            redo,
            parallel,
        } => {
            let n = crate::caption::run(&work, min_q, redo, parallel, progress)?;
            writeln!(
                out,
                "{n} segments captioned -> {}",
                work.join("shotlog.md").display()
            )?;
        }
        Cmd::Proxy { work, force } => {
            let n = crate::proxy::run(&work, force, progress)?;
            writeln!(
                out,
                "{n} playback copies -> {}",
                work.join("proxies").display()
            )?;
        }
        Cmd::Beats { track, none, work } => {
            let Some(track) = track.filter(|_| !none) else {
                crate::beats::write_no_music(&work)?;
                writeln!(
                    out,
                    "no music: cuts paced at {} BPM, with the clips' own sound",
                    crate::beats::NO_MUSIC_BPM
                )?;
                return Ok(());
            };
            let name = track
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            progress.stage("beats", format!("mapping {name}"));
            let map = crate::beats::reanalyze(&track, &work)?;
            progress.stage("beats", "done");
            writeln!(
                out,
                "{:.1} BPM, {} bars, {:.1}s",
                map.bpm,
                map.bars.len(),
                map.duration
            )?;
            for d in map.drops.iter().take(4) {
                writeln!(
                    out,
                    "drop candidate: bar {} at {:.2}s (score {:.2})",
                    d.bar, d.t, d.score
                )?;
            }
        }
        Cmd::People { work, force } => {
            let path = work.join("people.toml");
            anyhow::ensure!(
                force || !path.exists(),
                "{} exists; pass --force to replace it",
                path.display()
            );
            progress.stage("people", "drafting from the captions");
            let index = crate::index::Index::load(&work)?;
            std::fs::write(&path, crate::people::draft(&index))?;
            writeln!(out, "drafted {}", path.display())?;
        }
        Cmd::Plan {
            name,
            work,
            aspect,
            seconds,
            end_card,
            card_bars,
        } => {
            let aspect: crate::plan::Aspect = serde_json::from_value(serde_json::Value::String(
                aspect.clone(),
            ))
            .map_err(|_| anyhow::anyhow!("--aspect must be 9x16, 4x5 or 16x9, not {aspect}"))?;
            progress.stage("plan", format!("drafting {name}"));
            let index = crate::index::Index::load(&work)?;
            let map = crate::project::music_for(&work)?;
            let people = crate::people::People::load(&work)?;
            let spec = crate::draft::Spec {
                name: name.clone(),
                aspect,
                seconds,
                end_card,
                card_bars,
                rules: crate::recipe::Recipe::load(&work).0.rules,
            };
            let plan = crate::draft::draft(&index, &map, &people, &spec, &map.path)?;
            let path = work.join("plans").join(format!("{name}.toml"));
            plan.save(&path)?;
            let problems = crate::plan::check(&plan, &index, &map, &people);
            write!(out, "{}", crate::plan::report(&plan, &problems))?;
            if plan.rules != spec.rules {
                writeln!(
                    out,
                    "rules loosened to fit the footage: the same person {} shots apart, one clip's shots {:.0} s apart",
                    plan.rules.person_gap, plan.rules.same_clip_gap
                )?;
            }
            writeln!(out, "-> {}", path.display())?;
        }
        Cmd::Check { plan, work } => {
            let (plan, problems) = load_and_check(&plan, &work)?;
            write!(out, "{}", crate::plan::report(&plan, &problems))?;
            let hard = crate::plan::hard_count(&problems);
            anyhow::ensure!(hard == 0, "{hard} hard finding(s)");
        }
        Cmd::Render {
            plan,
            work,
            encoder,
            force,
        } => {
            progress.stage("render", "checking the plan");
            let (plan, problems) = load_and_check(&plan, &work)?;
            if !problems.is_empty() {
                write!(out, "{}", crate::plan::report(&plan, &problems))?;
                anyhow::ensure!(
                    force || crate::plan::hard_count(&problems) == 0,
                    "the plan fails its check; fix it or pass --force"
                );
            }
            let index = crate::index::Index::load(&work)?;
            let map = crate::project::music_for(&work)?;
            let video = work.join("renders").join(format!("{}.mp4", plan.name));
            progress.check()?;
            progress.stage("render", format!("encoding {}", plan.name));
            let secs = crate::render::render(&plan, &index, &map, &video, encoder)?;
            progress.check()?;
            progress.stage("render", "writing the 720p preview");
            let small = crate::render::preview(&video, plan.aspect)?;
            progress.stage("render", "done");
            writeln!(
                out,
                "{} ({secs:.2}s) + {}",
                video.display(),
                small.display()
            )?;
        }
        Cmd::Otio { plan, work } => {
            let plan = crate::plan::Plan::load(&plan)?;
            let index = crate::index::Index::load(&work)?;
            let map = crate::project::music_for(&work)?;
            let path = work.join("renders").join(format!("{}.otio", plan.name));
            progress.stage("otio", format!("exporting {}", plan.name));
            crate::otio::export(&plan, &index, &map, &path)?;
            writeln!(out, "{}", path.display())?;
        }
        Cmd::Mcp { work } => crate::mcp::serve(&work)?,
        Cmd::Show { work, ids } => {
            let index = crate::index::Index::load(&work)?;
            if ids.is_empty() {
                let flagged = index
                    .segments
                    .iter()
                    .filter(|s| !s.flags.is_empty())
                    .count();
                let captioned = index
                    .segments
                    .iter()
                    .filter(|s| s.caption.is_some())
                    .count();
                writeln!(
                    out,
                    "{} clips, {} segments ({flagged} flagged, {captioned} captioned), focus ref {:.3}",
                    index.clips.len(),
                    index.segments.len(),
                    index.sharp_ref
                )?;
            }
            for id in ids {
                match index.segments.iter().find(|s| s.id == id) {
                    Some(s) => writeln!(out, "{}", serde_json::to_string_pretty(s)?)?,
                    None => writeln!(out, "{id}: no such segment")?,
                }
            }
        }
    }
    Ok(())
}

/// Loads a plan and checks it against the work folder's index and music.
fn load_and_check(
    plan: &std::path::Path,
    work: &std::path::Path,
) -> anyhow::Result<(crate::plan::Plan, Vec<crate::plan::Finding>)> {
    let plan = crate::plan::Plan::load(plan)?;
    let index = crate::index::Index::load(work)?;
    let map = crate::project::music_for(work)?;
    let people = crate::people::People::load(work)?;
    let problems = crate::plan::check(&plan, &index, &map, &people);
    Ok((plan, problems))
}

#[cfg(test)]
mod tests {
    use super::{ItemState, Progress, Recorder, Report, eta, is_cancelled};
    use crate::project::ClipState;
    use std::sync::{Arc, Mutex};

    #[test]
    fn reports_round_trip_through_their_text() -> anyhow::Result<()> {
        let reports = [
            Report::stage("render", "encoding reel-30"),
            Report::item("index", 12, 81, "MVI_7925", ItemState::Done)
                .took(3.25)
                .remaining(Some(40.4)),
            Report::item("caption", 3, 9, "My Clip 2/4", ItemState::Failed)
                .because("the server said no: 500\nmore detail"),
            Report::item("index", 0, 81, "MVI_7930", ItemState::Started),
        ];
        for report in reports {
            let line = report.line();
            let back = Report::parse(&line)
                .ok_or_else(|| anyhow::anyhow!("did not parse back: {line}"))?;
            match (&report, &back) {
                (
                    Report::Item {
                        item: a,
                        state: sa,
                        reason: ra,
                        ..
                    },
                    Report::Item {
                        item: b,
                        state: sb,
                        reason: rb,
                        secs,
                        ..
                    },
                ) => {
                    assert_eq!((a, sa, ra), (b, sb, rb), "{line}");
                    if *sa == ItemState::Done {
                        assert_eq!(*secs, Some(3.2), "seconds to a tenth: {line}");
                    }
                }
                (Report::Stage { .. }, Report::Stage { .. }) => {
                    assert_eq!(report, back, "{line}");
                }
                (Report::Item { .. } | Report::Stage { .. }, _) => {
                    anyhow::bail!("kind changed: {line} -> {back:?}")
                }
            }
        }
        let eta_back =
            Report::parse("index 12/81 MVI_7925: done 3.2s eta 40s").and_then(|r| match r {
                Report::Item { eta, .. } => eta,
                Report::Stage { .. } => None,
            });
        assert_eq!(eta_back, Some(40.0), "eta read back");
        for junk in [
            "3 clips, 417 segments -> shotlog.md",
            "index of things",
            "drop candidate: bar 17 at 32.10s (score 0.80)",
            "caption 1/2 X: exploded",
            "C:/footage/clip 1/2: done",
        ] {
            assert_eq!(Report::parse(junk), None, "not a report: {junk}");
        }
        Ok(())
    }

    #[test]
    fn eta_extrapolates_the_pace_so_far() {
        assert_eq!(eta(20.0, 10, 40), Some(60.0), "2 s an item, 30 left");
        assert_eq!(eta(20.0, 0, 40), None, "nothing done, no guess");
    }

    #[test]
    fn progress_fans_out_and_cancels() -> anyhow::Result<()> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let progress = Progress::default().tee(move |r| {
            if let Ok(mut v) = log.lock() {
                v.push(r.line());
            }
        });
        progress.stage("beats", "mapping");
        assert!(progress.check().is_ok(), "not cancelled yet");
        let worker = progress.clone();
        progress
            .cancel_flag()
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let err = worker
            .check()
            .err()
            .ok_or_else(|| anyhow::anyhow!("a clone sees the cancel"))?;
        assert!(is_cancelled(&format!("{err:#}")), "{err:#}");
        assert!(
            !is_cancelled("ffprobe failed: cancelled by codec"),
            "only the bare word"
        );
        let lines = seen.lock().map(|v| v.clone()).unwrap_or_default();
        assert_eq!(lines, ["beats: mapping"], "reports reach the sink");
        Ok(())
    }

    #[test]
    fn a_cancelled_index_stops_before_the_next_clip() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let footage = dir.path().join("video");
        std::fs::create_dir_all(&footage)?;
        std::fs::write(footage.join("A.mov"), b"not a video")?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let progress = Progress::default().tee(move |r| {
            if let Ok(mut v) = log.lock() {
                v.push(r.line());
            }
        });
        progress
            .cancel_flag()
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let err = crate::index::run(&footage, &dir.path().join("_ave"), 2, false, &progress)
            .err()
            .ok_or_else(|| anyhow::anyhow!("a cancelled run must not succeed"))?;
        assert!(is_cancelled(&format!("{err:#}")), "{err:#}");
        let lines = seen.lock().map(|v| v.clone()).unwrap_or_default();
        assert!(
            !lines
                .iter()
                .any(|l| l.contains("failed") || l.contains("started")),
            "no clip was touched and nothing reported as failed: {lines:?}"
        );
        Ok(())
    }

    #[test]
    fn the_recorder_keeps_failures_and_interruptions() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let rec = Recorder::new(dir.path());
        rec.observe(&Report::item("index", 0, 3, "A", ItemState::Started));
        rec.observe(&Report::item("index", 1, 3, "A", ItemState::Done).took(2.0));
        rec.observe(&Report::item("index", 1, 3, "B", ItemState::Started));
        rec.observe(&Report::item("index", 2, 3, "C", ItemState::Failed).because("bad file"));
        rec.finish();
        let status = crate::project::NightStatus::load(dir.path());
        let state = |id: &str| status.clips.get(id).map(|c| c.state);
        assert_eq!(state("A"), Some(ClipState::Indexed), "done");
        assert_eq!(
            state("B"),
            Some(ClipState::Pending),
            "interrupted is pending again"
        );
        assert_eq!(state("C"), Some(ClipState::IndexFailed), "failed");
        assert_eq!(
            status.clips.get("C").and_then(|c| c.reason.as_deref()),
            Some("bad file"),
            "with its reason"
        );
        Ok(())
    }
}

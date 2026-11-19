// ============================================================================
// Module:       jobs
// Description:  Runs pipeline steps (index, caption, beats, plan, render...) on
//               background threads and reports their output, progress and
//               outcome; cancels them on request.
//
// Dependencies: crate::commands, crossbeam-channel
// ============================================================================

//! Background jobs.
//!
//! A job is one [`Cmd`] — exactly what `ave <step>` runs from a terminal —
//! executed on its own thread. Its report lines stream back over a channel;
//! [`Jobs::poll`] collects them each frame and returns the jobs that just
//! finished, so the app can reload the parts of the project they changed.
//!
//! Running the same `Cmd` the CLI runs keeps the GUI honest: anything the
//! window can do, a script or an agent can do too.
//!
//! **Progress.** The engine's progress reports ([`crate::commands::Report`])
//! arrive as ordinary lines in the job's log, in the same text a terminal
//! shows; [`Jobs::poll`] parses them into the job's [`JobProgress`] (items
//! done of total, the stage, the remaining time, each clip's or segment's
//! state), which is what the UI draws bars from.
//!
//! **Cancelling.** [`Jobs::cancel`] sets the job's cancel flag; the engine
//! checks it between clips and segments and stops, keeping what it
//! finished. A cancelled job ends as `Failed("cancelled")`;
//! [`Job::cancelled`] tells it apart from a real failure.

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::commands::{Cmd, ItemState, Progress, Report};

/// How a job ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Status {
    /// Still running.
    Running,
    /// Finished without error.
    Done,
    /// Failed; the error chain.
    Failed(String),
}

/// One item's latest reported state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ItemStatus {
    /// Where it stands.
    pub(crate) state: ItemState,
    /// Seconds it took, once finished.
    pub(crate) secs: Option<f32>,
    /// Why it failed.
    pub(crate) reason: Option<String>,
}

/// What a job's progress reports add up to.
#[derive(Clone, Debug, Default)]
pub(crate) struct JobProgress {
    /// The latest stage (`scoring segments`, `encoding reel-30`).
    pub(crate) stage: Option<String>,
    /// Items finished.
    pub(crate) done: usize,
    /// Items in the run (0 until the first item report).
    pub(crate) total: usize,
    /// The engine's latest remaining-time estimate, and when it was made.
    eta: Option<(f32, Instant)>,
    /// Every item reported, by id.
    pub(crate) items: BTreeMap<String, ItemStatus>,
    /// Items that failed.
    pub(crate) failed: usize,
}

impl JobProgress {
    /// Folds one report in.
    pub(crate) fn apply(&mut self, report: &Report) {
        match report {
            Report::Stage { text, .. } => self.stage = Some(text.clone()),
            Report::Item {
                n,
                total,
                item,
                state,
                secs,
                eta,
                reason,
                ..
            } => {
                self.total = *total;
                if state.finished() {
                    self.done = self.done.max(*n);
                }
                if let Some(eta) = eta {
                    self.eta = Some((*eta, Instant::now()));
                }
                if *state == ItemState::Failed {
                    self.failed += 1;
                }
                self.items.insert(
                    item.clone(),
                    ItemStatus {
                        state: *state,
                        secs: *secs,
                        reason: reason.clone(),
                    },
                );
            }
        }
    }

    /// Items done as a fraction of the run, once the run's size is known.
    pub(crate) fn fraction(&self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f32 / self.total as f32).clamp(0.0, 1.0))
    }

    /// Seconds left: the engine's estimate counted down since it was made,
    /// else the pace so far (`elapsed` seconds for the items done) carried
    /// over to the items left. `None` before anything is known, never a
    /// made-up number.
    pub(crate) fn remaining(&self, elapsed: f32) -> Option<f32> {
        if self.total > 0 && self.done >= self.total {
            return Some(0.0);
        }
        match self.eta {
            Some((eta, at)) => Some((eta - at.elapsed().as_secs_f32()).max(0.0)),
            None => crate::commands::eta(elapsed, self.done, self.total),
        }
    }

    /// Items being worked on now.
    pub(crate) fn working(&self) -> impl Iterator<Item = &str> {
        self.items
            .iter()
            .filter(|(_, s)| s.state == ItemState::Started)
            .map(|(id, _)| id.as_str())
    }
}

/// One job, as the UI shows it.
#[derive(Debug)]
pub(crate) struct Job {
    /// Stable id within this session.
    pub(crate) id: u64,
    /// Short label, e.g. `index`, `render reel-30`.
    pub(crate) label: String,
    /// The step it runs.
    pub(crate) cmd: Cmd,
    /// When it started.
    pub(crate) started: Instant,
    /// Seconds it took, once finished.
    pub(crate) took: Option<f32>,
    /// Its report lines so far.
    pub(crate) lines: Vec<String>,
    /// How it is going.
    pub(crate) status: Status,
    /// Its parsed progress reports.
    pub(crate) progress: JobProgress,
    /// Set to ask the job to stop.
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// Whether the job ended because it was cancelled.
    pub(crate) fn cancelled(&self) -> bool {
        matches!(&self.status, Status::Failed(e) if crate::commands::is_cancelled(e))
    }

    /// Whether a cancel has been asked for (it may still be stopping).
    pub(crate) fn cancelling(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// A message from a job thread.
enum Event {
    /// A report line.
    Line(u64, String),
    /// The job ended.
    End(u64, Result<(), String>),
}

/// Feeds `write!` output from a job back to the UI line by line.
struct LineSink {
    id: u64,
    tx: Sender<Event>,
    ctx: Option<egui::Context>,
    partial: String,
}

impl Write for LineSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.partial.push_str(&String::from_utf8_lossy(buf));
        while let Some(pos) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=pos).collect();
            let _ = self
                .tx
                .send(Event::Line(self.id, line.trim_end().to_owned()));
            if let Some(ctx) = &self.ctx {
                ctx.request_repaint();
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if !self.partial.is_empty() {
            let line = std::mem::take(&mut self.partial);
            let _ = self.tx.send(Event::Line(self.id, line));
        }
        Ok(())
    }
}

/// The job list and its channel.
pub(crate) struct Jobs {
    tx: Sender<Event>,
    rx: Receiver<Event>,
    next: u64,
    /// Every job this session, oldest first.
    pub(crate) list: Vec<Job>,
    ctx: Option<egui::Context>,
}

impl Default for Jobs {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            tx,
            rx,
            next: 1,
            list: Vec::new(),
            ctx: None,
        }
    }
}

impl Jobs {
    /// Lets job threads wake the UI when they have news.
    pub(crate) fn set_context(&mut self, ctx: &egui::Context) {
        self.ctx = Some(ctx.clone());
    }

    /// Starts `cmd` on a new thread and returns its id.
    pub(crate) fn spawn(&mut self, label: impl Into<String>, cmd: Cmd) -> u64 {
        let id = self.next;
        self.next += 1;
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        let thread_cmd = cmd.clone();
        let report_tx = tx.clone();
        let report_ctx = ctx.clone();
        let progress = Progress::default().tee(move |report| {
            let _ = report_tx.send(Event::Line(id, report.line()));
            if let Some(ctx) = &report_ctx {
                ctx.request_repaint();
            }
        });
        let cancel = progress.cancel_flag();
        std::thread::spawn(move || {
            let mut sink = LineSink {
                id,
                tx: tx.clone(),
                ctx: ctx.clone(),
                partial: String::new(),
            };
            let result = crate::commands::execute_with(thread_cmd, &mut sink, &progress)
                .map_err(|e| format!("{e:#}"));
            let _ = sink.flush();
            let _ = tx.send(Event::End(id, result));
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        self.list.push(Job {
            id,
            label: label.into(),
            cmd,
            started: Instant::now(),
            took: None,
            lines: Vec::new(),
            status: Status::Running,
            progress: JobProgress::default(),
            cancel,
        });
        id
    }

    /// Asks a running job to stop; it finishes the clip or segment in hand
    /// and ends as cancelled. Returns whether there was such a job.
    pub(crate) fn cancel(&mut self, id: u64) -> bool {
        match self
            .list
            .iter()
            .find(|j| j.id == id && j.status == Status::Running)
        {
            Some(job) => {
                job.cancel.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Collects news from job threads; returns the jobs that finished now.
    pub(crate) fn poll(&mut self) -> Vec<u64> {
        let mut finished = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Line(id, line) => {
                    if let Some(job) = self.list.iter_mut().find(|j| j.id == id) {
                        if let Some(report) = Report::parse(&line) {
                            job.progress.apply(&report);
                        }
                        job.lines.push(line);
                    }
                }
                Event::End(id, result) => {
                    if let Some(job) = self.list.iter_mut().find(|j| j.id == id) {
                        job.took = Some(job.started.elapsed().as_secs_f32());
                        job.status = match result {
                            Ok(()) => Status::Done,
                            Err(e) => Status::Failed(e),
                        };
                    }
                    finished.push(id);
                }
            }
        }
        finished
    }

    /// The job with this id.
    pub(crate) fn get(&self, id: u64) -> Option<&Job> {
        self.list.iter().find(|j| j.id == id)
    }

    /// Jobs still running.
    pub(crate) fn running(&self) -> impl Iterator<Item = &Job> {
        self.list.iter().filter(|j| j.status == Status::Running)
    }
}

#[cfg(test)]
mod tests {
    use super::{JobProgress, Jobs, Status};
    use crate::commands::{Cmd, ItemState, Report};

    /// Polls until `id` stops running or ten seconds pass.
    fn wait(jobs: &mut Jobs, id: u64) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while jobs.get(id).is_some_and(|j| j.status == Status::Running)
            && std::time::Instant::now() < deadline
        {
            let _ = jobs.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn a_failing_job_reports_its_error() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut jobs = Jobs::default();
        let id = jobs.spawn(
            "show",
            Cmd::Show {
                work: dir.path().to_path_buf(),
                ids: Vec::new(),
            },
        );
        wait(&mut jobs, id);
        let job = jobs
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("job vanished"))?;
        assert!(
            matches!(&job.status, Status::Failed(e) if e.contains("ave index")),
            "{:?}",
            job.status
        );
        assert!(job.took.is_some(), "a finished job has a duration");
        assert!(!job.cancelled(), "a real failure is not a cancel");
        Ok(())
    }

    #[test]
    fn progress_lines_add_up_to_bars_and_item_states() -> anyhow::Result<()> {
        let mut p = JobProgress::default();
        let lines = [
            "index: 3 clips to check",
            "index 0/3 A: started",
            "index 1/3 A: done 4.0s eta 8s",
            "index 2/3 B: cached",
            "index 3/3 C: failed \u{2014} ffprobe exited 1",
            "not a report 5/9",
        ];
        for line in lines {
            if let Some(r) = Report::parse(line) {
                p.apply(&r);
            }
        }
        assert_eq!((p.done, p.total), (3, 3), "counts from the reports");
        assert_eq!(p.fraction(), Some(1.0), "all done");
        assert_eq!(p.remaining(10.0), Some(0.0), "nothing left");
        assert_eq!(p.failed, 1, "one failure");
        let c = p
            .items
            .get("C")
            .ok_or_else(|| anyhow::anyhow!("C was reported"))?;
        assert_eq!(
            (c.state, c.reason.as_deref()),
            (ItemState::Failed, Some("ffprobe exited 1")),
            "the failure keeps its reason"
        );
        assert_eq!(
            p.items.get("A").and_then(|a| a.secs),
            Some(4.0),
            "seconds kept"
        );
        let mut half = JobProgress::default();
        half.apply(&Report::item("caption", 10, 40, "X/1", ItemState::Done));
        assert_eq!(
            half.remaining(20.0),
            Some(60.0),
            "without an engine estimate, the pace so far: 2 s an item, 30 left"
        );
        Ok(())
    }

    #[test]
    fn cancel_flags_only_running_jobs() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut jobs = Jobs::default();
        let id = jobs.spawn(
            "show",
            Cmd::Show {
                work: dir.path().to_path_buf(),
                ids: Vec::new(),
            },
        );
        wait(&mut jobs, id);
        assert!(!jobs.cancel(id), "a finished job cannot be cancelled");
        assert!(!jobs.cancel(999), "no such job");
        Ok(())
    }
}

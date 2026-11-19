// ============================================================================
// Module:       jobs
// Description:  Runs pipeline steps (index, caption, beats, plan, render...) on
//               background threads and reports their output and outcome.
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

use std::io::Write;
use std::time::Instant;

use crossbeam_channel::{Receiver, Sender, unbounded};

use crate::commands::Cmd;

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
        std::thread::spawn(move || {
            let mut sink = LineSink {
                id,
                tx: tx.clone(),
                ctx: ctx.clone(),
                partial: String::new(),
            };
            let result =
                crate::commands::execute(thread_cmd, &mut sink).map_err(|e| format!("{e:#}"));
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
        });
        id
    }

    /// Collects news from job threads; returns the jobs that finished now.
    pub(crate) fn poll(&mut self) -> Vec<u64> {
        let mut finished = Vec::new();
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::Line(id, line) => {
                    if let Some(job) = self.list.iter_mut().find(|j| j.id == id) {
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
    use super::{Jobs, Status};
    use crate::commands::Cmd;

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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while jobs.get(id).is_some_and(|j| j.status == Status::Running)
            && std::time::Instant::now() < deadline
        {
            let _ = jobs.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let job = jobs
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("job vanished"))?;
        assert!(
            matches!(&job.status, Status::Failed(e) if e.contains("ave index")),
            "{:?}",
            job.status
        );
        assert!(job.took.is_some(), "a finished job has a duration");
        Ok(())
    }
}

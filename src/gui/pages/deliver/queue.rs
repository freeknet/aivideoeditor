// ============================================================================
// Module:       gui::pages::deliver::queue
// Description:  The render queue: render, copy to the delivery name, OTIO,
//               one job at a time through ctx.jobs.
//
// Dependencies: crate::{commands, jobs, recipe}, super::files
// ============================================================================

//! The render queue.
//!
//! Renders are serial: two ffmpeg graphs at once only make both slower.
//! Each render is a `Cmd::Render` job (which writes
//! `<work>/renders/<plan>.mp4`); when it finishes, the video is copied to
//! the recipe's output folder under its delivery name on a background
//! thread while the next job starts. An OTIO export, when the recipe asks
//! for one, follows its render.

use std::collections::VecDeque;
use std::path::PathBuf;

use super::files;
use crate::commands::Cmd;
use crate::jobs::{Jobs, Status};

/// What a queued job is for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Kind {
    /// A render; the finished video is copied to `deliver`.
    Render {
        /// The video the render writes.
        video: PathBuf,
        /// Where it is delivered.
        deliver: PathBuf,
        /// Whether it was rendered despite failing its check.
        forced: bool,
    },
    /// An OTIO export, written to `timeline`.
    Otio {
        /// The timeline file.
        timeline: PathBuf,
    },
}

/// One queued job.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Task {
    /// The job label (`render reel-30`).
    pub(crate) label: String,
    /// The step.
    pub(crate) cmd: Cmd,
    /// What happens when it finishes.
    pub(crate) kind: Kind,
}

/// A file the queue produced.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Output {
    /// The file.
    pub(crate) path: PathBuf,
    /// What it is: `video`, `timeline`.
    pub(crate) what: &'static str,
    /// Size in bytes, when known.
    pub(crate) bytes: Option<u64>,
    /// Rendered from a plan that failed its check.
    pub(crate) forced: bool,
}

/// The queue's state.
#[derive(Debug, Default)]
pub(crate) struct Queue {
    /// Waiting, in order.
    pub(crate) pending: VecDeque<Task>,
    /// Running now: the job id and its task.
    pub(crate) current: Option<(u64, Task)>,
    /// Copies in flight, with whether their render was forced.
    copies: Vec<(files::Copying, bool)>,
    /// What finished, newest last.
    pub(crate) outputs: Vec<Output>,
    /// What failed, as one line each.
    pub(crate) failures: Vec<String>,
}

impl Queue {
    /// Adds tasks to the end of the queue.
    pub(crate) fn push(&mut self, tasks: Vec<Task>) {
        self.pending.extend(tasks);
    }

    /// Whether anything is running, waiting or copying.
    pub(crate) fn busy(&self) -> bool {
        self.current.is_some() || !self.pending.is_empty() || !self.copies.is_empty()
    }

    /// Copies still running, by destination.
    pub(crate) fn copying(&self) -> impl Iterator<Item = &PathBuf> {
        self.copies.iter().map(|(c, _)| &c.to)
    }

    /// Advances the queue: finishes the running job's follow-up, collects
    /// finished copies, starts the next job.
    pub(crate) fn tick(&mut self, jobs: &mut Jobs, ctx: Option<&egui::Context>) {
        self.copies.retain(|(copy, forced)| match copy.poll() {
            None => true,
            Some(Ok(bytes)) => {
                self.outputs.push(Output {
                    path: copy.to.clone(),
                    what: "video",
                    bytes: Some(bytes),
                    forced: *forced,
                });
                false
            }
            Some(Err(e)) => {
                self.failures.push(e);
                false
            }
        });
        if let Some((id, task)) = &self.current {
            let status = jobs.get(*id).map(|j| j.status.clone());
            match status {
                Some(Status::Running) => return,
                Some(Status::Done) => match &task.kind {
                    Kind::Render {
                        video,
                        deliver,
                        forced,
                    } => {
                        let copy =
                            files::Copying::start(video.clone(), deliver.clone(), ctx.cloned());
                        self.copies.push((copy, *forced));
                    }
                    Kind::Otio { timeline } => self.outputs.push(Output {
                        path: timeline.clone(),
                        what: "timeline",
                        bytes: std::fs::metadata(timeline).ok().map(|m| m.len()),
                        forced: false,
                    }),
                },
                Some(Status::Failed(e)) => {
                    let first = e.lines().next().unwrap_or_default();
                    self.failures
                        .push(format!("{} failed: {first}", task.label));
                }
                None => {}
            }
            self.current = None;
        }
        if let Some(task) = self.pending.pop_front() {
            let id = jobs.spawn(task.label.clone(), task.cmd.clone());
            self.current = Some((id, task));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, Queue, Task};
    use crate::commands::Cmd;

    #[test]
    fn a_failing_job_is_reported_and_the_queue_moves_on() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut jobs = crate::jobs::Jobs::default();
        let mut queue = Queue::default();
        let task = |name: &str| Task {
            label: format!("otio {name}"),
            cmd: Cmd::Otio {
                plan: dir.path().join(format!("{name}.toml")),
                work: dir.path().to_path_buf(),
            },
            kind: Kind::Otio {
                timeline: dir.path().join(format!("{name}.otio")),
            },
        };
        queue.push(vec![task("a"), task("b")]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while queue.busy() && std::time::Instant::now() < deadline {
            let _ = jobs.poll();
            queue.tick(&mut jobs, None);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!queue.busy(), "the queue drained");
        assert_eq!(queue.failures.len(), 2, "{:?}", queue.failures);
        assert!(queue.outputs.is_empty(), "nothing produced");
        Ok(())
    }
}

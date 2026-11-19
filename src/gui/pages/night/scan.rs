// ============================================================================
// Module:       gui::pages::night::scan
// Description:  Reads a night's footage folder, clip cache, status file and
//               measured speeds on a worker thread, for the wizard's
//               estimates and the processing list.
//
// Dependencies: crossbeam-channel; crate::project
// ============================================================================

//! The footage scan.
//!
//! Listing the footage folder, reading every clip's cache entry and, for
//! clips never analysed, asking ffprobe for their length takes seconds on
//! a night of 80 clips, so it never runs on the UI thread. A [`Scan`] is
//! asked for what it should hold ([`Scan::want`]) every frame; it starts a
//! worker when the request changed and keeps showing the last result
//! until the new one lands.

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;

use crate::project::{FootageClip, NightStatus, Speeds};

/// What a scan found.
#[derive(Debug, Clone)]
pub(crate) struct Scanned {
    /// The work folder it was for.
    pub(crate) work: PathBuf,
    /// The footage folder's videos, in name order.
    pub(crate) clips: Vec<FootageClip>,
    /// Why the folder could not be listed.
    pub(crate) error: Option<String>,
    /// `<work>/status.json`.
    pub(crate) status: NightStatus,
    /// Measured speeds, when asked for.
    pub(crate) speeds: Option<Speeds>,
}

impl Scanned {
    /// Seconds of footage whose length is known.
    pub(crate) fn footage_secs(&self) -> f64 {
        self.clips.iter().filter_map(|c| c.duration).sum()
    }
}

/// What a scan is for: the folders, whether to probe and measure, and a
/// generation that forces a rescan when it moves.
#[derive(Debug, Clone, PartialEq)]
struct Key {
    work: PathBuf,
    footage: PathBuf,
    probe: bool,
    generation: usize,
}

/// A scan and its latest result.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    /// The worker in flight.
    rx: Option<Receiver<Scanned>>,
    /// What the worker in flight (or the last one) was for.
    key: Option<Key>,
    /// The latest result.
    pub(crate) result: Option<Scanned>,
}

impl Scan {
    /// Makes sure the result is (or will be) for these folders. `probe`
    /// asks ffprobe for lengths the cache does not know and measures the
    /// speeds; `generation` forces a rescan when it changes (a job ended).
    pub(crate) fn want(&mut self, work: &Path, footage: &Path, probe: bool, generation: usize) {
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(found) => {
                    self.result = Some(found);
                    self.rx = None;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => return,
                Err(crossbeam_channel::TryRecvError::Disconnected) => self.rx = None,
            }
        }
        let key = Key {
            work: work.to_path_buf(),
            footage: footage.to_path_buf(),
            probe,
            generation,
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        if self.result.as_ref().is_some_and(|r| r.work != work) {
            self.result = None;
        }
        self.key = Some(key);
        let (tx, rx) = crossbeam_channel::bounded(1);
        let (work, footage) = (work.to_path_buf(), footage.to_path_buf());
        std::thread::spawn(move || {
            let (clips, error) = match crate::project::scan_footage(&work, &footage, probe) {
                Ok(c) => (c, None),
                Err(e) => (Vec::new(), Some(format!("{e:#}"))),
            };
            let found = Scanned {
                status: NightStatus::load(&work),
                speeds: probe.then(|| crate::project::speeds(&work)),
                work,
                clips,
                error,
            };
            let _ = tx.send(found);
        });
        self.rx = Some(rx);
    }

    /// Whether a worker is still reading.
    pub(crate) fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// The result, when it is for `work`.
    pub(crate) fn for_work(&self, work: &Path) -> Option<&Scanned> {
        self.result.as_ref().filter(|r| r.work == work)
    }
}

#[cfg(test)]
mod tests {
    use super::Scan;

    #[test]
    fn a_scan_lands_and_reruns_only_when_asked() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let footage = dir.path().join("video");
        std::fs::create_dir_all(&footage)?;
        std::fs::write(footage.join("A.mov"), b"not really a video")?;
        let work = dir.path().join("_ave");
        let mut scan = Scan::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while scan.for_work(&work).is_none() && std::time::Instant::now() < deadline {
            scan.want(&work, &footage, false, 0);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let found = scan
            .for_work(&work)
            .ok_or_else(|| anyhow::anyhow!("the scan never landed"))?;
        assert_eq!(found.clips.len(), 1, "one video listed");
        assert!(!found.clips[0].cached, "never analysed");
        scan.want(&work, &footage, false, 0);
        assert!(!scan.busy(), "same request, no new worker");
        scan.want(&work, &footage, false, 1);
        assert!(scan.busy(), "a new generation rescans");
        Ok(())
    }
}

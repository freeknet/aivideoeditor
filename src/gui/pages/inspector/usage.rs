// ============================================================================
// Module:       gui::pages::inspector::usage
// Description:  The night's plans, cached by modification time, and which of
//               their shots use a given stretch of footage or a performer.
//
// Dependencies: crate::{plan, beats, people}
// ============================================================================

//! "Used in reel-30 shot 7": back-links from footage to the plans.
//!
//! Plans are small TOML files, but the inspector draws every frame, so they
//! are parsed once and re-read only when a file's modification time moves.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::beats::MusicMap;
use crate::people::People;
use crate::plan::Plan;

/// One plan file, parsed.
#[derive(Debug)]
struct Cached {
    path: PathBuf,
    modified: Option<SystemTime>,
    plan: Option<Plan>,
}

/// One shot that uses the footage asked about.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Use {
    /// Plan name (file stem).
    pub(super) plan: String,
    /// Shot number, 1-based.
    pub(super) shot: usize,
    /// Source range of the shot, seconds.
    pub(super) range: (f32, f32),
}

/// The plans cache.
#[derive(Debug, Default)]
pub(super) struct Plans {
    files: Vec<Cached>,
}

impl Plans {
    /// Brings the cache in line with `paths` (re-reading changed files).
    pub(super) fn refresh(&mut self, paths: &[PathBuf]) {
        self.files.retain(|c| paths.contains(&c.path));
        for path in paths {
            let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
            match self.files.iter_mut().find(|c| c.path == *path) {
                Some(c) if c.modified == modified => {}
                Some(c) => {
                    c.modified = modified;
                    c.plan = Plan::load(path).ok();
                }
                None => self.files.push(Cached {
                    path: path.clone(),
                    modified,
                    plan: Plan::load(path).ok(),
                }),
            }
        }
        self.files.sort_by(|a, b| a.path.cmp(&b.path));
    }

    /// The plan called `name`, if it parsed.
    pub(super) fn get(&self, name: &str) -> Option<&Plan> {
        self.files
            .iter()
            .find(|c| stem(&c.path) == name)
            .and_then(|c| c.plan.as_ref())
    }

    /// Every shot whose source range overlaps `t0..t1` of `clip`.
    pub(super) fn uses(&self, clip: &str, t0: f32, t1: f32, music: Option<&MusicMap>) -> Vec<Use> {
        let mut out = Vec::new();
        for c in &self.files {
            let Some(plan) = &c.plan else {
                continue;
            };
            let lengths = shot_lengths(plan, music);
            for (k, shot) in plan.shots.iter().enumerate() {
                let len = lengths.get(k).copied().unwrap_or(0.0);
                let range = (shot.t_in, shot.t_in + len.max(0.04));
                if shot.clip == clip && range.0 < t1.max(t0 + 0.01) && t0 < range.1 {
                    out.push(Use {
                        plan: stem(&c.path),
                        shot: k + 1,
                        range,
                    });
                }
            }
        }
        out
    }

    /// Every shot that shows `name`, by its own `who` or the people map.
    pub(super) fn uses_of_person(
        &self,
        name: &str,
        people: &People,
        music: Option<&MusicMap>,
    ) -> Vec<Use> {
        let mut out = Vec::new();
        for c in &self.files {
            let Some(plan) = &c.plan else {
                continue;
            };
            let lengths = shot_lengths(plan, music);
            for (k, shot) in plan.shots.iter().enumerate() {
                if people.who(shot) == name {
                    let len = lengths.get(k).copied().unwrap_or(0.0);
                    out.push(Use {
                        plan: stem(&c.path),
                        shot: k + 1,
                        range: (shot.t_in, shot.t_in + len),
                    });
                }
            }
        }
        out
    }
}

/// Each shot's length in seconds: from the music when it is mapped, else
/// at 120 bpm, which is near enough to tell which footage a shot covers.
pub(super) fn shot_lengths(plan: &Plan, music: Option<&MusicMap>) -> Vec<f32> {
    match music {
        Some(m) => plan.timing(m).into_iter().map(|(_, d)| d).collect(),
        None => plan.shots.iter().map(|s| s.beats as f32 * 0.5).collect(),
    }
}

/// A plan's name: its file stem.
fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `reel-30 #7, #9 · feed-30 #2`: uses grouped per plan.
pub(super) fn summary(uses: &[Use]) -> String {
    let mut parts: Vec<(String, Vec<usize>)> = Vec::new();
    for u in uses {
        match parts.iter_mut().find(|(p, _)| *p == u.plan) {
            Some((_, shots)) => shots.push(u.shot),
            None => parts.push((u.plan.clone(), vec![u.shot])),
        }
    }
    parts
        .into_iter()
        .map(|(p, shots)| {
            let shots: Vec<String> = shots.iter().map(|s| format!("#{s}")).collect();
            format!("{p} {}", shots.join(", "))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::{Plans, Use, summary};

    #[test]
    fn uses_group_per_plan() {
        let u = |plan: &str, shot| Use {
            plan: plan.to_owned(),
            shot,
            range: (0.0, 1.0),
        };
        assert_eq!(
            summary(&[u("reel-30", 7), u("reel-30", 9), u("feed-30", 2)]),
            "reel-30 #7, #9 · feed-30 #2",
            "grouped"
        );
    }

    #[test]
    fn a_plan_file_is_found_by_its_shots() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("reel-30.toml");
        std::fs::write(
            &path,
            "name = \"reel-30\"\naspect = \"9x16\"\nmusic = \"m.wav\"\nstart_bar = 0\n\
             drop_bar = 1\ncard_bar = 2\nend_bar = 3\n\n[[shot]]\nclip = \"MVI_1\"\nin = 10.0\nbeats = 4\n",
        )?;
        let mut plans = Plans::default();
        plans.refresh(&[path]);
        assert!(plans.get("reel-30").is_some(), "parsed");
        let uses = plans.uses("MVI_1", 11.0, 12.0, None);
        assert_eq!(uses.len(), 1, "overlaps shot 1: {uses:?}");
        assert!(
            plans.uses("MVI_1", 20.0, 21.0, None).is_empty(),
            "past the shot"
        );
        assert!(
            plans.uses("MVI_2", 11.0, 12.0, None).is_empty(),
            "another clip"
        );
        Ok(())
    }
}

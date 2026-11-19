// ============================================================================
// Module:       gui::pages::plan::history
// Description:  Plan versions: every save writes plans/<name>.toml and a
//               numbered copy under plans/.history/<name>/ with when and why.
//
// Dependencies: crate::plan, toml
// ============================================================================

//! Version history for plans.
//!
//! The live plan is `plans/<name>.toml`, the file every engine command
//! reads. Each save also writes `plans/.history/<name>/v<N>.toml`: the same
//! TOML under a short comment header saying when it was saved and what
//! changed, so a version is still a plan `ave check` / `ave render` accept.
//! The first save of a plan that has no history first keeps what was on
//! disk as `v1`, so the drafter's (or a person's) original is never lost.
//!
//! The live file is written through a temporary file and a rename, so a
//! crash mid-write cannot leave a half-written plan.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::plan::Plan;

/// One saved version.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Version {
    /// Version number, from 1.
    pub(super) n: u32,
    /// Its file.
    pub(super) path: PathBuf,
    /// When it was saved, `YYYY-MM-DD HH:MM UTC` (empty if unknown).
    pub(super) saved: String,
    /// What changed, e.g. `roll 4|5`.
    pub(super) change: String,
}

/// The live plan file.
pub(super) fn live_path(work: &Path, name: &str) -> PathBuf {
    work.join("plans").join(format!("{name}.toml"))
}

/// Where `name`'s versions live.
pub(super) fn dir(work: &Path, name: &str) -> PathBuf {
    work.join("plans").join(".history").join(name)
}

/// Seconds since the Unix epoch, now.
pub(super) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `unix` seconds as `YYYY-MM-DD HH:MM UTC` (civil-from-days, Howard
/// Hinnant's algorithm), so no date crate is needed for one label.
pub(super) fn format_utc(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs = unix % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        secs / 3600,
        (secs % 3600) / 60
    )
}

/// Every version of `name`, oldest first.
pub(super) fn list(work: &Path, name: &str) -> Vec<Version> {
    let Ok(entries) = std::fs::read_dir(dir(work, name)) else {
        return Vec::new();
    };
    let mut out: Vec<Version> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter_map(|path| {
            let stem = path.file_stem()?.to_str()?;
            let n: u32 = stem.strip_prefix('v')?.parse().ok()?;
            if path.extension().is_none_or(|x| x != "toml") {
                return None;
            }
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let header = |key: &str| {
                text.lines()
                    .take_while(|l| l.starts_with('#'))
                    .find_map(|l| l.strip_prefix(key))
                    .map(|v| v.trim().to_owned())
                    .unwrap_or_default()
            };
            Some(Version {
                n,
                saved: header("# saved"),
                change: header("# change"),
                path,
            })
        })
        .collect();
    out.sort_by_key(|v| v.n);
    out
}

/// Writes `text` to `path` through a temporary file and a rename.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// The plan's TOML without any comment header, for comparing versions.
fn body(text: &str) -> &str {
    let mut rest = text;
    while rest.starts_with('#') {
        rest = rest.split_once('\n').map_or("", |(_, r)| r);
    }
    rest.trim()
}

/// Adds `text` (a plan's TOML) as the next version of `name`, unless it is
/// the same as the latest one. Returns the version it is now.
pub(super) fn record(work: &Path, name: &str, text: &str, change: &str, unix: u64) -> Result<u32> {
    let versions = list(work, name);
    if let Some(last) = versions.last()
        && let Ok(prev) = std::fs::read_to_string(&last.path)
        && body(&prev) == body(text)
    {
        return Ok(last.n);
    }
    let n = versions.last().map_or(1, |v| v.n + 1);
    let change = change.replace(['\n', '\r'], " ");
    let stamped = format!(
        "# ave plan version {n}\n# saved {}\n# change {change}\n{}\n",
        format_utc(unix),
        body(text)
    );
    write_atomic(&dir(work, name).join(format!("v{n}.toml")), &stamped)?;
    Ok(n)
}

/// Keeps what is on disk for `name` as a version when the plan has no
/// history yet (before the first save, or before a draft overwrites it).
pub(super) fn keep_original(work: &Path, name: &str, unix: u64) -> Result<()> {
    if !list(work, name).is_empty() {
        return Ok(());
    }
    if let Ok(text) = std::fs::read_to_string(live_path(work, name)) {
        record(work, name, &text, "original", unix)?;
    }
    Ok(())
}

/// Saves `plan` as `name`'s live file and as the next version. Returns the
/// version number.
pub(super) fn save(work: &Path, name: &str, plan: &Plan, change: &str, unix: u64) -> Result<u32> {
    keep_original(work, name, unix)?;
    let text = toml::to_string_pretty(plan)?;
    write_atomic(&live_path(work, name), &text)?;
    record(work, name, &text, change, unix)
}

/// Loads one version.
pub(super) fn load(version: &Version) -> Result<Plan> {
    Plan::load(&version.path)
}

#[cfg(test)]
mod tests {
    use super::{format_utc, keep_original, list, live_path, load, save};
    use crate::plan::{Aspect, Plan, Rules, Shot};

    fn plan() -> Plan {
        Plan {
            name: "reel-15".into(),
            aspect: Aspect::Vertical,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 1,
            card_bar: 2,
            end_bar: 3,
            end_card: String::new(),
            rules: Rules::default(),
            shots: vec![Shot {
                clip: "A".into(),
                t_in: 1.0,
                beats: 8,
                cx: 0.5,
                who: String::new(),
                seg: String::new(),
                note: "hero".into(),
            }],
            finish: Default::default(),
        }
    }

    #[test]
    fn dates_format_in_utc() -> anyhow::Result<()> {
        assert_eq!(format_utc(0), "1970-01-01 00:00 UTC", "epoch");
        assert_eq!(
            format_utc(1_795_100_000),
            "2026-11-19 14:53 UTC",
            "a date this year"
        );
        assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC", "leap day");
        Ok(())
    }

    #[test]
    fn saves_keep_the_original_and_number_each_version() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let work = dir.path();
        let mut p = plan();
        p.save(&live_path(work, &p.name))?;

        p.shots[0].note = "edited".into();
        assert_eq!(
            save(work, "reel-15", &p, "note 1", 100)?,
            2,
            "original is v1"
        );
        let versions = list(work, &p.name);
        assert_eq!(versions.len(), 2, "two versions");
        assert_eq!(versions[0].change, "original", "v1 is what was on disk");
        assert_eq!(versions[1].change, "note 1", "v2 says what changed");
        assert_eq!(versions[1].saved, "1970-01-01 00:01 UTC", "and when");

        assert_eq!(
            save(work, "reel-15", &p, "same again", 200)?,
            2,
            "no-op saves"
        );
        p.shots[0].t_in = 2.0;
        assert_eq!(save(work, "reel-15", &p, "slip 1", 300)?, 3, "v3");

        let v1 = load(&list(work, &p.name)[0])?;
        assert_eq!(v1.shots[0].note, "hero", "a version is a loadable plan");
        assert_eq!(Plan::load(&live_path(work, &p.name))?, p, "live is latest");
        Ok(())
    }

    #[test]
    fn keeping_the_original_needs_a_file_and_no_history() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        keep_original(dir.path(), "none", 0)?;
        assert!(list(dir.path(), "none").is_empty(), "nothing to keep");
        Ok(())
    }
}

// ============================================================================
// Module:       commands
// Description:  The pipeline subcommands (index, caption, beats, plan, render)
//               and what each one does.
//
// Dependencies: clap (derive), std::io for output
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

use std::io::Write;
use std::path::PathBuf;

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
    /// Map a music track: tempo, beats, bars, drops -> <work>/music.json.
    Beats {
        /// The track.
        track: PathBuf,
        /// Work folder.
        #[arg(long)]
        work: PathBuf,
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
    /// Check a plan against its rules; exits 1 if any fail.
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
        /// Render even if the check fails.
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
    /// Print one segment's numbers and caption, or a shot-log summary.
    Show {
        /// Work folder.
        #[arg(long)]
        work: PathBuf,
        /// Segment ids (`MVI_7987/3`); none prints a summary.
        ids: Vec<String>,
    },
}

/// Runs the command `options` selects.
pub(crate) fn execute(command: Cmd, out: &mut impl Write) -> anyhow::Result<()> {
    match command {
        Cmd::Index {
            footage,
            work,
            jobs,
            force,
        } => {
            let index = crate::index::run(&footage, &work, jobs, force)?;
            writeln!(
                out,
                "{} clips, {} segments -> {}",
                index.clips.len(),
                index.segments.len(),
                work.join("shotlog.md").display()
            )?;
        }
        Cmd::Caption {
            work,
            min_q,
            redo,
            parallel,
        } => {
            let n = crate::caption::run(&work, min_q, redo, parallel)?;
            writeln!(
                out,
                "{n} segments captioned -> {}",
                work.join("shotlog.md").display()
            )?;
        }
        Cmd::Beats { track, work } => {
            let map = crate::beats::analyze(&track)?;
            std::fs::create_dir_all(&work)?;
            std::fs::write(work.join("music.json"), serde_json::to_string_pretty(&map)?)?;
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
                    "drop candidate: bar {} at {:.2}s (jump {:.2})",
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
            let index = crate::index::Index::load(&work)?;
            let map = crate::beats::MusicMap::load(&work)?;
            let people = crate::people::People::load(&work)?;
            let spec = crate::draft::Spec {
                name: name.clone(),
                aspect,
                seconds,
                end_card,
                card_bars,
            };
            let plan = crate::draft::draft(&index, &map, &people, &spec, &map.path)?;
            let path = work.join("plans").join(format!("{name}.toml"));
            plan.save(&path)?;
            let problems = crate::plan::check(&plan, &index, &map, &people);
            write!(out, "{}", crate::plan::report(&plan, &problems))?;
            writeln!(out, "-> {}", path.display())?;
        }
        Cmd::Check { plan, work } => {
            let (plan, problems) = load_and_check(&plan, &work)?;
            write!(out, "{}", crate::plan::report(&plan, &problems))?;
            anyhow::ensure!(problems.is_empty(), "{} problem(s)", problems.len());
        }
        Cmd::Render {
            plan,
            work,
            encoder,
            force,
        } => {
            let (plan, problems) = load_and_check(&plan, &work)?;
            if !problems.is_empty() {
                write!(out, "{}", crate::plan::report(&plan, &problems))?;
                anyhow::ensure!(force, "the plan fails its check; fix it or pass --force");
            }
            let index = crate::index::Index::load(&work)?;
            let map = crate::beats::MusicMap::load(&work)?;
            let video = work.join("renders").join(format!("{}.mp4", plan.name));
            let secs = crate::render::render(&plan, &index, &map, &video, encoder)?;
            let small = crate::render::preview(&video, plan.aspect)?;
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
            let map = crate::beats::MusicMap::load(&work)?;
            let path = work.join("renders").join(format!("{}.otio", plan.name));
            crate::otio::export(&plan, &index, &map, &path)?;
            writeln!(out, "{}", path.display())?;
        }
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
) -> anyhow::Result<(crate::plan::Plan, Vec<crate::plan::Problem>)> {
    let plan = crate::plan::Plan::load(plan)?;
    let index = crate::index::Index::load(work)?;
    let map = crate::beats::MusicMap::load(work)?;
    let people = crate::people::People::load(work)?;
    let problems = crate::plan::check(&plan, &index, &map, &people);
    Ok((plan, problems))
}

// ============================================================================
// Module:       walkthrough (example)
// Description:  Drives the real app headlessly through a whole project, the
//               way a person would: open a footage folder, follow the guide's
//               Next into set-up, start the analysis, wait for the first cut,
//               and screenshot every stage.
//
// Dependencies: egui, egui_kittest (headless harness + wgpu renderer), anyhow;
//               aivideoeditor::gui::app
// ============================================================================

//! `cargo run --example walkthrough -- <footage dir> <track> [<out dir>]`
//!
//! The footage folder should be a fresh copy with no `_ave` in it: the walk
//! starts a project from it. The track is written into the new project's
//! settings before it opens, as if carried over from the last project,
//! because a native file dialog cannot be clicked headlessly.
//!
//! Each step clicks a button by its accessible name, as a screen reader
//! would, and fails loudly when the button is not there, so a broken step
//! in the guided path shows up as an error naming it. The run ends when the
//! guide offers "Review the cut" with the editor open on the timeline.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use aivideoeditor::gui::app::{Action, App, Page};
use egui_kittest::kittest::{NodeT as _, Queryable};

/// The harness the walk drives.
type Harness<'a> = egui_kittest::Harness<'a, Option<App>>;

/// Runs frames for about `ms` of wall time, letting jobs and thumbnails land.
fn settle(harness: &mut Harness<'_>, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        harness.run_steps(4);
        std::thread::sleep(Duration::from_millis(40));
    }
}

/// Clicks the widget named `label`, waiting up to `secs` for it to appear
/// and be enabled (set-up's Continue waits for the folder scan).
fn click(harness: &mut Harness<'_>, label: &str, secs: u64) -> anyhow::Result<()> {
    let until = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(node) = harness.query_by_label(label)
            && !node.accesskit_node().is_disabled()
        {
            node.click();
            settle(harness, 300);
            return Ok(());
        }
        if Instant::now() > until {
            anyhow::bail!("no \"{label}\" button after {secs} s");
        }
        settle(harness, 200);
    }
}

/// Saves a screenshot as `<out>/<name>.png`.
fn shoot(harness: &mut Harness<'_>, out: &Path, name: &str) -> anyhow::Result<()> {
    let image = harness
        .render()
        .map_err(|e| anyhow::anyhow!("rendering {name} failed: {e}"))?;
    let path = out.join(format!("{name}.png"));
    image.save(&path)?;
    println!("wrote {}", path.display());
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(footage), Some(track)) = (args.next(), args.next()) else {
        anyhow::bail!("usage: walkthrough <footage dir> <track> [<out dir>]");
    };
    let footage = PathBuf::from(footage);
    let out = args.next().map_or_else(
        || {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("walkthrough")
        },
        PathBuf::from,
    );
    std::fs::create_dir_all(&out)?;
    let work = footage.join("_ave");
    anyhow::ensure!(
        !work.exists(),
        "{} exists: use a fresh copy",
        work.display()
    );
    std::fs::create_dir_all(&work)?;
    std::fs::write(
        work.join("ave.toml"),
        format!("footage = '{}'\ntrack = '{track}'\n", footage.display()),
    )?;

    let open = footage.clone();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1680.0, 1000.0))
        .build_ui_state(
            move |ui, app: &mut Option<App>| {
                let app = app.get_or_insert_with(|| {
                    let mut app =
                        App::new(ui.ctx(), aivideoeditor::config::Config::default(), None);
                    app.apply(ui.ctx(), Action::OpenProject(open.clone()));
                    app.apply(ui.ctx(), Action::Go(Page::Night));
                    app
                });
                app.show(ui);
            },
            None,
        );
    settle(&mut harness, 1500);
    shoot(&mut harness, &out, "01-opened")?;

    click(&mut harness, "Set up and analyse", 10)?;
    shoot(&mut harness, &out, "02-setup-footage")?;
    click(&mut harness, "Continue", 30)?;
    shoot(&mut harness, &out, "03-setup-music")?;
    click(&mut harness, "Continue", 10)?;
    shoot(&mut harness, &out, "04-setup-deliverables")?;
    click(&mut harness, "Continue", 10)?;
    settle(&mut harness, 1500);
    shoot(&mut harness, &out, "05-setup-approve")?;
    click(&mut harness, "Start analysis", 30)?;
    settle(&mut harness, 4000);
    shoot(&mut harness, &out, "06-analysing")?;

    // The run ends with the guide offering review and the editor open.
    let until = Instant::now() + Duration::from_secs(600);
    let mut last = Instant::now();
    loop {
        settle(&mut harness, 500);
        let page = harness.state().as_ref().map(|a| a.page);
        if harness.query_by_label("Review the cut").is_some() && page == Some(Page::Plan) {
            break;
        }
        if last.elapsed() > Duration::from_secs(30) {
            println!("still waiting, on {page:?}");
            last = Instant::now();
        }
        if Instant::now() > until {
            shoot(&mut harness, &out, "99-timed-out")?;
            anyhow::bail!("no first cut after 10 minutes; see 99-timed-out.png");
        }
    }
    settle(&mut harness, 3000);
    shoot(&mut harness, &out, "07-first-cut")?;
    Ok(())
}

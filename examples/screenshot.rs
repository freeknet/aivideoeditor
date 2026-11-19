// ============================================================================
// Module:       screenshot (example)
// Description:  Renders every page, view mode and overlay of the app headlessly
//               to PNG files, for the README and for reviewing a visual change.
//
// Dependencies: egui_kittest (headless harness + wgpu renderer), anyhow;
//               aivideoeditor::gui::app
// ============================================================================

//! `cargo run --example screenshot [-- <out dir>]`
//!
//! Drives the real [`App`] in egui_kittest's headless harness, the same way
//! a window runtime would, and renders each page with wgpu. The default
//! output directory is under `target/`: these are build output, regenerated
//! on demand, and a directory of stale committed PNGs is what makes someone
//! trust a picture that is three changes out of date.
//!
//! With a night open it also shoots the timeline's A/B compare
//! (`plan-compare.png`): it opens the tab strip's "Compare" menu and picks
//! the first other plan, or else the latest earlier version.

use aivideoeditor::gui::app::{Action, App, Page};
use aivideoeditor::gui::ui::layout::ViewMode;
use egui_kittest::kittest::Queryable;

/// The shot that opens the timeline's compare view.
const COMPARE: &str = "plan-compare";

fn main() -> anyhow::Result<()> {
    let out = std::env::args().nth(1).map_or_else(
        || {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("screenshots")
        },
        std::path::PathBuf::from,
    );
    std::fs::create_dir_all(&out)?;
    // A night to open (its _ave folder), so the pages show real content.
    let night = std::env::var_os("AVE_SCREENSHOT_NIGHT").map(std::path::PathBuf::from);
    let [width, height] = if night.is_some() {
        [1680.0, 1000.0]
    } else {
        aivideoeditor::gui::DEFAULT_SIZE
    };
    // Every page, then the workspace's view modes and overlays.
    let mut shots: Vec<(String, Vec<Action>)> = Page::ALL
        .into_iter()
        .map(|page| (page.key().to_owned(), vec![Action::Go(page)]))
        .collect();
    for mode in [ViewMode::Vertical, ViewMode::Comment, ViewMode::Preview] {
        shots.push((
            format!("view-{}", mode.key()),
            vec![Action::Go(Page::Review), Action::SetViewMode(mode)],
        ));
    }
    for (name, overlay) in [
        ("shortcuts", Action::ToggleShortcuts),
        ("search", Action::FocusSearch),
        ("settings", Action::OpenSettings),
    ] {
        shots.push((name.to_owned(), vec![Action::Go(Page::Review), overlay]));
    }
    if night.is_some() {
        shots.push((COMPARE.to_owned(), vec![Action::Go(Page::Plan)]));
    }
    for (name, actions) in shots {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(width, height))
            // Every input must be reachable by name; `run_steps` does not
            // run the check, so it is also called before each render below.
            .with_accessibility_check(true)
            .build_ui_state(
                |ui, app: &mut Option<App>| {
                    let app = app.get_or_insert_with(|| {
                        let mut app = App::new(
                            ui.ctx(),
                            aivideoeditor::config::Config::default(),
                            night.clone(),
                        );
                        for action in &actions {
                            app.apply(ui.ctx(), action.clone());
                        }
                        app
                    });
                    app.show(ui);
                },
                None,
            );
        // Long enough for the page's arrival animation to finish, and, with
        // a night open, for thumbnails decoded on worker threads to land.
        let rounds = if night.is_some() { 40 } else { 1 };
        for _ in 0..rounds {
            harness.run_steps(15);
            if night.is_some() {
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        }
        if name == COMPARE {
            open_compare(&mut harness)?;
        }
        let unnamed = harness.unnamed_widgets();
        if !unnamed.is_empty() {
            anyhow::bail!(
                "{name}: {} widget(s) lack an accessible name:\n{}",
                unnamed.len(),
                unnamed.join("\n")
            );
        }
        let image = harness
            .render()
            .map_err(|error| anyhow::anyhow!("rendering {name} failed: {error}"))?;
        let path = out.join(format!("{name}.png"));
        image.save(&path)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// Opens the timeline's "Compare" menu and picks B: the first other plan,
/// or else the latest earlier version of the open one.
fn open_compare(harness: &mut egui_kittest::Harness<'_, Option<App>>) -> anyhow::Result<()> {
    harness
        .query_by_label("Compare")
        .ok_or_else(|| anyhow::anyhow!("no Compare menu: the night has no plan"))?
        .click();
    harness.run_steps(4);
    let is_plan = |label: &str| label.starts_with("Plan ");
    let is_version = |label: &str| {
        label.strip_prefix('v').is_some_and(|rest| {
            rest.starts_with(|c: char| c.is_ascii_digit()) && rest.contains(" · ")
        })
    };
    let pick = |want: &dyn Fn(&str) -> bool| {
        harness
            .query_all_by(|node| node.label().is_some_and(|l| want(&l)))
            .next()
            .map(|node| node.click())
            .is_some()
    };
    if !pick(&is_plan) && !pick(&is_version) {
        anyhow::bail!("nothing to compare with: one plan and no earlier version");
    }
    for _ in 0..20 {
        harness.run_steps(15);
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(())
}

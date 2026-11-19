// ============================================================================
// Module:       screenshot (example)
// Description:  Renders every page, view mode and overlay of the app headlessly
//               to PNG files, for the README and for reviewing a visual change.
//
// Dependencies: egui, egui_kittest (headless harness + wgpu renderer), anyhow;
//               aivideoeditor::gui::app
// ============================================================================

//! `cargo run --example screenshot [-- <out dir>]`
//!
//! Drives the real [`App`] in egui_kittest's headless harness, the same way
//! a window runtime would, and renders each page with wgpu. The default
//! output directory is under `target/`: these are build output, regenerated
//! on demand, and a directory of stale committed PNGs is what makes someone
//! trust a picture that is three changes out of date.

use aivideoeditor::gui::app::{Action, App, Page};
use aivideoeditor::gui::ui::layout::ViewMode;

/// One picture: its name, the actions that set it up, then the keys
/// pressed once the app is up.
type Shot = (String, Vec<Action>, Vec<(egui::Modifiers, egui::Key)>);

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
    let mut shots: Vec<Shot> = Page::ALL
        .into_iter()
        .map(|page| (page.key().to_owned(), vec![Action::Go(page)], Vec::new()))
        .collect();
    for mode in [ViewMode::Vertical, ViewMode::Comment, ViewMode::Preview] {
        shots.push((
            format!("view-{}", mode.key()),
            vec![Action::Go(Page::Review), Action::SetViewMode(mode)],
            Vec::new(),
        ));
    }
    for (name, overlay) in [
        ("shortcuts", Action::ToggleShortcuts),
        ("search", Action::FocusSearch),
        ("settings", Action::OpenSettings),
    ] {
        shots.push((
            name.to_owned(),
            vec![Action::Go(Page::Review), overlay],
            Vec::new(),
        ));
    }
    // The Shots browser's review modes, entered by their keys the way a
    // user would (they need a night: without one the tab is empty).
    let none = egui::Modifiers::NONE;
    let first = (none, egui::Key::ArrowRight);
    for (name, keys) in [
        (
            "review-tape",
            vec![
                first,
                (none, egui::Key::T),
                (egui::Modifiers::SHIFT, egui::Key::L),
            ],
        ),
        ("review-compare", vec![first, (none, egui::Key::C)]),
        ("review-survey", vec![first, (none, egui::Key::G)]),
    ] {
        shots.push((name.to_owned(), vec![Action::Go(Page::Review)], keys));
    }
    for (name, actions, keys) in shots {
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
        harness.run_steps(5);
        for (modifiers, key) in keys {
            harness.key_press_modifiers(modifiers, key);
            harness.run_steps(3);
        }
        // Long enough for the page's arrival animation to finish, and, with
        // a night open, for thumbnails decoded on worker threads to land.
        let rounds = if night.is_some() { 40 } else { 1 };
        for _ in 0..rounds {
            harness.run_steps(15);
            if night.is_some() {
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
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

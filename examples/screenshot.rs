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
//!
//! With a night open it also shoots the timeline's A/B compare
//! (`plan-compare.png`): it opens the tab strip's "Compare" menu and picks
//! the first other plan, or else the latest earlier version. It also shoots
//! the chat panel's Chats menu open (`chat-list.png`) and, after "New chat"
//! from it, the strip of open chat tabs (`chat-tabs.png`).

use aivideoeditor::gui::app::{Action, App, Page};
use aivideoeditor::gui::ui::layout::ViewMode;
use egui_kittest::kittest::Queryable;

/// The shot that opens the timeline's compare view.
const COMPARE: &str = "plan-compare";

/// The shot with the chat's Chats menu open.
const CHAT_LIST: &str = "chat-list";

/// The shot with two chats open as tabs.
const CHAT_TABS: &str = "chat-tabs";

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
    if night.is_some() {
        shots.push((COMPARE.to_owned(), vec![Action::Go(Page::Plan)], Vec::new()));
        for name in [CHAT_LIST, CHAT_TABS] {
            shots.push((name.to_owned(), vec![Action::Go(Page::Review)], Vec::new()));
        }
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
        if name == COMPARE {
            open_compare(&mut harness)?;
        }
        if name == CHAT_LIST || name == CHAT_TABS {
            open_chats(&mut harness, name == CHAT_TABS)?;
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

/// Opens the chat's Chats menu; with `new_tab`, starts a new chat from it so
/// the panel shows its tab strip.
fn open_chats(
    harness: &mut egui_kittest::Harness<'_, Option<App>>,
    new_tab: bool,
) -> anyhow::Result<()> {
    harness
        .query_by_label("Chats")
        .ok_or_else(|| anyhow::anyhow!("no Chats button: the night has no plan"))?
        .click();
    harness.run_steps(6);
    if new_tab {
        // Open the second chat row (52 px rows at the list's left edge; the
        // first is the open chat), then start another.
        let mut rows: Vec<_> = harness
            .query_all_by(|node| node.role() == egui::accesskit::Role::Button)
            .filter(|node| {
                let r = node.rect();
                r.left() < 60.0 && r.height() > 40.0
            })
            .collect();
        rows.sort_by(|a, b| a.rect().top().total_cmp(&b.rect().top()));
        let first = rows.get(1).map(|node| node.click()).is_some();
        if first {
            harness.run_steps(6);
            harness
                .query_by_label("Chats")
                .ok_or_else(|| anyhow::anyhow!("no Chats button"))?
                .click();
            harness.run_steps(6);
        }
        // The menu's own button heads the list (a row may be titled the same).
        harness
            .query_all_by_label("New chat")
            .min_by(|a, b| a.rect().top().total_cmp(&b.rect().top()))
            .ok_or_else(|| anyhow::anyhow!("no New chat in the Chats menu"))?
            .click();
        harness.run_steps(6);
    }
    Ok(())
}

/// Opens the timeline's "Compare plans" menu and picks B: the first other plan,
/// or else the latest earlier version of the open one.
fn open_compare(harness: &mut egui_kittest::Harness<'_, Option<App>>) -> anyhow::Result<()> {
    harness
        .query_by_label("Compare plans")
        .ok_or_else(|| anyhow::anyhow!("no Compare plans menu: the night has no plan"))?
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

// ============================================================================
// Module:       screenshot (example)
// Description:  Renders every page, view mode and overlay of the app headlessly
//               to PNG files, for the README and for reviewing a visual change.
//
// Dependencies: egui, egui_kittest (headless harness + wgpu renderer), anyhow,
//               serde_json, toml (the chat fixtures); aivideoeditor::gui::app
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
//! the first other plan, or else the latest earlier version. It also
//! shoots the inspector's Shot list tab on the plan page
//! (`plan-shot-list.png`), with the second shot selected, and the
//! timeline's markers (`plan-markers.png`): three markers written into
//! every plan for the shot, the list open beside the tracks and the second
//! marker selected, so its colour swatches show.
//!
//! And the Shots browser grouped by act (`review-acts.png`): `ave.toml`
//! switched to *Group by → Acts* for the shot, the first act folded and
//! the first shot selected, so the inspector's Facts show its recording
//! time, camera and act.
//!
//! `AVE_SCREENSHOT_ONLY` (comma-separated shot names) limits the run to
//! those shots.
//!
//! And the chat's cards: a staged edit (`chat-proposal.png`), a slow step
//! waiting for approval (`chat-approval.png`) and the first-cut settings
//! card (`chat-first-cut.png`). The first two need a conversation: for each
//! plan without a chat, the example writes a short fixture chat (a log in
//! the night's `chat/chats/` and an entry in `chat/chats.json`). It also
//! shoots the Chats menu open (`chat-list.png`) and, after opening a chat
//! and starting a new one from it, the strip of open chat tabs
//! (`chat-tabs.png`). After each shot the fixtures go and the chat index and
//! `ave.toml` are put back as they were, so every shot starts from the same
//! night. Point it at a copy of a night, not the original.

use std::path::{Path, PathBuf};

use aivideoeditor::gui::app::{Action, App, Page};
use aivideoeditor::gui::ui::layout::ViewMode;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

/// The shot that opens the timeline's compare view.
const COMPARE: &str = "plan-compare";

/// The shot that opens the inspector's Shot list tab.
const SHOT_LIST: &str = "plan-shot-list";
/// The shot of the timeline's markers: pins on the ruler and the list.
const MARKERS: &str = "plan-markers";
/// Two shots flagged into the chat.
const FLAGGED: &str = "plan-flagged";

/// The shot of the Shots browser grouped by act.
const ACTS: &str = "review-acts";

/// The shot of a staged edit's card.
const PROPOSAL: &str = "chat-proposal";

/// The shot of a step waiting for approval.
const APPROVAL: &str = "chat-approval";

/// The shot of the first-cut settings card.
const FIRST_CUT: &str = "chat-first-cut";

/// The shot with the chat's Chats menu open.
const CHAT_LIST: &str = "chat-list";

/// The shot with two chats open as tabs.
const CHAT_TABS: &str = "chat-tabs";

/// What one shot changed in the night: fixture logs to remove, and files
/// (the chat index, `ave.toml`) to put back as they were (`None`: absent).
struct Fixtures {
    /// Files written for the shot.
    written: Vec<PathBuf>,
    /// Files to restore, with their contents before the shot.
    restore: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl Drop for Fixtures {
    fn drop(&mut self) {
        for path in &self.written {
            let _ = std::fs::remove_file(path);
        }
        for (path, before) in &self.restore {
            let _ = match before {
                Some(bytes) => std::fs::write(path, bytes),
                None => std::fs::remove_file(path),
            };
        }
    }
}

/// A chat-log line holding one message.
fn message(role: &str, blocks: Value) -> Value {
    json!({"kind": "msg", "msg": {"role": role, "blocks": blocks, "ts": 0}})
}

/// Writes the chat `shot` needs for every plan of `work` that has none yet
/// (in the chat index, or as an old per-plan log the app adopts), and
/// remembers the files to put back after the shot.
fn fixtures(work: &Path, shot: &str) -> anyhow::Result<Fixtures> {
    let chat = work.join("chat");
    let index_path = chat.join("chats.json");
    let mut written = Fixtures {
        written: Vec::new(),
        restore: [index_path.clone(), work.join("ave.toml")]
            .into_iter()
            .map(|p| {
                let before = std::fs::read(&p).ok();
                (p, before)
            })
            .collect(),
    };
    if shot == MARKERS {
        markers(work, &mut written)?;
        return Ok(written);
    }
    if shot == ACTS {
        page_setting(work, "review", "sort", "acts".into())?;
        return Ok(written);
    }
    std::fs::create_dir_all(chat.join("chats"))?;
    let mut index: Value = std::fs::read_to_string(&index_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({"chats": []}));
    let has_chat = |index: &Value, stem: &str| {
        index["chats"]
            .as_array()
            .is_some_and(|a| a.iter().any(|c| c["plan"] == stem))
    };
    let mut changed = false;
    // A project with no cut yet has no plans folder: nothing to do then.
    for entry in std::fs::read_dir(work.join("plans")).into_iter().flatten() {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        if chat.join(format!("{stem}.jsonl")).exists() || has_chat(&index, &stem) {
            continue;
        }
        let id = format!("fixture-{stem}");
        let log = chat.join("chats").join(format!("{id}.jsonl"));
        let lines = match shot {
            PROPOSAL => {
                let base: Value = toml::from_str(&std::fs::read_to_string(&path)?)?;
                let ops = json!([
                    {"op": "move_shot", "from": 2, "to": 1},
                    {"op": "set_note", "index": 1, "note": "opens on the brighter, closer shot"}
                ]);
                vec![
                    message(
                        "user",
                        json!([{"type": "text", "text": "Open on a stronger shot"}]),
                    ),
                    message(
                        "assistant",
                        json!([
                            {"type": "text", "text": "Shot 2 is the stronger opener, so I moved it to the front."},
                            {"type": "tool_use", "id": "fixture-edit", "name": "plan_edit",
                             "input": {"name": stem, "reason": "Open on shot 2", "ops": ops}}
                        ]),
                    ),
                    json!({"kind": "proposal", "proposal": {
                        "id": "fixture-edit", "plan": stem,
                        "summary": "Open on the stronger shot 2", "ops": ops,
                        "base": base, "base_version": "", "ts": 0}}),
                ]
            }
            APPROVAL => vec![
                message("user", json!([{"type": "text", "text": "Render it"}])),
                message(
                    "assistant",
                    json!([
                        {"type": "text", "text": "The check passes; rendering needs your go-ahead."},
                        {"type": "tool_use", "id": "fixture-step", "name": "run_step",
                         "input": {"step": "render", "plan": stem}}
                    ]),
                ),
            ],
            _ => continue,
        };
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&log, text)?;
        written.written.push(log);
        // Newest of the plan's chats, so it is the one that opens.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        if let Some(chats) = index["chats"].as_array_mut() {
            chats.push(json!({"id": id, "title": stem, "plan": stem,
                              "created": now, "updated": now}));
            changed = true;
        }
    }
    if changed {
        std::fs::write(&index_path, serde_json::to_string_pretty(&index)?)?;
    }
    Ok(written)
}

/// Writes three markers into every plan of `work` and turns the timeline's
/// marker list on in `ave.toml`, remembering each file to put back.
fn markers(work: &Path, fixtures: &mut Fixtures) -> anyhow::Result<()> {
    // A project with no cut yet has no plans folder: nothing to do then.
    for entry in std::fs::read_dir(work.join("plans")).into_iter().flatten() {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let mut plan: toml::Table = toml::from_str(&text)?;
        let marker = |time: f64, note: &str, color: &str, author: &str| {
            let mut m = toml::Table::new();
            m.insert("time".into(), time.into());
            m.insert("note".into(), note.into());
            m.insert("color".into(), color.into());
            m.insert("author".into(), author.into());
            toml::Value::Table(m)
        };
        plan.insert(
            "comment".into(),
            toml::Value::Array(vec![
                marker(1.2, "Open tighter on the hoop", "green", "you"),
                marker(3.4, "Cut on the snare, not the kick", "yellow", "you"),
                marker(6.8, "Flash frame here?", "red", "assistant"),
            ]),
        );
        fixtures
            .restore
            .push((path.clone(), Some(text.into_bytes())));
        std::fs::write(&path, toml::to_string_pretty(&plan)?)?;
    }
    let settings = work.join("ave.toml");
    let mut table: toml::Table = std::fs::read_to_string(&settings)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();
    let pages = table
        .entry("pages")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let toml::Value::Table(pages) = pages {
        let plan = pages
            .entry("plan")
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(plan) = plan {
            plan.insert("comments".into(), true.into());
        }
    }
    std::fs::write(&settings, toml::to_string_pretty(&table)?)?;
    Ok(())
}

/// Sets `[pages.<page>] <key> = value` in the night's `ave.toml` (the
/// fixtures put the file back after the shot).
fn page_setting(work: &Path, page: &str, key: &str, value: toml::Value) -> anyhow::Result<()> {
    let settings = work.join("ave.toml");
    let mut table: toml::Table = std::fs::read_to_string(&settings)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();
    let pages = table
        .entry("pages")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if let toml::Value::Table(pages) = pages {
        let table = pages
            .entry(page)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(table) = table {
            table.insert(key.into(), value);
        }
    }
    std::fs::write(&settings, toml::to_string_pretty(&table)?)?;
    Ok(())
}

/// Folds the first act in the grouped Shots browser (the headers below it
/// are off screen, so not in the tree, until it folds), then opens the
/// inspector's Facts tab for the selected shot.
fn fold_first_act(harness: &mut egui_kittest::Harness<'_, Option<App>>) -> anyhow::Result<()> {
    harness
        .query_all_by(|node| node.label().is_some_and(|l| l.starts_with("Set 1 · ")))
        .next()
        .ok_or_else(|| anyhow::anyhow!("no header for set 1: the night has no sets"))?
        .click();
    harness.run_steps(4);
    harness
        .query_all_by(|node| node.label().is_some_and(|l| l == "Facts"))
        .next()
        .ok_or_else(|| anyhow::anyhow!("no Facts tab in the inspector"))?
        .click();
    for _ in 0..6 {
        harness.run_steps(15);
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(())
}

/// Selects the second marker in the list (its timecode seeks and selects).
fn select_marker(harness: &mut egui_kittest::Harness<'_, Option<App>>) -> anyhow::Result<()> {
    harness
        .query_by_label("0:03.4")
        .ok_or_else(|| anyhow::anyhow!("no marker at 0:03.4 in the list"))?
        .click();
    for _ in 0..6 {
        harness.run_steps(15);
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(())
}

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
        shots.push((
            SHOT_LIST.to_owned(),
            vec![Action::Go(Page::Plan)],
            Vec::new(),
        ));
        shots.push((MARKERS.to_owned(), vec![Action::Go(Page::Plan)], Vec::new()));
        // Two shots of the first cut pointed at in the chat: their @ flags on
        // the timeline, the message's references and the inspector's summary.
        let first_plan = night.as_ref().and_then(|n| {
            let mut names: Vec<String> = std::fs::read_dir(n.join("plans"))
                .ok()?
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    (p.extension()? == "toml")
                        .then(|| p.file_stem().map(|s| s.to_string_lossy().into_owned()))?
                })
                .collect();
            names.sort();
            names.into_iter().next()
        });
        if let Some(plan) = first_plan {
            let shot = |index: usize| {
                Action::AddRef(aivideoeditor::refs::Reference::new(
                    aivideoeditor::refs::Target::Shot {
                        plan: plan.clone(),
                        index,
                    },
                ))
            };
            shots.push((
                FLAGGED.to_owned(),
                vec![Action::Go(Page::Plan), shot(2), shot(5)],
                Vec::new(),
            ));
        }
        shots.push((
            ACTS.to_owned(),
            vec![Action::Go(Page::Review)],
            vec![(egui::Modifiers::NONE, egui::Key::ArrowRight)],
        ));
        for name in [PROPOSAL, APPROVAL, FIRST_CUT, CHAT_LIST, CHAT_TABS] {
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
    // `AVE_SCREENSHOT_ONLY=review-acts,plan` shoots just those.
    let only: Option<Vec<String>> = std::env::var("AVE_SCREENSHOT_ONLY")
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_owned()).collect());
    shots.retain(|(name, _, _)| only.as_ref().is_none_or(|o| o.contains(name)));
    for (name, actions, keys) in shots {
        let _fixtures = match &night {
            Some(work) => Some(fixtures(work, &name)?),
            None => None,
        };
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
        if name == SHOT_LIST {
            open_shot_list(&mut harness)?;
        }
        if name == MARKERS {
            select_marker(&mut harness)?;
        }
        if name == ACTS {
            fold_first_act(&mut harness)?;
        }
        if name == FIRST_CUT {
            harness
                .query_by_label("Suggest a first cut")
                .ok_or_else(|| anyhow::anyhow!("no first-cut quick action: the chat is not empty"))?
                .click();
            harness.run_steps(15);
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

/// Opens the timeline's "Compare cuts" menu and picks B: the first other plan,
/// or else the latest earlier version of the open one.
fn open_compare(harness: &mut egui_kittest::Harness<'_, Option<App>>) -> anyhow::Result<()> {
    harness
        .query_by_label("Compare cuts")
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

/// Opens the inspector's Shot list tab and selects the second shot.
fn open_shot_list(harness: &mut egui_kittest::Harness<'_, Option<App>>) -> anyhow::Result<()> {
    harness
        .query_by_label("Shot list")
        .ok_or_else(|| anyhow::anyhow!("no Shot list tab in the inspector"))?
        .click();
    harness.run_steps(4);
    if let Some(row) = harness.query_by_label("Shot 2") {
        row.click();
    }
    for _ in 0..10 {
        harness.run_steps(15);
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Ok(())
}

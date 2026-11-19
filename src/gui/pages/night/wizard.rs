// ============================================================================
// Module:       gui::pages::night::wizard
// Description:  The get-started wizard for a new night: footage folder, music
//               and end cards, deliverables, then the analysis approval card
//               with time estimates and "Start analysis".
//
// Dependencies: egui, rfd; crate::{commands, project, recipe},
//               crate::gui::pages::night::{pipeline, scan}
// ============================================================================

//! Get started.
//!
//! Three steps and an approval, after Selects' wizard and its "Analyze
//! media" card:
//!
//! 1. **Footage**: the folder, how many clips and minutes it holds, a strip
//!    of frames from it; change it if the guess was wrong.
//! 2. **Music and end cards**: the approved track and the end card for each
//!    aspect the recipe needs, with last week's carried over.
//! 3. **Deliverables**: the recipe's outputs; untick one to leave it out of
//!    this week's first drafts.
//! 4. **Approve the analysis**: what will run (index, captions, music map,
//!    people, plans), each with the time it should take and where that
//!    number comes from (measured on this machine, on a sibling night, or
//!    assumed, and it says which), and a total. Nothing slow starts before
//!    "Start analysis".
//!
//! Every choice is written to the night's `ave.toml` the moment it is made,
//! so leaving the wizard halfway loses nothing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{pipeline, scan};
use crate::commands::Cmd;
use crate::gui::pages::PageCtx;
use crate::gui::ui::{theme, widgets};
use crate::project::{self, StepEstimate};
use crate::recipe::{self, Recipe};

/// A wizard step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// The footage folder.
    Footage,
    /// Track and end cards.
    Music,
    /// The recipe's outputs.
    Deliverables,
    /// The analysis approval.
    Approve,
}

impl Step {
    /// Every step, in order.
    const ALL: [Self; 4] = [
        Self::Footage,
        Self::Music,
        Self::Deliverables,
        Self::Approve,
    ];

    /// Its name in the stepper.
    fn title(self) -> &'static str {
        match self {
            Self::Footage => "Footage",
            Self::Music => "Music & end cards",
            Self::Deliverables => "Deliverables",
            Self::Approve => "Analysis",
        }
    }

    /// Its place, from 0.
    fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// A step the processing page tracks: its job label, name and estimate.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Planned {
    /// The job label (`index`, `plan reel-30`).
    pub(crate) label: String,
    /// What the user reads.
    pub(crate) title: String,
    /// Expected wall seconds.
    pub(crate) secs: f32,
}

/// What the wizard asks the night page to do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    /// Queue these jobs and show their progress.
    Start {
        /// The jobs, in order.
        jobs: Vec<(String, Cmd)>,
        /// What the processing page tracks.
        plan: Vec<Planned>,
    },
    /// Leave the wizard for the night page.
    Close,
}

/// The wizard's state.
#[derive(Debug)]
pub(crate) struct State {
    /// The night it sets up.
    pub(crate) work: PathBuf,
    /// The step on screen.
    pub(crate) step: Step,
    /// The night it carried over from.
    pub(crate) previous: Option<PathBuf>,
    /// What setting it up carried over, one line each.
    pub(crate) notes: Vec<String>,
    /// The footage scan, with speeds for the estimates.
    scan: scan::Scan,
    /// Bumped to rescan (the footage folder changed).
    generation: usize,
    /// The recipe, read once.
    recipe: Option<Recipe>,
    /// Deliverables left out of the first drafts.
    skip: BTreeSet<String>,
    /// Analysis rows switched off.
    off: BTreeSet<&'static str>,
}

impl State {
    /// A wizard for the night at `work`.
    pub(crate) fn new(work: PathBuf, previous: Option<PathBuf>, notes: Vec<String>) -> Self {
        Self {
            work,
            step: Step::Footage,
            previous,
            notes,
            scan: scan::Scan::default(),
            generation: 0,
            recipe: None,
            skip: BTreeSet::new(),
            off: BTreeSet::new(),
        }
    }

    /// The night's recipe.
    fn recipe(&mut self) -> &Recipe {
        let work = self.work.clone();
        self.recipe.get_or_insert_with(|| Recipe::load(&work).0)
    }
}

/// One row of the approval card.
struct Row {
    /// Key for switching it off.
    key: &'static str,
    /// Its name.
    title: &'static str,
    /// The estimate; `None` when it cannot run, with `blocked` saying why.
    estimate: StepEstimate,
    /// Why it cannot run (or need not).
    blocked: Option<String>,
    /// The jobs it queues.
    jobs: Vec<(String, Cmd)>,
}

/// Seconds a music map takes when nothing measured it.
const BEATS_SECS: f32 = 20.0;
/// Seconds drafting people takes.
const PEOPLE_SECS: f32 = 2.0;
/// Seconds drafting one plan takes.
const PLAN_SECS: f32 = 3.0;

/// The approval rows for the open night.
fn rows(ctx: &PageCtx<'_>, state: &mut State, scanned: Option<&scan::Scanned>) -> Vec<Row> {
    let recipe = state.recipe().clone();
    let Some(p) = ctx.project.as_ref() else {
        return Vec::new();
    };
    let work = p.work.clone();
    let clips = scanned.map(|s| s.clips.as_slice()).unwrap_or_default();
    let speeds = scanned.and_then(|s| s.speeds.clone());
    let pending = |what: &str| StepEstimate {
        what: what.to_owned(),
        secs: 0.0,
        basis: String::new(),
    };
    let mut out = Vec::new();

    // Index.
    let (estimate, blocked, jobs) = match (&p.settings.footage, &speeds) {
        (None, _) => (
            pending("no footage folder"),
            Some("Set the footage folder".to_owned()),
            Vec::new(),
        ),
        (Some(_), None) => (pending("reading the footage\u{2026}"), None, Vec::new()),
        (Some(footage), Some(speeds)) => (
            project::estimate_index(clips, speeds),
            None,
            vec![pipeline::index_cmd(&work, footage, false)],
        ),
    };
    out.push(Row {
        key: "index",
        title: "Index",
        estimate,
        blocked,
        jobs,
    });

    // Captions: what the shot log says is left, plus the new footage's share.
    let estimate = match &speeds {
        Some(speeds) => {
            let mut e = project::estimate_caption(p.index.as_ref(), clips, speeds);
            if p.index.is_some() {
                let new: f64 = clips
                    .iter()
                    .filter(|c| !c.cached)
                    .filter_map(|c| c.duration)
                    .sum();
                let extra = (new as f32 * speeds.density.value).round();
                if extra > 0.0 {
                    e.secs += extra * speeds.caption.value;
                    e.what
                        .push_str(&format!(" + about {extra:.0} from new clips"));
                }
            }
            e
        }
        None => pending("reading the footage\u{2026}"),
    };
    out.push(Row {
        key: "caption",
        title: "Captions",
        estimate,
        blocked: p
            .settings
            .footage
            .is_none()
            .then(|| "Needs the footage".to_owned()),
        jobs: vec![pipeline::caption_cmd(&work)],
    });

    // Music map.
    let mapped = p
        .music
        .as_ref()
        .zip(p.settings.track.as_ref())
        .is_some_and(|(m, t)| Path::new(&m.path) == t.as_path());
    let (estimate, blocked, jobs) = match &p.settings.track {
        None => (
            pending("no track"),
            Some("Choose the track in step 2".to_owned()),
            Vec::new(),
        ),
        Some(_) if mapped => (
            pending("already mapped"),
            Some("Already mapped".to_owned()),
            Vec::new(),
        ),
        Some(track) => (
            StepEstimate {
                what: format!(
                    "tempo, bars and drops of {}",
                    track.file_name().unwrap_or_default().to_string_lossy()
                ),
                secs: BEATS_SECS,
                basis: "assumed".to_owned(),
            },
            None,
            vec![(
                "beats".to_owned(),
                Cmd::Beats {
                    track: track.clone(),
                    work: work.clone(),
                },
            )],
        ),
    };
    out.push(Row {
        key: "beats",
        title: "Music map",
        estimate,
        blocked,
        jobs,
    });

    // People.
    let drafted = work.join("people.toml").exists();
    out.push(Row {
        key: "people",
        title: "People",
        estimate: if drafted {
            pending("already drafted")
        } else {
            StepEstimate {
                what: "who's who, drafted from the captions".to_owned(),
                secs: PEOPLE_SECS,
                basis: "assumed".to_owned(),
            }
        },
        blocked: drafted.then(|| "Already drafted".to_owned()),
        jobs: vec![(
            "people".to_owned(),
            Cmd::People {
                work: work.clone(),
                force: false,
            },
        )],
    });

    // Plans for the chosen deliverables that have none yet.
    let have = recipe.match_plans(&p.plans);
    let ids: Vec<String> = recipe
        .deliverables
        .iter()
        .enumerate()
        .filter(|(i, d)| {
            !state.skip.contains(&d.id)
                && !have
                    .iter()
                    .any(|m| m.deliverable == *i && m.version == recipe::MAIN_VERSION)
        })
        .map(|(_, d)| d.id.clone())
        .collect();
    let music = p.settings.track.is_some() || p.music.is_some();
    out.push(Row {
        key: "plans",
        title: "First drafts",
        estimate: StepEstimate {
            what: if ids.is_empty() {
                "nothing to draft".to_owned()
            } else {
                format!("{} plans: {}", ids.len(), ids.join(", "))
            },
            secs: ids.len() as f32 * PLAN_SECS,
            basis: "assumed".to_owned(),
        },
        blocked: if ids.is_empty() {
            Some("No deliverables to draft".to_owned())
        } else if !music {
            Some("Needs the track".to_owned())
        } else {
            None
        },
        jobs: pipeline::plan_cmds(ctx, &recipe, &ids),
    });
    out
}

/// Draws the wizard; returns what the night page should do.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) -> Option<Outcome> {
    let (name, footage) = ctx
        .project
        .as_ref()
        .map(|p| (p.name(), p.settings.footage.clone()))?;
    if let Some(footage) = &footage {
        state
            .scan
            .want(&state.work, footage, true, state.generation);
    }
    let theme = ctx.theme;
    let width = ui.available_width().min(880.0);
    let side = ((ui.available_width() - width) / 2.0).max(0.0);
    let mut outcome = None;
    ui.horizontal(|ui| {
        ui.add_space(side);
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.add_space(theme::SPACE_XL);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Set up {name}"))
                        .font(theme::font(theme::HEADING_SIZE + 6.0))
                        .color(theme::rgb(theme.text))
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .link("Skip setup")
                        .on_hover_text("Go to the night page; nothing runs")
                        .clicked()
                    {
                        outcome = Some(Outcome::Close);
                    }
                });
            });
            ui.add_space(theme::SPACE_MD);
            stepper(ctx, state, ui);
            ui.add_space(theme::SPACE_LG);
            egui::Frame::new()
                .fill(theme::rgb(theme.panel))
                .stroke(egui::Stroke::new(1.0, theme::rgb(theme.border)))
                .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
                .inner_margin(theme::margin(theme::SPACE_LG))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match state.step {
                        Step::Footage => footage_step(ctx, state, ui),
                        Step::Music => music_step(ctx, state, ui),
                        Step::Deliverables => deliverables_step(ctx, state, ui),
                        Step::Approve => {
                            if let Some(o) = approve_step(ctx, state, ui) {
                                outcome = Some(o);
                            }
                        }
                    }
                });
            ui.add_space(theme::SPACE_MD);
            ui.horizontal(|ui| {
                let i = state.step.index();
                if i > 0
                    && widgets::button(ui, theme, "Back", widgets::Emphasis::Secondary).clicked()
                {
                    state.step = Step::ALL[i - 1];
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(next) = Step::ALL.get(i + 1)
                        && widgets::button(ui, theme, "Continue", widgets::Emphasis::Primary)
                            .clicked()
                    {
                        state.step = *next;
                    }
                });
            });
        });
    });
    outcome
}

/// The numbered steps across the top; earlier ones can be clicked.
fn stepper(ctx: &PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let theme = ctx.theme;
    ui.horizontal(|ui| {
        for (i, step) in Step::ALL.into_iter().enumerate() {
            let here = step == state.step;
            let past = i < state.step.index();
            let (fill, text) = if here {
                (theme.accent, theme.text_on_accent)
            } else if past {
                (theme.accent_soft, theme.text)
            } else {
                (theme.raised, theme.text_muted)
            };
            let label = format!(
                "{}  {}",
                if past {
                    "\u{2713}".to_owned()
                } else {
                    (i + 1).to_string()
                },
                step.title()
            );
            let galley = ui.painter().layout_no_wrap(
                label,
                theme::font(theme::BODY_SIZE - 1.0),
                theme::rgb(text),
            );
            let size = galley.size() + egui::vec2(theme::SPACE_LG, theme::SPACE_SM * 1.5);
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(theme::RADIUS_LG),
                theme::rgb(fill),
            );
            ui.painter().galley(
                rect.center() - galley.size() / 2.0,
                galley,
                theme::rgb(text),
            );
            if response.clicked() {
                state.step = step;
            }
            if i + 1 < Step::ALL.len() {
                ui.label(egui::RichText::new("\u{203a}").color(theme::rgb(theme.text_faint)));
            }
        }
    });
}

/// A labelled line: muted label, then the value and any buttons.
fn field(ctx: &PageCtx<'_>, ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(130.0, 22.0), egui::Sense::hover());
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            label,
            theme::font(theme::BODY_SIZE - 1.0),
            theme::rgb(ctx.theme.text_muted),
        );
        add(ui);
    });
}

/// A file name, or a muted placeholder.
fn file_label(ctx: &PageCtx<'_>, ui: &mut egui::Ui, path: Option<&Path>, none: &str) {
    match path {
        Some(p) => {
            ui.label(
                egui::RichText::new(p.file_name().unwrap_or_default().to_string_lossy())
                    .color(theme::rgb(ctx.theme.text)),
            )
            .on_hover_text(p.display().to_string());
        }
        None => {
            ui.label(egui::RichText::new(none).color(theme::rgb(ctx.theme.text_faint)));
        }
    }
}

/// Step 1: the footage folder.
fn footage_step(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let theme = ctx.theme;
    widgets::title(ui, theme, "This week's footage");
    widgets::muted(ui, theme, "Every video directly in this folder is indexed.");
    ui.add_space(theme::SPACE_MD);
    let footage = ctx
        .project
        .as_ref()
        .and_then(|p| p.settings.footage.clone());
    let mut change = false;
    field(ctx, ui, "Folder", |ui| {
        match &footage {
            Some(f) => {
                ui.label(
                    egui::RichText::new(f.display().to_string()).color(theme::rgb(theme.text)),
                );
            }
            None => {
                ui.label(egui::RichText::new("not set").color(theme::rgb(theme.warning)));
            }
        }
        change = ui.button("Change\u{2026}").clicked();
    });
    if change
        && let Some(picked) = rfd::FileDialog::new()
            .set_title("This week's footage folder")
            .pick_folder()
        && let Some(p) = ctx.project.as_mut()
    {
        p.settings.footage = Some(picked);
        let _ = p.save_settings();
        state.generation += 1;
    }
    let scanned = state.scan.for_work(&state.work).cloned();
    field(ctx, ui, "Clips", |ui| match &scanned {
        Some(s) if s.error.is_some() => {
            ui.colored_label(
                theme::rgb(theme.danger),
                s.error.clone().unwrap_or_default(),
            );
        }
        Some(s) => {
            let cached = s.clips.iter().filter(|c| c.cached).count();
            let mut text = format!(
                "{} clips \u{b7} {:.1} min",
                s.clips.len(),
                s.footage_secs() / 60.0
            );
            if cached > 0 {
                text.push_str(&format!(" \u{b7} {cached} analysed already"));
            }
            ui.label(text);
            if state.scan.busy() {
                ui.add(egui::Spinner::new().size(12.0));
            }
        }
        None => {
            ui.add(egui::Spinner::new().size(12.0));
            ui.label(
                egui::RichText::new("reading the folder\u{2026}")
                    .color(theme::rgb(theme.text_muted)),
            );
        }
    });
    if let Some(s) = scanned.filter(|s| !s.clips.is_empty()) {
        ui.add_space(theme::SPACE_MD);
        let size = egui::vec2(112.0, 63.0);
        let fit = ((ui.available_width() + 6.0) / (size.x + 6.0))
            .floor()
            .max(1.0) as usize;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for clip in s.clips.iter().take(fit) {
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
                let t = clip.duration.map_or(1.0, |d| d * 0.4);
                ctx.thumbs
                    .paint(ui, rect, &clip.path, t, 0.5, theme::rgb(theme.raised));
                response.on_hover_text(&clip.id);
            }
        });
    }
}

/// Step 2: track and end cards, last week's carried over.
fn music_step(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let theme = ctx.theme;
    widgets::title(ui, theme, "Music and end cards");
    widgets::muted(
        ui,
        theme,
        "The approved track every cut is timed to, and the card each aspect closes on.",
    );
    ui.add_space(theme::SPACE_MD);
    let keys: BTreeSet<String> = state
        .recipe()
        .deliverables
        .iter()
        .map(|d| d.end_card_key().to_owned())
        .collect();
    let Some(settings) = ctx.project.as_ref().map(|p| p.settings.clone()) else {
        return;
    };
    let mut pick_track = false;
    field(ctx, ui, "Track", |ui| {
        file_label(ctx, ui, settings.track.as_deref(), "no track yet");
        pick_track = ui.button("Choose\u{2026}").clicked();
    });
    let mut pick_card = None;
    for key in &keys {
        field(ctx, ui, &format!("End card {key}"), |ui| {
            file_label(
                ctx,
                ui,
                settings.end_cards.get(key).map(PathBuf::as_path),
                "none",
            );
            if ui.button("Choose\u{2026}").clicked() {
                pick_card = Some(key.clone());
            }
        });
    }
    let start = recipe::night_dir(&state.work);
    if pick_track
        && let Some(track) = rfd::FileDialog::new()
            .set_title("The night's music track")
            .set_directory(&start)
            .add_filter("Audio", &["wav", "mp3", "flac", "m4a", "aac", "ogg"])
            .pick_file()
        && let Some(p) = ctx.project.as_mut()
    {
        p.settings.track = Some(track);
        let _ = p.save_settings();
    }
    if let Some(key) = pick_card
        && let Some(card) = rfd::FileDialog::new()
            .set_title(format!("End card for {key}"))
            .set_directory(&start)
            .add_filter("Image", &["png", "jpg", "jpeg"])
            .pick_file()
        && let Some(p) = ctx.project.as_mut()
    {
        p.settings.end_cards.insert(key, card);
        let _ = p.save_settings();
    }
    ui.add_space(theme::SPACE_MD);
    let last: Option<project::Settings> = state.previous.as_ref().and_then(|w| {
        std::fs::read_to_string(w.join("ave.toml"))
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
    });
    let missing = last.as_ref().is_some_and(|l| {
        (settings.track.is_none() && l.track.is_some())
            || l.end_cards
                .keys()
                .any(|k| !settings.end_cards.contains_key(k))
    });
    if let (Some(last), true) = (last, missing) {
        let from = state
            .previous
            .as_deref()
            .map(|w| recipe::Names::for_work(w).night)
            .unwrap_or_default();
        if widgets::button(
            ui,
            theme,
            &format!("Carry over {from}'s"),
            widgets::Emphasis::Secondary,
        )
        .on_hover_text("Fill the track and end cards still missing from last week's night")
        .clicked()
            && let Some(p) = ctx.project.as_mut()
        {
            if p.settings.track.is_none() {
                p.settings.track = last.track.clone();
            }
            for (k, v) in &last.end_cards {
                p.settings
                    .end_cards
                    .entry(k.clone())
                    .or_insert_with(|| v.clone());
            }
            let _ = p.save_settings();
            state.notes.push(format!("track and end cards from {from}"));
        }
    }
    if !state.notes.is_empty() {
        ui.add_space(theme::SPACE_SM);
        for note in &state.notes {
            ui.label(
                egui::RichText::new(format!("\u{2713} {note}"))
                    .small()
                    .color(theme::rgb(theme.success)),
            );
        }
    }
}

/// Step 3: the recipe's deliverables.
fn deliverables_step(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let theme = ctx.theme;
    widgets::title(ui, theme, "Deliverables");
    widgets::muted(
        ui,
        theme,
        "From the night's recipe. Untick one to leave it out of the first drafts; edit the \
         recipe itself on the night page.",
    );
    ui.add_space(theme::SPACE_MD);
    let cards = ctx
        .project
        .as_ref()
        .map(|p| p.settings.end_cards.clone())
        .unwrap_or_default();
    let list = state.recipe().deliverables.clone();
    if list.is_empty() {
        ui.colored_label(theme::rgb(theme.warning), "The recipe has no deliverables.");
    }
    for d in list {
        let mut on = !state.skip.contains(&d.id);
        ui.horizontal(|ui| {
            if ui.checkbox(&mut on, "").changed() {
                if on {
                    state.skip.remove(&d.id);
                } else {
                    state.skip.insert(d.id.clone());
                }
            }
            ui.label(
                egui::RichText::new(&d.id)
                    .strong()
                    .color(theme::rgb(theme.text)),
            );
            let card = if cards.contains_key(d.end_card_key()) {
                "end card \u{2713}"
            } else {
                "no end card"
            };
            ui.label(
                egui::RichText::new(format!(
                    "{} \u{b7} {:.0} s \u{b7} {card}",
                    recipe::aspect_key(d.aspect),
                    d.seconds
                ))
                .color(theme::rgb(theme.text_muted)),
            );
        });
    }
}

/// Step 4: the approval card.
fn approve_step(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) -> Option<Outcome> {
    let theme = ctx.theme;
    widgets::title(ui, theme, "Analyse the footage");
    widgets::muted(
        ui,
        theme,
        "Runs on this machine, in this order. Open the editor as soon as indexing is done; \
         captions keep going in the background.",
    );
    ui.add_space(theme::SPACE_MD);
    let scanned = state.scan.for_work(&state.work).cloned();
    let rows = rows(ctx, state, scanned.as_ref());
    let mut total = 0.0;
    let mut chosen: Vec<&Row> = Vec::new();
    for row in &rows {
        let can = row.blocked.is_none() && !row.jobs.is_empty();
        let mut on = can && !state.off.contains(row.key);
        ui.horizontal(|ui| {
            let changed = ui
                .add_enabled_ui(can, |ui| ui.checkbox(&mut on, ""))
                .inner
                .changed();
            if changed {
                if on {
                    state.off.remove(row.key);
                } else {
                    state.off.insert(row.key);
                }
            }
            let (rect, _) = ui.allocate_exact_size(egui::vec2(110.0, 20.0), egui::Sense::hover());
            ui.painter().text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                row.title,
                theme::font(theme::BODY_SIZE),
                theme::rgb(if can { theme.text } else { theme.text_faint }),
            );
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&row.estimate.what).color(theme::rgb(theme.text)));
                let note = match (&row.blocked, row.estimate.basis.is_empty()) {
                    (Some(why), _) if why.eq_ignore_ascii_case(&row.estimate.what) => String::new(),
                    (Some(why), _) => why.clone(),
                    (None, true) => String::new(),
                    (None, false) => row.estimate.basis.clone(),
                };
                if !note.is_empty() {
                    ui.label(
                        egui::RichText::new(note)
                            .small()
                            .color(theme::rgb(theme.text_faint)),
                    );
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if can {
                    ui.label(
                        egui::RichText::new(format!(
                            "\u{2248} {}",
                            project::duration_text(row.estimate.secs)
                        ))
                        .strong()
                        .color(theme::rgb(if on {
                            theme.text
                        } else {
                            theme.text_faint
                        })),
                    );
                }
            });
        });
        ui.add_space(theme::SPACE_XS);
        if can && on {
            total += row.estimate.secs;
            chosen.push(row);
        }
    }
    ui.separator();
    let reading = scanned.as_ref().is_none_or(|s| s.speeds.is_none());
    let mut outcome = None;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(if reading {
                "Estimating\u{2026}".to_owned()
            } else {
                format!("About {} in all", project::duration_text(total))
            })
            .font(theme::font(theme::TITLE_SIZE))
            .color(theme::rgb(theme.text)),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let go = ui
                .add_enabled_ui(!chosen.is_empty(), |ui| {
                    widgets::button(ui, theme, "Start analysis", widgets::Emphasis::Primary)
                })
                .inner
                .clicked();
            if go {
                // Index first, the music map early (it is quick and plans need
                // it), captions next (long; the editor opens meanwhile), then
                // what reads the captions.
                let order = ["index", "beats", "caption", "people", "plans"];
                let mut jobs = Vec::new();
                let mut plan = Vec::new();
                for key in order {
                    let Some(row) = chosen.iter().find(|r| r.key == key) else {
                        continue;
                    };
                    let n = row.jobs.len().max(1) as f32;
                    for (label, cmd) in &row.jobs {
                        plan.push(Planned {
                            label: label.clone(),
                            title: if row.jobs.len() > 1 {
                                label.clone()
                            } else {
                                row.title.to_owned()
                            },
                            secs: row.estimate.secs / n,
                        });
                        jobs.push((label.clone(), cmd.clone()));
                    }
                }
                outcome = Some(Outcome::Start { jobs, plan });
            }
        });
    });
    outcome
}

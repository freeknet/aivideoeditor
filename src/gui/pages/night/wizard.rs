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

use super::{parts, pipeline, scan};
use crate::commands::Cmd;
use crate::gui::pages::PageCtx;
use crate::gui::ui::{icons, theme, widgets};
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
    // A step certain to fail is not offered: without the vision model the
    // rest runs without captions, and the row says how to add them.
    let missing_model = crate::caption::not_set_up();
    out.push(Row {
        key: "caption",
        title: "Captions",
        estimate: match &missing_model {
            Some(why) => pending(why),
            None => estimate,
        },
        blocked: p
            .settings
            .footage
            .is_none()
            .then(|| "Needs the footage".to_owned())
            .or_else(|| missing_model.map(|_| "Not set up".to_owned())),
        jobs: vec![pipeline::caption_cmd(&work)],
    });

    // Music map.
    let mapped = p
        .music
        .as_ref()
        .zip(p.settings.track.as_ref())
        .is_some_and(|(m, t)| Path::new(&m.path) == t.as_path());
    let (estimate, blocked, jobs) = match &p.settings.track {
        None if p.no_music() => (
            pending(NO_MUSIC_NOTE),
            Some("No music".to_owned()),
            Vec::new(),
        ),
        None => (
            pending("no track"),
            Some("Choose a track, or No music, in step 2".to_owned()),
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
                    track: Some(track.clone()),
                    none: false,
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
            Some("Needs a track, or No music".to_owned())
        } else {
            None
        },
        jobs: pipeline::plan_cmds(ctx, &recipe, &ids),
    });
    out
}

/// The column's widest.
const COLUMN: f32 = 720.0;

/// What the approval step chose, for the card's footer to start.
struct Approval {
    /// The chosen rows' estimated seconds.
    total: f32,
    /// Whether the footage is still being read (no estimate yet).
    reading: bool,
    /// The jobs to queue, in order, and what the processing page tracks;
    /// empty when nothing is chosen.
    jobs: Vec<(String, Cmd)>,
    /// What the processing page tracks.
    plan: Vec<Planned>,
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
    let mut outcome = None;
    parts::centred(ui, COLUMN, |ui| {
        ui.add_space((ui.clip_rect().height() * 0.06).clamp(theme::SPACE_XL, 56.0));
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!("Set up {name}"))
                    .font(theme::semibold(ui.ctx(), theme::HEADING_SIZE + 4.0))
                    .color(theme::rgb(theme.text)),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::ghost_button(ui, theme, "Skip setup")
                    .on_hover_text("Go to the Home screen; nothing runs")
                    .clicked()
                {
                    outcome = Some(Outcome::Close);
                }
            });
        });
        ui.add_space(theme::SPACE_LG);
        stepper(ctx, state, ui);
        ui.add_space(theme::SPACE_LG);
        egui::Frame::new()
            .fill(theme::rgb(theme.panel))
            .stroke(egui::Stroke::new(1.0, theme::accent_fill(theme)))
            .corner_radius(egui::CornerRadius::same(theme::RADIUS_XL))
            .inner_margin(theme::margin(theme::SPACE_LG + theme::SPACE_XS))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                let i = state.step.index();
                ui.label(
                    egui::RichText::new(format!("Step {} of {}", i + 1, Step::ALL.len()))
                        .font(theme::tabular(ui.ctx(), theme::SMALL_SIZE))
                        .color(theme::rgb(theme.text_faint)),
                );
                ui.add_space(theme::SPACE_XS);
                let approval = match state.step {
                    Step::Footage => {
                        footage_step(ctx, state, ui);
                        None
                    }
                    Step::Music => {
                        music_step(ctx, state, ui);
                        None
                    }
                    Step::Deliverables => {
                        deliverables_step(ctx, state, ui);
                        None
                    }
                    Step::Approve => Some(approve_step(ctx, state, ui)),
                };
                ui.add_space(theme::SPACE_LG);
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .hline(rect.x_range(), rect.center().y, theme::ink_line(theme));
                ui.add_space(theme::SPACE_MD);
                ui.horizontal(|ui| {
                    if i > 0 && widgets::outline_button(ui, theme, "Back").clicked() {
                        state.step = Step::ALL[i - 1];
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = theme::SPACE_MD;
                        match approval {
                            Some(approval) => {
                                let go = ui
                                    .add_enabled_ui(!approval.jobs.is_empty(), |ui| {
                                        widgets::button_with(
                                            ui,
                                            theme,
                                            widgets::ButtonKind::Key,
                                            Some(icons::SPARKLES),
                                            "Start analysis",
                                        )
                                    })
                                    .inner
                                    .clicked();
                                ui.label(
                                    egui::RichText::new(if approval.reading {
                                        "Estimating\u{2026}".to_owned()
                                    } else {
                                        format!(
                                            "About {} in all",
                                            project::duration_text(approval.total)
                                        )
                                    })
                                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                                    .color(theme::rgb(theme.text_muted)),
                                );
                                if go {
                                    outcome = Some(Outcome::Start {
                                        jobs: approval.jobs,
                                        plan: approval.plan,
                                    });
                                }
                            }
                            None => {
                                // Nothing after the footage step works
                                // without clips, so it is the one gate.
                                let has_clips = state.step != Step::Footage
                                    || state
                                        .scan
                                        .for_work(&state.work)
                                        .is_some_and(|s| !s.clips.is_empty());
                                if let Some(next) = Step::ALL.get(i + 1) {
                                    let go = ui
                                        .add_enabled_ui(has_clips, |ui| {
                                            widgets::primary_button(ui, theme, "Continue")
                                        })
                                        .inner
                                        .on_disabled_hover_text(
                                            "Pick a folder with video clips in it first",
                                        )
                                        .clicked();
                                    if go {
                                        state.step = *next;
                                    }
                                }
                            }
                        }
                    });
                });
            });
        ui.add_space(theme::SPACE_XL * 2.0);
    });
    outcome
}

/// The steps across the top: a numbered circle and a name each, joined by
/// hairlines; done steps carry a check, the current one is filled. Any
/// step can be clicked.
fn stepper(ctx: &PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    // The step circles' diameter.
    const DOT: f32 = 22.0;
    let theme = ctx.theme;
    let width = ui.available_width();
    let (row, _) = ui.allocate_exact_size(egui::vec2(width, DOT), egui::Sense::hover());
    let font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    let items: Vec<_> = Step::ALL
        .into_iter()
        .map(|step| {
            let galley = ui.painter().layout_no_wrap(
                step.title().to_owned(),
                font.clone(),
                egui::Color32::PLACEHOLDER,
            );
            (step, galley)
        })
        .collect();
    let content: f32 = items
        .iter()
        .map(|(_, g)| DOT + theme::SPACE_SM + g.size().x)
        .sum();
    let gap = ((width - content) / (items.len().max(2) - 1) as f32).max(theme::SPACE_LG);
    let mut x = row.left();
    let here = state.step.index();
    let count = items.len();
    for (i, (step, galley)) in items.into_iter().enumerate() {
        let item_w = DOT + theme::SPACE_SM + galley.size().x;
        let rect = egui::Rect::from_min_size(egui::pos2(x, row.top()), egui::vec2(item_w, DOT));
        let response = ui.interact(
            rect,
            ui.scope_id().with(("wizard-step", i)),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Tab, true, i == here, step.title())
        });
        let lift = widgets::hover_t(ui, response.id, response.hovered());
        let dot = egui::Rect::from_min_size(rect.min, egui::Vec2::splat(DOT));
        let painter = ui.painter();
        let (label_ink, number_ink) = if i == here {
            painter.circle_filled(dot.center(), DOT / 2.0, theme::rgb(theme.text));
            (theme::rgb(theme.text), theme::rgb(theme.panel))
        } else if i < here {
            painter.circle_filled(dot.center(), DOT / 2.0, theme::selected_fill(theme));
            (
                widgets::blend(theme.text_muted, theme.text, lift),
                theme::rgb(theme.text),
            )
        } else {
            painter.circle_stroke(dot.center(), DOT / 2.0 - 0.5, theme::input_line(theme));
            (
                widgets::blend(theme.text_muted, theme.text, lift),
                theme::rgb(theme.text_muted),
            )
        };
        if i < here {
            icons::paint(painter, dot, icons::CHECK, icons::SMALL, number_ink);
        } else {
            painter.text(
                dot.center(),
                egui::Align2::CENTER_CENTER,
                (i + 1).to_string(),
                theme::tabular(ui.ctx(), theme::SMALL_SIZE),
                number_ink,
            );
        }
        painter.galley(
            egui::pos2(
                dot.right() + theme::SPACE_SM,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            label_ink,
        );
        if i + 1 < count {
            let from = rect.right() + theme::SPACE_MD;
            let to = rect.right() + gap - theme::SPACE_MD;
            if to > from {
                painter.hline(from..=to, rect.center().y, theme::ink_line(theme));
            }
        }
        if response.clicked() {
            state.step = step;
        }
        x += item_w + gap;
    }
}

/// A step's title in 14 px Medium and its one-line description.
fn step_head(ui: &mut egui::Ui, theme: &crate::theme::Palette, title: &str, description: &str) {
    ui.label(
        egui::RichText::new(title)
            .font(theme::medium(ui.ctx(), theme::TITLE_SIZE))
            .color(theme::rgb(theme.text)),
    );
    ui.add_space(2.0);
    parts::muted(ui, theme, description);
    ui.add_space(theme::SPACE_LG);
}

/// A path row: the path as a read-only field and a "Choose…" button;
/// returns whether the button was pressed.
fn choose_row(
    ui: &mut egui::Ui,
    theme: &crate::theme::Palette,
    label: &str,
    glyph: char,
    path: Option<&Path>,
    none: &str,
    button: &str,
) -> bool {
    let mut pressed = false;
    parts::form_row(ui, theme, label, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            pressed = widgets::outline_button(ui, theme, button).clicked();
            ui.add_space(theme::SPACE_XS);
            let width = ui.available_width();
            parts::path_box(ui, theme, glyph, path, none, width);
        });
    });
    ui.add_space(theme::SPACE_XS + 2.0);
    pressed
}

/// A list of rows inside one hairline box (Selects' option list); `add`
/// draws the rows, with a [`rule`] between each two.
fn list_box(ui: &mut egui::Ui, theme: &crate::theme::Palette, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(theme::ink(theme, theme::INK_03))
        .stroke(theme::ink_line(theme))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_MD))
        .inner_margin(theme::margin_xy(theme::SPACE_MD, 0.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui);
        });
}

/// A hairline across a [`list_box`], between two rows.
fn rule(ui: &mut egui::Ui, theme: &crate::theme::Palette) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, theme::ink_line(theme));
}

/// Step 1: the footage folder.
fn footage_step(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let theme = ctx.theme;
    step_head(
        ui,
        theme,
        "Footage",
        "Every video directly in this folder is indexed.",
    );
    let footage = ctx
        .project
        .as_ref()
        .and_then(|p| p.settings.footage.clone());
    let change = choose_row(
        ui,
        theme,
        "Folder",
        icons::FOLDER,
        footage.as_deref(),
        "Not set",
        "Change\u{2026}",
    );
    if change
        && let Some(picked) = rfd::FileDialog::new()
            .set_title("The folder your clips are in")
            .pick_folder()
        && let Some(p) = ctx.project.as_mut()
    {
        p.settings.footage = Some(picked);
        let _ = p.save_settings();
        state.generation += 1;
    }
    let scanned = state.scan.for_work(&state.work).cloned();
    let busy = state.scan.busy();
    parts::form_row(ui, theme, "Clips", |ui| match &scanned {
        Some(s) if s.error.is_some() => {
            parts::status_line(
                ui,
                theme,
                parts::Kind::Danger,
                &s.error.clone().unwrap_or_default(),
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
            ui.label(
                egui::RichText::new(text)
                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(theme.text)),
            );
            if busy {
                ui.add(
                    egui::Spinner::new()
                        .size(icons::SMALL - 2.0)
                        .color(theme::rgb(theme.text_muted)),
                );
            }
        }
        None => {
            ui.add(
                egui::Spinner::new()
                    .size(icons::SMALL - 2.0)
                    .color(theme::rgb(theme.text_muted)),
            );
            ui.label(
                egui::RichText::new("Reading the folder\u{2026}")
                    .font(theme::font(theme::BODY_SIZE))
                    .color(theme::rgb(theme.text_muted)),
            );
        }
    });
    let empty = scanned
        .as_ref()
        .is_some_and(|s| s.error.is_none() && s.clips.is_empty())
        && !busy;
    if empty && let Some(folder) = footage.as_deref() {
        let why = project::footage_problem(folder)
            .unwrap_or_else(|| format!("{} has no video clips in it.", folder.display()));
        ui.add_space(theme::SPACE_XS);
        parts::status_line(ui, theme, parts::Kind::Warning, &why);
        if let Some((sub, _)) = project::clips_below(folder)
            && widgets::outline_button(ui, theme, "Use that folder").clicked()
            && let Some(p) = ctx.project.as_mut()
        {
            p.settings.footage = Some(sub);
            let _ = p.save_settings();
            state.generation += 1;
        }
    }
    if let Some(s) = scanned.filter(|s| !s.clips.is_empty()) {
        ui.add_space(theme::SPACE_MD);
        let gap = theme::SPACE_XS + 2.0;
        let size = egui::vec2(112.0, 63.0);
        let fit = ((ui.available_width() + gap) / (size.x + gap))
            .floor()
            .max(1.0) as usize;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for clip in s.clips.iter().take(fit) {
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
                let t = clip.duration.map_or(1.0, |d| d * 0.4);
                parts::thumb(ctx.thumbs, ui, theme, rect, Some((clip.path.as_path(), t)));
                response.on_hover_text(&clip.id);
            }
        });
    }
}

/// What a cut without music is, in the words set-up and the guide use.
pub(crate) const NO_MUSIC_NOTE: &str =
    "No music: cuts paced every two seconds, with the clips' own sound";

/// Step 2: track and end cards, last week's carried over.
fn music_step(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let theme = ctx.theme;
    step_head(
        ui,
        theme,
        "Music and end cards",
        "The track every cut is timed to (or none), and the card each aspect closes on.",
    );
    let keys: BTreeSet<String> = state
        .recipe()
        .deliverables
        .iter()
        .map(|d| d.end_card_key().to_owned())
        .collect();
    let Some(settings) = ctx.project.as_ref().map(|p| p.settings.clone()) else {
        return;
    };
    let silent = ctx.project.as_ref().is_some_and(project::Project::no_music);
    let pick_track = choose_row(
        ui,
        theme,
        "Track",
        icons::MUSIC,
        settings.track.as_deref(),
        if silent {
            NO_MUSIC_NOTE
        } else {
            "No track yet"
        },
        "Choose\u{2026}",
    );
    let mut pick_card = None;
    for key in &keys {
        if choose_row(
            ui,
            theme,
            &format!("End card {key}"),
            icons::IMAGE,
            settings.end_cards.get(key).map(PathBuf::as_path),
            "None",
            "Choose\u{2026}",
        ) {
            pick_card = Some(key.clone());
        }
    }
    let start = recipe::night_dir(&state.work);
    if pick_track
        && let Some(track) = rfd::FileDialog::new()
            .set_title("The track the cut is timed to")
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
        ui.add_space(theme::SPACE_SM);
        if widgets::button_with(
            ui,
            theme,
            widgets::ButtonKind::Outline,
            Some(icons::HISTORY),
            &format!("Carry over {from}'s"),
        )
        .on_hover_text("Fill the track and end cards still missing from the last project")
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
        ui.add_space(theme::SPACE_MD);
        for note in &state.notes {
            parts::status_line(ui, theme, parts::Kind::Success, note);
        }
    }
}

/// Step 3: the recipe's deliverables.
fn deliverables_step(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    // A deliverable's row.
    const ROW: f32 = 40.0;
    let theme = ctx.theme;
    step_head(
        ui,
        theme,
        "Deliverables",
        "What this project makes. Untick one to leave it out of the first drafts; change the \
         list itself on the Home screen.",
    );
    let cards = ctx
        .project
        .as_ref()
        .map(|p| p.settings.end_cards.clone())
        .unwrap_or_default();
    let list = state.recipe().deliverables.clone();
    if list.is_empty() {
        parts::status_line(
            ui,
            theme,
            parts::Kind::Warning,
            "The recipe has no deliverables.",
        );
        return;
    }
    list_box(ui, theme, |ui| {
        for (i, d) in list.into_iter().enumerate() {
            if i > 0 {
                rule(ui, theme);
            }
            let mut on = !state.skip.contains(&d.id);
            ui.horizontal(|ui| {
                ui.set_min_height(ROW);
                if widgets::checkbox(ui, theme, &mut on, &d.id).changed() {
                    if on {
                        state.skip.remove(&d.id);
                    } else {
                        state.skip.insert(d.id.clone());
                    }
                }
                ui.label(
                    egui::RichText::new(format!(
                        "{} \u{b7} {:.0} s",
                        recipe::aspect_key(d.aspect),
                        d.seconds
                    ))
                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(theme.text_muted)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if cards.contains_key(d.end_card_key()) {
                        widgets::pill(ui, theme, "end card", widgets::Tone::Success);
                    } else {
                        widgets::pill(ui, theme, "no end card", widgets::Tone::Neutral);
                    }
                });
            });
        }
    });
}

/// Step 4: the approval card. Returns what is chosen, for the footer.
fn approve_step(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) -> Approval {
    // An analysis row.
    const ROW: f32 = 48.0;
    let theme = ctx.theme;
    step_head(
        ui,
        theme,
        "Analyse the footage",
        "Runs on this machine, in this order. Open the editor as soon as indexing is done; \
         captions keep going in the background.",
    );
    let scanned = state.scan.for_work(&state.work).cloned();
    let rows = rows(ctx, state, scanned.as_ref());
    let mut total = 0.0;
    let mut chosen: Vec<&Row> = Vec::new();
    list_box(ui, theme, |ui| {
        for (i, row) in rows.iter().enumerate() {
            if i > 0 {
                rule(ui, theme);
            }
            let can = row.blocked.is_none() && !row.jobs.is_empty();
            let mut on = can && !state.off.contains(row.key);
            ui.horizontal(|ui| {
                ui.set_min_height(ROW);
                let changed = ui
                    .add_enabled_ui(can, |ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(132.0, ROW),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.set_min_width(132.0);
                                widgets::checkbox(ui, theme, &mut on, row.title)
                            },
                        )
                        .inner
                    })
                    .inner
                    .changed();
                if changed {
                    if on {
                        state.off.remove(row.key);
                    } else {
                        state.off.insert(row.key);
                    }
                }
                let estimate = if can {
                    format!("\u{2248} {}", project::duration_text(row.estimate.secs))
                } else {
                    String::new()
                };
                let estimate_w = 72.0;
                let room = (ui.available_width() - estimate_w - theme::SPACE_SM).max(0.0);
                ui.allocate_ui(egui::vec2(room, ROW), |ui| {
                    ui.set_min_width(room);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add_space(theme::SPACE_SM);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&row.estimate.what)
                                    .font(theme::font(theme::BODY_SIZE))
                                    .color(theme::rgb(if can {
                                        theme.text
                                    } else {
                                        theme.text_muted
                                    })),
                            )
                            .truncate(),
                        );
                        let note = match (&row.blocked, row.estimate.basis.is_empty()) {
                            (Some(why), _) if why.eq_ignore_ascii_case(&row.estimate.what) => {
                                String::new()
                            }
                            (Some(why), _) => why.clone(),
                            (None, true) => String::new(),
                            (None, false) => row.estimate.basis.clone(),
                        };
                        if !note.is_empty() {
                            ui.label(
                                egui::RichText::new(note)
                                    .font(theme::font(theme::SMALL_SIZE))
                                    .color(theme::rgb(theme.text_faint)),
                            );
                        }
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(estimate)
                            .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                            .color(theme::rgb(if on { theme.text } else { theme.text_faint })),
                    );
                });
            });
            if can && on {
                total += row.estimate.secs;
                chosen.push(row);
            }
        }
    });
    let reading = scanned.as_ref().is_none_or(|s| s.speeds.is_none());
    // Index first, the music map early (it is quick and plans need it),
    // captions next (long; the editor opens meanwhile), then what reads
    // the captions.
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
    Approval {
        total,
        reading,
        jobs,
        plan,
    }
}

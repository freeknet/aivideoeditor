// ============================================================================
// Module:       gui::assistant::cards
// Description:  The chat's settings and approval cards: "Here's our plan"
//               before a first cut is drafted, "Allow this?" before a slow
//               engine step starts, and the live progress row of a step the
//               chat started.
//
// Dependencies: egui; crate::{commands, jobs, plan, project, refs, render};
//               crate::gui::pages::night::pipeline (the step commands);
//               super::look
// ============================================================================

//! Cards above the composer.
//!
//! **First cut.** When the person asks for a new plan (the "Suggest a first
//! cut" quick action, or the agent calling `plan_draft` with no length or
//! aspect) the chat shows Selects' "Here's our plan" card: length, aspect,
//! pacing, an optional hero shot taken from the selection, and three rule
//! switches. Continue starts the same `ave plan` job the plan page's
//! "Draft new…" window starts ([`crate::gui::pages::plan::draft_job`]); the
//! rule switches are written into the drafted plan's rules, and pacing and
//! the hero, which take editorial judgment, become a message for the agent.
//!
//! **Approval.** The agent's `run_step` calls (index, caption, render) never
//! run on their own: each becomes an "Allow this?" strip naming what the
//! step will do and how long the night's measured speeds say it will take
//! (the get-started wizard's estimates, [`crate::project::estimate_index`]
//! and [`crate::project::estimate_caption`]). Approve starts the job in the
//! app's job list and a progress row (step, percent, time left, Stop) follows
//! it in the chat. When the person lets the assistant apply edits, steps are
//! started at once and labelled auto-approved.
//!
//! Everything here is drawing and plain data; starting jobs and writing the
//! log is the panel's business.

use crate::gui::ui::hint::Hint as _;
use egui::{Align, CornerRadius, Layout, RichText, Sense, Stroke, Ui, Vec2};
use serde_json::Value;

use super::look;
use crate::commands::Cmd;
use crate::gui::ui::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::gui::ui::{icons, widgets};
use crate::plan::{Aspect, Rules};
use crate::project::{self, Project, StepEstimate};
use crate::refs::{Modifier, Reference, Target};
use crate::theme::Palette;

/// Wall seconds a render takes per second of video when nothing measured
/// it (x264 at 1080p on the editing PC, with the 720p preview after it).
const RENDER_RATIO: f32 = 2.0;

/// The length choices of the first-cut card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Length {
    /// 15 seconds.
    Fifteen,
    /// 30 seconds.
    Thirty,
    /// 60 seconds.
    Sixty,
    /// Any length, typed.
    Custom,
}

/// How the cut moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pacing {
    /// Longer shots, cuts on phrases.
    Calm,
    /// The drafter's phrase pacing: cuts on the beat.
    OnTheBeat,
    /// Runs of short shots on the build and the drop.
    Bursts,
}

impl Pacing {
    /// How the card and the messages name it.
    fn label(self) -> &'static str {
        match self {
            Self::Calm => "calm",
            Self::OnTheBeat => "on the beat",
            Self::Bursts => "bursts",
        }
    }

    /// What the agent is asked to do for it, when the draft does not
    /// already do it.
    fn ask(self) -> Option<&'static str> {
        match self {
            Self::Calm => Some(
                "make it calmer: fewer, longer shots, cutting on phrase starts rather than every bar",
            ),
            Self::OnTheBeat => None,
            Self::Bursts => Some(
                "add bursts: runs of one- and two-beat shots into the drop and on the build, keeping the beat total",
            ),
        }
    }
}

/// The first-cut settings card.
#[derive(Clone, Debug)]
pub(super) struct FirstCut {
    /// Length choice.
    pub(super) length: Length,
    /// Seconds when the length is custom.
    pub(super) custom: f32,
    /// Frame shape.
    pub(super) aspect: Aspect,
    /// Pacing.
    pub(super) pacing: Pacing,
    /// The shot to build around, from the selection.
    pub(super) hero: Option<Reference>,
    /// Rule: no repeated shots (same clip, look-alikes).
    pub(super) no_repeats: bool,
    /// Rule: the same performer not again within a few shots.
    pub(super) people_spacing: bool,
    /// Rule: the drop lands on the hero shot.
    pub(super) drop_on_hero: bool,
    /// The plan name the agent asked for, if it named one.
    pub(super) name_hint: Option<String>,
    /// The agent's `plan_draft` call that opened the card, if one did.
    pub(super) tool: Option<String>,
    /// Why Continue was refused.
    pub(super) error: Option<String>,
}

/// What a card's buttons asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    /// Continue / Approve.
    Go,
    /// Cancel.
    Cancel,
}

/// The first footage reference in `selection`, as a hero candidate.
pub(super) fn hero_candidate(selection: &[Reference]) -> Option<Reference> {
    selection
        .iter()
        .find(|r| match &r.target {
            Target::Shot { .. }
            | Target::Segment { .. }
            | Target::Range { .. }
            | Target::Frame { .. } => true,
            Target::Bars { .. } | Target::Person { .. } | Target::Plan { .. } => false,
        })
        .cloned()
}

/// Whether `name` is a usable plan file name.
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

impl FirstCut {
    /// A card with the defaults (30 s, 9:16, on the beat, every rule on),
    /// shaped by the agent's `plan_draft` arguments when it opened it.
    pub(super) fn new(selection: &[Reference], tool: Option<String>, input: &Value) -> Self {
        let aspect = input
            .get("aspect")
            .cloned()
            .and_then(|a| serde_json::from_value(a).ok())
            .unwrap_or(Aspect::Vertical);
        let seconds = input
            .get("seconds")
            .and_then(Value::as_f64)
            .map(|s| s as f32);
        let length = match seconds {
            None => Length::Thirty,
            Some(s) if (s - 15.0).abs() < 0.5 => Length::Fifteen,
            Some(s) if (s - 30.0).abs() < 0.5 => Length::Thirty,
            Some(s) if (s - 60.0).abs() < 0.5 => Length::Sixty,
            Some(_) => Length::Custom,
        };
        Self {
            length,
            custom: seconds.unwrap_or(45.0).clamp(8.0, 240.0),
            aspect,
            pacing: Pacing::OnTheBeat,
            hero: hero_candidate(selection),
            no_repeats: true,
            people_spacing: true,
            drop_on_hero: true,
            name_hint: input
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|n| valid_name(n))
                .map(str::to_owned),
            tool,
            error: None,
        }
    }

    /// The target length in seconds.
    pub(super) fn seconds(&self) -> f32 {
        match self.length {
            Length::Fifteen => 15.0,
            Length::Thirty => 30.0,
            Length::Sixty => 60.0,
            Length::Custom => self.custom.clamp(8.0, 240.0),
        }
    }

    /// A plan name not taken yet: the agent's, else `cut-30`, `cut-30-2`, …
    pub(super) fn name(&self, taken: &[String]) -> String {
        let base = self
            .name_hint
            .clone()
            .unwrap_or_else(|| format!("cut-{:.0}", self.seconds()));
        if !taken.contains(&base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|n| !taken.contains(n))
            .unwrap_or(base)
    }

    /// The settings in one line: `30 s · 9:16 · on the beat · hero …`.
    pub(super) fn summary(&self) -> String {
        let mut parts = vec![
            format!("{:.0} s", self.seconds()),
            aspect_label(self.aspect).to_owned(),
            self.pacing.label().to_owned(),
        ];
        if let Some(h) = &self.hero {
            parts.push(format!("hero {h}"));
        }
        let off: Vec<&str> = [
            (self.no_repeats, "repeated shots allowed"),
            (self.people_spacing, "no people spacing"),
            (self.drop_on_hero, "drop not tied to the hero"),
        ]
        .into_iter()
        .filter(|(on, _)| !on)
        .map(|(_, what)| what)
        .collect();
        if !off.is_empty() {
            parts.push(off.join(", "));
        }
        parts.join(" · ")
    }

    /// The drafted plan's rules with the switches applied: a switch that is
    /// off turns its rule off; one that is on keeps the night's value.
    pub(super) fn apply_rules(&self, rules: &mut Rules) {
        if !self.no_repeats {
            rules.same_clip_gap = 0.0;
            rules.lookalike = 0.0;
        }
        if !self.people_spacing {
            rules.person_gap = 0;
        }
        if !self.drop_on_hero {
            rules.drop_subject = String::new();
        }
    }

    /// The message for the agent once plan `name` is drafted: what the
    /// drafter cannot do by itself (pacing, the hero). `None` when the
    /// draft already is the cut asked for and no agent asked for it.
    pub(super) fn follow_up(&self, name: &str) -> Option<String> {
        let mut asks: Vec<String> = Vec::new();
        if let Some(a) = self.pacing.ask() {
            asks.push(a.to_owned());
        }
        if let Some(h) = &self.hero {
            let hero = h.clone().with(Modifier::Hero);
            asks.push(if self.drop_on_hero {
                format!("put {hero} on the drop")
            } else {
                format!("feature {hero}")
            });
        }
        if asks.is_empty() && self.tool.is_none() {
            return None;
        }
        let head = format!(
            "I drafted @plan:{name} as the first cut ({}).",
            self.summary()
        );
        Some(if asks.is_empty() {
            format!("{head} Read it and tell me what you would change.")
        } else {
            format!(
                "{head} Now {}. Stage the edits as one proposal.",
                asks.join("; ")
            )
        })
    }
}

/// `9:16`, `4:5`, `16:9`.
fn aspect_label(a: Aspect) -> &'static str {
    match a {
        Aspect::Vertical => "9:16",
        Aspect::Feed => "4:5",
        Aspect::Wide => "16:9",
    }
}

/// Selects' option-card frame: radius 14, a `tone-700` hairline, the panel
/// fill, padding 10 × 12.
fn card_frame(theme: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(theme::rgb(theme.panel))
        .stroke(Stroke::new(1.0, theme::accent_fill(theme)))
        .corner_radius(CornerRadius::same(theme::RADIUS_XL))
        .inner_margin(theme::margin_xy(SPACE_MD, 10.0))
}

/// A field's name in the card: 12 px muted, in a fixed column so the
/// controls line up.
fn field(ui: &mut Ui, theme: &Palette, label: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(64.0, theme::CONTROL_HEIGHT), Sense::hover());
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        label,
        theme::font(theme::BODY_SIZE),
        theme::rgb(theme.text_muted),
    );
}

/// The "Here's our plan" card. `selection` offers a hero when none is set.
pub(super) fn first_cut(
    ui: &mut Ui,
    theme: &Palette,
    card: &mut FirstCut,
    selection: &[Reference],
) -> Option<Choice> {
    let mut choice = None;
    card_frame(theme).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = Vec2::new(SPACE_XS, SPACE_SM);
        ui.horizontal(|ui| {
            icons::icon(ui, icons::SPARKLES, icons::SMALL, theme::rgb(theme.accent));
            ui.label(
                RichText::new("Here's our plan")
                    .font(theme::medium(ui.ctx(), theme::LARGE_SIZE))
                    .color(look::tone_200(theme)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_btn(ui, theme, icons::X, "Close the first-cut card", "").clicked()
                {
                    choice = Some(Choice::Cancel);
                }
            });
        });
        ui.push_id("first-cut-length", |ui| {
            ui.horizontal_wrapped(|ui| {
                field(ui, theme, "Length");
                widgets::segmented(
                    ui,
                    theme,
                    &mut card.length,
                    &[
                        (Length::Fifteen, "15 s"),
                        (Length::Thirty, "30 s"),
                        (Length::Sixty, "60 s"),
                        (Length::Custom, "Custom"),
                    ],
                );
                if card.length == Length::Custom {
                    ui.add(
                        widgets::number(&mut card.custom, "Target length")
                            .range(8.0..=240.0)
                            .drag_points_per_step(1.0)
                            .suffix(" s"),
                    )
                    .hint("Target length in seconds; the cut lands on whole phrases");
                }
            });
        });
        ui.push_id("first-cut-aspect", |ui| {
            ui.horizontal_wrapped(|ui| {
                field(ui, theme, "Aspect");
                widgets::segmented(
                    ui,
                    theme,
                    &mut card.aspect,
                    &[
                        (Aspect::Vertical, "9:16"),
                        (Aspect::Feed, "4:5"),
                        (Aspect::Wide, "16:9"),
                    ],
                );
            });
        });
        ui.push_id("first-cut-pacing", |ui| {
            ui.horizontal_wrapped(|ui| {
                field(ui, theme, "Pacing");
                widgets::segmented(
                    ui,
                    theme,
                    &mut card.pacing,
                    &[
                        (Pacing::Calm, "Calm"),
                        (Pacing::OnTheBeat, "On the beat"),
                        (Pacing::Bursts, "Bursts"),
                    ],
                );
            });
        });
        ui.horizontal_wrapped(|ui| {
            field(ui, theme, "Hero shot");
            hero_chip(ui, theme, card, selection);
        });
        rule(ui, theme);
        ui.spacing_mut().item_spacing.y = 0.0;
        // The response is not needed: drawing it is the point.
        let _ = widgets::switch(ui, theme, &mut card.no_repeats, "No repeated shots")
            .hint("Two shots from one clip start far apart; look-alike shots count as repeats");
        // The response is not needed: drawing it is the point.
        let _ = widgets::switch(ui, theme, &mut card.people_spacing, "People spacing")
            .hint("The same performer does not come back within a few shots");
        // The response is not needed: drawing it is the point.
        let _ = widgets::switch(ui, theme, &mut card.drop_on_hero, "Drop on the hero").hint(
            "The shot on the drop is the hero: the one you picked, else the strongest fire shot",
        );
        ui.spacing_mut().item_spacing.y = SPACE_SM;
        if let Some(e) = &card.error {
            ui.add(
                egui::Label::new(
                    RichText::new(e)
                        .size(theme::BODY_SIZE)
                        .color(theme::rgb(theme.danger)),
                )
                .wrap(),
            );
        }
        ui.add_space(2.0);
        button_row(ui, |ui| {
            ui.spacing_mut().item_spacing.x = SPACE_SM;
            let go = widgets::primary_button(ui, theme, "Continue");
            if widgets::tip(go, theme, "Draft the first cut with these settings", "").clicked() {
                choice = Some(Choice::Go);
            }
            if widgets::ghost_button(ui, theme, "Cancel").clicked() {
                choice = Some(Choice::Cancel);
            }
        });
    });
    choice
}

/// The hero: a lime mention with a remove cross, or "None" and a button
/// taking the selection.
fn hero_chip(ui: &mut Ui, theme: &Palette, card: &mut FirstCut, selection: &[Reference]) {
    match card.hero.clone() {
        Some(h) => {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("@{}", h.short()))
                        .size(theme::LARGE_SIZE)
                        .color(theme::rgb(theme.accent)),
                )
                .selectable(false),
            )
            .hint(h.to_string());
            if widgets::icon_btn(ui, theme, icons::X, "Remove the hero shot", "").clicked() {
                card.hero = None;
            }
        }
        None => {
            ui.label(
                RichText::new("None")
                    .size(theme::BODY_SIZE)
                    .color(theme::rgb(theme.text_faint)),
            );
            if let Some(candidate) = hero_candidate(selection) {
                let r = widgets::button_with(
                    ui,
                    theme,
                    widgets::ButtonKind::Ghost,
                    Some(icons::PLUS),
                    "Use selection",
                );
                if widgets::tip(r, theme, &format!("Build around {candidate}"), "").clicked() {
                    card.hero = Some(candidate);
                }
            }
        }
    }
}

/// A card's buttons: one control-high row, laid out from the right (a bare
/// right-to-left layout would take all the height left).
fn button_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    // The response is not needed: drawing it is the point.
    let _ = ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), theme::CONTROL_HEIGHT),
        Layout::right_to_left(Align::Center),
        add,
    );
}

/// A hairline across a card.
fn rule(ui: &mut Ui, theme: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, theme::ink_line(theme));
}

/// A slow engine step the agent asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// Index the footage.
    Index,
    /// Caption the shot log.
    Caption,
    /// Render a plan.
    Render(String),
}

impl Step {
    /// The step a `run_step` call asks for.
    pub(super) fn from_input(input: &Value) -> Option<Self> {
        match input.get("step").and_then(Value::as_str)? {
            "index" => Some(Self::Index),
            "caption" => Some(Self::Caption),
            "render" => input
                .get("plan")
                .and_then(Value::as_str)
                .map(|p| Self::Render(p.to_owned())),
            _ => None,
        }
    }

    /// What it does, in a few words: `index the footage`.
    pub(super) fn what(&self) -> String {
        match self {
            Self::Index => "index the footage".to_owned(),
            Self::Caption => "caption the shot log".to_owned(),
            Self::Render(plan) => format!("render {plan}"),
        }
    }

    /// The progress row's name: `Indexing`.
    fn doing(&self) -> String {
        match self {
            Self::Index => "Indexing".to_owned(),
            Self::Caption => "Captioning".to_owned(),
            Self::Render(plan) => format!("Rendering {plan}"),
        }
    }

    /// The chip naming the call: `run_step · index`.
    fn chip(&self) -> String {
        let key = match self {
            Self::Index => "index",
            Self::Caption => "caption",
            Self::Render(_) => "render",
        };
        format!("{} \u{b7} {key}", crate::mcp::RUN_STEP)
    }

    /// The job that runs it: the same commands the night page's pipeline
    /// and the deliver page queue.
    pub(super) fn job(&self, project: &Project) -> Result<(String, Cmd), String> {
        let work = &project.work;
        match self {
            Self::Index => project
                .settings
                .footage
                .as_ref()
                .map(|f| crate::gui::pages::night::pipeline::index_cmd(work, f, false))
                .ok_or_else(|| {
                    "The project has no footage folder: set it on the Home screen.".to_owned()
                }),
            Self::Caption if project.index.is_none() => {
                Err("Analyse the footage before captioning it.".to_owned())
            }
            Self::Caption => Ok(crate::gui::pages::night::pipeline::caption_cmd(work)),
            Self::Render(plan) => {
                let path = work.join("plans").join(format!("{plan}.toml"));
                if !path.exists() {
                    return Err(format!("There is no plan {plan}."));
                }
                Ok((
                    format!("render {plan}"),
                    Cmd::Render {
                        plan: path,
                        work: work.clone(),
                        encoder: crate::render::Encoder::X264,
                        force: false,
                    },
                ))
            }
        }
    }

    /// What the step will do and how long it should take, from the speeds
    /// measured on this night (or its neighbours), as the get-started wizard
    /// estimates it. Reads the clip cache: call once per card.
    pub(super) fn estimate(&self, project: &Project) -> Result<StepEstimate, String> {
        let work = &project.work;
        let clips = || {
            project
                .settings
                .footage
                .as_ref()
                .map(|f| project::scan_footage(work, f, false))
                .transpose()
                .map(Option::unwrap_or_default)
                .map_err(|e| format!("cannot read the footage folder: {e:#}"))
        };
        match self {
            Self::Index => {
                if project.settings.footage.is_none() {
                    return Err("no footage folder".to_owned());
                }
                Ok(project::estimate_index(&clips()?, &project::speeds(work)))
            }
            Self::Caption => Ok(project::estimate_caption(
                project.index.as_ref(),
                &clips()?,
                &project::speeds(work),
            )),
            Self::Render(plan) => {
                let path = work.join("plans").join(format!("{plan}.toml"));
                let p = crate::plan::Plan::load(&path).map_err(|e| format!("{e:#}"))?;
                let video = project.music.as_ref().map_or(0.0, |m| {
                    m.beat_time(p.end_bar, 0) - m.beat_time(p.start_bar, 0)
                });
                Ok(StepEstimate {
                    what: format!(
                        "{} shots, {} of video, then a 720p preview",
                        p.shots.len(),
                        project::duration_text(video)
                    ),
                    secs: video * RENDER_RATIO,
                    basis: "assumed".to_owned(),
                })
            }
        }
    }
}

/// A step waiting for the person's yes.
#[derive(Clone, Debug)]
pub(super) struct Approval {
    /// The `run_step` call.
    pub(super) id: String,
    /// The step.
    pub(super) step: Step,
    /// What it will do and take, once computed (`Err`: why not).
    pub(super) estimate: Option<Result<StepEstimate, String>>,
    /// Read back from the log rather than asked for now: never started
    /// without the person, whatever the permission says.
    pub(super) restored: bool,
}

/// "Allow this?": Selects' approval strip above the composer.
pub(super) fn approval(ui: &mut Ui, theme: &Palette, a: &Approval) -> Option<Choice> {
    let mut choice = None;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, theme::rgb(theme.raised)),
    );
    egui::Frame::new()
        .inner_margin(theme::margin_xy(SPACE_MD, SPACE_SM))
        .outer_margin(egui::Margin {
            left: 0,
            right: 0,
            top: 0,
            bottom: theme::as_margin(SPACE_SM),
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::new(SPACE_SM, SPACE_SM);
            ui.horizontal(|ui| {
                icons::icon(
                    ui,
                    icons::TRIANGLE_ALERT,
                    12.0,
                    theme::rgb(theme.warning).gamma_multiply(0.8),
                );
                ui.label(
                    RichText::new("Allow this?")
                        .size(theme::BODY_SIZE)
                        .color(theme::rgb(theme.text_muted)),
                );
                egui::Frame::new()
                    .fill(theme::rgb(theme.raised))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(theme::margin_xy(6.0, 2.0))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(a.step.chip())
                                .size(theme::SMALL_SIZE)
                                .color(look::tone_200(theme)),
                        );
                    });
            });
            let (detail, blocked) = match &a.estimate {
                Some(Ok(e)) => {
                    let basis = if e.basis.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", e.basis)
                    };
                    (
                        format!(
                            "{}: {}\nAbout {}{basis}",
                            capitalised(&a.step.what()),
                            e.what,
                            project::duration_text(e.secs)
                        ),
                        false,
                    )
                }
                Some(Err(why)) => (
                    format!("{}: cannot start: {why}", capitalised(&a.step.what())),
                    true,
                ),
                None => (
                    format!("{}: estimating\u{2026}", capitalised(&a.step.what())),
                    false,
                ),
            };
            egui::Frame::new()
                .fill(theme::rgb(theme.app))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(theme::margin(SPACE_SM))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            RichText::new(detail)
                                .font(theme::tabular(ui.ctx(), theme::SMALL_SIZE))
                                .color(theme::rgb(theme.text_muted)),
                        )
                        .wrap(),
                    );
                });
            button_row(ui, |ui| {
                let go = ui
                    .add_enabled_ui(!blocked, |ui| widgets::primary_button(ui, theme, "Approve"))
                    .inner;
                if widgets::tip(go, theme, "Start it; progress shows in the chat", "").clicked() {
                    choice = Some(Choice::Go);
                }
                if widgets::ghost_button(ui, theme, "Cancel").clicked() {
                    choice = Some(Choice::Cancel);
                }
            });
        });
    choice
}

/// `Index the footage`.
pub(super) fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().chain(chars).collect::<String>()
    })
}

/// A step the chat started, followed until it ends.
#[derive(Clone, Debug)]
pub(super) struct Running {
    /// The job in the app's job list.
    pub(super) job: u64,
    /// The step.
    pub(super) step: Step,
    /// Whether it started without asking (the permission said so).
    pub(super) auto: bool,
}

/// The progress row of a running step: its name, percent and time left,
/// a thin bar, and Stop. Returns whether Stop was pressed.
pub(super) fn progress(
    ui: &mut Ui,
    theme: &Palette,
    run: &Running,
    job: &crate::jobs::Job,
) -> bool {
    let mut stop = false;
    let elapsed = job.started.elapsed().as_secs_f32();
    let fraction = job.progress.fraction();
    let left = job.progress.remaining(elapsed);
    let mut parts = vec![run.step.doing()];
    if let Some(f) = fraction {
        parts.push(format!("{:.0}%", f * 100.0));
    }
    parts.push(match left {
        Some(s) => format!("{} left", project::duration_text(s)),
        None => "estimating time left".to_owned(),
    });
    if job.cancelling() {
        parts.push("stopping\u{2026}".to_owned());
    }
    ui.add_space(SPACE_SM);
    egui::Frame::new()
        .stroke(theme::ink_line(theme))
        .corner_radius(CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(theme::margin_xy(SPACE_MD, SPACE_SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = Vec2::new(SPACE_SM, SPACE_XS);
            ui.horizontal(|ui| {
                icons::icon(ui, icons::LOADER, 12.0, theme::rgb(theme.text_muted));
                ui.label(
                    RichText::new(parts.join(" \u{b7} "))
                        .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                        .color(look::tone_200(theme)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let r = ui
                        .add_enabled_ui(!job.cancelling(), |ui| {
                            widgets::icon_btn(ui, theme, icons::SQUARE, "Stop", "")
                        })
                        .inner;
                    if widgets::tip(r, theme, "Stop: what is finished is kept", "").clicked() {
                        stop = true;
                    }
                    if run.auto {
                        // The response is not needed: drawing it is the point.
                        let _ = widgets::pill(ui, theme, "Auto-approved", widgets::Tone::Neutral);
                    }
                });
            });
            // The response is not needed: drawing it is the point.
            let _ = widgets::meter(
                ui,
                theme,
                ui.scope_id().with(("job-progress", run.job)),
                fraction.unwrap_or(0.0),
                theme.accent,
            );
            if let Some(stage) = &job.progress.stage {
                ui.label(
                    RichText::new(stage)
                        .size(theme::SMALL_SIZE)
                        .color(theme::rgb(theme.text_faint)),
                );
            }
        });
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(250));
    stop
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{FirstCut, Length, Pacing, Step};
    use crate::plan::{Aspect, Rules};
    use crate::refs::{Reference, Target};

    #[test]
    fn the_card_takes_the_agents_arguments_and_the_selection() -> anyhow::Result<()> {
        let sel = vec![
            Reference::new(Target::Bars { from: 1, to: 2 }),
            Reference::new(Target::Segment {
                id: "MVI_1/3".into(),
            }),
        ];
        let card = FirstCut::new(
            &sel,
            Some("t1".into()),
            &json!({"name": "reel-15", "seconds": 15, "aspect": "4x5"}),
        );
        assert_eq!(card.length, Length::Fifteen, "15 s picked");
        assert_eq!(card.aspect, Aspect::Feed, "4:5 picked");
        assert!(
            matches!(&card.hero, Some(r) if matches!(&r.target, Target::Segment { .. })),
            "the hero is the first footage reference: {:?}",
            card.hero
        );
        assert_eq!(
            card.name(&["reel-15".into()]),
            "reel-15-2",
            "a taken name gets a suffix"
        );
        let plain = FirstCut::new(&[], None, &json!({}));
        assert_eq!(plain.seconds(), 30.0, "30 s by default");
        assert_eq!(plain.name(&[]), "cut-30", "named after its length");
        assert_eq!(
            plain.follow_up("cut-30"),
            None,
            "a plain quick-action draft needs no agent"
        );
        Ok(())
    }

    #[test]
    fn switches_turn_rules_off_and_judgment_goes_to_the_agent() -> anyhow::Result<()> {
        let mut card = FirstCut::new(
            &[Reference::new(Target::Segment {
                id: "MVI_2/1".into(),
            })],
            None,
            &json!({}),
        );
        card.people_spacing = false;
        card.no_repeats = false;
        card.pacing = Pacing::Bursts;
        let mut rules = Rules::default();
        card.apply_rules(&mut rules);
        assert_eq!(rules.person_gap, 0, "people spacing off");
        assert_eq!(rules.same_clip_gap, 0.0, "repeats allowed");
        assert_eq!(
            rules.drop_subject,
            Rules::default().drop_subject,
            "drop rule kept"
        );
        let msg = card
            .follow_up("cut-30")
            .ok_or_else(|| anyhow::anyhow!("no follow-up"))?;
        assert!(msg.contains("@plan:cut-30"), "names the plan: {msg}");
        assert!(
            msg.contains("*@seg:MVI_2/1"),
            "the hero as a hero mention: {msg}"
        );
        assert!(msg.contains("bursts"), "the pacing: {msg}");
        Ok(())
    }

    #[test]
    fn steps_parse_from_the_tool_call() -> anyhow::Result<()> {
        assert_eq!(
            Step::from_input(&json!({"step": "render", "plan": "reel-30"})),
            Some(Step::Render("reel-30".into())),
            "render names its plan"
        );
        assert_eq!(
            Step::from_input(&json!({"step": "render"})),
            None,
            "render without a plan is no step"
        );
        assert_eq!(
            Step::from_input(&json!({"step": "index"})),
            Some(Step::Index),
            "index"
        );
        assert_eq!(super::capitalised("index it"), "Index it", "capitalised");
        Ok(())
    }
}

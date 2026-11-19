// ============================================================================
// Module:       gui::pages::night::start
// Description:  The start page (Home with no night open): a drop zone for
//               this week's footage, a first request with prompt templates,
//               and the recent nights as a grid of thumbnails and status.
//
// Dependencies: egui, rfd; crate::gui::pages::night::recent
// ============================================================================

//! The start page, after Selects' home.
//!
//! - **Drop zone.** Drop this week's footage folder anywhere on the page
//!   (or pick it) and the night is set up from last week's — recipe, end
//!   cards, track — and the get-started wizard opens.
//! - **First request.** "What are we making?" with one-click templates. The
//!   text waits here until a night is set up (or a recent one opened); then
//!   it goes to the assistant through `Action::ChatInbox`.
//! - **Recent nights** as cards: the first clip's frame, the date, how far
//!   it got. Click one to open it.

use std::path::PathBuf;

use super::recent;
use crate::gui::app::Action;
use crate::gui::pages::PageCtx;
use crate::gui::ui::{theme, widgets};

/// The first-request templates: the chip's label and the request it fills.
pub(crate) const TEMPLATES: [(&str, &str); 3] = [
    (
        "Make this week's reels",
        "Make this week's reels: draft every deliverable in the recipe from the best moments, \
         beat-synced, no repeated shots or performers, drop on the hero fire shot.",
    ),
    (
        "Find the best fire moments",
        "Find the best fire moments of the night: list the strongest fire shots with their \
         segment ids, sharpest and best lit first, one per performer.",
    ),
    (
        "Cut a 60 s recap",
        "Cut a 60 s recap of the night in 16:9: open wide on the venue, build through the acts, \
         land the hero fire shot on the drop, end card last.",
    ),
];

/// The page's own state.
#[derive(Debug, Default)]
pub(crate) struct State {
    /// The first request, handed to the chat once a night is ready.
    pub(crate) prompt: String,
}

/// What the start page asks the night page to do.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    /// Set a new night up from this footage folder.
    NewNight(PathBuf),
    /// Open this night (work folder).
    Open(PathBuf),
}

/// A folder from files dropped on the window: a dropped folder, or the
/// folder holding a dropped file.
fn dropped(ui: &egui::Ui) -> Option<PathBuf> {
    let files = ui.ctx().input(|i| i.raw.dropped_files.clone());
    files.into_iter().find_map(|f| {
        let path = f.path().to_path_buf();
        if path.as_os_str().is_empty() {
            None
        } else if path.is_dir() {
            Some(path)
        } else {
            path.parent().map(std::path::Path::to_path_buf)
        }
    })
}

/// Draws the page.
pub(crate) fn draw(
    ctx: &mut PageCtx<'_>,
    state: &mut State,
    nights: &mut recent::State,
    ui: &mut egui::Ui,
) -> Option<Outcome> {
    recent::refresh(ctx, nights, false);
    let mut outcome = dropped(ui).map(Outcome::NewNight);
    let width = ui.available_width().min(1040.0);
    let side = ((ui.available_width() - width) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.add_space(side);
        ui.vertical(|ui| {
            ui.set_width(width);
            ui.add_space(theme::SPACE_XL * 1.5);
            ui.label(
                egui::RichText::new("What are we making?")
                    .font(theme::font(theme::HEADING_SIZE + 10.0))
                    .color(theme::rgb(ctx.theme.text))
                    .strong(),
            );
            ui.add_space(theme::SPACE_XS);
            widgets::muted(
                ui,
                ctx.theme,
                "Drop this week's footage and cut straight to the good parts: ave indexes it, \
                 captions it, maps the music and drafts every deliverable.",
            );
            ui.add_space(theme::SPACE_LG);
            if drop_zone(ctx, ui, width) && outcome.is_none() {
                outcome = recent::pick_footage(ctx, nights).map(Outcome::NewNight);
            }
            ui.add_space(theme::SPACE_LG);
            prompt(ctx, state, ui);
            ui.add_space(theme::SPACE_XL);
            if let Some(open) = grid(ctx, nights, ui, width) {
                outcome = Some(Outcome::Open(open));
            }
            ui.add_space(theme::SPACE_XL);
        });
    });
    outcome
}

/// The drop zone; returns whether it asked for the folder picker.
fn drop_zone(ctx: &PageCtx<'_>, ui: &mut egui::Ui, width: f32) -> bool {
    let theme = ctx.theme;
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 190.0), egui::Sense::click());
    let lift = widgets::hover_t(ui, response.id, response.hovered() || hovering);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(theme::RADIUS_LG),
        theme::rgb(theme.panel.lerp(theme.accent_soft, lift * 0.6)),
    );
    let border = theme::rgb(theme.border_strong.lerp(theme.accent, lift));
    let r = rect.shrink(1.0);
    let corners = [
        r.left_top(),
        r.right_top(),
        r.right_bottom(),
        r.left_bottom(),
        r.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(
        &corners,
        egui::Stroke::new(1.5, border),
        8.0,
        6.0,
    ));
    let c = rect.center();
    painter.text(
        c + egui::vec2(0.0, -42.0),
        egui::Align2::CENTER_CENTER,
        "\u{2913}",
        theme::font(30.0),
        border,
    );
    painter.text(
        c + egui::vec2(0.0, -6.0),
        egui::Align2::CENTER_CENTER,
        if hovering {
            "Drop to set up the night"
        } else {
            "Drop this week's footage folder"
        },
        theme::font(theme::TITLE_SIZE + 2.0),
        theme::rgb(theme.text),
    );
    painter.text(
        c + egui::vec2(0.0, 18.0),
        egui::Align2::CENTER_CENTER,
        "Recipe, end cards and track come over from last week",
        theme::font(theme::BODY_SIZE - 1.0),
        theme::rgb(theme.text_muted),
    );
    let button = egui::Rect::from_center_size(c + egui::vec2(0.0, 56.0), egui::vec2(200.0, 32.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
    let picked = child
        .vertical_centered(|ui| {
            widgets::button(
                ui,
                theme,
                "Open footage folder\u{2026}",
                widgets::Emphasis::Primary,
            )
            .clicked()
        })
        .inner;
    picked
        || response
            .on_hover_text("Or drag a folder of clips anywhere onto the window")
            .clicked()
}

/// The first-request box and its templates.
fn prompt(ctx: &PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    let theme = ctx.theme;
    egui::Frame::new()
        .fill(theme::rgb(theme.raised))
        .stroke(egui::Stroke::new(1.0, theme::rgb(theme.border)))
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_LG))
        .inner_margin(theme::margin(theme::SPACE_MD))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::TextEdit::multiline(&mut state.prompt)
                    .hint_text(
                        "Describe what to make. It goes to the assistant once the night is set up.",
                    )
                    .desired_rows(2)
                    .desired_width(f32::INFINITY)
                    .frame(egui::Frame::NONE),
            );
            ui.add_space(theme::SPACE_SM);
            ui.horizontal_wrapped(|ui| {
                for (label, text) in TEMPLATES {
                    let chosen = state.prompt == text;
                    let emphasis = if chosen {
                        widgets::Emphasis::Primary
                    } else {
                        widgets::Emphasis::Secondary
                    };
                    if widgets::button(ui, theme, label, emphasis)
                        .on_hover_text(text)
                        .clicked()
                    {
                        state.prompt = text.to_owned();
                    }
                }
                if !state.prompt.trim().is_empty() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new("goes to the chat when a night opens")
                                .small()
                                .color(theme::rgb(theme.text_faint)),
                        );
                    });
                }
            });
        });
}

/// The recent nights as cards; returns the one clicked.
fn grid(
    ctx: &mut PageCtx<'_>,
    nights: &recent::State,
    ui: &mut egui::Ui,
    width: f32,
) -> Option<PathBuf> {
    let theme = ctx.theme;
    ui.horizontal(|ui| {
        widgets::title(ui, theme, "Recent nights");
        if nights.scanning() {
            ui.add(egui::Spinner::new().size(12.0));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, theme, "Open\u{2026}", widgets::Emphasis::Secondary)
                .on_hover_text("Open any night's footage or _ave folder (Ctrl+O)")
                .clicked()
            {
                ctx.actions.push(Action::PickProject);
            }
        });
    });
    ui.add_space(theme::SPACE_SM);
    let list = nights.list().to_vec();
    if list.is_empty() {
        widgets::muted(
            ui,
            theme,
            if nights.scanning() {
                "Looking for nights\u{2026}"
            } else {
                "Nights you open show up here, with their siblings on disk."
            },
        );
        return None;
    }
    let gap = theme::SPACE_MD;
    // As many columns as fit, even with few nights: a lone card keeps its size.
    let (columns, card_w) = widgets::grid_layout(width, 230.0, gap, usize::from(u8::MAX));
    let columns = columns.max(1);
    let card_h = card_w * 9.0 / 16.0 + 58.0;
    let mut clicked = None;
    for row in list.chunks(columns) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for night in row {
                if card(ctx, ui, night, egui::vec2(card_w, card_h)) {
                    clicked = Some(night.work.clone());
                }
            }
        });
        ui.add_space(gap);
    }
    clicked
}

/// One night's card; returns whether it was clicked.
fn card(
    ctx: &mut PageCtx<'_>,
    ui: &mut egui::Ui,
    night: &recent::Summary,
    size: egui::Vec2,
) -> bool {
    let theme = ctx.theme;
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(theme::RADIUS_LG),
        theme::rgb(theme.panel.lerp(theme.hover, lift)),
        egui::Stroke::new(
            1.0,
            theme::rgb(theme.border.lerp(theme.border_strong, lift)),
        ),
        egui::StrokeKind::Inside,
    );
    let thumb = egui::Rect::from_min_size(
        rect.left_top() + egui::vec2(1.0, 1.0),
        egui::vec2(size.x - 2.0, (size.x - 2.0) * 9.0 / 16.0),
    );
    match &night.thumb {
        Some((path, t)) => ctx
            .thumbs
            .paint(ui, thumb, path, *t, 0.5, theme::rgb(theme.raised)),
        None => {
            ui.painter()
                .rect_filled(thumb, egui::CornerRadius::same(4), theme::rgb(theme.raised));
        }
    }
    let x = rect.left() + theme::SPACE_MD;
    let painter = ui.painter();
    painter.text(
        egui::pos2(x, thumb.bottom() + 10.0),
        egui::Align2::LEFT_TOP,
        &night.name,
        theme::font(theme::BODY_SIZE),
        theme::rgb(theme.text),
    );
    let (dot, word) = match night.progress {
        recent::Progress::Delivered => (theme.success, night.progress.label()),
        recent::Progress::Planned | recent::Progress::Captioned => {
            (theme.accent, night.progress.label())
        }
        recent::Progress::Indexed => (theme.warning, night.progress.label()),
        recent::Progress::New => (theme.text_faint, night.progress.label()),
    };
    let y = thumb.bottom() + 38.0;
    painter.circle_filled(egui::pos2(x + 3.5, y), 3.5, theme::rgb(dot));
    let sub = if night.clips > 0 {
        format!("{word} \u{b7} {} clips", night.clips)
    } else {
        format!("{word} \u{b7} no footage yet")
    };
    let sub = if night.delivered.1 > 0 && night.delivered.0 > 0 {
        format!(
            "{sub} \u{b7} {}/{} out",
            night.delivered.0, night.delivered.1
        )
    } else {
        sub
    };
    painter.text(
        egui::pos2(x + 11.0, y),
        egui::Align2::LEFT_CENTER,
        sub,
        theme::font(theme::BODY_SIZE - 2.0),
        theme::rgb(theme.text_muted),
    );
    response
        .on_hover_text(night.work.display().to_string())
        .clicked()
}

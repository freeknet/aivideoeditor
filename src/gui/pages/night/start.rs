// ============================================================================
// Module:       gui::pages::night::start
// Description:  The start page (Home with no night open): the first request
//               in a composer box with prompt templates, a drop zone for
//               this week's footage, and the nights as a list.
//
// Dependencies: egui, rfd; crate::gui::pages::night::{parts, recent}
// ============================================================================

//! The start page, after Selects' home.
//!
//! One centred column, at most [`COLUMN`] wide, on the window colour:
//!
//! - **"What are we making?"** and a composer box like the chat's (the
//!   window colour, a `tone-800` hairline that turns `ring` while focused,
//!   14 px corners). The text waits here until a night is set up (or a
//!   recent one opened); then it goes to the assistant through
//!   `Action::ChatInbox`. The paperclip and the send button both ask for
//!   this week's footage folder.
//! - **Templates** in category rows, as quiet pills that fill the box.
//! - **Drop zone.** Drop this week's footage folder anywhere on the page
//!   (or pick it) and the night is set up from last week's (recipe, end
//!   cards, track) and the get-started wizard opens. While files hover
//!   over the window the whole page says so.
//! - **Nights**: the recent nights and their siblings as 40 px rows (frame,
//!   name, counts, how far it got). Click one to open it.

use crate::gui::ui::hint::Hint as _;
use std::path::PathBuf;

use super::{parts, recent};
use crate::gui::app::Action;
use crate::gui::pages::PageCtx;
use crate::gui::ui::{icons, theme, widgets};

/// The first-request templates: the pill's label and the request it fills.
pub(crate) const TEMPLATES: [(&str, &str); 5] = [
    (
        "Make the reels",
        "Make the reels: draft every deliverable in the recipe from the best moments, \
         beat-synced, no repeated shots or performers, drop on the hero fire shot.",
    ),
    (
        "Find the best fire moments",
        "Find the best fire moments in the footage: list the strongest fire shots with their \
         segment ids, sharpest and best lit first, one per performer.",
    ),
    (
        "Cut a 60 s recap",
        "Cut a 60 s recap in 16:9: open wide on the venue, build through the acts, \
         land the hero fire shot on the drop, end card last.",
    ),
    (
        "Who is in the footage",
        "List everyone in the footage with their three best shots and segment ids, so I \
         can check the people file before the cuts.",
    ),
    (
        "A 15 s story",
        "Cut a 15 s vertical story: three quick fire moments and one crowd reaction, beat-synced, \
         end card last.",
    ),
];

/// The template rows: a category's name and its templates, by index into
/// [`TEMPLATES`].
const CATEGORIES: [(&str, &[usize]); 2] = [("Cut", &[0, 2, 4]), ("Find", &[1, 3])];

/// The column's widest.
const COLUMN: f32 = 720.0;

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
    let view = ui.clip_rect();
    let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
    let mut pick = false;
    parts::centred(ui, COLUMN, |ui| {
        ui.add_space((view.height() * 0.12).clamp(theme::SPACE_XL, 112.0));
        heading(ctx, ui);
        ui.add_space(theme::SPACE_XL);
        pick |= composer(ctx, state, ui);
        ui.add_space(theme::SPACE_MD);
        templates(ctx, state, ui);
        ui.add_space(theme::SPACE_XL + theme::SPACE_SM);
        pick |= drop_zone(ctx, ui, hovering);
        ui.add_space(theme::SPACE_XL + theme::SPACE_SM);
        if let Some(open) = list(ctx, nights, ui) {
            outcome = Some(Outcome::Open(open));
        }
        ui.add_space(theme::SPACE_XL * 2.0);
    });
    if pick && outcome.is_none() {
        outcome = recent::pick_footage(ctx, nights).map(Outcome::NewNight);
    }
    if hovering {
        overlay(ctx, ui, view);
    }
    outcome
}

/// "What are we making?", centred.
fn heading(ctx: &PageCtx<'_>, ui: &mut egui::Ui) {
    // The start page's one large line, as Selects' home title.
    const HERO_SIZE: f32 = 28.0;
    ui.vertical_centered(|ui| {
        ui.label(
            egui::RichText::new("What are we making?")
                .font(theme::semibold(ui.ctx(), HERO_SIZE))
                .color(theme::rgb(ctx.theme.text)),
        );
    });
}

/// The first-request box, as the chat's composer: the text, then a toolbar
/// with the paperclip (pick the footage) and send. Returns whether either
/// asked for the footage folder.
fn composer(ctx: &PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) -> bool {
    let theme = ctx.theme;
    let id = ui.make_persistent_id("home-first-request");
    let focused = ui.memory(|m| m.has_focus(id));
    let stroke = if focused {
        egui::Stroke::new(1.0, theme::ring(theme))
    } else {
        egui::Stroke::new(1.0, theme::rgb(theme.raised))
    };
    let mut pick = false;
    egui::Frame::new()
        .fill(theme::rgb(theme.app))
        .stroke(stroke)
        .corner_radius(egui::CornerRadius::same(theme::RADIUS_XL))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: theme::as_margin(theme::SPACE_LG),
                    right: theme::as_margin(theme::SPACE_LG),
                    top: theme::as_margin(theme::SPACE_LG),
                    bottom: theme::as_margin(theme::SPACE_SM),
                })
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut state.prompt)
                            .id(id)
                            .hint_text(
                                egui::RichText::new(
                                    "Describe what to make, or drop in your footage",
                                )
                                .color(theme::rgb(theme.text_muted)),
                            )
                            .font(theme::font(theme::LARGE_SIZE))
                            .desired_rows(3)
                            .desired_width(f32::INFINITY)
                            .frame(egui::Frame::NONE),
                    )
                    .accessible_name("What are we making?");
                });
            egui::Frame::new()
                .inner_margin(theme::margin(theme::SPACE_SM + 2.0))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        // Selects' composer controls are 32 px.
                        const TOOL: f32 = 32.0;
                        pick |= widgets::icon_toggle_sized(
                            ui,
                            theme,
                            icons::PAPERCLIP,
                            "Open a footage folder",
                            "",
                            false,
                            TOOL,
                        )
                        .clicked();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let ready = !state.prompt.trim().is_empty();
                            let send = ui
                                .add_enabled_ui(ready, |ui| {
                                    widgets::filled_icon_button(
                                        ui,
                                        theme,
                                        widgets::ButtonKind::Secondary,
                                        icons::ARROW_UP,
                                        "Pick the footage; the request goes to the chat once \
                                         the project is set up",
                                        "",
                                        TOOL,
                                    )
                                })
                                .inner;
                            pick |= send.clicked();
                        });
                    });
                });
        });
    pick
}

/// The template rows: a category's name, then its templates as pills.
fn templates(ctx: &PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    // The category names' column.
    const NAME_WIDTH: f32 = 48.0;
    let theme = ctx.theme;
    for (name, items) in CATEGORIES {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing =
                egui::vec2(theme::SPACE_XS + 2.0, theme::SPACE_XS + 2.0);
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(NAME_WIDTH, theme::CONTROL_HEIGHT),
                egui::Sense::hover(),
            );
            ui.painter().text(
                rect.left_center() + egui::vec2(theme::SPACE_XS, 0.0),
                egui::Align2::LEFT_CENTER,
                name,
                theme::medium(ui.ctx(), theme::BODY_SIZE),
                theme::rgb(theme.text_faint),
            );
            for &i in items {
                let Some(&(label, text)) = TEMPLATES.get(i) else {
                    continue;
                };
                let chosen = state.prompt == text;
                if template_pill(ui, theme, label, chosen).hint(text).clicked() {
                    state.prompt = text.to_owned();
                }
            }
        });
        ui.add_space(theme::SPACE_XS);
    }
}

/// A template as Selects' quick-action pill: fully rounded, a `tone-700`
/// hairline, 12 px muted text; under the pointer a `tone-500` hairline,
/// the `tone-800` fill and brighter text; ink 12 % when it is the request
/// in the box.
fn template_pill(
    ui: &mut egui::Ui,
    theme: &crate::theme::Palette,
    label: &str,
    chosen: bool,
) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        theme::font(theme::BODY_SIZE),
        egui::Color32::PLACEHOLDER,
    );
    let size = egui::vec2(
        galley.size().x + theme::SPACE_MD * 2.0,
        theme::CONTROL_HEIGHT,
    );
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, chosen, label));
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    let radius = egui::CornerRadius::same((size.y / 2.0).round() as u8);
    let fill = if chosen {
        theme::selected_fill(theme)
    } else {
        theme::rgb(theme.raised).gamma_multiply(lift)
    };
    // `tone-700`, the accent fill's colour, warming to `tone-500`.
    let border = theme::rgb(
        theme
            .raised
            .lerp(theme.text, 0.12)
            .lerp(theme.text_muted, lift * 0.5),
    );
    let painter = ui.painter();
    painter.rect(
        rect,
        radius,
        fill,
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    let ink = if chosen {
        theme::rgb(theme.text)
    } else {
        widgets::blend(theme.text_muted, theme.text, lift)
    };
    painter.galley(rect.center() - galley.size() / 2.0, galley, ink);
    widgets::focus_ring(ui, theme, &response);
    response
}

/// The drop zone: a dashed, rounded strip with an upload mark, what to
/// drop, and the folder picker. Returns whether it asked for the picker.
fn drop_zone(ctx: &PageCtx<'_>, ui: &mut egui::Ui, hovering: bool) -> bool {
    // The strip's height: Selects' drop card, trimmed to one row.
    const HEIGHT: f32 = 88.0;
    let theme = ctx.theme;
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, HEIGHT), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, "Drop a footage folder")
    });
    let lift = widgets::hover_t(ui, response.id, response.hovered() || hovering);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(theme::RADIUS_XL),
        theme::ink(
            theme,
            (f32::from(theme::INK_03) * (1.0 + lift)).round() as u8,
        ),
    );
    let border = theme::ink(
        theme,
        egui::lerp(
            f32::from(theme::INK_15)..=f32::from(theme::INK_20) * 2.0,
            lift,
        )
        .round() as u8,
    );
    dashed_round_rect(
        painter,
        rect.shrink(0.5),
        f32::from(theme::RADIUS_XL),
        border,
    );
    let disc = egui::Rect::from_center_size(
        egui::pos2(rect.left() + theme::SPACE_XL + 20.0, rect.center().y),
        egui::Vec2::splat(40.0),
    );
    painter.circle_filled(disc.center(), 20.0, theme::ink(theme, theme::INK_08));
    icons::paint(
        painter,
        disc,
        icons::UPLOAD,
        icons::SIZE + 2.0,
        widgets::blend(theme.text_muted, theme.text, lift),
    );
    let x = disc.right() + theme::SPACE_LG;
    painter.text(
        egui::pos2(x, rect.center().y - 9.0),
        egui::Align2::LEFT_CENTER,
        if hovering {
            "Drop to start a project"
        } else {
            "Drop a footage folder"
        },
        theme::medium(ui.ctx(), theme::LARGE_SIZE),
        theme::rgb(theme.text),
    );
    painter.text(
        egui::pos2(x, rect.center().y + 10.0),
        egui::Align2::LEFT_CENTER,
        "Start a project from it: set up, analysed and cut, step by step",
        theme::font(theme::BODY_SIZE),
        theme::rgb(theme.text_muted),
    );
    let label = "Open footage folder\u{2026}";
    let font = theme::semibold(ui.ctx(), theme::BODY_SIZE);
    let text_w = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x;
    let button_w = text_w + icons::SMALL + theme::SPACE_XS + 2.0 + theme::SPACE_MD * 2.0 - 2.0;
    let button = egui::Rect::from_min_size(
        egui::pos2(
            rect.right() - theme::SPACE_XL - button_w,
            rect.center().y - theme::CONTROL_HEIGHT / 2.0,
        ),
        egui::vec2(button_w, theme::CONTROL_HEIGHT),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(button)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let picked = widgets::button_with(
        &mut child,
        theme,
        widgets::ButtonKind::Outline,
        Some(icons::FOLDER_OPEN),
        label,
    )
    .clicked();
    picked
        || widgets::tip(
            response,
            theme,
            "Or drag a folder of clips anywhere onto the window",
            "",
        )
        .clicked()
}

/// A dashed outline round `rect` with `radius` corners.
fn dashed_round_rect(painter: &egui::Painter, rect: egui::Rect, radius: f32, color: egui::Color32) {
    // Points per corner arc.
    const ARC: usize = 6;
    let r = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (egui::pos2(rect.right() - r, rect.top() + r), -90.0_f32),
        (egui::pos2(rect.right() - r, rect.bottom() - r), 0.0),
        (egui::pos2(rect.left() + r, rect.bottom() - r), 90.0),
        (egui::pos2(rect.left() + r, rect.top() + r), 180.0),
    ];
    let mut points = vec![egui::pos2(rect.left() + r, rect.top())];
    for (centre, start) in corners {
        for step in 0..=ARC {
            let angle = (start + 90.0 * step as f32 / ARC as f32).to_radians();
            points.push(centre + r * egui::vec2(angle.cos(), angle.sin()));
        }
    }
    points.push(egui::pos2(rect.left() + r, rect.top()));
    painter.extend(egui::Shape::dashed_line(
        &points,
        egui::Stroke::new(1.0, color),
        6.0,
        5.0,
    ));
}

/// The nights as a list; returns the one clicked.
fn list(ctx: &mut PageCtx<'_>, nights: &recent::State, ui: &mut egui::Ui) -> Option<PathBuf> {
    let theme = ctx.theme;
    let keys = crate::gui::ui::keymap::command("open-night")
        .map(|c| c.hint(ctx.keymap))
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.set_min_height(theme::CONTROL_HEIGHT);
        ui.add_space(theme::SPACE_XS);
        parts::label(ui, theme, "Projects");
        if nights.scanning() {
            ui.add(
                egui::Spinner::new()
                    .size(icons::SMALL - 2.0)
                    .color(theme::rgb(theme.text_muted)),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let open = widgets::button_with(
                ui,
                theme,
                widgets::ButtonKind::Ghost,
                Some(icons::FOLDER_OPEN),
                "Open\u{2026}",
            );
            if widgets::tip(
                open,
                theme,
                "Open any project's footage or _ave folder",
                &keys,
            )
            .clicked()
            {
                ctx.actions.push(Action::PickProject);
            }
        });
    });
    ui.add_space(theme::SPACE_XS);
    let list = nights.list().to_vec();
    if list.is_empty() {
        // The empty state centres in the space it is given.
        const EMPTY_HEIGHT: f32 = 168.0;
        let scanning = nights.scanning();
        let width = ui.available_width();
        ui.allocate_ui(egui::vec2(width, EMPTY_HEIGHT), |ui| {
            ui.set_min_size(egui::vec2(width, EMPTY_HEIGHT));
            widgets::empty_state(
                ui,
                theme,
                if scanning {
                    icons::LOADER
                } else {
                    icons::FOLDER
                },
                if scanning {
                    "Looking for projects"
                } else {
                    "No projects yet"
                },
                if scanning {
                    ""
                } else {
                    "Projects you open show up here, with the ones beside them on disk."
                },
                |_| {},
            );
        });
        return None;
    }
    let mut clicked = None;
    ui.spacing_mut().item_spacing.y = 2.0;
    for night in &list {
        if recent::row(ctx, ui, night, false) {
            clicked = Some(night.work.clone());
        }
    }
    clicked
}

/// While files hover over the window: the page dimmed under a dashed
/// frame that says what dropping does.
fn overlay(ctx: &PageCtx<'_>, ui: &egui::Ui, view: egui::Rect) {
    let theme = ctx.theme;
    let painter = ui.painter().with_clip_rect(view);
    painter.rect_filled(view, 0.0, theme::translucent(theme.app, 217));
    let frame = view.shrink(theme::SPACE_LG);
    dashed_round_rect(
        &painter,
        frame,
        f32::from(theme::RADIUS_XL),
        theme::ring(theme),
    );
    let c = frame.center();
    icons::paint(
        &painter,
        egui::Rect::from_center_size(c - egui::vec2(0.0, 22.0), egui::Vec2::splat(32.0)),
        icons::UPLOAD,
        icons::LARGE + 4.0,
        theme::rgb(theme.text),
    );
    painter.text(
        c + egui::vec2(0.0, 14.0),
        egui::Align2::CENTER_CENTER,
        "Drop to start a project",
        theme::medium(ui.ctx(), theme::LARGE_SIZE),
        theme::rgb(theme.text),
    );
}

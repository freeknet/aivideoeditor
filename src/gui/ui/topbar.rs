// ============================================================================
// Module:       gui::ui::topbar
// Description:  The top bar, laid out as Selects' header: home and the night on
//               the left, a centred search box with its popover, and view mode,
//               export, panel toggles and settings on the right.
//
// Dependencies: egui; super::{keymap, layout, theme, widgets}, crate::gui::app
// ============================================================================

//! The top bar.
//!
//! ```text
//! ⌂ night name        [ ⌕ Search shots, captions, moments ]       View ▾  Export  ▤ ▤ ▤ ▤  ⚙
//!                     ┌ recent searches ───────────────┐
//!                     │ filters you can type (facets)  │
//!                     └────────────────────────────────┘
//! ```
//!
//! The search box is centred on the window, not on the gap between the two
//! sides, so it does not drift as the night's name changes length. Its
//! popover lists recent searches and the facets the shot-log search
//! understands (see `crate::search`), so the query language is discoverable
//! without a manual. The panel toggles draw a miniature of the workspace
//! with their own panel lit, and mirror whether it is open or folded.

use super::layout::{Region, ViewMode};
use super::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use super::{keymap, paint_glyph, widgets};
use crate::gui::app::{Action, App, BrowserTab, Page};
use crate::theme::Palette;
use egui::{Align, Color32, CornerRadius, Layout, Rect, Stroke, Ui, Vec2};

/// The search box's width when there is room for it.
const SEARCH_WIDTH: f32 = 440.0;

/// The least width the search box shrinks to.
const SEARCH_MIN: f32 = 200.0;

/// How many recent searches the popover lists.
const RECENT_SHOWN: usize = 6;

/// The facets the popover offers, with what each one keeps. Every token
/// here parses in `crate::search::Query`.
const FACETS: [(&str, &str); 8] = [
    ("fire>0.3", "light share, relative to the night"),
    ("person:", "who is in it"),
    ("move:pan", "camera movement: pan, static, zoom-in…"),
    ("q>0.7", "quality (focus, exposure)"),
    ("-dark", "leave out a flag: dark, soft, shaky…"),
    ("pick", "your picks (reject, unreviewed, rated)"),
    ("moment:performing", "a caption field"),
    ("like:", "more like a segment"),
];

/// Draws the top bar.
pub fn draw(app: &mut App, ui: &mut Ui, actions: &mut Vec<Action>) {
    let theme = app.theme.clone();
    egui::Panel::top("topbar")
        .exact_size(theme::HEADER_HEIGHT)
        .resizable(false)
        .show_separator_line(true)
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(theme.panel))
                .inner_margin(theme::margin_xy(SPACE_SM, 0.0)),
        )
        .show(ui, |ui| {
            let bar = ui.max_rect();
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_XS;
                left(app, ui, &theme, actions);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_XS;
                    right(app, ui, &theme, actions);
                    let gap = ui.available_rect_before_wrap();
                    search(app, ui, &theme, bar, gap);
                });
            });
        });
}

/// Home and the night's name.
fn left(app: &App, ui: &mut Ui, theme: &Palette, actions: &mut Vec<Action>) {
    let home = widgets::toggle_button(ui, theme, "Home: pipeline and recent nights", app.home);
    paint_glyph(ui, &home, super::glyph::NIGHT);
    if home.response.clicked() {
        actions.push(Action::Go(if app.home {
            Page::Review
        } else {
            Page::Night
        }));
    }
    ui.add_space(SPACE_XS);
    let name = app
        .project
        .as_ref()
        .map_or_else(|| crate::NAME.to_owned(), crate::project::Project::name);
    ui.add(
        egui::Label::new(
            egui::RichText::new(name)
                .font(theme::font(theme::TITLE_SIZE))
                .color(theme::rgb(theme.text)),
        )
        .truncate(),
    );
}

/// Right to left: settings, the panel toggles, export, the view menu.
fn right(app: &mut App, ui: &mut Ui, theme: &Palette, actions: &mut Vec<Action>) {
    let settings = widgets::icon_button(ui, theme, &tip("Settings", "settings", app));
    paint_glyph(ui, &settings, super::glyph::SETTINGS);
    if settings.response.clicked() {
        actions.push(Action::OpenSettings);
    }
    divider(ui, theme);
    let workspace = !app.home && app.project.is_some();
    for region in [
        Region::Timeline,
        Region::Inspector,
        Region::Browser,
        Region::Chat,
    ] {
        let open = workspace && app.layout.is_open(region);
        let verb = if open { "Hide" } else { "Show" };
        let label = tip(
            &format!("{verb} the {}", region.label().to_lowercase()),
            region.toggle_command(),
            app,
        );
        let slot = widgets::toggle_button(ui, theme, &label, open);
        paint_region_glyph(ui.painter(), slot.icon, region, open, slot.ink, theme);
        if slot.response.clicked() {
            actions.push(if workspace {
                Action::TogglePanel(region)
            } else {
                Action::FocusRegion(region)
            });
        }
    }
    divider(ui, theme);
    let emphasis = if app.export_open {
        widgets::Emphasis::Primary
    } else {
        widgets::Emphasis::Secondary
    };
    let export = widgets::button(ui, theme, "Export", emphasis).on_hover_text(tip(
        "Export: renders and hand-off",
        "export",
        app,
    ));
    if export.clicked() {
        actions.push(Action::ToggleExport);
    }
    view_menu(app, ui, theme, actions);
}

/// The view mode menu: the mode's name, opening the four modes.
fn view_menu(app: &App, ui: &mut Ui, theme: &Palette, actions: &mut Vec<Action>) {
    let label = egui::RichText::new(format!("{}  ▾", app.layout.mode.label()))
        .font(theme::font(theme::BODY_SIZE))
        .color(theme::rgb(theme.text));
    ui.menu_button(label, |ui| {
        ui.set_min_width(260.0);
        for mode in ViewMode::ALL {
            let on = app.layout.mode == mode;
            let keys = keymap::command(&format!("view-{}", mode.key()))
                .map(|command| command.hint(app.keymap))
                .unwrap_or_default();
            let clicked = ui
                .horizontal(|ui| {
                    let row = ui.selectable_label(
                        on,
                        egui::RichText::new(mode.label()).color(theme::rgb(theme.text)),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(keys)
                                .font(theme::font(theme::SMALL_SIZE))
                                .color(theme::rgb(theme.text_faint)),
                        );
                    });
                    row.on_hover_text(mode.hint()).clicked()
                })
                .inner;
            if clicked {
                actions.push(Action::SetViewMode(mode));
                ui.close();
            }
        }
        ui.separator();
        widgets::muted(ui, theme, "Choose the mode on screen again to reset it.");
    })
    .response
    .on_hover_text("View mode");
}

/// The search box, centred on the bar and kept inside `gap`, with its
/// popover.
fn search(app: &mut App, ui: &mut Ui, theme: &Palette, bar: Rect, gap: Rect) {
    let width = SEARCH_WIDTH
        .min(gap.width() - SPACE_MD * 2.0)
        .max(SEARCH_MIN.min(gap.width()));
    if width <= 0.0 {
        return;
    }
    let height = theme::CONTROL_HEIGHT;
    let mut left = bar.center().x - width / 2.0;
    left = left.clamp(gap.left(), (gap.right() - width).max(gap.left()));
    let rect = Rect::from_min_size(
        egui::pos2(left, bar.center().y - height / 2.0),
        Vec2::new(width, height),
    );
    let field = egui::TextEdit::singleline(&mut app.search)
        .id(egui::Id::unique("topbar-search"))
        .hint_text("Search shots, captions, moments")
        .font(theme::font(theme::BODY_SIZE))
        .margin(egui::Margin {
            left: theme::as_margin(SPACE_MD * 2.0),
            right: theme::as_margin(SPACE_SM),
            top: theme::as_margin(SPACE_XS),
            bottom: theme::as_margin(SPACE_XS),
        })
        .align(egui::Align2::LEFT_CENTER);
    let response = ui.put(rect, field);
    let glyph = Rect::from_center_size(
        egui::pos2(rect.left() + SPACE_MD, rect.center().y),
        Vec2::splat(theme::BODY_SIZE),
    );
    super::draw_glyph(
        ui.painter(),
        glyph,
        super::glyph::SEARCH,
        theme::rgb(theme.text_faint),
    );
    if app.focus_search {
        response.request_focus();
        app.focus_search = false;
    }
    if response.gained_focus() {
        app.search_open = true;
    }
    if response.changed() {
        app.browser_tab = BrowserTab::Shots;
        app.layout.set_open(Region::Browser, true);
        app.search_open = true;
    }
    let (enter, escape) = ui.input(|i| {
        (
            i.key_pressed(egui::Key::Enter),
            i.key_pressed(egui::Key::Escape),
        )
    });
    if response.lost_focus() && enter {
        let query = app.search.clone();
        app.remember_search(&query);
        app.search_open = false;
    }
    if escape && app.search_open {
        app.search_open = false;
        response.surrender_focus();
    }
    if app.search_open {
        popover(app, ui.ctx(), theme, &response);
    }
}

/// The popover under the search box: recent searches and facet hints.
fn popover(app: &mut App, ctx: &egui::Context, theme: &Palette, field: &egui::Response) {
    let mut chosen: Option<String> = None;
    let mut append: Option<&str> = None;
    let area = egui::Area::new(egui::Id::unique("topbar-search-popover"))
        .order(egui::Order::Foreground)
        .fixed_pos(field.rect.left_bottom() + Vec2::new(0.0, SPACE_XS))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(theme::rgb(theme.raised))
                .stroke(theme::ink_line(theme))
                .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                .inner_margin(theme::margin(SPACE_SM))
                .show(ui, |ui| {
                    ui.set_width(field.rect.width() - SPACE_SM * 2.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    heading(ui, theme, "Recent");
                    if app.recent_searches.is_empty() {
                        widgets::muted(ui, theme, "Searches you keep with Enter show here.");
                    }
                    for query in app.recent_searches.iter().take(RECENT_SHOWN) {
                        if row(ui, theme, query, "").clicked() {
                            chosen = Some(query.clone());
                        }
                    }
                    ui.add_space(SPACE_SM);
                    heading(ui, theme, "Filters");
                    for (token, what) in FACETS {
                        if row(ui, theme, token, what).clicked() {
                            append = Some(token);
                        }
                    }
                    ui.add_space(SPACE_XS);
                    ui.label(
                        egui::RichText::new(
                            "Enter keeps a search · Esc closes · - in front leaves out",
                        )
                        .font(theme::font(theme::SMALL_SIZE))
                        .color(theme::rgb(theme.text_faint)),
                    );
                });
        });
    if let Some(query) = chosen {
        app.search = query.clone();
        app.remember_search(&query);
        app.search_open = false;
        app.browser_tab = BrowserTab::Shots;
        app.layout.set_open(Region::Browser, true);
        return;
    }
    if let Some(token) = append {
        if !app.search.is_empty() && !app.search.ends_with(' ') {
            app.search.push(' ');
        }
        app.search.push_str(token);
        app.focus_search = true;
        return;
    }
    // A press anywhere but the box or the popover closes it.
    let outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer
                .interact_pos()
                .is_some_and(|pos| !area.response.rect.contains(pos) && !field.rect.contains(pos))
    });
    if outside {
        app.search_open = false;
    }
}

/// A popover section heading, in small faint capitals.
fn heading(ui: &mut Ui, theme: &Palette, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .font(theme::font(theme::SMALL_SIZE))
            .color(theme::rgb(theme.text_faint)),
    );
}

/// One popover row: a token in the key colour and what it does, muted.
fn row(ui: &mut Ui, theme: &Palette, token: &str, what: &str) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, theme::CONTROL_HEIGHT - SPACE_XS),
        egui::Sense::click(),
    );
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    let painter = ui.painter();
    if lift > 0.0 {
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS),
            theme::translucent(theme.text, (f32::from(theme::INK_HOVER) * lift) as u8),
        );
    }
    let token_galley = painter.layout_no_wrap(
        token.to_owned(),
        egui::FontId::monospace(theme::BODY_SIZE),
        theme::rgb(if what.is_empty() {
            theme.text
        } else {
            theme.accent
        }),
    );
    let token_width = token_galley.size().x;
    painter.galley(
        egui::pos2(
            rect.left() + SPACE_SM,
            rect.center().y - token_galley.size().y / 2.0,
        ),
        token_galley,
        Color32::PLACEHOLDER,
    );
    if !what.is_empty() {
        painter.text(
            egui::pos2(
                rect.left() + SPACE_SM + token_width.max(128.0) + SPACE_MD,
                rect.center().y,
            ),
            egui::Align2::LEFT_CENTER,
            what,
            theme::font(theme::BODY_SIZE),
            theme::rgb(theme.text_muted),
        );
    }
    response
}

/// A hairline between groups of controls.
fn divider(ui: &mut Ui, theme: &Palette) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(SPACE_SM, theme::CONTROL_HEIGHT),
        egui::Sense::hover(),
    );
    let x = rect.center().x;
    ui.painter().line_segment(
        [
            egui::pos2(x, rect.top() + SPACE_XS),
            egui::pos2(x, rect.bottom() - SPACE_XS),
        ],
        theme::ink_line(theme),
    );
}

/// A tooltip naming the command and its keys under the preset in force.
fn tip(what: &str, command: &str, app: &App) -> String {
    let keys = keymap::command(command)
        .map(|command| command.hint(app.keymap))
        .unwrap_or_default();
    if keys.is_empty() {
        what.to_owned()
    } else {
        format!("{what} ({keys})")
    }
}

/// A miniature of the workspace in `rect` with `region` lit: chat down the
/// left, browser, viewer and inspector across the top, timeline along the
/// foot. Lit in the key colour when open, in the ink when folded.
fn paint_region_glyph(
    painter: &egui::Painter,
    rect: Rect,
    region: Region,
    open: bool,
    ink: Color32,
    theme: &Palette,
) {
    let frame = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), rect.height() * 0.8));
    let at = |x0: f32, y0: f32, x1: f32, y1: f32| {
        Rect::from_min_max(
            egui::pos2(
                egui::lerp(frame.x_range(), x0),
                egui::lerp(frame.y_range(), y0),
            ),
            egui::pos2(
                egui::lerp(frame.x_range(), x1),
                egui::lerp(frame.y_range(), y1),
            ),
        )
    };
    let lit = match region {
        Region::Chat => at(0.0, 0.0, 0.3, 1.0),
        Region::Browser => at(0.3, 0.0, 0.5, 0.55),
        Region::Inspector => at(0.8, 0.0, 1.0, 0.55),
        Region::Timeline => at(0.3, 0.55, 1.0, 1.0),
    };
    let fill = if open {
        theme::rgb(theme.accent)
    } else {
        theme::translucent(theme.text_muted, 90)
    };
    painter.rect_filled(lit.shrink(1.0), CornerRadius::same(1), fill);
    painter.rect_stroke(
        frame,
        CornerRadius::same(2),
        Stroke::new(1.0, ink),
        egui::StrokeKind::Inside,
    );
}

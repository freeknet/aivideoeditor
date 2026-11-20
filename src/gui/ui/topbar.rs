// ============================================================================
// Module:       gui::ui::topbar
// Description:  The top bar, laid out as Selects' header: home and the night on
//               the left, a centred search pill with its popover, and panel
//               toggles, view mode, preferences and export on the right.
//
// Dependencies: egui; super::{keymap, layout, theme, widgets, icons},
//               crate::gui::app
// ============================================================================

//! The top bar.
//!
//! ```text
//! ┌──────────────────────────────────────────── 52 px, #141414, hairline under ┐
//! │[⌂] night name        [⌕ Search shots, captions, moments  Ctrl F]  ▢▢▢▢ [▣][⚙] [⇪]│
//! └─────────────────────────────────────────────────────────────────────────────┘
//!  36 px ghost buttons, 8 px corners    36 tall, ≥ 400 wide, ink 5 %   36 secondary
//! ```
//!
//! Measurements are Selects' (`docs/research/07-selects-visual-chat-project.md`
//! §2): a 52 px bar of the window colour at 20 % over the panels, a 14 px
//! Medium title, a 36 px search pill centred on the window (not on the gap
//! between the two sides, so it does not drift as the title changes
//! length), 36 px icon buttons with an ink-10 % hover, the view-mode menu
//! behind `PanelsLeftBottom`, preferences behind `Settings2`, and Export as a
//! filled secondary square. The four panel toggles are ave's own (Selects
//! folds panels from their headers); they use the same buttons.
//!
//! The search popover lists recent searches and the facets the shot-log
//! search understands (see `crate::search`), so the query language is
//! discoverable without a manual.

use super::layout::{Region, ViewMode};
use super::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use super::{icons, keymap, widgets};
use crate::gui::app::{Action, App, BrowserTab, Page};
use crate::theme::Palette;
use egui::{Align, CornerRadius, Layout, Rect, Ui, Vec2};

/// The search pill's width when there is room for it (Selects' minimum).
const SEARCH_WIDTH: f32 = 400.0;

/// The least width the search pill shrinks to in a narrow window.
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
                .fill(theme::bar_fill(&theme))
                .inner_margin(theme::margin_xy(SPACE_SM, 0.0)),
        )
        .show(ui, |ui| {
            let bar = ui.max_rect();
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_SM;
                left(app, ui, &theme, actions);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    right(app, ui, &theme, actions);
                    let gap = ui.available_rect_before_wrap();
                    search(app, ui, &theme, bar, gap);
                });
            });
        });
}

/// The keys bound to `command` under the preset in force, or nothing.
fn keys(command: &str, app: &App) -> String {
    keymap::command(command)
        .map(|command| command.hint(app.keymap))
        .unwrap_or_default()
}

/// A 36 px top-bar icon button.
fn bar_button(
    ui: &mut Ui,
    theme: &Palette,
    glyph: char,
    label: &str,
    keys: &str,
    on: bool,
) -> egui::Response {
    widgets::icon_toggle_sized(ui, theme, glyph, label, keys, on, theme::BAR_CONTROL)
}

/// Home and the night's name.
fn left(app: &App, ui: &mut Ui, theme: &Palette, actions: &mut Vec<Action>) {
    let home = bar_button(
        ui,
        theme,
        icons::HOME,
        "Home: pipeline and recent nights",
        "",
        app.home,
    );
    if home.clicked() {
        actions.push(Action::Go(if app.home {
            Page::Review
        } else {
            Page::Night
        }));
    }
    let name = app
        .project
        .as_ref()
        .map_or_else(|| crate::NAME.to_owned(), crate::project::Project::name);
    let font = theme::medium(ui.ctx(), theme::LARGE_SIZE);
    ui.add(
        egui::Label::new(
            egui::RichText::new(name)
                .font(font)
                .color(theme::rgb(theme.text)),
        )
        .truncate(),
    );
}

/// Right to left: export, preferences, the view menu, the panel toggles.
fn right(app: &mut App, ui: &mut Ui, theme: &Palette, actions: &mut Vec<Action>) {
    let export = widgets::filled_icon_button(
        ui,
        theme,
        widgets::ButtonKind::Secondary,
        icons::SHARE,
        "Export: renders and hand-off",
        &keys("export", app),
        theme::BAR_CONTROL,
    );
    if export.clicked() {
        actions.push(Action::ToggleExport);
    }
    ui.add_space(SPACE_SM);
    if bar_button(
        ui,
        theme,
        icons::SETTINGS_2,
        "Preferences",
        &keys("settings", app),
        false,
    )
    .clicked()
    {
        actions.push(Action::OpenSettings);
    }
    view_menu(app, ui, theme, actions);
    widgets::bar_divider(ui, theme);
    let workspace = !app.home && app.project.is_some();
    for region in [
        Region::Inspector,
        Region::Timeline,
        Region::Browser,
        Region::Chat,
    ] {
        let open = workspace && app.layout.is_open(region);
        let verb = if open { "Hide" } else { "Show" };
        let glyph = match region {
            Region::Chat => icons::MESSAGE_SQUARE,
            Region::Browser => icons::PANEL_LEFT,
            Region::Inspector => icons::PANEL_RIGHT,
            Region::Timeline => icons::PANEL_BOTTOM,
        };
        let label = format!("{verb} the {}", region.label().to_lowercase());
        if bar_button(
            ui,
            theme,
            glyph,
            &label,
            &keys(region.toggle_command(), app),
            open,
        )
        .clicked()
        {
            actions.push(if workspace {
                Action::TogglePanel(region)
            } else {
                Action::FocusRegion(region)
            });
        }
    }
}

/// The view-mode menu: a 36 px icon button opening the four modes, the
/// current one checked (Selects' "View mode" menu, 200 px wide).
fn view_menu(app: &App, ui: &mut Ui, theme: &Palette, actions: &mut Vec<Action>) {
    let id = ui.scope_id().with("view-menu");
    let open = egui::Popup::is_id_open(ui.ctx(), id);
    let response = bar_button(
        ui,
        theme,
        icons::PANELS_LEFT_BOTTOM,
        &format!("View mode: {}", app.layout.mode.label()),
        "",
        open,
    );
    let _ = egui::Popup::menu(&response)
        .id(id)
        .width(220.0)
        .gap(SPACE_XS)
        .show(|ui| {
            ui.horizontal(|ui| {
                ui.add_space(SPACE_SM);
                widgets::menu_heading(ui, theme, "View mode");
            });
            for mode in ViewMode::ALL {
                let on = app.layout.mode == mode;
                let keys = keys(&format!("view-{}", mode.key()), app);
                let row = widgets::menu_item(ui, theme, None, mode.label(), &keys, on);
                if row.on_hover_text(mode.hint()).clicked() {
                    actions.push(Action::SetViewMode(mode));
                    ui.close();
                }
            }
            widgets::menu_separator(ui, theme);
            if widgets::menu_item(
                ui,
                theme,
                Some(icons::ROTATE_CCW),
                "Reset this view",
                "",
                false,
            )
            .on_hover_text("Choosing the mode on screen again resets it too.")
            .clicked()
            {
                actions.push(Action::SetViewMode(app.layout.mode));
                ui.close();
            }
        });
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
    let height = theme::BAR_CONTROL;
    let mut left = bar.center().x - width / 2.0;
    left = left.clamp(gap.left(), (gap.right() - width).max(gap.left()));
    let rect = Rect::from_min_size(
        egui::pos2(left, bar.center().y - height / 2.0),
        Vec2::new(width, height),
    );
    let response = ui
        .push_id("topbar-search", |ui| {
            widgets::field_frame(
                ui,
                theme,
                rect,
                &mut app.search,
                "Search shots, captions, moments",
                Some(icons::SEARCH),
            )
        })
        .inner;
    // The shortcut, as a chip at the field's right end, while it is empty
    // and unfocused: the hint lives in the field, not in a line under it.
    let shortcut = keys("search", app);
    if app.search.is_empty() && !response.has_focus() && !shortcut.is_empty() {
        let chip = Rect::from_min_max(
            egui::pos2(rect.right() - 96.0, rect.top()),
            egui::pos2(rect.right() - SPACE_XS - 2.0, rect.bottom()),
        );
        let mut slot = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(chip)
                .layout(Layout::right_to_left(Align::Center)),
        );
        widgets::kbd(&mut slot, theme, &shortcut);
    }
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
            widgets::popover_frame(theme)
                .inner_margin(theme::margin(SPACE_XS))
                .show(ui, |ui| {
                    ui.set_width(field.rect.width() - SPACE_XS * 2.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    section(ui, theme, "Recent");
                    if app.recent_searches.is_empty() {
                        let font = theme::font(theme::BODY_SIZE);
                        ui.horizontal(|ui| {
                            ui.add_space(SPACE_SM);
                            ui.label(
                                egui::RichText::new("Searches you keep with Enter show here")
                                    .font(font)
                                    .color(theme::rgb(theme.text_faint)),
                            );
                        });
                        ui.add_space(SPACE_XS);
                    }
                    for query in app.recent_searches.iter().take(RECENT_SHOWN) {
                        if row(ui, theme, icons::HISTORY, query, "").clicked() {
                            chosen = Some(query.clone());
                        }
                    }
                    widgets::menu_separator(ui, theme);
                    section(ui, theme, "Filters");
                    for (token, what) in FACETS {
                        if row(ui, theme, icons::LIST_FILTER, token, what).clicked() {
                            append = Some(token);
                        }
                    }
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

/// A popover section heading, with the menu's own inset.
fn section(ui: &mut Ui, theme: &Palette, text: &str) {
    ui.add_space(SPACE_XS);
    ui.horizontal(|ui| {
        ui.add_space(SPACE_SM);
        widgets::menu_heading(ui, theme, text);
    });
    ui.add_space(SPACE_XS);
}

/// One popover row: an icon, a token in monospace and
/// what it does, muted.
fn row(ui: &mut Ui, theme: &Palette, glyph: char, token: &str, what: &str) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, theme::CONTROL_HEIGHT),
        egui::Sense::click(),
    );
    let lift = widgets::hover_t(ui, response.id, response.hovered());
    let painter = ui.painter();
    if lift > 0.0 {
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS),
            theme::ink(theme, (f32::from(theme::INK_15) * lift).round() as u8),
        );
    }
    let slot = Rect::from_center_size(
        egui::pos2(rect.left() + SPACE_SM + icons::SMALL / 2.0, rect.center().y),
        Vec2::splat(icons::SMALL),
    );
    icons::paint(
        painter,
        slot,
        glyph,
        icons::SMALL,
        theme::rgb(theme.text_faint),
    );
    let token_galley = painter.layout_no_wrap(
        token.to_owned(),
        theme::mono(theme::BODY_SIZE),
        theme::rgb(theme.text),
    );
    let text_left = slot.right() + SPACE_SM;
    let token_width = token_galley.size().x;
    painter.galley(
        egui::pos2(text_left, rect.center().y - token_galley.size().y / 2.0),
        token_galley,
        egui::Color32::PLACEHOLDER,
    );
    if !what.is_empty() {
        painter.text(
            egui::pos2(
                text_left + token_width.max(132.0) + SPACE_MD,
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

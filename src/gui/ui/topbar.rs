// ============================================================================
// Module:       gui::ui::topbar
// Description:  The top bar, laid out as Selects' header: home and the night on
//               the left, a centred search bar with its search view, and panel
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
//! The search view lists recent searches and the facets the shot-log
//! search understands (see `crate::search`), so the query language is
//! discoverable without a manual.

use super::layout::{Region, ViewMode};
use super::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use super::{icons, keymap, widgets};
use crate::gui::app::{Action, App, BrowserTab, Page};
use crate::gui::ui::hint::Hint as _;
use crate::theme::Palette;
use egui::{Align, Layout, Rect, Ui, Vec2};

/// The search pill's width when there is room for it (Selects' minimum).
const SEARCH_WIDTH: f32 = 400.0;

/// The least width the search pill shrinks to in a narrow window.
const SEARCH_MIN: f32 = 200.0;

/// How many recent searches the search view lists.
const RECENT_SHOWN: usize = 6;

/// The facets the search view offers, with what each one keeps. Every token
/// here parses in `crate::search::Query`.
const FACETS: [(&str, &str); 8] = [
    ("fire>0.3", "light share, relative to the project"),
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
        "Home: the project, its pipeline and recent projects",
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
    let open = widgets::menu_is_open(ui.ctx(), id);
    let response = bar_button(
        ui,
        theme,
        icons::PANELS_LEFT_BOTTOM,
        &format!("View mode: {}", app.layout.mode.label()),
        "",
        open,
    );
    // The response is not needed: drawing it is the point.
    let _ = widgets::menu(&response)
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
                if row.hint(mode.hint()).clicked() {
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
            .hint("Choosing the mode on screen again resets it too.")
            .clicked()
            {
                actions.push(Action::SetViewMode(app.layout.mode));
                ui.close();
            }
        });
}

/// The id the top bar's search keeps its state under.
const SEARCH_ID: &str = "topbar-search";

/// The search box, centred on the bar and kept inside `gap`: Material's
/// search bar, 36 px tall, the shortcut as a chip at its end. Focused, it
/// opens Material's docked search view under it, listing the recent
/// searches and then the filter tokens: picking a recent search runs it,
/// picking a filter adds its token to the query and keeps typing.
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
    if app.focus_search {
        egui_material3::search::set_expanded(ui.ctx(), SEARCH_ID, true);
        app.focus_search = false;
    }
    let recent: Vec<String> = app
        .recent_searches
        .iter()
        .take(RECENT_SHOWN)
        .cloned()
        .collect();
    let mut suggestions: Vec<egui_material3::search::Suggestion> = recent
        .iter()
        .map(|query| {
            egui_material3::search::Suggestion::new(query.as_str())
                .icon(widgets::lucide(icons::HISTORY))
        })
        .collect();
    suggestions.extend(FACETS.iter().map(|(token, what)| {
        egui_material3::search::Suggestion::new(*token)
            .supporting_text(*what)
            .icon(widgets::lucide(icons::LIST_FILTER))
    }));
    let before = app.search.clone();
    let shortcut = keys("search", app);
    let mut slot = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let out = egui_material3::search::MaterialSearch::new(SEARCH_ID, &mut app.search)
        .placeholder("Search shots, captions, moments")
        .leading_icon(widgets::lucide(icons::SEARCH))
        .width(width)
        .height(height)
        .suggestions(suggestions)
        .trailing_content(|ui| {
            // The hint lives in the bar, not in a line under it.
            if !shortcut.is_empty() {
                widgets::kbd(ui, theme, &shortcut);
            }
        })
        .show(&mut slot);
    app.search_open = out.expanded;
    if let Some(index) = out.selected {
        if let Some(token) = index
            .checked_sub(recent.len())
            .and_then(|facet| FACETS.get(facet))
            .map(|(token, _)| *token)
        {
            // A filter adds to the query rather than replacing it, and
            // the view stays open to go on typing.
            app.search = before;
            if !app.search.is_empty() && !app.search.ends_with(' ') {
                app.search.push(' ');
            }
            app.search.push_str(token);
            egui_material3::search::set_expanded(ui.ctx(), SEARCH_ID, true);
            app.search_open = true;
        } else {
            let query = app.search.clone();
            app.remember_search(&query);
            app.search_open = false;
        }
        app.browser_tab = BrowserTab::Shots;
        app.layout.set_open(Region::Browser, true);
        return;
    }
    if out.changed {
        app.browser_tab = BrowserTab::Shots;
        app.layout.set_open(Region::Browser, true);
    }
    if let Some(query) = out.submitted {
        app.remember_search(&query);
        app.search_open = false;
    }
}

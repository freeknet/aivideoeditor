// ============================================================================
// Module:       gui::ui::workspace
// Description:  The editing workspace, laid out as Selects': chat full height on
//               the left; browser, viewer and inspector above the timeline;
//               every split a remembered fraction, every side panel foldable
//               to a labelled rail, four view modes.
//
// Dependencies: egui; super::{layout, topbar, chrome, widgets, theme};
//               crate::gui::{app, pages, assistant}
// ============================================================================

//! The workspace layout.
//!
//! One window, laid out the way Selects lays out its editor, because the
//! work moves between finding footage, watching it, talking about it and
//! placing it on the timeline without changing screens:
//!
//! ```text
//! ┌ top bar: home · night · [ search … ] · view ▾ · export · ▤▤▤▤ · ⚙ ─────┐
//! │ chat 22% │ browser 21% │       viewer 58%        │ inspector 21%       │
//! │          │ Shots       │                         │                     │
//! │          │ People      │                         │                     │
//! │          │ Music       │                         │                     │
//! │          ├─────────────┴─────────────────────────┴─────────────────────┤
//! │          │ timeline, 45% of the height                                 │
//! └──────────┴─────────────────────────────────────────────────────────────┘
//! ```
//!
//! **Splits are fractions** of the region they divide (see
//! [`super::layout`]), so the layout keeps its proportions when the window
//! is resized, and a drag updates the fraction, which the config keeps.
//! egui's panels remember sizes in pixels; to make them proportional each
//! panel's id carries its parent's size, so a new parent size starts the
//! panel afresh from its fraction.
//!
//! **Folded panels leave a rail** with their name on it; clicking the rail
//! unfolds the panel. The top bar's toggles and `Ctrl+J/B/I/T` fold and
//! unfold them too.
//!
//! **View modes** rearrange the same panels: Vertical makes the viewer a
//! 9:16 column between browser and inspector, Comment widens the viewer and
//! timeline and folds the browser, Preview folds everything but the viewer.
//!
//! The Home screen (pipeline, recent nights) replaces all of it when no
//! night is open or the home button is pressed; Export docks on the right.
//!
//! Selection is app-wide: whatever is selected in the browser, timeline or
//! inspector is a list of [`Reference`]s. `R` mentions it in the chat;
//! `Shift-R` asks which intent (keep, avoid, replace, more like, feature).

use super::layout::{self, Region, ViewMode};
use super::theme::{self, SPACE_XS};
use super::{icons, widgets};
use crate::gui::app::{Action, App, BrowserTab};
use crate::gui::pages::PageCtx;
use crate::refs::{Modifier, Reference};
use egui::{Align, Layout, Rangef, Ui};

/// Builds a [`PageCtx`] from the app's fields; used once per region.
macro_rules! page_ctx {
    ($app:expr, $actions:expr, $blocked:expr) => {
        PageCtx {
            theme: &$app.theme,
            project: &mut $app.project,
            jobs: &mut $app.jobs,
            thumbs: &mut $app.thumbs,
            player: &mut $app.player,
            config: &mut $app.config,
            refs: &mut $app.refs,
            selection: &mut $app.selection,
            search: &mut $app.search,
            chat_inbox: &mut $app.chat_inbox,
            actions: $actions,
            blocked: $blocked,
            keymap: $app.keymap,
        }
    };
}

/// The width of the docked export panel.
const EXPORT_WIDTH: f32 = 380.0;

/// Which edge a split is attached to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    /// Attached to the left edge; sized by width.
    Left,
    /// Attached to the right edge; sized by width.
    Right,
    /// Attached to the foot; sized by height.
    Bottom,
}

/// Draws everything below the window frame.
pub fn draw(app: &mut App, ui: &mut Ui, actions: &mut Vec<Action>, blocked: bool) {
    super::topbar::draw(app, ui, actions);
    super::chrome::status_bar(app, ui);
    // Home's pipeline queue and Export's render queue advance every frame,
    // whether or not their page is on screen.
    crate::gui::pages::night::tick(&mut app.jobs, &mut app.pages.night);
    crate::gui::pages::deliver::tick(&mut app.jobs, &mut app.pages.deliver, ui.ctx());

    // The guide: where the project stands and the next step, on every
    // screen of an open project but the set-up wizard (which is its own
    // step-by-step).
    let in_wizard = app.home
        && app.pages.night.view == crate::gui::pages::night::View::Wizard
        && app.pages.night.wizard.is_some();
    if app.project.is_some() && !in_wizard {
        egui::Panel::top("guide")
            .resizable(false)
            .show_separator_line(true)
            .frame(
                egui::Frame::new()
                    .fill(theme::bar_fill(&app.theme))
                    .inner_margin(theme::margin_xy(theme::SPACE_SM, SPACE_XS)),
            )
            .show(ui, |ui| {
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::night::guide::draw(&mut ctx, &mut app.pages.night, ui);
            });
    }

    if app.home || app.project.is_none() {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::rgb(app.theme.app)))
            .show(ui, |ui| {
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::night::draw(&mut ctx, &mut app.pages.night, ui);
            });
        return;
    }

    let mode = app.layout.mode;
    let folds = app.layout.folds();
    let mut sizes = app.layout.sizes();
    let egui_ctx = ui.ctx().clone();

    // The chat: full height, leftmost.
    let window = ui.available_rect_before_wrap().width();
    if folds.chat {
        let width = layout::span(sizes.chat, window, layout::MIN_CHAT);
        let dragged = split(
            ui,
            Side::Left,
            ("chat", mode),
            window,
            width,
            limits(layout::MIN_CHAT, window),
            true,
            panel_frame(app, &egui_ctx, "chat"),
            "chat",
            |ui| {
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::assistant::panel(&mut ctx, &mut app.pages.assistant, ui);
            },
        );
        if let Some(width) = dragged {
            sizes.chat = width / window;
        }
    } else {
        rail(app, ui, Region::Chat, actions);
    }

    // The timeline, under browser, viewer and inspector but not the chat.
    let editor = ui.available_rect_before_wrap();
    if folds.timeline {
        let range = Rangef::new(
            layout::MIN_TIMELINE.min(editor.height()),
            (editor.height() * 0.7).max(layout::MIN_TIMELINE.min(editor.height())),
        );
        let height = (sizes.timeline * editor.height()).clamp(range.min, range.max);
        let dragged = split(
            ui,
            Side::Bottom,
            ("timeline", mode),
            editor.height(),
            height,
            range,
            true,
            panel_frame(app, &egui_ctx, "timeline"),
            "timeline",
            |ui| {
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::plan::draw(&mut ctx, &mut app.pages.plan, ui);
            },
        );
        if let Some(height) = dragged {
            sizes.timeline = height / editor.height();
        }
    } else {
        rail(app, ui, Region::Timeline, actions);
    }

    if app.export_open {
        let shown = egui::Panel::right("export")
            .resizable(true)
            .default_size(EXPORT_WIDTH)
            .frame(panel_frame(app, &egui_ctx, "export"))
            .show(ui, |ui| {
                let theme = app.theme.clone();
                let keys = super::keymap::command("export")
                    .map(|command| command.hint(app.keymap))
                    .unwrap_or_default();
                widgets::panel_header(ui, &theme, "Export", |ui| {
                    if widgets::icon_btn(ui, &theme, icons::X, "Close export", &keys).clicked() {
                        app.export_open = false;
                    }
                });
                ui.add_space(SPACE_XS);
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::deliver::draw(&mut ctx, &mut app.pages.deliver, ui);
            });
        claim_focus(&egui_ctx, "export", shown.response.rect);
    }

    // The upper row: inspector right, browser left, viewer between. In the
    // Vertical view the sides are whatever leaves the viewer a 9:16 column.
    let upper = ui.available_rect_before_wrap();
    let vertical = mode == ViewMode::Vertical;
    let side = layout::vertical_sides(upper.width(), upper.height());
    if folds.inspector {
        let width = if vertical {
            side
        } else {
            layout::span(sizes.inspector, editor.width(), layout::MIN_INSPECTOR)
        };
        let dragged = split(
            ui,
            Side::Right,
            ("inspector", mode),
            editor.width(),
            width,
            limits(layout::MIN_INSPECTOR, editor.width()),
            !vertical,
            panel_frame(app, &egui_ctx, "inspector"),
            "inspector",
            |ui| {
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::inspector::draw(&mut ctx, &mut app.pages.inspector, ui);
            },
        );
        if let Some(width) = dragged.filter(|_| !vertical) {
            sizes.inspector = width / editor.width();
        }
    }
    if folds.browser {
        let width = if vertical {
            side
        } else {
            layout::span(sizes.browser, editor.width(), layout::MIN_BROWSER)
        };
        let dragged = split(
            ui,
            Side::Left,
            ("browser", mode),
            editor.width(),
            width,
            limits(layout::MIN_BROWSER, editor.width()),
            !vertical,
            panel_frame(app, &egui_ctx, "browser"),
            "browser",
            |ui| browser(app, ui, actions, blocked),
        );
        if let Some(width) = dragged.filter(|_| !vertical) {
            sizes.browser = width / editor.width();
        }
    }
    let shown = egui::CentralPanel::default()
        .frame(panel_frame(app, &egui_ctx, "viewer"))
        .show(ui, |ui| {
            let panel = ui.max_rect().expand(theme::PANEL_INSET);
            {
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::viewer::draw(&mut ctx, &mut app.pages.viewer, ui);
            }
            // A folded browser or inspector leaves no rail: a glass button
            // floats in the viewer's top corner instead, as in Selects.
            let inset = theme::SPACE_XS + 2.0;
            let theme = app.theme.clone();
            if !folds.browser {
                let keys = command_keys(app, Region::Browser);
                if widgets::glass_button(
                    ui,
                    &theme,
                    panel.left_top() + egui::vec2(inset, inset),
                    icons::PANEL_LEFT,
                    Region::Browser.label(),
                    widgets::IconSide::Leading,
                    &keys,
                )
                .clicked()
                {
                    actions.push(Action::TogglePanel(Region::Browser));
                }
            }
            if !folds.inspector {
                let keys = command_keys(app, Region::Inspector);
                if widgets::glass_button(
                    ui,
                    &theme,
                    panel.right_top() + egui::vec2(-inset, inset),
                    icons::PANEL_RIGHT,
                    Region::Inspector.label(),
                    widgets::IconSide::Trailing,
                    &keys,
                )
                .clicked()
                {
                    actions.push(Action::TogglePanel(Region::Inspector));
                }
            }
        });
    claim_focus(&egui_ctx, "viewer", shown.response.rect);

    if let Some(slot) = app.layout.sizes_mut()
        && sizes.is_valid()
    {
        *slot = sizes;
    }
    reference_popup(app, ui.ctx(), actions);
}

/// The frame every docked panel (and the viewer) is drawn in: the panel
/// surface and the [`theme::PANEL_INSET`]. Panels touch, with one
/// white-10 % separator line between them and no gap. The outline is 1 px,
/// transparent except on the focused panel, where it is `ring`
/// (`#737373`): the only focus cue a panel has, as in Selects. A
/// [`widgets::panel_header`] called first bleeds back across the inset.
fn panel_frame(app: &App, ctx: &egui::Context, key: &str) -> egui::Frame {
    let focused =
        ctx.data(|data| data.get_temp::<egui::Id>(focus_id())) == Some(egui::Id::unique(key));
    let stroke = if focused {
        theme::panel_focus(&app.theme)
    } else {
        egui::Stroke::new(1.0, egui::Color32::TRANSPARENT)
    };
    egui::Frame::new()
        .fill(theme::rgb(app.theme.panel))
        .stroke(stroke)
        .inner_margin(theme::margin(theme::PANEL_INSET - 1.0))
}

/// Where the focused panel's key is kept between frames.
fn focus_id() -> egui::Id {
    egui::Id::unique("workspace-focused-panel")
}

/// Whether the panel `key` (`"timeline"`, `"viewer"`, …) is the focused
/// one: the panel that last received a pointer-down. Panels that paint a
/// selection use it to pick Selects' key colour (focused) or white.
pub(crate) fn panel_focused(ctx: &egui::Context, key: &str) -> bool {
    ctx.data(|data| data.get_temp::<egui::Id>(focus_id())) == Some(egui::Id::unique(key))
}

/// Makes the panel `key` the focused one when a press lands in `rect`
/// (the panel that last received a pointer-down is the focused one).
fn claim_focus(ctx: &egui::Context, key: &str, rect: egui::Rect) {
    let pressed = ctx.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .press_origin()
                .is_some_and(|pos| rect.contains(pos))
    });
    if pressed {
        ctx.data_mut(|data| data.insert_temp(focus_id(), egui::Id::unique(key)));
    }
}

/// The keys that fold or unfold `region`, under the preset in force.
fn command_keys(app: &App, region: Region) -> String {
    super::keymap::command(region.toggle_command())
        .map(|command| command.hint(app.keymap))
        .unwrap_or_default()
}

/// The size range of a side panel `min` wide at least, in a parent `total`.
fn limits(min: f32, total: f32) -> Rangef {
    let max = (total * layout::MAX_SHARE).max(min.min(total));
    Rangef::new(min.min(max), max)
}

/// Shows a resizable panel on `side`, `size` points across, and returns
/// its new size if the user dragged it this frame.
///
/// The id carries `total` (the parent's size), so a resized window starts
/// the panel afresh from `size`: that is what keeps the split a fraction.
#[expect(
    clippy::too_many_arguments,
    reason = "one call per panel; a struct for the arguments would only rename them"
)]
fn split(
    ui: &mut Ui,
    side: Side,
    salt: impl std::hash::Hash + std::fmt::Debug,
    total: f32,
    size: f32,
    range: Rangef,
    resizable: bool,
    frame: egui::Frame,
    key: &str,
    add: impl FnOnce(&mut Ui),
) -> Option<f32> {
    let id = (salt, total.round() as i64);
    let panel = match side {
        Side::Left => egui::Panel::left(id),
        Side::Right => egui::Panel::right(id),
        Side::Bottom => egui::Panel::bottom(id),
    };
    let panel = if resizable {
        panel.resizable(true).default_size(size).size_range(range)
    } else {
        panel.resizable(false).exact_size(size)
    };
    // The region draws into a child fenced to the panel's own rect: egui
    // grows a panel to fit whatever its contents ask for, and a region that
    // fills "all available height" would otherwise ratchet its panel up to
    // the maximum and be taken for a drag.
    let shown = panel.frame(frame).show(ui, |ui| {
        let rect = ui.max_rect();
        let mut fenced = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
        );
        // Clipped to the panel, inset included, so a panel header can
        // bleed across the inset to the panel's edges.
        fenced.shrink_clip_rect(rect.expand(theme::PANEL_INSET));
        add(&mut fenced);
        ui.advance_cursor_after_rect(rect);
    });
    claim_focus(ui.ctx(), key, shown.response.rect);
    let got = match side {
        Side::Left | Side::Right => shown.response.rect.width(),
        Side::Bottom => shown.response.rect.height(),
    };
    ((got - size).abs() > 0.5 && got > 0.0).then_some(got)
}

/// A folded panel's rail: its name, read down the rail (or across, for the
/// timeline); a click unfolds it.
fn rail(app: &App, ui: &mut Ui, region: Region, actions: &mut Vec<Action>) {
    let id = ("rail", region.key());
    let panel = match region {
        Region::Chat | Region::Browser => egui::Panel::left(id),
        Region::Inspector => egui::Panel::right(id),
        Region::Timeline => egui::Panel::bottom(id),
    };
    let keys = super::keymap::command(region.toggle_command())
        .map(|command| command.hint(app.keymap))
        .unwrap_or_default();
    let hint = if keys.is_empty() {
        format!("Show the {}", region.label().to_lowercase())
    } else {
        format!("Show the {} ({keys})", region.label().to_lowercase())
    };
    if region == Region::Chat {
        // Selects' folded chat: a 44 px rail holding one 32 px icon
        // button, 8 px from the top; no label.
        panel
            .exact_size(theme::CHAT_RAIL)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::rgb(app.theme.panel))
                    .inner_margin(egui::Margin {
                        left: 0,
                        right: 0,
                        top: theme::as_margin(theme::SPACE_SM),
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    let open = widgets::icon_toggle_sized(
                        ui,
                        &app.theme,
                        icons::MESSAGE_SQUARE,
                        "Assistant",
                        &keys,
                        false,
                        theme::SPACE_XL + theme::SPACE_SM,
                    );
                    if open.clicked() {
                        actions.push(Action::TogglePanel(region));
                    }
                });
            });
        return;
    }
    let side = match region {
        Region::Timeline => widgets::RailSide::Horizontal,
        Region::Chat | Region::Browser | Region::Inspector => widgets::RailSide::Vertical,
    };
    panel
        .exact_size(layout::RAIL)
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::rgb(app.theme.panel)))
        .show(ui, |ui| {
            if widgets::rail(ui, &app.theme, region.label(), &hint, side).clicked() {
                actions.push(Action::TogglePanel(region));
            }
        });
}

/// The browser: its tab strip with a fold button, then the open tab.
fn browser(app: &mut App, ui: &mut Ui, actions: &mut Vec<Action>, blocked: bool) {
    let theme = app.theme.clone();
    let tabs: Vec<(BrowserTab, &str)> = BrowserTab::ALL
        .into_iter()
        .map(|tab| (tab, tab.label()))
        .collect();
    let keys = super::keymap::command(Region::Browser.toggle_command())
        .map(|command| command.hint(app.keymap))
        .unwrap_or_default();
    let mut current = app.browser_tab;
    widgets::panel_bar(
        ui,
        &theme,
        |ui| {
            widgets::tab_strip(ui, &theme, &mut current, &tabs);
        },
        |ui| {
            let fold = widgets::icon_btn(
                ui,
                &theme,
                icons::PANEL_LEFT_CLOSE,
                "Fold the browser",
                &keys,
            );
            if fold.clicked() {
                actions.push(Action::TogglePanel(Region::Browser));
            }
        },
    );
    app.browser_tab = current;
    ui.add_space(SPACE_XS);
    let mut ctx = page_ctx!(app, actions, blocked);
    match app.browser_tab {
        BrowserTab::Shots => crate::gui::pages::review::draw(&mut ctx, &mut app.pages.review, ui),
        BrowserTab::People => crate::gui::pages::people::draw(&mut ctx, &mut app.pages.people, ui),
        BrowserTab::Music => crate::gui::pages::music::draw(&mut ctx, &mut app.pages.music, ui),
    }
}

/// The "mention in chat as…" chooser (`Shift+R`).
fn reference_popup(app: &mut App, ctx: &egui::Context, actions: &mut Vec<Action>) {
    if !app.ref_popup {
        return;
    }
    let mut open = true;
    let mut chosen = None;
    egui::Window::new("Mention in chat as…")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(format!("{} selected", app.selection.len()));
            for m in Modifier::ALL {
                if ui.button(widgets::mention_label(m)).clicked() {
                    chosen = Some(m);
                }
            }
        });
    if let Some(m) = chosen {
        let picked: Vec<Reference> = app.selection.clone();
        actions.extend(widgets::mention_actions(&picked, m));
        open = false;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    app.ref_popup = open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_side_panel_range_never_inverts() {
        for total in [0.0, 100.0, 400.0, 1800.0] {
            let range = limits(layout::MIN_BROWSER, total);
            assert!(range.min <= range.max, "{total}: {range:?}");
        }
    }
}

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
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets;
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
    let frame = panel_frame(app);

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
            frame,
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
            frame,
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
        egui::Panel::right("export")
            .resizable(true)
            .default_size(EXPORT_WIDTH)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    widgets::title(ui, &app.theme, "Export");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let close = widgets::icon_button(ui, &app.theme, "Close export");
                        super::paint_glyph(ui, &close, super::glyph::CLOSE);
                        if close.response.clicked() {
                            app.export_open = false;
                        }
                    });
                });
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::deliver::draw(&mut ctx, &mut app.pages.deliver, ui);
            });
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
            frame,
            |ui| {
                let mut ctx = page_ctx!(app, actions, blocked);
                crate::gui::pages::inspector::draw(&mut ctx, &mut app.pages.inspector, ui);
            },
        );
        if let Some(width) = dragged.filter(|_| !vertical) {
            sizes.inspector = width / editor.width();
        }
    } else {
        rail(app, ui, Region::Inspector, actions);
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
            frame,
            |ui| browser(app, ui, actions, blocked),
        );
        if let Some(width) = dragged.filter(|_| !vertical) {
            sizes.browser = width / editor.width();
        }
    } else {
        rail(app, ui, Region::Browser, actions);
    }
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(theme::rgb(app.theme.app))
                .inner_margin(theme::margin(SPACE_SM)),
        )
        .show(ui, |ui| {
            let mut ctx = page_ctx!(app, actions, blocked);
            crate::gui::pages::viewer::draw(&mut ctx, &mut app.pages.viewer, ui);
        });

    if let Some(slot) = app.layout.sizes_mut()
        && sizes.is_valid()
    {
        *slot = sizes;
    }
    reference_popup(app, ui.ctx(), actions);
}

/// The frame every docked panel is drawn in: the panel surface, a small
/// inset, and no outline (the separator line is the hairline between).
fn panel_frame(app: &App) -> egui::Frame {
    egui::Frame::new()
        .fill(theme::rgb(app.theme.panel))
        .inner_margin(theme::margin(SPACE_SM))
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
        fenced.shrink_clip_rect(rect);
        add(&mut fenced);
        ui.advance_cursor_after_rect(rect);
    });
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
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for tab in BrowserTab::ALL {
            if widgets::tab(ui, &app.theme, tab.label(), app.browser_tab == tab).clicked() {
                app.browser_tab = tab;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let fold = widgets::icon_button(ui, &app.theme, "Fold the browser (Ctrl+B)");
            let center = fold.icon.center();
            widgets::chevron(ui.painter(), center, -egui::Vec2::X, fold.ink);
            if fold.response.clicked() {
                actions.push(Action::TogglePanel(Region::Browser));
            }
        });
    });
    let line = ui.available_rect_before_wrap();
    ui.painter().hline(
        line.x_range().expand(SPACE_SM),
        line.top(),
        theme::ink_line(&app.theme),
    );
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

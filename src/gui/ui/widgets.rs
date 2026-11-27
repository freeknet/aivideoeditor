// ============================================================================
// Module:       gui::ui::widgets
// Description:  The app's own controls: panel headers, buttons, icon buttons,
//               tabs, segmented controls, badges, key chips, empty states,
//               fields, menus, switches, cards, meters and swatches, all
//               painted from the active palette.
//
// Dependencies: egui; super::{theme, motion, icons}, crate::theme::Palette
// ============================================================================

//! The app's controls: the design system's components.
//!
//! egui's built-in widgets follow the theme through `Visuals` (see
//! [`super::theme`]); the ones here are drawn by hand because they need
//! something `Visuals` cannot say: a hover that fades rather than switches,
//! Selects' button variants, a header that runs edge to edge.
//!
//! `docs/DESIGN.md` says which one to use for what. In short:
//!
//! | Need | Use |
//! |---|---|
//! | the row at the top of a panel | [`panel_header`] / [`panel_bar`] |
//! | an icon-only action | [`icon_btn`] / [`icon_toggle`] |
//! | a text action | [`ghost_button`], [`outline_button`], [`primary_button`], [`key_button`] ([`button_with`] for an icon and a variant) |
//! | views of one panel | [`tab_strip`] |
//! | a mode, an option set | [`segmented`] |
//! | a status, a count | [`pill`] |
//! | a shortcut | [`kbd`], or a tooltip via [`tip`] |
//! | nothing to show | [`empty_state`] |
//! | a field | [`text_field`] |
//! | a menu | [`menu_item`], [`menu_heading`], [`menu_separator`] in `ui.menu_button` |
//!
//! Every colour comes from the [`Palette`] passed in. Every animation goes
//! through [`hover_t`], [`fade`] and [`level`], which are the only places
//! that know whether motion is compiled in.

use super::icons;
use super::theme::{self, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XS};
use crate::color::Rgb;
use crate::gui::ui::hint::Hint as _;
use crate::theme::Palette;
use egui::{Align, Align2, Color32, CornerRadius, Layout, Rect, Response, Sense, Stroke, Ui, Vec2};

/// How far a hover has progressed, 0..=1, for the control `id`.
#[must_use]
pub fn hover_t(ui: &Ui, id: egui::Id, hovered: bool) -> f32 {
    super::motion::toggle(ui.ctx(), id, hovered, super::motion::INSTANT)
}

/// How far a state change has progressed, 0..=1: a selection, a modal.
#[must_use]
pub fn fade(ctx: &egui::Context, id: egui::Id, on: bool) -> f32 {
    super::motion::toggle(ctx, id, on, super::motion::QUICK)
}

/// A value that settles on `target` rather than jumping to it.
#[must_use]
pub fn level(ctx: &egui::Context, id: egui::Id, target: f32) -> f32 {
    super::motion::value(ctx, id, target)
}

/// How far a page keyed `key` has arrived, 0..=1; restarts when `key`
/// changes. See `motion::arrival`.
#[must_use]
pub fn arrival(ctx: &egui::Context, id: egui::Id, key: egui::Id) -> f32 {
    super::motion::arrival(ctx, id, key)
}

/// Blends two palette colours by `t` and converts for painting.
#[must_use]
pub fn blend(rest: Rgb, lifted: Rgb, t: f32) -> Color32 {
    theme::rgb(rest.lerp(lifted, t))
}

/// What a hand-drawn control hands back: its response, where its icon goes,
/// and the colour that icon should be drawn in.
pub struct Slot {
    /// The control's interaction response.
    pub response: Response,
    /// The square reserved for the control's icon.
    pub icon: Rect,
    /// The colour the icon should take, matching the control's label.
    pub ink: Color32,
}

/// A sidebar destination. The caller paints the icon into [`Slot::icon`].
pub fn nav_item(ui: &mut Ui, theme: &Palette, label: &str, active: bool, compact: bool) -> Slot {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, theme::CONTROL_HEIGHT), Sense::click());
    let response = if compact {
        response.hint(label)
    } else {
        response
    };
    let lift = hover_t(ui, response.id, response.hovered());
    let selected = fade(ui.ctx(), response.id.with("active"), active);

    let fill = theme
        .panel
        .lerp(theme.hover, lift * 0.8)
        .lerp(theme.selection, selected);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), theme::rgb(fill));
    if selected > 0.0 {
        let bar = Rect::from_min_size(
            rect.left_top() + Vec2::new(0.0, SPACE_SM),
            Vec2::new(theme::SELECTION_BAR, (rect.height() - SPACE_MD) * selected),
        );
        painter.rect_filled(bar, CornerRadius::same(2), theme::rgb(theme.accent));
    }
    let ink = blend(theme.text_muted, theme.text, lift.max(selected));
    let icon = Rect::from_center_size(
        rect.left_center() + Vec2::new(SPACE_MD + super::icon_size() / 2.0, 0.0),
        Vec2::splat(super::icon_size()),
    );
    if !compact {
        painter.text(
            icon.right_center() + Vec2::new(SPACE_MD, 0.0),
            Align2::LEFT_CENTER,
            label,
            theme::font(theme::BODY_SIZE),
            ink,
        );
    }
    focus_ring(ui, theme, &response);
    Slot {
        response,
        icon,
        ink,
    }
}

/// A square, borderless button for the header. The caller paints the icon
/// into [`Slot::icon`]; `label` is its tooltip and accessible name.
pub fn icon_button(ui: &mut Ui, theme: &Palette, label: &str) -> Slot {
    toggle_button(ui, theme, label, false)
}

/// A square, borderless header button that shows whether what it toggles
/// is on: an ink fill and full-strength ink when on, muted when off. The
/// caller paints the icon into [`Slot::icon`].
pub fn toggle_button(ui: &mut Ui, theme: &Palette, label: &str, on: bool) -> Slot {
    let size = Vec2::splat(theme::CONTROL_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let response = response.hint(label);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, label));
    let lift = hover_t(ui, response.id, response.hovered());
    paint_ghost(ui, theme, rect, on, lift, theme::RADIUS_MD);
    focus_ring(ui, theme, &response);
    let rest = if on { theme.text } else { theme.text_muted };
    Slot {
        icon: rect.shrink((theme::CONTROL_HEIGHT - icons::SIZE) / 2.0),
        ink: blend(rest, theme.text, lift),
        response,
    }
}

/// A header control's fill: the selected fill (ink 12 %) when on, fading
/// in to `hover` (an ink alpha) under the pointer.
fn paint_ghost(ui: &Ui, theme: &Palette, rect: Rect, on: bool, lift: f32, radius: u8) {
    paint_fill(ui, theme, rect, on, lift, radius, theme::INK_12);
}

/// [`paint_ghost`] with the hover alpha given.
fn paint_fill(ui: &Ui, theme: &Palette, rect: Rect, on: bool, lift: f32, radius: u8, hover: u8) {
    let alpha = if on {
        f32::from(theme::INK_12).max(f32::from(hover) * lift)
    } else {
        f32::from(hover) * lift
    };
    if alpha > 0.0 {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(radius),
            theme::ink(theme, alpha.round() as u8),
        );
    }
}

/// A tab pill, as the titles in Selects' panel headers are: 28 px, 12 px
/// Medium, 10 px padding, 6 px corners; the open one filled with ink at
/// 12 % in the foreground colour, the others muted with an ink-8 % hover.
/// See [`tab_strip`] for the whole strip.
pub fn tab(ui: &mut Ui, theme: &Palette, label: &str, open: bool) -> Response {
    let font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let size = Vec2::new(
        galley.size().x + (SPACE_SM + 2.0) * 2.0,
        theme::CONTROL_HEIGHT,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, open, label));
    let lift = hover_t(ui, response.id, response.hovered());
    paint_fill(ui, theme, rect, open, lift, theme::RADIUS, theme::INK_08);
    let ink = if open {
        theme::rgb(theme.text)
    } else {
        blend(theme.text_muted, theme.text, lift)
    };
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, ink);
    focus_ring(ui, theme, &response);
    super::hint::auto(&response, label);
    response
}

/// Which way a rail runs, and so which way its label reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailSide {
    /// A tall rail at the left or right edge; the label reads downwards.
    Vertical,
    /// A short rail along the foot; the label reads across.
    Horizontal,
}

/// A folded panel's rail: the panel's name, a chevron, and a hover fill.
/// The whole rail is the button that unfolds it.
pub fn rail(ui: &mut Ui, theme: &Palette, label: &str, hint: &str, side: RailSide) -> Response {
    let rect = ui.max_rect();
    let response = ui
        .interact(rect, ui.scope_id().with(("rail", label)), Sense::click())
        .hint(hint);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let lift = hover_t(ui, response.id, response.hovered());
    let painter = ui.painter();
    if lift > 0.0 {
        painter.rect_filled(
            rect,
            CornerRadius::ZERO,
            theme::translucent(theme.text, (f32::from(theme::INK_HOVER) * lift) as u8),
        );
    }
    let ink = blend(theme.text_muted, theme.text, lift);
    let galley = painter.layout_no_wrap(
        label.to_owned(),
        theme::medium(ui.ctx(), theme::BODY_SIZE),
        ink,
    );
    match side {
        RailSide::Vertical => {
            // Rotated a quarter turn clockwise about its top-left corner,
            // the galley's height runs leftwards: start half a line right
            // of the centre so it lands centred.
            let x = rect.center().x + galley.size().y / 2.0;
            let glyph = Rect::from_center_size(
                egui::pos2(
                    rect.center().x,
                    rect.top() + theme::PANEL_HEADER_HEIGHT / 2.0,
                ),
                Vec2::splat(icons::SMALL),
            );
            icons::paint(painter, glyph, icons::CHEVRON_RIGHT, icons::SMALL, ink);
            let at = egui::pos2(x, rect.top() + theme::PANEL_HEADER_HEIGHT);
            painter.add(
                egui::epaint::TextShape::new(at, galley, ink)
                    .with_angle(std::f32::consts::FRAC_PI_2),
            );
        }
        RailSide::Horizontal => {
            let glyph = Rect::from_center_size(
                egui::pos2(rect.left() + SPACE_MD + SPACE_XS, rect.center().y),
                Vec2::splat(icons::SMALL),
            );
            icons::paint(painter, glyph, icons::CHEVRON_UP, icons::SMALL, ink);
            let at = egui::pos2(
                glyph.right() + SPACE_SM,
                rect.center().y - galley.size().y / 2.0,
            );
            painter.galley(at, galley, ink);
        }
    }
    focus_ring(ui, theme, &response);
    response
}

/// A small open chevron at `center`, pointing along `towards` (a unit
/// vector), drawn as two strokes so it needs no glyph from any font.
pub fn chevron(painter: &egui::Painter, center: egui::Pos2, towards: Vec2, color: Color32) {
    let size = 3.5;
    let across = towards.rot90();
    let tip = center + towards * size * 0.5;
    let back = center - towards * size * 0.5;
    let stroke = Stroke::new(1.5, color);
    painter.line_segment([back + across * size, tip], stroke);
    painter.line_segment([back - across * size, tip], stroke);
}

/// The "Mention in chat ▸ about / keep / avoid / replace / more like /
/// feature" submenu, for any region's context menu. Choosing an intent
/// pushes [`Action::AddRef`](crate::gui::app::Action::AddRef) with that
/// modifier, which stages the reference and opens the chat.
pub fn mention_menu(
    ui: &mut Ui,
    actions: &mut Vec<crate::gui::app::Action>,
    reference: &crate::refs::Reference,
) {
    mention_menu_many(ui, actions, std::slice::from_ref(reference));
}

/// [`mention_menu`] for several references at once (a multi-selection),
/// staged in the order given: pass timeline items in timeline order.
pub fn mention_menu_many(
    ui: &mut Ui,
    actions: &mut Vec<crate::gui::app::Action>,
    references: &[crate::refs::Reference],
) {
    if references.is_empty() {
        return;
    }
    submenu(ui, "Mention in chat", |ui| {
        for intent in crate::refs::Modifier::ALL {
            if ui.button(mention_label(intent)).clicked() {
                actions.extend(mention_actions(references, intent));
                ui.close();
            }
        }
    });
}

/// How an intent reads in the mention menu: its label, then its prefix.
#[must_use]
pub fn mention_label(intent: crate::refs::Modifier) -> String {
    let prefix = intent.prefix();
    if prefix.is_empty() {
        intent.label().to_owned()
    } else {
        format!("{}   {prefix}", intent.label())
    }
}

/// The actions that mention `references` with `intent`.
#[must_use]
pub fn mention_actions(
    references: &[crate::refs::Reference],
    intent: crate::refs::Modifier,
) -> Vec<crate::gui::app::Action> {
    references
        .iter()
        .map(|reference| {
            crate::gui::app::Action::AddRef(crate::refs::Reference {
                modifier: intent,
                ..reference.clone()
            })
        })
        .collect()
}

/// The emphasis a button carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emphasis {
    /// The accent-filled action a view is for.
    Primary,
    /// Everything else: a quiet surface button.
    Secondary,
}

/// A text button: [`Emphasis::Primary`] is the key (lime) button the view
/// is for, [`Emphasis::Secondary`] an outline button. Prefer the named
/// variants ([`key_button`], [`outline_button`], …) in new code.
pub fn button(ui: &mut Ui, theme: &Palette, label: &str, emphasis: Emphasis) -> Response {
    let kind = match emphasis {
        Emphasis::Primary => ButtonKind::Key,
        Emphasis::Secondary => ButtonKind::Outline,
    };
    button_with(ui, theme, kind, None, label)
}

/// An on/off switch with its label to the left. Returns the response; the
/// value is flipped in place when clicked.
pub fn switch(ui: &mut Ui, theme: &Palette, on: &mut bool, label: &str) -> Response {
    // Material's switch (52 x 32: a primary track and an on-primary thumb
    // when on, an outlined surface track when off) at the right end of a
    // full-width row; a click on the label toggles it too.
    const SWITCH: Vec2 = Vec2::new(52.0, 32.0);
    let width = ui.available_width();
    let height = theme::CONTROL_HEIGHT.max(SWITCH.y);
    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    ui.painter().text(
        rect.left_center(),
        Align2::LEFT_CENTER,
        label,
        theme::font(theme::BODY_SIZE),
        theme::rgb(theme.text),
    );
    let slot = Rect::from_center_size(rect.right_center() - Vec2::new(SWITCH.x / 2.0, 0.0), SWITCH);
    let enabled = ui.is_enabled();
    let switch = ui.put(
        slot,
        egui_material3::MaterialSwitch::new(on).enabled(enabled),
    );
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if switch.changed() {
        response.mark_changed();
    }
    let value = *on;
    let info = || egui::WidgetInfo::selected(egui::Role::Switch, enabled, value, label);
    response.widget_info(info);
    // The library's switch takes its accessible name from its own text,
    // which would also draw beside it; the row's label names it instead.
    switch.widget_info(info);
    let response = response | switch;
    super::hint::auto(&response, label);
    response
}

/// A card: Material's outlined card, clickable, with its state layer under
/// the pointer and its selected look when `selected`. Click anywhere on it
/// to select it.
pub fn card(
    ui: &mut Ui,
    _theme: &Palette,
    id: egui::Id,
    selected: bool,
    add_contents: impl FnOnce(&mut Ui),
) -> Response {
    ui.push_id(id, |ui| {
        ui.add(
            egui_material3::MaterialCard2::outlined()
                .clickable(true)
                .selected(selected)
                .content(add_contents),
        )
    })
    .inner
}

/// Lays `count` cards out in as many equal columns of at least `minimum`
/// width as fit, wrapping onto new rows.
pub fn card_grid(ui: &mut Ui, minimum: f32, count: usize, mut card: impl FnMut(&mut Ui, usize)) {
    let (columns, width) = grid_layout(ui.available_width(), minimum, SPACE_MD, count);
    if columns == 0 {
        return;
    }
    let mut index = 0;
    while index < count {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_MD;
            for _ in 0..columns {
                if index >= count {
                    break;
                }
                let column = egui::Layout::top_down(egui::Align::Min);
                ui.allocate_ui_with_layout(Vec2::new(width, 0.0), column, |ui| {
                    ui.set_width(width);
                    card(ui, index);
                });
                index += 1;
            }
        });
        ui.add_space(SPACE_XS);
    }
}

/// How many columns of at least `minimum` fit in `available` with `gap`
/// between them, and how wide each one is. Zero columns when nothing fits
/// or there is nothing to lay out.
#[must_use]
pub fn grid_layout(available: f32, minimum: f32, gap: f32, count: usize) -> (usize, f32) {
    if count == 0 || !available.is_finite() || available <= 0.0 || minimum <= 0.0 {
        return (0, 0.0);
    }
    let fit = ((available + gap) / (minimum + gap)).floor().max(1.0) as usize;
    let columns = fit.min(count);
    let width = (available - gap * (columns as f32 - 1.0)) / columns as f32;
    (columns, width.max(0.0))
}

/// A busy mark `size` across in `color`: Material's indeterminate circular
/// progress indicator, the stroke scaled to the size so a 10 px one in a
/// list row still reads. Add it with `ui.add` or place it with `ui.put`.
pub fn spinner(size: f32, color: Color32) -> egui_material3::MaterialProgress {
    egui_material3::MaterialProgress::circular()
        .indeterminate(true)
        .size(Vec2::splat(size))
        .stroke_width((size / 7.0).max(1.5))
        .active_color(color)
}

/// A dropdown of `options` with `selected` the chosen index: Material's
/// compact (32 px) outlined select, its menu scrolling past 300 px. The
/// response is marked changed when a new option is picked.
pub fn select<S: AsRef<str>>(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    selected: &mut usize,
    options: &[S],
    width: f32,
) -> Response {
    let mut pick = Some(*selected);
    let mut field = egui_material3::MaterialSelect::new(&mut pick)
        .id_salt(egui::IdSalt::new(id_salt))
        .variant(egui_material3::SelectVariant::Outlined)
        .compact(true)
        .width(width)
        .menu_max_height(300.0);
    for (index, option) in options.iter().enumerate() {
        field = field.option(index, option.as_ref());
    }
    let mut response = ui.add(field);
    if let Some(index) = pick
        && index != *selected
        && index < options.len()
    {
        *selected = index;
        response.mark_changed();
    }
    response
}

/// A horizontal meter, `fraction` full, that settles rather than jumps.
pub fn meter(ui: &mut Ui, theme: &Palette, id: egui::Id, fraction: f32, fill: Rgb) -> Response {
    // Material's linear progress indicator (active track, gap, inactive
    // track, stop dot), fed the settled value so it still glides.
    let width = ui.available_width();
    let shown = level(ui.ctx(), id, fraction.clamp(0.0, 1.0));
    ui.add(
        egui_material3::MaterialProgress::linear()
            .value(shown)
            .max(1.0)
            .width(width)
            .height(6.0)
            .active_color(theme::rgb(fill))
            .track_color(theme::rgb(theme.hover)),
    )
}

/// A colour swatch with a hairline, so a swatch matching its surface still
/// shows where it is.
pub fn swatch(ui: &mut Ui, theme: &Palette, color: Rgb, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(theme::RADIUS),
        theme::rgb(color),
        Stroke::new(1.0, theme::rgb(theme.border)),
        egui::StrokeKind::Inside,
    );
    response
}

/// The first letter of `label` in a rounded square: the stand-in for an
/// icon in a build without the icon set.
pub fn monogram(painter: &egui::Painter, rect: Rect, label: &str, color: Color32) {
    let letter: String = label.chars().take(1).collect();
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        letter,
        theme::font(rect.height() * 0.8),
        color,
    );
}

/// A label in the theme's title size, in Mona Sans Medium.
pub fn title(ui: &mut Ui, theme: &Palette, text: &str) -> Response {
    let font = theme::medium(ui.ctx(), theme::TITLE_SIZE);
    ui.label(
        egui::RichText::new(text)
            .font(font)
            .color(theme::rgb(theme.text)),
    )
}

/// Body text in the secondary colour, wrapping.
pub fn muted(ui: &mut Ui, theme: &Palette, text: &str) -> Response {
    ui.add(egui::Label::new(egui::RichText::new(text).color(theme::rgb(theme.text_muted))).wrap())
}

/// The keyboard focus ring, drawn only while the control has focus:
/// Selects' `ring` (neutral-500), a hairline just outside the control.
pub fn focus_ring(ui: &Ui, theme: &Palette, response: &Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(1.0),
            CornerRadius::same(theme::RADIUS_MD + 1),
            Stroke::new(1.5, theme::ring(theme)),
            egui::StrokeKind::Outside,
        );
    }
}

// ---------------------------------------------------------------------------
// The design system's components (see the module docs and docs/DESIGN.md).
// ---------------------------------------------------------------------------

/// The row at the top of a panel: `title` on the left, `right` laid out
/// right to left at the other end, [`theme::PANEL_HEADER_HEIGHT`] tall with
/// a hairline under it. Call it first in the panel; see [`panel_bar`].
///
/// Selects has no bold panel titles: the title is the panel's one open tab,
/// a [`tab`] pill (12 px Medium on ink 12 %). A panel with several views
/// uses [`panel_bar`] with a [`tab_strip`] instead.
pub fn panel_header<R>(
    ui: &mut Ui,
    theme: &Palette,
    title: &str,
    right: impl FnOnce(&mut Ui) -> R,
) -> R {
    panel_bar(
        ui,
        theme,
        |ui| {
            title_pill(ui, theme, title);
        },
        right,
    )
}

/// A panel's title as an open, inert tab pill.
fn title_pill(ui: &mut Ui, theme: &Palette, title: &str) -> Response {
    let font = theme::medium(ui.ctx(), theme::PANEL_TITLE_SIZE);
    let galley = ui
        .painter()
        .layout_no_wrap(title.to_owned(), font, theme::rgb(theme.text));
    let size = Vec2::new(
        galley.size().x + (SPACE_SM + 2.0) * 2.0,
        theme::CONTROL_HEIGHT,
    );
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(theme::RADIUS),
        theme::selected_fill(theme),
    );
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::rgb(theme.text),
    );
    response
}

/// A panel header with any content on the left (a [`tab_strip`], a menu)
/// and `right` at the other end, laid out right to left.
///
/// Every docked panel's frame insets its contents by
/// [`theme::PANEL_INSET`]; called first in a panel, the bar bleeds back
/// across that inset (as far as the clip allows), so it sits flush with
/// the panel's top and its hairline runs edge to edge. Every panel's header
/// is therefore the same height and lines up across the workspace.
pub fn panel_bar<R>(
    ui: &mut Ui,
    theme: &Palette,
    left: impl FnOnce(&mut Ui),
    right: impl FnOnce(&mut Ui) -> R,
) -> R {
    let available = ui.available_rect_before_wrap();
    let clip = ui.clip_rect();
    let bleed = |gap: f32| gap.clamp(0.0, theme::PANEL_INSET);
    let top = available.top() - bleed(available.top() - clip.top());
    let outer = Rect::from_min_max(
        egui::pos2(
            available.left() - bleed(available.left() - clip.left()),
            top,
        ),
        egui::pos2(
            available.right() + bleed(clip.right() - available.right()),
            top + theme::PANEL_HEADER_HEIGHT,
        ),
    );
    let (_, _) = ui.allocate_exact_size(
        Vec2::new(
            available.width(),
            (outer.bottom() - available.top()).max(0.0),
        ),
        Sense::hover(),
    );
    ui.painter().hline(
        outer.x_range(),
        outer.bottom() - 0.5,
        theme::ink_line(theme),
    );
    // Selects' header: 6 px in from the panel's edges, 2 px between items.
    let pad = (SPACE_XS + 2.0).min(outer.width() / 2.0);
    let row = Rect::from_min_max(
        egui::pos2(outer.left() + pad, outer.top()),
        egui::pos2(outer.right() - pad, outer.bottom() - 1.0),
    );
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(Layout::left_to_right(Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 2.0;
    left(&mut bar);
    bar.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        right(ui)
    })
    .inner
}

/// The button variants: Selects' `buttonVariants`, the ones this app uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// No fill until hovered: toolbar actions, secondary actions in a
    /// header.
    Ghost,
    /// A hairline outline on a faint ink fill: the default action.
    Outline,
    /// A filled muted surface: an action that should read as a button
    /// without competing.
    Secondary,
    /// Near-white with dark text: the one main action of a dialog or bar
    /// (Export).
    Primary,
    /// The lime key colour with dark text: run, apply, send — the thing the
    /// view is for.
    Key,
    /// Red: delete, discard.
    Destructive,
}

/// A button: `kind`, an optional leading Lucide icon, and a label, drawn
/// by egui-material3's `MaterialButton` (Ghost is a text button, Outline
/// outlined, Secondary filled tonal, Primary and Key filled, Destructive
/// filled in the error colour). 28 px tall in panels, fully rounded, label
/// in Material's label-medium. An empty `label` with an icon makes a
/// square icon button.
pub fn button_with(
    ui: &mut Ui,
    theme: &Palette,
    kind: ButtonKind,
    icon: Option<char>,
    label: &str,
) -> Response {
    button_sized(ui, theme, kind, icon, label, theme::CONTROL_HEIGHT)
}

/// [`button_with`] at a given `height`: [`theme::BAR_CONTROL`] (36, the
/// top bar and dialogs), 32 (`sm`) or [`theme::CONTROL_HEIGHT`] (28,
/// `xs`, the default in panels).
pub fn button_sized(
    ui: &mut Ui,
    theme: &Palette,
    kind: ButtonKind,
    icon: Option<char>,
    label: &str,
    height: f32,
) -> Response {
    use egui_material3::MaterialButton;
    // The label is one layout job, ave's Lucide glyph and then the text in
    // Material's label-medium style, so the library's button keeps the
    // app's icons (it only knows Material Symbols by name) and lays the
    // pair out as one centred line in its own content colour.
    let mut job = egui::text::LayoutJob::default();
    let colour = Color32::PLACEHOLDER;
    if let Some(glyph) = icon {
        let size = if label.is_empty() {
            icons::SIZE
        } else {
            icons::SMALL
        };
        job.append(
            &glyph.to_string(),
            0.0,
            egui::TextFormat::simple(
                crate::gui::font::weight(ui.ctx(), crate::gui::font::ICONS, size),
                colour,
            ),
        );
    }
    if !label.is_empty() {
        let style = egui_material3::tokens::TypeRole::LabelMedium.style();
        let leading = if icon.is_some() { SPACE_SM } else { 0.0 };
        job.append(
            label,
            leading,
            egui::TextFormat {
                font_id: style.font_id(ui.ctx()),
                extra_letter_spacing: style.tracking,
                color: colour,
                ..egui::TextFormat::default()
            },
        );
    }
    let content_width = ui.painter().layout_job(job.clone()).size().x;
    let width = if label.is_empty() {
        height
    } else {
        content_width + SPACE_LG * 2.0
    };
    let button = match kind {
        ButtonKind::Ghost => MaterialButton::text(job),
        ButtonKind::Outline => MaterialButton::outlined(job),
        ButtonKind::Secondary => MaterialButton::filled_tonal(job),
        ButtonKind::Primary | ButtonKind::Key => MaterialButton::filled(job),
        ButtonKind::Destructive => MaterialButton::filled(job)
            .fill(theme::rgb(theme.danger))
            .text_color(theme::rgb(theme.text_on_danger)),
    };
    // `small` drops Material's 40 px minimum so the button fits the dense
    // rows (28 px in a panel header); the width keeps M3's 16 px sides.
    let response = ui.add(button.small().min_size(Vec2::new(width, height)));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    super::hint::auto(&response, label);
    response
}

/// A square icon button in a filled variant (`kind`), `size` across, named
/// `label` for screen readers and its tooltip (with `keys`): the top bar's
/// Export, the composer's send. A ghost icon button is [`icon_btn`].
pub fn filled_icon_button(
    ui: &mut Ui,
    theme: &Palette,
    kind: ButtonKind,
    glyph: char,
    label: &str,
    keys: &str,
    size: f32,
) -> Response {
    let response = button_sized(ui, theme, kind, Some(glyph), "", size);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    tip(response, theme, label, keys)
}

/// A ghost button: no fill until hovered.
pub fn ghost_button(ui: &mut Ui, theme: &Palette, label: &str) -> Response {
    button_with(ui, theme, ButtonKind::Ghost, None, label)
}

/// An outline button: the default action.
pub fn outline_button(ui: &mut Ui, theme: &Palette, label: &str) -> Response {
    button_with(ui, theme, ButtonKind::Outline, None, label)
}

/// A primary button: near-white, the one main action of a bar or dialog.
pub fn primary_button(ui: &mut Ui, theme: &Palette, label: &str) -> Response {
    button_with(ui, theme, ButtonKind::Primary, None, label)
}

/// A key button: lime with dark text, the action the view is for.
pub fn key_button(ui: &mut Ui, theme: &Palette, label: &str) -> Response {
    button_with(ui, theme, ButtonKind::Key, None, label)
}

/// A ghost icon button: 28 px square, a tooltip naming it and its
/// shortcut (`keys` empty for none). Every icon-only control in the app is
/// one of these or an [`icon_toggle`].
pub fn icon_btn(ui: &mut Ui, theme: &Palette, glyph: char, label: &str, keys: &str) -> Response {
    material_icon_button(ui, theme, glyph, label, keys, None, theme::CONTROL_HEIGHT)
}

/// An icon button that shows whether what it toggles is on: Material's
/// standard toggle icon button, the icon in the primary colour when on.
/// It reports the toggled state to screen readers; the caller flips its
/// own state on `clicked`.
pub fn icon_toggle(
    ui: &mut Ui,
    theme: &Palette,
    glyph: char,
    label: &str,
    keys: &str,
    on: bool,
) -> Response {
    icon_toggle_sized(ui, theme, glyph, label, keys, on, theme::CONTROL_HEIGHT)
}

/// [`icon_toggle`] at a given square `size`: [`theme::BAR_CONTROL`] (36)
/// in the top bar and the transport, 32 in the chat header and rail.
pub fn icon_toggle_sized(
    ui: &mut Ui,
    theme: &Palette,
    glyph: char,
    label: &str,
    keys: &str,
    on: bool,
    size: f32,
) -> Response {
    material_icon_button(ui, theme, glyph, label, keys, Some(on), size)
}

/// A Lucide `glyph` as an egui-material3 icon, drawn from the app's icon
/// family.
#[must_use]
pub fn lucide(glyph: char) -> egui_material3::Icon {
    egui_material3::Icon::glyph(
        glyph,
        egui::FontFamily::Name(crate::gui::font::ICONS.into()),
    )
}

/// egui-material3's standard icon button, square-cornered, `size` across,
/// drawing the Lucide `glyph`: a toggle when `on` is given. Material gives
/// every icon button a 48 dp touch target; in ave's dense rows that target
/// would overlap the neighbouring controls, so the button is laid out in a
/// child clipped to its visible square, which is also where egui clips its
/// hit area. The focus ring the clip cuts off is painted again outside it.
fn material_icon_button(
    ui: &mut Ui,
    theme: &Palette,
    glyph: char,
    label: &str,
    keys: &str,
    on: Option<bool>,
    size: f32,
) -> Response {
    use egui_material3::iconbutton::{
        IconButtonShape, IconButtonSize, MIN_TOUCH_TARGET, MaterialIconButton,
    };
    let (slot, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let enabled = ui.is_enabled();
    let mut selected = on.unwrap_or(false);
    let mut button = MaterialIconButton::standard(lucide(glyph))
        .size(IconButtonSize::Custom(size))
        .shape(IconButtonShape::Square)
        .label(label)
        .enabled(enabled);
    if on.is_some() {
        button = button.toggleable(&mut selected);
    }
    let target = Rect::from_center_size(slot.center(), Vec2::splat(size.max(MIN_TOUCH_TARGET)));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(target)
            .layout(Layout::left_to_right(Align::Min)),
    );
    child.set_clip_rect(slot.intersect(ui.clip_rect()));
    let response = child.add(button);
    if response.has_focus() {
        let radius = IconButtonSize::Custom(size)
            .metrics()
            .square_shape
            .radius(slot);
        egui_material3::paint::paint_focus_ring(
            ui.painter(),
            slot,
            CornerRadius::same(radius.round().clamp(0.0, 255.0) as u8),
            egui_material3::MaterialTheme::of(ui.ctx())
                .color(egui_material3::tokens::ColorRole::Secondary),
        );
    }
    tip(response, theme, label, keys)
}

/// A ghost dropdown trigger: `label` in 12 px Medium, muted until hovered
/// (or `open`), and a trailing chevron, 28 px tall, 8 px corners. Hand the
/// response to `egui::Popup::menu` for its menu. `tooltip` names it.
pub fn dropdown_button(
    ui: &mut Ui,
    theme: &Palette,
    label: &str,
    tooltip: &str,
    open: bool,
) -> Response {
    let font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
    let width = galley.size().x + icons::SMALL + SPACE_XS + SPACE_SM * 2.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, theme::CONTROL_HEIGHT), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tooltip));
    let lift = hover_t(ui, response.id, response.hovered());
    paint_ghost(ui, theme, rect, open, lift, theme::RADIUS_MD);
    let ink = if open {
        theme::rgb(theme.text)
    } else {
        blend(theme.text_muted, theme.text, lift)
    };
    let text_pos = egui::pos2(
        rect.left() + SPACE_SM,
        rect.center().y - galley.size().y / 2.0,
    );
    ui.painter().galley(text_pos, galley, ink);
    let slot = Rect::from_center_size(
        egui::pos2(
            rect.right() - SPACE_SM - icons::SMALL / 2.0,
            rect.center().y,
        ),
        Vec2::splat(icons::SMALL),
    );
    icons::paint(ui.painter(), slot, icons::CHEVRON_DOWN, icons::SMALL, ink);
    focus_ring(ui, theme, &response);
    tip(response, theme, tooltip, "")
}

/// A borderless toolbar row under a panel header (Selects' Project
/// toolbar): `height` tall, 2 px between items. `right` is laid out first,
/// right to left from the far end, so it always fits; `left` then gets
/// what is left, left to right, clipped short of it. Both sides are handed
/// `cx`, whatever they both need to change.
pub fn toolbar_row<C>(
    ui: &mut Ui,
    height: f32,
    cx: &mut C,
    left: impl FnOnce(&mut Ui, &mut C),
    right: impl FnOnce(&mut Ui, &mut C),
) {
    let width = ui
        .available_width()
        .min(ui.clip_rect().right() - ui.cursor().left());
    let (row, _) = ui.allocate_exact_size(Vec2::new(width.max(0.0), height), Sense::hover());
    let mut tail = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(Layout::right_to_left(Align::Center)),
    );
    tail.spacing_mut().item_spacing.x = 2.0;
    right(&mut tail, cx);
    let used = tail.min_rect();
    let end = if used.width() > 0.0 {
        used.left() - SPACE_XS
    } else {
        row.right()
    };
    let head_rect = Rect::from_min_max(row.min, egui::pos2(end.max(row.left()), row.bottom()));
    let mut head = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(head_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    head.set_clip_rect(head_rect.intersect(ui.clip_rect()));
    head.spacing_mut().item_spacing.x = 2.0;
    left(&mut head, cx);
}

/// The glass recipe as a frame, for anything floating over footage:
/// [`theme::glass_fill`], a white-10 % hairline, 12 px corners, 4 px inset
/// (Selects' glass island).
pub fn glass_frame(theme: &Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(theme::glass_fill(theme))
        .stroke(theme::ink_line(theme))
        .corner_radius(CornerRadius::same(theme::RADIUS_ISLAND))
        .inner_margin(theme::margin(SPACE_XS))
}

/// Which side of a [`glass_button`] its icon sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSide {
    /// Icon, then label (a button in a left corner).
    Leading,
    /// Label, then icon (a button in a right corner).
    Trailing,
}

/// A floating glass button with an icon and a label, painted at `at`
/// (its top-left corner, or its top-right with [`IconSide::Trailing`]):
/// what stands in for a folded panel over the viewer. 14 px Medium,
/// 8 x 10 padding, 12 px corners, ink 10 % on hover.
pub fn glass_button(
    ui: &mut Ui,
    theme: &Palette,
    at: egui::Pos2,
    glyph: char,
    label: &str,
    side: IconSide,
    keys: &str,
) -> Response {
    let font = theme::medium(ui.ctx(), theme::LARGE_SIZE);
    let ink = theme::rgb(theme.text);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, ink);
    let size = Vec2::new(
        galley.size().x + icons::SIZE + SPACE_SM + (SPACE_SM + 2.0) * 2.0,
        galley.size().y.max(icons::SIZE) + SPACE_SM * 2.0,
    );
    let rect = match side {
        IconSide::Leading => Rect::from_min_size(at, size),
        IconSide::Trailing => Rect::from_min_size(at - Vec2::new(size.x, 0.0), size),
    };
    let id = ui.scope_id().with(("glass", label));
    let response = ui.interact(rect, id, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let lift = hover_t(ui, id, response.hovered());
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(theme::RADIUS_ISLAND),
        theme::glass_fill(theme),
        theme::ink_line(theme),
        egui::StrokeKind::Inside,
    );
    if lift > 0.0 {
        painter.rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_ISLAND),
            theme::ink(theme, (f32::from(theme::INK_10) * lift).round() as u8),
        );
    }
    let inner = rect.shrink2(Vec2::new(SPACE_SM + 2.0, 0.0));
    let (icon_x, text_x) = match side {
        IconSide::Leading => (inner.left(), inner.left() + icons::SIZE + SPACE_SM),
        IconSide::Trailing => (inner.right() - icons::SIZE, inner.left()),
    };
    let slot = Rect::from_min_size(
        egui::pos2(icon_x, rect.center().y - icons::SIZE / 2.0),
        Vec2::splat(icons::SIZE),
    );
    icons::paint(painter, slot, glyph, icons::SIZE, ink);
    painter.galley(
        egui::pos2(text_x, rect.center().y - galley.size().y / 2.0),
        galley,
        ink,
    );
    focus_ring(ui, theme, &response);
    tip(
        response,
        theme,
        &format!("Show the {}", label.to_lowercase()),
        keys,
    )
}

/// A tooltip on `response`: `label`, then `keys` as [`kbd`] chips when
/// there are any, in Material's plain tooltip. Put hints here, not in a
/// line of text under a control. The tooltip takes its colours from the
/// Material theme, so the palette is not read.
pub fn tip(response: Response, _theme: &Palette, label: &str, keys: &str) -> Response {
    if label.is_empty() {
        return response;
    }
    if response.hovered() {
        super::hint::set(&response.ctx, label, keys);
    }
    let name = if keys.is_empty() {
        label.to_owned()
    } else {
        format!("{label} ({keys})")
    };
    super::hint::name_for_screen_readers(&response, &name);
    // Whether it showed changes nothing here: the response is returned
    // either way.
    let _ = egui_material3::MaterialTooltip::new(name).show_for_with(&response, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = SPACE_SM;
            ui.label(label);
            if !keys.is_empty() {
                let ink = ui.visuals().text_color();
                kbd_in(ui, keys, ink);
            }
        });
    });
    response
}

/// A rich tooltip on `response`: Material's rich tooltip (the raised
/// surface container) holding whatever `add_contents` lays out, for a
/// preview with several lines. `name` is what screen readers are told.
/// Like egui's tooltips it closes when the pointer leaves the control.
pub fn rich_tip(response: Response, name: &str, add_contents: impl FnOnce(&mut Ui)) -> Response {
    // Neither the tooltip's response nor the content's return value is
    // needed: it has no actions.
    let _ = egui_material3::RichTooltip::new(name)
        .persistent(false)
        .show_for_with(&response, add_contents);
    response
}

/// A keyboard shortcut as chips: `Ctrl+Shift+R` becomes three, one per
/// key; anything with a space in it (`Space / K`) stays one chip.
pub fn kbd(ui: &mut Ui, theme: &Palette, keys: &str) -> Response {
    kbd_in(ui, keys, theme::rgb(theme.text_muted))
}

/// [`kbd`] in `ink`, the chips tinted from it: on the inverse surface of a
/// tooltip the palette's muted ink would not show.
fn kbd_in(ui: &mut Ui, keys: &str, ink: Color32) -> Response {
    let parts: Vec<&str> = if keys.contains(' ') || keys.len() == 1 {
        vec![keys]
    } else {
        keys.split('+').filter(|part| !part.is_empty()).collect()
    };
    let font = theme::medium(ui.ctx(), theme::TINY_SIZE);
    let gap = 2.0;
    let galleys: Vec<_> = parts
        .into_iter()
        .map(|part| {
            ui.painter()
                .layout_no_wrap(part.to_owned(), font.clone(), ink)
        })
        .collect();
    let widths: Vec<f32> = galleys
        .iter()
        .map(|galley| (galley.size().x + SPACE_SM).max(SPACE_LG + 2.0))
        .collect();
    let total = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
    // One allocation, chips painted left to right inside it: in a
    // right-to-left row, separate allocations would reverse the keys.
    let (rect, response) = ui.allocate_exact_size(Vec2::new(total, SPACE_LG + 2.0), Sense::hover());
    let mut x = rect.left();
    for (galley, width) in galleys.into_iter().zip(widths) {
        let chip = Rect::from_min_size(egui::pos2(x, rect.top()), Vec2::new(width, rect.height()));
        x += width + gap;
        ui.painter().rect(
            chip,
            CornerRadius::same(4),
            ink.gamma_multiply(0.12),
            Stroke::new(1.0, ink.gamma_multiply(0.2)),
            egui::StrokeKind::Inside,
        );
        ui.painter()
            .galley(chip.center() - galley.size() / 2.0, galley, ink);
    }
    response
}

/// Views of one panel (Shots / People / Music): Material secondary tabs,
/// the open one underlined in primary. Returns whether the choice changed.
pub fn tab_strip<T: PartialEq + Copy>(
    ui: &mut Ui,
    _theme: &Palette,
    current: &mut T,
    items: &[(T, &str)],
) -> bool {
    // Material's secondary tabs (an on-surface label and a primary
    // underline on the open one), each as wide as its label, on the
    // header's own fill and without the bar's divider (the header has its
    // hairline). Even in its content layout the library claims the whole
    // row, so it gets a box exactly as wide as its tabs, and the header
    // keeps its other controls.
    use egui_material3::tabs::{TAB_HORIZONTAL_PADDING, TabLayout};
    if items.is_empty() {
        return false;
    }
    let style = egui_material3::tokens::TypeRole::TitleSmall.style();
    let font = style.font_id(ui.ctx());
    let width: f32 = items
        .iter()
        .map(|&(_, label)| {
            ui.painter()
                .layout_no_wrap(label.to_owned(), font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
                + TAB_HORIZONTAL_PADDING * 2.0
        })
        .sum();
    let height = theme::CONTROL_HEIGHT + SPACE_XS;
    let before = items.iter().position(|&(value, _)| value == *current);
    let mut index = before.unwrap_or(0);
    let salt = items
        .iter()
        .map(|&(_, label)| label)
        .collect::<Vec<_>>()
        .join("|");
    let tabs = items.iter().fold(
        egui_material3::MaterialTabs::secondary(&mut index)
            .id_salt(salt)
            .layout(TabLayout::Content)
            .transparent()
            .divider(false)
            .height(height),
        |tabs, &(_, label)| tabs.tab(label),
    );
    ui.allocate_ui(Vec2::new(width, height), |ui| {
        ui.set_width(width);
        ui.add(tabs)
    });
    match items.get(index) {
        Some(&(value, _)) if Some(index) != before => {
            *current = value;
            true
        }
        Some(_) | None => false,
    }
}

/// A segmented control: Material's segmented button at its densest (28 px,
/// the height of the controls around it), connected outlined segments with
/// the chosen one in the secondary container, no check mark. For a mode or
/// a small option set. Returns whether the choice changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut Ui,
    theme: &Palette,
    current: &mut T,
    items: &[(T, &str)],
) -> bool {
    segmented_gated(ui, theme, current, items, |_| None)
}

/// [`segmented`] with options that cannot be picked: `off` gives the reason
/// an option is unavailable (`None`: available). An unavailable option is
/// drawn faint, does not take a click, and says why on hover.
pub fn segmented_gated<T: PartialEq + Copy>(
    ui: &mut Ui,
    _theme: &Palette,
    current: &mut T,
    items: &[(T, &str)],
    off: impl Fn(T) -> Option<String>,
) -> bool {
    use egui_material3::{Segment, SegmentedButton};
    if items.is_empty() {
        return false;
    }
    let before = items.iter().position(|&(value, _)| value == *current);
    let mut index = before;
    let group = items
        .iter()
        .map(|&(_, label)| label)
        .collect::<Vec<_>>()
        .join(" / ");
    let reasons: Vec<Option<String>> = items.iter().map(|&(value, _)| off(value)).collect();
    let button = items.iter().zip(&reasons).fold(
        SegmentedButton::single_optional(&mut index)
            .id_salt(group.clone())
            .label(group.clone())
            .density(-3)
            .show_check(false),
        |button, (&(_, label), why)| {
            button.segment(match why {
                Some(why) => Segment::new(label).enabled(false).hover_text(why.clone()),
                None => Segment::new(label),
            })
        },
    );
    let shown = button.show(ui);
    // The status bar names what a segment does, or why it is off.
    for ((segment, &(_, label)), why) in shown.segments.iter().zip(items).zip(&reasons) {
        match why {
            Some(why) if segment.hovered() => super::hint::set(ui.ctx(), why, ""),
            Some(_) => {}
            None => super::hint::auto(segment, label),
        }
    }
    match index.and_then(|i| items.get(i)) {
        Some(&(value, _)) if index != before => {
            *current = value;
            true
        }
        Some(_) | None => false,
    }
}

/// The colour a [`pill`] takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Ink: a count, a neutral status.
    Neutral,
    /// The key colour: new, on, the thing to notice.
    Key,
    /// Passed, ready.
    Success,
    /// Needs a look.
    Warning,
    /// Failed, blocked.
    Danger,
}

/// A badge: short text on a tinted fill of its tone (the colour at 15 %
/// behind text in that colour), drawn as Material's labelled badge.
pub fn pill(ui: &mut Ui, theme: &Palette, text: &str, tone: Tone) -> Response {
    let (fill, ink) = match tone {
        Tone::Neutral => (
            theme::ink(theme, theme::INK_10),
            theme::rgb(theme.text_muted),
        ),
        Tone::Key => (
            theme::translucent(theme.accent, 38),
            theme::rgb(theme.accent),
        ),
        Tone::Success => (
            theme::translucent(theme.success, 38),
            theme::rgb(theme.success),
        ),
        Tone::Warning => (
            theme::translucent(theme.warning, 38),
            theme::rgb(theme.warning),
        ),
        Tone::Danger => (
            theme::translucent(theme.danger, 38),
            theme::rgb(theme.danger),
        ),
    };
    // Material's labelled badge (16 px, label-small, fully rounded) in the
    // tone's tint, inline rather than pinned to a corner.
    let response = ui.add(
        egui_material3::MaterialBadge::new(text)
            .color(egui_material3::BadgeColor::Custom(fill, ink))
            .description(text),
    );
    // Name it as text, so a screen reader reads the status.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    response
}

/// Nothing to show, as Selects says it: a quiet centred column (12 px
/// gaps, 24 px side padding) of a muted icon, the title in 12 px Medium
/// muted text, the hint under it at 70 %, and `actions` (buttons) below.
/// Use it for every empty list, panel and page instead of a line of muted
/// text; pass `""` for no hint and `|_| {}` for no action.
pub fn empty_state(
    ui: &mut Ui,
    theme: &Palette,
    glyph: char,
    title: &str,
    hint: &str,
    actions: impl FnOnce(&mut Ui),
) {
    let area = ui.available_rect_before_wrap();
    let id = ui.scope_id().with(("empty-state", title));
    // The content's height, measured last frame, centres it this frame.
    let last: f32 = ui.data(|data| data.get_temp(id)).unwrap_or(160.0);
    let top = area.top() + ((area.height() - last) / 2.0).max(SPACE_LG);
    let width = (area.width() - theme::SPACE_XL * 2.0).clamp(0.0, 320.0);
    let column = Rect::from_min_size(
        egui::pos2(area.center().x - width / 2.0, top),
        Vec2::new(width, area.height()),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(column)
            .layout(Layout::top_down(Align::Center)),
    );
    child.spacing_mut().item_spacing.y = SPACE_XS;
    let muted = theme::rgb(theme.text_muted);
    icons::icon(
        &mut child,
        glyph,
        icons::LARGE - 4.0,
        muted.gamma_multiply(0.7),
    );
    child.add_space(SPACE_SM);
    let title_font = theme::medium(child.ctx(), theme::BODY_SIZE);
    child.label(egui::RichText::new(title).font(title_font).color(muted));
    if !hint.is_empty() {
        child.add(
            egui::Label::new(
                egui::RichText::new(hint)
                    .line_height(Some(SPACE_LG + SPACE_XS))
                    .color(muted.gamma_multiply(0.7)),
            )
            .wrap()
            .halign(Align::Center),
        );
    }
    child.add_space(SPACE_SM);
    child.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_SM;
        actions(ui);
    });
    let used = child.min_rect().height();
    ui.data_mut(|data| data.insert_temp(id, used));
    ui.advance_cursor_after_rect(area);
}

/// A single-line field with Selects' input look: the `input` hairline, 8 px
/// corners, 28 px tall, the `ring` colour around it while focused, and
/// `hint` as the placeholder.
pub fn text_field(ui: &mut Ui, theme: &Palette, text: &mut String, hint: &str) -> Response {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, theme::CONTROL_HEIGHT), Sense::hover());
    field_frame(ui, theme, rect, text, hint, None)
}

/// [`text_field`] at a given `rect`, with an optional leading icon: for a
/// field placed by hand (the top bar's search). A field as tall as
/// [`theme::BAR_CONTROL`] takes 14 px text, 12 px padding and a 16 px
/// icon, as Selects' search pill; a smaller one 12 px text. Returns the
/// text edit's response.
pub fn field_frame(
    ui: &mut Ui,
    theme: &Palette,
    rect: Rect,
    text: &mut String,
    hint: &str,
    leading: Option<char>,
) -> Response {
    let large = rect.height() >= theme::BAR_CONTROL;
    let (size, pad, icon_size) = if large {
        (theme::LARGE_SIZE, SPACE_MD, icons::SIZE)
    } else {
        (theme::BODY_SIZE, SPACE_SM, icons::SMALL)
    };
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(theme::RADIUS_MD),
        theme::ink(theme, theme::INK_05),
    );
    let left = if leading.is_some() {
        pad + icon_size + SPACE_SM
    } else {
        pad + 2.0
    };
    let edit = egui::TextEdit::singleline(text)
        .hint_text(egui::RichText::new(hint).color(theme::rgb(theme.text_muted)))
        .font(theme::font(size))
        // A frame of our own replaces the theme's, margin included: the
        // fill and outline are painted around the edit, not by it.
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            left: theme::as_margin(left),
            right: theme::as_margin(SPACE_SM),
            top: 0,
            bottom: 0,
        }))
        .min_size(rect.size())
        .align(Align2::LEFT_CENTER)
        .desired_width(rect.width());
    let response = ui.put(rect, edit);
    let hovered = response.hovered();
    let stroke = if response.has_focus() {
        Stroke::new(1.0, theme::ring(theme))
    } else if hovered {
        Stroke::new(1.0, theme::ink(theme, theme::INK_20))
    } else {
        theme::input_line(theme)
    };
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(theme::RADIUS_MD),
        stroke,
        egui::StrokeKind::Inside,
    );
    if let Some(glyph) = leading {
        let slot = Rect::from_center_size(
            egui::pos2(rect.left() + pad + icon_size / 2.0, rect.center().y),
            Vec2::splat(icon_size),
        );
        icons::paint(
            ui.painter(),
            slot,
            glyph,
            icon_size,
            theme::rgb(theme.text_muted),
        );
    }
    response
}

/// The id a [`menu`] anchored to `anchor` keeps its open state under,
/// unless it is given one with [`Menu::id`].
#[must_use]
pub fn menu_id(anchor: &Response) -> egui::Id {
    anchor.id.with("material-menu")
}

/// Whether the [`menu`] with this id is open (to draw its anchor pressed).
#[must_use]
pub fn menu_is_open(ctx: &egui::Context, id: egui::Id) -> bool {
    ctx.data(|d| d.get_temp::<bool>(id.with("open")))
        .unwrap_or(false)
}

/// Opens the [`menu`] with this id if it is closed, closes it if open.
pub fn toggle_menu(ctx: &egui::Context, id: egui::Id) {
    let open = menu_is_open(ctx, id);
    ctx.data_mut(|d| d.insert_temp(id.with("open"), !open));
}

/// A Material 3 menu anchored to a control: the drop-in for
/// `egui::Popup::menu`. A click on the anchor opens and closes it; it
/// closes on a click outside, on Escape, and (by default) when an entry
/// inside is clicked. Build its content with ave's rows ([`menu_item`],
/// [`menu_heading`], [`menu_separator`]) and nest submenus with
/// [`submenu`].
pub fn menu(anchor: &Response) -> Menu<'_> {
    Menu {
        anchor,
        id: menu_id(anchor),
        width: None,
        gap: SPACE_XS,
        above: false,
        end: false,
        close: egui::PopupCloseBehavior::CloseOnClick,
    }
}

/// A menu being built; see [`menu`].
#[must_use = "show it with `.show(...)`"]
pub struct Menu<'a> {
    /// The control it opens from.
    anchor: &'a Response,
    /// Where its open state lives.
    id: egui::Id,
    /// A fixed width, or the content's own within Material's 112–280.
    width: Option<f32>,
    /// The space between the anchor and the menu.
    gap: f32,
    /// Opens above the anchor rather than below.
    above: bool,
    /// Lines up with the anchor's end edge rather than its start.
    end: bool,
    /// What clicks close it.
    close: egui::PopupCloseBehavior,
}

impl Menu<'_> {
    /// Keeps the open state under `id` (read it with [`menu_is_open`]).
    pub fn id(mut self, id: egui::Id) -> Self {
        self.id = id;
        self
    }

    /// A fixed width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// The space between the anchor and the menu.
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Where it opens: above the anchor (`egui::RectAlign::TOP_START` or
    /// `TOP_END`) or below it (any other), at the start or end edge.
    pub fn align(mut self, align: egui::RectAlign) -> Self {
        self.above = align == egui::RectAlign::TOP_START || align == egui::RectAlign::TOP_END;
        self.end = align == egui::RectAlign::TOP_END || align == egui::RectAlign::BOTTOM_END;
        self
    }

    /// What clicks close it (default: any click inside or outside).
    pub fn close_behavior(mut self, close: egui::PopupCloseBehavior) -> Self {
        self.close = close;
        self
    }

    /// Shows it while open; the content's value while it is.
    pub fn show<R>(self, add_contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
        use egui_material3::menu::{Corner, MenuStyle};
        let ctx = self.anchor.ctx.clone();
        let key = self.id.with("open");
        let mut open = ctx.data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
        if self.anchor.clicked() {
            open = !open;
        }
        let (anchor_corner, menu_corner) = match (self.above, self.end) {
            (false, false) => (Corner::BottomLeft, Corner::TopLeft),
            (false, true) => (Corner::BottomRight, Corner::TopRight),
            (true, false) => (Corner::TopLeft, Corner::BottomLeft),
            (true, true) => (Corner::TopRight, Corner::BottomRight),
        };
        let style = MenuStyle {
            min_width: self.width,
            max_width: self.width,
            ..MenuStyle::default()
        };
        let shown = egui_material3::MaterialMenu::new(self.id, &mut open)
            .anchor_rect(self.anchor.rect)
            .anchor_corner(anchor_corner)
            .menu_corner(menu_corner)
            .y_offset(if self.above { -self.gap } else { self.gap })
            .close_behavior(self.close)
            .style(style)
            .show_with(&ctx, add_contents);
        ctx.data_mut(|d| d.insert_temp(key, open));
        shown.map(|inner| inner.inner)
    }
}

/// A Material 3 context menu on `response`, opened by a secondary click:
/// the drop-in for `Response::context_menu`.
pub fn context_menu<R>(response: &Response, add_contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    egui_material3::MaterialMenu::context(response.id.with("material-context"))
        .show_context_with(response, add_contents)
        .map(|inner| inner.inner)
}

/// A row in a menu that opens a nested Material 3 submenu (on hover, a
/// click, Enter or the Right arrow): the drop-in for `ui.menu_button`.
/// Returns the row's response.
pub fn submenu<R>(
    ui: &mut Ui,
    label: impl Into<String>,
    add_contents: impl FnOnce(&mut Ui) -> R,
) -> Response {
    egui_material3::MaterialMenu::submenu(ui, label, add_contents).0
}

/// The frame of a popover or a hand-built menu, on Material's menu tokens:
/// the surface container, no outline, the extra-small shape, the level-2
/// key shadow, a 4 px inset.
pub fn popover_frame(theme: &Palette) -> egui::Frame {
    use egui_material3::tokens::{ColorRole, Elevation};
    let scheme = theme::material_scheme(theme);
    let [key, _ambient] = Elevation::Level2.shadows(scheme.get(ColorRole::Shadow));
    egui::Frame::new()
        .fill(scheme.get(ColorRole::SurfaceContainer))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .shadow(key)
        .inner_margin(theme::margin(SPACE_XS))
}

/// One entry in a menu or popover, on Material's menu-item tokens kept
/// dense (32 px tall rather than 48): label-large text, 12 px sides, an
/// optional Lucide icon, the shortcut right-aligned and faint, an
/// on-surface state layer under the pointer, and a check when `checked`
/// (pass `false` for a plain action).
pub fn menu_item(
    ui: &mut Ui,
    theme: &Palette,
    icon: Option<char>,
    label: &str,
    keys: &str,
    checked: bool,
) -> Response {
    let width = ui.available_width().max(160.0);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(width, theme::CONTROL_HEIGHT + SPACE_XS),
        Sense::click(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::MenuItem, ui.is_enabled(), checked, label)
    });
    let lift = hover_t(ui, response.id, response.hovered());
    // Material's menu item: an on-surface state layer (8 % hovered, 10 %
    // pressed or focused) and the label in label-large.
    egui_material3::paint::paint_state_layer(
        ui.painter(),
        rect,
        CornerRadius::same(theme::RADIUS),
        theme::rgb(theme.text),
        egui_material3::tokens::InteractionState::from_response(&response),
    );
    let painter = ui.painter();
    let mut x = rect.left() + SPACE_MD;
    let ink = theme::rgb(theme.text);
    if let Some(glyph) = icon {
        let slot = Rect::from_center_size(
            egui::pos2(x + icons::SMALL / 2.0, rect.center().y),
            Vec2::splat(icons::SMALL),
        );
        icons::paint(
            painter,
            slot,
            glyph,
            icons::SMALL,
            blend(theme.text_muted, theme.text, lift),
        );
        x += icons::SMALL + SPACE_SM;
    }
    painter.text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        egui_material3::tokens::TypeRole::LabelLarge
            .style()
            .font_id(ui.ctx()),
        ink,
    );
    let mut right = rect.right() - SPACE_MD;
    if checked {
        let slot = Rect::from_center_size(
            egui::pos2(right - icons::SMALL / 2.0, rect.center().y),
            Vec2::splat(icons::SMALL),
        );
        icons::paint(painter, slot, icons::CHECK, icons::SMALL, ink);
        right -= icons::SMALL + SPACE_SM;
    }
    if !keys.is_empty() {
        painter.text(
            egui::pos2(right, rect.center().y),
            Align2::RIGHT_CENTER,
            keys,
            theme::font(theme::SMALL_SIZE),
            theme::rgb(theme.text_faint),
        );
    }
    response
}

/// A menu section's label: 12 px Medium, muted.
pub fn menu_heading(ui: &mut Ui, theme: &Palette, text: &str) -> Response {
    let font = theme::medium(ui.ctx(), theme::BODY_SIZE);
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .font(font)
                .color(theme::rgb(theme.text_muted)),
        )
        .wrap_mode(egui::TextWrapMode::Truncate),
    )
}

/// The divider between groups in a menu: Material's divider (1 px in the
/// outline-variant colour) with 4 px of air above and below.
pub fn menu_separator(ui: &mut Ui, _theme: &Palette) {
    ui.add_space(SPACE_XS);
    ui.add(egui_material3::MaterialDivider::new());
    ui.add_space(SPACE_XS);
}

/// A vertical hairline between groups of controls in a bar.
pub fn bar_divider(ui: &mut Ui, theme: &Palette) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(SPACE_SM + 1.0, theme::COMPACT_HEIGHT),
        Sense::hover(),
    );
    let x = rect.center().x.round() + 0.5;
    ui.painter()
        .vline(x, rect.y_range().shrink(SPACE_XS), theme::ink_line(theme));
}

/// The side of a [`checkbox`] box.
pub const CHECKBOX_SIZE: f32 = 16.0;

/// Paints a tick box into `rect`: ticked is the primary fill with a check in
/// its text colour, unticked a 1.5 px outline in muted text (4.5:1 on any
/// surface), and either is knocked back when `enabled` is false. One
/// painter for every tick box, so a ticked box can never again read as an
/// unticked one (egui's own drew a grey check on a grey box).
pub fn paint_tick_box(
    painter: &egui::Painter,
    theme: &Palette,
    rect: Rect,
    on: bool,
    enabled: bool,
) {
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.45) };
    let corners = CornerRadius::same(theme::RADIUS_XS + 2);
    if on {
        painter.rect_filled(rect, corners, fade(theme::rgb(theme::primary_fill(theme))));
        icons::paint(
            painter,
            rect,
            icons::CHECK,
            rect.height() - 4.0,
            fade(theme::primary_text(theme)),
        );
    } else {
        painter.rect_stroke(
            rect,
            corners,
            Stroke::new(1.5, fade(theme::rgb(theme.text_muted))),
            egui::StrokeKind::Inside,
        );
    }
}

/// A checkbox with a label, a drop-in for `ui.checkbox` that is readable:
/// see [`paint_tick_box`]. Clicking the box or the label toggles `on` and
/// marks the response changed; `label` may be empty.
pub fn checkbox(ui: &mut Ui, _theme: &Palette, on: &mut bool, label: &str) -> Response {
    // Material's checkbox: primary fill and an on-primary check when ticked,
    // an on-surface-variant outline when not, its own state layer. It
    // takes its colours from the scheme `theme::apply` installed.
    let enabled = ui.is_enabled();
    let response = ui.add(egui_material3::MaterialCheckbox::new(on, label).enabled(enabled));
    super::hint::auto(&response, label);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_boxes_read_in_every_built_in_theme() -> anyhow::Result<()> {
        for theme in crate::theme::Catalog::built_in().themes() {
            // The check is painted in the panel colour on the primary fill.
            let fill = theme::primary_fill(theme);
            let tick = fill.contrast(theme.panel);
            assert!(
                tick >= crate::theme::SECONDARY_CONTRAST,
                "{}: the tick is {tick:.2}:1 on its box",
                theme.id
            );
            for surface in theme.surfaces() {
                let outline = theme.text_muted.contrast(surface);
                assert!(
                    outline >= crate::theme::INDICATOR_CONTRAST,
                    "{}: an unticked box is {outline:.2}:1 on its surface",
                    theme.id
                );
                let filled = fill.contrast(surface);
                assert!(
                    filled >= crate::theme::INDICATOR_CONTRAST,
                    "{}: a ticked box is {filled:.2}:1 on its surface",
                    theme.id
                );
            }
        }
        Ok(())
    }

    /// egui's own checkbox paints a grey check on a grey box, so a ticked
    /// box reads as unticked; every checkbox goes through [`checkbox`].
    #[test]
    fn no_screen_uses_egui_s_unreadable_checkbox() -> anyhow::Result<()> {
        fn walk(dir: &std::path::Path, hits: &mut Vec<String>) -> anyhow::Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    walk(&path, hits)?;
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path)?;
                    let banned = [concat!("ui.", "checkbox("), concat!("egui::", "Checkbox")];
                    for (n, line) in text.lines().enumerate() {
                        if banned.iter().any(|b| line.contains(b)) {
                            hits.push(format!("{}:{}", path.display(), n + 1));
                        }
                    }
                }
            }
            Ok(())
        }
        let mut hits = Vec::new();
        walk(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut hits,
        )?;
        assert!(
            hits.is_empty(),
            "use widgets::checkbox instead at: {hits:?}"
        );
        Ok(())
    }

    #[test]
    fn a_mention_keeps_the_reference_and_takes_the_intent() {
        use crate::refs::{Modifier, Reference};
        let picked = [
            Reference::new(crate::refs::Target::Person {
                name: "ava".to_owned(),
            }),
            Reference::new(crate::refs::Target::Person {
                name: "bo".to_owned(),
            }),
        ];
        let actions = mention_actions(&picked, Modifier::Avoid);
        assert_eq!(actions.len(), 2, "one per reference, in order");
        for (action, original) in actions.iter().zip(&picked) {
            assert_eq!(
                action,
                &crate::gui::app::Action::AddRef(Reference {
                    modifier: Modifier::Avoid,
                    ..original.clone()
                }),
                "the target is kept, the intent replaced"
            );
        }
        assert_eq!(mention_label(Modifier::About), "about");
        assert_eq!(mention_label(Modifier::Keep), "keep   +");
    }

    #[test]
    fn the_grid_fits_as_many_columns_as_the_minimum_allows() {
        assert_eq!(grid_layout(1000.0, 240.0, 12.0, 6).0, 4);
        assert_eq!(
            grid_layout(1000.0, 240.0, 12.0, 2).0,
            2,
            "never more than cards"
        );
        assert_eq!(
            grid_layout(100.0, 240.0, 12.0, 6).0,
            1,
            "one column at worst"
        );
    }

    #[test]
    fn the_columns_and_gaps_fill_the_width_exactly() {
        let (columns, width) = grid_layout(1000.0, 240.0, 12.0, 6);
        let total = width * columns as f32 + 12.0 * (columns as f32 - 1.0);
        assert!((total - 1000.0).abs() < 0.01, "the grid covers {total}");
    }

    #[test]
    fn nothing_to_lay_out_is_zero_columns_not_a_division_by_zero() {
        assert_eq!(grid_layout(1000.0, 240.0, 12.0, 0), (0, 0.0));
        assert_eq!(grid_layout(f32::NAN, 240.0, 12.0, 3), (0, 0.0));
    }
}

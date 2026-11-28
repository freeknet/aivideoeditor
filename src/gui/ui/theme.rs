// ============================================================================
// Module:       gui::ui::theme
// Description:  The active palette, the egui style derived from it, and the
//               design tokens every margin, radius, size and overlay in the
//               app comes from.
//
// Dependencies: egui; egui_material3 (tokens and its theme); crate::theme
//               (the palette), crate::color::Rgb, crate::gui::font (the
//               weights)
// ============================================================================

//! Where a theme becomes a window.
//!
//! The design tokens are Material 3's, from egui-material3: the corner
//! radii are its shape scale ([`CornerSize`]), the text sizes its type
//! scale ([`TypeRole`]), and the default themes its baseline colour scheme
//! (`crate::theme::material`). [`apply`] also hands the active palette to
//! the library as a Material colour scheme ([`material_scheme`]), so its
//! buttons, chips, tabs and switches draw in whichever theme is picked.
//!
//! [`apply`] installs the palette into egui's own `Visuals` once per frame,
//! before anything paints, so built-in widgets follow the theme without any
//! call site being involved, and no widget can be drawn under the previous
//! frame's theme. Custom drawing takes its colours from the same
//! [`Palette`], converted here by [`rgb`] — the single conversion point
//! between [`Rgb`] and `Color32`.
//!
//! The tokens are Selects' (its Tailwind v4 theme and shadcn components),
//! measured from its renderer; `docs/DESIGN.md` lists them with what each
//! one is for.
//!
//! ## A 4 px grid
//!
//! Every margin, inset and gap is one of [`SPACE_XS`] (4), [`SPACE_SM`]
//! (8), [`SPACE_MD`] (12), [`SPACE_LG`] (16) or [`SPACE_XL`] (24). A value
//! between two steps is one of the two steps, which is what makes the left
//! edges of every panel header, list and card line up.
//!
//! ## Ink
//!
//! Hover fills, dividers and quiet surfaces are not separate greys: they
//! are the theme's text colour (white in a dark theme, near-black in a
//! light one) at a fixed opacity — Selects' `rgb(var(--ink) / .05)`. The
//! same overlay reads the same on a panel, a card or a popover. The scale
//! is [`INK_03`] … [`INK_20`]; draw with [`ink`].

use crate::color::Rgb;
use crate::theme::{Mode, Palette};
use egui::{Color32, CornerRadius, Margin, Stroke, TextStyle, Vec2};
use egui_material3::MaterialTheme;
use egui_material3::tokens::{ColorRole, ColorScheme, CornerSize, TypeRole};

/// The tightest gap: between an icon and its label (Tailwind `1`).
pub const SPACE_XS: f32 = 4.0;

/// Between related controls in a row, and a panel's inset (`2`).
pub const SPACE_SM: f32 = 8.0;

/// Between groups of controls, and a card's inner margin (`3`).
pub const SPACE_MD: f32 = 12.0;

/// Between sections, and around a modal's body (`4`).
pub const SPACE_LG: f32 = 16.0;

/// Around a page's content (`6`).
pub const SPACE_XL: f32 = 24.0;

/// A Material 3 corner size as an egui radius. `Full` (a pill) has no
/// fixed value; it maps to the largest radius, which egui clamps to half
/// the shorter side, which is what a pill is.
const fn corner(size: CornerSize) -> u8 {
    match size.value() {
        Some(radius) => radius as u8,
        None => u8::MAX,
    }
}

/// The corner radius on thumbnails (2 px). Below Material's smallest step
/// on purpose: a thumbnail is footage, not a control.
pub const RADIUS_XS: u8 = 2;

/// The corner radius on small controls: menu items, tab pills, chips,
/// badges (Material's extra-small shape, 4 px).
pub const RADIUS: u8 = corner(CornerSize::ExtraSmall);

/// The corner radius on buttons, fields, segmented tracks and menus
/// (Material's small shape, 8 px).
pub const RADIUS_MD: u8 = corner(CornerSize::Small);

/// The corner radius on cards, popovers and modals (Material's medium
/// shape, 12 px).
pub const RADIUS_LG: u8 = corner(CornerSize::Medium);

/// The corner radius on floating glass islands: the buttons that stand in
/// for folded panels over the viewer, the timeline's tool island
/// (Material's large shape, 16 px).
pub const RADIUS_ISLAND: u8 = corner(CornerSize::Large);

/// The corner radius on the composer and option cards (Material's
/// large-increased shape, 20 px).
pub const RADIUS_XL: u8 = corner(CornerSize::LargeIncreased);

/// The corner radius on a modal dialog (Material's extra-large shape,
/// 28 px).
pub const RADIUS_DIALOG: u8 = corner(CornerSize::ExtraLarge);

/// The height of panel-header controls: tab pills, icon buttons, `xs`
/// buttons and fields (Selects' `h-7`, 28 px).
pub const CONTROL_HEIGHT: f32 = 28.0;

/// The height of a compact control: a chip, a small toggle (`h-6`, 24 px).
pub const COMPACT_HEIGHT: f32 = 24.0;

/// The size of the top bar's controls: its icon buttons and search pill
/// (Selects' default button size, `h-9`, 36 px).
pub const BAR_CONTROL: f32 = 36.0;

/// The width of the folded chat's icon rail (`w-11`, 44 px).
pub const CHAT_RAIL: f32 = 44.0;

/// The height of the top bar (52 px).
pub const HEADER_HEIGHT: f32 = 52.0;

/// The height of every panel's header row, with its hairline underneath:
/// chat, browser, viewer, inspector, timeline and export all match
/// (Selects' `h-10`, 40 px).
pub const PANEL_HEADER_HEIGHT: f32 = 40.0;

/// The inset a docked panel's frame gives its contents. A panel header
/// bleeds back across it, so its hairline runs edge to edge.
pub const PANEL_INSET: f32 = SPACE_SM;

/// The height of the status bar.
pub const STATUS_HEIGHT: f32 = 24.0;

/// The width of the expanded sidebar.
pub const SIDEBAR_WIDTH: f32 = 196.0;

/// The width of the collapsed, icon-only sidebar: one control and its insets.
pub const SIDEBAR_MINI_WIDTH: f32 = CONTROL_HEIGHT + SPACE_MD * 2.0;

/// The width of the accent bar down a selected item's leading edge. The
/// selection fill is a light tint, so this is what makes it unmistakable.
pub const SELECTION_BAR: f32 = 3.0;

/// The dense UI text size: labels, buttons, lists (Material's body small,
/// 12/16). The default.
pub const BODY_SIZE: f32 = TypeRole::BodySmall.style().size;

/// Reading text: the composer, messages, body copy (body medium, 14/20).
pub const LARGE_SIZE: f32 = TypeRole::BodyMedium.style().size;

/// Captions, units, badges (label small, 11/16).
pub const SMALL_SIZE: f32 = TypeRole::LabelSmall.style().size;

/// The smallest text: keyboard chips, track labels (10/12). Below
/// Material's scale on purpose: a timeline track label has to fit a lane.
pub const TINY_SIZE: f32 = 10.0;

/// Page heading size (title large, 22/28).
pub const HEADING_SIZE: f32 = TypeRole::TitleLarge.style().size;

/// Card and section title size (title small, 14/20, in Medium).
pub const TITLE_SIZE: f32 = TypeRole::TitleSmall.style().size;

/// The size of a panel header's title (label medium, 12/16, in Medium):
/// headers are labels, not headlines.
pub const PANEL_TITLE_SIZE: f32 = TypeRole::LabelMedium.style().size;

/// Ink at 3 %: the quietest wash, a striped row.
pub const INK_03: u8 = 8;

/// Ink at 5 %: an outline button's fill, a field's fill, an empty state's
/// icon plate (`bg-ink/5`).
pub const INK_05: u8 = 13;

/// Ink at 8 %: a hovered list row (`hover:bg-ink/[0.08]`).
pub const INK_08: u8 = 20;

/// Ink at 10 %: every hairline, Selects' `border` token (`white / .1`).
pub const INK_10: u8 = 26;

/// Ink at 12 %: the selected tab, toggle or icon button, and the hover of
/// a header icon button (`#333333` over the panel). Selection is this
/// fill, not a colour.
pub const INK_12: u8 = 31;

/// Ink at 15 %: a field's outline (`input`), a hovered hairline.
pub const INK_15: u8 = 38;

/// Ink at 20 %: a pressed control.
pub const INK_20: u8 = 51;

/// The opacity of a hairline. See [`INK_10`].
pub const INK_LINE: u8 = INK_10;

/// The opacity of a field's outline and a hovered hairline. See [`INK_15`].
pub const INK_STRONG: u8 = INK_15;

/// The opacity of a hovered row's fill. See [`INK_08`].
pub const INK_HOVER: u8 = INK_08;

// Relations between constants, checked when the crate is compiled.
const _: () = {
    assert!(SPACE_XS < SPACE_SM && SPACE_SM < SPACE_MD);
    assert!(SPACE_MD < SPACE_LG && SPACE_LG < SPACE_XL);
    assert!(
        SIDEBAR_MINI_WIDTH < SIDEBAR_WIDTH,
        "collapsing must narrow it"
    );
    assert!(HEADER_HEIGHT > CONTROL_HEIGHT, "the header holds a control");
    assert!(
        PANEL_HEADER_HEIGHT > CONTROL_HEIGHT + SPACE_XS,
        "a panel header holds a control with air around it"
    );
    assert!(COMPACT_HEIGHT < CONTROL_HEIGHT);
    assert!(TINY_SIZE < SMALL_SIZE && SMALL_SIZE < BODY_SIZE && BODY_SIZE < LARGE_SIZE);
    assert!(RADIUS < RADIUS_MD && RADIUS_MD < RADIUS_LG);
    assert!(INK_03 < INK_05 && INK_05 < INK_08 && INK_08 < INK_10);
    assert!(INK_10 < INK_12 && INK_12 < INK_15 && INK_15 < INK_20);
    assert!(RADIUS_XS < RADIUS && RADIUS_LG < RADIUS_ISLAND && RADIUS_ISLAND < RADIUS_XL);
    assert!(HEADER_HEIGHT > BAR_CONTROL && BAR_CONTROL > CONTROL_HEIGHT);
};

/// The theme's ink (its text colour) at `alpha`: one of [`INK_03`] …
/// [`INK_20`]. See the module docs.
#[must_use]
pub fn ink(theme: &Palette, alpha: u8) -> Color32 {
    translucent(theme.text, alpha)
}

/// A hairline in the theme's ink: its text colour at [`INK_LINE`], so a
/// divider reads the same on every surface it crosses.
#[must_use]
pub fn ink_line(theme: &Palette) -> Stroke {
    Stroke::new(1.0, ink(theme, INK_LINE))
}

/// A field's outline: Selects' `input` token, ink at 15 %.
#[must_use]
pub fn input_line(theme: &Palette) -> Stroke {
    Stroke::new(1.0, ink(theme, INK_STRONG))
}

/// The hover fill of a list row: ink at [`INK_HOVER`], laid over whatever
/// surface the row sits on.
#[must_use]
pub fn ink_hover(theme: &Palette) -> Color32 {
    ink(theme, INK_HOVER)
}

/// Selects' `accent` (`#404040`): the hover fill of a ghost button and a
/// menu item. Opaque, a step forward of the popover surface, so it reads
/// the same in a menu and on a panel.
#[must_use]
pub fn accent_fill(theme: &Palette) -> Color32 {
    rgb(theme.raised.lerp(theme.text, 0.12))
}

/// The selected fill: a tab, a toggle, an icon button that is on (ink at
/// 12 %, `#333333` over the panel).
#[must_use]
pub fn selected_fill(theme: &Palette) -> Color32 {
    ink(theme, INK_12)
}

/// A selected row in a list (Selects' `sidebar-ring`, `#525252`): stronger
/// than [`selected_fill`], because a list row has no other edge.
#[must_use]
pub fn selected_row(theme: &Palette) -> Color32 {
    rgb(theme.panel.lerp(theme.text, 0.26))
}

/// The glass recipe for anything floating over footage (folded-panel
/// buttons, the tool island, badges on video): the popover surface at
/// about 85 % (egui has no backdrop blur, so it is more opaque than
/// Selects' 50 %), a white-10 % hairline, white text.
#[must_use]
pub fn glass_fill(theme: &Palette) -> Color32 {
    translucent(theme.raised, 217)
}

/// The top bar's fill: the window colour at 20 % over the panels
/// (`#141414` in the signature theme).
#[must_use]
pub fn bar_fill(theme: &Palette) -> Color32 {
    rgb(theme.panel.lerp(theme.app, 0.2))
}

/// The outline of the focused panel (`#737373`, the only focus cue a
/// panel has); unfocused panels keep a transparent outline so nothing
/// shifts.
#[must_use]
pub fn panel_focus(theme: &Palette) -> Stroke {
    Stroke::new(1.0, ring(theme))
}

/// Selects' `ring`: the keyboard focus outline (neutral-500 in the dark
/// theme), between the muted text and the panel.
#[must_use]
pub fn ring(theme: &Palette) -> Color32 {
    rgb(theme.text_muted.lerp(theme.panel, 0.36))
}

/// Selects' `primary`: the neutral-200 fill of a primary button (a step
/// back from the text colour), with [`primary_text`] on it.
#[must_use]
pub fn primary_fill(theme: &Palette) -> Rgb {
    theme.text.lerp(theme.panel, 0.08)
}

/// The text on [`primary_fill`]: the panel colour.
#[must_use]
pub fn primary_text(theme: &Palette) -> Color32 {
    rgb(theme.panel)
}

/// Selects' `popover` and `muted`: the surface of a menu, a tooltip, a
/// segmented control's track.
#[must_use]
pub fn popover_fill(theme: &Palette) -> Color32 {
    rgb(theme.raised)
}

/// The shadow under floating things (menus, popovers, windows): soft and
/// low, as Selects' `shadow-md`, darkened to show on a dark surface.
#[must_use]
pub fn float_shadow() -> egui::Shadow {
    egui::Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(90),
    }
}

/// A font id in Mona Sans Medium at `size`: panel headers, titles.
#[must_use]
pub fn medium(ctx: &egui::Context, size: f32) -> egui::FontId {
    crate::gui::font::weight(ctx, crate::gui::font::MEDIUM, size)
}

/// A font id in Mona Sans SemiBold at `size`: buttons.
#[must_use]
pub fn semibold(ctx: &egui::Context, size: f32) -> egui::FontId {
    crate::gui::font::weight(ctx, crate::gui::font::SEMIBOLD, size)
}

/// A font id in Roboto Mono at `size`: code and query tokens.
#[must_use]
pub fn mono(size: f32) -> egui::FontId {
    egui::FontId::monospace(size)
}

/// A font id in Inter at `size`: timecode and numbers that line up
/// (Selects' `font-tabular`). egui cannot switch on Inter's `tnum`
/// feature, so where digits must not move at all (a running clock), lay
/// each digit out in a fixed-width cell or use [`mono`].
#[must_use]
pub fn tabular(ctx: &egui::Context, size: f32) -> egui::FontId {
    crate::gui::font::weight(ctx, crate::gui::font::TABULAR, size)
}

/// The Material 3 colour scheme the library's widgets draw `theme` with.
///
/// The Material themes hand over their own baseline scheme, with `surface`
/// set to the panel (see below). Any
/// other theme is mapped role by role from its palette, starting from the
/// baseline of its mode so a role ave has no colour for still has a sane
/// one: surfaces back to front onto the surface containers, the accent onto
/// primary, the selection fill onto the secondary container (the tonal
/// fill M3 uses for a selected chip or nav item), hairlines onto outlines.
#[must_use]
pub fn material_scheme(theme: &Palette) -> ColorScheme {
    let dark = theme.mode == Mode::Dark;
    let mut scheme = ColorScheme::baseline(dark);
    // The library paints a widget's backdrop (a tab bar, a disabled
    // button) in `surface`; the app's panels are the low container, so
    // `surface` is the panel or those widgets would sit in a darker box.
    scheme.set(ColorRole::Surface, rgb(theme.panel));
    scheme.set(ColorRole::Background, rgb(theme.panel));
    if theme.id == crate::theme::MATERIAL_NIGHT || theme.id == crate::theme::MATERIAL_LIGHT {
        return scheme;
    }
    let mid = rgb(theme.panel.lerp(theme.raised, 0.5));
    for (role, color) in [
        (ColorRole::Primary, rgb(theme.accent)),
        (ColorRole::SurfaceTint, rgb(theme.accent)),
        (ColorRole::OnPrimary, rgb(theme.text_on_accent)),
        (ColorRole::PrimaryContainer, rgb(theme.accent_soft)),
        (ColorRole::OnPrimaryContainer, rgb(theme.text)),
        (ColorRole::InversePrimary, rgb(theme.accent)),
        (ColorRole::PrimaryFixed, rgb(theme.accent)),
        (ColorRole::PrimaryFixedDim, rgb(theme.accent_hover)),
        (ColorRole::OnPrimaryFixed, rgb(theme.text_on_accent)),
        (ColorRole::OnPrimaryFixedVariant, rgb(theme.text_on_accent)),
        (ColorRole::Secondary, rgb(theme.text_muted)),
        (ColorRole::OnSecondary, rgb(theme.panel)),
        (ColorRole::SecondaryContainer, rgb(theme.selection)),
        (ColorRole::OnSecondaryContainer, rgb(theme.selection_text)),
        (ColorRole::Tertiary, rgb(theme.success)),
        (ColorRole::TertiaryContainer, rgb(theme.accent_soft)),
        (ColorRole::OnTertiaryContainer, rgb(theme.text)),
        (ColorRole::Error, rgb(theme.danger)),
        (ColorRole::OnError, rgb(theme.text_on_danger)),
        (
            ColorRole::ErrorContainer,
            rgb(theme.panel.lerp(theme.danger, 0.25)),
        ),
        (ColorRole::OnErrorContainer, rgb(theme.text)),
        (ColorRole::Background, rgb(theme.panel)),
        (ColorRole::OnBackground, rgb(theme.text)),
        (ColorRole::Surface, rgb(theme.panel)),
        (ColorRole::OnSurface, rgb(theme.text)),
        (ColorRole::SurfaceVariant, rgb(theme.raised)),
        (ColorRole::OnSurfaceVariant, rgb(theme.text_muted)),
        (ColorRole::Outline, rgb(theme.border_strong)),
        (ColorRole::OutlineVariant, rgb(theme.border)),
        (ColorRole::InverseSurface, rgb(theme.text)),
        (ColorRole::InverseOnSurface, rgb(theme.panel)),
        (ColorRole::SurfaceDim, rgb(theme.app)),
        (ColorRole::SurfaceBright, rgb(theme.hover)),
        (ColorRole::SurfaceContainerLowest, rgb(theme.app)),
        (ColorRole::SurfaceContainerLow, rgb(theme.panel)),
        (ColorRole::SurfaceContainer, mid),
        (ColorRole::SurfaceContainerHigh, rgb(theme.raised)),
        (ColorRole::SurfaceContainerHighest, rgb(theme.hover)),
    ] {
        scheme.set(role, color);
    }
    scheme
}

/// Hands `theme` to egui-material3 on this context (what its widgets read
/// through [`MaterialTheme::of`]), so they draw in the active palette.
/// Only when the scheme changed.
fn install_material(ctx: &egui::Context, theme: &Palette) {
    let scheme = material_scheme(theme);
    let dark = theme.mode == Mode::Dark;
    let current = MaterialTheme::of(ctx);
    if current.colors == scheme && current.is_dark == dark {
        return;
    }
    MaterialTheme::set(
        ctx,
        MaterialTheme {
            colors: scheme,
            is_dark: dark,
            ..MaterialTheme::baseline(dark)
        },
    );
}

/// Installs `theme` into egui's style. Called once per frame, first.
pub fn apply(ctx: &egui::Context, theme: &Palette) {
    install_material(ctx, theme);
    // egui keeps a light and a dark `Style` and picks by the OS setting.
    // The palette already decides, so the preference is pinned to it —
    // otherwise the fields set here would sit on top of the wrong defaults.
    ctx.set_theme(match theme.mode {
        Mode::Dark => egui::ThemePreference::Dark,
        Mode::Light => egui::ThemePreference::Light,
    });
    let derived = visuals(theme);
    // Buttons and headings take their weights only once the bundled faces
    // are in; before that (the first frame) the regular face stands in.
    let button = semibold(ctx, BODY_SIZE);
    let heading = semibold(ctx, HEADING_SIZE);
    ctx.all_styles_mut(|style| {
        style.visuals = derived.clone();
        // Dense, as Selects is: 4 px rows, 8 px between controls in a row.
        style.spacing.item_spacing = Vec2::new(SPACE_SM, SPACE_XS);
        style.spacing.button_padding = Vec2::new(SPACE_SM, SPACE_XS);
        style.spacing.interact_size = Vec2::new(0.0, COMPACT_HEIGHT);
        style.spacing.icon_width = SPACE_LG;
        style.spacing.icon_width_inner = SPACE_SM;
        style.spacing.icon_spacing = SPACE_SM - 2.0;
        style.spacing.menu_margin = margin(SPACE_XS);
        style.spacing.menu_spacing = 2.0;
        style.spacing.window_margin = margin(SPACE_LG);
        style.spacing.combo_width = 120.0;
        style.spacing.text_edit_width = 200.0;
        style.spacing.tooltip_width = 280.0;
        // Thin bars floating over the content, as in a browser: a 4 px
        // handle that is always faintly visible (a bar that shows only
        // under the pointer cannot be found), widening to 8 px on hover.
        let mut scroll = egui::style::ScrollStyle::floating();
        scroll.bar_width = SPACE_SM;
        scroll.floating_width = SPACE_XS;
        scroll.bar_inner_margin = 2.0;
        scroll.bar_outer_margin = 2.0;
        scroll.handle_min_length = SPACE_XL;
        scroll.foreground_color = false;
        scroll.dormant_handle_opacity = 0.45;
        scroll.active_handle_opacity = 0.7;
        scroll.interact_handle_opacity = 1.0;
        scroll.dormant_background_opacity = 0.0;
        scroll.active_background_opacity = 0.0;
        scroll.interact_background_opacity = 0.0;
        style.spacing.scroll = scroll;
        // Labels are not text fields; a selectable label makes the pointer
        // an I-beam over every piece of chrome.
        style.interaction.selectable_labels = false;
        style.interaction.tooltip_delay = 0.35;
        for (text_style, font) in [
            (TextStyle::Body, egui::FontId::proportional(BODY_SIZE)),
            (TextStyle::Button, button.clone()),
            (TextStyle::Small, egui::FontId::proportional(SMALL_SIZE)),
            (TextStyle::Heading, heading.clone()),
            (TextStyle::Monospace, egui::FontId::monospace(BODY_SIZE)),
        ] {
            style.text_styles.insert(text_style, font);
        }
    });
}

/// Converts a palette colour to an egui one.
#[must_use]
pub const fn rgb(color: Rgb) -> Color32 {
    Color32::from_rgb(color.r, color.g, color.b)
}

/// A palette colour at a given opacity, for scrims and overlays, where the
/// alpha is the design decision and the colour still comes from the theme.
#[must_use]
pub fn translucent(color: Rgb, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r, color.g, color.b, alpha)
}

/// A spacing-scale value as a margin unit, clamped rather than wrapped:
/// `Margin` is `i8`, and a bare cast of a large value would go negative.
#[must_use]
pub fn as_margin(space: f32) -> i8 {
    if space.is_nan() {
        return 0;
    }
    space.round().clamp(0.0, 127.0) as i8
}

/// A uniform margin from a scale value.
#[must_use]
pub fn margin(space: f32) -> Margin {
    Margin::same(as_margin(space))
}

/// A margin with separate horizontal and vertical steps.
#[must_use]
pub fn margin_xy(horizontal: f32, vertical: f32) -> Margin {
    Margin::symmetric(as_margin(horizontal), as_margin(vertical))
}

/// A proportional font at `size`.
#[must_use]
pub fn font(size: f32) -> egui::FontId {
    egui::FontId::proportional(size)
}

/// egui's `Visuals`, derived from a palette: shadcn's look for every
/// built-in widget. Buttons and combo boxes are Selects' `outline` variant
/// (ink 5 % fill, `input` outline, `accent` on hover); frameless controls
/// (menu entries, selectable labels) show only the `accent` fill on hover;
/// hairlines are ink; the selection is the key colour.
#[must_use]
pub fn visuals(theme: &Palette) -> egui::Visuals {
    let mut visuals = match theme.mode {
        Mode::Dark => egui::Visuals::dark(),
        Mode::Light => egui::Visuals::light(),
    };
    visuals.panel_fill = rgb(theme.panel);
    visuals.window_corner_radius = CornerRadius::same(RADIUS_LG);
    // Every egui menu and popup takes Material's menu shape (extra-small).
    visuals.menu_corner_radius = CornerRadius::same(RADIUS);
    visuals.faint_bg_color = ink(theme, INK_03);
    // Text fields draw on this: the panel itself, with the `input` outline
    // doing the work, as shadcn's transparent inputs do.
    visuals.extreme_bg_color = rgb(theme.panel);
    visuals.text_edit_bg_color = Some(rgb(theme.panel));
    visuals.code_bg_color = ink(theme, INK_10);
    visuals.hyperlink_color = rgb(theme.accent);
    visuals.warn_fg_color = rgb(theme.warning);
    visuals.error_fg_color = rgb(theme.danger);
    visuals.override_text_color = Some(rgb(theme.text));
    visuals.weak_text_color = Some(rgb(theme.text_muted));
    // Selection is a neutral fill with foreground text, not the key colour
    // (which egui would otherwise paint selected labels and the focused
    // field's outline in).
    visuals.selection.bg_fill = rgb(theme.selection);
    visuals.selection.stroke = Stroke::new(1.0, rgb(theme.text));
    visuals.slider_trailing_fill = true;
    visuals.striped = false;
    visuals.indent_has_left_vline = false;
    visuals.button_frame = true;
    visuals.collapsing_header_frame = false;
    visuals.handle_shape = egui::style::HandleShape::Circle;

    // egui paints *buttons* from `weak_bg_fill` and *filled controls* — the
    // scrollbar handle, a checkbox's interior, a slider's rail — from
    // `bg_fill`. Pointing both at a surface makes every scrollbar in the
    // app invisible, so the second always gets the control colour.
    for (state, fill, control, outline) in [
        (
            &mut visuals.widgets.noninteractive,
            Color32::TRANSPARENT,
            theme.control,
            ink(theme, INK_LINE),
        ),
        (
            &mut visuals.widgets.inactive,
            ink(theme, INK_05),
            theme.control,
            ink(theme, INK_STRONG),
        ),
        (
            &mut visuals.widgets.hovered,
            accent_fill(theme),
            theme.control_hover,
            ink(theme, INK_20),
        ),
        (
            &mut visuals.widgets.active,
            ink(theme, INK_20),
            theme.accent,
            ring(theme),
        ),
        (
            &mut visuals.widgets.open,
            accent_fill(theme),
            theme.control_hover,
            ink(theme, INK_20),
        ),
    ] {
        state.weak_bg_fill = fill;
        state.bg_fill = rgb(control);
        state.bg_stroke = Stroke::new(1.0, outline);
        state.fg_stroke = Stroke::new(1.0, rgb(theme.text));
        state.corner_radius = CornerRadius::same(RADIUS);
        // No growth on hover: it nudges neighbouring widgets.
        state.expansion = 0.0;
    }
    // The noninteractive stroke is every separator and panel edge: a
    // hairline, never heavier.
    visuals.widgets.noninteractive.bg_stroke = ink_line(theme);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, rgb(theme.text_muted));
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, rgb(theme.text_muted));
    // Flat panels; only floating things (menus, popovers, windows) carry a
    // shadow, and a soft one.
    // Menus and popups sit at Material's level 2 (its key shadow; egui
    // draws one shadow per frame).
    let [key, _ambient] = egui_material3::tokens::Elevation::Level2.shadows(Color32::BLACK);
    visuals.popup_shadow = key;
    visuals.window_shadow = float_shadow();
    visuals.window_stroke = ink_line(theme);
    visuals.window_fill = popover_fill(theme);
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scrollbar_handle_is_never_the_colour_of_the_surface_it_scrolls() {
        let theme = crate::theme::signature();
        let visuals = visuals(&theme);
        for state in [
            &visuals.widgets.noninteractive,
            &visuals.widgets.inactive,
            &visuals.widgets.hovered,
            &visuals.widgets.open,
        ] {
            assert_ne!(state.bg_fill, state.weak_bg_fill, "a control vanishes");
            assert_ne!(state.bg_fill, rgb(theme.panel), "a handle on the panel");
        }
    }

    #[test]
    fn the_derived_visuals_carry_the_themes_own_colours() {
        let theme = crate::theme::signature();
        let visuals = visuals(&theme);
        assert_eq!(visuals.panel_fill, rgb(theme.panel));
        assert_eq!(visuals.override_text_color, Some(rgb(theme.text)));
        assert_eq!(visuals.window_fill, popover_fill(&theme));
        assert!(visuals.dark_mode, "the signature theme is dark");
    }

    #[test]
    fn the_signature_tokens_are_selects_own() {
        let theme = crate::theme::signature();
        assert_eq!(theme.panel, Rgb::new(0x17, 0x17, 0x17), "background");
        assert_eq!(theme.raised, Rgb::new(0x26, 0x26, 0x26), "popover");
        assert_eq!(theme.text, Rgb::new(0xfa, 0xfa, 0xfa), "foreground");
        assert_eq!(theme.accent, Rgb::new(0xa3, 0xe6, 0x35), "key");
        assert_eq!(theme.text_on_accent, Rgb::new(23, 23, 23), "key text");
        assert_eq!(theme.danger, Rgb::new(0xef, 0x44, 0x44), "destructive");
        // The hover fill is `accent`, #404040, and the selection #333333,
        // to within a step or two.
        let accent = accent_fill(&theme);
        assert!(
            (i32::from(accent.r()) - 0x40).abs() <= 2,
            "accent fill is {accent:?}"
        );
        assert!(
            (i32::from(theme.selection.r) - 0x33).abs() <= 2,
            "selection is {:?}",
            theme.selection
        );
        assert_eq!(theme.text_muted, Rgb::new(0xa3, 0xa3, 0xa3), "muted");
        // The ring is neutral-500, #737373, to within a few steps.
        let ring = ring(&theme);
        assert!((i32::from(ring.r()) - 0x73).abs() <= 4, "ring is {ring:?}");
    }

    #[test]
    fn every_theme_still_derives_a_style() {
        for theme in crate::theme::Catalog::built_in().themes() {
            let visuals = visuals(theme);
            assert_eq!(
                visuals.dark_mode,
                theme.mode == Mode::Dark,
                "{} keeps its mode",
                theme.id
            );
        }
    }

    #[test]
    fn an_absurd_margin_is_clamped_rather_than_wrapping() {
        assert_eq!(as_margin(10_000.0), 127);
        assert_eq!(as_margin(-50.0), 0);
        assert_eq!(as_margin(f32::NAN), 0);
        assert_eq!(as_margin(SPACE_MD), 12);
    }
}

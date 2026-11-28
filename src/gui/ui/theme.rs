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

/// The glass recipe for controls floating over the viewer (folded-panel
/// buttons, the tool island): the popover surface at about 85 % (egui has
/// no backdrop blur, so it is more opaque than Selects' 50 %), an ink
/// hairline, the theme's text. Badges on footage use [`scrim`] instead.
#[must_use]
pub fn glass_fill(theme: &Palette) -> Color32 {
    translucent(theme.raised, 217)
}

/// How opaque [`scrim`] is: about 63 %, so white text on it clears 4.5:1
/// even over a white frame, the brightest footage there is.
const SCRIM_ALPHA: u8 = 160;

/// The plate under small text and icons painted on footage (a
/// thumbnail's duration, mark, rating and count badges, the tape's play
/// disc): black at about 63 % in every theme, light or dark, with
/// [`ON_SCRIM`] text, the video-app convention. Footage is not a theme
/// surface, so a theme-coloured plate (a light one in Material Light)
/// would leave white text unreadable over a bright frame.
#[must_use]
pub fn scrim() -> Color32 {
    Color32::from_black_alpha(SCRIM_ALPHA)
}

/// Text on [`scrim`]: white, in every theme.
pub const ON_SCRIM: Color32 = Color32::WHITE;

/// What [`scrim`] looks like over a frame of colour `under`: the colour
/// text on it is read against.
#[must_use]
pub fn scrim_over(under: Rgb) -> Rgb {
    under.lerp(Rgb::new(0, 0, 0), f32::from(SCRIM_ALPHA) / 255.0)
}

/// `color` as an icon on [`scrim`]: lifted toward white, in tenths, until
/// it clears 3:1 against the scrim over a white frame. A light theme's
/// warning or danger is dark enough to vanish on the scrim otherwise.
#[must_use]
pub fn on_scrim(color: Rgb) -> Color32 {
    let worst = scrim_over(Rgb::new(255, 255, 255));
    let white = Rgb::new(255, 255, 255);
    let lifted = (0..=10)
        .map(|k| color.lerp(white, k as f32 / 10.0))
        .find(|c| c.contrast(worst) >= 3.0)
        .unwrap_or(white);
    rgb(lifted)
}

/// A region of the window, named for the surface it is painted on.
///
/// Every region takes its own tonal level from Material 3's surface
/// container roles ([`Surface::role`]), so neighbouring regions read apart
/// by one quiet step and a theme (Material Night, Material Light or any
/// other) decides the actual colours. This is the one place that says
/// which region sits on which role; a panel's fill comes from
/// [`surface`] (or [`surface_of`] where no context is at hand), never from
/// a palette field picked at the call site.
///
/// Back to front, in a dark theme (a light one runs the same order from
/// white toward grey):
///
/// | Region | Role | What |
/// |---|---|---|
/// | [`Canvas`](Surface::Canvas) | lowest container | behind the picture in the viewer; the Home screen |
/// | [`Chrome`](Surface::Chrome) | dim (dark) / bright (light) | top bar, guide strip, status bar |
/// | [`Content`](Surface::Content) | low container | the viewer panel; cards on Home |
/// | [`Panel`](Surface::Panel) | container | chat, browser, inspector, export, folded rails |
/// | [`Timeline`](Surface::Timeline) | high container | the timeline |
/// | [`Island`](Surface::Island) | highest container | the timeline's floating tool island |
/// | [`Overlay`](Surface::Overlay) | bright (dark) / dim (light) | menus, popovers, dialogs, hover labels |
///
/// `Chrome` and `Overlay` are the two roles whose tone is absolute rather
/// than relative to the mode: the dim surface is the darkest-but-one in a
/// dark scheme and the darkest of all in a light one. Picking by mode keeps
/// the chrome just behind the content and the overlays just in front of
/// everything in both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Surface {
    /// Behind the footage: the viewer's stage, the Home screen.
    Canvas,
    /// The window's frame: top bar, guide strip, status bar.
    Chrome,
    /// The main work surface: the viewer panel, Home's cards.
    Content,
    /// The side panels: chat, browser, inspector, export.
    Panel,
    /// The timeline.
    Timeline,
    /// Controls floating over the timeline.
    Island,
    /// Menus, popovers, dialogs and hover labels.
    Overlay,
}

impl Surface {
    /// Every region, back to front.
    pub const ALL: [Self; 7] = [
        Self::Canvas,
        Self::Chrome,
        Self::Content,
        Self::Panel,
        Self::Timeline,
        Self::Island,
        Self::Overlay,
    ];

    /// The Material 3 role this region is painted in, in a dark scheme or
    /// a light one. See the type's docs for the table.
    #[must_use]
    pub const fn role(self, dark: bool) -> ColorRole {
        match self {
            Self::Canvas => ColorRole::SurfaceContainerLowest,
            Self::Chrome if dark => ColorRole::SurfaceDim,
            Self::Chrome => ColorRole::SurfaceBright,
            Self::Content => ColorRole::SurfaceContainerLow,
            Self::Panel => ColorRole::SurfaceContainer,
            Self::Timeline => ColorRole::SurfaceContainerHigh,
            Self::Island => ColorRole::SurfaceContainerHighest,
            Self::Overlay if dark => ColorRole::SurfaceBright,
            Self::Overlay => ColorRole::SurfaceDim,
        }
    }

    /// Pairs of regions that touch in the window (or, for the overlay, can
    /// be drawn over one another): each pair must read apart.
    pub const NEIGHBOURS: [(Self, Self); 9] = [
        (Self::Canvas, Self::Content),
        (Self::Chrome, Self::Content),
        (Self::Chrome, Self::Panel),
        (Self::Chrome, Self::Timeline),
        (Self::Content, Self::Panel),
        (Self::Content, Self::Timeline),
        (Self::Panel, Self::Timeline),
        (Self::Timeline, Self::Island),
        (Self::Island, Self::Overlay),
    ];
}

/// The colour of `region` in the Material theme installed on `ctx` (what
/// [`apply`] put there this frame).
#[must_use]
pub fn surface(ctx: &egui::Context, region: Surface) -> Color32 {
    let m3 = MaterialTheme::of(ctx);
    m3.color(region.role(m3.is_dark))
}

/// The colour of `region` under `theme`, for code with no context at hand
/// (deriving egui's visuals). Same answer as [`surface`] once the theme is
/// applied.
#[must_use]
pub fn surface_of(theme: &Palette, region: Surface) -> Color32 {
    material_scheme(theme).get(region.role(theme.mode == Mode::Dark))
}

/// The surface the widget being drawn sits on: the region [`on_surface`]
/// scoped, or the content surface outside any. Paint with it where a shape
/// has to cut through to the background (a gap between segments, a dimmed
/// clip, a ruler band).
#[must_use]
pub fn current_surface(ctx: &egui::Context) -> Color32 {
    MaterialTheme::of(ctx).color(ColorRole::Surface)
}

/// The keyboard focus indicator's colour: Material's `secondary` role, its
/// focus-ring token. It clears 3:1 against every region in every built-in
/// theme (checked by the tests), which the neutral [`ring`] does not.
#[must_use]
pub fn focus_ink(ctx: &egui::Context) -> Color32 {
    MaterialTheme::of(ctx).color(ColorRole::Secondary)
}

/// Runs `add` with Material's `surface` role set to `region`'s colour, and
/// puts the previous theme back afterwards.
///
/// egui-material3 paints a widget's own backdrop (a list's container, a tab
/// row) in `surface`, and [`current_surface`] reads it; scoping it to the
/// region a panel is drawn in makes those blend into the panel instead of
/// sitting in a box of the content colour.
pub fn on_surface<R>(
    ui: &mut egui::Ui,
    region: Surface,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let ctx = ui.ctx().clone();
    let before = MaterialTheme::of(&ctx);
    let fill = before.color(region.role(before.is_dark));
    if before.color(ColorRole::Surface) == fill {
        return add(ui);
    }
    let mut scoped = (*before).clone();
    scoped.colors.set(ColorRole::Surface, fill);
    MaterialTheme::set(&ctx, scoped);
    let out = add(ui);
    MaterialTheme::set(&ctx, (*before).clone());
    out
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

/// The surface of a menu, a popover, a hover label or a badge laid over
/// other content: the [`Surface::Overlay`] level, in front of every region.
#[must_use]
pub fn popover_fill(theme: &Palette) -> Color32 {
    surface_of(theme, Surface::Overlay)
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
    // The dim and bright surfaces as Material places them: in a dark
    // scheme dim sits just behind the low container (between it and the
    // window) and bright beyond the highest; in a light one the other way
    // round. `Surface::Chrome` and `Surface::Overlay` rely on that.
    let behind = rgb(theme.app.lerp(theme.panel, 0.5));
    let beyond = rgb(theme.hover);
    let (dim, bright) = if dark {
        (behind, beyond)
    } else {
        (beyond, behind)
    };
    let highest = rgb(theme.raised.lerp(theme.hover, 0.5));
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
        (ColorRole::SurfaceDim, dim),
        (ColorRole::SurfaceBright, bright),
        (ColorRole::SurfaceContainerLowest, rgb(theme.app)),
        (ColorRole::SurfaceContainerLow, rgb(theme.panel)),
        (ColorRole::SurfaceContainer, mid),
        (ColorRole::SurfaceContainerHigh, rgb(theme.raised)),
        (ColorRole::SurfaceContainerHighest, highest),
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
    visuals.panel_fill = surface_of(theme, Surface::Content);
    visuals.window_corner_radius = CornerRadius::same(RADIUS_LG);
    // Every egui menu and popup takes Material's menu shape (extra-small).
    visuals.menu_corner_radius = CornerRadius::same(RADIUS);
    visuals.faint_bg_color = ink(theme, INK_03);
    // Text fields are transparent, so they take whichever region's surface
    // they sit on, with the `input` outline doing the work, as shadcn's
    // transparent inputs do. Other wells (a progress track) are the
    // content surface.
    visuals.extreme_bg_color = surface_of(theme, Surface::Content);
    visuals.text_edit_bg_color = Some(Color32::TRANSPARENT);
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
    fn badges_on_footage_read_over_any_frame_in_every_theme() {
        let on = Rgb::new(ON_SCRIM.r(), ON_SCRIM.g(), ON_SCRIM.b());
        for frame in [
            Rgb::new(255, 255, 255),
            Rgb::new(250, 230, 120),
            Rgb::new(128, 128, 128),
            Rgb::new(0, 0, 0),
        ] {
            let plate = scrim_over(frame);
            assert!(
                on.contrast(plate) >= 4.5,
                "badge text over {} is {:.2}:1",
                frame.to_hex(),
                on.contrast(plate)
            );
        }
        let worst = scrim_over(Rgb::new(255, 255, 255));
        for theme in crate::theme::Catalog::built_in().themes() {
            for (name, color) in [
                ("warning", theme.warning),
                ("success", theme.success),
                ("danger", theme.danger),
                ("accent", theme.accent),
            ] {
                let icon = on_scrim(color);
                let icon = Rgb::new(icon.r(), icon.g(), icon.b());
                assert!(
                    icon.contrast(worst) >= 3.0,
                    "{}: the {name} badge icon is {:.2}:1 on the scrim",
                    theme.id,
                    icon.contrast(worst)
                );
            }
        }
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
    fn every_region_has_its_own_surface_role_in_both_modes() {
        for dark in [true, false] {
            for (i, a) in Surface::ALL.iter().enumerate() {
                for b in &Surface::ALL[i + 1..] {
                    assert_ne!(
                        a.role(dark),
                        b.role(dark),
                        "{a:?} and {b:?} share a role (dark: {dark})"
                    );
                }
            }
        }
    }

    #[test]
    fn neighbouring_regions_read_apart_in_every_theme() {
        for theme in crate::theme::Catalog::built_in().themes() {
            let dark = theme.mode == Mode::Dark;
            for (a, b) in Surface::NEIGHBOURS {
                assert_ne!(a.role(dark), b.role(dark), "{a:?} / {b:?} share a role");
                assert_ne!(
                    surface_of(theme, a),
                    surface_of(theme, b),
                    "{}: {a:?} and {b:?} are the same colour",
                    theme.id
                );
            }
            // The overlay floats over any region, so it differs from all.
            for region in Surface::ALL {
                if region != Surface::Overlay {
                    assert_ne!(
                        surface_of(theme, region),
                        surface_of(theme, Surface::Overlay),
                        "{}: a popover vanishes on {region:?}",
                        theme.id
                    );
                }
            }
        }
    }

    #[test]
    fn text_reads_at_aa_on_every_region_in_every_theme() {
        for theme in crate::theme::Catalog::built_in().themes() {
            for region in Surface::ALL {
                let fill = surface_of(theme, region);
                let fill = Rgb::new(fill.r(), fill.g(), fill.b());
                // Faint text (times, units, steps not reached) lives on the
                // docked regions; menus and the island use muted text, as
                // Material's trailing text and icons do.
                let faint = !matches!(region, Surface::Island | Surface::Overlay);
                let inks = [
                    ("text", theme.text, true),
                    ("muted text", theme.text_muted, true),
                    ("faint text", theme.text_faint, faint),
                ];
                for (what, ink, _) in inks.into_iter().filter(|(_, _, checked)| *checked) {
                    let ratio = ink.contrast(fill);
                    assert!(
                        ratio >= crate::theme::SECONDARY_CONTRAST,
                        "{}: {what} is {ratio:.2}:1 on {region:?}",
                        theme.id
                    );
                }
            }
        }
    }

    #[test]
    fn the_focus_ring_stands_out_on_every_region_in_every_theme() {
        let rgb_of = |c: Color32| Rgb::new(c.r(), c.g(), c.b());
        for theme in crate::theme::Catalog::built_in().themes() {
            let ring = rgb_of(material_scheme(theme).get(ColorRole::Secondary));
            for region in Surface::ALL {
                let ratio = ring.contrast(rgb_of(surface_of(theme, region)));
                assert!(
                    ratio >= crate::theme::INDICATOR_CONTRAST,
                    "{}: the focus ring is {ratio:.2}:1 on {region:?}",
                    theme.id
                );
            }
        }
    }

    #[test]
    fn the_regions_step_back_to_front_in_the_material_themes() {
        // A rough lightness: the channel sum, enough to order greys.
        let lightness = |c: Color32| u32::from(c.r()) + u32::from(c.g()) + u32::from(c.b());
        for dark in [true, false] {
            let theme = crate::theme::material(dark);
            let levels: Vec<u32> = Surface::ALL
                .into_iter()
                .map(|region| lightness(surface_of(&theme, region)))
                .collect();
            for (back, front) in levels.iter().zip(levels.iter().skip(1)) {
                let forward = if dark { back <= front } else { back >= front };
                assert!(
                    forward,
                    "dark {dark}: the levels {levels:?} do not step forward"
                );
            }
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

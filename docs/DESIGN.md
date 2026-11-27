# Design system

> **Being replaced.** The visual theme is moving to the `egui-material3` crate
> (Material 3). This page describes the current theme and will be superseded
> once that lands.

The look of the desktop app: Selects' dark shadcn workspace, rebuilt in
egui. This page is for anyone drawing a panel or page. The tokens and
components live in code; this says which to use for what.

| Where | What |
|---|---|
| `src/gui/ui/theme.rs` | tokens: spacing, radii, sizes, type scale, ink overlays, fills, egui `Visuals` |
| `src/theme/mod.rs` | the palette (`signature` is Selects' dark theme; other themes derive the same way) |
| `src/gui/font.rs` | bundled faces and the weight helper |
| `src/gui/ui/icons.rs` | Lucide icons by name |
| `src/gui/ui/widgets.rs` | the components |

The values are Selects' own, measured from its renderer: see
`docs/research/07-selects-visual-chat-project.md` and
`docs/research/07-selects-visual-viewer-timeline.md` for the per-region
specs (they win over `05-selects-ui.md` §8 where the two differ).

## Rules for page owners

1. **Start every panel with `widgets::panel_header`** (or `panel_bar` when
   the left side is several tabs or a menu). It is 40 px with a hairline
   under it, bleeds across the panel inset to the panel's edges, and lines
   up with every other panel's header. **No bold titles**: the title is the
   panel's open tab, a 28 px pill in 12 px Medium on ink 12 %. Never draw
   your own title row.
2. **Icons, not glyphs.** Use `icons::*` constants; never type `▶ ✕ ⚙ ▾ ⌕
   ✔ ★` or any other symbol. An icon-only control is `widgets::icon_btn` or
   `widgets::icon_toggle`, never a text button with a symbol in it.
3. **No hint lines.** Do not put a line of muted "Click a shot · Shift-click
   to add · …" text under a control or along a panel's foot. Put the hint in
   the control's tooltip (`widgets::tip(response, theme, label, keys)`), with
   its shortcut as `kbd` chips. Shortcuts are listed on the `?` sheet.
4. **Empty states via `widgets::empty_state`**: a quiet centred column (icon,
   a short title, one sentence, the action that fills it), not a paragraph.
5. **Selection is a neutral fill, not a colour.** Selected tabs, toggles and
   items use `theme::selected_fill` (ink 12 %); a selected list row uses
   `theme::selected_row` (`#525252`); hover is ink 8 %. **Lime (`key`) is
   only for** mentions, the focused selection's outline, the active
   playhead, switches, unread dots, and the one key button of a view.
6. **Colours from the palette and the helpers only.** No `Color32::from_rgb`
   for chrome. Hover fills and dividers are ink (`theme::ink(theme,
   theme::INK_08)`), not new greys.
7. **Sizes from tokens only.** Spacing is the 4 px grid (`SPACE_*`); panel
   controls are `CONTROL_HEIGHT` (28), top-bar and transport controls
   `BAR_CONTROL` (36); corners are one of the radii below.
8. **Text sizes:** chrome is 12 px (Medium for labels and tabs); 14 px only
   for message bodies, the composer, list-row names and menu items; numbers
   and timecode in `theme::tabular`. Nothing in a panel header is bold or
   larger than 12.
9. **Panels are flat.** No shadows, no gaps, no card around a panel's body.
   Only floating things carry a shadow (menus, popovers) or the glass recipe
   (anything over footage).
10. **Default egui widgets are already styled** (outline buttons, combo
    boxes, text fields, menus, scrollbars). Use them freely; reach for a
    component below when you need what it adds.

## Tokens

### Colour (signature theme)

| Selects token | Value | Palette / helper | Use |
|---|---|---|---|
| `tone-950` | `#0a0a0a` | `theme.app` | the window; the composer box; the home screen |
| (top bar) | `#0a0a0a` @ 20 % over the panels ≈ `#141414` | `theme::bar_fill` | the top bar |
| `background`, `card` | `#171717` | `theme.panel` | every panel, the viewer, the status bar |
| `popover`, `secondary`, `muted` | `#262626` | `theme.raised`, `theme::popover_fill` | menus, tooltips, popovers, segmented tracks, secondary buttons, list hover |
| `accent` | `#404040` | `theme::accent_fill` | ghost-button and menu-item hover |
| selected (ink 12 %) | ≈ `#333333` | `theme::selected_fill`, `theme.selection` | open tab, toggle on, selected item |
| `sidebar-ring` | `#525252` | `theme::selected_row` | a selected list row |
| `border` | white @ 10 % ≈ `#2e2e2e` | `theme::ink_line` | every hairline: panel separators, header underline |
| `input` | white @ 15 % | `theme::input_line` | field and outline-button outlines |
| `foreground` | `#fafafa` | `theme.text` | text |
| `muted-foreground` | `#a3a3a3` | `theme.text_muted` | secondary text, idle icons, placeholders |
| (faint) | — | `theme.text_faint` | units, version, timings |
| `primary` | `#e5e5e5` / text `#171717` | `theme::primary_fill`, `theme::primary_text` | primary button |
| `key` | `#a3e635` | `theme.accent` | see rule 5 |
| `key-foreground` | `#171717` | `theme.text_on_accent` | text on the key colour |
| `destructive` | `#ef4444` | `theme.danger` | delete, errors, the blade line |
| `ring`, `tone-500` | `#737373` | `theme::ring`, `theme::panel_focus` | the focused panel's outline, a focused field |
| glass | `#262626` @ 85 % + white 10 % border | `theme::glass_fill`, `widgets::glass_frame` | floating over footage (Selects uses 50 % + 16 px blur; egui has no blur) |

The `hover` palette surface is `#3a3a3a`, a step short of `#404040`,
because muted text must clear 4.5:1 on every surface; the `accent` *fill*
is still exactly `#404040`.

### Ink

Overlays of the text colour, so one value reads the same on any surface
(Selects' `rgb(var(--ink) / …)`): `theme::ink(theme, alpha)`.

| Constant | Opacity | Over `#171717` | Use |
|---|---|---|---|
| `INK_03` | 3 % | | the quietest wash |
| `INK_05` | 5 % | `#232323` | outline-button and field fill |
| `INK_08` | 8 % | `#2a2a2a` | hovered tab or row (`INK_HOVER`), `kbd` chip |
| `INK_10` | 10 % | `#2e2e2e` | hairlines (`INK_LINE`), glass hover |
| `INK_12` | 12 % | `#333333` | selected tab/toggle, header icon-button hover |
| `INK_15` | 15 % | `#3a3a3a` | input outline (`INK_STRONG`) |
| `INK_20` | 20 % | `#454545` | pressed, a switch that is off |

### Type

Mona Sans for the UI, Inter for numbers and timecode (Selects'
`font-tabular`), Roboto Mono for code and query tokens, Lucide for icons.
egui has no weights, so each weight is a family: `theme::medium(ctx,
size)`, `theme::semibold(ctx, size)`, `theme::tabular(ctx, size)`. They fall
back to Regular on the first frame, before the fonts are installed, instead
of panicking. egui cannot switch on Inter's `tnum`, so a running clock
should lay digits out in fixed cells (or use `theme::mono`).

| Constant | Size | Selects | Use |
|---|---|---|---|
| `TINY_SIZE` | 10 | `text-3xs` 10/12 | `kbd` chips, meter scales |
| `SMALL_SIZE` | 11 | `text-2xs` 11/16 | captions, units, badges, status bar |
| `BODY_SIZE` | 12 | `text-xs` 12/16 | **default chrome**: labels, tabs (Medium), buttons (SemiBold) |
| `PANEL_TITLE_SIZE` | 12 Medium | `text-xs font-medium` | the title pill |
| `LARGE_SIZE` | 14 | `text-sm` 14/20 | message bodies, composer, list-row names, menu items, the top bar's title and search |
| `TITLE_SIZE` | 14 Medium | `text-sm font-medium` | card titles |
| `HEADING_SIZE` | 18 SemiBold | `text-lg` | page headings (rare) |

### Spacing, sizes, radii

| Token | Value | Use |
|---|---|---|
| `SPACE_XS` / `SM` / `MD` / `LG` / `XL` | 4 / 8 / 12 / 16 / 24 | every gap and margin |
| `PANEL_INSET` | 8 | a docked panel's inset (7 + the 1 px focus outline) |
| `PANEL_HEADER_HEIGHT` | 40 | every panel header (`h-10`), 6 px side padding, 2 px gaps |
| `HEADER_HEIGHT` | 52 | the top bar |
| `STATUS_HEIGHT` | 24 | the status bar |
| `BAR_CONTROL` | 36 | top-bar buttons and search pill, transport buttons (`h-9`) |
| `CONTROL_HEIGHT` | 28 | panel controls: tab pills, icon buttons, `xs` buttons, fields (`h-7`) |
| `COMPACT_HEIGHT` | 24 | chips, small toggles (`h-6`) |
| `CHAT_RAIL` | 44 | the folded chat's icon rail (`w-11`) |
| `RADIUS_XS` | 2 | thumbnails |
| `RADIUS` | 6 | tab pills, menu items, chips, badges (`rounded-sm`) |
| `RADIUS_MD` | 8 | buttons, icon buttons, fields, menus, tooltips (`rounded-md`) |
| `RADIUS_LG` | 10 | cards, the user's message bubble (`rounded-lg`) |
| `RADIUS_ISLAND` | 12 | glass islands and buttons |
| `RADIUS_XL` | 14 | the composer, option cards (`rounded-xl`) |

## The shell (what the workspace already does)

- **Panels touch**: one 1 px white-10 % separator between neighbours, no
  gap; each is `#171717` with a 1 px outline that is transparent, or
  `#737373` on the focused panel (the last one pressed in).
- **Top bar**: 52 px, `bar_fill`, hairline under; home and the 14 px Medium
  night name on the left; the 36 px search pill centred; panel toggles,
  view mode (`PANELS_LEFT_BOTTOM`), preferences (`SETTINGS_2`) and Export
  (a filled secondary `SHARE` square) on the right.
- **Folded panels**: the chat leaves a 44 px rail with one 32 px
  `MESSAGE_SQUARE` button; the browser and inspector leave **no rail** but a
  glass button in the viewer's top-left / top-right corner (6 px inset).
  **Viewer owners: keep the top 48 px of the two corners free of controls
  that matter.** The timeline folds to a 24 px labelled rail.
- **Status bar**: 24 px, panel colour, hairline above, 11 px text.
- **Which panel is focused**: `workspace::panel_focused(ctx, key)` says
  whether the panel `key` (`"timeline"`, `"viewer"`, …) took the last
  pointer-down. A panel that paints a selection asks it to pick the
  selection outline: the key colour when focused, white otherwise.

## Home and export pieces (`pages/night/parts.rs`)

Compositions of the tokens and components above that only the Home pages
(start, wizard, processing, night page) and the export panel use, so they
live beside them rather than in `widgets`. Reach for them before drawing
your own on those pages.

| Need | Use | Notes |
|---|---|---|
| a centred column | `parts::centred(ui, max, \|ui\| ..)` | at most `max` wide, centred in the space |
| a quiet label over a group | `parts::label(ui, theme, text)` | 12 px Medium, muted |
| a section | `parts::section(ui, theme, title, description)` | hairline above, 14 px Medium title, one muted sentence |
| a muted line | `parts::muted(ui, theme, text)` | 12 px, wraps |
| a form row | `parts::form_row(ui, theme, label, \|ui\| ..)` | 12 px muted label in a `LABEL_WIDTH` (132) column, the control beside it, one 28 px line |
| a read-only path | `parts::path_box(..)` | a field cut from the left to fit, leading icon, amber when the file is gone, full path as tooltip |
| run progress | `parts::progress_bar(ui, theme, id, fraction)`, `parts::paint_progress(ui, theme, rect, id, fraction)` | Material's linear progress indicator (`paint_at`), 8 px, ink 80 % on ink 10 % (not the key colour); eases |
| a notice in the column | `parts::notice(ui, theme, Kind::Warning, text)` | Material's banner in the kind's tone, the Lucide icon, a dismiss button; returns whether dismissed. `Kind::{Info, Success, Warning, Danger}` |
| one status line | `parts::status_line(ui, theme, kind, text)` | icon and text in the kind's colour |
| a step's mark | `parts::mark(ui, theme, rect, Mark::Running)` | `Mark::{Queued, Running, Done, Failed, Stopped, Skipped}`; a spinner while running |
| a frame | `parts::thumb(thumbs, ui, theme, rect, Some((path, t)))` | 2 px corners, ink 10 % hairline; a raised fill when there is no frame |
| a list row's fill | `parts::row_fill(ui, theme, &response, selected)` | selected row colour, or ink 8 % under the pointer, 8 px corners |
| elapsed / remaining time | `parts::clock(secs)` | `1:05`, `12:40`, `1:02:03` |

## Components: which to use for what

| Need | Use | Notes |
|---|---|---|
| a panel's top row | `panel_header(ui, theme, "Title", \|ui\| { right-side controls })` | title drawn as the open tab pill; right side laid out right to left |
| a panel's top row with several views | `panel_bar(ui, theme, \|ui\| { tab_strip(..); }, \|ui\| { .. })` | |
| views of one panel | `tab_strip(ui, theme, &mut current, &[(A, "A"), (B, "B")])` | 28 px pills, 12 px Medium, open one ink 12 % |
| a toolbar under a panel header | `toolbar_row(ui, height, &mut cx, \|ui, cx\| { left }, \|ui, cx\| { right })` | borderless, 2 px between items; `right` is laid out first (right to left) so it always fits, `left` gets the rest, clipped short of it; both sides share `cx` |
| a menu trigger that names its current choice | `dropdown_button(ui, theme, "Best", "Sort by", open)` | ghost, 28 px, 12 px Medium, muted until hovered or `open`, trailing chevron; hand the response to `egui::Popup::menu` |
| an icon-only action | `icon_btn(ui, theme, icons::X, "Close", keys)` | egui-material3's standard icon button, 28 px, square shape, the Lucide glyph via `Icon::glyph` (`widgets::lucide`); hit area clipped to the visible square; tooltip with `kbd` keys; `keys` may be `""` |
| an icon toggle | `icon_toggle(ui, theme, glyph, label, keys, on)` | Material's toggle icon button (primary icon when on, toggled state for screen readers); `icon_toggle_sized(.., size)` for 24/32/36 px |
| a text action | `ghost_button`, `outline_button`, `primary_button`, `key_button` | 28 px, SemiBold 12, radius 8 |
| a text action with an icon, or another variant or size | `button_with(ui, theme, ButtonKind::Outline, Some(icons::PLUS), "Add")`, `button_sized(.., 36.0)` | `ButtonKind::{Ghost, Outline, Secondary, Primary, Key, Destructive}`; empty label = square icon button |
| a mode, a small option set | `segmented(ui, theme, &mut current, &[..])` | muted track, raised pill |
| a status, a count | `pill(ui, theme, "3", Tone::Neutral)` | Material's labelled badge on the tone's container colour (`MaterialTheme::success()`/`warning()` for those tones); `Tone::{Neutral, Key, Success, Warning, Danger}` |
| a shortcut | `kbd(ui, theme, "Ctrl+B")` | one chip per key |
| a hint | `tip(response, theme, label, keys)` | Material's plain tooltip, keys as chips; `rich_tip(response, name, \|ui\| ..)` for a several-line preview (Material's rich tooltip) |
| nothing to show | `empty_state(ui, theme, icons::FILM, "No shots yet", "Index the night to fill this.", \|ui\| { .. })` | centred in the space left |
| a single-line field | `text_field(ui, theme, &mut text, "Placeholder")`, `input(&mut text).label(..).show(ui)` | Material's outlined field at 32 px; `field_frame` only for a hand-placed field in a 24/28 px row or with a Lucide leading icon |
| a number | `ui.add(number(&mut value, "Name").range(..).suffix(" s"))` | Material's number field, 32 px: drag, Up/Down steps, typing |
| a list of rows | `egui_material3::MaterialList` with `ListItem::content_ui` | its own state layer, selected fill, arrow keys and screen-reader names; `row_sense(Sense::drag())` for rows that drag |
| a menu | `ui.menu_button(..)` / `egui::Popup::menu(&response)` with `menu_item`, `menu_heading`, `menu_separator` | items 14 px, 32 tall, `accent` hover, check when current |
| a custom popover | `popover_frame(theme).show(ui, ..)` | popover fill, hairline, radius 8, shadow |
| something floating over footage | `glass_frame(theme)`, `glass_button(..)` | radius 12, glass fill, white-10 % border |
| on/off | `switch(..)` | 32 × 18 track, key when on |
| a divider in a bar | `bar_divider(ui, theme)` | |
| a card | `card(..)`, `card_grid(..)` | |

Older helpers still work and now draw in the new style: `button(.., Emphasis)`
(`Primary` is a key button, `Secondary` an outline button), `tab`,
`icon_button` / `toggle_button` (caller paints into `Slot::icon`), `title`,
`muted`, `rail`. Prefer the table above in new code.

## Icons

`src/gui/ui/icons.rs`, Lucide 1.60.0 (`lucide-static`, ISC). Each constant
is a `char`, so it can be drawn alone (`icons::icon(ui, glyph, size,
colour)`, `icons::paint(painter, rect, glyph, size, colour)`) or inline in
text (`format!("{} Export", icons::DOWNLOAD)`, or `icons::with_label`).
Sizes: `icons::SIZE` 16 (default, in 28–36 px buttons), `icons::SMALL` 14
(menus, chips, beside 12 px text), `icons::LARGE` 24.

| Area | Constants |
|---|---|
| playback | `PLAY`, `PAUSE`, `SQUARE` (stop), `SKIP_BACK`, `SKIP_FORWARD`, `STEP_BACK`, `STEP_FORWARD`, `REPEAT`, `VOLUME`, `VOLUME_X`, `MAXIMIZE`, `MAXIMIZE_2`, `MINIMIZE_2` |
| editing | `SCISSORS`, `SPLIT`, `POINTER`, `HAND`, `MAGNET`, `MOVE_HORIZONTAL`, `ARROW_LEFT_RIGHT`, `UNDO`, `REDO`, `ZOOM_IN`, `ZOOM_OUT`, `CROP`, `RATIO`, `LANDSCAPE`, `PORTRAIT`, `SMARTPHONE`, `MONITOR` |
| media | `FILM`, `VIDEO`, `CLAPPERBOARD`, `IMAGE`, `FILE_VIDEO`, `FILE_TEXT`, `MUSIC`, `AUDIO_LINES`, `AUDIO_WAVEFORM`, `HEADPHONES` (solo a track), `CAPTIONS`, `TYPE`, `LAYERS`, `FLAME`, `ZAP` |
| people and review | `USERS`, `USER`, `STAR`, `FLAG`, `HEART`, `THUMBS_UP`, `THUMBS_DOWN`, `TAG`, `BOOKMARK`, `PIN`, `EYE`, `EYE_OFF`, `LOCK`, `LOCK_OPEN`, `GAUGE` |
| chat and AI | `MESSAGE_SQUARE`, `MESSAGE_SQUARE_TEXT`, `SEND`, `SEND_HORIZONTAL`, `ARROW_UP`, `PAPERCLIP`, `AT_SIGN`, `SPARKLES`, `SPARKLE`, `WAND`, `BOT` |
| layout | `PANEL_LEFT`, `PANEL_RIGHT`, `PANEL_BOTTOM`, `PANEL_LEFT_CLOSE`, `PANEL_LEFT_OPEN`, `PANEL_RIGHT_CLOSE`, `PANEL_RIGHT_OPEN`, `PANEL_BOTTOM_CLOSE`, `PANEL_BOTTOM_OPEN`, `PANELS_LEFT_BOTTOM`, `GRID`, `LAYOUT_GRID`, `LIST`, `COLUMNS`, `COLUMNS_3`, `COMPARE` |
| navigation | `HOME`, `MENU`, `FOLDER`, `FOLDER_OPEN`, `SEARCH`, `FILTER`, `LIST_FILTER`, `ARROW_UP_DOWN`, `HISTORY`, `CHEVRON_DOWN`, `CHEVRON_UP`, `CHEVRON_LEFT`, `CHEVRON_RIGHT`, `CHEVRONS_UP_DOWN`, `EXTERNAL_LINK` |
| actions | `PLUS`, `MINUS`, `X`, `CHECK`, `COPY`, `PEN`, `PENCIL`, `TRASH`, `DOWNLOAD`, `UPLOAD`, `SHARE`, `REFRESH`, `ROTATE_CCW`, `ELLIPSIS`, `ELLIPSIS_VERTICAL`, `SLIDERS`, `SETTINGS`, `SETTINGS_2`, `COMMAND`, `KEYBOARD` |
| status | `CIRCLE`, `CIRCLE_CHECK`, `CIRCLE_X`, `CIRCLE_ALERT`, `CIRCLE_DOT`, `CIRCLE_PLAY`, `TRIANGLE_ALERT`, `INFO`, `LOADER`, `CLOCK`, `TIMER`, `ACTIVITY`, `SUN`, `MOON` |

To add one: find the name on lucide.dev, look its codepoint up in
`https://unpkg.com/lucide-static@1.60.0/font/codepoints.json`, add the
constant (with its Lucide name in the doc comment) and a cell above. The
whole font is bundled, so any Lucide icon of that version is available.

## Fonts and licences

`assets/fonts/NOTICE` lists every bundled font, its source and licence;
the licence texts sit beside the fonts and ship in the release zip. Fonts
are not crates, so `cargo deny` does not see them: adding one means adding
its licence text and a NOTICE entry in the same commit.

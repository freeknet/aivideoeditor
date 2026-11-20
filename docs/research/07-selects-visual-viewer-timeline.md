# 07 — Selects: visual spec for the viewer, side panel and timeline

A pixel-level spec for rebuilding the look of the Selects (Cutback v2.1.301)
editor's Preview, right side panel and Timeline in egui. Companion to
`05-selects-ui.md` (what the panels do) and `06-selects-data.md`.

Sources: the renderer chunks recovered from the app's HTTP cache
(`TimelineRoute`, `ReadOnlyTranscription`, `TranscriptViewSnapshotContext`,
`expandedMulticamLaneLayouts`, `ExpandedMulticamClipView`, `markerPaint`,
`clipLabelColors`, `clipTimelineLabel`, `main`) and the Tailwind token sheet
`global-*.css`. Everything here is paraphrased. Class names are quoted only as
short evidence. Values are for the **dark theme**, which is the default. Light
theme notes are given where the code branches.

**The single most important fact:** the timeline is not DOM. It is a PixiJS
canvas (`<timeline-renderer>`) that paints ruler, clips, waveforms, playhead,
markers and track headers itself, with constants in JS. Only the tab strip,
the floating tool island, the scrollbars, the zoom buttons and the audio
meter are HTML. That maps well onto egui: paint the timeline with a `Painter`
in one custom widget, and build the chrome with ordinary widgets.

---

## 0. Tokens used everywhere

Tailwind v4, 4 px spacing unit. Radii are **not** stock Tailwind: the sheet
redefines them.

| Token | Value |
|---|---|
| `rounded-xs` | 2 px |
| `rounded` | 4 px |
| `rounded-sm` | 6 px |
| `rounded-md` | 8 px |
| `rounded-lg` | 10 px |
| `rounded-xl` | 14 px |
| `rounded-2xl` | 16 px |
| `rounded-[12px]` | 12 px (floating "islands") |

Type scale (size / line height): `text-3xs` 10/12, `text-2xs` 11/16,
`text-xs` 12/16, `text-sm` 14/20, `text-base` 16/24, `text-lg` 18/28.

Fonts: **Mona Sans** for UI text, **Inter with tabular numerals**
(`font-tabular`) for every timecode and number, Roboto Mono for code.

Dark colours (all hex resolved; there is no oklch in this build, colours are
`rgb(var(--x))` triplets):

| Name | Hex | Use |
|---|---|---|
| background / card | `#171717` | panels, timeline canvas, toasts |
| popover / secondary / muted | `#262626` | menus, inspector fields |
| accent (hover fill) | `#404040` | ghost-button hover |
| border | white @ 10% = `#2E2E2E` on bg | hairlines |
| input border | white @ 15% | inputs |
| foreground | `#FAFAFA` | primary text |
| muted-foreground | `#A3A3A3` | secondary text, inactive icons |
| primary (button fill) | `#E5E5E5` on `#171717` text | default buttons |
| **key** (accent) | lime `#A3E635`, key-foreground `#171717` | selection, active playhead, toggles |
| destructive | `#EF4444` | errors, blade line |
| ring | `#737373` | focus |

The `tone-*` scale is a theme-flipping neutral ramp. In dark it equals
Tailwind neutral: tone-50 `#FAFAFA`, 100 `#F5F5F5`, 300 `#D4D4D4`,
400 `#A3A3A3`, 500 `#737373`, 600 `#525252`, 700 `#404040`, 800 `#262626`,
900 `#171717`, 950 `#0A0A0A`. In light it inverts (tone-900 = white).

`ink` is white in dark and near-black (`#0A0A0A`) in light; hover and
divider fills are `ink` at a low alpha, not separate greys:
ink/5 `#232323`, ink/8 `#2A2A2A`, ink/10 `#2E2E2E`, ink/12 `#333333`,
ink/15 `#3A3A3A`, ink/20 `#454545` (all as composited on `#171717`).

The key colour is user-selectable (`data-key-color`): lime (default), white,
pink `#F472B6`, red `#F87171`, orange `#FB923C`, yellow `#FACC15`,
green `#4ADE80`, cyan `#22D3EE`, blue `#60A5FA`, violet `#A78BFA`,
fuchsia `#E879F9`. Make it one variable.

Panels are flat: no shadows, 1 px hairlines. A panel that has focus gets a
**1 px `tone-500` (`#737373`) border** around the whole panel; unfocused
panels keep a transparent 1 px border so nothing shifts.

### Shared primitives

- **Panel header bar:** 40 px tall (`h-10`), bottom border 1 px border
  colour, horizontal padding 6 px, items gap 2 px. Every panel header
  (Timeline tabs, side panel tabs) uses this.
- **Icon button (transport):** 16 px icon in 10 px padding = 36x36,
  corner radius = icon size / 2 = 8 px, hover fill ink/10, 200 ms colour
  transition.
- **Button sizes:** default 36 h, `sm` 32 h text-xs, `xs` 28 h text-xs,
  `icon-sm` 32x32, `icon-xs` 28x28, `icon-2xs` 20x20 radius 6. Font weight
  600 by default; `quiet` and `text` variants are 400.
- **Switch (menu rows):** track 32x18 fully rounded; on = key, off = ink/20;
  knob 14 px, colour tone-900, x = 2 px off / 16 px on.
- **Tooltip:** `bg-secondary` (`#262626`), radius 8, padding 12x6, text-xs,
  text colour primary.
- **Glass island:** padding 4, radius 12, backdrop blur 16 px, fill
  tone-800 @ 50%, 1 px border ink/10. Used for the timeline tool island and
  floating panel buttons. In egui: fill `#262626` at ~80% alpha (no blur).
- **Empty state:** centred column, gap 12, padding-x 24, text-xs with
  20 px line height, muted-foreground; optional second line at
  muted-foreground @ 70%; optional action underneath.

---

## 1. Preview (viewer) panel

### 1.1 Layout

There is **no header bar** on the Preview. The panel is the video surface
plus a 48 px transport strip at the bottom. Aspect, guides, safe margins and
zoom live in a menu on the right of the transport, not above the video.

```
┌──────────────────────────────────────────────────────────┐
│[▣ Project]  ← glass button, only when Project panel folded│
│                                                          │
│                  video surface (forced dark)             │
│               letterboxed, 4 px bottom padding           │
│                                    [Preview quality ↓]   │
├══════════════════════════════════════════════════════════┤ ← 4 px scrub bar
│ ▶  1x  ⧉          00:00:12:05 / 00:03:41:10      ◫ ⚙ ⌕   │ 48 px
└──────────────────────────────────────────────────────────┘
```

- Video area: flexes to fill, always rendered in the dark palette even in
  light theme (`dark` class), 4 px padding below the picture.
- A loading spinner over the video: 20 px spinner in a black @ 45% circle
  with 8 px padding, centred.
- "Preview quality reduced" badge: bottom 12, right 8, radius 4,
  black @ 20% with blur, padding 8x4, text-xs, ink @ 50%.
- When the Project panel is folded, a glass button sits at top 6, left 6:
  radius 12, padding 10x8, 16 px folder icon + "Project" in text-sm 500,
  tone-50 text, hover ink/10.
- When a source clip is open, a **source strip** (a second, mini timeline)
  animates in under the video: 113 px tall (spring animation to 114),
  track height 30, markers on, bad-shot overlay on. It is the same canvas
  renderer as section 3.

### 1.2 Scrub bar

- Hit area 20 px tall, overlapping the bottom of the video (it is pulled up
  by its own height, so it adds no layout height). The visible bar sits at
  the bottom edge of that hit area, i.e. on the top edge of the transport.
- Bar: **4 px tall, 6 px on hover** (150 ms), full width, square ends.
- Track `tone-700` `#404040`; played portion solid **ink (white)**. There is
  **no thumb/knob**.
- Timeline markers are drawn on the bar as 2 px wide full-height ticks in
  the marker colour (see 3.9).
- Drag starts after a small pointer threshold; playback pauses while
  scrubbing and resumes after.

### 1.3 Transport row

48 px tall, horizontal padding 8. Three zones; the timecode is absolutely
centred so the side groups never push it.

Left group (gap 2 px):
1. **Play/Pause** — 36x36 icon button, radius 8; 16 px solid triangle or
   two bars, colour tone-50 (`#FAFAFA`).
2. **Speed select** — 36 px tall, min width 36, padding-x 8, no border,
   text-xs muted-foreground showing e.g. `1x`; hover ink/10 and text
   tone-50. Opens a plain list of speeds suffixed `x`, no check marks.
3. **Compare with source** toggle (draft timelines only) — 36x36 icon
   button, 16 px icon; muted-foreground, **key colour when on**.

Centre: **timecode** `HH:MM:SS:FF / HH:MM:SS:FF` (placeholder
`00:00:00:00`), text-sm (14/20), weight 400, Inter tabular, muted-foreground.
The current time is a borderless text button (tooltip "Playhead position");
clicking turns it into an in-place input of the same width, foreground
colour, Enter commits, Escape cancels. Accepts absolute and `+`/`-` relative
entries. Drop-frame uses `;` before frames at 29.97/59.94.

Right group:
- **Frame actions** menu (32x32 ghost, muted, hover foreground): Mark frame,
  Snapshot frame (with shortcut chip), "Include overlays" switch row. Menu
  224 px wide, padding 4, text-sm.
- **Preview settings** (gear, 36x36, muted → tone-50 on hover). Opens a menu
  above, aligned right: min width 220, radius 10, 1 px ink/10 border,
  popover fill, padding 4, shadow-md. Rows are 36 px, radius 8, padding-x
  10, text-sm tone-50, hover ink/10, label left and a switch right:
  "Snap to frame, clips and guides", "Rulers", "Guides", "Safe margins";
  a 1 px ink/10 divider (margin 4); then "Zoom" with a segmented control
  `75% | 100% | 150%` on an ink/5 track.
- **Track view** grid menu (synced timelines only): 1 track / 2x2 / 3x3 and
  "Hide empty tracks".
- **Search** button.

Frame size (aspect) editor, opened from the same area: 224 px panel, title
"Frame size" in 500 weight, then 36 px rows (radius 8, hover and pressed
ink/10) each with a small aspect glyph: Original (WxH), 3840x2160,
1920x1080, 1080x1920, 1080x1080, 1440x1080, 1080x1440, then "Custom" with a
chevron that reveals two 32 px numeric inputs (radius 4, 1 px ink/10
border, fill tone-950 @ 25%).

There is **no volume slider in the transport**. Clip volume is an Adjust
section (dB) and the mix level is the timeline audio meter (3.13).

---

## 2. Right side panel (Transcript | Adjust)

### 2.1 Tab header

The standard 40 px header bar. Tabs scroll horizontally with hidden
scrollbars.

```
┌───────────────────────────────────────────────┐
│ [Adjust] [× Transcript] [× App…]  +           │ 40 px, border-b
├───────────────────────────────────────────────┤
```

- Fixed tab (e.g. "Adjust"): 28 px tall, padding-x 10, radius 6, text-xs
  weight 500. Active: fill ink/12 (`#333333`), text foreground. Inactive:
  text muted-foreground, hover fill ink/8.
- Closable app tab (e.g. "Transcript"): same 28 px pill, radius 6, gap 6,
  left padding 6; a **close × on the left** (14 px glyph, muted, 2 px
  padding, opacity 30% → 100% on hover), then the label (max 128 px,
  truncated, right padding 10). Middle-click closes. While dragging a tab, a
  2 px x 20 px key-coloured insertion bar marks the drop slot.
- Tabs gap 2 px.
- **+ button**: 28x28 ghost, 16 px plus, colour tone-500 (`#737373`), hover
  muted-foreground. Opens a 256 px app picker.

### 2.2 Transcript body

Rows are a two-column grid: a 20 px speaker column and the words column,
gap 8 px, 2 px vertical padding per line.

```
 (2)  so the thing about the ⋯ new edit is that it
      [cursor-word] cuts straight to the good parts
 ──── Scene description: wide shot of the kitchen (italic, muted)
```

- **Speaker badge:** 20 px circle, text 12/16 weight 400, showing the
  speaker number (or the first letter of a custom name). Colour pairs cycle
  per speaker: text Tailwind 300 on bg Tailwind 950, in this order —
  green, pink, blue, orange, violet, teal, rose, sky, fuchsia, yellow,
  indigo, lime, red; unknown = tone-300 on tone-950. Example: green
  `#86EFAC` on `#052E16`. Badge sits 2 px down from the line top and shows
  only on the first line of a caption block.
- **Word:** 16 px Mona Sans, line height 24, each word an inline box 26 px
  tall with 1 px horizontal padding and a 1 px transparent border;
  6 px bottom margin between wrapped lines.
- Word states:
  - normal: foreground `#FAFAFA`.
  - cut (contains a cut piece) or "compare with source" off-draft: ink @ 30%.
  - **at playhead / selected:** fill **key** when the transcript panel is
    the active panel, else tone-400 `#A3A3A3`; text primary-foreground
    `#171717` (cut words: key-foreground @ 30%). A run of selected words
    forms one block whose **outer corners only** are rounded 2 px (computed
    per visual line).
  - silence/gap: `⋯` glyph, or with "show silence length" a pill 16 px
    tall, radius 8, 1 px current-colour border, text-xs frame count.
  - highlight repeats: fill `#FFA500` @ 27%.
  - annotations italic.
- Scene descriptions: full-width italic text-sm muted line,
  "Scene description: …", margin 4 vertical.
- Translation line: text-sm 14/20, ink @ 65%, 2 px left border violet-400
  @ 45%, 8 px left padding.
- Scrollbar: 12 px gutter, transparent track, thumb `#484848` fully
  rounded with a 2 px transparent border (so the visible thumb is 8 px).
- Empty states (empty-state primitive): "Transcript shows here";
  unanalysed clip → "Analyze to see the transcript" plus an outline `xs`
  button "Analyze file", or a 14 px spinner + "Analyzing…".

### 2.3 Adjust body (inspector)

Sections stacked vertically, separated by a 1 px divider with 4 px below.
Row padding is **16 px left, 8 px right** everywhere.

```
 Opacity                          ↺       ← section header, 40 px
 ┌ Opacity ─────────────── 100 % ┐        ← slider cell, 32 px
 Position                         ↺  ◆
 ┌ X ──────── 0 ┐ ┌ Y ──────── 0 ┐       ← number fields, 32 px
 Audio                            +       ← offered section (collapsed)
```

- **Section header:** 40 px tall, title text-xs weight 600 (foreground if
  editable, muted otherwise), "×N" count in tabular muted when several
  clips are selected, right-aligned 12 px icon buttons (reset ↺ or remove,
  keyframes ◆).
- **Offered (not yet applied) section:** a 40 px full-width button, title
  text-xs 600 muted, 12 px plus at the far right.
- **Field shell:** 32 px tall, radius 6, fill secondary `#262626`
  (`#404040` inside popovers, ink/8 on cards), padding-x 10, gap 6, no
  border; focus ring 1 px ring colour.
- **Number field:** label text-xs muted on the left (max 56% width,
  truncates, drag horizontally to scrub, `ew-resize` cursor), value
  right-aligned text-xs foreground tabular, unit text-xs muted after it.
  Mixed values show an italic placeholder "Mixed".
- **Slider cell:** 32 px, radius 6, same fill; label text-xs foreground
  padded 10 left; value + unit on the right padded 10; the filled portion is
  foreground @ 10% (15% hover, 20% while scrubbing) and a 1 px foreground
  hairline marks the value on hover. Double-click resets.
- Field rows stack with 4 px gaps; group headings are 32 px rows.
- Section body has 16 px bottom padding. A "Save as preset" button is a
  full-width 32 px default button, radius 6, text-xs.
- Empty: "No adjustable settings." / "Select a clip" via the empty-state
  primitive.

---

## 3. Timeline panel

### 3.1 Overall layout

```
┌────────────────────────────────────────────────────────────────────┐
│ [× ▤ Timeline 1] [× ▤ Selects]  +                                  │ 40 px tab bar
├──────────┬─────────────────────────────────────────────────────────┤
│          │▽ 00:00      00:05      00:10      00:15                 │ ruler 35 px
│          │|    '    '  |    '    ' |   ◆                           │ (ticks 16 px,
│          │ ◆ ← markers at y=16                                     │  markers below)
├──────────┼──┬──────────────┬─┬────────────────────────────┐────────┤
│ V1   👁  │  │▦▦ clip name   │ │▦▦ next clip                │  ║ 24px │
│          │  │▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦│ │▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦▦│  ║ vscroll
├══════════┼══════════════════ video/audio pane divider ═══════════════┤
│ A1 🔈 🎧 │  │∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿│ │∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿∿│  ║      │
│          │                                        ┌──┐            │
│          │                                        │↖ │ tool island│
│          │                                        │✂ │            │
│          │                                        │… │            │
├──────────┼────────────────────────────────────────┴──┴────────────┤
│ (96 px)  │ ◖━━━━━━━━━━━━━━━━━◗                       ⤡  −  +      │ h-scroll + zoom
└──────────┴─────────────────────────────────────────────────────────┘
   track headers                   canvas                    audio meter →
```

- Panel: background `#171717`, 1 px border (tone-500 when focused).
- Canvas background = background colour (`#171717`), antialiased, device
  pixel ratio aware.
- Content starts right of the **96 px** track-header column. The draft
  timeline uses horizontal padding 0 and vertical padding 24 above tracks.
- Default zoom: the draft opens at vertical zoom 0.7; vertical zoom range
  0.33–2.0; horizontal 0.03–14 px per frame.

### 3.2 Timeline tab strip

The 40 px header bar, padding-x 6, bottom border.

- Tab: **28 px tall, radius 8**, gap 6, padding 6 left / 10 right,
  full-width pill. Active: fill ink/12 (`#333333`), text and icon
  sidebar-accent-foreground `#FAFAFA`. Inactive: transparent, text and icon
  muted-foreground `#A3A3A3`, hover fill sidebar-accent `#262626`.
- Contents left to right: **close × first** (14 px glyph, muted, 2 px
  padding, radius 4, pulled 4 px into the next item, opacity 30% →
  100% on hover; always visible, not hover-only), a 16 px timeline glyph,
  the name in text-xs weight 500, truncated with ellipsis.
- Double-click renames in place (input 22 px tall, text-xs 500).
  Middle-click closes. Tabs reorder by drag. Tabs gap 2 px; the strip
  scrolls horizontally with hidden scrollbars.
- **+ button** after the tabs: 28x28, radius 8, 16 px plus, tone-500
  (`#737373`), hover muted-foreground with ink/10 fill. Opens a menu
  (min width 160): "New timeline", divider, then unopened timelines, or
  "All timelines open" in ink/40.

### 3.3 Tools: the floating tool island

There is **no horizontal toolbar row**. Tools live in a vertical glass
island floating inside the canvas, anchored **24 px from the right and 49 px
from the bottom** (just above the scroll bar).

- Island: glass island primitive (padding 4, radius 12, ink/10 border,
  tone-800 @ 50% with 16 px blur), items stacked with 1 px gaps.
- Buttons: **32x32, radius 8**, 16 px icons, colour tone-50 `#FAFAFA`,
  hover fill ink/10; the **active tool has a persistent ink/10 fill** (no
  key colour). Tooltips open to the left: "Selection (V)", "Razor (C)" etc.
- Items: Selection (arrow), Razor (blade), optional "See prompt", and a
  **More (…)** button that opens a menu to the left (min width 196): Zoom
  tool, Hand tool, Add marker, Add ▸ (adjustment layer, blank clip, media),
  divider, **Snap** row with a switch, **Multi-cam view** switch, divider,
  Shortcuts.
- Snapping toggles from the menu or its shortcut and announces itself with
  a toast "Snap is on" / "Snap is off".

### 3.4 Zoom control and scrollbars

There is **no zoom slider**. Zoom is the scroll thumb's length plus three
buttons.

- **Horizontal bar** (bottom): a strip with 1 px top border and 8 px
  padding. A left spacer equal to the header column width carries a 1 px
  right border. The thumb is **8 px tall, radius 10, `tone-700` `#404040`**,
  4 px side margins, with an 8 px round handle at each end; dragging a
  handle changes the visible width (that is the zoom); dragging the body
  scrolls; `grab` cursor. Clicking the empty track jumps.
- Right of it, gap 8, opacity 70%: **zoom to fit** (diagonal arrows),
  **−**, **+**, each a 16 px stroked icon. Each step changes the thumb width
  by 120 px.
- **Vertical bar** (right edge of the track area): a 24 px wide column,
  8 px padding, fill tone-900 @ 50% with 2 px blur, thumb **8 px wide,
  radius 10, tone-700**, 4 px top/bottom margin, round 8 px handles that set
  vertical zoom. Minimum thumb 20 px.

### 3.5 Ruler

Canvas-painted. The draft timeline sets the ruler to **35 px**; the tick
zone is the top **16 px**, and the space below it is the marker lane.

- Background: same as the canvas.
- **Labels every major step**, chosen as the first of 1, 2, 5, 10, 15, 30 s,
  1, 2, 5, 10, 15, 30 min, 1, 2, 6 h whose spacing is at least **80 px**.
  Labels are clock time, **`MM:SS`, or `HH:MM:SS`** once the step or time
  reaches an hour (not frame timecode).
- Major tick: 1 px vertical line, border colour `#2E2E2E`, from y=0 down
  16 px.
- Minor ticks: half height (**8 px**), same colour at 80% alpha, at a frame
  divisor (1, 2, 3, 4, 5, 6, 8, 10, 12, 15, 20, 30 frames) or a seconds
  multiple that divides the major step at least twice and is at least
  **12 px** apart.
- Label text: **Inter 12 px**, colour white @ 30% over background =
  **`#5D5D5D`**, drawn 8 px right of its tick, 2 px from the top.
- Clicking or dragging in the ruler seeks (playhead drag).

### 3.6 Track headers

A **96 px** column painted on the canvas, to the left of the clips. No
separate background fill; the canvas background shows through. All glyphs
and text are tinted muted-foreground `#A3A3A3`.

Layout is computed on a 76 px inner grid with 8 px outer padding and 2 px
column gaps, three columns of about 18.7 px. Centre x positions:

| Element | Video track | Audio track |
|---|---|---|
| Track label "V1" / "A1" | x ≈ 17 | x ≈ 17 |
| Eye (show/hide) | x ≈ 48 | — |
| Speaker (mute) | — | x = 38 |
| Headphones (solo) | — | x ≈ 59 |

- Label: **Inter 12 px**, centred vertically in the row. While the track is
  being dragged to reorder, the label drops to 35% alpha and a ghost chip
  (30x24, radius 6, background @ 92%, 1 px ink/20 border) follows the
  pointer.
- Icons: **16 px** line icons (eye / eye-off, speaker / speaker-x,
  headphones). Off state: eye-off and speaker-x at **50% alpha**; solo
  when not soloed at **30% alpha**.
- Hover: a square behind the icon, column width, **radius 8, ink @ 8%**.
  Tooltips: "Hide track", "Mute", "Solo" and so on. Click-drag across rows
  applies the same toggle to each row passed over.
- Lock icons exist in the texture set but the draft timeline shows no lock
  column; locking is a context-menu action ("Lock track"), and edits on a
  locked track raise a toast.
- Between the video and audio panes: a 2 px line in border colour with a
  6 px hit zone; it brightens on hover. Track-height resize happens by
  dragging row edges in the header column.

### 3.7 Clip blocks

Painted per clip. Geometry:

- **Default track height 60 px** at zoom 1 (42 px at the 0.7 opening
  zoom). Tracks are **4 px apart** (`TRACK_SPACING`). Topic/chapter band
  tracks are 16 px.
- A video clip that carries its own audio is drawn as **two stacked
  rounded rects**: the thumbnail part on top and the waveform part below,
  separated by a **2 px gap** (`LAYER_GAP`). The draft timeline instead
  splits embedded audio into its own row under the video pane, so most
  clips are single blocks: video rows show only thumbnails, audio rows only
  the waveform (waveform height 60 at zoom 1).
- **Corner radius 4 px.** Border **1 px**, drawn inside (inner radius 3.5).
- **1 px gap at the clip's right end** (width = floor(width) − border −
  1), so butted clips read as separate blocks.
- Minimum drawn width 8 px; below that a clip is just a filled sliver with
  no thumbnail, waveform or name.
- Colours: per clip label (3.8) if one is set, otherwise the defaults:
  video fill `#0E4B6F`, border `#075985`; audio fill and border `#075985`;
  synced sequences fill `#0C4A6E`; generated graphics fill `#4C1D95`,
  border `#7C3AED`. In light theme colours are re-toned in OKLab (fills to
  lightness 0.78, borders to 0.62, chroma clamped); in dark they are used
  as is.
- **Thumbnail strip:** fills the top part edge to edge, masked to the
  rounded rect. It is a filmstrip of frame sprites anchored to the clip's
  source time (so it does not swim while trimming); the clip fill shows
  behind until sprites load, and a load scheduler runs at most 6 at once.
  Thumbnails are skipped below 4 px clip width. (Exact sprite width was not
  extracted; matching the frame aspect at the lane height is the safe
  choice.)
- **Clip name:** **Mona Sans 11 px, white, 70% opacity**, at **x = 6,
  y = 4** from the clip's top-left (6 px inset on the right too), with a
  soft drop shadow (black 85%, blur 2, 1 px down). Ellipsised to fit; hidden
  if the clip is shorter than the text height + 8 px. A playback-speed
  change appends ` [150%]`. The name tracks the visible part of a clip that
  is scrolled half off-screen. Full name shows as a tooltip on hover. In
  light theme, with no thumbnail, the name switches to a plain font tinted
  with a darkened border colour.
- **Hover handles:** on clips at least 40 px wide, two pills 3 px wide,
  height min(20, half the clip), radius 1.5, inset 4 px from each edge,
  vertically centred, fill black @ 30%, 1 px white @ 30% outline. They fade
  in on hover and mark the trim zones (the outer 12 px or 25% of the clip).
- **Muted / hidden / non-soloed track:** the clip's media drops to 40%
  alpha.

Selection and state overlays (drawn above the clip):

- **Selected:** a **2 px stroke, radius 3**, inset 1 px, **key colour
  (`#A3E635`) when the timeline has focus, white when it does not**. For
  stacked video+audio blocks each half gets its own outline. No fill.
- **Dirty** (being moved/changed): black @ 70% fill with a key-colour
  stroke, 2 px if focused, 1 px otherwise.
- **Disabled ranges** inside a clip: black @ 80% fill over the range;
  outline 1 px black @ 80% (or the selection outline when selected).
- **Bad-shot band** (source strip): black @ 50% fill with diagonal red
  `#EF4444` stripes, 1.5 px wide, 8 px apart, 80% alpha.
- Selection band (multi-cam): a translucent column over selected clips.

### 3.8 Clip label colours

From `clipLabelColors`: each label has a fill (Tailwind 800) and border
(Tailwind 700), and maps to a Premiere label slot for handoff.

| Label | Fill | Border |
|---|---|---|
| Pink | `#9D174D` | `#BE185D` |
| Red | `#991B1B` | `#B91C1C` |
| Orange | `#7C2D12` | `#9A3412` |
| Yellow | `#854D0E` | `#A16207` |
| Lime | `#3F6212` | `#4D7C0F` |
| Green | `#166534` | `#15803D` |
| Cyan | `#075985` | `#0369A1` |
| Blue | `#1E40AF` | `#1D4ED8` |
| Violet | `#5B21B6` | `#6D28D9` |
| Fuchsia | `#86198F` | `#A21CAF` |
| Stone | `#44403C` | `#57534E` |

Picker order is the table order. Sync-failure tinting reuses the palette:
held = Yellow, membership/leftover failure = Red, unsynced = Stone;
synced-sequence resources default to Lime.

### 3.9 Audio waveform

- The waveform part of a clip is a rounded rect (**radius 2**, 1 px inset)
  filled with a **vertical gradient of the clip's audio colour, 40% alpha
  at the top to 70% at the bottom** (default `rgba(14,75,111,…)`, i.e.
  `#0E4B6F`).
- Peaks are drawn as **solid white** bars in dark theme (in light theme
  the tint is a darkened version of the fill), masked to the rect.
- **Bars grow up from the bottom edge, not mirrored around a centre line**
  (Final Cut style). Bar height = the larger of |min| and |max| for the
  column, scaled so full scale fills the whole lane; any non-silent column
  is at least 2 px tall; silent columns draw nothing. Edges are
  antialiased over half a pixel. Peak bucket size follows zoom so there are
  about 2 px per bucket.
- **Clipped columns** get a **3 px red `#EF4444` mark at the top** of the
  lane (only that column, not a full red bar).
- The gain envelope (a line with draggable points) and fade handles are
  drawn on top when the clip is selected or hovered; a tooltip shows the dB
  value while dragging.

### 3.10 Playhead

- Colour: **key (`#A3E635`) when this timeline is the active sequence**,
  otherwise `#A3A3A3` (tone-400, "quiet mark").
- **Head:** a downward-pointing triangle **12 px wide, 10 px tall**, its
  top edge at y = 0 (top of the ruler), filled with the playhead colour at
  **35% alpha** and outlined with a **1 px** stroke of the full colour
  (stroke inside, mitred). Hit area extends 10 px left/right and 8 px
  above/below.
- **Line:** **1 px**, full colour, from y = 8 (just inside the head) to the
  bottom of the canvas. It is painted above clips.
- Snap guide while dragging: a 1 px `#A3A3A3` line at the snapped frame.
- Razor tool: a 1 px **red `#EF4444`** vertical line under the pointer,
  spanning the hovered clip (or the whole height with Shift).
- Zoom tool drag: 2 px key stroke rectangle with key @ 10% fill.

### 3.11 Markers

Timeline markers (ruler lane):

- Shape: a **12 x 16 px** pin: a 12x12 rounded square (about 1.7 px corner)
  whose bottom tapers to a point at (6, 16). Drawn with its top at
  **y = 16** (just under the ruler ticks), centred on the frame (x − 6).
- Colours (fill / when at the playhead, "active"):

| Colour | Dark fill | Dark active | Light fill | Light active |
|---|---|---|---|---|
| green (default) | `#4ADE80` | `#86EFAC` | `#15803D` | `#166534` |
| blue | `#60A5FA` | `#93C5FD` | `#2563EB` | `#1D4ED8` |
| yellow | `#FACC15` | `#FDE047` | `#A16207` | `#854D0E` |
| red | `#F87171` | `#FCA5A5` | `#DC2626` | `#B91C1C` |

- Click selects/seeks, double-click or right-click opens the note popover,
  hover shows the note, drag moves (3 px threshold, snaps when snapping is
  on).

Clip markers (comment markers on clips): same pin shape, drawn at the top
of the clip's track minus 4 px; fill `#22C55E` (green-500), `#4ADE80` when
selected. The same marker colours appear as 2 px ticks on the Preview scrub
bar.

### 3.12 In/out range

- Range inside the ruler: ink (white) @ **18%** fill over the ruler height.
- Range below the ruler: ink @ **4%** fill down to the bottom of the canvas.
- In and out points: **1 px** vertical lines in the ruler only, ink @
  **70%** (the out line sits one pixel inside the out frame).

### 3.13 Audio meter

At the right edge of the timeline body, separated by a 1 px left border,
padding 12 vertical, 8 left, 4 right:

- Two clip indicators on top, each 8 px tall, radius 2: muted fill, **red-400
  `#F87171` when clipped**; click clears.
- Two meter bars, **8 px wide**, radius 2, 1 px apart, muted background.
  Fill gradient bottom-to-top: emerald-400 to 70%, amber-300 at 90%, red-400
  at the top; peak and RMS masks in muted (RMS at 60%); a 1 px ink hold line.
  Scale −60 to 0 dB; peak falls at 20 dB/s while playing, hold lasts 1.5 s.
- dB scale on the right, 20 px wide: text-3xs (10/12) tabular, ink @ 30%,
  labels 0, −6 … −54.

### 3.14 Other canvas details

- Track boundary lines (optional): 1 px, white @ 5.6% = `#242424`.
- Gap clips paint nothing until hovered.
- Retake indicators are 8 px tall.
- Drag ghost text for clips: "Clip from …" in a small chip.
- Empty draft: a centred `secondary` button, 32 px, padding-x 12, radius 8,
  text-xs 500, shadow-sm: "Create new timeline".

---

## 4. Progress and toasts

### 4.1 Toasts (sonner)

- Position **top-centre**, 96 px from the top (titlebar height + 8 on a
  Mac-style titlebar).
- Card: background `#171717`, **1 px border colour**, shadow-lg, **radius
  8**, **padding 16**, **13 px text**, gap 6 between icon and text, width
  fits content up to 356 px. Icon box is 1.5 em tall; with an action
  button the title gets a 24 px line height.
- Description in muted-foreground. Action button: primary fill `#E5E5E5`
  with `#171717` text, 6 px left margin. Cancel: muted fill, muted text.
- Loading toasts use the app spinner as the icon.
- Typical strings: "Snap is on", "Snapshot saved", "Timeline opened" with a
  View action, analysis summaries ("3 clips analyzed and ready to edit.").

### 4.2 Run list (titlebar workflow menu)

The titlebar has a 36 px ghost icon button showing run state: 20 px spinner
while running, a 16 px pie (circle outline 1.25 px stroke, filled wedge for
progress) when a ratio is known, key-coloured check on success, yellow-400
warning on partial.

Its popover: **320 px wide**, no padding, list capped at 208 px with a thin
scrollbar (8 px, ink @ 20% thumb, radius 4).

```
┌──────────────────────────────────────────────┐
│ Keep computer awake        [switch]          │
├──────────────────────────────────────────────┤
│ Analyzing clips   ▕████████░░░░▏         ✕   │ 52 px row
│ 3 of 12 clips                                │
├──────────────────────────────────────────────┤
│ Export video                  Done           │
└──────────────────────────────────────────────┘
```

- Row **52 px**, hover ink/5, 1 px dividers between rows.
- Title column: padding 16 left / 8 right, text-sm 400 at ink @ 90%,
  truncated; a second line in text-2xs ink @ 45%. While running the title
  column is fixed at **150 px** and a progress column fills the rest.
- **Progress bar:** **8 px tall, fully rounded**, track ink @ 10%, fill
  ink @ 80% (white, not key). Under it, text-xs muted "N of M clips".
- Finished: "Done" in key colour, "Failed" in red-400, partial in yellow-400,
  canceled in ink @ 60%. Cancel/close: 36x36 ghost, 16 px × in tone-500.
- Time estimate copy: "Takes about N min", or "Taking longer than expected".

A **handoff/export hint** reuses the same row as a floating card at the top
right: 320 px, radius 8, 1 px ink/10 border, tone-900 fill, shadow
(0 4 6 / 0 2 4 at 10% black), fades in and slides 8 px down; after 6 s it
shrinks into the run-list button.

### 4.3 Analysis pill in the Project panel

Floating at the bottom of the media panel (8 px insets, 12 px from bottom,
centred, wrap with 8 px gaps): glass pill, **radius 14**, blur 16,
tone-800 @ 50%, 1 px ink/10 border, 12 px weight 500, padding-x 10,
28 px tall.

- Idle: 14 px sparkle icon + "Analyze N clips" (or "Reanalyze").
- Running: 14 px spinner in tone-300 + "Analyzing N clips" rendered with a
  **shimmer**: text filled by a gradient tone-500 → tone-100 → tone-500 at
  200% width, sliding across in 2 s, linear, infinite; then "~N min left"
  in tone-400.
- Canceling: disabled pill with spinner and "Canceling analysis".

The old full-page processing screen strings exist in the dictionaries but
no component in this build renders them; progress is these three surfaces.

---

## 5. egui implementation notes

- One `Painter`-driven widget for the whole timeline canvas: ruler, header
  column, clips, waveforms, overlays, playhead, markers, in this z-order:
  grid/ruler → in/out → clips → status overlays → markers → snap/blade →
  playhead → header column.
- Keep every number above as a module-private `const` next to the painter
  that uses it (track height 60, spacing 4, header 96, ruler 35/16, radius
  4, selection 2 px, playhead head 12x10, marker 12x16 at y 16).
- egui has no backdrop blur: draw glass islands as `#262626` at about 85%
  alpha with the ink/10 border.
- Load Inter for all numbers and timecodes (tabular figures) and Mona Sans
  (or a close sans) for UI text; the ruler and track labels are Inter 12.
- The key colour should be a single `Color32` the whole UI reads, so the
  playhead, selection outline, active words and switches change together.

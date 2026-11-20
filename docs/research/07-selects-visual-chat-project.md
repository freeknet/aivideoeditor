# 07 — Selects visual spec: header, chat, Project panel, panel chrome

A pixel-level spec of four regions of the Selects (Cutback) 2.1.301 editor,
recovered from the renderer chunks in the app's HTTP cache
(`TimelineRoute`, `main`, `ReadOnlyTranscription`,
`TranscriptViewSnapshotContext`, `resource-thumbnail`) and the token sheet
`global-*.css`. Companion to `05-selects-ui.md` (what the UI does); this file
is about how it looks. Dark mode is the reference; light values are given
where they differ in a way that matters.

Everything is paraphrased. Class names appear only as short evidence in
backticks. Icon names are lucide-react names, resolved by following each
minified import back to its `createLucideIcon("Name", …)` definition in the
vendor chunk (lucide-react 0.378).

## 0. Ground rules recovered from the CSS

### 0.1 Units and the scale overrides

The stylesheet is Tailwind **v3** (the `--tw-*` preamble), spacing unit
4 px (`p-1` = 4, `p-1.5` = 6, `p-2.5` = 10, `h-7` = 28, `h-8` = 32,
`h-10` = 40). The theme **overrides the radius and text scales**, so do not
use stock Tailwind numbers:

| Class | Selects value | Stock Tailwind would be |
|---|---|---|
| `rounded-xs` | **2 px** | (not stock) |
| `rounded` | 4 px | 4 px |
| `rounded-sm` | **6 px** | 2 px |
| `rounded-md` | **8 px** | 6 px |
| `rounded-lg` | **10 px** | 8 px |
| `rounded-xl` | **14 px** | 12 px |
| `rounded-2xl` | 16 px | 16 px |
| `text-3xs` | 10 / 12 px | — |
| `text-2xs` | 11 / 16 px | — |
| `text-xs` | 12 / 16 px | same |
| `text-sm` | 14 / 20 px | same |
| `leading-tight` / `snug` / `relaxed` | ×1.25 / ×1.375 / ×1.625 | same |

Fonts: UI and headers **Mona Sans**; numbers that must align use
`font-tabular` = **Inter** with tabular figures; timecode/code **Roboto Mono**.
Weights used: 400, 500 (`font-medium`, the default for every label and
tab), 600 (only buttons: the base `Button` is `font-semibold`).

### 0.2 Colour tokens (dark)

Semantic tokens (shadcn names) resolve to neutral greys:

| Token | Dark | Light |
|---|---|---|
| `background`, `card` | `#171717` | `#ffffff` |
| `popover` | `#262626` | `#ffffff` |
| `secondary`, `muted` | `#262626` | `#f5f5f5` |
| `accent` (hover fill in menus) | `#404040` | `#f5f5f5` |
| `foreground` | `#fafafa` | `#0a0a0a` |
| `muted-foreground` | `#a3a3a3` | `#737373` |
| `border` | white 10 % (≈ `#2e2e2e` on bg) | `#e5e5e5` |
| `input` | white 15 % (≈ `#3a3a3a` on bg) | `#e5e5e5` |
| `primary` / `primary-foreground` | `#e5e5e5` / `#171717` | `#171717` / `#fafafa` |
| `ring` | `#737373` | `#a3a3a3` |
| `sidebar-accent` (list hover) | `#262626` | `#f5f5f5` |
| `sidebar-ring` (list selected) | `#525252` | `#e5e5e5` |
| `destructive` | `#ef4444` | `#dc2626` |
| **`key`** (brand accent) | lime **`#a3e635`**, text on it `#171717` | olive `#4d7c0f`, text white |

Two non-shadcn scales carry most of the chrome:

- **`ink`** = pure white in dark, `#0a0a0a` in light, always used with an
  opacity. Composited on `#171717`: ink/5 ≈ `#232323`, ink/8 ≈ `#2a2a2a`,
  ink/10 ≈ `#2e2e2e`, **ink/12 ≈ `#333333`** (the "selected tab/toggle"
  fill), ink/15 ≈ `#3a3a3a`, ink/20 ≈ `#454545`, ink/30 ≈ `#5d5d5d`.
- **`tone-50…950`** flips between modes so that `tone-950` is always the
  deepest surface and `tone-50` the strongest text. Dark values:

| tone | 50 | 100 | 200 | 300 | 400 | 500 | 600 | 700 | 800 | 900 | 950 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| hex | `#fafafa` | `#f5f5f5` | `#e5e5e5` | `#d4d4d4` | `#a3a3a3` | `#737373` | `#525252` | `#404040` | `#262626` | `#171717` | `#0a0a0a` |

  Useful composites: tone-800/50 on bg ≈ `#1f1f1f`, tone-800/60 ≈ `#202020`,
  tone-800/40 ≈ `#1d1d1d`, tone-950/20 ≈ `#141414`.

"Glass" overlays (floating buttons, badges over video) are always the same
recipe: `bg-tone-800/50` + `backdrop-blur-[16px]` + `border-ink/10`, white
text.

Shadows: `shadow-raised` is a soft two-layer shadow in light and **none in
dark**; popovers use `shadow-md`. Panels are flat.

### 0.3 Two classes that silently do nothing

The code uses two utilities that the stylesheet never generated, so they
fall back to defaults. Match the rendered result, not the source:

- `border-base-border` (timeline header, in-app browser) does not exist;
  the global rule `* { border-color: var(--border) }` applies, so those
  borders are **white 10 %**, the same as every other divider.
- `bg-input/30` (the user message bubble) does not exist because `--input`
  is an `hsla()` value, not RGB channels. **The user bubble has no fill**;
  it is a 1 px white-15 % outline only.

## 1. Panel chrome shared by all panels

### 1.1 Layout and separators

- react-resizable-panels. Default split (percent of parent): chat 22
  (min 3, foldable), editor 78; editor = upper row 55 / timeline 45; upper
  row = Project 21 (min 15, foldable) / Preview 58 / side panel 21 (min 15,
  foldable).
- **No gaps between panels.** Panels touch a **1 px resize line** in
  `border` colour (white 10 %). The line has an invisible 4 px hit strip
  centred on it (`after:w-1`); the Project/side handles widen the pointer hit
  area to 15 px for coarse pointers and 2 px for fine ones. No grip is drawn.
- Every panel body is `bg-background` (`#171717`) with a **1 px border that
  is transparent normally and `tone-500` (`#737373`) on the focused panel**
  (the panel that last received a pointer-down; Shift+1..4 also moves it).
  This full-panel outline is the only focus indicator; it replaces any
  header highlight.
- On macOS only, the outermost bottom corners of the panel area are rounded
  16 px; on Windows they are square.

### 1.2 Panel header

- Height **40 px** (`h-10 min-h-10`), `border-b` white 10 %, horizontal
  padding 8 px, items centred, gap 8 px. Same background as the panel; the
  header is not a different shade.
- There is no bold title. A panel's title is a **tab pill** in a tab strip
  (padding tightened to 6 px, gap 2 px):
  - pill: height 28, padding 0 10 px, `text-xs` (12 px) **medium**,
    radius 6 px (`rounded-sm`);
  - active: fill ink/12 (`#333333`), text `foreground`;
  - inactive: text `muted-foreground` (`#a3a3a3`), hover fill ink/8;
  - icon-only tab: 28 × 28 square;
  - closable tab (an opened App): same height, close `X` (14 px) on the
    left at 30 % opacity, 100 % on hover; middle-click closes; label max
    128 px, truncated.
- Header icon buttons come in two sizes:
  - 28 × 28 (`size-7`, radius 8), 16 px icon, `muted-foreground`; hover
    ink/12 fill + foreground (Project toolbar) or `accent` fill (ghost
    button);
  - 32 × 32 (`h-8 w-8`, radius 4), 16 px icon, muted; hover `secondary`
    fill (`#262626`) + foreground; the open/pressed state keeps the hover
    look (chat header).
- Tooltips: radius 8, `secondary` fill, 12 px text, padding 6 × 12, 4 px
  offset.
- Menus/popovers: radius 8, 1 px border, `popover` fill (`#262626`), 4 px
  inner padding, `shadow-md`; items 14 px, padding 6 × 8, radius 6, hover
  `accent` (`#404040`); section labels 12 px medium muted; separators 1 px
  `border`, inset −4 px, margin 4 px.

### 1.3 Folded panels

- **Chat folded**: a 44 px wide rail on the far left (`w-11`), panel
  background, `border-r` ink/10, 8 px top padding, holding one 32 × 32
  button (radius 8, `MessageSquare` 16, muted, hover ink/10 + `tone-50`),
  tooltip "Assistant" to the right. No vertical text label.
- **Project folded**: no rail. A floating glass button sits in the Preview's
  top-left corner (6 px inset): radius 12, glass recipe, button padding
  8 × 10, `text-sm` medium `tone-50`, `PanelLeft` 16 + "Project", gap 8,
  hover ink/10.
- **Side panel (Transcript/Adjust) folded**: the same glass button in the
  Preview's top-right corner (6 px inset), label first then `PanelRight` 16,
  text "Transcript".
- A panel snaps shut when dragged below 5 % (chat) or 3 % (side panels).

### 1.4 Scroll areas

- Custom scrollbar `cutback-transcription-scrollbar`: 12 px track,
  transparent; thumb `#484848`, fully rounded, with a 2 px transparent
  border (so the visible thumb is 8 px). `scrollbar-gutter: stable` keeps
  the 12 px reserved, so scrolling content has 12 px right padding even
  without overflow.
- Content that can scroll gets 24 px fades at the top and/or bottom
  (gradient from the panel background to transparent), stopping 12 px short
  of the right edge to clear the scrollbar.

```
┌ header 40 ─────────────────┐│┌ header 40 ──────────────┐
│[Tab][Tab]           [□][□] │││[Project][Apps]          │   1 px line
├────────────────────────────┤│├─────────────────────────┤   (white 10 %)
│ body #171717               │││ body                    │   between panels,
│ (1 px border: transparent, │││                         │   no gap
│  #737373 when focused)     │││                         │
└────────────────────────────┘│└─────────────────────────┘
```

## 2. App header (top bar)

- **Fixed, 52 px tall**, full width minus the Windows caption-button
  overlay on the right (right edge = `--window-controls-inset`). A 52 px
  spacer sits under it in the flow.
- Fill `tone-950` at 20 % over the content (≈ `#141414` over the panels)
  with `backdrop-blur-md` (12 px); `border-b` ink/10. Whole bar is a window
  drag region; controls opt out.
- Padding 0 8 px. Three zones:
  - **Left** (gap 8, max width `50% − 220 px` on ≥ 1024 px):
    - macOS: 84 px left inset for the traffic lights. Windows: a fixed
      36 px application-menu button at x = 4 (`Menu` 16) and a 44 px inset.
    - Back button: 36 × 36 (16 px icon + 10 px padding), radius 8, hover
      ink/10, icon `Home` 16 (or `X` when it closes a modal page).
    - Breadcrumb: `text-sm` (14/20) **medium**, truncate. Earlier crumbs
      `tone-400`; the current (project) title `foreground`; separator
      `ChevronRight` 15 px `#a3a3a3`. Double-click the title to rename; a
      20 × 20 `Pen` (12 px) button fades in on hover (radius 4, hover
      `secondary`).
  - **Centre** (only ≥ 1024 px, absolutely centred): the search trigger, a
    pill 36 px tall, min width 400, padding 0 12, radius 8, fill ink/5
    (`#232323`), 1 px border ink/15, `Search` 16 + placeholder "Search
    dialogue, scene, moments" in `text-sm` `tone-400`, gap 8, hover fill
    ink/10. Below 1024 px it collapses to a 36 px icon button.
  - **Right** (gap 0): background-job progress chip, handoff hint, then
    36 × 36 icon buttons with radius 8 and hover ink/10: **View mode**
    (`PanelsLeftBottom` 16; menu 200 px wide with a "View mode" label and a
    `Check` beside the current mode), **Preferences** (`Settings2` 16; menu
    279 px). Then, after 8 px, **Export**: a filled button 36 × 36, radius 8,
    fill `secondary` (`#262626`), hover 80 %, `Share` 16; menu 279 px
    aligned right. If a quick-handoff target is set, a second 36 × 36
    secondary button shows that tool's 16 px logo (radius 3).

```
┌────────────────────────────────────────── 52 px, #171717/20 + blur, border-b ink/10 ┐
│[≡][⌂]  Project name ✎        [⌕ Search dialogue, scene, moments ]   ◔  [▣][⚙] [⇪]│ ▢ ✕
└──────────────────────────────────────────────────────────────────────────────────────┘
 36px btns, r8                     36 tall, ≥400 wide, r8, ink/5, ink/15 border    36 secondary
```

## 3. Agent chat panel

### 3.1 Frame and header

- Panel: background `#171717`, 1 px focus border (section 1.1). The header
  is **absolutely positioned over the top** (40 px, `border-b` white 10 %,
  padding 0 8, gap 0, panel background) with a 40 px spacer under it.
- In the shipping timeline configuration (chat list enabled, tabs not shown
  in the header) the header holds:
  - a 32 × 32 **Chats** button (`Menu` 16, radius 4, muted, hover/open
    `secondary` fill + foreground) that opens the chat list;
  - the **current chat title**, `text-xs` (12 px) muted, truncate, filling
    the row; double-click to rename (inline input `text-xs`, padding 2 × 6);
    a 20 × 20 `Pen` button fades in on hover. A new title types itself in
    with a shimmer (gradient `tone-300 → ink → tone-300`, 200 % width).
  - Find-in-chat (Cmd+F) replaces the title with a search field, a match
    counter (`text-2xs` tabular muted) and up/down buttons.
- In **Comments** mode the header shows a fold button (`PanelLeft` 16,
  32 × 32), a 1 × 20 px divider (`border` colour, 4 px side margins) and the
  title "Comments on <timeline>" in `text-xs` **medium** foreground.
- Chat list popover (from the Chats button): 320 px wide, height
  max(480 px, 70 vh), no padding, `text-sm`.
  - Toolbar row, 4 px padding, gap 4: Hide panel (`PanelLeft`, 32 × 32) and,
    pushed right, a ghost "New chat" button (28 px tall, padding 0 8,
    `text-xs`, `Plus` 14, gap 4).
  - Rows: padding 12 × 16, gap 2, hover and active fill `accent`
    (`#404040`). Line 1: title `text-sm` foreground truncate, then a
    timestamp `text-xs` `tone-500`; an 8 px `key`-lime dot means "needs your
    answer"; a spinner means streaming. Line 2: last-message snippet
    `text-xs` `tone-500`, clamped to 2 lines. A 24 × 24 "…" actions button
    (radius 4, `tone-400`, hover ink/10) appears top-right on hover.
- "Recent chats" (History) is a separate dropdown used by the tab-strip
  configuration below: 288 px wide, max list height 288 px; rows are menu
  items with title truncate and a muted time on the right. Its trigger is a
  32 × 32 `History` button. In the default timeline configuration the chat
  list popover above takes its place.
- **Alternate tab strip** (configurations without the chat list): tabs in a
  horizontally scrolling row, gap 2; each tab max 140 px, padding 6 × 8,
  `text-xs`, radius 4 with a 1 px border; active border `tone-600`
  (`#525252`) + fill `tone-800` (`#262626`) + foreground; inactive border
  transparent, muted, hover fill tone-800/60; streaming tabs show a 10 px
  blue-400 spinner; close `X` 10 px. Right cluster: New chat (`Plus`),
  History (`History`), each 32 × 32.

### 3.2 Message list

- Scroll area: padding 16 px top/bottom, **12 px left**, right side is the
  12 px stable scrollbar gutter; 12 px vertical rhythm between items
  (`space-y-3`). 24 px top/bottom fades.
- Jump-to-latest button: 28 px circle centred 12 px above the composer,
  glass recipe (`bg-tone-800/50`, blur 16, border ink/10), `ArrowDown` 14
  white; fades out when pinned to bottom.
- "Load older": centred pill, padding 4 × 10, radius 8, border `tone-800`,
  fill `tone-900`, `text-xs` `tone-400`, hover fill `tone-800` + `tone-200`.

**User message**

- Right-aligned, max width 85 % of the list, 12 px top margin.
- Bubble: radius **10 px** (`rounded-lg`), **1 px border white 15 %, no
  fill** (see 0.3), padding 6 × 12, `text-sm` (14/20), pure white text,
  wraps anywhere, preserves newlines.
- Attachments stack above the bubble, right-aligned, gap 6.
- Meta row under it, right-aligned, 4 px gap: time and a `Copy` (12 px)
  button, `text-3xs` (10 px) `tone-600`, visible only on hover.

**Assistant message**

- **No bubble.** Left-aligned with 4 px side padding, 12 px top margin
  (0 when it continues a tool/thinking block). Markdown in `text-sm`
  `tone-200` (`#e5e5e5`), selectable.
- Same hover meta row (time + copy), left-aligned.

**Thinking / tool activity rows** (one shared row component)

- Whole row `text-xs`. A full-width button: padding 4 px, gap 8, radius 4,
  text `tone-400`. If expandable: hover fill tone-800/40 and text
  `tone-200`, plus a 12 px chevron (`ChevronRight` collapsed, `ChevronDown`
  open, stroke 2.5, `tone-500`) right after the label.
- Label `tone-500`; while running it **shimmers** (gradient
  `tone-500 → tone-100 → tone-500` clipped to text, 200 % background,
  animated). Labels: "Thinking"/"Thought", grouped tool summaries
  ("Inspected project, Searched…"), single tool names.
- Right side: elapsed time `text-3xs` (10 px) `tone-600` tabular; "N tool
  calls" in 10 px `tone-600`; "N steps" `text-2xs` `tone-600` while running;
  "Error" `text-2xs` `tone-700`.
- Leading icon only for stopped (`Square` 10, filled `tone-500`) or error
  (`CircleAlert` 12, red-400).
- Expanded raw output: a block indented 24 px, 4 px above, radius 4, fill
  `tone-950` (`#0a0a0a`), padding 8, `text-2xs` (11 px) `tone-400`
  (red-400 on error), pre-wrapped. Thinking text is italic. Child rows of a
  group are indented 16 px.
- "Worked" summary row wraps a finished turn the same way; "turn took 1m
  12s" lines are `text-2xs` muted tabular, padding 4.
- Context-compacted divider: 16 px vertical margin, 1 px `border` rules
  either side of a `text-2xs` muted label (max 75 % width).
- Sub-agent rows: min height 36, padding 6, `text-xs`, a 6 px status dot
  (amber-400 waiting, red-400 failed, `tone-500` stopped, violet-400 pulsing
  running, emerald-400 done), title `font-medium` `tone-300`, counters
  `text-3xs` `tone-600`; rows separated by `tone-800`/70 hairlines.
- Edit-agent card: 8 px top margin, radius 10, 1 px border key/20, fill
  key/5 (≈ `#1e2118`), padding 10 × 12; title `text-xs` medium **lime**.

### 3.3 Approval and option cards

**"Allow this action?"** (sits directly above the composer, not in the list)

- Strip with `border-t` `tone-800`, padding 8 × 12, `text-xs`, 8 px bottom
  margin.
- Row 1 (gap 8, `tone-300`): an amber-400/80 warning icon, the title, and
  the tool name as a chip (radius 4, fill `tone-800`, padding 2 × 6,
  `text-2xs` `tone-200`).
- Detail box: max height 128, scrolls, radius 4, fill `tone-950`, padding
  8, `text-2xs` `tone-400`, pre-wrapped.
- Buttons right-aligned, gap 8, both padding 4 × 10, radius 4, `text-2xs`:
  **Cancel** text `tone-300`, hover fill `tone-800` + white text;
  **Approve** solid white fill with black text, medium, hover `tone-200`.

**Options card** (agent asks a question)

- Card: radius **14 px**, 1 px border `tone-700` (`#404040`), fill
  `tone-900` (`#171717`), padding 10 × 12, `text-sm`, `shadow-lg`.
- Top row: "Step 2 of 4" in `text-2xs` `tone-500` between back/next
  chevron buttons (padding 2, radius 4, hover `tone-800` + `tone-200`), and
  a Minimise button on the right.
- Question: `tone-200`, pre-wrapped, 8 px below.
- Choices: one bordered list, radius 8, border `tone-800`, rows divided by
  `tone-800` hairlines. Row: padding 8 × 10, gap 8, `text-sm` white; fill
  tone-950/40, hover tone-800/60; chosen fill tone-800/80, hover
  tone-700/60. Leading 20 px number circle (fill `tone-700`, `text-2xs`
  `tone-300`). "Recommended" follows the label in `tone-400` with 6 px gap.
  Optional description below in `text-xs` `tone-400`. A hover-only arrow
  on the right (`tone-300`).
- "Something else" row: muted label, plus a "Skip" button (radius 8,
  border `tone-700`, padding 4 × 10, `text-xs` `tone-300`, hover fill
  `tone-700`).
- Free-text mode: an inner box radius 10, border `tone-800`, fill
  `tone-950`, textarea padding 12/12/4, `text-sm`, send button 32 × 32
  radius 10, fill `tone-800`, hover `tone-700`, `ArrowUp`.
- Multi-select footer: "Pick several" `text-2xs` `tone-500` + confirm.
- Minimised: a centred pill, radius full, border `tone-700`, fill
  tone-900/95, padding 6 × 12, `text-xs` `tone-200`, `shadow-lg`.
- Answered: collapses into the list as the question (`tone-200`) plus
  "chose X" (`text-xs` italic `tone-500`) and the answer time
  (`text-2xs` muted tabular).

### 3.4 Empty state and quick actions

- Centred column filling the list height, gap 12, padding 0 16.
- Message: centred `text-sm` `tone-500` ("Your assistant editor, here and
  ready.").
- Under it a centred column of **pill buttons**, gap 6: radius full,
  1 px border `tone-700`, padding 4 × 12, `text-xs` `tone-400`; hover border
  `tone-500`, fill `tone-800`, text `tone-200`. First pill is "View chat
  guide" with an `ExternalLink` 12 after the text (gap 6); then "Suggest a
  first cut" and "Sync my angles and audio".
- Prep/quick-start variant: left-aligned message in `text-sm` foreground
  that types itself out, then left-aligned option buttons with radius
  **12 px** (not full), border `tone-700`, padding 4 × 12, `text-sm`
  foreground, same hover; they enter one by one (120 ms stagger, fade +
  8 px slide-up, 300 ms).

### 3.5 Composer

- Form padding: 0 12 px, 12 px bottom.
- **Box**: radius **14 px** (`rounded-xl`), 1 px border `tone-800`
  (`#262626`), **border `tone-500` while focused**, fill **`tone-950`
  (`#0a0a0a`, darker than the panel)**, no shadow in dark.
- Inside, top to bottom:
  1. Optional notice rows (errors red-200, reconnecting amber, hints
     `tone-400`): min height 32, padding 8 × 12, `text-xs`,
     `border-b` `tone-800`.
  2. **Chat / Comments switch** row, padding 10 px left/top: a segmented
     control, radius 8, fill `tone-950` (blends into the box), 2 px inner
     padding, `text-xs`; segments radius 6, padding 4 × 8, **medium**;
     active fill ink/12 + foreground; inactive `tone-400`, hover `tone-200`.
  3. Attachments: image strip and file chips (padding 12 px top/sides; file
     chip = radius 8, 1 px border, padding 12 × 10, 16 px file icon muted,
     name `text-xs` medium, size `text-xs` muted, remove button 20 px
     circle at the top-right corner).
  4. **Textarea**: padding 16 top, 16 sides, 8 bottom; `text-sm` (14/20);
     4 rows (≈ 80 px of text, ≈ 104 px total); placeholder `muted-foreground`
     ("What do you want to do with this video?"); selection fill `tone-600`;
     no visible scrollbar.
  5. **Toolbar**: padding 10, space-between.
     - Left: **Add** button 32 × 32, radius 4, `Plus` 16, muted, hover/open
       `secondary` fill; a 6 px white dot top-right when downloads are
       pending. Menu items: "Attach a file" (`Paperclip` 16), "Web search"
       (`Globe` 16).
     - Right, gap 2: **model picker** trigger, 32 px tall, padding 0 12,
       radius 4, `text-xs` **medium** muted, hover/open `secondary`; label
       truncated, then "· effort" in `tone-500`. Its menu (min 224 px)
       holds the models, effort, and a **Permissions** sub-section (Ask /
       Auto / Bypass, each with a one-line `text-xs` muted description).
       Optional **context meter**: a 12 px ring, stroke 2, track `tone-700`,
       progress sky-500 (amber-400 ≥ 70 %, red-400 ≥ 90 %), button padding
       2 × 4, radius 4, hover `tone-800`; tooltip "Context 42k / 200k
       (21 %)". (Off in the default timeline config.)
       **Send**: 32 × 32, radius 8, fill `secondary` (`#262626`), hover
       80 %, `ArrowUp` 16, 4 px left margin; disabled 50 % opacity. While
       streaming it becomes Stop (`Square` 16) with the same shape, or a
       queue-send with tooltip "Sends when this reply finishes".
       In Comments mode it is "Add comment": 32 × 32, radius 10, fill
       `tone-800`, hover `tone-700`.
- Under the box, centred, padding 0 16 12, −4 px top margin: "Having
  trouble? Report an issue or bug" + `ChevronRight` 12, `text-xs`
  `tone-500`, hover `tone-300`.
- Queued messages appear above the composer as user-style bubbles clamped
  to 3 lines, with "Cancel and edit" / "Send now" text links in `text-xs`.

### 3.6 Mentions

- **Mentions are coloured text, not chips.** In the composer a mirror layer
  under the transparent textarea paints `@Name` runs in `key` (lime
  `#a3e635`); the rest is foreground. In sent user bubbles the same runs
  are lime. No background, border or padding.
- Autocomplete opens **above** the composer (12 px side insets, 8 px gap):
  radius 8, 1 px border, `popover` fill, `shadow-md`, inner padding 4, max
  height 240. Rows: padding 6 × 8, gap 8, radius 6, `text-sm`; active fill
  `accent`. Leading 16 px visual: thumbnail (radius 2, 1 px ink/10 border)
  or icon — `Folder` filled blue-400, `GanttChart` timeline,
  `SquareGanttChart` synced timeline, `AudioWaveform` audio, `Film` video.
  Name truncates; right meta `text-xs` muted ("0:42", "Timeline",
  "Folder · 12 files").

```
┌ chat panel ──────────────────────────────┐
│[≡] Untitled chat                         │ 40, border-b white10
├──────────────────────────────────────────┤
│                    ┌───────────────────┐ │ user: r10, 1px white15
│                    │ cut it to 30 s    │ │ outline, no fill, 14px
│                    └───────────────────┘ │
│ ◦ Inspected project, Searched ›    0:04  │ tool row 12px tone-400
│ Here's a tighter cut. I dropped …        │ assistant: no bubble,
│                                          │ 14px tone-200
│                 ( ↓ )                    │ 28px glass jump button
│ ╭──────────────────────────────────────╮ │ box r14, #0a0a0a,
│ │ [Chat|Comments]                      │ │ border #262626 (#737373 focus)
│ │ What do you want to do with this…    │ │ textarea 14px, 4 rows
│ │                                      │ │
│ │ [+]                 Sonnet · high [↑]│ │ 32px controls, p10
│ ╰──────────────────────────────────────╯ │
│     Having trouble? Report an issue ›    │ 12px tone-500
└──────────────────────────────────────────┘
```

## 4. Project (source browser) panel

### 4.1 Header and toolbar

- Panel tab strip (section 1.2) reading **"Project"**, **"Apps"**, then one
  closable tab per opened App. 40 px, `border-b`.
- Toolbar row directly under it, **40 px, no border**, padding 0 8, gap 8:
  - **Add or create** `Plus` 16 in a 28 × 28 button, radius 8, muted; hover
    and open: fill ink/12 + foreground. Menu 220 px: Media, Generate,
    Record, Blank clip, — , Folder, Timeline (`GanttChart`), Synced timeline
    (`SquareGanttChart`), — , Import synced timeline; 16 px icons, gap 8.
  - **Search project** `Search` 16, 28 × 28 ghost, muted, hover `accent`.
  - Pushed right: **view toggle**, three 28 × 28 toggles, no gap, padding 6,
    radius 8, 16 px icons: `FolderTree` (tree list), `List` (flat list),
    `LayoutGrid` (cards). On: fill ink/12, icon `tone-50`; off: `tone-400`,
    hover `tone-200`.
- With search open the Add+Search pair is replaced by a search field that
  fills the row and the view toggle hides: height 28, radius 6, 1 px border
  white 15 %, transparent fill, `shadow-sm`, padding 0 4 0 8, gap 4;
  `Search` 13 muted; input `text-xs`; placeholder "Search"; trailing 20 px
  ghost buttons: **Find moments** (`Fullscreen` 12, tooltip "Find moments by
  what's on screen") and close (`X` 12).
- There is **no "All" filter** in this version's Project header (the string
  exists but is only used by the speaker panel).
- List view only: a column header, 28 px, `border-b`, padding 0 18 0 8:
  "Name" left, a metric menu (default Duration; sort ascending/descending)
  right.

### 4.2 List view

- Scroll padding: 8 px top/bottom, 8 left, 4 right (+12 px gutter).
- Row: **28 px tall, 2 px apart** (30 px pitch), radius 8, gap 8, padding-left
  6 + 24 × depth (+24 when the row has no disclosure chevron), right 2.
  - Hover fill `sidebar-accent` (`#262626`); **selected fill `sidebar-ring`
    (`#525252`)**, also on press. No accent colour for selection here.
  - Optional colour label: a 4 × 16 px pill with its own border colour.
  - Leading 16 px visual: thumbnail 16 × 16, radius 2, 1 px ink/10 border;
    or icon in muted (`GanttChart` timeline, `AudioWaveform` audio);
    `TriangleAlert` 16 destructive if analysis failed; `TriangleAlert`
    yellow-400 if the file is offline.
  - Name: `text-sm` (14 px) foreground, truncate, tooltip when truncated.
  - Status after the name: analysis progress ring or 14 px spinner (muted);
    a 20 × 16 count badge (radius 4, 10 px medium; `tone-600` fill +
    foreground when in the current synced timeline, else `tone-700` +
    muted); a 6 px dot ink/40 for a timeline already open as a tab.
  - Right: metric (duration) `text-xs` muted, Inter tabular, right-aligned,
    6 px right padding.
  - On hover the metric hides and small pill buttons appear: "Analyze" /
    "View" / "Cancel" (fill `secondary`, padding 2 × 8, radius 4, `text-xs`
    medium, `shadow-sm`, hover `accent`) and row actions (24 × 24, radius 4,
    muted, hover `accent`).
  - Inline rename: 22 px input, `text-sm`.
  - Disabled/dragged rows fade to 40 %.
- New-folder row: 28 px, radius 8, fill `accent`, blue-400 folder icon.

### 4.3 Card view

- Scroll padding: 16 left, 4 right, 16 bottom (+12 px gutter), 16 px top
  offset.
- Grid: column count = max(1, ⌈(panel width − 16 + 12) / (160 + 12)⌉), so
  cards are **at most ~160 px wide** and stretch to fill (`minmax(0, 1fr)`);
  12 px column gap, 16 px between rows.
- Sections per folder: sticky header, padding 12 top/bottom, `text-xs`
  **uppercase** muted, folder path joined with " · ", on a 70 % panel-colour
  backdrop with blur; later sections get a `border-t`; 8 px under the
  header.
- **Media card** (column, gap 4, radius 4):
  - Thumbnail: **16:9** (`aspect-video`), radius **2 px**, fill `muted`
    (`#262626`), image cover; placeholder icon 20 px muted (`Film` video,
    `AudioLines` audio, `GanttChart` timeline). Hover-scrub replaces the
    image with the frame under the pointer.
  - Colour label: a 4 px band across the top edge of the thumbnail.
  - **Duration badge**: bottom-right, 4 px inset; glass (tone-800/50 +
    blur 16, transparent border), radius 2, padding 0 4, `text-xs` (12/16)
    **medium**, white, e.g. "0:42".
  - Analysis-failed badge: bottom-left, 4 px inset, same glass, padding 0 2,
    `TriangleAlert` 14 destructive. (Unanalysed items have no card badge;
    that state shows only in the list view and the Analyze pill.)
  - Name: under the thumbnail, 2 px side padding, `text-xs` **medium**,
    line-height 1.25 (15 px), foreground, wraps (break-all, no clamp); an
    offline `TriangleAlert` 14 yellow-400 can follow it.
  - **Selection**: 2 px outline with 2 px offset around the thumbnail only,
    **lime `key` when the Project panel is focused, `tone-400` grey when
    it is not**; hover outline 2 px ink/20. The name is not highlighted.
  - Marquee selection rectangle: 1 px `key` border, key/10 fill, radius 6.
- Topic (chapter) cards inside an analysed clip: same thumbnail; text
  block gap 4: topic text `text-xs` medium, 16 px line height, clamped to 2
  lines; range "00:12-00:45" in Inter `text-2xs` (11 px), line-height 1,
  ink/30, tabular.

### 4.4 Analyze pill

- Floats 12 px above the bottom, 8 px side insets, centred (a 100 px spacer
  ends the list so it never covers the last row).
- Glass pill: radius 14, `bg-tone-800/50`, blur 16, border ink/10, white
  12 px medium text, padding 0 10, gap 4, hover tone-700/40. Labels:
  "Analyze 3 clips", "Re-analyze 3 clips", "Analyzing 3 clips · 4m left"
  (remaining part in `tone-400`), "Canceling…", each with a `tone-300`
  spinner while working.

```
┌ Project panel ───────────────────────────┐
│[Project][Apps]                           │ 40, tabs 28 r6, ink/12 active
├──────────────────────────────────────────┤
│[+][⌕]                        [⌸][≡][▦]   │ 40, no border, 28px buttons r8
│ ╔══════════╗ ┌──────────┐ ┌──────────┐   │ cards ≤160 wide, gap 12
│ ║ 16:9  r2 ║ │          │ │          │   │ selected: 2px lime outline,
│ ║    [0:42]║ │    [1:07]│ │    [0:09]│   │ 2px offset (grey if unfocused)
│ ╚══════════╝ └──────────┘ └──────────┘   │ badge: glass, 12px medium
│ A001_C002.mov IMG_4471.mov Interview.wav  │ name 12px medium, wraps
│                                          │
│        ( Analyze 3 clips )               │ glass pill r14, bottom 12
└──────────────────────────────────────────┘
List view row:  ▏ [▣] A001_C002.mov ·          0:42   (28 tall, r8,
                hover #262626, selected #525252, name 14px, time 12px Inter)
```

## 5. What to get right first (for the egui port)

1. Radii are one step larger than stock Tailwind: 2 / 4 / 6 / 8 / 10 / 14.
   Tabs and toggles 6–8, cards and the user bubble 10, composer and option
   cards 14, thumbnails only 2.
2. Default text is 12 px medium for chrome, 14 px only for message bodies,
   the composer, list-row names and menu items. The header title is 14
   medium; nothing in a panel header is bold or larger than 12.
3. Panels are separated by a 1 px white-10 % line with no gap. Focus is a
   1 px `#737373` outline around the whole focused panel.
4. All panel headers are 40 px with a bottom hairline; the top bar is
   52 px, translucent, blurred.
5. Selected/active state is a translucent white fill (ink/12 ≈ `#333`), not
   a colour. Lime (`#a3e635`) is used only for mentions, the card selection
   outline when the panel is focused, unread dots and edit-agent accents.
6. The composer box is darker than the panel (`#0a0a0a` on `#171717`) with
   a `#262626` border turning `#737373` on focus; send is a 32 px `#262626`
   square with an up arrow.
7. User messages are an outlined (not filled) 10 px-radius bubble on the
   right; assistant messages have no bubble and use `#e5e5e5` text.
8. Tool and thinking activity is low-contrast 12 px text (`#737373` labels,
   `#525252` timings) with a shimmer while running, not cards.
9. Media cards: 16:9, ~160 px max width, 12 px gaps, glass duration badge
   bottom-right, name below in 12 px medium; list rows 28 px with a `#525252`
   selected fill.
10. Floating overlays (folded-panel buttons, jump-to-bottom, Analyze pill,
    badges) share one glass recipe: `#262626` at 50 %, 16 px blur, white
    10 % border.

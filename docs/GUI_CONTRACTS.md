# GUI contracts

How the desktop app is put together, and the rules that let several people
(or agents) build pages at the same time without stepping on each other.

Read with: `CLAUDE.md` (house rules, enforced by the build),
`docs/research/01-problem-and-competitors.md` (what problem we solve),
`docs/research/02-ui-and-workflow.md` §9 (the screen-by-screen spec),
`docs/research/03-techniques.md` (engine upgrades).

## The problem we are solving

A weekly night of dark, dialogue-free event footage (fire, LED, silks, DJs;
~80 clips, 20+ minutes) must become 15/30/60 s reels, a 4:5 feed cut, a 16:9
cut and a recap, beat-synced, with hard rules (no repeated shots or
performers, best moments only, drop on the hero fire shot). **Most of the time
goes on finding and choosing moments, then on versions per length/aspect.**
Paid tools are transcript-first or cloud/credit-based and none sees fire or
light as the subject or enforces rules. Our edge: a measured, captioned shot
log; rule-checked plans with a reason per shot; local and free.

## Layout (Selects-style single workspace)

```
┌ top bar: home · night name · [ search shots, captions, people… ] · chat · inspector · export · ⌘K · ⚙ ┐
│ chat      │ browser    │            viewer            │ inspector                                   │
│ (history, │ Shots      │  (player, aspect + safe      │ (everything about the selection)            │
│  ref chips│ People     │   areas, transport)          │                                             │
│  composer)│ Music      │                              │  [export panel docks here when open]        │
│           ├────────────┴──────────────────────────────┴─────────────────────────────────────────────┤
│           │ timeline: ruler · V1 shots on the beat grid · A1 music waveform · drops · check bar     │
└───────────┴──────────────────────────────────────────────────────────────────────────────────────────┘
status bar: running jobs · night · last error
```

Home screen (`pages::night`) replaces the workspace when no night is open or Home is pressed.
Look: neutral studio greys (signature theme), small type, flat panels with 1 px borders,
thumbnails carry the colour. Reference: Selects (tryselects.com) — study its density.

Proportions are Selects': chat 22 % of the window; browser 21 %, viewer 58 %,
inspector 21 % of the editor width; timeline 45 % of the editor height. Every
split is a fraction (`Config::panel_sizes`), chat / browser / inspector /
timeline fold to labelled rails, and the view modes Default / Vertical /
Comment / Preview (`Alt+1…4`) rearrange them (`gui::ui::layout`).

Keys live in one table, `gui::ui::keymap::COMMANDS` (id, label, scope, ave /
Premiere / Final Cut keys). Global rows are dispatched by the shell; region
rows document keys a region reads and give it `keymap::chord(id, preset)` to
adopt the user's preset. `?` prints the table.

| Region | Module | Shortcut (ave preset) |
|---|---|---|
| Home | `gui::pages::night` | home button, Ctrl+1 |
| Browser · Shots / People / Music | `gui::pages::{review,people,music}` | Ctrl+B fold, Shift+1 focus, Ctrl+2 / 3 / 4 |
| Viewer | `gui::pages::viewer` (+ `gui::player`) | — |
| Inspector | `gui::pages::inspector` | Ctrl+I fold, Shift+4 focus |
| Timeline | `gui::pages::plan` | Ctrl+T fold, Shift+3 focus, Ctrl+5 |
| Export | `gui::pages::deliver` | Ctrl+E, Ctrl+6 |
| Chat | `gui::assistant` | Ctrl+J fold, Shift+2 focus |
| Search | top bar → `ctx.search` | Ctrl+F |

**Mention in chat** from a context menu: call
`crate::gui::ui::widgets::mention_menu(ui, ctx.actions, &reference)` (or
`mention_menu_many` for a selection, in timeline order). It renders "Mention
in chat ▸ about / keep / avoid / replace / more like / feature" and pushes
`Action::AddRef`.

## Code map

| Area | Files | Owner |
|---|---|---|
| Shell, app state, actions | `src/gui/app/mod.rs`, `src/gui/ui/*.rs`, `src/gui/run.rs` | integration only |
| Page contract | `src/gui/pages/mod.rs` (`PageCtx`, `States`, helpers) | integration only |
| Regions | `src/gui/pages/<region>.rs` + optional `src/gui/pages/<region>/*.rs` | one owner per region |
| Workspace layout | `src/gui/ui/workspace.rs` | integration only |
| Reserved engine modules | `src/search.rs`, `src/recipe.rs`, `src/alternatives.rs` (declared in lib.rs, empty) | see owners in the brief |
| Assistant panel | `src/gui/assistant.rs` (+ `src/gui/assistant/*.rs`) | assistant owner |
| Viewer | `src/gui/player.rs` (+ `src/gui/player/*.rs`) | player owner |
| Thumbnails | `src/gui/thumbs.rs` | integration only |
| Open night | `src/project.rs` | integration only |
| Background jobs | `src/jobs.rs` | integration only |
| References | `src/refs.rs` | integration only |
| Engine | `src/{analyze,index,caption,beats,people,plan,draft,render,otio,media,commands}.rs` | engine owners |

**Integration-only files** may be changed by a page owner only to *add*
something their page needs that cannot live in their own files, and the
change must be additive and small (a new method, a new `Action` variant at
the end of the enum, a new field). Say so in your final report.

## Pages

```rust
// src/gui/pages/<page>.rs
#[derive(Debug, Default)]
pub(crate) struct State { /* whatever the page needs between frames */ }

pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui);
```

- Draw into `ui` (the whole central area). Use `egui::Panel::left/right/top/bottom(..).show(ui, ..)`
  for docked panes inside the page; manage your own `ScrollArea`.
- No night open → call `super::no_project(ctx, ui)` and return.
- Single-key shortcuts only when `ctx.keys_free(ui)` (no modal, no focused text field).
- Colours from `ctx.theme` via `crate::gui::ui::theme::rgb`, spacing from `theme::SPACE_*`,
  controls from `crate::gui::ui::widgets` (button, card, card_grid, switch, meter, title, muted).
- Persist page options in `project.settings.pages["<page>"]` (a `toml::Table`) and
  `project.save_settings()`; app-wide preferences go in `Config` (integration).

### What `PageCtx` gives you

| Field | Use |
|---|---|
| `theme` | the palette |
| `project: &mut Option<Project>` | the night: `work`, `settings`, `index` (shot log), `music`, `people`, `plans`; `reload_*()` after you write a file |
| `jobs` | `spawn(label, Cmd)` runs an engine step (`crate::commands::Cmd`) in the background; `running()`, `get(id)`; the app reloads the project when one finishes |
| `thumbs` | `paint(ui, rect, path, t, cx, placeholder)` cover-fits a frame (cropped around `cx`); `get(..)` for the raw texture |
| `player` | the shared viewer: `open_clip(path, t0, t1, cx)`, `open_sequence(Vec<SeqShot>, music)`, `open_audio(path, t0, t1)`, `sequence_shot()`, `play/pause/toggle/seek/position/duration`, `show(ui, thumbs, size, aspect, bg)`, `transport(ui)`, `handle_keys(ui, ctx.keymap)` (Space, J/K/L, ←/→, under the preset) |
| `config` | saved preferences |
| `keymap` | the keyboard preset: read region keys with `keymap::pressed(input, id, ctx.keymap)` / `consume` / `fires`, never a hard-coded `Key` |
| `refs` | references staged for the assistant (read; add with `Action::AddRef`) |
| `selection` | the app-wide selection as `Vec<Reference>`: **replace it** when the user selects in your region; the inspector shows it; `R`/`Shift-R` stage it for chat (handled by the workspace) |
| `search` | the top bar search text (Shots filters by it; others may too) |
| `actions` | push `Action`s; applied after the frame |

### Actions (`src/gui/app/mod.rs`)

`Go(Page)`, `PickProject`, `OpenProject(path)`, `ReloadProject`, `CloseProject`,
`AddRef(Reference)`, `ClearRefs`, `ToggleChat`, `ToggleInspector`, `ToggleExport`, plus the shell's own (settings, theme, sidebar, quit).

## References (`src/refs.rs`) — the "reference it in the conversation" feature

The user constantly needs to point at *these* clips/shots/bars while talking to
the assistant. Every page makes references from its selection; the assistant
composer shows them as chips and sends them (resolved to facts) with the message.

- **Kinds:** `Shot{plan,index}`, `Segment{id}`, `Range{clip,t0,t1}`, `Frame{clip,t}`,
  `Bars{from,to}`, `Person{name}`, `Plan{name}`.
- **Modifiers** (intent): `About`, `Keep` `+`, `Avoid` `!`, `Replace` `>`, `Like` `~`, `Hero` `*`.
- **Text form** round-trips: `>@shot:reel-30#5 ~@seg:MVI_7925/3 !@person:white-mask @bars:17-20`.
  Typing that in the composer is the same as clicking.
- `refs::resolve(&ref, project)` → `Resolved { clip, range, facts }` — the precise
  facts (source range, shot-log numbers, caption, who) the model receives.

**Every page must offer reference-making on its selection:**

| Gesture | Effect |
|---|---|
| click / Shift-click / Ctrl-click | select one / range / toggle items (cards, timeline shots, people, bars) |
| `R` | add the selection as references (`Action::AddRef` per item), modifier `About` |
| `Shift-R` | add with a modifier picked from a small popup (keep / avoid / replace / more like / feature) |
| `Alt-R` | add the current I/O-marked range or playhead frame as `Range`/`Frame` |
| right-click → "Reference in chat ▸ <modifier>" | same, from the context menu |
| drag an item onto the assistant panel | add as reference (where the page and panel are both visible) |

## Engine calls a page may make directly

Read-only and cheap: `plan::Plan::load/save/timing`, `plan::check`, `refs::resolve`,
`people::People` lookups, anything on `Index`/`MusicMap`. Anything slow (decoding,
analysis, rendering, captioning) goes through `jobs.spawn` with a `Cmd`, or a page-owned
worker thread that never blocks the UI thread.

## Gates (every branch, before reporting done)

```
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Plus: every new `.rs` file has the header banner (`src/header_check.rs`), no `unsafe`
outside `src/win/`, no `unwrap/expect/panic`, `#[expect(..., reason)]` never `#[allow]`.
Add a `CHANGELOG.md` line under `## [Unreleased]` → `### Added` for what you built.

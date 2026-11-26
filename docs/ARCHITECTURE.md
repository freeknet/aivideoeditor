# Architecture

AI Video Editor is one crate with a library and a thin binary, `ave`. The library is in
three layers plus the engine, and the split is what keeps it testable.

```
src/
  main.rs            parse the command line; run a subcommand headless, or open the window
  lib.rs             module graph; NAME, TAGLINE, ACCENT, APP_ID; run_command
  cli.rs             the command line: window switches and the subcommands
  logging.rs         stderr logging, and the crash report for the window app
  color.rs           Rgb, OKLCh, the rainbow Ramp, WCAG contrast
  theme/             Palette (derived from a few stated colours) + readability rules
  motion.rs          the four durations, the easing curves, Tween
  icon.rs            every icon as polylines on a 16x16 grid
  brand.rs           the mark: geometry, colours from the accent, a rasteriser
  config.rs          preferences: forgiving per-field parse, atomic write
  command_palette.rs the commands and the fuzzy matcher, without egui

  media.rs           ffmpeg / ffprobe: finding them, probing, decoding
  analyze/           one clip -> samples and segments; moments, camera motion,
                     sets (the night's acts), bad spans, crowd cheers
  index.rs           a footage folder -> index.json + shotlog.md (cached per clip)
  caption.rs         vision-model captions for segments (llama.cpp)
  beats.rs           the music map: tempo, beats, bars, sections, drops -> music.json
  people.rs          who is in which clip: people.toml
  plan.rs            the plan (a TOML storyboard), its timing and the rule check
  draft.rs           drafts a plan from the index and the music map
  alternatives.rs    ranked replacements for one plan slot
  fix.rs             automatic fixes and plain-words advice for check findings
  search.rs          the shot-log query language; search/marks.rs: pick / reject / rating
  recipe.rs          the night's deliverables (lengths, aspects, file names, end cards)
  template.rs        recipes to start a project from (General, Flow Night, the user's own)
  render.rs          a plan -> mp4, one ffmpeg filter graph
  otio.rs            a plan -> OpenTimelineIO timeline
  proxy.rs           light H.264 playback copies of heavy clips
  project.rs         an open night: its work folder, ave.toml and the engine files
  jobs.rs            engine steps on background threads, with progress
  refs.rs            references: typed pointers at shots, segments, bars, people
  commands.rs        the subcommands (index, caption, beats, proxy, people, plan,
                     check, render, otio, mcp, show)
  mcp.rs             `ave mcp`: the engine as MCP tools over stdio

  gui/
    run.rs           the runtime: opens the window and drives App (the only eframe code)
    app/             App: state, Action, and the one place actions are applied
    ui/              the shell: theme, widgets, chrome, top bar, workspace layout,
                     keymap, modal, settings, command palette, shortcut sheet
    pages/           the workspace's regions: Home, Shots / People / Music, viewer,
                     inspector, timeline, export
    assistant.rs     the chat panel (+ assistant/: model, agent loop, providers,
                     Claude Code runtime, tools, edit ops, skills, chat store)
    player.rs        the shared viewer (+ player/: video, audio, clock, transport)
    thumbs.rs        thumbnails decoded on worker threads, cached
    font.rs          the bundled faces and an installed UI font by family name
    icons.rs         the mark painted with egui, and the window icon
  win/               every Win32 call, as safe leaf wrappers

  header_check.rs    test: every source file opens with the header banner
  unsafe_check.rs    test: `unsafe` stays inside its boundary
```

`examples/` holds tools that use the library without the binary: `walkthrough` and
`screenshot` drive the real app headlessly, `brand_assets` writes the mark's files,
`changelog` regenerates `CHANGELOG.md`. `tests/cli.rs` runs the subcommands end to end.

## The portable core

`color`, `theme` and the other top-level modules
name no window and no Windows type. Their tests — the WCAG checks on every theme, the easing
curves, the config parser — run on any machine, including a CI runner with no GPU.

## The engine

The engine turns footage into an edit and needs no window either. Each step reads and
writes plain files in the night's work folder (usually `<footage>/_ave`): `index.json` and
`shotlog.md`, `music.json`, `people.toml`, `plans/<name>.toml` (every save also keeps
`plans/.history/<name>/v<N>.toml`), `recipe.toml`, `review.toml`, `ave.toml` for the
night's settings, and `renders/`, `proxies/`, `audio/` for what is made. So the command
line, the window and the chat agent always agree on what is on disk.

`ave <step>` runs any step headless (`commands::Cmd`). The window runs the same steps
through `jobs`, and `ave mcp` serves the engine as tools to an agent. The chat panel
starts Claude Code with `ave mcp --work <night>/_ave` as its only MCP server; its other
providers (the Claude API, a local llama.cpp server) see the same tool set, run by the
same code (`mcp::call`), so the editing knowledge lives in one place. The agent's skills
are in `agent/skills/<name>/SKILL.md`.

`plan::check` returns findings, hard (the render would be wrong) or soft (a taste rule
bent). `fix` proposes edits that clear a finding without adding a hard one; a soft finding
can instead be accepted in the plan's `accepted` list.

## The GUI

`gui::app::App` uses egui only. The runtime in `gui::run` owns the window and calls
`App::show(ui)` (or `App::ui(ctx)` inside `Context::run_pass`) once per frame, and
`App::on_exit()` on a normal quit. Swapping the runtime touches that one file.

Drawing never mutates the app directly: `gui::ui::draw` returns `Action`s and `App::apply`
carries them out. A button, a keyboard shortcut, a menu entry
and anything you add later are one code path. Shortcuts come from one table,
`gui::ui::keymap::COMMANDS`.

The window is one workspace (`gui::ui::workspace`): chat, browser, viewer, inspector and
timeline, with export docked on the right and Home in place of all of it when no night is
open. `docs/GUI_CONTRACTS.md` is the contract for the regions.

Every colour comes from the active `Palette`; every margin from the spacing scale in
`gui::ui::theme`; every animation from `gui::ui::motion` (and the helpers in
`gui::ui::widgets` that wrap it), so no call site picks its own duration.

## Themes

`assets/themes.toml` is compiled in. Each theme states eleven colours; `Palette::derive`
works out the rest. `*.toml` files in the user theme directory (shown in Settings) load on
top, and a theme whose `id` matches a built-in replaces it. A theme that fails
`Palette::readability_issues` is reported in Settings and not loaded; a built-in one that
fails is a failing test.

## The Win32 layer

`src/win/` is the only place with `unsafe`. Each FFI call sits alone in a private function
with a `// SAFETY:` comment, handles are owned by types with `Drop`, and nothing outside
the module needs `unsafe` to use it. `src/unsafe_check.rs` fails the build otherwise (see
`AGENTS.md`).

## Adding a page

1. Add a variant to `Page` in `gui/app/mod.rs` (label, key, glyph) and to `Page::ALL`.
2. Give it a `State` and `draw` in `gui/pages/`, a field in `pages::States`, and draw it
   where it belongs in `gui/ui/workspace.rs`.
3. Add a `go-…` row to `keymap::COMMANDS` that returns `Action::Go(page)`; the tests
   will remind you.

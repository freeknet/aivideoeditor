# Changelog

All notable changes to AI Video Editor, newest first.

Entries follow the conventional-commit subjects in the git history
(`feat:` under Added, `fix:` under Fixed, and so on). See `CONTRIBUTING.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Start and processing flow, after Selects: a start page (drop zone or "Open footage
  folder", a first request with templates that goes to the chat once the night is set up,
  recent nights as thumbnail cards), a get-started wizard (footage → music and end cards
  with last week's carried over → deliverables → an analysis approval card with time
  estimates from measured throughput), and a processing page (a bar per step, elapsed and
  remaining time, Stop, every clip's status, "Open editor" as soon as indexing is done).
  `ave index` and `ave caption` report per clip / segment progress with ETA
  (`index 12/81 MVI_7925: done 3.2s eta 40s`), beats/plan/render report stages, every
  step can be cancelled between items, and `<work>/status.json` keeps per-clip status,
  failure reasons and measured speeds.
- Music tab: whole-track overview and FCP-style zoomed beat grid (sections, drops,
  song ranges, playhead), grid fixes (drag the downbeat, `,`/`.` beat, Shift 10 ms
  offset, tempo, `D` drop, rename sections, re-analyse), per-deliverable song ranges,
  and bar selections as `@bars` references. `ave beats` now also finds song sections
  (intro / build / drop / break / outro) on phrase lines, scores drops by their build,
  keeps an energy envelope, and stores the user's grid corrections in `music.json`,
  applied on load and kept across re-analysis.
- Project scaffolded with create-rust-app.
- Assistant chat panel: a conversation per plan (`<work>/chat/<plan>.jsonl`) with
  Claude (Messages API, streamed, adaptive thinking, prompt caching; key from
  `ANTHROPIC_API_KEY` or the panel settings) or a local llama.cpp server as the offline
  fallback. Reference chips and an `@` autocomplete with intent prefixes, scope chips
  (whole plan / selected shots / before / from the drop) enforced on every edit, tools
  over the shot log, and edits only as proposal cards: each operation before → after
  with thumbnails and its reason, the predicted `check`, apply all or some, preview in
  the viewer, discard. Applying writes `plans/.history/<plan>/v<N>.toml`; History lists
  applied proposals and versions to play or restore. Clarifying questions come back as
  answer buttons; an optional per-night setting applies proposals that keep the check
  clean.
- Viewer: real decoded playback (an ffmpeg decoder thread per shot into a frame ring,
  next shot pre-rolled so cuts land on time) with sound through cpal as the master
  clock (a cut plays its music from the plan's offset; clips their own sound; silent
  fallback with no device); 16:9 / 9:16 / 4:5 framing around `cx` with a full-frame
  mode, safe-area and TikTok/Reels zone guides, an info overlay (Ctrl+Shift+I), a
  timecode transport with I/O range, loop and volume, J/K/L shuttle, frame steps, and
  Alt+R to reference the marked range or frame in chat.
- `ave index`: per-clip analysis over ffmpeg (exposure, focus, fire / LED / white
  emitters, motion-compensated moving light, camera pan and zoom by block matching,
  colour cuts), segmentation by camera behaviour, shoot-relative scores and flags,
  cached per clip; writes `index.json` and a text `shotlog.md`.
- `ave caption`: schema-constrained captions from a local Qwen3-VL on llama.cpp
  (starts the server when `AVE_LLAMA_DIR` / `AVE_VLM_MODEL` / `AVE_VLM_MMPROJ` are set),
  saved every 24 segments.
- `ave beats`: tempo, dynamic-programming beat tracking with the grid shifted onto
  measured kick onsets, bars, and drop candidates.
- `ave people`: drafts `people.toml` from caption descriptions and recording times.
- `ave plan` / `ave check`: TOML storyboard plans on the beat grid, drafted from the
  index under configurable rules, and a checker for hand-edited plans.
- Shots browser: search over the shot log (free text over captions, people and clip
  ids, plus facets such as `fire>0.5`, `-flag:soft`, `move:static`, `person:`, `pick`,
  `like:<id>` more-like-this) with a score and a match reason per result; thumbnail
  grid with hover scrub and metric bars, or a dense virtualised list; clip stacks,
  score cut-off, removable filter chips, smart searches; P / X / U pick-reject with
  auto-advance and 1-5 ratings saved to `<work>/review.toml`; right-click to reference
  shots in chat.
- `ave render` / `ave otio`: one-graph ffmpeg render with a 720p preview, and
  OpenTimelineIO export.
- GUI People tab: performer cards (frames cropped on the action, clip/segment counts,
  props) plus Unassigned; inline rename, merge (Ctrl+M), split (Ctrl+Shift+M), drag or
  "Move to ▸" segments between people (partial clips become `clip@from-to` ranges),
  "Assign range…", a Y/N/S confirm queue ranked by caption likeness, undo/redo, and
  "Reference in chat ▸" / "Avoid this person in plans". Every edit saves `people.toml`
  atomically with its descriptive comments regenerated.
- GUI inspector: everything about the selection: shot-log numbers, each flag with the
  threshold that set it, the full caption, who, which plan shots use it, the clip's
  segment strip; plan shot fields and beat position; bar energy and drops; a person's
  frames, entries and shots; a compact list for several items.
- Weekly recipes (`recipe.toml`): the night's deliverables (id, aspect, length, end
  card, file name template with `{night} {id} {aspect} {version} {date}`, output
  folder, encoder), parsed per row, carried from night to night.
- GUI Home: recent nights with first-clip frames and progress, "New night…" from last
  week (recipe, end cards, track), the pipeline as a step bar on one serial job queue
  with "Run everything" (runs only what is missing), live progress, settings rows and
  a recipe editor.
- GUI Export: deliverables × plan versions with check status, file names before
  rendering, a serial render + OTIO queue that copies each video to its delivery
  name, "render anyway" for failing plans, reveal / open folder / copy path.
- Night-tuned technical cull: exposure is measured on the lit region (pixels
  above black), so a fire spinner on black is no longer `dark`; per-night
  thresholds in `ave.toml` `[thresholds]` (or `[pages.thresholds]`); every flag
  records its measured value and threshold, and the shot log prints them.
- Moment detector: a per-sample burst signal (onset of fire / LED area and
  moving light), with each segment's `peak_t` and `moments` in the index and
  the shot log.
- Look-alike rule: each segment stores a signature (colour histogram, emitter
  layout grid, action centre); `ave check` flags two shots from different clips
  closer than `lookalike` (default 0.12, calibrated on 2026-11-11) unless the
  people map names different performers.
- Cross-week memory: `ave plan` avoids, and `ave check` rejects (rule
  `reject_reused`), moments used by sibling nights' plans
  (`<night>/../*/_ave/plans/*.toml`), matched by clip id, file size and range.
- Planner: windows centre on each segment's best moment; the drop puts the
  biggest burst on the downbeat (in-point at the burst onset); shot lengths
  follow per-bar energy through a pacing DP that penalises uniform lengths, and
  the pace eases when distinct footage runs out; the draft avoids look-alikes.
- Render: exposure matched on the lit region, mid-tone colour matching to the
  drop shot (black, highlights and emitters excluded, gains clamped), and an
  optional cut ahead of the beat; both set in the plan's `[finish]` table
  (`match_colour`, `cut_lead_frames`).

### Fixed

- Drafted crops are rounded inward so a two-decimal `cx` never leaves the frame,
  and windows never run past the end of the file.
- GUI timeline: plans on the beat grid (ruler in seconds and bars, V1 shots with
  thumbnails, A1 music waveform, drop flag, playhead synced to the viewer), edits
  with the music as the clock (reorder keeps slot lengths, roll edit points on
  beats, Alt-drag slip, lift, crop-centre framing, notes), undo / redo, live
  `check` with a problem list and red edges, autosaved versions in
  `plans/.history/<name>/` with view and restore, "Draft new…", context menu with
  chat references, and ranked swap alternatives per slot (`A`, `1`–`8`).
- `ave mcp --work <night>/_ave`: the engine as MCP tools (overview, shot search, segment facts, `look`
  contact sheets, music map, plan get/edit/draft with stage-then-commit, alternatives, versions, revert,
  people), so Claude Code or any MCP client can edit inside a night.

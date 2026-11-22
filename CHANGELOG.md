# Changelog

All notable changes to AI Video Editor, newest first.

Entries follow the conventional-commit subjects in the git history
(`feat:` under Added, `fix:` under Fixed, and so on). See `CONTRIBUTING.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The night's acts in the Shots browser. The order menu's new *Group by → Acts* puts
  the shots in recording order under a full-width header per set (`Set 3 · Indoor neon ·
  LED · 20:35–20:42 · 4 clips`, with its shot count) in both the grid and the list,
  which stay virtualised; a click folds an act away and the header's filter button scopes
  the search to it. A new `set:3` search token (also `act:3`) keeps the shots of that act,
  for the search bar and the chat agent's `search_shots`. The inspector's Facts (and the
  facts a reference gives the chat) now open with the clip's recording time on the camera
  clock, its camera and its act.
- Plan check findings carry a stable id and a severity: `plan::check` returns
  `Finding { id, severity: hard | soft, shot, seam, message }`, and `ave check` prints
  each as `<plan> <where> [<severity> <id>] <message>` with a verdict line. **Only hard
  findings fail now** (exit 1; `ave render` refuses them without `--force`): the plan
  is invalid or would render wrong (`beat-count`, `bar-order`, `past-track`,
  `off-drop`, `missing-clip`, `outside-clip`, `bad-span` (a bad span with a rejected
  flag), `crop-outside`, `empty-shot`). The taste
  rules (`drop-subject`, `flagged`, `bad-moment`, `wide-action`, `same-stretch`,
  `repeat-performer`, `lookalike`, `reused`) are soft: reported, never fatal. New seam
  checks across each cut, all soft: `jump-cut` (one clip cut to itself skipping under
  10 s), `exposure-jump` (over 1.5 stops of frame brightness between the edge samples,
  the cut onto the drop spared) and `colour-jump` (mid-tone chromaticity moving over
  0.40); plus `flash-shot` (under 6 frames, soft) and `empty-shot` (no frames, hard).
  The MCP's plan text and `plan_edit` predictions, the assistant's check line, the
  Deliver page and the timeline's check bar read the severity: the bar counts hard and
  soft findings, says "Ready to render" whenever nothing is hard, lists findings hard
  first, and clicking one selects its shot, or both shots around a seam finding's cut
  with the playhead on it.
- Crowd cheers as a measured moment. `ave index` measures each clip's camera audio as a
  10 Hz loudness envelope in three bands (full, below 250 Hz, above 2 kHz), cached under
  `<work>/audio/` by the clip's stamp, so cached clips gain cheers without their video
  being analysed again. A cheer is a sustained (0.4 s or longer) treble-heavy rise over
  a rolling 5 s baseline that does not take the bass away, so steady music, drum hits,
  a song getting louder and a breakdown are not cheers. Each segment carries its cheers
  (onset, length, rise, strength 0..1; old indexes load without them), the shot log
  prints `cheer 0.66 @6.71`, search finds them with `moment:cheer` and `cheer>0.5`, MCP
  `segment_details` lists fire-burst and cheer moments, and `ave plan` also tries
  windows whose out-point lands just after a cheer, scoring them up to a quarter higher.
- Recording time, camera and sets. The probe now reads when each clip was recorded
  (`recorded_at`: UTC, the camera's local clock when known, and its source) and the
  camera make / model from the container tags. A `creation_time` written in `Z` from a
  camera's local clock (Canon EOS, many DJI drones) is corrected by the file time; with no
  tag the file time less the duration is used and marked `mtime`. Re-running `ave index`
  fills both for cached clips with one ffprobe each, no re-analysis. Clips are grouped
  into `sets`, the night's acts: runs split by pauses over 15 minutes and by lasting
  indoor/outdoor changes in the captions, each titled from its captions and props
  (`Outdoor parking lot · fire`). `index.json` stores them, `shotlog.md` lists them and
  heads each set's clips in recording order, and `night_overview` shows them to the
  chat agent.
- Bad spans inside segments: a half-second focus hunt, whip, jolt, or dark or blown
  moment inside an otherwise good segment is now recorded as `bad_spans` (start, end,
  flag, worst value), found from the 6 Hz samples against the night's thresholds when
  the index is scored, so an existing index gains them on the next `ave index` without
  re-measuring. The shot log prints them on a `bad:` line, `segment_details` shows them
  to the chat agent, `ave plan` shifts a shot's window clear of any span with a rejected
  flag, and `ave check` reports a shot that overlaps one.
- Focal time: every search hit carries its segment's best moment (`peak_t`, the
  strongest burst's onset or the peak of the action, kept inside the segment). Opening a
  hit in the Shots browser, an `@` / `ave://` segment link in chat, or a segment from the
  inspector seeks the viewer there instead of the segment's first frame, and MCP
  `search_shots` lists it on each hit (`focal 12.5s`).
- Search says how much of the night it could see: results carry `complete` and
  `uncaptioned_in_scope` (segments that pass the measured filters but have no caption,
  so free text and caption facets could not see them). The Shots browser shows it as a
  quiet "312 shots · 41 not captioned", and MCP `search_shots` opens with "complete" or
  "PARTIAL … uncaptioned_in_scope=N" so the chat agent knows when results are partial.
- Markers on plans, end to end, as one concept with review comments (Selects' marker:
  a moment, a colour, a note that is the comment). `M` adds one at the playhead (`F`
  still works), `Shift+M` / `Alt+M` go to the next / previous one (`Ctrl+Shift+M` under
  the Premiere preset), a click on a ruler pin selects it, a double-click opens its note
  and colour swatches, and `Delete` removes the selected marker when no shot is
  selected. Plans also read `[[marker]]` tables; version restores, reverts and "adopt B"
  keep the live plan's markers. OTIO export writes them as timeline markers with their
  colour and note. The chat agent sees numbered markers in `plan_get` and edits them
  with the `add_marker`, `remove_marker` and `resolve_marker` ops of `plan_edit`.
- A Shot list tab in the inspector, Selects' Transcript for a cut: the open plan's shots
  in timeline order under their song sections, each with its number, time (MM:SS.s),
  bar, source segment, caption line, the plan's note and a quiet icon for its
  `plan::check` problems. Clicking a row selects the shot and seeks the viewer to it,
  the row under the playhead carries a lime marker while the plan plays, Up / Down walk
  the rows, Enter plays from one, Delete lifts it through the timeline's undoable lift,
  `R` and the right-click menu mention it in chat.
- Chat cards for new plans and slow steps. "Suggest a first cut" (or the agent calling
  `plan_draft` with no length or aspect, which now answers `SETTINGS_NEEDED`) shows a
  "Here's our plan" card: length 15/30/60/custom, aspect, pacing, a hero shot from the
  selection and switches for no repeated shots, people spacing and drop on the hero.
  Continue starts the same `ave plan` job as the plan page's "Draft new…" window, writes
  the switches into the plan's rules and puts what needs judgment (pacing, the hero) in
  the composer for the agent. A new `run_step` tool (index, caption, render) never runs
  anything itself: the chat shows "Allow this?" with the step's estimated time from the
  night's measured speeds, and Approve starts the job with a live progress row (percent,
  time left, Stop); with "Let it apply edits" on, steps start at once, labelled
  auto-approved. The proposal card is now covered by a headless test (Apply writes a plan
  version) and the screenshot example shoots all three cards.
- A design system matched to Selects' dark workspace (`docs/DESIGN.md`): its exact
  tokens (neutral surfaces, white-10 % hairlines, ink overlays, neutral selection,
  lime key, 6/8/10/12/14 px radii, 4 px grid, 10/11/12/14 px type), the Mona Sans
  (three weights), Roboto Mono, Inter and Lucide fonts bundled with their licences
  (`assets/fonts/NOTICE`), named Lucide icons (`gui::ui::icons`), and components every
  panel can use (`panel_header`, `icon_btn`, `button_with`, `tab_strip`, `segmented`,
  `pill`, `kbd`, `empty_state`, `text_field`, `menu_item`, `glass_button`). Default egui
  widgets follow it too. The shell uses it: a 52 px top bar with a centred search pill
  and Lucide buttons, edge-to-edge panels with one hairline between them and a focus
  outline on the panel last clicked, 40 px panel headers, a 44 px icon rail for the
  folded chat and glass buttons over the viewer for a folded browser or inspector.
- Home and Export in the Selects style: the start page is one centred column with
  "What are we making?" in a composer box (paperclip and send ask for the footage),
  prompt templates as pills in category rows (two new ones: who performed tonight, a
  15 s story), a dashed drop zone, a whole-window drop overlay and the nights as 40 px
  rows with frames and status badges; the get-started wizard is a stepper over one
  stepped card; the processing page is a centred step list (spinner or check per step,
  8 px progress bars, remaining time, Stop as an outline button); the night page's
  settings and recipe are form rows with path fields, text fields, a segmented encoder
  and a switch; the export panel lists deliverables with status badges, versions as
  tickable chips and icon buttons for each file. Lucide icons replace every glyph.
- The viewer and inspector in Selects' style. The viewer drops its toolbar: the
  picture is letterboxed on the panel above a 48 px transport with a 4 px knobless
  scrub bar on its top edge (I/O range and shot boundaries on it), Lucide skip, play
  and loop buttons, a `1x` speed menu, a centred `HH:MM:SS:FF / HH:MM:SS:FF` timecode
  in fixed digit cells, and a gear menu holding aspect, guides, the info overlay,
  preview quality and volume. The inspector gets an Inspector | Facts tab header,
  12 px label/value rows under hairline sections, neutral links and quality colours,
  and centred empty states.

- The browser's Shots, People and Music tabs restyled as Selects' Project panel: one
  40 px borderless toolbar of Lucide icon buttons under the tab strip (search, a
  filter popover holding the minimum score, stacks, hover scrub and card size, the
  order as a ghost dropdown, smart searches; Tape / Compare / Survey as icon toggles
  with their keys in the tooltips, folded into one menu when the pane is narrow; the
  grid / list toggle). Shot cards are Selects' media cards: 16:9 thumbnails at most
  160 px wide, glass duration, mark, rating and stack badges, the name in 12 px
  Medium over one muted caption line, a 2 px selection outline (lime while the tab has
  the keyboard, grey otherwise) and an ink hover; the measures moved to the hover card.
  List rows are 28 px with a `#525252` selection. People and Music use the same
  toolbar, cards, 28 px rows and empty states, with hint lines moved into tooltips.
  New shared helpers: `widgets::dropdown_button` and `widgets::toolbar_row`.

- The timeline restyled to Selects' canvas: plan tabs, compare, storyboard, comments,
  versions, aspect, undo and redo in one 40 px header; a 35 px ruler with `MM:SS`
  clock labels and a marker lane (bars, drop and end-card pins, comment pins); a 96 px
  track-header column with eye / mute / solo icons; shots as filmstrip clips coloured
  by subject from Selects' clip-label palette, with a lime (focused) or white
  selection outline and a striped band on shots the check fails; the music as an
  audio clip with bars growing from the bottom and red clip marks; Selects' playhead,
  in/out shading for the selection, a thin scroll thumb whose ends zoom, fit / − / +
  buttons, and a floating tool island (selection, slip, zoom and hand tools). The hint
  line is gone (hints live in tooltips) and the check bar shows Lucide count pills.
  Compare, storyboard and comments use the same tokens.
- Chat tabs and history, as Selects: a night holds any number of chats, each about a
  plan (an attribute) with its own log (`<work>/chat/chats/<id>.jsonl`, listed in
  `chat/chats.json`) and its own Claude Code session. Old per-plan logs are adopted on
  load as chats titled after their plan, read in place until next written. The Chats
  menu lists recent chats with plan and relative time, New chat (Ctrl+Shift+N), Load
  older, Rename, Archive and Delete (confirmed); a wide panel shows the open chats as
  28 px tabs with a close cross and "Close other tabs". A new chat is titled from its
  first message; double-click the title to rename. Find in chat (Ctrl+Shift+F; Ctrl+F
  stays the workspace search) highlights matches with a counter and next / previous.
  A message sent while a reply streams waits ("Waiting to send", "Send now" to stop
  the reply and send it, "Cancel and edit"); Arrow Up on an empty composer recalls the
  earlier prompts with their references; the composer shows "Context N%" when the
  runtime reports how full the context is.
- The assistant panel follows Selects' chat: a 40 px header with a Chats menu (the
  plan picker and New chat), the plan's name and History / settings icon buttons; an
  empty state of one quiet line and outline-pill quick actions; your messages as an
  outlined bubble on the right and the assistant's as plain 14 px text, with time and
  copy on hover; thinking and tool calls as quiet 12 px rows that expand; proposal and
  question cards with primary and ghost buttons; and one dark composer box holding a
  Chat / Comments switch, scope pills, the 14 px input with references in lime, and a
  toolbar of 32 px controls (add, the model picker with permissions, send / stop).
  Comments mode lists the plan's review comments (click to play from there) and puts
  the open ones in a message. Lucide icons replace every symbol glyph in the panel.

- Timeline playback ends like the render: after the last shot the viewer holds the
  plan's end card for the render's span (`card_bar` to `end_bar`, `Plan::card_span`)
  over the music, and V1 draws it as a block there. Plan compare (A/B): "Compare" in
  the tab strip (`Shift+C` for the previous version) stacks the open plan over another
  plan or a saved version on one beat grid, highlights the shots that differ, lists
  them as added / removed / replaced / moved / changed with both shots' notes, plays
  A or B in the viewer (`B`), and "Adopt B" saves B as a new version of the open plan.
- Shots browser review modes, switched from the header or by key: **Tape** (`T`) plays
  every result back to back in the viewer with the playing card lit and the selection
  following the playhead, `P`/`X`/`U` marking it and skipping on, and Fast Review
  (`Shift+L`) playing long shots faster; **Compare** (`C`) puts the select and a
  candidate two-up with their shot-log numbers and captions side by side (arrows move
  the candidate, Up promotes it); **Survey** (`G`) tiles the selection, `X` drops the
  focused shot until one remains and `]` / `[` load the next / previous group. The keys
  are rows in the command table, so the shortcut sheet lists them and presets apply.
- People, Music, Viewer (transport and player) and Export read their keys from the
  command table under the chosen preset (ave / Premiere Pro / Final Cut Pro), and the
  shortcut sheet lists them: Premiere clears marks with Ctrl+Shift+X, plays In to Out
  with Ctrl+Shift+Space and merges people with Ctrl+G (its Ctrl+M is Export); Premiere
  and Final Cut mark a music drop with M; Final Cut zooms the waveform with Cmd+= / Cmd+-.
- Chat panel runs on Claude Code (the default when `claude` is on PATH): one headless
  `claude -p` stream-json session per conversation with `ave mcp` as its only tool
  server (no shell, no file edits, no user settings or MCP servers), resumed across
  restarts, with live tool status lines, `look` contact sheets inline, cost per turn,
  Stop, a repeat-call guard and a clear "log in once" message that falls back to the
  Claude API or local model. The API and local providers now use the same `ave mcp`
  tools in-process. Staged `plan_edit`s become Apply / Discard cards; `plan_edit`
  takes `base_version` and refuses stale commits, and the agent may commit only when
  the night's "Let the assistant apply edits" permission is on. The per-turn reminder
  carries references, selection, playhead and matching skills (`agent/skills/`:
  beat-sync-reel, suggest-moments, fix-check, weekly-deliverables); `ave://` links in
  replies render as chips that select and show the shot, segment, range or bars.
- Timeline works like Selects: plans as tabs above the tracks (unsaved dot, failed-check
  count, close / reopen, rename, duplicate as new plan, `+` drafts one; a plan written by
  the assistant or the CLI appears as a tab), review comments as `[[comment]]` in the plan
  (`F` at the playhead, coloured pins in the ruler, a list to seek / edit / resolve /
  delete, "Send to chat" hands every open comment with its `@shot` reference to the
  assistant asking for a plan to approve), a read-only banner while
  `plans/.lock-<name>` is held by a chat edit, and a storyboard lane of shot cards with
  their notes grouped by song section.
- Workspace laid out as Selects: chat 22 % full height, browser 21 % · viewer 58 % ·
  inspector 21 % over a timeline at 45 % of the height, every split a fraction
  remembered in the config; chat, browser, inspector and timeline fold to labelled
  rails (click to unfold) with top-bar toggles that mirror them; view modes Default,
  Vertical (9:16 viewer centred), Comment and Preview (`Alt+1…4`, persisted); a
  centred top-bar search with a popover of recent searches and facet hints; one
  command table driving shortcuts, the palette and a `?` shortcut sheet, with ave /
  Premiere / Final Cut presets in Settings (Selects' `Shift+1…4` panel focus); a
  shared `widgets::mention_menu` for "Mention in chat ▸ intent" in any context menu;
  the signature theme on Selects' tokens (neutral-900, white-10 % hairlines, lime
  `#a3e635` key), 12/14 px type, tighter controls, and Inter (SIL OFL 1.1) bundled
  as the UI face.
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

- Delete / Backspace in the inspector's Shot list only lifted the selected shot while
  the timeline was open; it now lifts it whenever the list has a selection, through the
  timeline's undoable lift, saved at once as a new plan version.
- The timeline's A1 waveform no longer shows red "clipped" marks along a loud but
  mastered track. The music is now decoded at its own rate and channels (ffmpeg's
  stereo-to-mono downmix summed past full scale, and resampling rang over it), and a
  column is marked only where a channel holds three consecutive samples at 0.999 of
  full scale or more. Bars are 1.5 px on a 2 px pitch at 55 % white, so a loud master
  reads as a shape rather than a white wall.
- Short V1 clips no longer show cut-off names ("4 M", "15 I"): a clip draws its name
  only when it is at least the text's width plus 12 px, else just the shot number if
  that fits, else nothing, and the text never runs past the clip.
- The timeline's Zoom and Hand tools have shortcuts, Z and H (the same in Premiere
  and Final Cut), listed on the shortcut sheet and shown in the tool island tooltips.
- Symbols (▾ ✕ ▶ …) draw on Linux and macOS: the symbol-font fallback now looks
  in the per-family folders (`/usr/share/fonts/truetype/dejavu`, `…/noto`,
  `/System/Library/Fonts`) and loads every fallback it finds, not just the first.
- `ave index` no longer stops the night at one unreadable clip: a clip that fails
  (ffmpeg error, no frames, garbage file) is skipped with a warning, recorded in
  `index.json` (`skipped`, with the reason) and listed at the top of `shotlog.md`;
  the command prints `N clips, M segments, K skipped` and fails only when every clip
  does. The processing page shows the skip count on a finished index step.
- Every input has an accessible name (the recipe editor's deliverable rows, the chat's
  model picker and local-server field, People's dialogs, Music's tempo and range
  fields), and the screenshot example fails on any unnamed widget again.
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

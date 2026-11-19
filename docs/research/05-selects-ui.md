# 05 — Selects: user interface and workflow, reverse-engineered

What the installed Selects desktop app (Cutback, v2.1.301) actually shows
and does, recovered from its shipped code, and what `ave`'s workspace
should change to work the same way. Companion to `04-selects-agent.md`
(the agent runtime) and `06-selects-data.md` (the project data model).

All UI text below is paraphrased or quoted as short labels only. No code
was copied.

## 0. Method and where the UI lives

- `resources/app.asar` holds only the Electron **main process**
  (`dist/main`), the MCP server, preloads, ffmpeg and native addons. There
  is no renderer in it.
- The window loads a **remote renderer** from
  `static.cutback.video/app-selects/<version>/main/`. The UI is a React +
  Tailwind (shadcn-style) single-page app, code-split into chunks
  (`TimelineRoute`, `TranscriptViewSnapshotContext`, `main`, `global`, a
  large i18n chunk, ...).
- `overlay-renderer/renderer.asar` is only the Remotion overlay renderer
  (media staging, MOGRT assets), not the editor UI.
- The renderer chunks were recovered read-only from the app's own Chromium
  HTTP cache (`%APPDATA%\Cutback\Cache\Cache_Data`, blockfile format,
  brotli bodies). Both 2.1.270 and 2.1.301 were cached; 2.1.301 is used
  here.
- Sources of truth inside the bundle: the English i18n dictionaries (one
  per feature area, `JSON.parse` blobs), the shortcut preset table (one
  row per command per editor preset), the panel layout table in
  `TimelineRoute`, and the Tailwind token stylesheet.

## 1. Screen inventory

| Screen | What it is |
|---|---|
| Welcome | "Welcome to Selects", tagline about idea to video in minutes, Get started / Login. |
| Main-tool picker | "What's your main editing tool?" Premiere / Final Cut Pro / DaVinci Resolve / Other. Sets the **shortcut preset** and the default handoff target. |
| Genre guide | "Selects works best with videos with speech"; offer to open a sample project (about 400 MB download) or use your own footage. |
| Referral and plan pages | Marketing questions, subscription/credits, trial. |
| Start page (home) | A big drop zone ("Drop your raw videos and cut straight to the good parts", "Add videos or a folder", "or drag them in anywhere"), a **prompt box** ("What are we making?" / "Describe your video, or drop in your files") with attached-file count chips, **category rows** (Shortform, Explainer, Longform, Ad, Micro drama) each holding prompt templates with `BLANKS` to fill (TOPIC, MINUTES, PERSON...), a "Choose a direction" follow-up builder of one-click refinements per template, and the **Projects** list (open, delete). Dropping anywhere starts a new project. |
| Get-started wizard | Alternative linear flow: what are you making → describe it in a sentence → do you have videos (yes / start from scratch) → add files or "add them later" → pick a starting template → reference video link (YouTube, Instagram, TikTok, Pinterest) or none. |
| New-project / prep page | Older "Where should the edit start?" task picker: Sync (multicam, audio+video), Timeline tasks (split by topic, assembly cut, clips for shortform, 5-minute summary, prompt a storyline, script-based edit, YouTube-reference edit), "Check that your files are in the right category. Drag to change.", paste script, file/length limits, a manual-vs-Selects time estimate. |
| Processing page | Step list with spinner per step (syncing media, analyzing sound bites, labeling stringout, removing unnecessary parts, generating storylines, finishing up), estimated time remaining, Stop, "grab a coffee, I'll ping you". |
| Editor (TimelineRoute) | The workspace (section 2). Has five **view modes**: Default, Vertical video, Comment, Preview, App. |
| Detached timeline window | The timeline panel can open in its own window ("Open in new window" / "Dock panel"). |
| Settings / Preferences | Handoff, Transcript, Timeline, Keyboard shortcuts (visual keyboard editor), Connectors, Plans & usage, System (browser use, search engine). |
| Modals | Analyze media for edit, Export video, Export transcript, Handoff to Premiere/Resolve/FCP/Avid, Export vs Handoff explainer, Relink missing files, Choose audio tracks, Import synced timeline, Delete confirmations, Reanalyze credit dialog. |

## 2. The editor layout

The layout is a nested set of resizable splitters (react-resizable-panels).
Sizes are percentages of the parent; "foldable" panels collapse to zero and
leave a labelled rail ("Chat", "Project", "Transcript").

```
root (horizontal)
├─ Agent chat ............ 22%  min 3   foldable   (always leftmost)
└─ Editor area ........... 78%  min 40  (vertical)
   ├─ Upper row .......... 55%  min 30  (horizontal)
   │  ├─ Project (source)  21%  min 15  foldable
   │  ├─ Preview (viewer)  58%  min 25
   │  └─ Side panel ...... 21%  min 15  foldable  (Transcript | Adjust tabs)
   └─ Timeline (draft) ... 45%  min 20
(+ optional right column: multicam setup 22%, or an App opened "in right panel")
```

Other view modes rearrange the same panels:

- **Vertical video**: chat | (upper row over timeline, 44%) | a tall 9:16
  preview column (34%).
- **Comment**: preview large on the left, then the side panel (24%) and the
  chat-in-Comments-mode (24%) on the right; built for reviewing a cut.
- **Preview**: viewer dominates; panels minimal.
- **App**: an App (plugin) gets the centre with its own preview (28%) and
  Adjust (28%) panes; toasts say "Timeline opened / updated" with a View
  button to jump back.

The panel layout is remembered; a header toggle switches view modes; the
first-run tip says "Drag title to dock this panel".

### 2.1 What each panel holds

**Agent chat (left).**
- A tab strip of chats ("New chat", close, "Close other tabs", rename,
  archive), a history list ("Recent chats", "Load older"), "Find in chat"
  with match counter.
- Two modes at the top: **Chat** and **Comments**.
- Empty state changes with project state: no media yet → "Scan media,
  build timelines... Just ask."; before analysis → a prep message with
  quick-start buttons ("Analyze media", "Sync my angles and audio",
  "Suggest a first cut", "Review for files to sync", "Skip sync and analyze
  now"); with a timeline → "Your assistant editor, here and ready." plus
  quick actions ("Suggest an edit", "Create an assembly cut", "Edit
  according to a script", "View chat guide").
- Composer placeholder also changes by state ("What do you want to do with
  this video?", "Ask about syncing...", "Refine the timeline, add markers,
  explore the cut").
- Composer controls: Attach (txt, md, srt, docx, text PDF, images, .mogrt;
  10 MB per file, 50 images per message, 45k chars of script), model picker
  (Selects credits / Claude / ChatGPT / Gemini, with effort levels),
  **permission mode** (Ask: asks before paid actions; Auto: accepts all but
  paid; Bypass: never asks), Fast mode, a context-usage meter ("Context
  42%"), Send / Stop. While a reply streams, new messages queue ("Waiting
  to send", "Send now" to steer, "Cancel and edit").
- Transcript of the turn: collapsible "Thinking/Thought", grouped tool
  activity ("Inspected project", "Searched", "Edited project", "Updated
  plan", "N tool calls"), sub-agent cards with turn counters, background
  tasks with Stop, "Worked" elapsed time, token totals.
- Interactive cards (section 5): "Allow this action?" approval, multiple
  choice "options" cards, "Here's our plan" settings card, analysis-cost
  approval, scheduled-task cards.
- Footer: "AI can make mistakes...", "Report an issue".

**Project panel (upper left, "Project").**
- File tree of media, folders, timelines ("drafts") and synced timelines,
  with List and Card views (folders only show in List view), an "All"
  filter, "Search project", and **"Find moments"** (visual search: "Find
  moments by what's on screen").
- "Add or create" menu: Media, Generate, Record, Blank clip, Folder,
  Timeline, Synced timeline, Import synced timeline.
- Per-item analysis state: Analyze N clips / Re-analyze / Relink, a
  running line "Analyzing 3 clips · 4m left", "Canceling…", unanalyzed
  badge.
- Topics: an analyzed timeline shows its topic/subtopic outline ("Topics
  show here"; empty: "Drag sources onto this timeline and their topics will
  appear here").
- Context menu (section 6).

**Preview (centre).** Program viewer with transport, safe guides, zoom
gestures, frame capture (Shift+E "Snapshot frame"), "Draw on frame"
markups that become chat attachments, "Compare with source" toggle
(Shift+\\), audio meter option, "Hold to see the original" in Adjust.

**Side panel (upper right): Transcript | Adjust.**
- Transcript: word-level transcript of the open timeline (or source),
  chapters/subchapters, speakers, deleted words shown greyed ("Show cuts"),
  silence length in frames, highlight repeats; editing by text (delete a
  word / sentence / chapter marks the cut), split / merge captions, find and
  replace (Cmd+F), spell check, proper-noun check, export.
- Adjust: per-clip effects and colour (basic, wheels, curves, LUT import),
  transform and keyframes; empty states "Select a clip", "No effects
  applied", "No adjustable settings."

**Timeline (bottom).**
- **Tabs of timelines** ("drafts") across the top: each tab is one
  timeline; Cmd+N "Create new timeline"; tab menu Close / Close other tabs;
  per-timeline Rename, Duplicate, Delete, Handoff, Export video, Export
  transcript, Mention in chat.
- Tracks: video tracks V1..Vn and audio tracks A1..An, plus caption track(s)
  (optionally one per speaker), marker lane, topic/subtopic lane, comment
  markers. Multicam clips can expand to show lanes per angle.
- Tools: Select (V), Blade (C), Zoom (Z), Hand (H), Track select forward /
  backward (A / Shift+A), Rate stretch (R); snapping (S); in/out (I/O); lift
  and extract; ripple delete.
- Main footage is locked: "You can't edit the main footage. Move clips to a
  new timeline to make changes." Analyzed synced timelines are locked too.
- While the agent edits, the timeline is read-only ("You can edit the
  timeline again once chat finishes").

### 2.2 Apps panel

"Apps" is a plugin/tool catalogue (Favorites; categories Edit, Text,
Visuals, Media, Custom; search). An app opens in the Left panel, Right
panel or a New window, can be favourited, and a crashed app offers "Fix with
assistant", which sends a repair prompt to chat. "Chat to build a tool" is
the empty-state hint: tools are authored by the agent.

## 3. Workflow, new project to export

1. **Start.** Drop a folder anywhere (or type a prompt with files on the
   start page, or run the wizard). A project is created and the files are
   imported ("Reading files 3 of 40…", "Adding 40 files…"); duplicates and
   unsupported files are reported.
2. **Prep check (chat leads).** The chat opens in prep state and proposes
   checking for files that need syncing or are duplicates. Quick buttons:
   "My clips need syncing" / "Review for files to sync" / "Skip sync and
   analyze now". Sync works by timecode or waveform and creates a synced
   (multicam) timeline; "Review unsynced clips".
3. **Approve analysis.** "Analyze media for edit" dialog: two scopes,
   **Visuals** (every frame: shots, framing, angles, so the best take and
   camera get picked) and **Transcript** (every word: edit by text, cut
   silences, search); together they identify speakers across cameras. Shows
   estimated time and credits left; re-analysis warns that transcript edits
   are replaced. If the agent asks to analyze, chat shows "Approve analyzing
   the video to go on" and waits.
4. **Analysis progress.** Per-file progress in the Project panel and a
   step list in chat/processing view; "Please stay here until the first part
   of analysis is done", then background with a notification ("Get notified
   when the project is ready"). Stop is allowed and resumable ("you won't
   pay again for files already analyzed").
5. **First draft.** "Suggest a first cut" or a typed request. For
   interview-type footage the agent shows a **"Here's our plan" card**:
   Target length, Pacing (Original / Slow / Medium / Fast / Rapid: trims
   pauses and fillers), Cut unnecessary parts (pre-roll, mic check, false
   starts, audio drops), Add b-roll (from your own source), Camera switching
   (none / follow speaker / auto / auto scene). Back / Continue. The agent
   then builds a **new timeline tab** ("Timeline "X": edit job started",
   "Working on your timeline…"); it never overwrites the source.
6. **Review.** Play the tab; read the transcript of the cut; jump between
   topics; mention clips in chat for targeted changes (section 4); or switch
   to **Comment** view mode and leave timecoded comments.
7. **Revisions.** Comments are markers at the playhead with a colour and an
   optional drawing. "Send comments to chat" (to the current chat or a new
   one) sends them all at once with a preamble asking the agent to propose
   an edit plan covering every comment and to **wait for confirmation
   before creating a new timeline** with the changes. Each revision is a
   new timeline tab; older tabs remain to compare. Undo/redo and a "time
   machine" restore exist for edits.
8. **Export or handoff.** Header **Export** menu: Video, Audio, Transcript,
   Project, Socials, Advanced; and **Handoff to** Premiere / Final Cut Pro /
   DaVinci Resolve (and Avid). "Export vs. Handoff" explains: export renders
   everything; handoff carries project structure, timelines, clip placement,
   cuts and transcript but not overlays, motion graphics, filters, colour,
   transitions or transforms. Export dialog: resolution, range (In to Out or
   entire timeline), frame rate, pixel aspect, video/audio bitrate, sample
   rate, channels. Transcript export: edited clips or full transcript, txt or
   SRT, a format template (plain / with timestamps / with speakers) with a
   live preview.

## 4. Referring to timeline items in chat

Selects does **not** auto-attach the current selection to the next message.
References are explicit **@-mentions** written into the composer text:

- **Typing `@`** opens an autocomplete of project items (media, folders,
  timelines, synced timelines, clips). The `@` must start a word (preceded
  by whitespace); the query is the text after it, up to about 40 characters
  back from the caret. Arrow Up/Down move, Enter or Tab inserts, Escape
  closes. Each row shows a 16 px thumbnail and a kind or duration
  ("Timeline", "Synced timeline", "Folder · 12 files", "0:42").
- **Right-click → "Mention in chat"** on Project panel items (one or many)
  and on **selected timeline clips** inserts the same mentions. Timeline
  clips are inserted in timeline order (by position), named after their
  clip/graphic name.
- In the composer a mention renders as coloured text (`text-key`: lime in
  dark mode, olive in light) via an overlay over the textarea; the raw text
  stays plain.
- On send, each `@Name` is expanded into `@Name [descriptor]`, where the
  bracket carries the machine reference: `resource:<id>` plus media type,
  `draft:<id>` for a timeline, `clip:<id>` plus the timeline it lives in
  (`draft:<id>` or synced) and whether it is an adjustment layer or a
  graphic preset, `folder` plus file count; and for any of them the duration
  and "analyzed" / "not analyzed". The model sees both the human name and
  the exact ids.
- Arrow Up in an empty composer recalls earlier prompts of this chat, with
  their mentions; the composer has its own undo stack (300 steps).
- Other attachment kinds: files (scripts, images, MOGRT), pasted long text
  becomes a "Pasted text N" chip, **frame marks** (a drawing on a captured
  frame, named by timecode), and comment drawings.
- Comments are the second route: timecoded markers sent to chat in a batch.

## 5. Approval and plan cards

- **Permission prompt**: "Allow this action?" Approve / Cancel. Governed
  by the chat's permission mode; paid actions (credits) always ask unless
  Bypass. Auto-approved actions are labelled as such.
- **Options card** (the agent asks a question): single or multi-select
  choices, one marked "Recommended", "Something else" free text, multi-step
  ("Step 2 of 4", Back / Next / Skip), can be minimised; once answered it
  collapses to "chose X" with time to answer. If unanswered under Auto mode
  the recommended choice is picked and labelled.
- **"Here's our plan" card** (first cut settings, section 3.5).
- **Analysis approval** and credit-estimate cards.
- **Plan confirmation** is conversational: the comment preamble and the
  agent's rules make it propose a plan and wait; the edit then lands as a
  new timeline, so "reject" is just not continuing or deleting the tab.
- "Continue" button after an interrupted turn ("What was done before it
  stopped is kept").

## 6. Interaction details

- **Context menus.** Project item: Open synced timeline, Rename,
  Duplicate, Mention in chat, Delete, Handoff/Export to tool, Export video,
  Export transcript, Reveal in Finder / Show in File Explorer, Analyze /
  Re-analyze, Adjust gain, Add to new timeline, Add to current timeline,
  Sync, Create folder. Timeline clip: Mention in chat, Open synced timeline,
  Sync selected, Locate in project, Rename clip, Color label, Link/unlink.
  Tab: Close tab, Close other tabs.
- **Drag.** Media from Project to timeline; drag ghost labelled "Clip from
  NAME" / "Clips from N items"; stock/music browser items have their own drag
  types; dropping files anywhere on the start page creates a project.
- **Hover.** Tooltips carry the shortcut; comment rows say "Double-click to
  edit", "Jump to this point"; mentions in autocomplete highlight on hover.
- **Empty states** are specific and actionable everywhere ("Create a
  timeline to leave comments.", "Add a transcript to leave comments on this
  timeline.", "Nothing to export yet. Add clips to your timeline first.",
  "Analyze to see the transcript", "No previous chats").
- **Progress.** Inline lines with remaining time; step lists with one
  active step; long waits say what is happening ("Finding the best moments.
  This can take a few minutes."); every long job has Stop with a
  confirmation stating the cost consequence; notifications when input is
  needed or a turn finishes.
- **Search** (Cmd+F, "Search dialogue, scene, moments"): scope Source /
  All files; tabs Text and Scene; result paging "Page 2 / 5", "+N more
  matches", next/previous with arrows, replace mode (Cmd+R), replace all
  (Cmd+Enter). Separate visual "Find moments" in the Project panel.
- **Locks and guards.** Edits blocked during agent turns and on analyzed
  or main footage, each with a one-line reason and the way out.

## 7. Keyboard shortcuts

The table is per editor preset; the user's choice at onboarding picks it.
Premiere preset shown, Final Cut Pro differences in the last column (Resolve
mostly follows Premiere). Cmd = Ctrl on Windows. All are customisable in a
visual keyboard editor except the feedback marker; imports Premiere / FCP /
Resolve shortcut files and reports conflicts.

| Panel | Action | Premiere preset | FCP preset |
|---|---|---|---|
| global | Play / stop | Space | |
| global | Shuttle left / stop / right | J / K / L | |
| global | Undo / redo | Cmd+Z / Cmd+Shift+Z | |
| global | Copy / cut / paste | Cmd+C / Cmd+X / Cmd+V | |
| global | Paste insert / paste attributes | Cmd+Shift+V / Cmd+Alt+V | |
| global | Delete clip | Backspace, Delete | |
| timeline | Ripple delete | Shift+Backspace / Shift+Delete | |
| global | Add marker / clear marker | M / Alt+M | M / Ctrl+M |
| global | Add feedback (comment) marker | F (fixed) | |
| global | Focus Project / Chat / Timeline / Transcript panel | Shift+1 / 2 / 3 / 4 | Cmd+1..4 |
| global | Compare with source | Shift+\\ | \\ |
| global | Next / previous screen | PageDown / PageUp | |
| global | Deselect all | Shift+Cmd+A | |
| global | Find and replace | Cmd+F | |
| global | Snapshot frame | Shift+E | |
| global | New timeline | Cmd+N | |
| global | Tools: select, blade, zoom, hand | V, C, Z, H | A, B, Z, H |
| global | Track select fwd / back, rate stretch | A, Shift+A, R | none |
| global | Cut at playhead / all tracks | Cmd+K / Cmd+Shift+K | Cmd+B / Cmd+Shift+B |
| global | Escape tool mode | Escape | |
| timeline | Snapping | S | N |
| timeline | Audio gain | G | Ctrl+Alt+L |
| timeline | Next / previous marker | Shift+M / Shift+Cmd+M | Ctrl+' / Ctrl+; |
| timeline | Previous / next edit point | Up / Down | ; / ' |
| timeline | Start / end of sequence | Home / End | |
| timeline | Frame back / forward (large: +Shift) | Left / Right | |
| timeline | Trim back / forward one frame | Alt+Left / Alt+Right | |
| timeline | Trim start / end to playhead | Q / W | Alt+[ / Alt+] |
| timeline | Extend edit to playhead | E | Shift+X |
| timeline | Mark in / out | I / O | |
| timeline | Clear in / out / both | Alt+I / Alt+O / Alt+X | |
| timeline | Go to in / out | Shift+I / Shift+O | |
| timeline | Lift / extract in-out | ; / ' | none |
| timeline | Mark clip | X | |
| timeline | Insert / overwrite from source | , / . | W / D |
| timeline | Select all clips | Cmd+A | |
| timeline | Select previous / next clip | Cmd+Up / Cmd+Down | Cmd+Left / Cmd+Right |
| timeline | Select clip under playhead | D | C |
| timeline | Disable clip | Cmd+Shift+E | V |
| timeline | Link / unlink | Cmd+L | none |
| timeline | Go to selected clip start / end | Shift+Home / Shift+End | |
| timeline | Switch to video track 1..9 | 1..9 | Alt+1..9 |
| timeline | Cut and switch to track 1..9 | Ctrl+1..9 | 1..9 |
| timeline | Zoom in / out / fit | = / - / \\ | Cmd+= / Cmd+- / Shift+Z |
| timeline | Track height up / down | Shift+= / Shift+- | |
| timeline | Nudge clip left / right (large: +Shift) | Cmd+Left / Cmd+Right | , / . |
| timeline | Nudge clip up / down a track | Alt+Up / Alt+Down | |
| transcript | Move by word / sentence | Left/Right / Up/Down | |
| transcript | Extend selection | Shift+arrows | |
| transcript | Cut / restore sentence, chapter | W, E | |
| transcript | Split caption, line break | Enter, Shift+Enter | |
| transcript | Merge with above / previous word | S / D | |
| transcript | Edit word | Tab | |
| transcript | Start / end of sentence, subchapter, chapter | Home/End, Cmd+Home/End, Cmd+Shift+Home/End | |
| transcript | Previous / next chapter | Cmd+PageUp / Cmd+PageDown | |
| transcript | Assign speaker 1..9 / none | Alt+1..9 / Alt+0 | Ctrl+1..9 |
| project | Select all; add subchapter; move/extend in grid | Cmd+A; Enter; arrows / Shift+arrows | |
| preview | Nudge selected layer (large: +Shift) | arrows | |
| chat composer | Pick mention / recall prompt | Up/Down + Enter/Tab; Up on empty | |
| search | Toggle replace / replace all | Cmd+R / Cmd+Enter | |
| prep | Classify shot as Main / Insert | 1 / 2 | |

## 8. Design tokens (approximate)

The UI is Tailwind with shadcn semantic tokens; dark is the default look.

| Token | Dark | Light |
|---|---|---|
| background / card | `#171717` (neutral-900) | `#ffffff` |
| popover | `#262626` (neutral-800) | `#ffffff` |
| muted / secondary | `#262626` | `#f5f5f5` |
| accent (hover fill) | `#404040` | `#f5f5f5` |
| border | white at 10% | `#e5e5e5` |
| input border | white at 15% | `#e5e5e5` |
| foreground | `#fafafa` | `#0a0a0a` |
| muted foreground | `#a3a3a3` | `#737373` |
| primary button | `#e5e5e5` on `#171717` text | `#171717` on `#fafafa` |
| **key** (brand accent, mentions, active controls) | lime `rgb(163,230,53)` with dark text | olive `rgb(77,124,15)` with white text |
| destructive | `#ef4444` | `#dc2626` |
| ring | `#737373` | `#a3a3a3` |
| "ink" overlays | white at 3/5/8/10/15/20% | near-black at same steps |

Dividers and hover fills are mostly the "ink" colour at 5-10% opacity, not
separate greys. A white key colour variant exists (`data-key-color=white`).

- **Type**: Mona Sans (UI and headers), Inter for tabular numbers, Roboto
  Mono for code/timecode. Scale: 10/12, 11/16, **12/16** (default dense UI
  text), **14/20** (body, composer), 16/24, 18/28, 20/28, 24/32.
- **Radii**: 2, 6 (small controls), 8, 10 (cards), 14, 16, 24.
- **Spacing**: Tailwind 4 px grid; dense panels use 4/8/12 px; mention
  thumbnails 16 px; menus 256 px wide.
- **Shadows**: subtle (1-3 px blur, 5-10% black) on popovers only; panels
  are flat with hairline borders.

## 9. Gap list for `ave`, in priority order

`ave` already matches the broad shape (chat left, browser, viewer, right
pane, timeline foot; reference chips and an `@` composer; proposal cards
that apply as plan versions). The gaps are in how it *works*:

1. **Plans as timeline tabs, agent output as a new tab.** Selects shows
   every timeline as a tab above the timeline and the agent's edit lands as
   a new tab, never in place. `ave` has a plan ComboBox plus a version menu.
   Change `src/gui/pages/plan.rs` `header()`: replace the `plan_picker`
   ComboBox with a tab strip of `project.plans` (click opens, middle-click or
   x closes the view, context menu Rename / Duplicate / Delete / Export /
   Mention in chat, `+` = new plan, Ctrl+N). In `src/gui/assistant.rs`
   `apply()`, add "Apply as new plan" (writes `plans/<name>-v<N>.toml`) next
   to Apply, and make it the default for multi-op proposals, so comparing
   before/after is switching tabs.
2. **Comments mode and feedback markers.** Missing entirely. Add a
   Chat | Comments switch at the top of `src/gui/assistant.rs` `header()`;
   Comments lists timecoded notes on the current plan (stored with the plan,
   for example `plans/<name>.comments.toml`; add the type to `src/plan.rs` or
   a new `src/comments.rs`), each with colour, jump, double-click edit,
   delete. `F` in `src/gui/pages/plan.rs` `keys()` and in the viewer adds a
   comment at the playhead; `src/gui/pages/plan/timeline.rs` draws them as
   markers above V1. "Send comments to chat" builds one message: every
   comment as `@bars`/`@shot` refs plus "propose a plan covering all of
   these and wait for my go-ahead". Add a `Comment` view mode (section 10).
3. **"Mention in chat" as text, not only staged chips.** Selects writes the
   mention into the composer text in timeline order. Add a context-menu item
   "Mention in chat" in `src/gui/pages/plan/timeline.rs`, `review.rs`,
   `people.rs`, `music.rs` that pushes a new `Action::MentionInChat(Vec<Reference>)`
   (add at the end of the enum in `src/gui/app/mod.rs`); the assistant
   inserts `@shot:...` tokens at the caret (it already round-trips that
   text form in `assistant/complete.rs`) and focuses the composer. Keep `R`
   as the fast path. Sort clip references by timeline position. Render
   mention tokens in the accent colour inside the composer (`composer()`).
4. **State-aware empty state and quick actions.** `empty_state()` in
   `src/gui/assistant.rs` is static. Make it depend on the night: not
   indexed → "Index this night" / "Caption it" buttons (spawn the pipeline
   jobs from `pages/night/pipeline.rs`); indexed, no plan → "Suggest a first
   cut", "Plan a 30 s reel on the beat"; plan open → the current four
   suggestions. Mirror the composer hint text per state.
5. **First-cut settings card.** Before drafting a new plan, show a
   "Here's our plan" style card (length 15/30/60/custom, pacing, aspect,
   hero shot, rules toggles: no repeats, people spacing, drop on hero)
   whose Continue becomes the draft request. Build it in
   `src/gui/assistant.rs` as a new card kind next to `proposal_card()`;
   reuse the draft window fields from `src/gui/pages/plan.rs`
   `draft_window()` so both paths produce the same `Cmd`.
6. **Interactive options card and permission mode.** Add an
   `ask_user` tool in `src/gui/assistant/tools.rs` rendering choices with a
   Recommended badge, "Something else", multi-step paging, collapsing to
   "chose X" once answered. Add a permission selector (Ask / Auto / Never
   ask) in `settings()`, replacing the binary `auto_apply` page option
   (`auto_apply()`), so tool calls that render or run long jobs can ask.
7. **Chat tabs and history.** One conversation per plan today. Add a chat
   tab strip (New chat, Close, Close others, Rename, Archive) and a
   "Recent chats" list in `src/gui/assistant.rs` `header()`; extend
   `src/gui/assistant/store.rs` to key logs by chat id with the plan as an
   attribute. Add Find in chat, queued sends while streaming ("Send now"
   to steer), Arrow Up on empty composer to recall prompts with their refs,
   and a context-usage meter.
8. **Shortcut presets matching the user's NLE.** `docs/GUI_CONTRACTS.md`
   assigns Ctrl+1..6 to regions; Selects uses Shift+1..4 (Premiere preset)
   to focus Project / Chat / Timeline / Transcript and keeps digits for
   tracks. Add a central command table (id, label, region, default keys per
   preset) in `src/gui/ui/command_palette.rs` or a new
   `src/gui/ui/keymap.rs`, a preset choice in `src/gui/ui/settings.rs`, and
   route `pages/plan.rs keys()` through it. Add the missing timeline keys:
   I/O, Alt+I/O/X, Shift+I/O, M marker, Shift+M next marker, Up/Down edit
   points, Home/End, `=`/`-`/`\` zoom, Cmd+K cut, Q/W trim to playhead, `,`
   `.` insert/overwrite from browser, Cmd+Up/Down select clip, Shift+E
   snapshot frame. Update the region table in `docs/GUI_CONTRACTS.md`.
9. **Foldable panels with labelled rails and remembered splits.** Selects
   folds chat, project and side panel to thin labelled rails and remembers
   percentages. In `src/gui/ui/workspace.rs`, replace hard `default_size`
   pixels with fractions of the window (chat 22%, browser 21% of the editor
   width, viewer the rest, right pane 21%, timeline 45% of editor height),
   persist sizes in `Config`, and draw a 24 px rail with the vertical label
   ("Chat", "Shots", "Inspector") when `chat_open` / `inspector_open` are
   false instead of removing the pane. Make the timeline span under the
   browser, viewer and inspector but not under chat (it already does).
10. **View modes.** Add a header toggle in `top_bar()`
    (`src/gui/ui/workspace.rs`): Default; **Vertical** (viewer as a tall 9:16
    column on the right of the timeline, for reels, which is ave's main
    deliverable); **Comment** (viewer large, comments and chat on the
    right, browser hidden); **Preview** (viewer only plus timeline). Store
    the mode on `App`.
11. **Transcript-equivalent side tab.** Selects' right pane switches
    Transcript | Adjust. ave's equivalent of the transcript is the shot log
    of the open plan: add a "Shot list" tab to `src/gui/pages/inspector.rs`
    listing every shot of the plan in order with its caption line and the
    plan `note`, clicking seeks, Delete lifts, so the cut can be read as
    text (fits the "read shotlog.md, not frames" rule).
12. **Search like Selects.** The top-bar search filters Shots only.
    Add a results popover under the search box in `top_bar()` with tabs
    (Shots / Captions / People / Music), "+N more", next/previous with
    arrows and Enter to open in the viewer; keep the `fire>0.3` filter
    syntax. Placeholder: "Search shots, captions, moments".
13. **Analysis approval and progress in chat.** When the agent wants a
    slow job (index, caption, render), show an approval card with the
    estimated time before spawning (`src/gui/assistant/tools.rs`), then a
    live step list with time left in the chat and in the status bar
    (`src/gui/ui/chrome.rs`), with Stop and a note that finished clips are
    kept.
14. **Lock the timeline while the agent edits.** Show a one-line banner
    over `src/gui/pages/plan/timeline.rs` ("Editing is paused while chat
    works") and ignore edit keys in `keys()` while an agent turn is live
    (`assistant::State` already knows), rather than racing two writers.
15. **Tokens.** ave's signature greys (`#121212` app, `#191919` panel,
    `#303030` border in `src/theme/mod.rs` `signature_spec`) are close to
    Selects (`#171717` surfaces, white-10% borders). Two cheap
    changes: use one translucent "ink" overlay for hover and dividers
    (`theme::translucent(text, 13..26)`) instead of separate greys, and drop
    the default UI text to 12-13 px with 14 px reserved for the composer and
    body copy (`BODY_SIZE` in `src/gui/ui/theme.rs`) to reach Selects'
    density. Offer a lime key accent preset (`#a3e635` with dark text) for
    mentions and active toggles.

# 04: How the Selects chat agent works, and what `ave` should take from it

A reverse-engineering read of the installed Selects desktop app (Cutback,
package version 2.1.301, Electron), so the `ave` chat panel can copy the
workflow and leave out what doesn't fit. Read November 2026 from the
installed files only: `resources/selects-agent-resources/` (SDK
declarations, recipes, skills), `resources/claude/bundle.json`,
`resources/codex/`, the two `mcp-*.mjs` proxy scripts, and the main-process
JavaScript in `app.asar` (`dist/main/index-*.js`, `dist/mcp/server-*.js`).
Selects was not launched and none of its files were changed. Everything
below is paraphrased. Short phrases in quotes are the only text taken
directly.

Read with: `docs/GUI_CONTRACTS.md` (our panel and references contract),
`src/mcp.rs` (our MCP server), `src/gui/assistant.rs` and
`src/gui/assistant/*.rs` (our current in-process tool loop).

---

## 1. Architecture

```
                         ┌──────────────── remote web UI (static.cutback.video) ───────────────┐
                         │ chat pane · timeline · Apps/plugin panels · confirmation cards       │
                         └──────────────▲───────────────────────────────┬──────────────────────┘
                     IPC "agent:*" events│ (delta, tool-start/result,     │ IPC: send-message, cancel,
                     thinking-delta, usage,│ subagent-progress, done…)      │ respond-options, confirm-tool,
                                         │                                │ queue-steer/reorder
┌────────────────────────────────────────┴────────────────────────────────▼──────────────────────┐
│ Electron main process                                                                           │
│                                                                                                  │
│  Agent session manager ── picks a runtime per chat ────────────────────────────────────────┐    │
│   ├─ local_loop     : own tool loop over an LLM gateway (Selects credits) or a user's      │    │
│   │                   Claude / ChatGPT / Gemini subscription                                │    │
│   ├─ claude_managed : @anthropic-ai/claude-agent-sdk `query()` → bundled claude.exe        │    │
│   │                   (user's own Claude plan via OAuth token)                              │    │
│   └─ codex_managed / codex_chatgpt : bundled codex.exe `app-server` over JSON-RPC          │    │
│                       (user's ChatGPT plan)                                                 │    │
│                                                                                             │    │
│  Tool surface "selects"  (one definition list, re-exposed per runtime)                      │    │
│   • Claude: in-process SDK MCP server (createSdkMcpServer, tools mcp__selects__*)           │    │
│   • Codex : "dynamic tools", rare ones deferred behind a `selects` namespace                │    │
│   • local : plain tool schemas                                                              │    │
│        │                                                                                    │    │
│        ▼                                                                                    │    │
│  run_script sandbox: TypeScript typecheck against sdk/*.d.ts → esbuild → node:vm context    │    │
│        │   (no fs/network/require/timers; deadline; result-size cap; display() images)      │    │
│        ▼                                                                                    │    │
│  SDK host bridge → editor model in the renderer (draft working copy, stage, commitAll)      │    │
│                                                                                             │    │
│  Local HTTP MCP server 127.0.0.1:23100/mcp ◄── mcp-stdio-proxy.mjs ◄── Claude Desktop,      │    │
│  (same tools for external agents; proxy can start the app headless)       Cursor, etc.      │    │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
   Read-only on disk: SELECTS_AGENT_RESOURCES_ROOT = sdk/ (README, *.d.ts, recipes/) + skills/*/SKILL.md + bin/rg.exe
```

The core idea: **the model does not get one tool per edit verb.** It gets a
small set of tools, and the main one, `run_script`, runs TypeScript the
model writes against a typed SDK (`selects.project(..)`, `selects.draft(..)`,
`selects.media..`, `selects.editor..`). The SDK's `.d.ts` files, a README
and focused recipe files are on disk. The model searches them with a
bundled `rg` instead of having them pasted into the prompt. Skills are
Claude-Code-style `SKILL.md` playbooks. The Claude, Codex and in-house
runtimes all share the same tools, SDK, skills and prompt sections.

## 2. Runtimes and model/provider selection

| Runtime | How it runs | Who pays |
|---|---|---|
| `claude_managed` | `claude-agent-sdk` 0.3.283 `query()` with a streaming input queue. The platform `claude.exe` is downloaded on demand from the npm tarball named in `claude/bundle.json` and checked against its checksum. | The user's own Claude plan. An OAuth flow (`agent:claude-oauth-*` IPC) yields a token passed as `CLAUDE_CODE_OAUTH_TOKEN`. The session logs a warning if any other credential source shows up. |
| `codex_managed` / `codex_chatgpt` | Bundled `codex.exe` (0.162.1) in `app-server` mode: approval policy "never", full-access sandbox. Tool calls come back as `item/tool/call` requests that the main process executes. | The user's ChatGPT plan. |
| `local_loop` | Their own loop over an AI-SDK style gateway (Anthropic/OpenAI/Google providers). | Either "managed" (Selects credits through their gateway) or "subscription" transport (the user's Claude, ChatGPT or Gemini key/plan). Errors are reported per transport. |

How `claude_managed` is locked down (worth copying in full):

- `HOME`, `USERPROFILE` and `CLAUDE_CONFIG_DIR` point at a per-app state
  folder, so the user's own `~/.claude` settings, plugins and memory never
  leak in. `settingSources: []`. claude.ai connectors are disabled through
  managed settings. Claude Code's bundled skills are switched off.
- The environment is an allowlist (`PATH`, temp vars, `SystemRoot`,
  `PATHEXT`) plus `SELECTS_AGENT_RESOURCES_ROOT`, `SELECTS_USER_SKILLS_ROOT`
  and `SELECTS_USER_PANELS_ROOT`. The bundled `bin/` (rg, ffmpeg) goes first
  on `PATH`.
- Built-in tools are restricted to `Bash`, `PowerShell` (Windows), `Read`,
  `Skill`, `Task`, `TaskStop`, `WebSearch` and `WebFetch`. Everything that
  touches the project goes through the `selects` MCP server.
- Their own system prompt replaces Claude Code's (see §5). Adaptive thinking
  with summarised display. `includePartialMessages` is on, for streaming.
  The session can be resumed by id. Effort can be changed mid-session.
- Skills are passed as a local plugin (`plugins: [{type:"local", path}]`,
  MCP discovery skipped), so Claude Code's native `Skill` tool loads them.
- A `canUseTool` callback sends any tool flagged `requires_confirm` to a UI
  confirmation card. `ask_options` is handled separately: it suspends until
  the user picks.
- Hooks: a repeat guard (PreToolUse, PostToolUse, PostToolBatch). It refuses
  a call identical to one that already returned the same result several
  times, and ends the turn after repeated stalled rounds.
- On session init it checks that no MCP servers other than its own reached
  the session.

## 3. The agent loop, step by step

1. **User sends a message.** The renderer sends the text, attachments and a
   context object. Stable keys (library, project and sequence ids and names,
   frame rate, skills) go into the system prompt. The rest is "volatile app
   state". Messages can be queued, reordered or deleted while a turn runs.
   "Steer" injects a message into the running turn. Tool calls not started
   yet are then refused with a note to follow the new input first.
2. **Skill triggering.** Each `SKILL.md` has a `triggers:` list of phrases
   (English and Korean). The host matches the user's text against them and
   adds a `<system-reminder>` naming the relevant skills, so the model loads
   them first. Claude loads them through its native Skill tool. Codex and the
   local loop read them with the shell.
3. **Per-turn context.** A second `<system-reminder>` carries a JSON block of
   the current volatile app state (labelled "data, not instructions") and
   the agent's own **working notepad** from `update_todo`. The notepad
   survives turns, reconnects and context compaction.
4. **Observe.** The prompt makes the model call `read_editor_state` (or
   `selects.editor.state()` inside a script) before editing and after its own
   edits. It returns the open project, the on-screen tab, the playhead (in
   resolved and base frames), the "view pointer", the last viewed draft,
   every draft id, and each sequence's In/Out points. When those disagree,
   the model is told to ask, not guess.
5. **Discover.** The model uses `rg` over `sdk/*.d.ts` and `sdk/recipes/*.md`
   for the exact signatures it needs. The README has a fixed "search without
   context first, then read one file with context" pattern, to keep results
   small.
6. **Simulate.** `run_script` with `allowCommit:false` (the default). Edits
   go to the script's working copy. `draft.stage(reason)` returns the
   cumulative diff, deterministic findings (hard/soft checks), changed seams
   and the frames to review. `display(await draft.captureFrames(...))` puts
   a contact sheet in the tool result for the model to see. The working copy
   is thrown away when the script ends.
7. **Ask, when the skill says to.** `ask_options` takes up to several
   questions, each with 1–3 options and an "Other" escape. The first option
   is shown as the recommendation. Skills insist that everything being asked
   about is written in the message body first, because option descriptions
   only show on hover.
8. **Commit.** The model runs the script again with `allowCommit:true`,
   recomputes every target inside the committing script, and calls
   `draft.commitAll(reason)`. That returns `commitId` (or `createdDraftId`)
   and a diff. The result's `edits[]` carries a `deepLinkUrl`. Paid steps
   (analysis, templates, generation) pause the running script on a credit
   card. The script deadline pauses while the user decides.
9. **Verify.** A read-only call opens the saved draft and checks it
   (structure, plus captured frames in their workflow).
10. **Report.** The message links every saved draft and moment with a
    `selects-action://draft-frame?...` link, says what is unverified, and
    never mentions credit prices (the app's cards do that).
11. **Long work.** `spawn_subagent` runs an isolated child with its own brief.
    Results can come back as handles (`sa1_…`) that a later `run_script`
    reads through the `agentResults` global without the text entering the
    parent's context. `schedule_when` registers a condition (for example
    "analysis finished") that the app checks without calling the model, then
    wakes the chat. Context is compacted server-side. `read_user_requests`
    pages back through the original user messages after compaction.

Streaming events sent to the UI: `agent:turn-started`, `agent:delta`,
`agent:thinking-delta`, `agent:tool-start`, `agent:tool-result`,
`agent:subagent-progress`, `agent:usage-update`, `agent:stream-retry` and
`-clear`, `agent:context-compacted`, `agent:session-title`, `agent:done`,
`agent:cancelled`, `agent:error`, `agent:token-expired`. Every `run_script`
call carries a short `summary` field ("Read transcript", "Trim silences")
that the UI shows as the tool row's label. Per-turn telemetry records time
to first token, tool time, confirmation waits, cache hits and cost.

## 4. Tool surface

**Core (always loaded)**

| Tool | What it does |
|---|---|
| `run_script` | Runs TS/JS against the SDK. Inputs: `summary`, `script`, `allowCommit`, `timeoutSeconds` (≤120), `maxResultBytes` (≤1 MB), `resultHandles`. Oversized results are saved to disk as `rs1_…` handles. |
| `run_shell` / native Bash | The user's shell with an allowlisted environment, for searching the resources root and for tasks no Selects tool covers. Each call gets a scratch directory. |
| `read_editor_state` | What the user is looking at right now (see §3.4). |
| `ask_options` | Structured multiple-choice questions shown as chips or a wizard. Suspends the turn until the user answers. |
| `update_todo` | Overwrites the durable notepad (todos plus notes). |
| `spawn_subagent` | Isolated child agent; can resume; can return its result as a handle. |
| `schedule_when` | Condition-based wake-up, checked by the app without calling the model. Lost on restart. |

**Deferred ("loaded on demand" under the `selects` namespace)**: `app_repl`
(Playwright-like control of the Selects window, limited to Apps panels and
view controls, with a lease the user can revoke by pressing Escape),
`browser_repl` (an in-app browser), `generate_media` (paid, confirmed by a
card), `scan_bad_shots`, `scan_gaze`, `compare_audio`, `analyze_retakes`,
`regenerate_topics`, `apply_object_cam_overlay`, `open_selects_guide` (help
centre, must link the source), `view_image`, `read_user_requests`,
`read_result`.

**Scoped "tool request" agents**: an inline request tied to one draft gets
only `inspect_frames` (1–4 frames of that draft), `compare_cameras` and
`read_timeline_context` (dialogue and clips in a range of 60 s at most).
That is a cheap, read-only agent with no edit rights.

**External MCP** (`dist/mcp/server-*.js`, HTTP on a fixed port per build
channel): the same core plus `create_draft` (a long-running edit-plan
workflow), `probe_media`, `scan_directory`, `export_transcript`,
`import_timeline_xml` and `preview_timeline_xml`. The stdio proxy speaks MCP
to Claude Desktop and starts Selects headless if it isn't running.

**The SDK, grouped** (`sdk/*.d.ts`, roughly 400 KB):

- *Reads*: `draft.meta/clips/words/turns/chapters/pauses/seams/markers/inOut`,
  `findUnique/findAll/findCandidates` for exact quotes, `searchScenes` for
  visual semantic search (paged with opaque `sc1_` tokens),
  `source.searchTranscript` for meaning search, `project.resources/
  sourceFiles/readFootage/organizeClips/workflows`.
- *Addressing*: `Span` / `SourceSpan` objects tied to the content they were
  read from ("occurrence-anchored"). A span read before an edit still points
  at the same content after it, and a deleted or foreign span throws instead
  of hitting new material. Raw frames are output only.
  `rangeAtFrames(start,end)` is the explicit, revision-checked escape hatch.
  `.toRef()` is the only portable form across calls.
- *Edits (working copy)*: `insert/remove/replace`, `splitAt`,
  `trimBoundary`, `restoreFrom`, `setClipColor`, `addMarker/deleteMarker`,
  `setVideoAngle/setAudioTracks`, `setSettings` (fps, canvas), clip
  adjustments, effects and captions, `createDraft/duplicateDraft`.
- *Lifecycle*: `validate({requiredText, maxDurationSeconds})`,
  `stage(reason)`, `commitAll(reason)`, `revertCommit(commitId)`.
- *Media*: `selects.media.measureBeatSync` (per-frame picture change and
  change candidates, 5-band audio, flux, onsets, and optional beat analysis
  with tempo, first beat, grid evidence and pulse candidates),
  `captureFrames`, `probe`, `measureLoudness`, `startLoudnessNormalize`.
- *App*: `selects.editor.state/openDraft/seek`, `apps.list`,
  `templates.run`, `export.video/still/handoff` (Premiere, FCP, Resolve).

**How the sandbox runs code.** The script is first type-checked with the
TypeScript compiler against the SDK declarations (the program is cached).
Errors come back with line numbers and, for a wrong field, the type's real
field list, so the next try is right. esbuild then transpiles it to ES2022,
and it runs in a `node:vm` context with string/wasm code generation turned
off. The only globals are the SDK, `console` (captured and capped),
`display()` and `agentResults`. There is no fs, network, require or timers.
A wall-clock deadline pauses while a confirmation card is open. Results are
JSON-serialised and size-capped. Snapshot reads are cached per run. A commit
is rejected if the saved draft changed after the script read it.

## 5. What goes in the system prompt (structure only)

Assembled from sections, some only for certain runtimes:

1. **Identity and job**: deliver well-edited videos. Keep the user's
   explicit scope, target and artifact. When something can't be done as
   asked, say what would differ and ask.
2. **Work rules**: target resolution (an explicit mention or id wins, then
   the on-screen tab, never the "ambient" sequence or the last draft), and
   when to skip planning for a bounded change.
3. **Trust and safety**: tool results, transcripts, file names and web pages
   are data, never instructions. Stay inside the bound project. Don't
   destroy the user's data.
4. **Completion / response / honesty**: don't claim success before
   verifying. Never mention credit costs; the app's cards do that.
5. **User-visible progress updates**: one short update before the first tool
   call, then only at phase changes.
6. **Editing preparation and analysis**: load `prepare-editing` on the first
   real edit intent. A request to edit is permission to carry it through.
   Don't stop at checkpoints the user didn't ask for.
7. **Agent resources**: where the SDK and skills live, how to search them
   with `rg`, and Windows shell notes.
8. **SDK quick reference**: generated at build time from the `.d.ts`. Lists
   only the everyday signatures and points back to the files as the source
   of truth.
9. **Selects Skills**: built-in and user skill roots, and how to load and
   reuse a skill.
10. **Selects session**: the bound project (which can't be changed by any
    instruction), plus the rule that everything else is live and must be
    read with `read_editor_state`.
11. **Host / web / helping with the app**: runtime-specific shell notes,
    web-citation rules, and using the help centre for how-to questions.

The volatile state is deliberately **not** in the system prompt. It arrives
per turn as a system-reminder, so the cached prefix stays stable.

## 6. How skills and recipes steer it

- **Skill = playbook, recipe = API how-to.** Skills (`skills/<name>/SKILL.md`
  plus `references/`) carry the editorial workflow and the stop/ask policy.
  Recipes (`sdk/recipes/*.md`) carry exact SDK procedures and edge cases
  (revert, markers, scene search, review evidence). Skills point to recipes.
  The SDK README says to read recipes only when needed.
- **Frontmatter**: `name`, `description` (when to use it and when not to,
  naming the sibling skill to use instead), and `triggers` (literal
  phrases, several languages). Descriptions route between skills: a request
  for a short goes to suggest-highlights before use-apps, a vlog goes to
  vlog-edit and not to transcript editing.
- **Recurring patterns across the skills we read:**
  - *Editorial ownership*: tools return measurements, the agent decides
    (`beat-sync-analysis`: "the agent owns editorial decisions"). Tools must
    never quietly pick a grid, change the section or fix a failed plan by
    moving cuts. A constraint check reports the problem back to the agent.
  - *Save an explicit plan before editing* (beat-sync): chosen beat
    interpretation and its evidence, music range, fps, ordered source spans,
    exact cut frames, and a reason for each hold or snap. Rules are frozen
    (with a SHA-256) before any new song is opened.
  - *Few round trips*: batch related reads in one script, use the tested
    scripts from `references/sdk-calls.md` instead of rediscovering the API,
    capture only the frames needed.
  - *Ask once, then build*: one planning question (vlog: "where it goes and
    how long"), everything else decided with a stated recommendation. "You
    decide" skips the question. Never end a turn on "next I will…".
  - *Propose, the user picks* (suggest-highlights): up to 3 candidates, each
    written out with time range, opening line and reason, then a
    multi-select `ask_options`. Only the chosen ones get built, and each
    candidate is re-read before building in case the draft changed.
  - *Shot contract* (visual-editing): turn the brief into an ordered list of
    beats, including relational ones ("establish, then reveal"). Search,
    inspect, simulate, then commit in a **separate** call after the model has
    seen the simulation, then verify the saved result in a third call.
  - *Readiness without gates* (prepare-editing): check analysis and sync
    once per new source scope. Don't re-offer analysis the user declined.
    The app's credit card is the consent.
  - *Honest reporting*: report execution, visible style, sync evidence and
    listening as separate outcomes. "You cannot listen" must be stated.
    Anything unverified is named as unverified.
- **video-request-intake** is a front door that routes by category (ad,
  explainer, drama, own-footage edit, existing App) and path. Its order is
  plan → storyboard → sample → full video, each approved before the next,
  and it ends with an exported file that was checked.
- **panel-authoring / use-apps / app-agent / visual-browse** cover the
  plugin system: the agent writes `panel.tsx` plugins whose buttons call the
  same SDK (`sdk.runScript`), runs templates with `templates.run`, and can
  operate panels through `app_repl` (snapshot refs first, pixel coordinates
  only as a last resort, never pressing buttons that spend credits).
  **story-clips-find** is web research that hands one story to an App.

## 7. How edits are applied safely

- **Working copy per script**: every edit verb changes a scratch copy only.
  Nothing persists without `commitAll`, and `commitAll` throws unless the
  tool call set `allowCommit:true`. The model has to set that flag on
  purpose, and the UI can see it.
- **Stage before commit**: `stage()` gives a validated diff, findings and
  frames to review without saving. The preferred flow is to simulate and
  preview, then rerun the same script with commit so it is recomputed
  against the current draft. Reusing frame numbers from an earlier call is
  explicitly called out as an anti-pattern.
- **Optimistic concurrency**: a commit is rejected if the saved draft
  changed after the script read it, so the user can keep editing while the
  agent works.
- **Revert**: `revertCommit(commitId)` saves the safe inverse right away,
  only while the draft is still at that commit's revision (`snapshot_stale`
  otherwise). The receipt lasts one hour in memory (`expired_commit` after
  that). After that, the fix is a new edit. Immediate-persist calls (project
  colours, imports, caption corrections, translations) are documented as
  separate from the working copy and must not be mixed with `commitAll` in
  the same script.
- **New drafts by default**: highlights, shorts and templates build into
  *new* drafts or duplicates ("pass only a Draft made for this run"), so the
  user's own cut is never edited in place unless they ask for that.
- **Comments are markers**: review notes are timeline markers with a note.
  `markers()` reads them and the agent can act on them, but must only delete
  rows that have a note, because a marker without a note may be a grader's
  score.
- **Confirmations belong to the app**: tool-level `requires_confirm` goes
  through `canUseTool` to a card. Paid steps pause the script on a credit
  card. A declined card throws a known error, and the skill continues
  without that step.

## 8. References, mentions and links

- **Into the model**: an explicit mention, name or id in the message beats
  everything else. Otherwise the target comes from fresh editor state
  (on-screen tab, playhead, the draft's In/Out marks). "Tool requests" from
  the timeline carry a draft id plus a limited tool set for questions about
  that range. Attachments arrive as `<attachment>` blocks. Long documents
  are passed by `attachment_id`, so the model hands the id to a tool instead
  of copying the text out.
- **Out of the model**: every saved draft or moment the agent mentions must
  be a link: `selects-action://draft-frame?sequenceId=<id>&frame=<n>`, and
  `selects-action://document?path=…` for Markdown files the agent wrote.
  The UI turns these into click-to-seek chips. Skills say to use only ids
  and frames the agent actually read, to take `deepLinkUrl` from the commit
  result as-is, and to leave unsaved drafts as plain text.
- **Coordinates are explicit**: draft frames in resolved (playback)
  coordinates, source frames in base coordinates, transcript times in source
  seconds. The README has a table of which read gives which target. Most
  agent mistakes in this domain are mixed-up coordinates, and the API types
  are built to prevent them.

---

## 9. What `ave` should copy, mapped onto our Rust app

We already have most of the hard parts. `src/mcp.rs` (`ave mcp --work
<night>/_ave`) exposes the engine with a working-copy model (`plan_edit` and
`plan_draft` only save with `commit:true`, `plan_revert` saves a new
version). `src/gui/assistant/` has an in-process Claude loop with diff
cards, `@` references (`src/refs.rs`) and plan versions. Selects shows how to
join the two.

### 9.1 Run Claude Code with our MCP server as the panel's main runtime

Add a runtime next to `provider::Claude`/`Local`, for example
`assistant/claude_code.rs`. It spawns the `claude` CLI headless (`-p
--output-format stream-json --input-format stream-json --verbose
--include-partial-messages`, which is the same protocol the Agent SDK
drives) on a worker thread, with:

- `--mcp-config` pointing at `{"ave": {"command": "<current exe>", "args":
  ["mcp", "--work", "<night>/_ave"]}}`, plus `--strict-mcp-config` so no
  other servers load.
- `--allowedTools` set to `mcp__ave__*`, `Read`, `Grep`, `Glob` and `Skill`,
  with no `Bash` at first. Our engine steps are MCP tools, so the agent
  never needs a shell to edit.
- An isolated `CLAUDE_CONFIG_DIR` and `HOME` under the app's config dir,
  `--setting-sources ""`, and an allowlisted environment. That keeps the
  user's `~/.claude` (including this repo's own CLAUDE.md) out of the editing
  agent. Copy Selects' check that only our MCP server is connected, and
  log any other server that shows up.
- `--system-prompt` set to our own sections (see 9.4), and `--resume
  <session>` with the session id saved next to `<work>/chat/<plan>.jsonl`.
- Auth: if the user already uses Claude Code, the CLI's own login (their
  Claude plan) works as-is. Keep the current `ANTHROPIC_API_KEY` path and
  `Local` as fallbacks. Model choice stays in `MODELS`; pass `--model`.

Map the stream-json events onto our existing `agent::Event`: text and
thinking deltas go to `Event::Delta`, `tool_use`/`tool_result` go to tool
rows (show the tool's `reason`/`summary` field as the label, as Selects
does), the result message goes to `Event::Done` with usage. Cancel works
by killing the process, or with an interrupt control message.

### 9.2 Make the MCP server the single edit path

- Put the panel's diff cards on top of MCP results, not a separate tool
  loop. `plan_edit` without `commit` already returns diff plus rule check.
  That *is* Selects' `stage()`. The card's **Apply** calls the same handler
  with `commit:true`, which writes `plans/.history/<plan>/v<N>.toml`.
  `assistant/tools.rs` and the MCP tool table should share one schema list,
  so the in-process loop and Claude Code see the same tools.
- Add an `allow_commit` gate at the panel level, like Selects: while the
  panel is in "propose" mode, the server refuses `commit:true` and returns
  the staged diff for the user to apply. In "auto" mode the agent may commit
  and every commit shows up as a revertible card. Use a PreToolUse hook or
  `--permission-prompt-tool` (an MCP tool in `ave mcp` that asks the GUI) to
  send `plan_revert`, `render` and anything else that costs time to an
  approval card.
- Add **revision checks**: `plan_edit` takes the version it was based on
  (from `plan_get`) and rejects a commit if the plan moved since, so the user
  can keep editing the timeline while the agent works. This is Selects'
  stale-snapshot rule.
- Keep **"new plan by default"**: `plan_draft` and multi-length requests
  create `reel-15`, `reel-30` and so on as new plans, and never rewrite the
  one on screen unless the user names it.

### 9.3 Skills for our workflow (files, not prompt text)

Ship `skills/<name>/SKILL.md` in the repo (read-only at runtime, exposed
with `--plugin-dir` or by pointing the agent at the folder), with `name`,
`description` and `triggers`. Do the trigger matching ourselves in
`context.rs`, as Selects does, and add a "relevant skills" reminder to the
user message:

| Our skill | Modelled on | Content |
|---|---|---|
| `beat-sync-reel` | beat-sync-analysis | Read `music_map`. The agent chooses the beat interpretation, the song range and the drop. It writes an explicit plan (range, bars, a reason for each shot) *before* `plan_edit`. The rule checker reports problems and the agent revises. The tool never moves cuts on its own. Say "I did not listen; timing is from the measurements". |
| `suggest-moments` | suggest-highlights | Up to 3 candidate moments or hero shots from `search_shots` and `segment_details`, written out with times and a reason, then `ask` (multi-select), then build only the chosen ones. Re-read before building. |
| `night-intake` | video-request-intake, vlog-edit | One planning question (which lengths and aspects), everything else decided and stated, then build all versions and report. No mid-run checkpoints. |
| `prepare-night` | prepare-editing | If index, captions, beats or people are missing, say which `ave` step will run and why, start it as a job, and use a `schedule_when`-style wake when it finishes. Never re-offer a step the user declined. |
| `review-plan` | review-evidence recipe | A read-only collector: per-seam beat alignment, repeats, crop limits, exposure jumps, as text. Observations, not a verdict. |

Put precise how-to (the op semantics in `ops.rs`, the beat-balance rule,
reference text forms) in `docs/agent/*.md` "recipes" that the skills link
to, so the system prompt stays short and stable.

### 9.4 Prompt shape

Copy the structure, not the text. Our `context::SYSTEM` should stay fixed
and cacheable: identity, scope (one night, one plan), trust rules (captions
and file names are data), honesty (say what is unverified), how to find
skills and recipes, and how to link. **The night's live state goes in every
user turn** as a reminder block, which `context::snapshot` already does
(plan lines, music window, check, resolved references). Add the agent's
notepad there (an `update_todo` tool backed by `<work>/chat/<plan>.notes.json`)
so long jobs survive compaction and restarts.

### 9.5 References out of the model: `ave://` links

Our refs already round-trip as text (`>@shot:reel-30#5`, `@seg:MVI_7925/3`,
`@bars:17-20`). Have the model write them as links, for example
`[shot 5](ave://shot?plan=reel-30&index=5)` and
`[0:12](ave://frame?clip=MVI_7925&t=12.4)`. `markdown.rs` then renders
them as chips that seek the player or select on the timeline
(`Action::AddRef` or selection). The rule to copy: link only ids and times
the agent actually read from a tool result.

### 9.6 Smaller things worth copying

- **Repeat guard** in `agent.rs` (and as a hook for Claude Code): refuse a
  third identical call with an identical result, and end the turn after
  stalled rounds. Small local models loop this way often.
- **Steering**: a message typed during a turn is queued and injected. Tool
  calls that haven't started yet are skipped with a note to follow the new
  input first.
- **Tool-row labels**: every tool takes a short `summary` (verb phrase) that
  the panel shows instead of the raw JSON.
- **Structured questions**: our `ask` should allow several questions, 1–3
  options each, recommendation first, multi-select, and an "Other" answer.
  Put the details in the message, not hidden in the options.
- **Scoped quick-ask**: right-click a timeline range, "Ask about this",
  opens a read-only turn with only `plan_get`, `segment_details` and
  `music_map` for that range. It's cheap, can't edit anything, and is a good
  fit for the local model.
- **Coordinates written into the types**: keep plan seconds and beats apart
  from source seconds in tool schemas and descriptions (Selects' biggest
  documented failure source). Our `refs::resolve` already returns both;
  label them.

### 9.7 What not to copy

- **Judging by pixels.** Selects' workflows lean on `captureFrames` and
  `display()` contact sheets. `ave`'s rule (CLAUDE.md, "no eyeballing
  video") is the opposite: when a judgment is missing, add a signal to the
  shot log. Keep `look` for humans and for debugging, and keep it out of
  the skills' required steps.
- **Agent-written code in a sandbox.** `run_script` + `node:vm` + a
  TypeScript typecheck fits a huge API surface (a full NLE). We have about
  12 tools and a small op set, so typed MCP tools with JSON-schema ops are
  simpler, and with Rust there's nothing to sandbox. Look again only if the
  op set grows past what schemas can say. The interpreter would then be an
  embedded one (Rhai, or QuickJS via rquickjs), never a shell.
- **Credits, gateways, remote UI, plugin store, app_repl screen control.**
  None of it fits a local, free, single-user tool.
- **Codex runtime.** Possible later (`codex app-server` with our tools as
  dynamic tools), but Claude Code plus our MCP server covers the "use your
  own plan" case.

### 9.8 Suggested order of work

1. Share one tool schema list between `assistant/tools.rs` and `src/mcp.rs`.
   Add `summary`, a revision check on `plan_edit`, and an `allow_commit`
   gate.
2. `assistant/claude_code.rs`: spawn, stream-json to `Event`, cancel,
   resume, isolated config. Add a runtime selector next to the model picker.
3. Diff cards fed from staged MCP results. Apply means commit, undo means
   `plan_revert`.
4. `skills/beat-sync-reel` and `skills/suggest-moments`, with trigger
   matching in `context.rs`.
5. `ave://` links in `markdown.rs`, multi-question `ask`, notepad, repeat
   guard.

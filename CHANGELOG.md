# Changelog

All notable changes to AI Video Editor, newest first.

Entries follow the conventional-commit subjects in the git history
(`feat:` under Added, `fix:` under Fixed, and so on). See `CONTRIBUTING.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-25

The first release: a Windows desktop editor (and the `ave` command line under
it) that turns a folder of raw clips and a music track into a beat-synced first
cut, checks it against the project's rules, and exports the video or an OTIO
timeline. The per-change history up to this point is in the git log.

### Added

- **Indexing** (`ave index`): per-clip analysis over ffmpeg of exposure on the
  lit region, focus, fire / LED / white light, moving light, camera motion and
  shot boundaries, written to `index.json` and a readable `shotlog.md`.
  Segments carry flags (`dark soft shaky whip blown no-action`), a best moment,
  bad spans inside them (a focus hunt, a whip, a jolt), recording time, camera
  and act. Crowd cheers are measured from the camera audio as moments of their
  own. One unreadable clip no longer stops the night, decoding uses an NVIDIA
  GPU when there is one, and heavy footage gets light playback copies.
- **Captions** (`ave caption`): schema-constrained descriptions of each segment
  (people, moment, framing, setting, issues) from a local Qwen3-VL model on
  llama.cpp, set up in *Settings → Captions*.
- **Beats** (`ave beats`): tempo, beat grid locked onto the kicks, bars,
  sections and drops, with a Music tab to see and fix the grid. Projects can
  also be cut without music, paced on the clips' own sound.
- **People** (`ave people`): a `people.toml` draft of who is in which clip, a
  People tab to name, merge and split performers, and a confirm queue.
- **Plans and the check**: cuts are TOML storyboards on the beat grid, drafted
  under the project's rules (repeats, people, footage to avoid, the drop),
  with a note on every shot. `ave check` reports each finding with a stable id
  and a severity. **Fix**, **Ask chat** and **Accept** sit on every finding in
  the timeline: Fix applies the best automatic fix as one undoable new version,
  Ask chat hands the shots to the assistant, Accept stops a soft finding
  counting until its shots change. `ave check --fix` does the same from the
  command line.
- **Templates and rules**: *Flow Night* and *General* templates, a project
  name, and a *Rules* section to edit every drafting rule per project.
- **The editor**: a guide strip whose *Next* always does the next step; a
  workspace with chat, Shots / People / Music browser, viewer with real
  playback and framing guides, inspector with a Shot list, and a timeline with
  plan tabs, versions, compare, undo, ranked alternatives (`A`), **markers**
  and comments. The Shots browser searches the shot log and groups shots by
  **act / set**; the status bar explains the control under the pointer.
- **Chat assistant**: one or more chats per project that propose edits as
  diff cards with the predicted check, applied as new versions. It runs on
  **Claude Code** (sign in from the panel, no API key), the **Claude API**, or
  a **local llama.cpp model**, with the engine's tools served over
  `ave mcp`, which any MCP client can also use.
- **Render and export**: `ave render` makes an MP4 and a 720p preview on x264,
  NVENC or AMF, and the Export panel renders every cut by deliverable to the
  names you chose. It **never overwrites a file ave did not make** and asks
  before replacing its own. `ave otio` writes an OpenTimelineIO timeline for
  Resolve or Kdenlive.
- **Settings**: ffmpeg's folder in *Settings → Tools*, with a clear notice when
  ffmpeg is missing; a crash shows its error in a dialog instead of closing.
- **Releases**: a tag-driven release workflow builds the Windows archive with
  an SBOM, checksums and build-provenance attestations.

[Unreleased]: https://github.com/freeknet/aivideoeditor/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/freeknet/aivideoeditor/releases/tag/v0.1.0

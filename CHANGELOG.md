# Changelog

All notable changes to AI Video Editor, newest first.

**This file is generated** from the conventional-commit subjects in the
git history by `cargo run --example changelog`, and CI checks it. The
lists under a release are rewritten on every run; a paragraph between a
release's heading and its first list is its summary, and is kept. A
wrong subject is corrected in `CHANGELOG.corrections`. Releases from
before the generator keep the section they shipped with. See
`CONTRIBUTING.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- ave --diagnose, and every pull request builds and checks the release zip ([`e1d46f4`](https://github.com/freeknet/aivideoeditor/commit/e1d46f4b6928ca3482d79308ae46209dee655b8e))
- every window session writes a log under %LOCALAPPDATA%\aivideoeditor\logs ([`82513b6`](https://github.com/freeknet/aivideoeditor/commit/82513b6949b41e59cee3eda631a43eec7b6c20c1))
- the release zip carries THIRD_PARTY.md and the full third-party notices ([`b0ae68f`](https://github.com/freeknet/aivideoeditor/commit/b0ae68feddd0ad48d380ac261d2f3d70829c4885))

### Fixed

- plan files and people keys with impossible numbers are refused instead of overflowing ([`0046ed2`](https://github.com/freeknet/aivideoeditor/commit/0046ed287fb76e6602fa7ff797f9f55f7d66c6d1))
- ave.exe starts on a machine without the Visual C++ runtime ([`2123a17`](https://github.com/freeknet/aivideoeditor/commit/2123a17d722174a8239e16ad8ddb5736e98222d7))

### Changed

- rustfmt the credits example ([`34e6aa3`](https://github.com/freeknet/aivideoeditor/commit/34e6aa3dde61ca05cd9fdf36440c6a938f50940d))

### Documentation

- SECURITY.md says the API key is kept in plain text, and the agent notes describe the desktop editor ([`cf495f3`](https://github.com/freeknet/aivideoeditor/commit/cf495f3b1c2005b60fd6ccf833aee17197f89823))

### Internal

- an automated review of each new pull request, and an on-demand assistant in comments ([`56c8d04`](https://github.com/freeknet/aivideoeditor/commit/56c8d04264f71f116ab22bbbd2098bb12d1c904c))
- actionlint checks the workflows ([`ccd972c`](https://github.com/freeknet/aivideoeditor/commit/ccd972c550d4aac4d8587430907b88312e7952fc))
- a release build without the Windows SDK warns instead of failing, unless AVE_REQUIRE_RESOURCES=1 ([`32e366c`](https://github.com/freeknet/aivideoeditor/commit/32e366c93850dc9ab4f0e33e5d4fb5b7d8ac82e1))
- MSRV 1.98, with the toolchain still pinned at 1.99.1 ([`6ab8e7e`](https://github.com/freeknet/aivideoeditor/commit/6ab8e7e07ba2c960bbb946fd366eaa00a683aace))
- THIRD_PARTY.md drops env_logger's crates ([`47cdd14`](https://github.com/freeknet/aivideoeditor/commit/47cdd145712f95806ea62a11a3b37e254419c485))
- unsafe_code is denied outside src/win, and every discarded result says why ([`d689af5`](https://github.com/freeknet/aivideoeditor/commit/d689af52e772ac04b1563026eda9f49bf1c816c0))
- a weekly advisories, unused-dependency and coverage run, and one ci-ok check to require ([`97269a7`](https://github.com/freeknet/aivideoeditor/commit/97269a7f2e93116e9275c1f1f6ccfe01879262d2))
- a release builds one resolved commit, after CI passes on it, and never rebuilds a published one ([`c820cac`](https://github.com/freeknet/aivideoeditor/commit/c820cac67739cdd1ab62c92d4f7069fdabf8320f))
- every cargo command in the workflows runs --locked, and a test keeps them pinned ([`007ff40`](https://github.com/freeknet/aivideoeditor/commit/007ff40e4ed47d376f420a6f9ed44db196a344c6))
- a mailmap folds the maintainer's commit identities into one name ([`58333ac`](https://github.com/freeknet/aivideoeditor/commit/58333ac9bff157d22d5c4f140a220e8c8c00aff2))
- the changelog is generated from commit subjects and checked ([`a3c69d5`](https://github.com/freeknet/aivideoeditor/commit/a3c69d53aeb52d33af0c8402df9b6a2435aa260a))
- release builds keep line tables and ship ave.pdb as a symbols archive ([`ed9e7e5`](https://github.com/freeknet/aivideoeditor/commit/ed9e7e584a0d1fcb07f465fbe2303bacca965e34))

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

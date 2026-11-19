# Changelog

All notable changes to AI Video Editor, newest first.

Entries follow the conventional-commit subjects in the git history
(`feat:` under Added, `fix:` under Fixed, and so on). See `CONTRIBUTING.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
- `ave render` / `ave otio`: one-graph ffmpeg render with a 720p preview, and
  OpenTimelineIO export.

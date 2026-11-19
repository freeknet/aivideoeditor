# Changelog

All notable changes to AI Video Editor, newest first.

Entries follow the conventional-commit subjects in the git history
(`feat:` under Added, `fix:` under Fixed, and so on). See `CONTRIBUTING.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Project scaffolded with create-rust-app.
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
- `ave render` / `ave otio`: one-graph ffmpeg render with a 720p preview, and
  OpenTimelineIO export.

# Changelog

All notable changes to AI Video Editor, newest first.

Entries follow the conventional-commit subjects in the git history
(`feat:` under Added, `fix:` under Fixed, and so on). See `CONTRIBUTING.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Music tab: whole-track overview and FCP-style zoomed beat grid (sections, drops,
  song ranges, playhead), grid fixes (drag the downbeat, `,`/`.` beat, Shift 10 ms
  offset, tempo, `D` drop, rename sections, re-analyse), per-deliverable song ranges,
  and bar selections as `@bars` references. `ave beats` now also finds song sections
  (intro / build / drop / break / outro) on phrase lines, scores drops by their build,
  keeps an energy envelope, and stores the user's grid corrections in `music.json`,
  applied on load and kept across re-analysis.
- Project scaffolded with create-rust-app.
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

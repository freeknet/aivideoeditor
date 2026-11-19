# Changelog

All notable changes to AI Video Editor, newest first.

Entries follow the conventional-commit subjects in the git history
(`feat:` under Added, `fix:` under Fixed, and so on). See `CONTRIBUTING.md`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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

### Fixed

- Drafted crops are rounded inward so a two-decimal `cx` never leaves the frame,
  and windows never run past the end of the file.

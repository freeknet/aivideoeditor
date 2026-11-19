<div align="center">

# AI Video Editor

**Index footage, plan beat-synced edits, render and export timelines.**

[![CI](https://github.com/lnorton89/aivideoeditor/actions/workflows/ci.yml/badge.svg)](https://github.com/lnorton89/aivideoeditor/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rust-1.98%2B-orange.svg)](Cargo.toml)

</div>

`ave` turns a folder of raw footage into text an LLM can plan an edit from,
then turns the plan back into video. The model never watches the footage:
a deterministic index (exposure, focus, fire/LED light, moving light, camera
pans and zooms, shot boundaries) plus local vision-model captions stand in
for watching, and the plan is a TOML storyboard on the music's beat grid
that a rule checker verifies before anything renders.

Built for the weekly JaJa Flow Night promo edits; nothing in it is specific
to that event except the default rules, which live in each plan file.

## The app

`ave` with no arguments opens the desktop editor; `ave --open <night>/_ave`
opens a night directly. One workspace, laid out like Selects or Resolve's Cut
page (details in [`docs/GUI_CONTRACTS.md`](docs/GUI_CONTRACTS.md)):

- **Home** — recent nights, "new night from last week", the pipeline
  (index → caption → music → people → plans → export) with live job logs,
  footage / track / end-card settings and the weekly recipe.
- **Chat** (left) — ask for changes in plain words; attach **references**
  as chips (`R` on any selection, `Shift-R` to say keep / avoid / replace /
  more like / feature, or type `@shot:reel-30#5`, `~@seg:MVI_7925/3`,
  `!@person:x`, `@bars:17-20`). Answers come back as proposals with a diff
  and the predicted rule check; applying one saves a new plan version.
  Claude via `ANTHROPIC_API_KEY`, or a local llama.cpp server.
- **Browser** — *Shots* (searchable shot log: `fire>0.3 -dark person:x`,
  grid or list, hover scrub, pick/reject P/X/U, more-like-this), *People*
  (groups, merge/split, confirm queue), *Music* (beat grid, sections, drops,
  grid fixes, song ranges).
- **Viewer** — real-time playback with audio, 16:9 / 9:16 / 4:5 framing with
  safe areas, J/K/L, I/O marks, `Alt-R` to reference a range.
- **Inspector** — everything known about the selection.
- **Timeline** — the plan on the beat grid over the waveform: reorder, roll,
  slip, `A` for ranked alternatives, live rule check, versions and undo.
- **Export** — deliverables × plan versions, filename templates, render and
  OTIO queue.

## The workflow (command line)

```bash
ave index  <footage-dir> --work <work>   # ~40 s for 20 min of 1080p: index.json + shotlog.md
ave caption --work <work>                # Qwen3-VL captions per segment (local GPU)
ave beats  <track.wav>  --work <work>    # tempo, beat grid on the kicks, bars, drops
ave people --work <work>                 # draft people.toml (who is in which clip)
ave plan reel-30 --work <work> --aspect 9x16 --seconds 30 --end-card card.png
ave check  <work>/plans/reel-30.toml --work <work>
ave render <work>/plans/reel-30.toml --work <work>      # mp4 + 720p preview
ave otio   <work>/plans/reel-30.toml --work <work>      # timeline for Resolve / Kdenlive
```

| Step | What it produces | How |
|---|---|---|
| `index` | `shotlog.md`, `index.json`, per-clip cache | ffmpeg decode at 6 fps; Rust measures luma, focus (Laplacian), emitters split by hue into fire / LED / white, moving light (motion-compensated), camera motion (block matching → pan + zoom fit), colour cuts; segments of one camera behaviour, scored and flagged `dark soft shaky whip blown no-action` |
| `caption` | captions under each shot-log line | two stills per segment → llama.cpp server running Qwen3-VL-4B, JSON-schema constrained: summary, people (appearance + prop), moment (`performing / transition / idle / no-performer`), framing, setting, issues |
| `beats` | `music.json` | spectral-flux onsets, autocorrelation tempo, dynamic-programming beat tracking, grid shifted onto measured kick onsets, bars by low-end phase, drops by low-end jump |
| `people` | `people.toml` draft | groups clips whose performer descriptions share identifying words; edit names and ranges by hand |
| `plan` | `plans/<name>.toml` | beat grid (breakdown → build 2+1+1 → hero on the drop → 2-beat cuts, 1-beat stutters → fire finale → end card), filled most-important-slot first under the rules |
| `check` | pass / list of problems | no repeated stretch of a clip (30 s), no performer within 8 shots, no rejected flags or captions, drop on fire, crop inside frame, action fits the crop |
| `render` | `renders/<name>.mp4` + `-720p.mp4` | one ffmpeg graph: frame-exact trims on the beat grid, crop, per-shot exposure match, light denoise, end card, straight music trim (no fades, no gain) |
| `otio` | `renders/<name>.otio` | OpenTimelineIO JSON: source clips at frame accuracy, end card, music, a marker on the drop |

A plan is meant to be edited: change a shot's `clip`, `in`, `beats` or `cx`,
run `ave check`, render. The checker is the guard rail whether the edit came
from the drafter, a person or an LLM.

## Setup

- **ffmpeg / ffprobe** on `PATH`, or `AVE_FFMPEG_DIR` pointing at their folder.
- **Vision captions** (optional but the planner uses them): a llama.cpp build
  with a vision model. Either run `llama-server` yourself on port 8089
  (`AVE_VLM_URL` to change it), or set these and `ave caption` starts and stops it:

  ```bash
  AVE_LLAMA_DIR=C:\Users\you\tools\llama.cpp            # llama-server.exe (Vulkan build works on AMD)
  AVE_VLM_MODEL=C:\Users\you\tools\models\Qwen3VL-4B-Instruct-Q4_K_M.gguf
  AVE_VLM_MMPROJ=C:\Users\you\tools\models\mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf
  ```

  The 4B model at Q4 uses about 3.7 GB of VRAM and captions a segment in 7–9 s.

```bash
cargo install --path .           # puts `ave` on your PATH
```

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check                                         # licences, advisories, sources
```

All of these are clean on `main` and enforced by CI
(`.github/workflows/ci.yml`). A warning is a build failure there.
See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the rest.

## Documentation

- [`CONTRIBUTING.md`](CONTRIBUTING.md): how to build, test, and what the build enforces
- [`CLAUDE.md`](CLAUDE.md): the rules this codebase enforces, for people and agents
- [`CHANGELOG.md`](CHANGELOG.md): what changed, release by release
- [`SECURITY.md`](SECURITY.md): how to report a vulnerability privately

## Licence

MIT. See [`LICENSE`](LICENSE).

---

<sub>Scaffolded with [create-rust-app](https://github.com/freeknet/create-rust-app).</sub>

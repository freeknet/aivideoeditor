<div align="center">

# AI Video Editor

**Index footage, plan beat-synced edits, render and export timelines.**

[![CI](https://github.com/freeknet/aivideoeditor/actions/workflows/ci.yml/badge.svg)](https://github.com/freeknet/aivideoeditor/actions/workflows/ci.yml)
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

## Install and run on Windows

Step by step on a fresh Windows 10 or 11 machine. Run the commands in
**PowerShell**. Steps 1–6 are all you need to edit footage. Step 7 adds
captions, which the planner leans on, and step 8 adds the chat assistant.

### 1. Install the build tools

```powershell
winget install --id Git.Git -e
winget install --id Rustlang.Rustup -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

The last one installs the MSVC C++ compiler and the Windows SDK, which Rust
needs to link on Windows. It is a large download and can take a while.
**Close and reopen PowerShell** afterwards so the new `PATH` is picked up.
You do not need to pick a Rust version: the repo's `rust-toolchain.toml`
pins it, and `rustup` installs that version on the first build.

### 2. Install ffmpeg

```powershell
winget install --id Gyan.FFmpeg -e
```

Reopen PowerShell, then check that both tools are found:

```powershell
ffmpeg -version
ffprobe -version
```

If you already have ffmpeg somewhere else, point `ave` at its folder instead:
`setx AVE_FFMPEG_DIR "C:\tools\ffmpeg\bin"`.

### 3. Get the code

The repository is private, so sign in to GitHub first. Either install the
GitHub CLI and log in (`winget install --id GitHub.cli -e`, then
`gh auth login`), or let Git ask for your credentials when you clone.

```powershell
cd $HOME\Documents
git clone https://github.com/freeknet/aivideoeditor.git
cd aivideoeditor
```

### 4. Build and install `ave`

```powershell
cargo install --path . --profile release-fast
```

The first build downloads the pinned toolchain and every dependency, so give
it several minutes. It puts `ave.exe` in `%USERPROFILE%\.cargo\bin`, which
rustup has already added to `PATH`. Check it worked:

```powershell
ave --help
```

`--profile release-fast` builds a few times faster than the shipped
`release` profile (no link-time optimisation) and runs at nearly the same
speed. Leave it off for the smallest, fastest binary.

**Updating later:** `git pull`, then run the same `cargo install` line again.

### 5. Lay out a night

Keep one folder per night with its raw clips inside. `ave` writes everything
it makes into a `_ave` work folder in the night folder, and never changes
the clips:

```text
C:\Users\you\Pictures\Flow Night\2026-11-11\
    originals\video\     the camera and drone clips
    _ave\                created by ave: index, shot log, plans, renders
```

The music can live anywhere; you pick the track in the app.

### 6. Open the app

```powershell
ave
```

With no arguments `ave` opens the desktop editor. Open a night by picking
its footage folder (or the night folder); `ave` creates `_ave` the first
time. Then run the pipeline from the Home page (index → caption → music →
people → plans → export). Each step shows its progress and log. To jump
straight into a night you have already set up:

```powershell
ave --open "C:\Users\you\Pictures\Flow Night\2026-11-11\_ave"
```

Or run the same steps from the command line, as in
[the workflow](#the-workflow-command-line):

```powershell
$night = "C:\Users\you\Pictures\Flow Night\2026-11-11"
ave index "$night\originals\video" --work "$night\_ave"
```

Re-running `index` is cheap: clips already measured come from the cache, so
it only does the new work.

### 7. Captions (optional, recommended)

Captions come from a vision model running locally on your GPU through
llama.cpp. You need about 4 GB of free video memory.

1. Download a llama.cpp Windows release from
   <https://github.com/ggml-org/llama.cpp/releases>. Pick the **Vulkan**
   build for AMD or Intel graphics, or a **CUDA** build for NVIDIA. Unzip it
   to, say, `C:\tools\llama.cpp`, the folder that contains `llama-server.exe`.
2. Download the Qwen3-VL 4B Instruct model and its vision projector from
   <https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF>:
   `Qwen3VL-4B-Instruct-Q4_K_M.gguf` and `mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf`.
   Put both in `C:\tools\models`.
3. Tell `ave` where they are. `setx` saves the setting for new PowerShell
   windows, so reopen PowerShell after running these:

   ```powershell
   setx AVE_LLAMA_DIR  "C:\tools\llama.cpp"
   setx AVE_VLM_MODEL  "C:\tools\models\Qwen3VL-4B-Instruct-Q4_K_M.gguf"
   setx AVE_VLM_MMPROJ "C:\tools\models\mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf"
   ```

`ave caption` (or the Caption step in the app) then starts `llama-server`,
captions every segment, and stops it again. Expect 7–9 s per segment.

### 8. The chat assistant (optional)

The chat panel can talk to Claude in one of three ways. Pick one in the
model menu under the chat box:

- **Claude Code:** install [Claude Code](https://docs.claude.com/en/docs/claude-code),
  then press **Sign in** in the chat settings (or on the notice above the chat
  box). Your browser opens to log in to Claude; no API key needed. `ave` then
  uses your Claude subscription, with only `ave`'s own tools available to it.
  The sign-in is Claude Code's own (`claude auth login`), so if you already
  use Claude Code you are signed in already.
- **Claude API key:** `setx ANTHROPIC_API_KEY "sk-ant-..."`, or paste the key
  into the chat settings.
- **Local model:** the llama.cpp server from step 7. No account needed.

### Troubleshooting

- **`link.exe` not found, or a linker error on the first build:** the C++
  build tools from step 1 are missing. Rerun that `winget` line, or open the
  Visual Studio Installer and add "Desktop development with C++".
- **`ave` is not recognised:** reopen PowerShell, or check that
  `%USERPROFILE%\.cargo\bin` is on your `PATH`.
- **"ffprobe failed" or "ffmpeg not available":** ffmpeg is not on `PATH`.
  Reopen PowerShell after installing it, or set `AVE_FFMPEG_DIR`.
- **Captions never start:** run `ave caption --work <work>` in a terminal to
  see the error. The usual cause is a wrong path in one of the `AVE_*`
  settings, or a llama.cpp build that does not match your graphics card.

## Setup reference

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

The desktop app bundles the [Inter](https://github.com/rsms/inter) typeface
(`assets/fonts/Inter-Regular.ttf`), © The Inter Project Authors, under the SIL
Open Font License 1.1; its licence text is in
[`assets/fonts/OFL.txt`](assets/fonts/OFL.txt).

---

<sub>Scaffolded with [create-rust-app](https://github.com/freeknet/create-rust-app).</sub>

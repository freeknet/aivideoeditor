<div align="center">

# AI Video Editor

**Index footage, plan beat-synced edits, render and export timelines.**

[![CI](https://github.com/freeknet/aivideoeditor/actions/workflows/ci.yml/badge.svg)](https://github.com/freeknet/aivideoeditor/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rust-1.98%2B-orange.svg)](Cargo.toml)

</div>

`ave` is a desktop video editor that cuts a first edit for you. You give it
a folder of raw clips and, if you want one, a music track. It analyses the footage (exposure,
focus, fire and LED light, camera moves, shot boundaries, plus captions from
a local vision model), finds the track's beats, bars and drops, and drafts a
cut on the beat grid (or, with no music, paced every two seconds over the
clips' own sound). Then you review it, change it by hand or by asking the
chat, and export the video or a timeline for Resolve or Kdenlive.

It was built for the weekly JaJa Flow Night promo edits; that setup is now the
*Flow Night* template, and new projects start from *General*, which favours no
kind of footage. Pick or save templates in set-up or on the Home screen. The
Flow Night rules favour fire on the drop. Footage without fire, or music without a drop,
still gets a cut, and every rule (repeats, people, footage to avoid, the
drop) is the project's to change under *Rules* in the Home screen's settings.

- [Install](#install-on-windows)
- [Make a video](#make-a-video)
- [Captions and the chat assistant (optional)](#optional-captions-and-the-chat-assistant)
- [Troubleshooting](#troubleshooting)
- [The command line (reference)](#the-command-line-reference)

## Install on Windows

Step by step on a fresh Windows 10 or 11 machine. Run the commands in
**PowerShell**. You only do this once.

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
rustup has already added to `PATH`.

`--profile release-fast` builds a few times faster than the shipped
`release` profile (no link-time optimisation) and runs at nearly the same
speed. Leave it off for the smallest, fastest binary.

**Updating later:** `git pull`, then run the same `cargo install` line again.

## Make a video

Everything from here happens in the app. Start it:

```powershell
ave
```

### Start a project

The start page asks **What are we making?** Drop the folder your clips are
in onto the window, or press **Open footage folder…**. The folder can be
anywhere and named anything. `ave` keeps everything it makes in an `_ave`
folder inside it: the analysis, the cuts and the renders. It never changes
your clips.

If the folder has no video clips in it, `ave` says so. If a subfolder has
clips (a camera card's `DCIM\100CANON`, say), it names that subfolder so
you can pick it instead.

### Follow the guide

Once a project is open, a strip under the top bar shows where it stands:

```text
✓ Footage  ›  ● Analyse  ›  ○ Music  ›  ○ First cut  ›  ○ Review  ›  ○ Export      Next: [ Set up and analyse ]
```

**Next** always does the next thing, so you can press it until the video
is done:

1. **Set up and analyse.** A short set-up asks for the footage, the music
   track and end cards, and what to make (the deliverables: a 15 s reel, a
   30 s reel, a 60 s recap…). It shows how long the analysis will take, and
   nothing slow starts until you press **Start analysis**. Analysis then
   runs on its own: it indexes the clips, captions them, maps the music,
   works out who's who and drafts the first cut. You can open the editor
   as soon as indexing finishes; the rest carries on in the background.
2. **Music** (optional). With a track, the guide maps its beats, bars and
   drops so cuts land on them. With no track the cut has no music: shots
   are paced every two seconds and keep each clip's own sound. Add a track
   any time from the *Music* tab or the project settings.
3. **First cut.** **Draft the first cut** drafts every deliverable that has
   no cut yet. It never redrafts one you have already edited.
4. **Review the cut.** The editor opens on the timeline. Play the cut,
   reorder, trim and slip shots, press `A` on a shot for ranked
   alternatives, or ask the chat for a change ("a stronger opening shot",
   "no shot of the DJ before the drop"). Every change keeps the old version,
   and a live rule check under the timeline flags anything that breaks the
   rules (a repeated clip, a performer twice in a row, a crop outside the
   frame).
5. **Export.** Renders each cut to MP4 (plus a 720p preview) and copies it
   to the output folder under the names you chose. Switch on **OTIO for
   Resolve** to also get an OpenTimelineIO timeline to finish the edit in
   Resolve or Kdenlive.

Click any stage on the strip to go to the screen for it. If you start a
step from somewhere else (an empty timeline offers **Map the music**, an
empty shot browser offers **Analyse footage**), it runs through the same
queue and shows up on the strip.

### Coming back to a project

The start page lists the projects you have opened. Pick one and the strip
carries on where you left off. Adding clips to the folder later is fine:
**Update** on the Home screen analyses only the new ones.

### The workspace

Laid out like Selects or Resolve's Cut page (details in
[`docs/GUI_CONTRACTS.md`](docs/GUI_CONTRACTS.md)):

- **Chat** (left): ask for changes in plain words; attach **references**
  as chips (`R` on any selection, `Shift-R` to say keep / avoid / replace /
  more like / feature, or type `@shot:reel-30#5`, `@bars:17-20`). Answers
  come back as proposals with a diff and the predicted rule check; applying
  one saves a new version.
- **Browser**: *Shots* (searchable: `fire>0.3 -dark person:x`, grid or
  list, hover scrub, pick/reject with P/X/U), *People* (groups, merge and
  split, a confirm queue), *Music* (beat grid, sections, drops, grid fixes).
- **Viewer**: real-time playback with audio, 16:9 / 9:16 / 4:5 framing with
  safe areas, J/K/L, I/O marks.
- **Inspector**: everything known about the selection, and the open cut as
  a shot list.
- **Timeline**: the cut on the beat grid over the waveform, with markers,
  comments, versions and undo.
- **Home** (the house button): the project's pipeline step by step, with
  each step's log, the project's settings and what it makes.

Press `?` for every keyboard shortcut.

## Optional: captions and the chat assistant

### Captions

Captions describe each shot in words (who is in it, what they are doing,
the framing), and the planner uses them to pick shots. They come from a
vision model running locally on your GPU through llama.cpp. You need about
4 GB of free video memory. Without them, analysis still works and the
captions step is skipped.

1. Download a llama.cpp Windows release from
   <https://github.com/ggml-org/llama.cpp/releases>. Pick the **Vulkan**
   build for AMD or Intel graphics, or a **CUDA** build for NVIDIA. Unzip it
   to, say, `C:\tools\llama.cpp`, the folder that contains `llama-server.exe`.
2. Download the Qwen3-VL 4B Instruct model and its vision projector from
   <https://huggingface.co/Qwen/Qwen3-VL-4B-Instruct-GGUF>:
   `Qwen3VL-4B-Instruct-Q4_K_M.gguf` and `mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf`.
   Put both in `C:\tools\models`.
3. Tell `ave` where they are, then restart it:

   ```powershell
   setx AVE_LLAMA_DIR  "C:\tools\llama.cpp"
   setx AVE_VLM_MODEL  "C:\tools\models\Qwen3VL-4B-Instruct-Q4_K_M.gguf"
   setx AVE_VLM_MMPROJ "C:\tools\models\mmproj-Qwen3VL-4B-Instruct-Q8_0.gguf"
   ```

The captions step then starts `llama-server`, captions every segment and
stops it again. Expect 7–9 s per segment.

### The chat assistant

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
- **Local model:** the llama.cpp server from the captions setup. No account
  needed, but small local models plan poorly.

## Troubleshooting

- **`link.exe` not found, or a linker error on the first build:** the C++
  build tools from step 1 are missing. Rerun that `winget` line, or open the
  Visual Studio Installer and add "Desktop development with C++".
- **"An Application Control policy has blocked this file" while building:**
  Windows Smart App Control blocks the build scripts cargo compiles. Turn it
  off in Windows Security → App & browser control → Smart App Control.
  A Dev Drive does not help.
- **`ave` is not recognised:** reopen PowerShell, or check that
  `%USERPROFILE%\.cargo\bin` is on your `PATH`.
- **"ffprobe failed" or "ffmpeg not available":** ffmpeg is not on `PATH`.
  Reopen PowerShell after installing it, or set `AVE_FFMPEG_DIR`.
- **A step failed:** the Home screen shows each step's log. Click the step
  to read why. For captions the usual cause is a wrong path in one of the
  `AVE_*` settings, or a llama.cpp build that does not match your graphics
  card.

## The command line (reference)

You never need it, but every step the app runs is also an `ave` command, on
the same `_ave` folder. That is useful for scripts and for agents. The app
picks up whatever the commands write.

```bash
ave index  <footage-dir> --work <work>   # ~40 s for 20 min of 1080p: index.json + shotlog.md
ave caption --work <work>                # Qwen3-VL captions per segment (local GPU)
ave beats  <track.wav>  --work <work>    # tempo, beat grid on the kicks, bars, drops
ave beats --none --work <work>           # or no music: a steady 2 s pace, the clips' own sound
ave proxy --work <work>                  # playback copies of 4K/HEVC clips, so the editor plays smoothly
ave people --work <work>                 # draft people.toml (who is in which clip)
ave plan reel-30 --work <work> --aspect 9x16 --seconds 30 --end-card card.png
ave check  <work>/plans/reel-30.toml --work <work>
ave render <work>/plans/reel-30.toml --work <work>      # mp4 + 720p preview (--encoder nvenc or amf: on the GPU)
ave otio   <work>/plans/reel-30.toml --work <work>      # timeline for Resolve / Kdenlive
ave --open <footage-dir or work>        # open the app on a project
```

| Step | What it produces | How |
|---|---|---|
| `index` | `shotlog.md`, `index.json`, per-clip cache | ffmpeg decode at 6 fps; Rust measures luma, focus (Laplacian), emitters split by hue into fire / LED / white, moving light (motion-compensated), camera motion (block matching → pan + zoom fit), colour cuts; segments of one camera behaviour, scored and flagged `dark soft shaky whip blown no-action` |
| `caption` | captions under each shot-log line | two stills per segment → llama.cpp server running Qwen3-VL-4B, JSON-schema constrained: summary, people (appearance + prop), moment (`performing / transition / idle / no-performer`), framing, setting, issues |
| `beats` | `music.json` | spectral-flux onsets, autocorrelation tempo, dynamic-programming beat tracking, grid shifted onto measured kick onsets, bars by low-end phase, drops by low-end jump |
| `people` | `people.toml` draft | groups clips whose performer descriptions share identifying words |
| `plan` | `plans/<name>.toml` | beat grid (breakdown → build 2+1+1 → hero on the drop → 2-beat cuts, 1-beat stutters → fire finale → end card), filled most-important-slot first under the rules; with no drop detected, the biggest rise stands in |
| `check` | pass / list of problems | no repeated stretch of a clip (30 s), no performer within 8 shots, no rejected flags or captions, drop on fire, crop inside frame, action fits the crop |
| `render` | `renders/<name>.mp4` + `-720p.mp4` | each shot conformed on its own, three at a time (frame-exact trim on the beat grid, crop, scale, light denoise, per-shot exposure and colour match), then joined, sharpened and finished with the end card and a straight music trim (no fades, no gain) or the clips' own sound; x264, NVENC or AMF |
| `otio` | `renders/<name>.otio` | OpenTimelineIO JSON: source clips at frame accuracy, end card, music, a marker on the drop |

A plan is a TOML storyboard: every shot carries a `note` saying why it is
there. The checker is the guard rail whether an edit came from the drafter,
a person, the chat or another program.

### Settings reference

- **ffmpeg / ffprobe** on `PATH`, or `AVE_FFMPEG_DIR` pointing at their folder.
- **Vision captions:** run `llama-server` yourself on port 8089
  (`AVE_VLM_URL` to change it), or set `AVE_LLAMA_DIR`, `AVE_VLM_MODEL` and
  `AVE_VLM_MMPROJ` and the captions step starts and stops it. The 4B model
  at Q4 uses about 3.7 GB of VRAM.
- **Chat:** `ANTHROPIC_API_KEY` (or the chat settings) for the Claude API;
  `AVE_LLM_URL` for a local llama.cpp chat server.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check                                         # licences, advisories, sources
cargo run --example screenshot                           # every screen, headless, to target/screenshots
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

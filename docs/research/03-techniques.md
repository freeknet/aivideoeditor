# 03: Techniques behind automatic montage, highlight and music-video editing

Research survey for `ave`, the offline engine (Rust + ffmpeg + Qwen3-VL-4B on an
RX 580 8 GB via llama.cpp Vulkan). It edits dark, dialogue-free event footage
(fire spinning, LED props, DJs) into beat-synced promo reels under hard rules:
no repeated shot, no repeated performer, and the drop lands on the hero shot.

Researched November 2026. Licences were checked against model cards and READMEs
where possible. Where a source was secondary or unclear, the text says so; check
the upstream LICENSE before shipping anything.

**What `ave` does today** (from `src/`), so the upgrades below have a baseline:

- `beats.rs`: spectral-flux onsets, autocorrelation tempo (90-180 BPM), Ellis-2007
  DP beat tracking, bars by kick phase, drops = biggest low-end jump over 4 bars.
- `analyze/`: hand-built per-frame features (luma, sharpness, emitters split
  warm/coloured/white), block-matching camera motion with outlier rejection,
  segmentation.
- `caption.rs`: two stills per segment go to Qwen3-VL with a JSON schema.
- `people.rs`: performers grouped by caption word overlap, then corrected by hand.
- `draft.rs`/`plan.rs`: a structured promo grid (breakdown, build, drop on the
  hero shot, 2-beat cuts in loud bars, phrase-end stutter, fire finale) plus rule
  checks.
- `render.rs`: frame-exact trims on the beat grid, a static crop per shot,
  per-shot exposure match, `hqdn3d`, AMF or x264.
- `otio.rs`: OTIO JSON with `file://` target URLs and `metadata.ave`.

The overall finding is that `ave`'s architecture (index to text, plan as
storyboard, rules checker) already matches where research has gone in 2025-26:
agentic, text-mediated pipelines with explicit alignment steps. The biggest gains
now come from **better signals**: learned beats and downbeats, embeddings for
look-alike and performer identity, a quality score, and a motion "visual beat".
The planner itself needs fewer changes.

---

## 1. Automatic music-video and montage editing

### State of the art

**Lineage.** Early work matched video and music tempo for "MTV-style" home video
([NTNU, 2004](https://scholar.lib.ntnu.edu.tw/en/publications/mtv-style-home-video-generation-via-tempo-analysis-2/)).
*Write-A-Video* (SIGGRAPH Asia 2019) retrieves shots per text segment with keyword
matching and a visual-semantic embedding. It then assembles them with an
optimiser over temporal constraints, camera-movement metrics and user-chosen
"idioms" ([paper PDF](https://faculty.runi.ac.il/arik/site/includes/papers/WriteAVideo.pdf),
[summary](https://www.aminer.org/pub/5dc69ae4df1a9c0c4152097f/write-a-video-computational-video-montage-from-themed-text)).
That is the same shape as `ave`: plan text, retrieve candidates, then solve under
constraints.

**Visual beats.** Davis & Agrawala, *Visual Rhythm and Beat* (SIGGRAPH 2018),
derive a visual analogue of musical rhythm from motion. A *directogram* bins
optical-flow magnitude by direction, and an *impact envelope* comes from sudden
deceleration. Aligning visual beats to musical beats makes footage look like it
is dancing ([CVPRW preview](https://openaccess.thecvf.com/content_cvpr_2018_workshops/papers/w49/Davis_Visual_Rhythm_and_CVPR_2018_paper.pdf),
[SIGGRAPH entry](https://history.siggraph.org/?p=100781), code as
[visbeat3](https://pypi.org/project/visbeat3/1.0.0)). This suits fire spinning
well: poi and staff have strong periodic direction reversals.

**MV-Crafter** (2025) splits music at onset-strength peaks until the mean clip is
about one bar long (Δbar = 4·60/bpm), so cuts land on strong beats. It detects
visual beats with the Davis directogram: local maxima in a 0.125 s window that are
at least 10 % of the global maximum above the local mean. It matches visual to
musical beats by monotonic DP with the cost
`|u_m/mean(u_m) − u_v/mean(u_v)| + |t_m/T_m − t_v/T_v|`
([arXiv 2504.17267](https://arxiv.org/html/2504.17267v1)). The DP is directly
reusable for **choosing in-points**: slide the window so that the clip's motion
peaks fall on the beats.

**MVAA** (2025) edits existing video so that its motion lines up with musical beats
([arXiv 2506.18881](https://arxiv.org/abs/2506.18881)).

**BEAT** (May 2026, trailers) rejects both "align afterwards" and strict
one-shot-per-beat mappings. Its *Bar-DP* is an energy-adaptive DP that maps one
shot to several bars: "rapid cuts accompany high-energy passages while sustained
shots span quieter bars". Atmospheric intros hold one shot for three to five bars.
A learned music-visual encoder (MuVA) is trained with Sinkhorn regularisation
([arXiv 2605.27067](https://arxiv.org/abs/2605.27067)). `draft.rs` already
hard-codes this pattern (one shot per bar in the breakdown, 2-beat cuts when loud).
BEAT suggests making it a DP over bar energy instead of a fixed grid.

**AutoMV** (Dec 2025, M-A-P/QMUL, open source) is a multi-agent system. Music tools
extract structure, vocals and lyrics. Screenwriter and director agents follow, and
a Verifier agent checks the output. It generates footage rather than selecting it,
but the **verifier loop** maps onto `ave check`
([arXiv 2512.12196](https://arxiv.org/html/2512.12196v1),
[QMUL news](https://www.qmul.ac.uk/eecs/news-and-events/news/items/reimagining-music-videos-with-ai-eecs-research-breaks-new-ground.html)).
Successors include AgentMV ([2605.10723](https://arxiv.org/pdf/2605.10723)) and
MVWeaver ([2609.06478](https://arxiv.org/pdf/2609.06478)).

**TeaserGen** (ICLR 2025) writes the narration first with an LLM, then retrieves
clips. The pretrained CLIP-style retrieval with a threshold (TeaserGen-PT) beat the
learned autoregressive model at finding relevant visuals
([arXiv 2410.05586](https://arxiv.org/abs/2410.05586)). The lesson for us: frozen
contrastive embeddings plus thresholds are a strong baseline, and no training is
needed.

**AutoTransition** (ECCV 2022) ranks transition types per shot pair from fused
audio and visual features, using a dataset mined from editing-app templates
([arXiv 2207.13479](https://arxiv.org/abs/2207.13479),
[code](https://github.com/acherstyx/AutoTransition)). V-Trans4Style extends it to
production styles ([NeurIPS 2025](https://neurips.cc/virtual/2025/loc/san-diego/133898)).
This is low priority for promo reels, which are mostly hard cuts.

**Highlight detection.** QVHighlights introduced per-2-second saliency labels and
Moment-DETR ([arXiv 2107.09609](https://arxiv.org/pdf/2107.09609)). UMT adds audio
([2203.12745](https://arxiv.org/pdf/2203.12745)). QD-DETR makes saliency
query-dependent ([2303.13874](https://www.arxiv.org/pdf/2303.13874)). Successors
such as Saliency-Guided DETR report about 58.8 mAP
([2410.01615](https://arxiv.org/html/2410.01615v1)). All of them run on frozen
CLIP/SlowFast features plus a small transformer, and they are trained on YouTube
vlogs and news. They will not transfer to dark fire footage without labels. The
transferable idea is **query-conditioned saliency**: score each segment against a
text query ("fire spinner mid-trick, flames close to camera") using
embedding similarity.

### Cut pacing: what the evidence says

- **Shot length tracks the bar.** Prétet, Richard & Peeters (ISMIR 2021) analysed
  official music videos with downbeat and shot detection. The dominant shot
  duration follows musical timing (88 BPM gives about 2.7 s, 124 BPM about 1.9 s),
  and the shot-length-to-bar correspondence is strongest in *performance* videos.
  Editors also use **anticipation**: a cut slightly before the musical event
  rather than exactly on it ([arXiv 2108.00970](https://arxiv.org/pdf/2108.00970)).
- **Film-scale pacing.** Cutting et al. found that shot-length sequences in post-1980
  films approach 1/f fluctuation, and average shot length fell from about 10-12 s
  to about 2.5-4 s. This did not correlate with IMDb ratings
  ([Cornell](https://news.cornell.edu/node/271563),
  [Bordwell](https://www.davidbordwell.net/blog/?p=14290)). For us that means
  avoiding perfectly uniform shot lengths. Some long-short variance that is
  correlated across neighbouring shots reads as natural.
- **Practitioners.** Cutting on every beat becomes "predictable" and mechanical.
  Use the beat as a guideline, align *visual accents* with musical accents, let the
  action inside the shot land on the beat, and break the pattern deliberately
  ([Videomaker](https://www.videomaker.com/article/c3/17048-rocking-out-with-a-music-video/),
  [LBB](https://lbbonline.com/news/more-than-just-cutting-to-the-beat-the-nuance-of-editing-music-videos),
  [StudentFilmmakers on downbeat vs upbeat cuts](https://studentfilmmakers.com/music-video-editing/),
  [PremiumBeat](https://www.premiumbeat.com/blog/how-to-edit-a-film-to-music-without-it-becoming-a-music-video)).

### Practical for us

Everything above that needs no training: the BEAT-style energy-adaptive bar DP,
MV-Crafter's visual-beat DP for in-points, Prétet's anticipation, and
TeaserGen-style embedding retrieval.

### Upgrades (prioritised)

1. **Visual beat envelope per segment.** `analyze/motion.rs` already block-matches;
   add a per-sample *local* (non-camera) motion magnitude and direction histogram,
   using the outliers it currently discards. Compute Davis's impact envelope from
   it. Store `visual_beats` in the clip JSON and print a "rhythmic: 0.7 @ 2.1 Hz"
   field in the shot log.
2. **In-point search by beat alignment.** In `draft.rs`, slide each candidate's
   in-point (±1 s) to maximise the alignment between visual impacts and the slot's
   beat times (MV-Crafter cost, or plain cross-correlation). The drop slot should
   put the biggest flame burst exactly on the downbeat.
3. **Energy-adaptive shot lengths** (BEAT Bar-DP). Replace the fixed loud/quiet
   grid with a DP over per-bar energy that allows 1/2/4/8-beat shots. Penalise
   uniformity (a 1/f-ish variance term) and keep phrase boundaries (4/8 bars) as
   strong cut points.
4. **Anticipation offset.** An optional `cut_lead_frames` rule (1-2 frames early)
   on hard cuts at downbeats. Prétet observed it, and it is cheap to A/B.
5. **Verifier pass** (AutoMV). After `check`, have the VLM read the plan and
   shot-log lines and flag weak slots as text, without looking at frames.

---

## 2. Video understanding that runs locally

### State of the art and candidate models

| Need | Model | Size | Licence | Notes |
|---|---|---|---|---|
| Aesthetic score (image) | LAION improved aesthetic predictor (MLP on CLIP ViT-L/14) | ~430 MB backbone + tiny head | head reported as Apache-2.0 via a mirror; **verify** upstream | Linear/MLP on frozen CLIP. Only works with the matching CLIP backbone ([LAION blog](https://laion.ai/blog/laion-aesthetics/), [HF note](https://huggingface.co/spaces/Geonmo/laion-aesthetic-predictor/discussions/2)). Trained on daylight photos, so it is biased against dark frames. |
| Quality + aesthetics (LMM) | Q-Align / OneAlign (mPLUG-Owl2 7B) | ~7B | see repo | Scores from the probabilities of "excellent…bad" ([arXiv 2312.17090](https://arxiv.org/abs/2312.17090), [GitHub](https://GitHub.com/Q-Future/Q-Align)). Too large next to Qwen3-VL on 8 GB, **but the trick transfers** (see upgrades). |
| Video quality | DOVER / DOVER-Mobile, FAST-VQA | small (Swin/ConvNeXt fragments) | check LICENSE (not stated in snippets) | Separate *technical* and *aesthetic* scores in [0,1] ([GitHub](https://github.com/vqassessment/DOVER), [arXiv 2211.04894](https://arxiv.org/abs/2211.04894)). The technical branch catches blur, noise and compression. |
| Text-to-footage, look-alikes | SigLIP 2 B/16 (86M vision), So400m (400M) | B/16 image tower ~350 MB fp32 | **Apache-2.0** | ONNX exports exist ([community bundle](https://huggingface.co/horrible/siglip2-base-patch16-224), [Qualcomm towers](https://huggingface.co/qualcomm/SigLIP2), [Google card](https://huggingface.co/google/siglip2-base-patch16-224), sizes in [paper](https://arxiv.org/pdf/2502.14786v1)). |
| Near-duplicates | DINOv2 S/B | 21M / 86M | Apache-2.0 (relicensed from CC-BY-NC) | Better than CLIP for instance-level near-duplicates ([HF card](https://huggingface.co/facebook/dinov2-base/raw/main/README.md)). |
| Avoid | MobileCLIP2 | small | apple-amlr, research only | [repo](https://github.com/apple/ml-mobileclip) |
| Person detection | RT-DETR / RT-DETRv2, D-FINE, RF-DETR | 20-80M | **Apache-2.0** | Avoid Ultralytics YOLO (AGPL-3.0) ([Roboflow](https://blog.roboflow.com/rf-detr-is-free-to-use-commercially/)). |
| Person ReID | OSNet x1.0 / OSNet-AIN | 2.2M params | MIT (torchreid) | Clothing-based appearance embedding ([arXiv 1905.00953](https://arxiv.org/pdf/1905.00953), [torchreid](https://github.com/KaiyangZhou/deep-person-reid)). AIN generalises across domains ([1910.06827](https://arxiv.org/pdf/1910.06827)). |
| Faces | InsightFace buffalo_l | — | **non-commercial weights** | Avoid. YuNet + SFace (Apache-2.0) is the permissive alternative ([discussion](https://huggingface.co/mudler/face-detect-gguf)). Faces are often masked or unlit at fire jams anyway. |
| Shot boundaries | TransNetV2 | ~8M | MIT (upstream) | ONNX available ([shot-detection](https://pypi.org/project/shot-detection/), [upstream](https://github.com/soCzech/TransNetV2)). AutoShot is newer ([2304.06116](https://arxiv.org/pdf/2304.06116)). |
| Saliency | UNISAL (4M params, 15 MB); ViNet (32M) | small | check | [UNISAL](https://github.com/rdroste/unisal), [ViNet](https://github.com/samyak0210/vinet) |
| Optical flow | NeuFlow v2 | small; >20 fps on Jetson Orin Nano | check | [arXiv 2408.10161](https://arxiv.org/abs/2408.10161) |

**Clothing change.** Standard CLIP-ReID degrades when clothes change
([CCAF, 2406.09198](https://arxiv.org/html/2406.09198v1)). That does not affect us:
within one night, the same outfit *is* the identity signal, which is exactly what
OSNet captures. The risk runs the other way. Two performers in matching black
outfits, lit only by their props, will collide. Keep the human-editable
`people.toml` as the final authority.

**Near-duplicate thresholds.** Published cut-offs are around cosine ≥ 0.90-0.93 on
CLIP/DINO frame embeddings, with a median over sampled frames, and pHash
Hamming ≤ 6-10 as a cheap pre-filter
([NeMo Curator](https://docs.nvidia.com/nemo/curator/26.02/curate-video/process-data/dedup.html),
[arXiv 2603.25091](https://arxiv.org/pdf/2603.25091),
[oximedia-dedup](https://docs.rs/crate/oximedia-dedup/0.1.2/source/src/phash.rs)).
Calibrate on our own footage. Dark frames inflate similarity because black matches
black, so compute it on emitter-masked or contrast-normalised frames.

### Rust integration and the RX 580

- **`ort`** (ONNX Runtime). DirectML is the practical GPU path on Windows with a
  DX12 card, and the RX 580 qualifies. ROCm and MIGraphX do not support Polaris. In
  2.x, DirectML ships in the Windows builds. ort falls back to CPU *silently*, so
  log the active execution provider
  ([docs.rs ort](https://docs.rs/ort), [EP source](https://docs.rs/crate/ort/latest/source/src/ep/mod.rs)).
  Licence: MIT/Apache.
- **`rten`**: a pure-Rust ONNX runtime with no native deps. It is CPU only, but fine
  for small models (OSNet, beat models, aesthetic heads). The `beat-this` crate
  uses it (see §3).
- **candle**: has CLIP and SigLIP in candle-transformers
  ([examples](https://docs.rs/crate/candle-examples/latest/source/examples/),
  [embed_anything SigLIP](https://docs.rs/crate/embed_anything/0.4.17/source/src/models/siglip.rs)).
  It has no AMD GPU backend on Windows, so it is CPU only for us.
- **`open_clip_inference`**: ONNX CLIP-family inference including SigLIP
  ([docs.rs](https://docs.rs/open_clip_inference)).
- **burn** has a wgpu (Vulkan/DX12) backend, but model coverage is thin. Treat it as
  a fallback.
- **VRAM budget.** Qwen3-VL-4B Q4 plus mmproj takes about 3.5-4.5 GB. A SigLIP-B
  image tower (fp16 ~180 MB), OSNet (~10 MB) and an RT-DETR-S (~80 MB) fit
  alongside it, but run them as **separate passes** so llama-server and DirectML
  are not fighting over memory. RX 580 llama.cpp Vulkan numbers are anecdotal:
  about 17 tok/s generation at Q4_K_M on an unnamed model, and roughly 3-4x over
  CPU ([RX 580 guide](https://dev.to/aivisionslab/running-local-ai-on-an-amd-rx-580-in-2026-the-complete-vulkan-guide-52a5),
  [llama.cpp Vulkan discussion](https://github.com/ggml-org/llama.cpp/discussions/10879)).
  No Qwen3-VL-on-Polaris benchmark was found, so measure image prefill time per
  caption.

### Upgrades (prioritised)

1. **SigLIP 2 B/16 embeddings per segment** (`ort` + DirectML, CPU fallback).
   Embed 3 frames per segment and store their mean in the clip JSON. This unlocks:
   - **look-alike rule**: `check` rejects two shots with cosine > τ (calibrated),
     a much stronger "no repeated shot" check than clip and time overlap;
   - **text search**: `ave find "LED hoop close-up"` prints ranked shot-log lines,
     and the planner can use it to fill slots;
   - **query-conditioned hero score**: similarity to prompts like "huge fireball"
     (TeaserGen-PT, QD-DETR idea).
2. **Performer identity from bodies.** Run RT-DETR person boxes on keyframes, then
   OSNet-AIN embeddings, then agglomerative clustering across clips. This replaces
   caption word-overlap in `people.rs` as the *draft*. Keep the TOML review step and
   print cluster members with their captions so the human can verify as text.
3. **Q-Align-style scoring with the VLM we already run.** Ask Qwen3-VL for a
   one-word rating (excellent/good/fair/poor/bad) and read the **token logprobs**
   from llama-server (`n_probs`). The score is the weighted mean, which gives
   calibrated continuous scores instead of a brittle integer field
   ([Q-Align](https://arxiv.org/abs/2312.17090)). Use separate prompts for
   "technical quality" and "how impressive is the performance".
4. **DOVER-Mobile technical score** as a second, footage-independent quality signal
   (blur, noise, compression), useful for rejecting soft or noisy segments.
   Verify the licence first.
5. **TransNetV2** only if the footage includes in-camera cuts or fades. Raw camera
   clips rarely do, so this is lower priority.
6. **Do not adopt** LAION aesthetics as a primary signal: it is trained on daylight
   photos. If used, normalise per shoot like `index.rs` already does for focus.

---

## 3. Music analysis

### State of the art

- **Beat This!** (CPJKU, ISMIR 2024): a transformer on mel spectrograms that beats
  the state of the art in F1 **without a DBN**. That means no fixed tempo-range or
  meter assumptions. It is weaker on continuity metrics and rare genres
  ([arXiv 2407.21658](https://arxiv.org/abs/2407.21658),
  [GitHub](https://github.com/CPJKU/beat_this)). Code and weights are **MIT**. The
  *small* model has about 2.1M params (~10 MB ONNX)
  ([ONNX exports](https://huggingface.co/ashudesai/songbird-models)). There is a
  **native Rust crate, `beat-this`**: symphonia decode, rubato resample to
  22.05 kHz, mel via a bundled ONNX graph, inference on `rten` (pure Rust) or
  `ort`, output of beats + downbeats + BPM, MIT
  ([docs.rs](https://docs.rs/crate/beat-this/0.1.0)).
- **All-In-One** (Kim & Nam, WASPAA 2023): beats, downbeats, tempo **and labelled
  sections** (intro/verse/chorus/bridge…), using neighbourhood attention on
  Demucs-separated stems and trained on Harmonix
  ([arXiv 2307.16425](https://arxiv.org/pdf/2307.16425),
  [GitHub](https://github.com/mir-aidj/all-in-one)). It needs NATTEN + Demucs +
  madmom (Python), which makes it heavy. Use it as an **offline sidecar** if at all,
  writing JSON that `beats.rs` can read. Check the licences of the repo and of
  madmom (below).
- **BeatNet / BeatNet+**: CRNN + particle filter, online, with meter inference
  ([arXiv 2108.03576](https://arxiv.org/pdf/2108.03576)). It is built for
  real-time use, which we do not need.
- **madmom**: the classic RNN+DBN. Its code is BSD but its **pretrained models are
  CC BY-NC-SA 4.0** ([PyPI](https://pypi.org/project/madmom/0.14/)). Do not ship
  them. Note that All-In-One's downbeat DBN and BeatNet's offline mode pull them in.
- **Drop detection**: Yadati et al. (ISMIR 2014) detect EDM drops with a
  content-based two-step method (segment boundaries, then classify), using
  SoundCloud timed comments as labels
  ([paper](https://archives.ismir.net/ismir2014/paper/000297.pdf)). A 2018 TMM
  follow-up adds builds and breaks. An open-source iteration exists
  ([drop-detector](https://github.com/felixmeyer6/drop-detector)). `beats.rs`'s
  "low-end jump over 4 bars" heuristic is already in this family. The usual
  missing pieces are: (a) a **build** detector (rising spectral centroid, snare
  roll density, filter sweeps, low end removed), and (b) snapping to a **phrase
  boundary** (drops land on bar 1 of a 8/16-bar phrase).

### Upgrades (prioritised)

1. **Adopt the `beat-this` crate** (small model on `rten`, CPU, a few seconds per
   track). Use its beats and downbeats instead of the Ellis DP and kick-phase guess.
   Keep the current DSP as a fallback and cross-check: if they disagree on BPM by
   anything other than ×2 or ×½, print a warning in the music map. This is the
   biggest accuracy win per line of code, and it fixes the 90-180 BPM prior and the
   hard-coded 4/4 assumption.
2. **Phrase grid.** Group downbeats into 4/8/16-bar phrases by self-similarity of
   per-bar chroma or MFCC (a checkerboard-kernel novelty curve). Require drop
   candidates to sit on a phrase start, and expose `phrase` in the music map so the
   planner can stutter on phrase ends correctly.
3. **Build detector.** Per bar, track the slope of spectral centroid, high-band onset
   density, and low-band energy drop. Score drop = (low-end jump) × (preceding build
   score). This reduces false drops on plain kick re-entries.
4. **Optional All-In-One sidecar.** `ave beats --sections <allin1.json>` imports
   labelled sections when the user has run it. Do not make it a dependency.

---

## 4. Auto-reframe (16:9 to 9:16)

### How the products work

- **Google AutoFlip** (MediaPipe, open source) works in three stages: shot
  detection, then content signals, then a per-scene crop path. Signals are fused
  with weight ranges (face core landmarks 0.85-0.9, full face 0.8-0.85, human
  0.75-0.8, pet/car 0.7-0.75, generic object 0.1-0.2) and an optional `is_required`
  flag. Per scene, it picks **stable** (all focus within
  `motion_stabilization_threshold_percent`, so the camera holds still) or
  **tracking**, plus **snap-to-centre** when close. When required content cannot
  fit, it pads with a blurred background. Detection runs on a downsampled stream
  (about 5 fps), and shot changes always reset the path
  ([docs](https://chuoling.github.io/mediapipe/solutions/autoflip.html),
  [Google blog](https://research.google/blog/autoflip-an-open-source-framework-for-intelligent-video-reframing/)).
  Raw detections "exhibit considerable jitter", which is why the path is solved per
  scene instead of following boxes.
- **Premiere Auto Reframe** (Sensei) and **Resolve Smart Reframe** (Neural Engine)
  analyse the clip and write editable **position keyframes**. Premiere exposes a
  motion-tracking speed (slower/default/faster). Resolve offers an auto or manual
  reference point. In one test, Premiere was faster and produced fewer keyframes
  (67 vs 80) on a 30 s clip
  ([Elements.tv comparison](https://elements.tv/blog/comparing-the-auto-reframe-in-davinci-resolve-and-premiere-pro/)).
- **Open-source**: pyautoflip (PySceneDetect + UNISAL saliency or faces, per-scene
  stationary/panning/tracking classification, trajectory smoothing)
  ([PyPI](https://pypi.org/project/pyautoflip/)). Vertigo and opensource-clipping
  use BlazeFace with EMA smoothing, a dead-zone and anti-jitter
  ([Vertigo](https://sysadmindoc.github.io/projects/Vertigo/),
  [opensource-clipping](https://github.com/NaufalRizqullah/opensource-clipping)).
  Industry write-ups describe splines or low-pass filters on the box centre
  ([dev.to](https://dev.to/kyle_clipspeedai/face-tracking-for-vertical-video-why-its-harder-than-it-looks-and-how-it-works-3e97)).

### What fits our footage

Faces are the wrong signal at a fire jam. The salient thing is **light**: flames
and LEDs. `analyze/features.rs` already finds emitters. A saliency map defined as
the emitter mask plus a person box is better than any face detector here, and it
is explainable in the shot log.

### Upgrades (prioritised)

1. **Per-shot crop path, AutoFlip-style.** For each planned shot, take the emitter
   centroid (weighted by brightness) and the person box per sample. Classify the
   shot as **stable** if the centroid stays within ~15 % of the crop width
   (constant crop, the current behaviour) or **tracking** otherwise. For tracking,
   fit a heavily smoothed path (a quadratic/cubic polynomial fit or a critically
   damped spring with a dead-zone, velocity-limited) and emit ffmpeg
   `crop=w:h:x(t):0` via `sendcmd` or an expression. Never smooth across a cut:
   each shot is its own scene.
2. **Cover the spinner's orbit, not the hands.** Size the crop to keep the emitter
   bounding box (the poi or staff arc) inside. If it cannot fit at 9:16, fall back
   to letterbox with a blurred fill (AutoFlip padding) rather than clipping flames.
3. **Write the path into OTIO** `metadata.ave.crop_path` (and later as keyframes in
   FCPXML/xmeml) so editors can adjust it, as Premiere and Resolve do.

---

## 5. Colour and night footage

### State of the art

- **Reinhard colour transfer** (2001): match per-channel mean and standard deviation
  in a decorrelated space (lαβ or CIELAB). It is global, fast and robust, and it
  fails on semantically mixed scenes
  ([paper](https://users.cs.northwestern.edu/~bgooch/PDFs/ColorTransfer.pdf),
  [survey arXiv 2204.13339](https://arxiv.org/pdf/2204.13339)).
  Histogram or CDF matching is the stronger but riskier sibling, since it can
  posterise.
- **Resolve Shot Match** moves target clips toward a reference clip's colour and
  contrast. It works best on normalised (non-log) footage and is treated as a
  starting point. **Colour Match** solves from a chart (ColorChecker, SpyderCheckr,
  DSC OneShot)
  ([Larry Jordan](https://larryjordan.com/articles/how-to-color-match-clips-in-davinci-resolve-20/),
  [Resolve Club](https://davinciresolveclub.com/davinci-resolve-shot-matching/)).
- **Denoise.** ffmpeg offers `hqdn3d` (fast, spatial+temporal low-pass, smears
  detail), `nlmeans` (slow, 8-bit only, best reported on low-light phone footage),
  `atadenoise` (weak on heavy noise), `bm3d`, `fftdnoiz` and `vaguedenoiser`. The
  ffmpeg wiki recommends none of them and says to compare split-screen
  ([wiki](https://trac.ffmpeg.org/wiki/denoise),
  [examples](https://trac.ffmpeg.org/wiki/DenoiseExamples?version=50)).
  `nlmeans_opencl`/`nlmeans_vulkan` exist for GPU use.
  Learned options: **FastDVDnet** (5-frame input, no flow, real-time on big GPUs)
  ([arXiv 1907.01361](https://arxiv.org/pdf/1907.01361)) and **PocketDVDNet**
  (2026, 74 % smaller, real camera noise)
  ([arXiv 2601.16780](https://arxiv.org/html/2601.16780)). Both are PyTorch and
  would need an ONNX export. At 1080p on an RX 580 via DirectML, expect well below
  real time, so they are only viable for the final render of a 30-60 s reel.

### Night-footage specifics

Mean or std transfer is dominated by the black background. **Match only on
non-black, non-emitter pixels** (the mid-tones: skin, smoke, ground). Clip
highlights out of the statistics, or flames will set the white balance. Never push
shadows up to match a brighter clip: lifting noise is worse than a mismatch.

### Upgrades (prioritised)

1. **Mid-tone Reinhard match in the render.** Pick a reference shot (the hero), and
   for each shot compute Lab mean and std over pixels with 0.08 < L < 0.7 and not
   emitter. Derive per-channel gain and offset (a 3×3 diagonal), clamp them
   (±0.3 stop, ±5 Δab), and emit `colorchannelmixer` + `eq` or a generated `.cube`
   via `lut3d`. This extends the existing per-shot exposure match to white balance.
2. **Denoise by measured noise.** Estimate noise per segment (MAD of a high-pass on
   flat dark regions) during `index`, print it in the shot log, and pick
   `hqdn3d` strength per shot, or `nlmeans` for the noisiest. This beats the current
   fixed `hqdn3d=2:1.5:4:4`. Denoise *before* any exposure lift.
3. **Export the grade, not just bake it.** Put gain, offset and LUT values in OTIO
   metadata so a colourist can start from them in Resolve.
4. Later, optionally: a FastDVDnet/PocketDVDNet ONNX final-render pass on the
   noisiest shots only.

---

## 6. Interchange: OTIO, FCPXML, Premiere XML, Resolve

### Facts and pitfalls

- **Resolve** imports and exports `.otio`/`.otioz` from **18.5** onward (File →
  Import Timeline, or Media Pool → Timelines → Import). The imported timeline takes
  its resolution from the OTIO file
  ([Resolve 18.6 manual](https://www.steakunderwater.com/VFXPedia/__man/Resolve18-6/DaVinciResolve18_Manual_files/part1411.htm),
  [OTIO wiki](https://github.com/AcademySoftwareFoundation/OpenTimelineIO/wiki/AWS-Picchu-Edit-OTIO-Conversion-Notes)).
  Known issues:
  - clips can arrive **unlinked**, especially through the scripting API, and need
    "Reconform from Bin" ([BMD forum](https://forum.blackmagicdesign.com/viewtopic.php?p=1152861));
  - **track order inverted** in Kdenlive-to-Resolve OTIO ([KDE bug 503692](https://bugs.kde.org/show_bug.cgi?id=503692));
  - **`target_url` with a `file://` prefix**: one Resolve 20.2 user on Windows
    could import only after stripping it
    ([Adobe thread](https://community.adobe.com/t5/premiere-pro-beta-discussions/now-in-beta-otio-import-and-export/m-p/14985932)).
    `otio.rs` currently writes `file:///C:/...`, so test on the user's Resolve and
    consider a `--plain-paths` switch.
- **Premiere** OTIO import and export went **official in 26.2 (April 2026)** after a
  beta that started in October 2024. The beta covered clip names, markers, frame
  size and sample rate, and "some sequence settings may not transfer"
  ([announcement](https://community.adobe.com/announcements-732/now-released-otio-import-and-export-311699),
  [release notes](https://helpx.adobe.com/sa_en/premiere/desktop/whats-new/release-notes.html)).
  Older Premiere needs **FCP7 XML (xmeml)**. Its `<pathurl>` is
  `file://localhost/` with percent-encoding, and it must resolve to a real
  filesystem path ([Adobe thread](https://community.adobe.com/t5/premiere-pro-discussions/generate-path-url-for-asset-in-xml-file/m-p/13301401),
  [Adobe help](https://helpx.adobe.com/premiere-pro/using/importing-xml-project-files-final.html)).
- **FCPXML** uses rational times. Every `offset`/`start`/`duration` must be an
  integer multiple of the `<format frameDuration>`, or Resolve and FCP warn "not
  on an edit frame boundary". NTSC rates (29.97 = 1001/30000 s) are the usual
  trap. FCPXML versions drift and Resolve follows them
  ([Apple thread](https://discussions.apple.com/thread/255681324),
  [CreativeCOW](https://creativecow.net/?p=1953313)).
- **Frame-rate traps**: Premiere XML has written 24 for 23.976 sequences
  ([Adobe](https://community.adobe.com/t5/premiere-pro-discussions/premiere-2024-xml-is-24-fps-when-the-sequence-is-23-976-fps/td-p/15568841)),
  and Resolve has read 25p as 50 in some exports
  ([Adobe](https://community.adobe.com/t5/premiere-pro-bugs/exporting-25fps-timeline-davinci-reads-as-50-fps/idi-p/14962341/page/2)).
  Mixed-rate sources get conformed to one timeline rate.
- **Unknown metadata** is ignored by NLEs, so it is safe for `metadata.ave`. Markers
  do survive into Premiere and Resolve, which makes them the best carrier for shot
  notes a human will actually see.

### Upgrades (prioritised)

1. **Golden-file tests per target.** Keep a fixture plan and assert the exact OTIO
   JSON, and check that every time is an integer frame at the *timeline* rate (not
   only the source rate) when sources are mixed-rate.
2. **Notes as markers.** Emit each shot's `note`, performer and score as an OTIO
   **marker** on the clip, in addition to metadata. Markers survive into Premiere
   26.2+ and Resolve, so the storyboard reasoning reaches the human editor.
3. **Music track and beat markers.** Include the audio clip and a marker per
   downbeat or phrase on the timeline, so editors can re-cut on the grid.
4. **Path-format switch** (`file:///` vs plain path), tested against Resolve 20 on
   Windows.
5. **FCPXML 1.9 writer** (rational times snapped to `frameDuration`) for FCP and
   older Resolve, and **xmeml** for pre-26.2 Premiere. Writing them directly in Rust
   is simpler than shelling out to OTIO's Python adapters.

---

## Overall priority list

Ordered by value per effort for this footage and these rules:

1. **`beat-this` crate for beats and downbeats** (MIT, pure Rust via rten).
   Largest accuracy gain, smallest change. Add a phrase grid and a build-aware drop
   score.
2. **SigLIP 2 segment embeddings** (Apache-2.0, `ort` DirectML). Enforces "no
   look-alike shots", adds text search, and gives a query-conditioned hero score.
3. **Visual-beat envelope + in-point alignment** (Davis/MV-Crafter). Flames hit on
   the beat, and the drop lands on the biggest burst. No model needed; reuses
   `analyze/motion.rs`.
4. **Energy-adaptive shot-length DP** (BEAT) with a uniformity penalty and
   anticipation offset, replacing parts of the fixed grid in `draft.rs`.
5. **Body-based performer clustering** (RT-DETR + OSNet-AIN, Apache/MIT) as the
   draft for `people.toml`.
6. **Q-Align-style logprob scoring** through the existing Qwen3-VL server. Gives
   continuous quality and impressiveness scores at no VRAM cost.
7. **Mid-tone colour match + measured-noise denoise** in `render.rs`.
8. **Emitter-saliency crop paths** for 9:16 (stable/tracking per shot, smoothed,
   never across cuts), carried into OTIO.
9. **Interchange hardening**: markers for notes, beat markers, a path switch,
   frame-snapping tests, then FCPXML/xmeml writers.
10. Later or optional: DOVER-Mobile, the All-In-One sidecar, TransNetV2, learned
    denoise, and AutoTransition.

### Licence watch-list (do not ship)

madmom pretrained models (CC BY-NC-SA), InsightFace weights (non-commercial),
Ultralytics YOLO (AGPL-3.0), MobileCLIP/MobileCLIP2 (Apple research licence).
Verify before use: the LAION aesthetic head, DOVER, UNISAL and All-In-One repo
licences.

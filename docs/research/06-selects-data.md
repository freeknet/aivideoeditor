# 06: Selects' data model and analysis pipeline, and what `ave` should take

Reverse-engineered from the installed Selects 2.1.301 (the Electron package is
still named `Cutback`) on 2026-11-19, read-only:

- the agent SDK contract, `resources/selects-agent-resources/sdk/*.d.ts`,
  `sdk/README.md` and `sdk/recipes/*.md`, plus a few skills
  (`fix-blur`, `multicam-cutting/references/data-map.md`, `suggest-highlights`);
- the main-process bundle extracted from `app.asar` (schemas, entity classes,
  SQLite DDL). The renderer is **not** in the bundle: it loads from
  `static.cutback.video`, and analysis runs on `selects-api.cutback.video`, so
  the server-side pipeline is known only by its inputs and outputs;
- a copy of the user's project database (the live file was never opened).

Nothing was written to Selects' files and Selects was not launched. Field and
identifier names are quoted; everything else is paraphrase.

Read with `docs/GUI_CONTRACTS.md` and `docs/research/02-ui-and-workflow.md`.

---

## 1. Where a project lives on disk

| Path | What it is |
|---|---|
| `%APPDATA%\Cutback\Cutback Studio\Temp\<libraryId>\<libraryId>.selects` | **The project: one SQLite file per library** (2.4 MB here). |
| `…\<libraryId>\Extracted\<resourceId>-0.flac`, `-0-part.wav` | Audio pulled from each source for upload. `CUTBACK_SETTINGS` in local storage sets `audioExportPreset: "wav-16k"` for transcription. |
| `…\<libraryId>\Samples\<resourceId>-0-hq.wav` | High-quality audio for playback and local measurement. |
| `…\<libraryId>\Peaks\<resourceId>-s0-<N>.lpeaks` | Waveform peaks at nine levels of detail, N = 2, 8, 32 … 4096 samples per peak. |
| `%APPDATA%\Cutback\agent-runtime.sqlite3` | Agent chat sessions and turns (`agent_runtime_sessions`, `agent_runtime_turns`). Not inspected. |
| `%APPDATA%\Cutback\Cutback Studio\Temp\CUTBACK-PROMPTS` | Saved user prompts ("Assembly cut", …). |
| `%APPDATA%\Selects\` | Only installer logs. |

Source media stay where they are and are referenced by absolute path.

### 1.1 The SQLite schema

Five tables. The model is a typed key-value document store, not relational:

| Table | Columns | Role |
|---|---|---|
| `entities` | `type, key, encoding, value BLOB, createdAt, updatedAt`, PK `(type,key)` | Every document. `encoding` is `json` or `json+gzip` (Resources are gzipped). |
| `entity_checkpoints` | the same plus `format, schemaVersion, compression, epoch, checkpointSeq, headSeq, updateBytes, migration` | Snapshot of a document for an incremental-update log. Empty here. |
| `entity_updates` | `type, key, epoch, sequence, updateId, value` | Append-only update log per document (CRDT-style), cascades from its checkpoint. Empty here. |
| `metadata` | `key, value` | `libraryId`, `schemaVersion = 3`, and one row per migration already run (`migration:caption-clocks:v1 = done`, `migration:split-draft-main:v1:<projectId> = [<draftId>]`, …). |
| `thumbnails` | `key, value BLOB` | One JPEG per Resource (16 to 24 KB), addressed as `selects-thumb://<libraryId>/<key>`. |

Every document carries a `schema` tag with a version number:
`Cutback.Project.5`, `Cutback.Resource.2`, `Cutback.Timeline.1`,
`Cutback.Track.1`, `Cutback.Clip.2`, `Cutback.Gap.2`, `Cutback.Effect.1`,
`Cutback.Transition.1`, `Cutback.Generator.2`, `Cutback.GeneratorReference.2`,
`Cutback.Animatable.1`, `Cutback.Transcription.2`,
`Cutback.TranscriptTranslation.1`, `Cutback.SourceSpanRef.2`. Loaders reject
an unknown tag instead of guessing.

### 1.2 What the user's Flow Night project holds

| Entity type | Rows | Notes |
|---|---|---|
| `Project` | 1 | "New project", 23.976 fps (stored as a ratio `24000/1001`, the float and the fraction string side by side), 1920×1080, 48 kHz, 75 file-tree nodes. |
| `Resource` | 74 | 73 `Video` (`MVI_*.MOV` from `Pictures\Flow Night\test`) and 1 `Audio` (the track). **All `status: "pending"`: no analysis was ever run**, so `visualCaptions`, `visualSpans`, `transcriptionId` are all null. |
| `Sequence` | 75 | 74 per-Resource "analyzed" source sequences (pre-analysis skeletons) and 1 Draft. |
| `MediaLink` | 74 | `<resourceId>:original` → absolute path. Lets a moved file be relinked without touching the Resource. |
| `Generator` | 2 | "Grade" and "Cinematic", kind `cutback.adjustment-layer`. |

The one Draft, "Flow Night Highlights", is a beat edit built **without
analysis** (the `unanalyzed-resource-placement` path): 162 picture clips of
exactly 20 frames (0.834 s) on one Video track, two adjustment-layer tracks
above it carrying WGSL shader effects ("Color & Tone", "Grain & halation"),
the music on the A1 track with a 48-frame fade-out, and Chapter/SubChapter
tracks holding one placeholder clip marked `unanalyzed`, text "NA". So the
analysis outputs below come from the SDK types and bundle code, not from this
project's data.

---

## 2. Entity model

### 2.1 Overview

```
Library (one .selects file)
└── Project ─┬─ fileTree (folders → file nodes → resourceId)
             ├─ resources[]  ──► Resource ──► analyzedSequenceId ──► Sequence (source, base frames)
             │                    ├─ audioSources[] / videoSources[] (camera angles)
             │                    ├─ speakers[] / profiles[]
             │                    ├─ visualCaptions, visualSpans, multicamClassification
             │                    └─ transcriptionId ──► Transcription (utterances, words, summaries)
             ├─ editedSequences[] ──► Sequence (Draft, resolved frames)
             │                         ├─ tracks: Chapter, SubChapter, Video…, Audio…
             │                         │    └─ Clip | Gap  (sourceRange in ticks, cut, enabled,
             │                         │                     mediaReferences, effects, linkId)
             │                         └─ markers[]
             ├─ syncedSequences[] (multicam timelines, themselves Resources of type Sequence)
             └─ Generator (adjustment layers, procedural media)
```

### 2.2 Project (`Cutback.Project.5`)

`id`, `libraryId`, `title`, `thumbnail`, `status`, `createdAt`,
`lastUpdatedAt`, `resources[]`, `editedSequences[]` (the Drafts),
`fileTree[]` (nodes `{id, name, type, resourceId, durationSeconds}`, folders
with children), `frameRateRatio {numerator, denominator}` plus `frameRate`
and `frameRateFraction`, `frameSize`, `sampleRate`, `syncedSequences[]`,
`stringoutSequences[]`, `brollResourceIds[]`, `soundResourceIds[]`,
`audioLabels`, `speakerMediaSync` (legacy multicam routing).

SDK view (`ProjectMeta`): `id, title, status, createdAt, lastUpdatedAt,
draftIds[]`. Folders (`FootageFolder`): `folderId, name, path, resourceIds,
draftIds, color`.

### 2.3 Resource (`Cutback.Resource.2`): one source file or synced timeline

Stored: `id`, `name`, `type` (`Video | Audio | Image | Sequence`), `status`,
`languages[]`, `content` (path, `totalSeconds`, `frameRate`/`frameRateF`,
size, sample rate, **`checksum`** (64 hex), the full ffprobe output and the
MediaInfo report), `audioSources[]` (stream index, channels, cached sample
and peak paths, `isActive`), `videoSources[]` (one per camera angle;
`kind: media | sequence`), `analyzedSequenceId`, `analyzeFrameRateRatio`,
`transcriptionId`, `speakers[]`, `profiles[]`, `audioGainDb`,
`visualCaptions`, `visualSpans`, `multicamClassification`, `syncedTopics`,
`syncedTranscript`, `speechAlignment`, display colours.

**Status lifecycle:** `pending → sampling → samplingSucceeded → analyzing →
analyzingSucceeded`, with `samplingFailed`, `analyzingFailed`,
`analysisNotApplicable`, and `analysisMerged` for a synced timeline whose
members' analyses were merged into it client-side. Derived flags:
`hasAnalysis`, `hasTranscription` (readable even mid-analysis).

SDK view adds `color`, `durationSeconds`, `recording {recordedAt,
creationAt, filenameTimestamp, timecodeStart, location {latitude,
longitude}}`, `owningSyncedSequenceResourceId`, and per file node
`hasTimecode, timecodeStartSec, frameRate, frameSize, sampleRate, slate
{scene, take, tape, note, circled}`.

**Ids.** Every Resource has a persistent UUID; the agent sees project-scoped
short ids `r0, r1, …` and every call accepts either. Synced timelines are
`s0, s1, …`.

### 2.4 Sequence (`Cutback.Timeline.1`): Drafts and source timelines are the same type

`id`, `name`, frame rate (ratio + float + fraction), `frameSize`,
`sampleRate`, `tracks.children[]`, `markers[]`, **`lastClipId`** (clip ids
are integers allocated per sequence), `multicamFlag`, `analyzedResourceId`
(set on a Resource's own source sequence), `ownerResourceId` (synced
timelines), `transcriptSource {audioTrackId, videoTrackId}` (which tracks
are "the storyline": V1 picture, A1 transcript sound), `properNouns`,
`transcriptionData`.

**Track** (`Cutback.Track.1`): `id`, `kind` (`Chapter`, `SubChapter`,
`Video`, `Audio`), `children[]` (clips laid end to end, gaps explicit),
`mute`, `visible`, `solo`, `audioOrder`, `lastGroupLocalId`.

**Clip** (`Cutback.Clip.2`): `id` (int), `sourceRange {startTime, duration}`
in **ticks of 1/705,600,000 s** (flicks, so 23.976, 25 and 29.97 fps frames
are all whole numbers), `referenceSourceDurationTick`, `mediaReferences
{defaultMedia: {id, name} | GeneratorReference}`, **`cut`** (soft cut: the
material stays in the sequence but does not play, and can be restored),
`enabled`, `name`, `effects[]`, `intrinsicVideoAdjustments`, `metadata`
(chapter title text, `unanalyzed`, `groupId`, `audioVolumeDb`,
`audioFades {in, out}` in frames). Gaps are `Cutback.Gap.2`.

SDK view (`ClipInfo`): `clipId, trackId, resourceId, linkId` (picture and
its sound share one; a V1 clip with no linked sound is a cutaway),
`sourceStartSeconds`, `trackKind` (`main | video | audio | chapter |
subchapter | sequence | other`), `startFrame, endFrame` (resolved),
`videoIndex` (camera angle), `audioSourceIndexes`, `color`,
`playbackSpeed {numerator, denominator}`, `preserveAudioPitch`.

**Marker**: `id`, `frame` (resolved), `note?`, `color?` (`green` default,
`blue`, `yellow`, `red`). Markers belong to the Draft, not a clip, so they
survive edits underneath. A marker with a note *is* a review comment (the
Comment panel lists them); grades from evaluators live on separate note-less
markers. Several may share a frame.

**Seam** (a join between two clips, computed): `id` (content-stable: the
same join keeps its id when other parts of the Draft change), `reviewKey`
(changes when anything reviewed at the join changes), `playbackFrame`,
`before`/`after` spans, `removedText`, `sourceJump` (false only when
continuity is proven, null when unknown), `hiddenDraftFrames`, `sourceGap`,
`videoAngleChanged`.

**Overlay** (captions, titles, motion graphics): `overlayId`, `range |
"full"`, `zIndex`, `label`, `data`.

### 2.5 Transcript entities (seconds of source media)

`Transcription { utterances[], summaries[], visualCaptions[] }`, aggregated
across the project.

- `Utterance { utteranceId, start, end, speaker, transcript, words[],
  resourceId }`; `TranscriptWord { word, punctuatedWord, start, end, speaker }`.
- `Summary { resourceId, type: chapter | subchapter, title, start, end }`:
  the **topics**. Chapter and SubChapter tracks in each sequence project them
  onto frames.
- Draft-side `Word` adds `cut`, `nonSpeech`, `unanalyzed` (unknown, never
  treat as silence), `type` (`word | audio_event`), `filler {type:
  hesitation | hedging | repeat, groupId, removable}`, `semanticEnd`
  (sentence boundary chosen as an edit point).
- `turns()` (speaker turns), `semanticCutSegments()` (units whose boundaries
  are natural edit points), `pauses()` (`durationSeconds`, `unrecognized`),
  `audioEvents()` (`label`, `speechOverlap`, `unverifiedAudio`),
  caption rows with translations per language.
- `Speaker { id (unique per Resource only), resourceId, name,
  audioSourceIndexes, profileId }`; `Profile { id, resourceId,
  representativeImageUri, speakerIds }` (a face, linked to the voices it
  belongs to).

### 2.6 Retake groups

`RetakeGroup { groupIndex, reference {text, source}, takes[],
bestTakeIndex, reason, selection {status, mode: audio_video | audio_only |
video_only, reason} }`. A take: `takeIndex, verdict, rank (lower is
better), note, directorCue, tags[], visualDetails, audioDetails` (free-text
descriptions keyed `appearance, setting, lighting / voice, background,
style`, so a take can be picked by content), `cutRanges`. The SDK maps takes
onto exact source spans (`mappingStatus: mapped | unavailable | ambiguous`)
and onto current Draft placements.

### 2.7 Story clips, generators, other

- **Story clips** are a separate app inside Selects: `addStory {projectId,
  title, text, sourceUrl, sourceName, reason, brief}` returns `storyId`;
  `setVoice` records a narration. They are text stories for scripted shorts,
  not footage moments.
- **Generator** (`Cutback.Generator.2`): `projectId, name, generatorKind
  (cutback.adjustment-layer, …), metadata`. Clips reference it instead of
  media.
- **Workflow** (background job, server side): `workflowId, type
  (project:create, project:analyze-resource, sequence:sync), status (queued,
  running, succeeded, failed, canceling, canceled, unknown), step, startedAt,
  estimatedDurationMs, progress 0..1, lastErrorMessage, resourceId,
  failedTasks [{id, label}]`, and for a fan-out `{done, failed, total,
  failures}`.

---

## 3. What analysis produces per source

Analysis is per Resource, server side, paid in credits. Locally the app only
probes the file (ffprobe + MediaInfo + checksum), extracts audio (FLAC, a
16 kHz WAV part for speech, an HQ WAV), builds peaks and a thumbnail. Then
`sampling` uploads samples and `analyzing` runs the models. The analysis
result is merged into the Resource and its source sequence through a fixed
field list: `summaries, title, description, visualCaptions, visualSpans,
preservedVisuals, retakeGroups, genre, isScripted, properNouns, isS2s,
speakers, speakerToSegment, speakerMediaSync`, plus the transcription.

| Output | Granularity | Shape |
|---|---|---|
| Transcript | word (seconds), utterance, speaker | §2.5 |
| Chapters / subchapters ("topics") | titled time ranges | `Summary`; re-runnable with a `regenerate_topics` tool |
| Semantic cut segments, `semanticEnd`, fillers, pauses | word-level edit points | §2.5 |
| **Visual captions** | one free-text description per stretch of source (`{resourceId, caption, start, end}` seconds). Videos only: images get no analysis. | `Caption` |
| **Scene index** (server "SearchStoryboard") | per camera track; hits are windows with a focal frame | see §4 |
| **Bad-shot spans** | intervals per camera, `{trackIndex: {badShotSpans: [[startSec, endSec]], frontalSpans: [...]}}` | `visualSpans`; blurry, shaky or out of focus |
| Frontal (facing camera) spans | intervals per camera | only from a separate `scan_gaze` tool |
| Camera facts (multicam) | per range and camera: `coverage {startFrame, endFrame, videoIndex, speakerId, score, isWide, faceVisible}`, `wideShots {frame, videoIndex, score, reason}`, `reactions {frame, videoIndex, score}`, `soloMulticam`, `screenRecording` | "moments the VLM already found engaging" |
| Multicam classification | per Resource: solo vs standard, screen recording | `multicamClassification` |
| Speakers, face profiles, speaker → camera | per Resource | §2.5 |
| Retake groups with best take | per line recorded several times | §2.6 |
| Audio events | labelled intervals (cough, laugh, …) | `audioEvents()` |
| Genre | legacy only (`REVIEW, INTERVIEW, TALKSHOW, PODCAST, OTHER`); null after 2.0 | |

Quality flags are deliberately thin: bad-shot intervals (and, for a single
camera, only after an on-demand `scan_bad_shots`, which scores every keyframe
on the server and takes minutes). There is no exposure, colour or motion
measurement exposed per shot. The SDK repeats one rule everywhere: **an empty
result is "not measured", never "clean"**, and every scan reports
`done | running | error` per Resource so the two can be told apart.

Local measurements the agent can run itself (`selects.media`):
`measureBeatSync` (audio RMS, five bands, spectral flux and onsets at 5 ms
hops; optional `beatAnalysis {bpm, firstBeat, grid: accepted | approximate |
none, hitRate, residualMedianMs, beatEnergy[], onsets [t, l|m|h band,
strength], pulseCandidates}`; video `change[]` per frame and `changes[]`
cut candidates with `changedArea`), `measureAudioPeaks`,
`measurePauseSounds` (clicks), `measureLoudness` (LUFS, true peak). Each is
documented as "measurements and candidates, not decisions".

---

## 4. Visual search and "moments"

- **Scene search is semantic and server-side**: `searchScenes(query,
  {pageSize 1..50, nextToken})` on a Resource (base frames), a Draft (resolved
  frames, optionally limited to `ranges`) or the whole project (one ranked
  search; the SDK forbids merging per-Resource pages because scores are only
  comparable within one search).
- A hit (`SceneSearchHit`) carries `resourceId`, `trackId` / `videoIndex`
  (camera), `startSeconds, timeSeconds, durationSeconds`, `score`,
  `coordinate: base | resolved`, a half-open `startFrame..endFrame` window and
  a focal `frame`. Project hits add their own `fps`.
- Pages: `nextToken` (an opaque `sc1_…` held a few hours, rejected after an
  app restart or a changed layout), `tokenAfter(hit)` to resume mid-page,
  `error`, and **`complete: false` when part of the scope had no scene index**
  (still analysing, failed, unavailable). Searches are rate-limited.
- Hits are **attested objects**: only an original hit from the same run can
  be passed to `captureFrames`, which renders at most 12 frames per contact
  sheet and 4 images per run.
- Transcript search is separate: exact (`find`, `findAll`, `findUnique`,
  `findCandidates` with surrounding turns) and semantic (`searchTranscript`,
  probabilities kept above 0.40 with confidence above 0.60, relative within a
  batch, not calibrated).
- There is **no stored "moment" entity**. Moments are (a) scene-search hits,
  (b) camera-fact `wideShots` / `reactions` with scores, (c) chapters, (d)
  highlight candidates the agent proposes in chat with a reason each and the
  user chooses (`suggest-highlights`: at most three, non-overlapping, on
  sentence boundaries).

---

## 5. Drafts, versions and revert

1. Every Draft edit in a script happens on a **working copy**. Reads on the
   same handle see the edits.
2. `stage(reason)` validates without saving and returns `editId`, an
   `EditDiff` (`beforeDurationFrames, afterDurationFrames, deltaFrames,
   opCount, removedDurationFrames, removedText, warnings`), deterministic
   **findings** (`checkId`: `word-boundary-cut`, `source-room`, `flash-clip`,
   `created-silence`, `speaker-visibility`, `multicam-structure`,
   `speechless-audio-route`; `severity: hard | soft`; `seamId`;
   `playbackFrame`; `message`) and a review packet (`stagedRevision`,
   `reviewFrames`, `changedSeams`). Findings inform; they do not block.
3. `commitAll(reason)` saves everything in one commit (needs the tool's
   `allowCommit: true`), returns `commitId` (or `createdDraftId` for a new
   Draft) and is **rejected if the saved Draft changed since the run read
   it**. Optional `wordChanges` lists words that became inaudible or audible.
4. `revertCommit(commitId)` is an immediate, revision-checked undo: it works
   only while the Draft still matches that commit's after-revision
   (`snapshot_stale` otherwise), and the receipt lives in memory for one hour
   (`expired_commit` after that or after a restart). It returns
   `beforeRevision`, `afterRevision`, `revertAvailable: false`. Neither
   rejection should be retried; make a new edit instead.
5. Some writes bypass the working copy and persist at once (transcript word
   fixes, translations, Project folder/colour/import writes, revert). The SDK
   lists them explicitly so a script is never mistaken for a simulation.
6. Versions as such are Drafts: `duplicateDraft` clones a committed Draft
   (sequence and retake groups). The storage has an update log
   (`entity_updates`) that could back history, but it was empty here.

Anti-pattern the SDK calls out: carrying frame numbers computed in one call
into a later committing call. Ranges are recomputed in the call that commits.

---

## 6. How the agent addresses things

| Thing | Address | Notes |
|---|---|---|
| Resource | `r0` short id or persistent UUID | chat mentions use `@name [resource:<uuid>]` |
| Synced timeline | `s0` | project-scoped |
| Draft | sequence UUID | `selects.draft(id)` |
| Clip | integer `clipId` per sequence, but edits take the **original `ClipInfo` row** read in this run | rows are opaque live targets, not reconstructible |
| Draft range | `Span` (live, occurrence-anchored: follows the same content across edits in the run; a deleted or foreign span throws) | `startFrame/endFrame` are output only; `rangeAtFrames` is the explicit, revision-bound escape hatch |
| Source range | `SourceSpan`; portable form `SourceSpanRef {schema, projectId, resourceId, sourceRevision, coordinate: "base", startFrame, endFrame, naturalCore?}` | `resolveSpan(ref)` throws `stale_source_span` if the source changed since |
| Span algebra | `expand({padding: "natural"})` or seconds, `expandTo(clip | turn | chapter | sentence | semanticCutSegment)`, `through`, `intersect`, `selection().union/subtract` | natural padding never swallows speech |
| Join | seam `id` (content-stable) and `reviewKey` | |
| Marker | `id` from the current `markers()` read | |
| Speaker | `(resourceId, speakerId)` pair; the user's "Speaker 2" is the label, not the id | |
| Frame in chat | `selects-action://draft-frame?sequenceId=…&frame=…`, or an OS deep link from `linkToDraftFrame` | only for a saved Draft, resolved frames |

Three coordinate systems, never mixed: Draft **resolved** frames (what
plays), source **base** frames (the source sequence, including soft-cut
material), and transcript **seconds** of the source file. Each read says
which one its numbers are in.

---

## 7. Analysis cost and estimates

- `project.estimateAnalysis({resourceIds})` is a free read: no credits, no
  confirmation, no job. It returns `resourceIds` (eligible ones), `skipped
  [{resourceId, reason: not_in_project | unsupported_resource_type |
  analysis_in_progress | already_analyzed | start_failed}]`,
  `estimatedCredits` and `estimatedDurationMs`.
- **Credits are a server quote** (`/api/selects/estimate`; the formula is not
  in the client). It is `null` when pricing fails or a media duration is
  unknown, so it is a function of media duration; `null` means unknown,
  never free; an empty selection is 0.
- `estimatedDurationMs` is the product's estimate of batch wall-clock time,
  not summed footage length, and ignores work already queued.
- `startAnalysis` rechecks eligibility, keeps its own credit confirmation,
  returns `started [{resourceId, workflowId}]` and `skipped`, and must not be
  retried automatically after refusal, failure or cancellation. Progress comes
  from `workflows()`: if `progress` is missing, report elapsed time since
  `startedAt`, never a made-up percentage; an expired estimate is not
  completion.
- **Analysis reuse:** an imported file this user already analysed can arrive
  analysed with no new credits when the server still has its analysis
  (keyed by content: the Resource stores a checksum). A miss leaves it
  `pending`, which does not mean it was never analysed.

---

## 8. Mapping to `ave`

`ave` today: a night is a work folder of text files. `index.rs` writes
`clips/<id>.json` (a `ClipIndex` of 6 Hz `Sample`s and `Segment`s, reused
while size and mtime match, versioned by `ANALYSIS_VERSION`), `index.json`
(scored segments with `q`, `flags` and `why`) and `shotlog.md`.
`caption.rs` adds a vision caption per segment (`summary, people[], moment,
props_active, blocked, framing, setting, issues[]`). `search.rs` filters and
ranks segments by caption words, facets and `like:` similarity;
`search/marks.rs` holds pick/reject/rating. `plan.rs` is the storyboard
(shots with clip, `t_in`, beats, crop, who, seg, `note`; rules; `check` →
`Problem {shot, message}`). `refs.rs` gives text references (`@seg:`,
`@clip:…@t0-t1`, `@shot:plan#n`, …) with intent modifiers and `resolve`.
`gui/assistant/store.rs` keeps every agent-applied plan as
`plans/.history/<plan>/v<N>.toml`; reverting writes an old version as a new
one.

| Selects concept | `ave` equivalent | Gap | What to add (file) |
|---|---|---|---|
| Resource status lifecycle + `hasAnalysis` / `hasTranscription` | presence of `clips/<id>.json`, `caption` on a segment; the Night page lists steps still to run | No per-clip status a page or the agent can query; "not captioned" and "captioning failed" look alike | A per-clip `status` (`pending, indexed, index_failed, captioned, caption_failed`) with the last error, derived on load (`src/index.rs`, surfaced in `src/project.rs`) |
| Workflows with `step`, `progress`, `startedAt`, `estimatedDurationMs`, `failedTasks` | `jobs.rs` (label, running) | No progress fraction, ETA or failed sub-step | `progress`, `started_at`, `estimate_ms`, `failed: Vec<(clip, reason)>` on a job (`src/jobs.rs`); fill from `index`/`caption` loops |
| `estimateAnalysis` (credits + duration, skipped with reasons) | none (local and free) | No time estimate before a long index or caption run | `estimate()` returning eligible clips, skipped clips with reasons (cached, already captioned, below `min_q`), and seconds from measured throughput (decode seconds per source second; VLM seconds per segment from the last run, stored in `ave.toml`) (`src/index.rs`, `src/caption.rs`) |
| Analysis reuse by content checksum across projects | reuse by `(size, mtime)` inside one work folder | Copying or re-importing a night re-indexes everything | A content key (size + hash of the first and last MiB) in `ClipIndex`, and an optional shared cache dir keyed by it (`src/index.rs`, `src/analyze/mod.rs`) |
| `visualCaptions` (free text per time stretch) | `Caption` per segment, from two stills | Equivalent; ours is structured, which is better for rules and search | Keep. Record the caption model and prompt version in the caption so stale captions can be found (`src/caption.rs`) |
| `visualSpans.badShotSpans` (intervals per camera) | `flags` per segment (`soft`, `shaky`, `dark`, `blown`, …) with `why` | Granularity is the whole segment (up to `MAX_LEN`); a 0.5 s focus hunt inside a good segment is invisible | `bad_spans: Vec<{start, end, flag, value}>` built from the 6 Hz samples, printed in the shot log and honoured by `draft` / `check` when a shot window overlaps one (`src/analyze/segments.rs`, `src/index.rs`, `src/plan.rs`) |
| "Empty means not measured" (`done / running / error` per scan, `complete` flag) | captions may be missing for low-`q` segments | Search results do not say how much footage was searchable | `complete` / `uncaptioned_in_scope` on search results, shown in the Shots browser and given to the agent (`src/search.rs`) |
| Semantic `searchScenes` (server embeddings, ranked, paged, focal frame) | lexical + facet search over captions, `like:` by signature | Paraphrases miss ("someone breathing fire" vs "fire eater") | A local text embedding of each caption (small model through the existing llama.cpp server's embedding endpoint), cosine ranking blended with the facet filter; keep the one-line `reason` per hit (`src/search.rs`, new `src/search/embed.rs`) |
| Hit focal `frame` + window | `Segment.peak_t`, `moments` | Equivalent (ours is measured: the flare onset) | Return `peak_t` as the hit's focal time so the viewer opens on it (`src/search.rs`) |
| Camera facts `wideShots` / `reactions` with scores | `analyze/moments.rs` bursts; `caption.moment` | Equivalent for fire; no "crowd reaction" signal | Optional: crowd-noise rise from the camera audio as a moment kind (`src/analyze/moments.rs`) |
| Chapters / subchapters (topics) | none | A night has acts (warm-up, DJ, fire set, finale) but no grouping | `sets`: runs of clips grouped by recording time gaps and caption `setting`, printed as headings in `shotlog.md` (new `src/analyze/sets.rs`, `src/index.rs`) |
| Footage grouper (`readFootage`: timecode / filename / checksum groups, residual, possible angles) and `recording.recordedAt`, GPS | `probe` (duration, size, fps) | No recording timestamp, so no ordering by shoot time or drone/7D grouping | `recorded_at` and camera model from container tags in `Probe`; groups by time gap (`src/media.rs`, `src/index.rs`) |
| Retake groups (same line several times, ranked takes, `visualDetails`) | `plan::lookalike`, `people.rs` likeness | Near-duplicate shots are rejected at check time but never grouped with a best pick | Group lookalike segments into alternatives with a rank (q, burst, rating) and a reason; the planner picks the best, the agent can offer the rest (reserved `src/alternatives.rs`) |
| Speakers / face `Profile` (`representativeImageUri`, `speakerIds`) | `people.toml` names per clip or range, drafted from captions | No representative still per person for the People page and refs | A `[profiles]` table: name → representative `clip@t` (`src/people.rs`) |
| Draft = Sequence of tracks of clips in rational ticks; soft `cut` | `Plan` with shots in beats, `t_in` in f32 seconds; OTIO export | f32 seconds drift against 23.976 frame grids; no soft cut | Snap `t_in` to the source frame grid on save and write rational time in OTIO (`src/plan.rs`, `src/otio.rs`). Soft cut: not needed; plan history covers it |
| Markers (frame, note, colour; notes are review comments) | review marks on segments (pick / reject / rating) | No notes pinned to a moment of a plan | `[[marker]] bar = 17, beat = 1, note = "…", color = "yellow"` in the plan, drawn on the timeline and exported as OTIO markers (`src/plan.rs`, `src/otio.rs`, `src/gui/pages/plan.rs`) |
| Working copy → `stage` (diff, findings, changed seams) → `commitAll` | `propose_edit`: validates on a copy, predicts `check`, diff card | Equivalent. Missing: severity and check ids, seam-level findings | `Problem {check_id, severity: hard | soft, shot, message}`; seam checks (same-clip jump, flash shot under N frames, colour jump) (`src/plan.rs`, `src/gui/assistant/tools.rs`) |
| Commit rejected if the Draft changed since read; revert only if still current | `plans/.history/<plan>/v<N>.toml`, append-only | A proposal made against v3 can be applied over a hand edit (v4) | `base_version` on each proposal; refuse (or rebase) when the plan on disk is newer (`src/gui/assistant/store.rs`) |
| `revertCommit` (1 h receipt, stale check) | revert = write an older version as new | Ours is better (durable, survives restart) | Keep |
| Opaque live `Span` and portable `SourceSpanRef {resourceId, sourceRevision, frames}`; `stale_source_span` | `@clip:MVI@t0-t1`, `@seg:MVI/3`, `@shot:plan#5` | `@seg` ids change when re-segmentation (`ANALYSIS_VERSION`) moves boundaries; `@shot` is positional, so it points at another shot after an insert | Optional revision suffix on refs (`@seg:MVI_7925/3~v2`, index version) with a stale error in `resolve`; a stable `id` per `Shot` so `@shot:reel-30#<id>` survives reordering (`src/refs.rs`, `src/plan.rs`) |
| Short ids `r0` for the model, UUIDs persisted | clip ids are file stems | Equivalent (stems are readable and stable) | Keep |
| Three explicit coordinate systems, each read says which | seconds of source; beats / bars on the plan | Equivalent | Keep the shot log and refs stating units |
| `measureBeatSync` `grid: accepted / approximate / none`, band onsets, pulse candidates | `beats.rs` music map | No confidence state on the grid | A `grid` confidence field on the music map; `draft` refuses to beat-cut on `none` (`src/beats.rs`) |
| Video `changes[]` with `changedArea` (cut candidates) | `Sample.change`, `Segment.after_cut` | Partial-frame changes (an LED panel flipping) read as cuts | Store the changed share of the 4×4 grid per sample (`src/analyze/mod.rs`) |
| `captureFrames` (max 12 frames, 4 images per run) for the agent | the house rule forbids judging by frames | Deliberate difference | Keep. Add signals instead |
| Storage: one SQLite of typed JSON documents, `schema` tags, recorded migrations, thumbnail table, peaks at 9 levels of detail | text files in the work folder | Ours is readable and diffable by people and models; only missing versioned schema tags and a peaks cache | A `schema = "ave.plan.1"`-style tag in each JSON/TOML file, checked on load; a waveform peaks cache for the timeline (`src/plan.rs`, `src/index.rs`, `src/gui/pages/plan.rs`) |
| Story clips | none | Out of scope (scripted text stories) | None |
| Transcript, fillers, semantic cut segments, translations | none | Out of scope: the footage is dialogue-free | None |

### 8.1 Priorities

1. **Sub-segment bad spans and search completeness** (`analyze/segments.rs`,
   `index.rs`, `search.rs`): the same "measured, never eyeballed; empty is
   not clean" stance Selects takes, at a finer grain than ours.
2. **Stable shot ids, revision-tagged refs and `base_version` on proposals**
   (`plan.rs`, `refs.rs`, `assistant/store.rs`): Selects' strongest idea is
   that every edit target is either live or carries the revision it was read
   at, and a stale one fails loudly.
3. **Severity and check ids on problems, plus seam checks** (`plan.rs`).
4. **Estimates and per-clip status** (`index.rs`, `caption.rs`, `jobs.rs`),
   so the Night page and the agent can say "captioning 212 segments, about
   18 minutes" before starting.
5. **Local caption embeddings for search** (`search/embed.rs`).
6. Alternatives (take groups), sets (chapters), plan markers.

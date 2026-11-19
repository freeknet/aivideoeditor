# 02 — UI and workflow research

How professional and prosumer tools design footage review, AI search, beat-driven
timelines, agentic assistance, review and delivery, and what users praise and
complain about. Ends with a recommended information architecture and a
screen-by-screen spec for the `ave` desktop app (Rust + egui).

Research date: November 2026. Source URLs are inline. Where a claim comes from
a vendor's own marketing, a competitor's blog, or a single forum post, it is
marked as such; treat those as weaker evidence than official documentation.

**Our context, restated so every recommendation can be checked against it:**
one night of dark, handheld, dialogue-free event B-roll (fire spinning, LED
props, DJs), ~80 clips / ~20 min, cut weekly into beat-synced promos in 9:16,
4:5 and 16:9. The engine already produces a shot log (segments with exposure,
focus, fire/LED share, moving light, camera move, quality score, flags, and a
vision-LLM caption), a people map, beat/bar/drop analysis, auto-drafted plans
on a beat grid with a rule checker, render and OTIO export. The GUI's job is
to let a human **review, steer and approve** that pipeline quickly, not to
replace Resolve.

---

## Contents

1. [Footage review, culling and logging](#1-footage-review-culling-and-logging)
2. [Search and filter for AI-tagged footage](#2-search-and-filter-for-ai-tagged-footage)
3. [Timeline UX for music-driven montage](#3-timeline-ux-for-music-driven-montage)
4. [Agentic / AI-assist UX](#4-agentic--ai-assist-ux)
5. [Preview, review and delivery](#5-preview-review-and-delivery)
6. [Information architecture and recurring-workflow onboarding](#6-information-architecture-and-recurring-workflow-onboarding)
7. [Keyboard maps worth copying](#7-keyboard-maps-worth-copying)
8. [Anti-patterns users complain about](#8-anti-patterns-users-complain-about)
9. [Recommended IA and screen-by-screen spec for ave](#9-recommended-ia-and-screen-by-screen-spec-for-ave)
10. [Source list](#10-source-list)

---

## 1. Footage review, culling and logging

### 1.1 Stills culling: Photo Mechanic and Lightroom Classic

The stills world solved "go through 1,000 items fast" long before video did,
and its patterns transfer almost directly to segments.

- **One key, one verdict, auto-advance.** Lightroom's `P` (pick), `X`
  (reject), `U` (unflag) plus *Photo > Auto Advance* move focus to the next
  item as soon as a flag, star or label is applied; holding Caps Lock does
  the same per keystroke
  ([Adobe LrC shortcuts](https://helpx.adobe.com/lightroom-classic/desktop/introduction-to-lightroom-classic/keyboard-shortcuts.html),
  [Photofocus](https://photofocus.com/photography/lightroom-shortcuts-for-faster-culling/)).
  `U` advancing without a verdict matters: "skip" is a first-class action.
  Photo Mechanic has the same auto-advance option after tag/rating/class
  ([PPMag power tips](https://www.ppmag.com/news/photo-mechanic-power-tips)).
- **Separate orthogonal dimensions.** Photo Mechanic gives a binary *tag*
  (`T`), eight *color classes* on number keys, and star ratings, and makes you
  choose whether number keys mean class or stars
  ([Camera Bits: Color Class Ratings](https://docs.camerabits.com/support/solutions/articles/48001142942)).
  Lesson: keep "keep/reject" (one dimension) separate from "what kind of
  shot" (another) and "how good" (a third); never overload one key family
  with two meanings.
- **Compare vs Survey.** Lightroom `C` = Compare (two-up, select vs
  candidate), `N` = Survey (n-up of a selected set, remove losers until one
  remains). Compare for two contenders, Survey for three or more
  ([Julieanne Kost](https://jkost.com/blog/2024/06/using-compare-and-survey-view-in-lightroom-classic.html)).
  **Known gap users complain about:** there is no key to advance to the next
  group in Survey; people bind mouse macros to fake it
  ([Adobe community](https://community.adobe.com/questions-675/navigation-in-survey-view-968214)).
  We should ship that key.
- **Customisable shortcuts are a release headline.** Photo Mechanic made
  custom shortcuts (with snapshots of the whole map) a featured release
  ([Camera Bits](https://home.camerabits.com/speed-up-your-workflow-with-photo-mechanics-new-custom-keyboard-shortcuts-mac-only/),
  [docs](https://docs.camerabits.com/en/support/solutions/articles/48001271746)).

### 1.2 AI-assisted culling: Lightroom Assisted Culling, Narrative Select

- **Lightroom Assisted Culling** exposes each criterion (subject sharpness,
  eye sharpness, eyes open) as a separate toggle with a **threshold slider**,
  combinable, producing Select/Reject that the user then overrides
  ([Adobe help](https://helpx.adobe.com/au/lightroom-classic/desktop/organize-photos-in-lightroom-classic/assisted-culling.html),
  [Fstoppers](https://fstoppers.com/lightroom/use-lightroom-ai-find-sharp-eyes-open-keepers-seconds-719139)).
  Users complained that intentional background blur was rejected as
  out-of-focus and that it misses closed eyes at scale (anecdote: ~50 bad in
  700 selects)
  ([Adobe community thread](https://community.adobe.com/t5/lightroom-classic-discussions/early-access-assisted-culling-lrclassic/td-p/15519069)).
  **Directly relevant:** our footage is *intentionally* dark and
  motion-blurred. A metric threshold that is right for weddings is wrong for
  fire spinning. Thresholds must be visible, per-project, and adjustable.
- **Narrative Select** groups near-duplicates into **Scenes**, ranks within a
  scene, and shows a **Close-ups panel** (grid of faces or, with no face, the
  "key element") so the user judges the deciding detail without zooming
  ([Narrative help: Scenes](https://help.narrative.so/en/articles/8119179-using-scenes-in-select),
  [Shotkit review](https://shotkit.com/narrative-select-review/),
  [Fstoppers review](https://fstoppers.com/reviews/fastest-way-cull-your-photos-reviews-narrative-select-558836)).
  Complaints: you cannot move an image to a different scene once grouped;
  progress indication on big jobs is weak.
  **Borrow:** "scene" = cluster of similar segments, sorted by our quality
  score, best first; the reviewer confirms the top one and moves on. **Avoid:**
  immutable grouping.

### 1.3 Final Cut Pro browser

- **Filmstrips + skimming.** Clips are filmstrips; moving the pointer skims
  (plays under the cursor) without selecting. Drag across a filmstrip or press
  `I`/`O` at the skimmer to mark a range (yellow border); `Option-X` clears
  ranges ([Apple: select ranges](https://support.apple.com/guide/final-cut-pro/ver28cca92/mac)).
- **Ratings apply to ranges, not just clips.** `F` favourites and `Delete`
  rejects the selected range; favourites draw a green line on the filmstrip,
  rejects red, manual keywords blue, analysis keywords purple
  ([Apple: rate clips / filter](https://support.apple.com/guide/final-cut-pro/ver3bf664ce4/mac)).
  The colored bar over the filmstrip is the single best "log at a glance"
  device in any NLE.
- **Keyword collections and Smart Collections.** Keywords on ranges create
  collections automatically; Smart Collections are saved filters at event or
  library scope, and new libraries ship with defaults (All Video, Favorites,
  Projects, Stills)
  ([Apple: Smart Collections](https://support.apple.com/guide/final-cut-pro/ver2833eb5b/mac),
  [Larry Jordan](https://larryjordan.com/articles/fcp-x-library-smart-collections/)).
- **Browser-based editing.** Many editors do the selects pass entirely in the
  browser (ranges + favourites) and only then go to the timeline
  ([Frame.io blog](https://blog.frame.io/2018/11/05/fcpx-browser-editing/)).

### 1.4 DaVinci Resolve Cut page

- **Source Tape**: the whole bin plays as one continuous reel; you scrub
  across 80 clips as if they were one, and mark/insert from there.
  **Fast Review** plays faster for longer clips, normal for short ones
  ([OWC: B-roll on the Cut page](https://owc.com/blog/how-to-edit-b-roll-and-stock-faster-with-davinci-resolves-cut-page),
  [Resolve 18 manual: Cut page](https://ltbits.github.io/davinci-resolve-manuals/DR18/DR18-RM04-TheCutPage.pdf)).
  For "20 minutes of B-roll" this is exactly the right review mode.
- **Dual timeline**: an overview strip of the whole program above a zoomed
  strip, so you never zoom in/out to navigate.
- **Intent-based edit commands** (Smart Insert, Append, Ripple Overwrite,
  Close Up, Place on Top, Source Overwrite) replace drag-and-drop for most
  edits ([Cutsio on Speed Editor modes](https://cutsio.com/blog/davinci-resolve-speed-editor-intelligent-edit-modes),
  [creative-video-tip shortcut guide](https://creative-video-tip.squarespace.com/blog/davinci-resolve-keyboard-shortcuts)).
- Complaints: the Sync Bin is hard to use and fails on some multicam projects
  ([PremiumBeat Cut page review](https://www.premiumbeat.com/blog/davinci-resolve-16-cut-page-review/));
  experienced users initially hated the Cut page because it is a *different
  philosophy* not a simplified Edit page
  ([davinciresolve21.com comparison](https://www.davinciresolve21.com/blog/davinci-resolve-cut-page-vs-edit-page-differences),
  [Skillshare course](https://www.skillshare.com/en/classes/the-cut-page-edit-faster-in-davinci-resolve/1636294272)).

### 1.5 Premiere Pro project panel

- **Hover scrub** in Icon and Freeform views: skim a thumbnail by moving
  across it; double-click opens the clip in the Source monitor at that frame
  ([ProVideo Coalition](https://www.provideocoalition.com/tool-tip-tuesday-for-adobe-premiere-hover-scrub/),
  [PremiumBeat](https://www.premiumbeat.com/blog/premiere-pro-hover-scrub/)).
  Open feature requests ask for hover scrub to drive the Source monitor and
  timeline too, and others ask to turn it off because it fires accidentally
  ([request](https://community.adobe.com/t5/premiere-pro-ideas/enable-hover-scrub-preview-from-bin-to-display-in-source-monitor/idi-p/15544206),
  [stop scrubbing thread](https://community.adobe.com/t5/premiere-pro-discussions/stop-scrubbing-in-clip-bin/m-p/14894029)).
  **Borrow:** hover scrub that *also* drives the main viewer, with a toggle.

### 1.6 Logging tools: Kyno; AI pre-editors: Selects, Eddie AI, Jumper

- **Kyno**: `M` add/edit marker, `S` add/edit subclip from in/out, `S` again
  opens the subclip's title and description; subclips get their own tab and
  batch-export ([Premierebro](https://premierebro.squarespace.com/blog/10-ways-kyno-can-speed-up-your-pre-editing-premiere-pro-workflow)).
  Complaint: deleting all markers on a clip cannot be undone.
- **Selects (Cutback)**: a standalone pre-editor that transcribes, detects
  speakers, organises by topic, builds "smart stringouts" with chapters and
  exports a structured timeline to Premiere/FCP/Resolve
  ([Cutback blog](https://cutback.video/blog/cutback-selects-ai-rough-cut-tool-for-video-editors-using-davinci-premiere-pro-final-cut),
  [GlobeNewswire](https://www.globenewswire.com/news-release/2025/12/30/3211315/0/en/Selects-Introduces-a-New-Baseline-for-AI-Assisted-Long-Form-Video-Automation.html)).
  Its positioning is ours: *pre-edit, then hand off to the NLE* (our OTIO
  export). Its strengths are dialogue-centric; ours are visual and musical.
- **Eddie AI**: transcribes and logs, builds rough cuts, has a text-based
  refinement panel, feedback mode and NLE extensions
  ([NFS review, non-independent author](https://nofilmschool.com/ai-assistant-video-editor),
  [NFS: feedback & unified logging](https://nofilmschool.com/eddie-ai-upgrades),
  [RedShark NAB 2026](https://www.redsharknews.com/eddie-ai-nab-2026-ai-video-editing-rough-cut)).
- **Jumper**: on-device visual + speech search inside Premiere/FCP; click a
  result to jump the Source monitor to the moment; "Find similar"
  ([PVC review](https://www.provideocoalition.com/review-ai-driven-video-search-with-jumper/),
  [PVC visual search tools compared](https://www.provideocoalition.com/visual-search-tools-compared/amp/)).
  Users asked for a standalone app
  ([Jumper Canny](https://jumper.canny.io/feature-requests/p/standalone-desktop-app)).

**Takeaway for 1:** the fastest flows share four traits: (a) one key per
verdict with auto-advance, (b) ranges are first-class (rate a *part* of a
clip), (c) a continuous "tape" mode for skimming everything, (d) the log is
visible *on* the filmstrip as colored bars. Our segments already are ranges,
so (b) is free.

---

## 2. Search and filter for AI-tagged footage

- **Premiere Search panel (Media Intelligence)**: one search box over visual
  descriptions (natural language), transcripts and metadata (exact match),
  analysis on-device, index stored as sidecar/cache; more specific queries
  work better ([Adobe help](https://helpx.adobe.com/premiere-pro/using/media-intelligence-and-search-panel.html),
  [Adobe FAQ](https://helpx.adobe.com/vn_vi/premiere-pro/using/media-intelligence-and-search-panel-faq.html),
  [NFS](https://nofilmschool.com/adobe-media-intelligence)). Results are
  *sections* of clips, not whole clips.
- **Search Similar Frames**: start from a frame in the Source/Program monitor
  or a result; query by image instead of text
  ([Adobe announcement](https://community.adobe.com/announcements-732/search-panel-updates-audio-search-and-search-similar-frames-311916),
  [PVC tip](https://www.provideocoalition.com/tool-tip-tuesday-for-adobe-premiere-pro-quick-finding-of-similar-shots-using-media-intelligence/)).
  Beta users report misses (clapperboards not matched to other clapperboards).
- **FCP 12 Visual / Transcript search**: added as a *filter type* in the
  existing browser search, so it composes with ratings and keywords
  ([Apple: find clips](https://support.apple.com/guide/final-cut-pro/ver65764b45/mac)).
  Larry Jordan found it unreliable (a "horse" search: 30 results, 7 horses)
  ([Larry Jordan](https://larryjordan.com/articles/apples-improved-search-in-final-cut-pro-12-is-very-unreliable/)).
  **Lesson: precision matters more than recall in a results grid; low-
  confidence hits erode trust in the whole feature.** Show the score, sort by
  it, and let the user cut off the tail.
- **Resolve IntelliSearch (Resolve 21)**: people and content search,
  including individual faces; results appear as whole clips in the Media Pool
  ([Blackmagic what's new](https://blackmagicdesign.com/products/davinciresolve/whatsnew),
  [Digital Production hands-on](https://digitalproduction.com/2026/07/29/how-much-intelligence-in-davinci-resolves-ai/)).
  Whole-clip results are a step back from range results for B-roll.
- **People grouping (Apple Photos)**: the system proposes; the human
  confirms with a Yes/No per face ("Confirm Additional Photos"), merges
  duplicates by multi-select then *Merge N People*, and names clusters
  ([Apple Photos guide](https://support.apple.com/en-ge/guide/photos/find-and-name-people-and-pets-phtad9d981ab/10.0/mac/15.0)).
  Complaints: look-alikes get merged, and fixing a bad merge is painful
  ([Apple community: correct merged albums](https://discussions.apple.com/docs/DOC-250002046)).
  Users are advised never to confirm when unsure, so "Not sure / skip" must
  exist. Our people map draft-review-rename loop is this pattern in text;
  the GUI should be the same loop with face crops.

**Takeaway for 2:** one query box that understands both free text (matched
against captions) and structured facets (`fire>0.3 dark camera:static
person:Ava`), results as *segments* with a visible match reason, a "more like
this" action from any segment, and saved searches as smart collections.

---

## 3. Timeline UX for music-driven montage

### 3.1 Beat grids and markers

- **FCP 12 Beat Detection**: `Option-B` analyses a music clip, `0` toggles
  the grid, `N` toggles snapping. Heavy lines = song parts (sections), solid
  = bars/major beats, dashed = beats, and **detail scales with zoom** (zoomed
  out you see only sections). Recommends 4/4 music. Larry Jordan: "simple and
  elegant", analysis under five seconds for a 3-minute song
  ([Apple: edit to the beat](https://support.apple.com/guide/final-cut-pro/ver65b55a2b7/mac),
  [Larry Jordan](https://larryjordan.com/articles/the-new-beat-detection-in-final-cut-pro-12-is-simple-and-effective/)).
  This is the best design of the lot, and it is a **grid**, not a pile of
  markers.
- **Resolve 20 Detect Music Beats** (Studio): places frame-quantised
  *markers*; works for beat-driven 4/4 or 3/4; snapping includes markers,
  clip edges and playhead, which compete
  ([DaVinci Resolve Club](https://davinciresolveclub.com/davinci-resolve-beat-detection/)).
  Markers clutter the marker list and cannot be "zoom-dependent".
- **Filmora**: right-click music, Beat Detection; red markers every 4 beats
  by default, configurable, with an **offset** to skip a slow intro
  ([Filmora guide](https://filmora.wondershare.com/guide/beat-detection.html)).
  Offset/downbeat correction is something users need because detectors
  miss the "one".
- **CapCut**: Auto Beat yellow markers at peaks; Auto Cut trims clips to beats;
  missing on web; competitor blogs say results need manual refinement
  ([CapCut help](https://www.capcut.com/help/auto-cut-not-working),
  [Splice blog, competitor](https://spliceapp.com/blog/what-editors-provide-timing-markers-for-beats)).
- Premiere desktop users have an open request for automatic beat detection
  ([Adobe idea](https://community.adobe.com/t5/premiere-pro-ideas/automatic-beat-detection/idi-p/14635575)),
  and third-party BeatEdit fills the gap
  ([aescripts BeatEdit for Resolve](https://aescripts.com/beatedit-for-davinci-resolve)).

**For us:** draw beats, bars, sections and **drops** as a zoom-dependent grid
in the ruler and behind the track (drops as a distinct color and a labelled
flag), let the user nudge the downbeat/offset, and make snapping prefer the
grid (bars > beats) over other targets.

### 3.2 Swap/replace a shot

- Premiere: *Replace with Clip > From Source Monitor, Match Frame* keeps
  duration and attributes and aligns a sync point; users report it
  sometimes ignores the match frame or fails for lack of head material
  ([PremiumBeat](https://www.premiumbeat.com/blog/matching-frame-and-replace-edit-in-premiere-pro/),
  [Adobe bug thread](https://community.adobe.com/t5/premiere-pro-bugs/replace-with-clip-gt-from-source-monitor-match-frame-not-matching-frame/idi-p/15285281)).
- Resolve: Ripple Overwrite (`Shift-F10`) replaces and lets the program
  length change; Source Overwrite matches by timecode.
- None of the NLEs offers what a planner can: **"show me the next-best
  candidates for this slot"**. That is our biggest UX opportunity. The plan
  knows the slot's duration, beat position, rule constraints and the shot's
  `note`; the index can rank alternatives that fit.

### 3.3 Storyboard vs timeline; magnetic vs tracks

- FCP's magnetic timeline: edits in the primary storyline close gaps; B-roll,
  titles and music connect to the clip above them; Position tool suspends
  magnetism ([Apple: magnetic timeline](https://support.apple.com/guide/final-cut-pro/verb8fcfc133/mac)).
  For montage on a fixed music bed the magnetism works *against* you: the
  music is the master clock and shots should not ripple the song.
- Resolve Cut page keeps one timeline shared with the Edit page, so moving
  between "fast" and "detailed" views loses nothing
  ([davinciresolve21.com](https://www.davinciresolve21.com/blog/davinci-resolve-cut-page-vs-edit-page-differences)).
- Descript added a storyboard view for scene-level structure
  ([Descript storyboard](https://www.descript.com/fr/storyboard)).

**For us:** our plan *is* a storyboard (shots with notes). Show it two ways
over one data model: a **storyboard strip** (one card per shot, note under
it, grouped by song section) and a **beat timeline**. Edits in either are the
same TOML change. Default to "music is fixed, shots are slots": trimming a
shot moves its neighbour's edge (roll), never shifts the song.

### 3.4 Multi-aspect versions

- **Premiere Auto Reframe** duplicates the sequence per ratio into an "Auto
  Reframe Sequences" bin; the duplicates are independent (edits to the master
  do not flow through); it refuses to enlarge frame size so you get
  non-standard resolutions; stills are not handled; applying twice causes
  black bars ([Adobe help](https://helpx.adobe.com/premiere-pro/using/auto-reframe.html),
  [Frame.io insider tips](https://blog.frame.io/2024/10/09/insider-tips-better-results-premiere-pro-auto-reframe/)).
- **Resolve Smart Reframe** (Studio): duplicate timeline, change to vertical,
  enable per clip; misidentifies subjects with multiple focal points; a
  forum user calls mixed-ratio nested delivery "a crucial flaw"
  ([Larry Jordan](https://larryjordan.com/articles/convert-horizontal-video-to-vertical-video-in-davinci-resolve-19/),
  [Blackmagic forum](https://forum.blackmagicdesign.com/viewtopic.php?p=954942)).
- **Adobe Express** resizes one size at a time; Adobe's own advice is to
  "resize your original video again and again"
  ([Adobe Express help](https://helpx.adobe.com/in/express/create-and-edit-videos/edit-videos/resize-videos.html)).
- **CapCut**: set ratio, run auto reframe, then inspect each action segment;
  captions added before the ratio change fall outside the safe area; fix
  misses with keyframes ([CapCut tool page](https://www.capcut.com/tools/auto-reframe)).

**The common complaint is divergence**: once you fork per ratio, every later
fix must be made N times. **For us:** one plan, N *framings*. A shot carries
a per-aspect crop (auto from our motion/light-centroid metrics, overridable).
The viewer shows all three ratios at once (safe-area overlays on the 16:9
master, or a 3-up), and the cut list is shared unless the user explicitly
forks a ratio (e.g. a shorter 9:16 hook).

---

## 4. Agentic / AI-assist UX

- **Premiere AI Assistant (beta, MAX 2026)**: chat panel at *Window >
  Assistant*, one tab per task, permission settings for disk and generative
  actions, every action reproducible, undoable and listed in the History
  panel. Weaknesses: if you edit while it works its steps interleave with
  yours in the undo stack; it missed many clips on a people-labelling task;
  chat goes to the cloud
  ([Adobe help](https://helpx.adobe.com/ca/premiere/desktop/premiere-ai-assistant/overview.html),
  [PVC review](https://www.provideocoalition.com/premiere-ai-assistant/),
  [Adobe announcement](https://community.adobe.com/announcements-727/meet-your-new-assistant-editor-ai-assistant-in-premiere-pro-is-now-in-public-beta-1629317)).
- **Descript Underlord**: chat co-editor; Descript added an **automatic
  review step after every editing turn** and clearer chat history and edit
  review "to make it more obvious how to control the agent's work and recover
  from mistakes"; reviewers say it is aggressive (cuts deliberate
  repetition)
  ([Descript changelog Feb 2026](https://feedback.descript.com/changelog/release-roundupfebruary-10th-2026),
  [Descript help](https://help.descript.com/hc/en-us/articles/36803785502221-Underlord-beta-Your-AI-co-editor-in-Descript),
  [Underlord primer](https://www.descript.com/blog/article/underlord-ai-video-editor-primer)).
- **Runway Aleph**: chat mode vs tool mode over a single clip; strong for
  generative VFX, not a full editing workflow; prompting overwhelms beginners
  ([Runway prompting guide](https://help.runwayml.com/hc/en-us/articles/52150503729171-Aleph-2-0-Prompting-Guide),
  [DataCamp tutorial](https://www.datacamp.com/tutorial/runway-aleph)).
- **What creators hate** (Entrepreneur): fixing one thing breaks another,
  each prompt is "a new guess" with no memory of why something worked, you
  can only describe changes "without being able to see or touch the
  underlying structure", and results are "technically fine but emotionally
  flat". Advice: edit directly when you can; skip blind re-prompting
  ([Entrepreneur](https://www.entrepreneur.com/business-news/tech/creators-are-handing-their-video-edits-to-ai-heres-where-it-starts-to-backfire)).
  ChatCut markets the inverse: 80% with control beats 90% as a black box
  ([Product Hunt](https://www.producthunt.com/posts/chatcut-2)).
- **General agentic patterns** (Smashing, Feb 2026): *Intent Preview*
  (plain-language plan, "Proceed / Edit plan / Handle it myself"), *Autonomy
  dial* per task type, *Explainable rationale* tied to the user's own rules,
  *Confidence signal*, *Action audit & undo* timeline, *Escalation* (ask a
  targeted question instead of guessing)
  ([Smashing Magazine](https://smashingmagazine.com/2026/02/designing-agentic-ai-practical-ux-patterns/)).
  An approval screen needs evidence, alternatives and a rollback path or the
  human becomes "a rubber stamp with anxiety"
  ([DEV Community](https://dev.to/pinnasys/how-to-implement-human-in-the-loop-controls-for-ai-agents-58ol)).
  Don't gate low-stakes actions
  ([agent blueprints](https://jagguvarma15.github.io/agent-blueprints/modifiers/human-in-the-loop/)).

**For us, the structural advantage:** our agent's output is a *plan file*
with a `note` per shot and a rule checker. That gives us, for free, what the
black-box tools lack: a visible underlying structure, a per-shot rationale,
diffable versions, and a machine check. The UX should therefore be:
**chat proposes a plan diff → diff shown in the storyboard (added / removed /
swapped shots, each with its note and the shot-log line that justified it) →
accept all / per shot / reject → `check` runs automatically → new version.**
Never regenerate the whole plan for a local request; scope changes to the
shots named (by selection or by section).

---

## 5. Preview, review and delivery

- **Proxies**: Resolve users report that proxies alone do not fix stutter
  and that lowering playback resolution and render cache matter as much
  ([Blackmagic forum](https://forum.blackmagicdesign.com/viewtopic.php?p=1115402),
  [Videvo](https://www.videvo.net/blog/?p=40868)). For us: generate small
  proxies at `index` time (we already decode every frame), always play
  proxies in the GUI, render from originals. No user-facing toggle needed
  beyond a "full quality frame" check.
- **Frame.io**: timestamped comments; **range comments** created by holding
  `R` during playback ("great for selecting sounds based on the rhythm of the
  music"); clicking a range comment loops that range; version stacks with
  side-by-side **Compare Versions**; all versions under one review link
  ([Frame.io commenting](https://support.frame.io/en/articles/3331455-commenting),
  [Videomaker on range comments](https://www.videomaker.com/news/frame-io-v3-5-adds-range-based-comments/),
  [version stacking](https://support.frame.io/en/articles/4431-version-stacking-and-comparison),
  [V4 player & commenting](https://blog.frame.io/2024/05/28/frame-io-v4-features-player-and-commenting/)).
  A Dropbox Replay user notes you cannot see the previous version's comments
  while comparing, an obvious gap
  ([Dropbox forum](https://community.dropbox.com/en/discussion/842839/compare-versions-show-comments)).
- **Delivery queues**: Resolve's Render Queue (serial), batch-adding several
  timelines with one preset since 18.6, but custom-filename tokens were
  ignored in that path and "Render All" was greyed out after a batch-add, per
  forum reports
  ([tryuncle guide](https://tryuncle.com/learn/davinci-resolve/how-to-render-multiple-timelines-at-once-in-davinci-resolve),
  [Blackmagic forum](https://forum.blackmagicdesign.com/viewtopic.php?p=1155619),
  [Larry Jordan on Resolve 20 batch export](https://larryjordan.com/articles/caution-batch-export-in-davinci-resolve-20-doesnt-do-what-you-expect/)).
  Adobe Media Encoder: presets dropped onto a watch folder each add an output,
  so one source yields many deliverables; parallel encode where possible
  ([Adobe: watch folder](https://www.adobe.com/learn/media-encoder/web/create-watch-folder),
  [AME overview](https://helpx.adobe.com/media-encoder/desktop/encoding-quick-start-and-basics/overview-media-encoder-user-interface.html)).

**For us:** a delivery matrix: rows are plan versions, columns are
deliverable presets (Reel 9:16 1080x1920, Feed 4:5 1080x1350, YouTube 16:9
1920x1080, plus OTIO); tick cells, press Render. Filenames from a template
that is visible before rendering. Local "review" = A/B compare of two plan
versions with range notes (hold `R`) that become plan comments.

---

## 6. Information architecture and recurring-workflow onboarding

- **Resolve pages** (Media, Cut, Edit, Fusion, Color, Fairlight, Deliver):
  a left-to-right row of pages that *is* the workflow, all sharing one
  project ([TourBox: pages explained](https://www.tourboxtech.com/en/news/davinci-resolve-pages-explained.html)).
  Strength: the page bar teaches the order of work. Weakness: two pages
  (Cut/Edit) doing overlapping jobs confuse people, and newcomers are told to
  ignore one ([Skillshare](https://www.skillshare.com/en/classes/the-cut-page-edit-faster-in-davinci-resolve/1636294272)).
- **FCP single window**: browser, viewer, timeline, inspector always present;
  everything is a mode of one window. Fast, but the browser and timeline fight
  for space on small screens.
- **CapCut / Filmora**: one window, templates first, export dialog at the end.
- **Premiere workspaces** are user-arranged panel layouts; flexible, but the
  AI Assistant chat panel shows the cost: the agent's steps blend into the
  editor's history.

**Weekly recurring workflow.** The user does the *same* job every week with
new footage. The patterns that help: project templates/presets (Resolve
project presets, FCP library defaults, Smart Collections that ship in every
new library), saved render presets, and remembering last-used settings. For
us: a **"New night from last week"** action that copies music analysis
choices, people map (the same performers recur), rule-checker thresholds,
deliverable presets and naming template, then only asks for the footage
folder and the track.

---

## 7. Keyboard maps worth copying

Industry-standard transport keys are not negotiable: every editor's hands
already know them.

| Action | Key | Source / convention |
|---|---|---|
| Play reverse / pause / forward (tap again = faster) | `J` `K` `L` | Universal in FCP, Premiere, Resolve |
| Frame step | `←` `→` (with `K` held: `J`/`L`) | Universal |
| Mark in / out / clear | `I` `O` `Option/Alt-X` | [FCP](https://support.apple.com/guide/final-cut-pro/ver28cca92/mac) |
| Play around / loop selection | `/` and `Cmd/Ctrl-L` | FCP / Premiere |
| Pick / reject / unflag (+ auto-advance) | `P` `X` `U` | [Lightroom](https://helpx.adobe.com/lightroom-classic/desktop/introduction-to-lightroom-classic/keyboard-shortcuts.html) |
| Favourite range / reject range | `F` / `Delete` | [FCP](https://support.apple.com/guide/final-cut-pro/ver3bf664ce4/mac) |
| Compare / Survey | `C` / `N` | Lightroom (conflicts with snapping `N` in NLEs; see below) |
| Append / insert / overwrite / connect | `E` `W` `D` `Q` | [FCP](https://support.apple.com/guide/final-cut-pro/ver90ba5929/mac) |
| Snapping toggle | `N` | FCP, Resolve |
| Beat grid toggle / detect | `0` / `Option-B` | [FCP 12](https://support.apple.com/guide/final-cut-pro/ver65b55a2b7/mac) |
| Ripple overwrite / smart insert | `Shift-F10` / `F9` | Resolve Cut preset (third-party cheat sheets) |
| Marker / subclip | `M` / `S` | [Kyno](https://premierebro.squarespace.com/blog/10-ways-kyno-can-speed-up-your-pre-editing-premiere-pro-workflow) |
| Range comment during playback | hold `R` | [Frame.io](https://support.frame.io/en/articles/3331455-commenting) |
| Color class / category | `1`–`8` | [Photo Mechanic](https://docs.camerabits.com/support/solutions/articles/48001142942) |
| Command palette | `Ctrl-K` / `Ctrl-Shift-P` | VS Code convention; we already have `command_palette.rs` |

Conflict to resolve: Lightroom uses `N` for Survey, NLEs use `N` for
snapping. We are an editor first, so `N` = snapping; Survey goes on `G`
("group") and is scoped to the Review page only.

---

## 8. Anti-patterns users complain about

1. **Black-box regeneration.** A local request regenerates everything and
   breaks what worked ([Entrepreneur](https://www.entrepreneur.com/business-news/tech/creators-are-handing-their-video-edits-to-ai-heres-where-it-starts-to-backfire)).
2. **Agent actions mixed into the user's undo stack**
   ([PVC on Premiere Assistant](https://www.provideocoalition.com/premiere-ai-assistant/)).
3. **Low-precision search results** presented with the same weight as good
   ones ([Larry Jordan on FCP 12](https://larryjordan.com/articles/apples-improved-search-in-final-cut-pro-12-is-very-unreliable/)).
4. **AI thresholds tuned for the wrong genre** (intentional blur rejected)
   ([Adobe community](https://community.adobe.com/t5/lightroom-classic-discussions/early-access-assisted-culling-lrclassic/td-p/15519069)).
5. **Immutable auto-grouping** (Narrative scenes) and **hard-to-undo merges**
   (Apple People) ([Narrative help](https://help.narrative.so/en/articles/8119179-using-scenes-in-select),
   [Apple community](https://discussions.apple.com/docs/DOC-250002046)).
6. **Forked aspect-ratio versions** that drift and must be fixed N times
   ([Adobe Auto Reframe](https://helpx.adobe.com/premiere-pro/using/auto-reframe.html),
   [Adobe Express](https://helpx.adobe.com/in/express/create-and-edit-videos/edit-videos/resize-videos.html)).
7. **Beat markers as clutter** rather than a zoom-aware grid; markers that
   compete with other snap targets
   ([Resolve Club](https://davinciresolveclub.com/davinci-resolve-beat-detection/)).
8. **Silent batch-export surprises** (ignored filename tokens, disabled Render
   All) ([Blackmagic forum](https://forum.blackmagicdesign.com/viewtopic.php?p=1155619)).
9. **Destructive actions without undo** (Kyno "delete all markers")
   ([Premierebro](https://premierebro.squarespace.com/blog/10-ways-kyno-can-speed-up-your-pre-editing-premiere-pro-workflow)).
10. **Missing "next group" key in survey** forcing macro hacks
    ([Adobe community](https://community.adobe.com/questions-675/navigation-in-survey-view-968214)).
11. **Weak progress indication** on long analyses
    ([Shotkit](https://shotkit.com/narrative-select-review/)).
12. **Hover scrub that cannot be turned off** or that does not drive the
    main viewer ([Adobe community](https://community.adobe.com/t5/premiere-pro-discussions/stop-scrubbing-in-clip-bin/m-p/14894029)).

---

## 9. Recommended IA and screen-by-screen spec for ave

### 9.1 Principles

1. **The pipeline is the navigation.** A Resolve-style page bar, but one page
   per pipeline stage, and no two pages that do the same job. *(Borrowed from
   Resolve pages; avoids Cut vs Edit confusion.)*
2. **Text is the source of truth, pictures are the preview.** Every screen
   reads and writes the same files the CLI does (`shotlog.md`, people map,
   plan TOML). A GUI action is an edit to one of those files, so the agent
   and the human work on one structure. *(Answers anti-pattern 1: the
   structure is visible and touchable.)*
3. **The music is the clock.** Shots are slots on a beat grid; trimming
   rolls, it never ripples the song. *(Rejects FCP magnetism for this job.)*
4. **One plan, many framings.** Aspect ratios are views of one cut, not
   forks. *(Answers anti-pattern 6.)*
5. **Every AI decision shows its reason and its confidence, and can be
   overridden in one key.** *(Smashing: rationale, confidence, undo.)*
6. **Keyboard-complete.** Every action reachable from the keyboard and the
   command palette; the mouse is optional. *(Photo Mechanic, Lightroom.)*

### 9.2 Page bar

```
[ Night ]  [ Review ]  [ People ]  [ Music ]  [ Plan ]  [ Deliver ]      ⌘K
   1           2          3           4          5          6
```

`Ctrl-1`…`Ctrl-6` switch pages. Each tab carries a status dot: grey = not
run, spinner = running with % (anti-pattern 11), amber = needs review (e.g.
unconfirmed people, failing `check`), green = done. A persistent bottom bar
shows background jobs (index, caption, beats, render) with cancel.

The **viewer** and **transport** are shared across pages (one decoder, one
playhead model), docked right or top depending on page.

### 9.3 Page 1: Night (project home)

*Borrows: CapCut/Filmora template-first start, Resolve project presets, FCP
default Smart Collections.*

- **New night from last week** (primary button): pick footage folder and
  track; inherits people map, thresholds, rule set, deliverables, naming
  template from the previous night.
- **Pipeline card list**: index → caption → beats → people → plan → render,
  each with last-run time, counts ("80 clips, 412 segments, 37 flagged"),
  run/re-run buttons, and a "view log" link. This is the GUI face of the CLI
  loop in `CLAUDE.md`.
- **Recent nights** list with thumbnail strip of the last delivered reel.

### 9.4 Page 2: Review (shot log as a culling grid)

*Borrows: FCP filmstrip + colored range bars, Lightroom P/X/U auto-advance,
Narrative Scenes and Close-ups, Resolve Source Tape and Fast Review, Premiere
hover scrub, Premiere/FCP unified search.*

Layout: left **filter rail**, centre **segment grid or list**, right
**viewer + inspector**.

- **Query bar** at top. Free text is matched against captions; tokens are
  facets: `fire>0.3`, `led`, `dark`, `focus<0.4`, `move:pan`, `person:Ava`,
  `flag:blown`, `q>0.7`, `pick`, `reject`. Recognised tokens turn into
  removable **chips**. Results are segments, sorted by relevance then
  quality, each with a one-line **match reason** ("caption: 'poi spinner
  silhouetted'; fire 0.62"). A *score cut-off* slider hides the tail
  (anti-pattern 3).
- **Filter rail**: facet histograms for each metric (exposure, focus,
  fire/LED share, moving light, camera move, quality) with draggable ranges;
  people chips; flags; pick state. **Saved searches** = Smart Collections,
  with defaults shipped: *Best fire*, *LED close-ups*, *DJ booth*, *Crowd*,
  *Flagged*, *Unreviewed*.
- **Segment cards**: filmstrip thumbnail with hover scrub that also drives the
  viewer (toggle in the view menu, anti-pattern 12); under it, a thin
  colored bar per metric (FCP's colored lines, extended to our metrics), the
  caption's first line, quality score, pick/reject badge.
- **Group by scene** toggle: clusters near-duplicate segments (same clip,
  similar caption/metrics), best first, collapsed to the top card with a
  "+4" stack count. Unlike Narrative, the user can drag a segment out of a
  group or merge groups (anti-pattern 5).
- **Tape mode** (`T`): plays every segment in the current result set back to
  back in the viewer, Resolve Source Tape style, with **Fast Review**
  (`Shift-L`) that plays long segments faster. `P`/`X`/`U` work during
  playback and advance to the next segment.
- **Survey** (`G`): selected 2–6 segments tiled with synced playback; `X` on
  one removes it; `]` advances to the next scene group (anti-pattern 10).
- **Inspector**: full caption, all metrics with the threshold that set any
  flag, people present, the clip's other segments as a mini timeline, and
  "used in plan v3, shot 7" back-links. **Find more like this** (`Shift-F`)
  runs similarity over caption embedding + metrics.
- **Thresholds drawer**: per-night sliders for what counts as "dark",
  "soft", "blown". Changing one re-flags live. Default thresholds tuned for
  night footage, not daylight (anti-pattern 4).
- **Notes**: `M` adds a note at the playhead that is written back as a
  caption field the planner reads (fits the project rule "add a signal to the
  index instead of opening images").

### 9.5 Page 3: People

*Borrows: Apple Photos Confirm Additional Photos and Merge, our existing
text people-map review loop.*

- Grid of **clusters** (face/body crops, count, name or "Unnamed 3").
- Select a cluster: its segments on the right; name it inline (`Enter`).
- **Confirm queue** (`Space` to start): one proposed segment at a time,
  `Y` yes, `N` no, `S` not sure / skip (Apple users are advised never to
  confirm when unsure). Auto-advance.
- **Merge** (`Ctrl-M` on a multi-select) and **Split** (select segments,
  `Ctrl-Shift-M`) are both undoable (anti-pattern 5).
- **Manual range** (`I`/`O` in the viewer then `Ctrl-P`): writes a
  `clip@from-to` entry, exactly what the CLI's people step accepts.
- People carry over between nights; the confirm queue for a new night starts
  with "Is this Ava?" matches against last week's names.

### 9.6 Page 4: Music

*Borrows: FCP 12 beat grid (zoom-dependent detail, `0` toggle), Filmora
offset, Resolve dual timeline.*

- **Waveform** of the track, full width, with the beat grid drawn over it:
  sections as heavy lines with labels (intro, build, drop, break, outro),
  bars solid, beats dashed, detail by zoom; **drops** as a distinct accent
  color and a flag in the ruler.
- **Fix the grid**: drag the downbeat; nudge offset (`,`/`.` by one beat);
  set tempo manually if detection is off; mark/unmark a drop (`D`); rename a
  section. Saved back to the beats analysis file.
- **Cut region**: `I`/`O` to choose which part of the song each deliverable
  uses (e.g. 9:16 uses build + first drop, 15 s; 16:9 uses 60 s). These are
  per-deliverable song ranges, stored with the deliverable preset.
- Overview strip above, zoomed strip below (Resolve Cut page dual timeline).

### 9.7 Page 5: Plan (the edit)

*Borrows: Resolve Cut page (dual timeline, intent-based edits), FCP beat
snapping, Descript review-after-every-turn, Smashing Intent Preview and
Action Audit, Frame.io version compare.*

Layout:

```
┌──────────────────────────── viewer ────────────────────────────┐ ┌─ assistant ─┐
│  [16:9 master with 9:16 and 4:5 safe-area overlays]  [3-up ▢]  │ │ chat        │
└────────────────────────────────────────────────────────────────┘ │ proposals   │
  storyboard strip: cards grouped by song section, note under each  │ history     │
  ───────────────────────────────────────────────────────────────── │             │
  overview timeline (whole song)                                    └─────────────┘
  zoomed timeline: V1 shots on beat grid, A1 music waveform, drop flags
  check bar: ✔ 14 rules passed  ⚠ 2 warnings  ✖ 1 error  (click to jump)
```

- **Version picker** at the top: `v1 (auto draft)`, `v2 (more fire in drop)`
  …; each version is a plan TOML in history. **Compare** (`Ctrl-D`) shows two
  versions as a storyboard diff (added green, removed red, swapped amber)
  and an A/B viewer that plays both in sync.
- **Storyboard strip**: one card per shot: thumbnail, duration in beats,
  people chips, and the **note** (why it is there) as editable text. Cards
  sit under their song section headers. Drag to reorder; reorder keeps slot
  durations (music is the clock).
- **Timeline**: V1 shots snapped to the grid; A1 the music; snapping (`N`)
  prefers bars, then beats, then other targets (anti-pattern 7). Trim at an
  edit = **roll** by default (song length fixed); `Shift`-trim = ripple when
  the user explicitly wants to change total length.
- **Swap** (`A` for alternatives): on a selected shot, opens a strip of the
  top 8 candidate segments that fit the slot (duration available, rules
  satisfied, not already used, similar or complementary to the note), each
  with its match reason. `1`–`8` picks one. `Shift-A` widens the search with
  a query. This is the flow no NLE offers (section 3.2).
- **Framing per aspect**: on a selected shot, the viewer's 3-up shows the
  auto crop for each ratio (from motion and light-centroid metrics); drag to
  override one ratio. Overrides are stored per shot per aspect. "Fork for
  9:16" exists but is explicit and shows a "diverged" badge thereafter.
- **Check bar**: runs `check` on every change (debounced); each failure links
  to the shot; rendering a failing plan requires a typed confirmation and
  stamps the output as "unchecked" (matches the project rule "never render a
  plan that fails `check` without saying so").
- **Assistant panel** (`Ctrl-J`):
  - Scope chips above the input show what the request applies to: *whole
    plan*, *selected shots*, *section: drop 1*. Default is the selection, so
    local requests stay local (anti-pattern 1).
  - The agent replies with a **proposal**, never a silent edit: a plain-
    language summary ("Replace shots 5, 7, 9 with higher fire share; keep
    durations"), the storyboard diff inline, each changed shot with its new
    note and the shot-log line it relied on, and the predicted `check`
    result. Buttons: **Apply**, **Apply selected**, **Edit**, **Discard**.
    *(Intent Preview + rationale + confidence.)*
  - When the request is ambiguous ("make it punchier"), the agent asks one
    targeted question with options (*shorter shots in the build / more drops
    of fire / faster camera moves*). *(Escalation pathway.)*
  - Applying creates a new plan version, so agent changes are never mixed
    into the user's fine-grained undo stack; undoing an agent turn is
    "revert to v4" (anti-pattern 2).
  - **History** tab: chronological audit of agent and user versions with
    who/what/when.
  - **Autonomy setting** per night: *Suggest only* (default) or *Apply and
    show diff* for low-stakes actions such as re-running `check` or
    regenerating framings.
- **Notes/comments**: hold `R` during playback to drop a range note on the
  plan (Frame.io range comment); notes are visible to the agent as
  instructions for the next turn and in Compare view for both versions
  (fixes the Dropbox Replay gap).

### 9.8 Page 6: Deliver

*Borrows: Media Encoder multiple presets per source, Resolve render queue,
our OTIO export.*

- **Matrix**: rows = plan versions (default: the current one), columns =
  deliverable presets (Reel 9:16, Feed 4:5, Wide 16:9, OTIO, plus user
  presets). Each cell shows estimated duration and song range. Tick cells,
  press **Render** (`Ctrl-Enter`).
- **Filename template** with live preview of every resulting filename before
  render (anti-pattern 8): `{night}_{version}_{aspect}_{date}.mp4`.
- **Queue**: serial or parallel per machine setting, progress per job,
  open-folder on completion, and a final "unchecked plan" warning if any job
  came from a failing plan.
- **Hand-off**: after render, a review card per output with play, reveal
  file, copy path; and "Export OTIO for Resolve" for the human's final pass.

### 9.9 Global keyboard map (ave)

| Scope | Keys |
|---|---|
| Global | `Ctrl-1..6` pages, `Ctrl-K` command palette, `Ctrl-Z`/`Ctrl-Shift-Z` undo/redo, `Ctrl-J` assistant, `?` shortcut sheet, `F11` full-screen viewer |
| Transport (any page) | `J` `K` `L` (repeat to speed up), `←`/`→` frame, `Shift-←/→` one beat, `Alt-←/→` one bar, `Space` play/pause, `I`/`O` mark, `Alt-X` clear, `/` play selection, `Ctrl-L` loop |
| Review | `P` pick, `X` reject, `U` skip (all auto-advance), `1`-`8` category, `T` tape mode, `Shift-L` fast review, `G` survey, `]`/`[` next/prev group, `Shift-F` more like this, `M` note, `/` focus query bar (when not playing) |
| People | `Space` start confirm queue, `Y`/`N`/`S` yes/no/skip, `Enter` rename, `Ctrl-M` merge, `Ctrl-Shift-M` split, `Ctrl-P` add manual range |
| Music | `0` toggle grid, `D` toggle drop, `,`/`.` nudge downbeat, `I`/`O` song range |
| Plan | `N` snapping, `A` alternatives, `1`-`8` pick alternative, `Shift-A` alternatives with query, `E` append selected segment, `W` insert at playhead, `D` overwrite at playhead (FCP), `Delete` lift (leave gap = black slot flagged by check), `Shift-Delete` remove and roll neighbours, `R` hold for range note, `Ctrl-D` compare versions, `Ctrl-Enter` render current |
| Deliver | `Space` toggle cell, `Ctrl-Enter` render ticked |

All bindings live in one table and are remappable (Photo Mechanic's
headline feature), with the command palette as the discoverable fallback.

### 9.10 Why this shape and not another

- **Pages, not one FCP-style window**: our stages have different main
  objects (segments, people, a song, a plan, outputs). A single window would
  need modes anyway; pages make the weekly order visible and match the CLI
  loop one-to-one.
- **No separate "Edit" page beyond Plan**: fine trimming, color, titles and
  mix belong in Resolve via OTIO. Building a second editor would repeat
  Resolve's Cut/Edit confusion and the project's own guidance that the
  human judges the final edit.
- **Assistant docked on Plan only**: the agent's main lever is the plan.
  Review and People have deterministic tools (filters, confirm queues) that
  are faster than chat for those jobs; the palette can still send a
  selection to the assistant from any page.

---

## 10. Source list

Official documentation
- Adobe Lightroom Classic shortcuts — https://helpx.adobe.com/lightroom-classic/desktop/introduction-to-lightroom-classic/keyboard-shortcuts.html
- Adobe Lightroom Assisted Culling — https://helpx.adobe.com/au/lightroom-classic/desktop/organize-photos-in-lightroom-classic/assisted-culling.html
- Camera Bits color classes — https://docs.camerabits.com/support/solutions/articles/48001142942
- Camera Bits custom shortcuts — https://docs.camerabits.com/en/support/solutions/articles/48001271746
- Apple FCP select ranges — https://support.apple.com/guide/final-cut-pro/ver28cca92/mac
- Apple FCP rate/filter clips — https://support.apple.com/guide/final-cut-pro/ver3bf664ce4/mac
- Apple FCP find clips — https://support.apple.com/guide/final-cut-pro/ver65764b45/mac
- Apple FCP Smart Collections — https://support.apple.com/guide/final-cut-pro/ver2833eb5b/mac
- Apple FCP keyboard shortcuts — https://support.apple.com/guide/final-cut-pro/ver90ba5929/mac
- Apple FCP edit to the beat — https://support.apple.com/guide/final-cut-pro/ver65b55a2b7/mac
- Apple FCP magnetic timeline — https://support.apple.com/guide/final-cut-pro/verb8fcfc133/mac
- Apple Photos people — https://support.apple.com/en-ge/guide/photos/find-and-name-people-and-pets-phtad9d981ab/10.0/mac/15.0
- Resolve 18 manual, Cut page — https://ltbits.github.io/davinci-resolve-manuals/DR18/DR18-RM04-TheCutPage.pdf
- Resolve 20 Editor's Guide — https://documents.blackmagicdesign.com/ae/UserManuals/DaVinci-Resolve-20-Editors-Guide.pdf
- Blackmagic Resolve what's new — https://blackmagicdesign.com/products/davinciresolve/whatsnew
- Adobe Media Intelligence & Search panel — https://helpx.adobe.com/premiere-pro/using/media-intelligence-and-search-panel.html
- Adobe Media Intelligence FAQ — https://helpx.adobe.com/vn_vi/premiere-pro/using/media-intelligence-and-search-panel-faq.html
- Adobe Search Similar announcement — https://community.adobe.com/announcements-732/search-panel-updates-audio-search-and-search-similar-frames-311916
- Adobe Premiere AI Assistant — https://helpx.adobe.com/ca/premiere/desktop/premiere-ai-assistant/overview.html
- Adobe Auto Reframe — https://helpx.adobe.com/premiere-pro/using/auto-reframe.html
- Adobe Express resize — https://helpx.adobe.com/in/express/create-and-edit-videos/edit-videos/resize-videos.html
- Adobe Media Encoder watch folders — https://www.adobe.com/learn/media-encoder/web/create-watch-folder
- Adobe Media Encoder overview — https://helpx.adobe.com/media-encoder/desktop/encoding-quick-start-and-basics/overview-media-encoder-user-interface.html
- Frame.io commenting — https://support.frame.io/en/articles/3331455-commenting
- Frame.io version stacking — https://support.frame.io/en/articles/4431-version-stacking-and-comparison
- Frame.io V4 player & commenting — https://blog.frame.io/2024/05/28/frame-io-v4-features-player-and-commenting/
- Narrative Scenes — https://help.narrative.so/en/articles/8119179-using-scenes-in-select
- Descript Underlord help — https://help.descript.com/hc/en-us/articles/36803785502221-Underlord-beta-Your-AI-co-editor-in-Descript
- Descript changelog Feb 2026 — https://feedback.descript.com/changelog/release-roundupfebruary-10th-2026
- Filmora beat detection — https://filmora.wondershare.com/guide/beat-detection.html
- CapCut auto cut help — https://www.capcut.com/help/auto-cut-not-working
- CapCut auto reframe — https://www.capcut.com/tools/auto-reframe
- Runway Aleph prompting guide — https://help.runwayml.com/hc/en-us/articles/52150503729171-Aleph-2-0-Prompting-Guide

Reviews, write-ups, tutorials
- Larry Jordan, FCP 12 beat detection — https://larryjordan.com/articles/the-new-beat-detection-in-final-cut-pro-12-is-simple-and-effective/
- Larry Jordan, FCP 12 search unreliable — https://larryjordan.com/articles/apples-improved-search-in-final-cut-pro-12-is-very-unreliable/
- Larry Jordan, Resolve Smart Reframe — https://larryjordan.com/articles/convert-horizontal-video-to-vertical-video-in-davinci-resolve-19/
- Larry Jordan, Resolve 20 batch export — https://larryjordan.com/articles/caution-batch-export-in-davinci-resolve-20-doesnt-do-what-you-expect/
- ProVideo Coalition, Premiere AI Assistant — https://www.provideocoalition.com/premiere-ai-assistant/
- ProVideo Coalition, Jumper review — https://www.provideocoalition.com/review-ai-driven-video-search-with-jumper/
- ProVideo Coalition, hover scrub — https://www.provideocoalition.com/tool-tip-tuesday-for-adobe-premiere-hover-scrub/
- ProVideo Coalition, similar shots — https://www.provideocoalition.com/tool-tip-tuesday-for-adobe-premiere-pro-quick-finding-of-similar-shots-using-media-intelligence/
- Julieanne Kost, Compare & Survey — https://jkost.com/blog/2024/06/using-compare-and-survey-view-in-lightroom-classic.html
- Photofocus, Lightroom culling — https://photofocus.com/photography/lightroom-shortcuts-for-faster-culling/
- PPMag, Photo Mechanic power tips — https://www.ppmag.com/news/photo-mechanic-power-tips
- Fstoppers, Lightroom AI culling — https://fstoppers.com/lightroom/use-lightroom-ai-find-sharp-eyes-open-keepers-seconds-719139
- Shotkit, Narrative Select review — https://shotkit.com/narrative-select-review/
- OWC, Resolve Cut page for B-roll — https://owc.com/blog/how-to-edit-b-roll-and-stock-faster-with-davinci-resolves-cut-page
- PremiumBeat, Resolve Cut page review — https://www.premiumbeat.com/blog/davinci-resolve-16-cut-page-review/
- PremiumBeat, match frame & replace edit — https://www.premiumbeat.com/blog/matching-frame-and-replace-edit-in-premiere-pro/
- Frame.io blog, FCP browser-based editing — https://blog.frame.io/2018/11/05/fcpx-browser-editing/
- Frame.io blog, Auto Reframe tips — https://blog.frame.io/2024/10/09/insider-tips-better-results-premiere-pro-auto-reframe/
- Premierebro, Kyno — https://premierebro.squarespace.com/blog/10-ways-kyno-can-speed-up-your-pre-editing-premiere-pro-workflow
- Cutback, Selects — https://cutback.video/blog/cutback-selects-ai-rough-cut-tool-for-video-editors-using-davinci-premiere-pro-final-cut
- No Film School, Eddie AI review — https://nofilmschool.com/ai-assistant-video-editor
- No Film School, Eddie upgrades — https://nofilmschool.com/eddie-ai-upgrades
- No Film School, Media Intelligence — https://nofilmschool.com/adobe-media-intelligence
- Smashing Magazine, agentic AI UX patterns — https://smashingmagazine.com/2026/02/designing-agentic-ai-practical-ux-patterns/
- DEV Community, HITL controls — https://dev.to/pinnasys/how-to-implement-human-in-the-loop-controls-for-ai-agents-58ol
- Entrepreneur, AI edits backfire — https://www.entrepreneur.com/business-news/tech/creators-are-handing-their-video-edits-to-ai-heres-where-it-starts-to-backfire
- Videomaker, Frame.io range comments — https://www.videomaker.com/news/frame-io-v3-5-adds-range-based-comments/
- DaVinci Resolve Club, beat detection — https://davinciresolveclub.com/davinci-resolve-beat-detection/
- tryuncle, Resolve multi-timeline render — https://tryuncle.com/learn/davinci-resolve/how-to-render-multiple-timelines-at-once-in-davinci-resolve
- Cutsio, Speed Editor edit modes — https://cutsio.com/blog/davinci-resolve-speed-editor-intelligent-edit-modes
- davinciresolve21.com, Cut vs Edit page — https://www.davinciresolve21.com/blog/davinci-resolve-cut-page-vs-edit-page-differences
- TourBox, Resolve pages explained — https://www.tourboxtech.com/en/news/davinci-resolve-pages-explained.html

Community / forum (anecdotal)
- Adobe community, Survey navigation — https://community.adobe.com/questions-675/navigation-in-survey-view-968214
- Adobe community, Assisted Culling feedback — https://community.adobe.com/t5/lightroom-classic-discussions/early-access-assisted-culling-lrclassic/td-p/15519069
- Adobe community, stop hover scrub — https://community.adobe.com/t5/premiere-pro-discussions/stop-scrubbing-in-clip-bin/m-p/14894029
- Adobe community, replace edit bug — https://community.adobe.com/t5/premiere-pro-bugs/replace-with-clip-gt-from-source-monitor-match-frame-not-matching-frame/idi-p/15285281
- Adobe idea, automatic beat detection — https://community.adobe.com/t5/premiere-pro-ideas/automatic-beat-detection/idi-p/14635575
- Apple community, merged people albums — https://discussions.apple.com/docs/DOC-250002046
- Blackmagic forum, nested mixed aspect ratios — https://forum.blackmagicdesign.com/viewtopic.php?p=954942
- Blackmagic forum, render queue filename — https://forum.blackmagicdesign.com/viewtopic.php?p=1155619
- Blackmagic forum, proxy playback — https://forum.blackmagicdesign.com/viewtopic.php?p=1115402
- Dropbox forum, compare versions comments — https://community.dropbox.com/en/discussion/842839/compare-versions-show-comments
- Jumper feature request, standalone app — https://jumper.canny.io/feature-requests/p/standalone-desktop-app

**Gaps:** web searches did not surface Reddit threads directly (results were
dominated by vendor and forum pages), and no YouTube transcripts were
fetched; user-sentiment claims above rest on Adobe/Apple/Blackmagic forums,
reviewer write-ups and one trade-press piece. A follow-up pass reading
r/editors, r/davinciresolve and r/finalcutpro threads would strengthen
section 8.

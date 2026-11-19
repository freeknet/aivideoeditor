# 01: The problem, and how paid tools approach it

Research for the desktop app: a local, offline AI video editor for a small
events promoter. Every week they film a fire-spinning, flow-arts and DJ night
on a handheld Canon 7D and a DJI drone, which gives 20 to 60 minutes of dark,
dialogue-free B-roll. From that they need 15, 30 and 60 s vertical reels, a
4:5 feed cut, a 16:9 60 s cut and a recap of about 3 minutes, all cut to the
beat of licensed EDM and all following hard rules.

Compiled 2026-11-19. Sources are linked inline. Where vendors or competitors
wrote a source, that is noted, because most of the "time saved" figures in
this market come from people selling something.

> **Method note.** Reddit (r/editors, r/VideoEditing, r/premiere,
> r/videography) refuses automated fetches and search from our tooling, so it
> could not be read directly. The practitioner evidence below comes from
> Creative COW, DVinfo, ProVideo Coalition, No Film School, Larry Jordan,
> Adobe Community and the Blackmagic forum, plus review aggregators (G2,
> Capterra, Trustpilot) and the Reddit quotes they reproduce. If someone
> spends half an hour in a browser on the Reddit threads, they can check
> sections 1 and 3.

---

## 1. The primary problem: finding the moments, not cutting them

### Where the time goes

| Phase | Evidence | Share for our case |
|---|---|---|
| **Review, logging and culling** (watching everything to find usable moments) | Event raw-to-final ratios run **20:1 to 60:1** ([FitGap](https://us.fitgap.com/stack-guides/build-post-event-highlight-reels-from-raw-footage-automatically)); 20:1 per [Reduct](https://reduct.video/blog/derush-footage-faster). One Creative COW editor took about 2 h of scrubbing to get 30 min of usable selects ([Creative COW](https://creativecow.net/?p=606404)). A sports shooter says "90% of the footage is worthless" and marks bad clips on location to save time at import ([DVinfo](https://www.dvinfo.net/forum/wedding-event-videography-techniques/529022-average-edit-2.html)). A former B-roll logger calls logging grueling and seemingly endless ([Todd Boruff](https://toddboruff.substack.com/p/the-four-ss-of-b-roll)). A vendor estimates 20 to 50 h just to watch a 3-day event at 2x ([Wideframe](https://try.wideframe.com/blog/how-to-create-recap-videos-from-long-events/)). | **Largest.** 20 to 60 min of footage a night, with long stretches too dark or soft to use, so finding the 10 to 40 good seconds is most of the job. |
| **Assembling selects into a story and cutting to music** | Wedding and event editors say the highlight reel takes the most time, and that choosing the song and the opening is the hard part ([DVinfo](https://www.dvinfo.net/forum/wedding-event-videography-techniques/529022-average-edit-2.html)). Beat grids help because "reading the waveform by eye" is slow ([Larry Jordan](https://larryjordan.com/articles/the-new-beat-detection-in-final-cut-pro-12-is-simple-and-effective/)). Users report that auto beat sync drifts when clips are moved and needs a manual pass ([Opus blog summary](https://www.opus.pro/blog/best-ai-beat-sync)). | Second. Beat-sync with a drop that has to land on the hero shot is a hard constraint, and re-syncing after every change eats time. |
| **Versioning (lengths and aspect ratios)** | Recaps run 1 to 3 min, and short vertical cuts are a separate deliverable that freelancers price as add-ons: one listing quotes 6 to 10 days for a recap plus a 30 s vertical cut ([Vimeo](https://vimeo.com/blog/post/event-recap-video), [Contra/Fiverr listings](https://fiverr.com/shahbazkhanpk/edit-recap-highlights-instagram-reels-video)). The standard advice is one 9:16 master with a centred 4:5 safe zone, trimmed per platform ([Influencer Marketing Hub](https://influencermarketinghub.com/instagram-video-sizes/), [CapCut guide](https://www.capcut.com/create/match-day-social-media-series-vs-master-reel-capcut)). In Adobe's creator survey the two most-wanted AI features were **auto-repurposing (19%)** and **instant platform-specific edits (14%)** ([Adobe, n=384](https://www.adobe.com/express/learn/blog/ai-video-tools)). | Third, and it multiplies. Six deliverables a week means each choice is made six times unless the tool shares the work between them. |
| **Ingest, proxies, sync** | Multicam and audio sync is quick now that waveform sync exists ([DVinfo](https://www.dvinfo.net/forum/wedding-event-videography-techniques/529022-average-edit-2.html)). Transcoding is slow but runs unattended ([Videomaker](https://www.videomaker.com/article/c3/15094-the-video-editing-log-jam)). | Small. It runs in the background. |
| **Revisions and export** | The recurring gripes are export drift and re-checking ([Opus blog summary](https://www.opus.pro/blog/best-ai-beat-sync)) and the reliability of Frame.io V4 for review ([TrustRadius](https://www.trustradius.com/reviews/frame-io-2025-10-29-17-03-59), [Adobe Community](https://community.adobe.com/questions-729/review-with-frame-io-not-working-1419231/index3.html)). | Small for a one-person promoter, since the approver is usually themselves or one other person. |

### Totals people quote

- A 3-min event highlight takes **15 to 30 h** of skilled editing; full-day
  multicam conferences take 4 to 10 h per finished minute
  ([activids](https://activids.com/how-long-does-video-editing-take/),
  [V2 Visuals](https://v2visuals.com/event-video-editing-timeline/)).
  Turnaround is 3 to 10 business days
  ([Zealous Media](https://www.zealousmedia.co.uk/event-filming-timeline/)).
- One vendor estimates about **4 h per finished minute** of highlight content
  ([FitGap](https://us.fitgap.com/stack-guides/build-post-event-highlight-reels-from-raw-footage-automatically)),
  close to the old newspaper benchmark of 4 h of production per finished
  minute ([journalism.co.uk](https://www.journalism.co.uk/lack-of-production-experience-holding-back-web-video-for-newspapers-survey-suggests/)).
- In one creator's time audit, a 4-hour session split into **150 min of
  organising** and 90 min of creative editing
  ([Wideframe blog](https://try-wideframe-6ke.pages.dev/blog/why-youtube-creators-need-an-edit-prep-workflow)).
  That is one anecdote, but it matches the pattern above.
- An AI-assisted recap is claimed at **4 to 8 h instead of 20 to 40**, with
  most of the saving in review: AI analysis of about 2 to 3 h per 8 h of
  footage, then 30 to 60 min approving 50 to 100 candidates
  ([Wideframe](https://try.wideframe.com/blog/how-to-create-recap-videos-from-long-events/),
  vendor).
- Adobe's survey: 56% of creators save more than 30 min per video with AI, and
  10% save more than 4 h ([Adobe](https://www.adobe.com/express/learn/blog/ai-video-tools), vendor).

**Conclusion.** The main problem in dialogue-free highlight and promo work is
*review and selection*: turning a high-ratio pile of footage into a ranked,
searchable set of usable moments. Second is fitting those moments to music.
Third is producing many deliverables without redoing the work. Every
transcript-based tool skips the first problem, because our footage has no
words.

---

## 2. What paid tools do

### Comparison table

Legend: **Local** = analysis runs on the user's machine. **B-roll fit** =
useful on dialogue-free footage. Prices are the latest found and often
disagree between sources; check the vendor before quoting.

| Tool | Main target | Key AI features | Local? | B-roll fit | Pricing (2026) | Top complaints |
|---|---|---|---|---|---|---|
| **Selects** (tryselects.com, Cutback) | Pre-edit rough cuts for creators | Chat-to-edit; "match any edit" from a reference link; auto highlights; multicam; NLE handoff; MCP | No: uploads audio plus stills or proxies ([PVC NAB 2026](https://www.provideocoalition.com/nab-2026-eddie-ai-quickture-selects-the-ai-editing-assistants/)) | Low to medium. Built around talking content | Free tier; paid tiers add footage analysis ([site](https://tryselects.com)); directories say ~$20/mo ([Slashdot](https://slashdot.org/software/p/Selects/)) | One-way handoff; G2 users ask for stills as B-roll ([G2](https://www.g2.com/survey_responses/selects-review-13326041)) |
| **Eddie AI** | AI assistant editor: interviews, logging | Transcript rough cuts; B-roll logging (objects, people, lens); "Night Shift" overnight logging and assembly; ~100 MCP tools ([CineD](https://www.cined.com/eddie-ai-overhauled-pay-as-you-go-pricing-editing-from-claude-and-chatgpt-and-an-iphone-app/), [PVC](https://www.provideocoalition.com/nab-2026-eddie-ai-quickture-selects-the-ai-editing-assistants/)) | Cloud | Medium for logging, low for cutting. Early reviews said it could not cut silent footage ([Wonder Tools](https://wondertools.substack.com/p/eddie-ai-video-editing)) | Credits: $10 per 1,000, 1,000 free | Hit-or-miss; credits add up on re-prompts; 10-min cap; open prompts give guesses ([No Film School](https://nofilmschool.com/a-solo-video-professionals-quest-to-make-ai-actually-useful), [Invideo](https://invideo.io/blog/invideo-editor-vs-eddie-ai-interview-editing/)) |
| **Jumper** | Visual and speech search inside NLEs | Natural-language visual search, find-similar, faces, color; Fast/Accurate models | **Yes**, offline after activation ([PVC review](https://www.provideocoalition.com/review-ai-driven-video-search-with-jumper/)) | **High** for finding shots | $29/mo, $149/yr, $249 perpetual per NLE | Always returns results, so false positives bury good hits; heavy initial analysis (~15x realtime on an M-series MacBook); single user ([PVC compare](https://www.provideocoalition.com/visual-search-tools-compared/)) |
| **Quickture** | High-volume unscripted assembly | Transcribe; action search over time ("dolly move left to right"); rough assemblies in Premiere/Avid | Cloud proxies | Medium | $399 to $499 per seat per month ([PVC](https://www.provideocoalition.com/nab-2026-eddie-ai-quickture-selects-the-ai-editing-assistants/)) | Price |
| **Simon Says** | Transcription, captions, translation | Transcripts, assembly editor, NLE extensions; on-prem option | Cloud; old on-prem version $1,995 ([Larry Jordan](https://larryjordan.com/articles/product-review-simon-says-on-prem-ai-transcription-for-mac-or-pc/)) | None | From $15/h or ~10¢/min ([pricing](https://www.simonsaysai.com/pricing)) | Caption timing slightly off-sync ([Shade](https://shade.inc/blog/simon-says-for-post-production)) |
| **Descript** | Text-based editing of talking content | Underlord agent; filler and silence removal; Studio Sound | Cloud | None | Free 60 min; $16 to $50/mo plus AI credits ([Sonix](https://sonix.ai/resources/descript-pricing/), competitor) | Credit metering |
| **Gling** | YouTuber talking-head cleanup | Bad takes, silences, captions | Cloud | None | ~$10 to $20/mo, up to $100 ([xpay](https://www.xpay.sh/saas-pricing/gling-ai/)) | No real timeline; wants more automation ([Capterra](https://www.capterra.com/p/10003793/Gling/)) |
| **Opus Clip** | Long video to viral shorts | Virality score; reframe; captions; **ClipAnything** says it handles little or no dialogue ([help](https://help.opus.pro/docs/article/9947095-clip-anything)) | Cloud | Medium (claimed) | Free 60 min; $15 for 150 min; $29 for 300 min; 1 credit = 1 source minute ([Castmagic](https://www.castmagic.io/blog/opus-clip-pricing)) | "You didn't use the good parts"; virality scores unreliable; AI B-roll inserts static images; reframing only good on simple shots ([eesel](https://www.eesel.ai/blog/opusclip-reviews), competitor; [G2](https://www.g2.com/products/opusclip)) |
| **Vizard / Munch** | Repurposing long talks into clips | Transcript clip-picking, captions, social posting | Cloud | Low | Vizard free plus $14.50 to $39; Munch $49 to $220/mo ([GetApp](https://www.getapp.com/video-editing-software/a/vizard-1), [vidpros](https://vidpros.com/munch-ai-review/)) | Credits run out ("a single task cost me a month's credits"); Munch export quality and render times ([Capterra](https://www.capterra.com/p/10009818/Vizard/reviews/), [gappsy](https://gappsy.com/compare/munch-vs-vizard/)) |
| **CapCut** | Mobile and desktop social editing | **Auto Cut / Beat Sync** (cut on every beat or strong beats, clip length range); templates; auto captions; reframe ([help](https://www.capcut.com/help/auto-cut-in-capcut)) | Partly (desktop and mobile app; web lacks Auto Cut) | Medium: picks beats, not moments | Free with limits; Pro pricing unclear | June 2025 ToS grants ByteDance broad rights to cloud-uploaded content ([async](https://async.com/blog/capcut-terms-of-service/), competitor); watermarks; features move around by region |
| **Premiere Pro** | Pro NLE | Scene Edit Detection; Auto Reframe (2019); **Remix** retimes music to length ([Adobe blog](https://blog.adobe.com/en/publish/2022/02/08/new-in-premiere-pro-remix-music-3x-faster-speech-to-text)); **Media Intelligence** visual search, on-device ([Adobe FAQ](https://helpx.adobe.com/premiere-pro/using/media-intelligence-and-search-panel-faq.html)); Generative Extend (≤2 s video, broadened in 26.2/27.0); AI Assistant panel (27.0) ([release notes](https://helpx.adobe.com/premiere-pro/kb/fixed-issues.html)) | Search is local; generative features are cloud/credits | Medium: search finds shots, nothing assembles | ~$23/mo subscription | Visual search sometimes returns nothing ([Adobe Community](https://community.adobe.com/t5/premiere-pro-discussions/media-intelligence-visual-search-not-working/m-p/15284182/highlight/true)); thumbnail-only results, no score shown ([PVC](https://www.provideocoalition.com/visual-search-tools-compared/)); Auto Reframe locks onto the wrong target ([Elements.tv](https://elements.tv/blog/comparing-the-auto-reframe-in-davinci-resolve-and-premiere-pro/)) |
| **DaVinci Resolve Studio** | Pro NLE plus grade | Smart Reframe (Studio); Cut page source tape; **AI Detect Music Beats** (snap); **AI Music Editor** (fit to length, several versions); IntelliScript; Multicam SmartSwitch; Audio Assistant ([PVC R20](https://www.provideocoalition.com/blackmagic-introduces-davinci-resolve-20/)); **IntelliSearch** (objects, dialogue, faces) in R21 ([MacRumors](https://macrumors.com/2026/04/14/davinci-resolve-21-photo-editing-ai-search/)) | **Yes**, Neural Engine local | Medium-high: search plus beats, but no auto-assembly for montage | $295 perpetual | Smart Reframe adds jittery motion even to static and drone shots ([Elements.tv](https://elements.tv/blog/comparing-the-auto-reframe-in-davinci-resolve-and-premiere-pro/)); IntelliScript misses scenes and cuts tight ([CineD](https://cined.com/davinci-resolve-20-released-with-handful-of-ai-assisted-features)); beat detection works best on 4/4 or 3/4 |
| **Final Cut Pro 12** | Pro NLE (Mac) | Keywords, favourites and rejects, Smart Collections; **Visual Search** and **Transcript Search** (on-device); **Beat Detection** grid (no auto-cut); Magnetic Mask; Smart Conform (auto reframe to square or vertical) ([PVC](https://www.provideocoalition.com/whats-new-in-final-cut-pro-12-and-more/)); iPad **Montage Maker** auto-cuts highlights to a song ([Apple](https://support.apple.com/guide/final-cut-pro-ipad/dev00a725416/ipados)) | **Yes** | Medium-high on Mac and iPad | $299 or Creator Studio $12.99/mo | Visual search misses shots; transcripts can't be exported; beat detection can't fit clips to music ([PVC](https://www.provideocoalition.com/whats-new-in-final-cut-pro-12-and-more/)); Montage Maker called "a skilled middle schooler" ([App Store](https://apps.apple.com/us/app/final-cut-pro-create-video/id1631624924)) |
| **Kyno** | Screening, logging, subclips, ingest | Fast scrub, markers, subclips, batch transcode, NLE export | Yes | Medium (manual logging, done fast) | Perpetual seat licences with update periods ([Lesspain](https://lesspain.freshdesk.com/support/solutions/articles/12000068853-kyno-license-and-trial-activation)) | Mostly older coverage (2016 to 2019); no AI |
| **Peakto** (CYME) | Photo and video DAM with AI search | Visual and semantic search; Premiere plugin (MAX 2025) ([PVC](https://www.provideocoalition.com/peakto-becomes-a-dam/)) | Yes, Mac only | Medium | ~$59 to $65/yr (promo) | Search less reliable than Jumper; no sub-clip selection ([PVC compare](https://www.provideocoalition.com/visual-search-tools-compared/)) |
| **Frame.io** | Review and approval | Frame-accurate comments, versions, NLE panels | Cloud | n/a | Per-seat plans | V4 lost features and broke panels; billing complaints ([Trustpilot](https://ie.trustpilot.com/review/frame.io), [TrustRadius](https://www.trustradius.com/reviews/frame-io-2025-10-29-17-03-59)) |
| **Magisto / Vimeo Create** | Auto-montage to music for SMBs | Pick music; AI cuts highlights | Cloud | Medium | Vimeo integration ended 31 Dec 2024 ([Magisto help](https://help.magisto.com/hc/en-us/articles/360031272952-Vimeo-Integration-Discontinued)) | Generic output, limited music ([Creative Bloq](https://creativebloq.com/reviews/vimeo-create)) |
| **Pictory** | Script or article to stock-footage video | Sentence-to-stock matching | Cloud | None (uses stock, not your footage) | $14 to $99/mo ([Wyzowl](https://wyzowl.com/pictory-review/)) | Generic stock ([dev.to](https://dev.to/techsifted/pictory-review-2026-does-ai-video-generation-actually-deliver-1f10)) |
| **Runway (Aleph)** | Generative video-to-video edits | Add or remove objects, relight, change angles; ≤30 s clips | Cloud | n/a (VFX, not selection) | Credits; Aleph 2.0 ~28 credits/s ([techbloat](https://www.techbloat.com/runway-gen-4-5-or-aleph-2-0-best-fit-for-each-video-task.html)) | Expensive per second |
| **Filmora** | Prosumer NLE | **Auto Beat Sync** that also drops dark, overexposed and shaky clips; auto montage ([Filmora](https://filmora.wondershare.com/auto-beat-sync.html)) | Mostly local | Medium | Subscription or perpetual | No independent quality testing found |
| **LumaFusion** | iPad/iPhone pro NLE | Multitrack, keyframes; little AI | Yes | Low (manual) | $29.99 one-time plus add-ons ([TechRadar](https://www.techradar.com/pro/software-services/lumafusion-review)) | Conservative; no AI assembly ([geekchamp](https://geekchamp.com/?p=249626)) |
| **Captions (Mirage)** | Mobile talking-head shorts | AI Edit, captions, dubbing, eye contact | Cloud | Low | Pro ~$9.99/mo ([App Store](https://apps.apple.com/us/app/captions-ai-short-form-video/id1541407007)) | Cluttered; weak on layered commercial work ([eesel](https://www.eesel.ai/blog/captions-ai)) |
| **Autopod** | Premiere podcast multicam | Auto multicam switching, social clips, reframe, batch export | Local plugin | None | $29/mo ([SourceForge](https://www.sourceforge.net/software/product/AutoPod)) | Talk-only |
| **FireCut** | Premiere/Resolve cleanup | Silence and filler removal, captions, zooms, chapters | Plugin plus cloud AI | Low | From ~$10/mo yearly ([firecut](https://firecut.ai/pricing/all/)) | Talk-only |
| **AutoCut** | Premiere/Resolve cleanup | Silence removal, captions, zooms, profanity, B-roll; 10 tools | Plugin | Low | $9.90 or $19.80/mo ([Capterra](https://www.capterra.com/p/10036511/AutoCut/)) | Subscription cost; time savings unproven ([Nemo](https://www.nemovideo.com/blog/video-tool-alternatives/autocut-pricing)) |

### How the market splits

1. **Transcript-first assistants** (Descript, Gling, Simon Says, Autopod,
   FireCut, AutoCut, Eddie, Selects, Vizard, Munch, Captions). These are the
   bulk of the market and the most mature, but they are built on speech. With
   no dialogue they have almost nothing to work with: Eddie's early version
   explicitly "won't work for silent footage"
   ([Wonder Tools](https://wondertools.substack.com/p/eddie-ai-video-editing)).
2. **Visual search** (Jumper, Premiere Media Intelligence, Resolve
   IntelliSearch, FCP Visual Search, Peakto, Quickture). These solve *finding*
   on B-roll and run locally, but they return candidates ranked without a
   score. They are bad at shot-technique words ("wide", "timelapse",
   "gimbal") and none of them were tested on dark footage
   ([PVC compare](https://www.provideocoalition.com/visual-search-tools-compared/)).
   None of them assemble a cut.
3. **Music-fit tools** (Resolve beat detector and AI Music Editor, Premiere
   Remix, FCP Beat Detection, CapCut Auto Cut, Filmora Auto Beat Sync,
   Montage Maker). Grids and music retiming are good and trusted. Auto cutting
   on the beat picks *when* to cut but not *what* to show: CapCut cuts on
   beats with whatever clips you give it.
4. **Reframe** (Premiere Auto Reframe, Resolve Smart Reframe, FCP Smart
   Conform, Opus, Autopod). They work on simple subjects. They lose the
   subject on drone and fast-motion shots and add jitter
   ([Elements.tv](https://elements.tv/blog/comparing-the-auto-reframe-in-davinci-resolve-and-premiere-pro/)).
5. **Generative** (Generative Extend, Runway Aleph). These fix gaps and shots
   but don't address selection. They are cloud-based and charged per second.

---

## 3. What actually saves time for montage and promo work, and what doesn't

**Reported as real time savers:**

- **Local visual and semantic search over a whole night's footage.** The PVC
  comparison found Jumper "most consistent". The Eddie B-roll logging user
  stopped scrubbing bins and searches by keyword instead
  ([No Film School, sponsored](https://nofilmschool.com/how-ai-logging-transformed-my-post-production-workflow-for-clients)).
  It replaces the largest phase.
- **Auto-logging to text** (descriptions, objects, faces, camera move) that
  can be read and filtered, i.e. Eddie's logging and Kyno-style screening.
- **Beat grids with snapping**: "simple and effective"
  ([Larry Jordan](https://larryjordan.com/articles/the-new-beat-detection-in-final-cut-pro-12-is-simple-and-effective/)).
  Resolve's version is trusted for 4/4 music, which EDM almost always is.
- **Music retiming to an exact length** (Remix, Resolve AI Music Editor with
  saved versions). This makes 15, 30 and 60 s cuts of the same track cheap.
- **One master with derived versions** (FCP Smart Conform, a 9:16 master with
  a 4:5 safe zone). Repurposing is the most-requested feature
  ([Adobe](https://www.adobe.com/express/learn/blog/ai-video-tools)).
- **Rejecting technically bad clips automatically.** Filmora explicitly drops
  too-dark, overexposed and shaky segments. For night footage this is the
  first and cheapest cull.
- **Scene and cut detection** on long continuous clips, so each segment is a
  candidate with its own score.
- **Overnight unattended processing** (Eddie Night Shift). Analysis takes
  hours (Jumper ~15x realtime), so batch it while the user sleeps.

**Reported as gimmicks or unreliable:**

- **Virality and "best moment" scores** with no reasons: users find low-scored
  clips outperform the "winners"
  ([eesel on Opus](https://www.eesel.ai/blog/opusclip-reviews)).
- **One-prompt "make me a video"** produces guesses
  ([No Film School, Eddie's own staff](https://nofilmschool.com/a-solo-video-professionals-quest-to-make-ai-actually-useful)).
- **AI B-roll insertion** (static images, irrelevant keyword matches) in Opus
  and Vizard.
- **Auto reframe on complex motion**: jittery on static shots, loses drone
  subjects.
- **Fully automatic montage apps** (Magisto, Montage Maker, CapCut templates).
  Output is generic and the rules are ones the user can't change.
- **Generative extend and replace** are useful as patches, irrelevant to the
  main problem, and cost credits.

---

## 4. Gaps: what nobody does well for our user

1. **Dark, light-source-dominated footage.** Every visual model is trained on
   well-lit content. Research shows low light degrades detection in ways
   training can't fully fix, and that strong light sources become false
   positives ([NOD paper](https://arxiv.org/abs/2110.10364v1),
   [night background subtraction](https://www.ncbi.nlm.nih.gov/pmc/articles/PMC8205302/)).
   None of the search comparisons tested dark footage. Fire, LED poi and
   moving light trails *are* the subject here, not noise. Nobody measures
   "fire present", "light trail", "how much of the frame is lit by the
   performer" or "flame peak".
2. **Dialogue-free assembly.** Every assistant that assembles works from
   transcripts. Visual-search tools stop at finding. Nobody goes from visual
   signals to a structured, beat-aligned cut with reasons.
3. **Rules about variety and fit.** No product enforces "no repeated shot or
   performer", "drop lands on the hero fire shot", "best moments only" or
   "end card required" as checks. CapCut and Montage Maker are black boxes,
   and NLEs leave it to the editor. Opus has a score but no rules.
4. **Weekly recurring, branded deliverables.** Tools treat each video as a
   one-off. Nobody stores "our six weekly outputs, our end card, our safe
   zones, our track library" as a recipe that is re-run on new footage, or
   remembers which shots and performers were used in earlier weeks.
5. **Offline, no credits, private.** Local analysis exists only in pro NLEs
   (Resolve, FCP, Premiere search) and Jumper. All assemblers (Eddie, Selects,
   Quickture, Opus) are cloud and credit-metered. CapCut's ToS worries
   professionals. A small promoter pays ~$295 to $600/yr in subscriptions and
   still has no assembler.
6. **Explainable selection.** Scores come with no reasons. Our users need to
   see *why* a shot was picked (the storyboard `note`), so they can correct
   the rule, not just the cut.
7. **Multi-length music fit plus drop alignment.** Remix and Resolve fit a
   track to a length, but nothing keeps the drop on a chosen shot across the
   15, 30 and 60 s and 3-min versions.
8. **Performer identity without faces.** Fire spinners are often backlit,
   masked or silhouetted. Face-based grouping (Resolve IntelliSearch faces,
   Jumper faces) breaks down. Costume colour, prop type and silhouette are
   more useful.
9. **Drone and handheld mix.** Reframe tools fail on drone motion. Nobody
   plans crops from known camera motion (our pan, zoom and shake analysis).

---

## 5. Top 15 features to build (ranked)

Ranked by time saved for this user multiplied by how badly competitors serve
it.

| # | Feature | Inspired by | Why it matters for our user |
|---|---|---|---|
| 1 | **Night-tuned technical cull**: auto-reject too dark, crushed, soft, shaky and whip-pan segments, with thresholds tuned for fire-lit scenes (exposure measured on the lit region, not the frame average) | Filmora Auto Beat Sync's bad-clip removal; Kyno screening | The largest time sink is review. Most of 20 to 60 min of night footage is unusable, and frame-average exposure would wrongly reject good fire shots. |
| 2 | **Fire and light "moment" detector**: flame peaks, light-trail density, flares, burst onset, ranked into candidate moments with reasons | Opus ClipAnything "key moments"; Wideframe highlight list | The "best moments only" rule. No competitor sees fire or LED as signal. |
| 3 | **Readable shot log plus local semantic search** (text query over VLM captions and measurements, with scores shown and a cut-off) | Jumper, Premiere Media Intelligence, Resolve IntelliSearch, FCP Visual Search | Finding shots is the job. Show the score and cap results to avoid Jumper's false-positive flood. |
| 4 | **Rule-checked plan with a `note` per shot** (no repeated shot or performer, minimum gap, coverage of each act, hero on drop) and `check` before render | Gap: nobody has it; Resolve IntelliScript's take stacking for alternates | This is our differentiator: rules as checks, with explanations instead of black-box scores. |
| 5 | **Drop-locked beat grid**: beats, bars and drop detection; cuts snap to bars; the hero fire shot is pinned to the drop | Resolve AI Detect Music Beats, FCP Beat Detection, CapCut Auto Cut sensitivity | Beat-sync is the format's grammar. EDM is 4/4, where the grids are most reliable. |
| 6 | **Music fit to length** (15, 30 and 60 s and ~3 min edits of one licensed track, keeping intro, build and drop) | Premiere Remix, Resolve AI Music Editor (multiple saved versions) | Six deliverables share one track, and each needs a clean musical edit that still hits the drop. |
| 7 | **Weekly recipe / deliverable preset**: one command produces all six outputs (lengths, aspect ratios, end card, safe zones, loudness) from a single plan | FCP Smart Conform; CapCut "one master, trimmed exports"; Adobe survey's top request | Versioning multiplies everything else. A recurring weekly job should be one click. |
| 8 | **Motion-aware reframing** for 9:16 and 4:5 that uses our pan, zoom and shake data and fire-region tracking, with smooth paths and no motion on static shots | Premiere Auto Reframe, Resolve Smart Reframe (and their failures) | Competitors jitter on static shots and lose drone subjects. Our camera-motion analysis is the fix. |
| 9 | **Cross-week memory**: track which shots, performers and moves were used in earlier weeks' edits and penalise reuse | Gap | Followers notice repeats. No tool remembers across projects. |
| 10 | **Performer grouping without faces** (costume colour, prop type, silhouette, with human rename and `clip@from-to`) | Resolve and Jumper face search (which fail on backlit performers) | Needed for the "no repeated performer" rule and fair coverage of acts. |
| 11 | **Branded end card and title templates** with platform safe zones (9:16 with a 4:5 centre) | CapCut templates; Instagram safe-zone guidance | A hard rule; it should never need hand placement. |
| 12 | **Overnight batch analysis** with progress and resume | Eddie Night Shift; Jumper's offline cache | Analysis is slow (Jumper ~15x realtime). Running it unattended after the event removes the wait. |
| 13 | **Fast human review loop**: approve or reject candidate moments as text and thumbnails in the GUI, then re-plan | Wideframe's 30 to 60 min candidate review; FCP favourites and rejects; Kyno | The human makes the final call cheaply, with no re-scrubbing. |
| 14 | **NLE handoff** (OTIO/XML into Resolve, Premiere, FCP) for finishing | Selects, Eddie, Quickture handoffs | Lets the promoter, or a hired editor, polish without redoing selection. |
| 15 | **Drone/handheld pairing and act structure** (establishing drone shot, then handheld close-ups; arc of intro, build, drop, outro) | Montage Maker / Magisto story templates; Wideframe chronological, thematic and narrative structures | Turns a pile of good shots into a promo that reads as an event, and the user can edit it. |

**Deliberately not prioritised:** virality scores, generative extend and
replace, AI stock B-roll, prompt-only "make me a video", and transcript
features (no dialogue). Users report these as unreliable or irrelevant for
this footage.

---

## Source index (additional, not linked above)

- Reduct, video logging: https://reduct.video/blog/video-logging-software
- Creative COW, 170 h project selects: https://creativecow.net/forums/thread/170-hrs-of-hdv-footage-2/
- Eddie AI logging v2: https://www.cined.com/eddie-ai-logging-v2-announced/
- No Film School, Eddie review: https://nofilmschool.com/ai-assistant-video-editor
- Resolve 21 review (Videomaker): https://www.videomaker.com/?p=73152360
- FCP 12 overview (Newsshooter): https://www.newsshooter.com/2026/01/28/final-cut-pro-12-0/
- Generative Extend review: https://www.videomaker.com/reviews/software/adobe-generative-extend-tool-review-a-tool-poised-to-change-how-we-edit-forever/
- Opus ClipAnything (Fast Company): https://fastcompany.com/91178960/opusclip-ai-videos-social-media
- Engadget on Remix: https://www.engadget.com/adobe-premiere-pro-remix-music-re-time-sync-213055147.html
- Engadget on IntelliScript: https://www.engadget.com/apps/davinci-resolve-20s-latest-ai-feature-can-create-an-entire-timeline-based-on-a-script-120009351.html

---
name: beat-sync-reel
description: Build or revise a promo cut to the drop, beat-synced, with the JaJa rules.
triggers: [promo, reel, cut to the drop, beat sync, beat-sync, new cut, build a cut, make a cut, 15s, 30s, 60s, teaser, the drop]
---

# Beat-sync reel

Use this when the person wants a new promo cut, or wants an existing one
re-timed or rebuilt around the music.

You did not listen to the track and you did not watch the footage: timing
comes from `music_map`, picture from the shot log. Say so once in your
answer ("timing is from the measurements").

## Steps

1. `night_overview`, then `music_map`. Choose the beat interpretation, the
   song range and the drop yourself (strongest drop candidate unless the
   person named one). A 15 s cut is about 8 bars at 128 BPM, a 30 s cut 16.
2. Decide the shape before any tool edits: hook (1-2 shots), breakdown,
   build (shots shorten toward the drop), the **hero fire shot exactly on
   the drop bar**, phrase-paced shots after it, finale, end card. Write this
   plan in the chat as a short list: bar range, then one line per slot with
   its beats and why.
3. Find footage with `search_shots` (for example `fire>0.3 q>0.7 -dark`,
   `moment:performing framing:close`, `person:<name>`), and confirm each
   candidate with `segment_details`. Prefer `pick`, high q, action centred
   (vertical crops are a third of the frame).
4. For a **new** cut: `plan_draft` with a new name (`reel-15`, `reel-30`),
   never over the plan on screen unless the person named it. Committing a
   new plan is allowed; then revise it.
5. For a **revision**: `plan_get` (note its version), then `plan_edit`
   with the ops, staged. Every `replace_shot` reason names the facts
   ("fire 9% · q 0.91 · performing close").
6. Read the check in the result. Fix what it reports with the smallest
   change (see the fix-check skill). The tool never moves cuts on its own;
   you do.
7. Answer: what you built, the drop bar, the check result, and the shots as
   `ave://shot/<plan>/<n>` links.

## The JaJa rules

- Structure (`rules.structure`): Flow Night cuts are `sections`, one run
  per performer (`section_bars` long, changing on whole pairs of bars),
  shots in recording order, jump-cutting forward past the slow stretches
  (at least `min_skip` seconds within one take). The breakdown is one calm
  performer, the drop opens the strongest fire performer's section, the
  rest follow in recording order. Nobody comes back after their section.
  `mix` (the General default) interleaves everyone instead.
- No repeated shots (with sections: never the same frames twice); with
  mix, no repeated performers within the person gap.
- Best moments only: nothing dark, soft, shaky, whip or blown; no
  transitions or idle moments; no blocked views.
- The drop lands on the hero fire shot.
- No slow motion, no digital zoom.
- Every shot has a note saying why it is there.

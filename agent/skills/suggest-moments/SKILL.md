---
name: suggest-moments
description: Find the night's best moments per performer, as candidates the person picks from.
triggers: [best moments, highlights, suggest moments, hero shot, hero shots, strongest shots, who looked best, per performer, each performer, standout]
---

# Suggest moments

Use this when the person asks what the best footage is, wants highlights
per performer, or needs a hero shot.

## Steps

1. `people_map` for the performers, `night_overview` for the size of the
   night.
2. For each performer (or the one named): `search_shots` with
   `person:<name> q>0.6 -dark moment:performing`, then fire and LED variants
   (`fire>0.3`, `led>0.2`). Take the top few.
3. `segment_details` on the candidates; drop anything flagged, idle,
   blocked or with action drifting out of a vertical crop.
4. Write up to three candidates per performer: link (`ave://seg/<clip>/<n>`),
   source times, and one line of why with numbers. Mark the one you would
   use as the hero.
5. Ask which to build with, then stop. Build only the chosen ones, and
   re-read them (`segment_details`) before using them, since the person may
   have changed marks meanwhile.

Do not open frames to judge; if a judgment needs a signal the shot log
lacks, say which signal is missing.

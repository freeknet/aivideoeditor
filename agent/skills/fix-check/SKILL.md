---
name: fix-check
description: Resolve the plan's rule-check problems with the smallest possible changes.
triggers: [fix the check, fix check, check problems, rule check, problems, fix everything, failing check, check fails, make it pass]
---

# Fix the check

Use this when the person asks to fix what the check flags.

## Steps

1. `plan_get` on the plan. Note its version and list the problems.
2. For each problem, pick the smallest op that fixes it:
   - repeated shot or performer: `alternatives` for that slot, then one
     `replace_shot`;
   - flagged segment (dark, soft, blown...): `alternatives` or a `set_in`
     to a clean part of the same segment;
   - drop not on the hero fire shot: `move_shot` the strongest fire shot to
     the drop slot, or `replace_shot` there;
   - beat total off: balance `set_beats` with another `set_beats`.
3. Touch nothing the check did not flag, and never shots the person marked
   keep (`+`).
4. One `plan_edit`, staged, with all the ops. Read the predicted check: it
   must have fewer problems and no new ones. If an op adds a problem, try
   the next alternative before staging again.
5. Answer: one line per fix with the shot link (`ave://shot/<plan>/<n>`),
   and the predicted check. If something cannot be fixed without breaking
   another rule, say which and why.

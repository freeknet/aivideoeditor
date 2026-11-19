// ============================================================================
// Module:       gui::assistant::ops
// Description:  The edit operations a proposal is made of, applied to a copy
//               of a plan with scope and beat-total checks, and the
//               before/after of every operation for the diff card.
//
// Dependencies: serde; crate::{plan, index, people, beats}
// ============================================================================

//! Edit operations.
//!
//! The assistant never writes a plan. It proposes a list of [`EditOp`]s;
//! [`apply`] runs them on a copy, in order, and says what each one changed
//! (the diff card) or why it cannot (sent back to the model as a tool error
//! so it can correct itself).
//!
//! Indices are 1-based shot numbers, as the person says them ("shot 5"),
//! and each operation sees the plan as the operations before it left it.
//!
//! The music is the clock: a plan's beat total is fixed by its bars, so
//! moving a shot moves its content between slots that keep their lengths,
//! removing a shot gives its beats to its neighbour, and `set_beats` must be
//! balanced by another `set_beats` in the same proposal.

use std::collections::BTreeSet;

use crate::beats::MusicMap;
use crate::index::Index;
use crate::people::People;
use crate::plan::{Plan, Problem, Shot};

/// One change to a plan.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum EditOp {
    /// Put different footage in slot `index` (its length stays).
    ReplaceShot {
        /// Slot, 1-based.
        index: usize,
        /// Source clip id.
        clip: String,
        /// In-point, seconds.
        #[serde(rename = "in")]
        t_in: f32,
        /// Crop centre; the segment's action centre when omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cx: Option<f32>,
        /// Why: becomes the shot's note.
        #[serde(default)]
        reason: String,
    },
    /// Move a shot's in-point.
    SetIn {
        /// Slot, 1-based.
        index: usize,
        /// In-point, seconds.
        #[serde(rename = "in")]
        t_in: f32,
    },
    /// Change a slot's length (balance it with another `set_beats`).
    SetBeats {
        /// Slot, 1-based.
        index: usize,
        /// Length in beats.
        beats: usize,
    },
    /// Move a shot's content to another slot; slot lengths stay.
    MoveShot {
        /// From slot, 1-based.
        from: usize,
        /// To slot, 1-based.
        to: usize,
    },
    /// Drop a shot; its beats go to the shot before it (after, for the first).
    RemoveShot {
        /// Slot, 1-based.
        index: usize,
    },
    /// Move the crop centre.
    SetCx {
        /// Slot, 1-based.
        index: usize,
        /// Crop centre, 0..1 of the source width.
        cx: f32,
    },
    /// Rewrite the note.
    SetNote {
        /// Slot, 1-based.
        index: usize,
        /// The note.
        note: String,
    },
}

impl EditOp {
    /// A few words for the diff card: `replace #5`.
    pub(crate) fn label(&self) -> String {
        match self {
            Self::ReplaceShot { index, .. } => format!("replace #{index}"),
            Self::SetIn { index, .. } => format!("trim #{index}"),
            Self::SetBeats { index, beats } => format!("#{index} → {beats} beats"),
            Self::MoveShot { from, to } => format!("move #{from} → #{to}"),
            Self::RemoveShot { index } => format!("remove #{index}"),
            Self::SetCx { index, .. } => format!("reframe #{index}"),
            Self::SetNote { index, .. } => format!("note #{index}"),
        }
    }

    /// The reason the model gave, when the op carries one.
    pub(crate) fn reason(&self) -> &str {
        match self {
            Self::ReplaceShot { reason, .. } => reason,
            Self::SetNote { note, .. } => note,
            Self::SetIn { .. }
            | Self::SetBeats { .. }
            | Self::MoveShot { .. }
            | Self::RemoveShot { .. }
            | Self::SetCx { .. } => "",
        }
    }
}

/// Which shots a request may touch.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) enum Scope {
    /// Any shot.
    #[default]
    Whole,
    /// Only these slots (1-based, of the plan the proposal starts from).
    Shots(BTreeSet<usize>),
}

impl Scope {
    /// Whether original slot `slot` may change.
    fn allows(&self, slot: usize) -> bool {
        match self {
            Self::Whole => true,
            Self::Shots(set) => set.contains(&slot),
        }
    }

    /// The scope as the model reads it.
    pub(crate) fn describe(&self) -> String {
        match self {
            Self::Whole => "the whole plan".to_owned(),
            Self::Shots(set) => format!(
                "only shots {} (leave every other shot exactly as it is)",
                set.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

/// What one operation did, for the diff card.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Effect {
    /// The op's label.
    pub(crate) label: String,
    /// The slot before (1-based) and the shot there, if any.
    pub(crate) before: Option<(usize, Shot)>,
    /// The slot after and the shot there, if any.
    pub(crate) after: Option<(usize, Shot)>,
    /// The reason given.
    pub(crate) reason: String,
}

/// Why a proposal cannot be applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OpError {
    /// The failing op, 0-based (`None` for the whole proposal).
    pub(crate) op: Option<usize>,
    /// What is wrong.
    pub(crate) message: String,
}

impl std::fmt::Display for OpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.op {
            Some(k) => write!(f, "op {}: {}", k + 1, self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// What applying needs to know about the night.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Night<'a> {
    /// The shot log, to check clips and fill `seg` / crop centres.
    pub(crate) index: Option<&'a Index>,
    /// The people map, to fill `who`.
    pub(crate) people: &'a People,
}

/// Applies `ops` to a copy of `base`, in order, within `scope`.
pub(crate) fn apply(
    base: &Plan,
    ops: &[EditOp],
    scope: &Scope,
    night: Night<'_>,
) -> Result<(Plan, Vec<Effect>), OpError> {
    let mut plan = base.clone();
    // The original slot of each current shot, for the scope check.
    let mut origin: Vec<usize> = (1..=plan.shots.len()).collect();
    let mut effects = Vec::new();
    let crop = plan.aspect.crop_frac();
    for (k, op) in ops.iter().enumerate() {
        let err = |message: String| OpError {
            op: Some(k),
            message,
        };
        let n = plan.shots.len();
        let slot = |i: usize| -> Result<usize, OpError> {
            if i == 0 || i > n {
                Err(err(format!("there is no shot {i} (the plan has {n})")))
            } else {
                Ok(i - 1)
            }
        };
        let in_scope = |i: usize| -> Result<(), OpError> {
            let o = origin[i];
            if scope.allows(o) {
                Ok(())
            } else {
                Err(err(format!(
                    "shot {} is outside the scope ({}); stay in scope or ask first",
                    i + 1,
                    scope.describe()
                )))
            }
        };
        let label = op.label();
        let reason = op.reason().to_owned();
        match op {
            EditOp::ReplaceShot {
                index,
                clip,
                t_in,
                cx,
                reason,
            } => {
                let i = slot(*index)?;
                in_scope(i)?;
                let before = plan.shots[i].clone();
                let seg = night.index.and_then(|ix| {
                    ix.segments
                        .iter()
                        .find(|g| g.clip == *clip && g.seg.start <= *t_in && *t_in < g.seg.end)
                });
                if let Some(ix) = night.index {
                    let Some(c) = ix.clip(clip) else {
                        return Err(err(format!("clip {clip} is not in the shot log")));
                    };
                    if *t_in < 0.0 || f64::from(*t_in) >= c.probe.duration {
                        return Err(err(format!(
                            "{clip} is {:.1}s long; {t_in:.2} is outside it",
                            c.probe.duration
                        )));
                    }
                }
                let half = crop / 2.0;
                let centre = cx
                    .or_else(|| seg.map(|g| g.seg.cx))
                    .unwrap_or(0.5)
                    .clamp(half.min(0.5), (1.0 - half).max(0.5));
                let shot = &mut plan.shots[i];
                shot.clip.clone_from(clip);
                shot.t_in = *t_in;
                shot.cx = centre;
                shot.who = night.people.name_at(clip, *t_in);
                shot.seg = seg.map(|g| g.id.clone()).unwrap_or_default();
                if !reason.is_empty() {
                    shot.note.clone_from(reason);
                }
                effects.push(Effect {
                    label,
                    before: Some((i + 1, before)),
                    after: Some((i + 1, plan.shots[i].clone())),
                    reason: reason.clone(),
                });
            }
            EditOp::SetIn { index, t_in } => {
                let i = slot(*index)?;
                in_scope(i)?;
                if *t_in < 0.0 {
                    return Err(err("an in-point cannot be negative".to_owned()));
                }
                let before = plan.shots[i].clone();
                plan.shots[i].t_in = *t_in;
                effects.push(Effect {
                    label,
                    before: Some((i + 1, before)),
                    after: Some((i + 1, plan.shots[i].clone())),
                    reason,
                });
            }
            EditOp::SetBeats { index, beats } => {
                let i = slot(*index)?;
                in_scope(i)?;
                if *beats == 0 {
                    return Err(err("a shot needs at least one beat".to_owned()));
                }
                let before = plan.shots[i].clone();
                plan.shots[i].beats = *beats;
                effects.push(Effect {
                    label,
                    before: Some((i + 1, before)),
                    after: Some((i + 1, plan.shots[i].clone())),
                    reason,
                });
            }
            EditOp::MoveShot { from, to } => {
                let a = slot(*from)?;
                let b = slot(*to)?;
                in_scope(a)?;
                in_scope(b)?;
                let before = plan.shots[a].clone();
                let lengths: Vec<usize> = plan.shots.iter().map(|s| s.beats).collect();
                let shot = plan.shots.remove(a);
                plan.shots.insert(b, shot);
                let o = origin.remove(a);
                origin.insert(b, o);
                for (s, len) in plan.shots.iter_mut().zip(lengths) {
                    s.beats = len;
                }
                effects.push(Effect {
                    label,
                    before: Some((a + 1, before)),
                    after: Some((b + 1, plan.shots[b].clone())),
                    reason,
                });
            }
            EditOp::RemoveShot { index } => {
                let i = slot(*index)?;
                in_scope(i)?;
                if n == 1 {
                    return Err(err("cannot remove the only shot".to_owned()));
                }
                let removed = plan.shots.remove(i);
                origin.remove(i);
                let heir = if i == 0 { 0 } else { i - 1 };
                plan.shots[heir].beats += removed.beats;
                effects.push(Effect {
                    label,
                    before: Some((i + 1, removed)),
                    after: Some((heir + 1, plan.shots[heir].clone())),
                    reason,
                });
            }
            EditOp::SetCx { index, cx } => {
                let i = slot(*index)?;
                in_scope(i)?;
                let before = plan.shots[i].clone();
                plan.shots[i].cx = cx.clamp(0.0, 1.0);
                effects.push(Effect {
                    label,
                    before: Some((i + 1, before)),
                    after: Some((i + 1, plan.shots[i].clone())),
                    reason,
                });
            }
            EditOp::SetNote { index, note } => {
                let i = slot(*index)?;
                in_scope(i)?;
                let before = plan.shots[i].clone();
                plan.shots[i].note.clone_from(note);
                effects.push(Effect {
                    label,
                    before: Some((i + 1, before)),
                    after: Some((i + 1, plan.shots[i].clone())),
                    reason,
                });
            }
        }
    }
    let total = |p: &Plan| p.shots.iter().map(|s| s.beats).sum::<usize>();
    let (was, now) = (total(base), total(&plan));
    if was != now {
        return Err(OpError {
            op: None,
            message: format!(
                "the shots now fill {now} beats but the music needs {was}: balance every \
                 set_beats with another (the music is the clock)"
            ),
        });
    }
    Ok((plan, effects))
}

/// The engine's rule check, when the night has what it needs.
pub(crate) fn check(
    plan: &Plan,
    index: Option<&Index>,
    music: Option<&MusicMap>,
    people: &People,
) -> Option<Vec<Problem>> {
    Some(crate::plan::check(plan, index?, music?, people))
}

/// Problems in `after` that `before` did not have, and the number fixed.
pub(crate) fn compare(before: &[Problem], after: &[Problem]) -> (Vec<Problem>, usize) {
    let new: Vec<Problem> = after
        .iter()
        .filter(|p| !before.iter().any(|q| q.message == p.message))
        .cloned()
        .collect();
    let fixed = before
        .iter()
        .filter(|p| !after.iter().any(|q| q.message == p.message))
        .count();
    (new, fixed)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeSet;

    use super::{EditOp, Night, Scope};
    use crate::people::People;
    use crate::plan::{Aspect, Plan, Rules, Shot};

    /// A plan of `n` four-beat shots of clips `A`, `B`, ... at 10 s.
    pub(crate) fn plan(n: usize) -> Plan {
        Plan {
            name: "t".into(),
            aspect: Aspect::Wide,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 0,
            card_bar: n,
            end_bar: n + 1,
            end_card: String::new(),
            rules: Rules::default(),
            shots: (0..n)
                .map(|k| Shot {
                    clip: char::from(b'A' + k as u8).to_string(),
                    t_in: 10.0,
                    beats: 4,
                    cx: 0.5,
                    who: String::new(),
                    seg: String::new(),
                    note: format!("n{}", k + 1),
                })
                .collect(),
            finish: Default::default(),
        }
    }

    fn night(people: &People) -> Night<'_> {
        Night {
            index: None,
            people,
        }
    }

    #[test]
    fn replace_keeps_the_slot_length_and_takes_the_reason() -> anyhow::Result<()> {
        let people = People::default();
        let ops = [EditOp::ReplaceShot {
            index: 2,
            clip: "Z".into(),
            t_in: 3.0,
            cx: None,
            reason: "more fire".into(),
        }];
        let (after, effects) = super::apply(&plan(3), &ops, &Scope::Whole, night(&people))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        assert_eq!(after.shots[1].clip, "Z", "replaced");
        assert_eq!(after.shots[1].beats, 4, "length kept");
        assert_eq!(after.shots[1].note, "more fire", "reason becomes note");
        assert_eq!(effects.len(), 1, "one effect");
        assert_eq!(
            effects[0].before.as_ref().map(|(_, s)| s.clip.as_str()),
            Some("B"),
            "before shown"
        );
        Ok(())
    }

    #[test]
    fn moves_keep_slot_lengths_and_removes_keep_the_total() -> anyhow::Result<()> {
        let people = People::default();
        let mut base = plan(4);
        base.shots[0].beats = 8;
        base.shots[1].beats = 2;
        base.shots[2].beats = 2;
        let ops = [EditOp::MoveShot { from: 1, to: 3 }];
        let (after, _) = super::apply(&base, &ops, &Scope::Whole, night(&people))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let order: Vec<&str> = after.shots.iter().map(|s| s.clip.as_str()).collect();
        assert_eq!(order, ["B", "C", "A", "D"], "A moved to slot 3");
        let lens: Vec<usize> = after.shots.iter().map(|s| s.beats).collect();
        assert_eq!(lens, [8, 2, 2, 4], "slots keep their lengths");

        let ops = [EditOp::RemoveShot { index: 2 }];
        let (after, _) = super::apply(&base, &ops, &Scope::Whole, night(&people))
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        assert_eq!(after.shots.len(), 3, "one fewer");
        assert_eq!(after.shots[0].beats, 10, "beats go to the shot before");
        Ok(())
    }

    #[test]
    fn unbalanced_beats_scope_and_bad_slots_are_refused() -> anyhow::Result<()> {
        let people = People::default();
        let base = plan(4);
        let unbalanced = [EditOp::SetBeats { index: 1, beats: 6 }];
        let e = super::apply(&base, &unbalanced, &Scope::Whole, night(&people));
        assert!(
            e.as_ref()
                .is_err_and(|e| e.op.is_none() && e.message.contains("beats")),
            "total must hold: {e:?}"
        );
        let balanced = [
            EditOp::SetBeats { index: 1, beats: 6 },
            EditOp::SetBeats { index: 2, beats: 2 },
        ];
        assert!(
            super::apply(&base, &balanced, &Scope::Whole, night(&people)).is_ok(),
            "balanced is fine"
        );
        let scope = Scope::Shots(BTreeSet::from([2]));
        let outside = [EditOp::SetCx { index: 3, cx: 0.4 }];
        let e = super::apply(&base, &outside, &scope, night(&people));
        assert!(
            e.as_ref()
                .is_err_and(|e| e.message.contains("outside the scope")),
            "scope enforced: {e:?}"
        );
        // Scope follows the shot, not the slot number: after moving the
        // original shot 2 to slot 1, slot 1 is in scope and slot 2 is not.
        let shifted = [
            EditOp::MoveShot { from: 2, to: 1 },
            EditOp::SetNote {
                index: 1,
                note: "x".into(),
            },
        ];
        let e = super::apply(&base, &shifted, &scope, night(&people));
        assert!(
            e.as_ref()
                .is_err_and(|e| e.message.contains("shot 1 is outside")),
            "moving needs both slots in scope: {e:?}"
        );
        let renote = [EditOp::SetNote {
            index: 2,
            note: "x".into(),
        }];
        assert!(
            super::apply(&base, &renote, &scope, night(&people)).is_ok(),
            "in scope"
        );
        let missing = [EditOp::SetNote {
            index: 9,
            note: "x".into(),
        }];
        let e = super::apply(&base, &missing, &Scope::Whole, night(&people));
        assert!(
            e.as_ref()
                .is_err_and(|e| e.op == Some(0) && e.message.contains("no shot 9")),
            "bad slot: {e:?}"
        );
        Ok(())
    }

    #[test]
    fn ops_parse_from_the_tool_json() -> anyhow::Result<()> {
        let ops: Vec<EditOp> = serde_json::from_value(serde_json::json!([
            {"op": "replace_shot", "index": 5, "clip": "MVI_1", "in": 2.5, "reason": "fire"},
            {"op": "set_beats", "index": 2, "beats": 8},
            {"op": "move_shot", "from": 1, "to": 3}
        ]))?;
        assert_eq!(ops.len(), 3, "three ops");
        assert_eq!(ops[0].label(), "replace #5", "label");
        Ok(())
    }

    #[test]
    fn comparing_check_results() -> anyhow::Result<()> {
        let p = |m: &str| crate::plan::Problem {
            shot: 1,
            message: m.into(),
        };
        let (new, fixed) = super::compare(&[p("a"), p("b")], &[p("b"), p("c")]);
        assert_eq!(new, vec![p("c")], "c is new");
        assert_eq!(fixed, 1, "a fixed");
        Ok(())
    }
}

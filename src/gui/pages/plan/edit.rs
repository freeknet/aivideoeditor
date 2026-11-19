// ============================================================================
// Module:       gui::pages::plan::edit
// Description:  The timeline's edits as pure functions on a plan (reorder,
//               roll, lift, slip, swap) and the undo / redo stack.
//
// Dependencies: crate::{plan, beats}
// ============================================================================

//! Editing a plan with the music as the clock.
//!
//! The song never moves: the plan's footage always fills
//! `start_bar..card_bar`. So a reorder moves *content* between slots and
//! leaves the slot lengths where they were, a roll moves one edit point and
//! gives the beats one neighbour loses to the other, and a lift hands the
//! removed shot's beats to the shot before it. Each function returns whether
//! it changed anything, so the caller only records real edits for undo.

use crate::beats::MusicMap;
use crate::plan::{Plan, Shot};

/// Undo steps kept.
const DEPTH: usize = 200;

/// Beat offset (from the plan start) at which each shot begins, plus the
/// end of the last one.
pub(super) fn starts(plan: &Plan) -> Vec<usize> {
    let mut out = Vec::with_capacity(plan.shots.len() + 1);
    let mut beat = 0;
    out.push(0);
    for s in &plan.shots {
        beat += s.beats;
        out.push(beat);
    }
    out
}

/// Times (seconds from the plan start) of every beat from the start bar to
/// the end bar, inclusive: the timeline's grid.
pub(super) fn grid(plan: &Plan, map: &MusicMap) -> Vec<f32> {
    let origin = map.beat_time(plan.start_bar, 0);
    let n = plan.end_bar.saturating_sub(plan.start_bar) * 4;
    (0..=n)
        .map(|k| map.beat_time(plan.start_bar, k) - origin)
        .collect()
}

/// The beat (index into `grid`) nearest to `t` seconds.
pub(super) fn nearest_beat(grid: &[f32], t: f32) -> usize {
    let after = grid.partition_point(|&g| g < t);
    match (after.checked_sub(1), grid.get(after)) {
        (Some(b), Some(&ta)) => {
            if (t - grid[b]).abs() <= (ta - t).abs() {
                b
            } else {
                after
            }
        }
        (Some(b), None) => b,
        (None, _) => 0,
    }
}

/// Moves the shot at `from` to slot `to`; every slot keeps its length, so
/// the shots in between shift by one slot and take that slot's length.
pub(super) fn reorder(plan: &mut Plan, from: usize, to: usize) -> bool {
    let n = plan.shots.len();
    if from >= n || to >= n || from == to {
        return false;
    }
    let lengths: Vec<usize> = plan.shots.iter().map(|s| s.beats).collect();
    let shot = plan.shots.remove(from);
    plan.shots.insert(to, shot);
    for (s, beats) in plan.shots.iter_mut().zip(lengths) {
        s.beats = beats;
    }
    true
}

/// Rolls the edit point between shot `edit - 1` and shot `edit` so the left
/// shot is `left` beats long; the right one takes the rest of the pair.
/// Each keeps at least one beat.
pub(super) fn roll(plan: &mut Plan, edit: usize, left: usize) -> bool {
    if edit == 0 || edit >= plan.shots.len() {
        return false;
    }
    let pair = plan.shots[edit - 1].beats + plan.shots[edit].beats;
    if pair < 2 {
        return false;
    }
    let left = left.clamp(1, pair - 1);
    if left == plan.shots[edit - 1].beats {
        return false;
    }
    plan.shots[edit - 1].beats = left;
    plan.shots[edit].beats = pair - left;
    true
}

/// Removes shot `k` and gives its beats to the shot before it (or after
/// it, for the first). The last remaining shot cannot be lifted.
pub(super) fn lift(plan: &mut Plan, k: usize) -> bool {
    if k >= plan.shots.len() || plan.shots.len() < 2 {
        return false;
    }
    let gone = plan.shots.remove(k);
    let heir = k.saturating_sub(1);
    if let Some(s) = plan.shots.get_mut(heir) {
        s.beats += gone.beats;
    }
    true
}

/// Sets shot `k`'s in-point to `t_in`, kept inside a clip of `clip_len`
/// seconds for a shot `dur` seconds long (unknown length: only `>= 0`).
pub(super) fn slip(plan: &mut Plan, k: usize, t_in: f32, dur: f32, clip_len: Option<f32>) -> bool {
    let Some(shot) = plan.shots.get_mut(k) else {
        return false;
    };
    let hi = clip_len.map_or(f32::MAX, |len| (len - dur).max(0.0));
    let t = ((t_in.clamp(0.0, hi)) * 100.0).round() / 100.0;
    if (t - shot.t_in).abs() < 1e-4 {
        return false;
    }
    shot.t_in = t;
    true
}

/// Puts `with` in slot `k`, keeping the slot's length.
pub(super) fn swap(plan: &mut Plan, k: usize, with: Shot) -> bool {
    let Some(shot) = plan.shots.get_mut(k) else {
        return false;
    };
    let next = Shot {
        beats: shot.beats,
        ..with
    };
    if *shot == next {
        return false;
    }
    *shot = next;
    true
}

/// Sets shot `k`'s crop centre, kept where the crop stays in frame.
pub(super) fn reframe(plan: &mut Plan, k: usize, cx: f32) -> bool {
    let half = plan.aspect.crop_frac() / 2.0;
    let Some(shot) = plan.shots.get_mut(k) else {
        return false;
    };
    let cx = (cx.clamp(half, 1.0 - half) * 100.0).round() / 100.0;
    if (cx - shot.cx).abs() < 1e-4 {
        return false;
    }
    shot.cx = cx;
    true
}

/// Plan states to step back and forward through, each with what changed.
#[derive(Debug, Default)]
pub(super) struct Undo {
    past: Vec<(Plan, String)>,
    future: Vec<(Plan, String)>,
}

impl Undo {
    /// Records the state before an edit called `label`.
    pub(super) fn push(&mut self, before: Plan, label: impl Into<String>) {
        self.past.push((before, label.into()));
        if self.past.len() > DEPTH {
            self.past.remove(0);
        }
        self.future.clear();
    }

    /// Steps back: `current` becomes the previous state. Returns the label
    /// of the edit undone.
    pub(super) fn undo(&mut self, current: &mut Plan) -> Option<String> {
        let (before, label) = self.past.pop()?;
        let now = std::mem::replace(current, before);
        self.future.push((now, label.clone()));
        Some(label)
    }

    /// Steps forward again. Returns the label of the edit redone.
    pub(super) fn redo(&mut self, current: &mut Plan) -> Option<String> {
        let (after, label) = self.future.pop()?;
        let now = std::mem::replace(current, after);
        self.past.push((now, label.clone()));
        Some(label)
    }

    /// What Undo would undo.
    pub(super) fn undo_label(&self) -> Option<&str> {
        self.past.last().map(|(_, l)| l.as_str())
    }

    /// What Redo would redo.
    pub(super) fn redo_label(&self) -> Option<&str> {
        self.future.last().map(|(_, l)| l.as_str())
    }

    /// Forgets everything (another plan was opened).
    pub(super) fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{Undo, lift, nearest_beat, reorder, roll, slip, starts, swap};
    use crate::plan::{Aspect, Plan, Rules, Shot};

    fn shot(clip: &str, beats: usize) -> Shot {
        Shot {
            clip: clip.into(),
            t_in: 1.0,
            beats,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: String::new(),
        }
    }

    fn plan() -> Plan {
        Plan {
            name: "t".into(),
            aspect: Aspect::Vertical,
            music: String::new(),
            start_bar: 0,
            drop_bar: 1,
            card_bar: 3,
            end_bar: 4,
            end_card: String::new(),
            rules: Rules::default(),
            shots: vec![shot("A", 4), shot("B", 2), shot("C", 2), shot("D", 4)],
            finish: Default::default(),
            comments: Vec::new(),
        }
    }

    fn clips(p: &Plan) -> String {
        p.shots.iter().map(|s| s.clip.as_str()).collect()
    }

    fn lengths(p: &Plan) -> Vec<usize> {
        p.shots.iter().map(|s| s.beats).collect()
    }

    #[test]
    fn reorder_moves_content_not_slots() -> anyhow::Result<()> {
        let mut p = plan();
        assert!(reorder(&mut p, 0, 2), "moved");
        assert_eq!(clips(&p), "BCAD", "A now third");
        assert_eq!(lengths(&p), vec![4, 2, 2, 4], "slot lengths unchanged");
        assert!(reorder(&mut p, 3, 0), "moved back");
        assert_eq!(clips(&p), "DBCA", "D first");
        assert!(!reorder(&mut p, 1, 1), "no-op is not an edit");
        assert!(!reorder(&mut p, 9, 0), "out of range");
        Ok(())
    }

    #[test]
    fn roll_trades_beats_between_neighbours() -> anyhow::Result<()> {
        let mut p = plan();
        let total: usize = lengths(&p).iter().sum();
        assert!(roll(&mut p, 1, 5), "A takes a beat from B");
        assert_eq!(lengths(&p), vec![5, 1, 2, 4], "pair kept at 6");
        assert!(!roll(&mut p, 1, 9), "clamped: B already has one beat");
        assert_eq!(lengths(&p), vec![5, 1, 2, 4], "B keeps one beat");
        assert!(!roll(&mut p, 1, 5), "same length is no edit");
        assert!(roll(&mut p, 3, 1), "C shrinks to 1");
        assert_eq!(lengths(&p), vec![5, 1, 1, 5], "D grows");
        assert_eq!(lengths(&p).iter().sum::<usize>(), total, "song fixed");
        assert!(!roll(&mut p, 0, 2), "no edit before the first shot");
        Ok(())
    }

    #[test]
    fn lift_hands_beats_to_the_previous_shot() -> anyhow::Result<()> {
        let mut p = plan();
        assert!(lift(&mut p, 2), "lift C");
        assert_eq!(clips(&p), "ABD", "C gone");
        assert_eq!(lengths(&p), vec![4, 4, 4], "B took C's beats");
        assert!(lift(&mut p, 0), "lift the first");
        assert_eq!(lengths(&p), vec![8, 4], "the next takes them");
        assert!(lift(&mut p, 1), "lift");
        assert!(!lift(&mut p, 0), "the last shot stays");
        assert_eq!(starts(&p), vec![0, 12], "still fills 12 beats");
        Ok(())
    }

    #[test]
    fn slip_stays_inside_the_clip() -> anyhow::Result<()> {
        let mut p = plan();
        assert!(slip(&mut p, 0, 50.0, 2.0, Some(10.0)), "clamped slip");
        assert!((p.shots[0].t_in - 8.0).abs() < 1e-4, "ends at the clip end");
        assert!(slip(&mut p, 0, -3.0, 2.0, Some(10.0)), "clamped");
        assert!(p.shots[0].t_in.abs() < 1e-4, "never before 0");
        assert!(!slip(&mut p, 0, 0.0, 2.0, None), "unchanged");
        Ok(())
    }

    #[test]
    fn swap_keeps_the_slot_length() -> anyhow::Result<()> {
        let mut p = plan();
        assert!(swap(&mut p, 1, shot("Z", 8)), "swapped");
        assert_eq!(clips(&p), "AZCD", "Z in slot 2");
        assert_eq!(lengths(&p), vec![4, 2, 2, 4], "slot length kept");
        Ok(())
    }

    #[test]
    fn undo_and_redo_walk_the_states() -> anyhow::Result<()> {
        let mut p = plan();
        let mut undo = Undo::default();
        let before = p.clone();
        lift(&mut p, 1);
        undo.push(before.clone(), "lift 2");
        assert_eq!(undo.undo_label(), Some("lift 2"), "labelled");
        assert_eq!(undo.undo(&mut p).as_deref(), Some("lift 2"), "undone");
        assert_eq!(p, before, "back to the start");
        assert_eq!(undo.redo(&mut p).as_deref(), Some("lift 2"), "redone");
        assert_eq!(clips(&p), "ACD", "lift again");
        assert!(undo.redo(&mut p).is_none(), "nothing more to redo");
        let mid = p.clone();
        reorder(&mut p, 0, 1);
        undo.push(mid, "move");
        assert!(undo.redo_label().is_none(), "a new edit clears redo");
        Ok(())
    }

    #[test]
    fn the_nearest_beat_snaps_both_ways() -> anyhow::Result<()> {
        let grid = [0.0, 0.5, 1.0, 1.5];
        assert_eq!(nearest_beat(&grid, 0.7), 1, "nearer 0.5");
        assert_eq!(nearest_beat(&grid, 0.8), 2, "nearer 1.0");
        assert_eq!(nearest_beat(&grid, -1.0), 0, "clamped low");
        assert_eq!(nearest_beat(&grid, 9.0), 3, "clamped high");
        Ok(())
    }
}

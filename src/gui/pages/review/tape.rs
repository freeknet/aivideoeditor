// ============================================================================
// Module:       gui::pages::review::tape
// Description:  Tape mode: every result of the Shots tab back to back in the
//               shared viewer, Resolve Source Tape style, with Fast Review.
//
// Dependencies: crate::gui::player (Player, SeqShot, Source), crate::index
// ============================================================================

//! The tape.
//!
//! `T` loads the current result set into the viewer as one sequence and
//! plays it from the focused shot. The card under the playhead is
//! highlighted, and the selection follows the playhead, so `P` / `X` / `U`
//! mark the shot that is playing and jump to the next one.
//!
//! **Fast Review** (`Shift+L`) is Resolve's: short shots play at normal
//! speed, longer ones faster, so a 20 s wide does not cost 20 s. The rate
//! is set as each shot starts: its length over [`FAST_TARGET`], between
//! 1x and [`FAST_MAX`].

use std::collections::HashMap;
use std::path::PathBuf;

use crate::gui::player::{Player, SeqShot, Source};
use crate::index::Index;

/// Under Fast Review a shot plays in about this many seconds, unless that
/// would mean slowing it down.
const FAST_TARGET: f64 = 3.0;

/// The fastest Fast Review plays a shot.
const FAST_MAX: f64 = 4.0;

/// The playback rate Fast Review gives a shot `dur` seconds long.
pub(super) fn fast_rate(dur: f64) -> f64 {
    (dur / FAST_TARGET).clamp(1.0, FAST_MAX)
}

/// A tape in the viewer.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Tape {
    /// The shots, as the viewer was given them.
    pub(super) shots: Vec<SeqShot>,
    /// The segment id of each shot.
    pub(super) ids: Vec<String>,
    /// The shot the playhead was last seen on.
    pub(super) last: Option<usize>,
    /// Fast Review on.
    pub(super) fast: bool,
}

/// What following the playhead found.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Follow {
    /// The viewer holds something else now: the tape is over.
    Gone,
    /// Still on the same shot.
    Same,
    /// The playhead moved onto this segment.
    Now(String),
}

impl Tape {
    /// A tape of segments `segs` (positions in `index.segments`), skipping
    /// any whose source file is unknown.
    pub(super) fn new(
        index: &Index,
        segs: impl Iterator<Item = usize>,
        paths: &HashMap<String, PathBuf>,
        fast: bool,
    ) -> Self {
        let mut shots = Vec::new();
        let mut ids = Vec::new();
        for s in segs.filter_map(|k| index.segments.get(k)) {
            let Some(path) = paths.get(&s.clip) else {
                continue;
            };
            shots.push(SeqShot {
                path: path.clone(),
                t_in: f64::from(s.seg.start),
                dur: f64::from((s.seg.end - s.seg.start).max(0.04)),
                cx: s.seg.cx,
            });
            ids.push(s.id.clone());
        }
        Self {
            shots,
            ids,
            last: None,
            fast,
        }
    }

    /// Where shot `k` starts on the tape, seconds.
    pub(super) fn start_of(&self, k: usize) -> f64 {
        self.shots.iter().take(k).map(|s| s.dur).sum()
    }

    /// The shot of segment `id`.
    pub(super) fn position_of(&self, id: &str) -> Option<usize> {
        self.ids.iter().position(|x| x == id)
    }

    /// The segment id under the playhead.
    pub(super) fn playing(&self) -> Option<&str> {
        self.last.and_then(|k| self.ids.get(k)).map(String::as_str)
    }

    /// Whether the viewer still holds this tape.
    pub(super) fn is_loaded(&self, player: &Player) -> bool {
        match player.source() {
            Source::Sequence { shots, music } => music.is_none() && *shots == self.shots,
            Source::Empty | Source::Clip { .. } | Source::Audio { .. } => false,
        }
    }

    /// The rate shot `k` plays at.
    pub(super) fn rate_for(&self, k: usize) -> f64 {
        match (self.fast, self.shots.get(k)) {
            (true, Some(s)) => fast_rate(s.dur),
            (true, None) | (false, _) => 1.0,
        }
    }
}

/// Follows the playhead: notes the shot it is on and, under Fast Review,
/// sets that shot's rate as it starts (or after Space reset it to 1x).
pub(super) fn follow(tape: &mut Tape, player: &mut Player) -> Follow {
    if !tape.is_loaded(player) {
        return Follow::Gone;
    }
    let Some((k, _)) = player.sequence_shot() else {
        return Follow::Same;
    };
    let changed = tape.last != Some(k);
    let reset = player.playing() && (player.rate() - 1.0).abs() < 1e-9;
    if tape.fast && player.rate() > 0.0 && (changed || reset) {
        player.set_rate(tape.rate_for(k));
    }
    if !changed {
        return Follow::Same;
    }
    tape.last = Some(k);
    tape.ids.get(k).cloned().map_or(Follow::Same, Follow::Now)
}

#[cfg(test)]
mod tests {
    use super::{FAST_MAX, Tape, fast_rate};
    use crate::gui::player::SeqShot;

    #[test]
    fn fast_review_leaves_short_shots_alone_and_caps_long_ones() {
        assert!(
            (fast_rate(1.5) - 1.0).abs() < 1e-9,
            "a short shot plays at 1x"
        );
        assert!(
            (fast_rate(6.0) - 2.0).abs() < 1e-9,
            "a 6 s shot plays in 3 s"
        );
        assert!(
            (fast_rate(120.0) - FAST_MAX).abs() < 1e-9,
            "a long shot is capped"
        );
    }

    #[test]
    fn shots_start_where_the_ones_before_end() {
        let shot = |dur| SeqShot {
            path: "a.mov".into(),
            t_in: 0.0,
            dur,
            cx: 0.5,
        };
        let tape = Tape {
            shots: vec![shot(2.0), shot(3.0), shot(9.0)],
            ids: vec!["a/1".into(), "a/2".into(), "b/1".into()],
            last: Some(2),
            fast: true,
        };
        assert!((tape.start_of(2) - 5.0).abs() < 1e-9, "after 2 + 3 s");
        assert_eq!(tape.position_of("a/2"), Some(1), "found by id");
        assert_eq!(tape.playing(), Some("b/1"), "the last shot seen");
        assert!((tape.rate_for(2) - 3.0).abs() < 1e-9, "9 s at 3x");
        assert!((tape.rate_for(0) - 1.0).abs() < 1e-9, "2 s at 1x");
    }
}

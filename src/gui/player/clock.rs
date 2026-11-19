// ============================================================================
// Module:       gui::player::clock
// Description:  The pure arithmetic of playback: where a position falls in a
//               list of segments, timecode, shuttle speeds.
//
// Dependencies: std
// ============================================================================

//! Time arithmetic for the viewer, kept free of egui, threads and ffmpeg so
//! it can be tested on its own.
//!
//! Every source the viewer plays is reduced to a list of [`Seg`]s laid back
//! to back: a clip range is one segment, a cut is one per shot. A position
//! is seconds from the start of that list; [`locate`] turns it into "segment
//! *i*, *w* seconds in", which is what the decoders need.

use std::path::PathBuf;

/// One stretch of one file on the viewer's timeline.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Seg {
    /// Source file.
    pub(crate) path: PathBuf,
    /// In-point in the file, seconds.
    pub(crate) t_in: f64,
    /// Length, seconds.
    pub(crate) dur: f64,
    /// Horizontal crop centre for narrow aspects.
    pub(crate) cx: f32,
}

impl Seg {
    /// The file time `within` seconds into this segment.
    pub(crate) fn file_time(&self, within: f64) -> f64 {
        self.t_in + within.clamp(0.0, self.dur)
    }

    /// The file time this segment ends at.
    pub(crate) fn t_out(&self) -> f64 {
        self.t_in + self.dur
    }
}

/// The segment under `pos` and the time within it. Past the end, the last
/// segment at its end; before the start, the first at zero.
pub(crate) fn locate(segs: &[Seg], pos: f64) -> Option<(usize, f64)> {
    let mut start = 0.0;
    for (i, seg) in segs.iter().enumerate() {
        if pos < start + seg.dur || i + 1 == segs.len() {
            return Some((i, (pos - start).clamp(0.0, seg.dur)));
        }
        start += seg.dur;
    }
    None
}

/// Where segment `index` starts on the timeline, seconds.
pub(crate) fn seg_start(segs: &[Seg], index: usize) -> f64 {
    segs.iter().take(index).map(|s| s.dur).sum()
}

/// Total length of the segments, seconds.
pub(crate) fn total(segs: &[Seg]) -> f64 {
    segs.iter().map(|s| s.dur).sum()
}

/// The segments after `index` that start within `lead` seconds of a
/// playhead `within` seconds into it, nearest first, at most `max`;
/// wrapping to the start when `wrap` (a looping cut). Never `index` itself.
pub(crate) fn upcoming(
    segs: &[Seg],
    index: usize,
    within: f64,
    lead: f64,
    wrap: bool,
    max: usize,
) -> Vec<usize> {
    let n = segs.len();
    let mut out = Vec::new();
    let Some(cur) = segs.get(index) else {
        return out;
    };
    let mut until = (cur.dur - within).max(0.0);
    for step in 1..n {
        let i = index + step;
        if i >= n && !wrap {
            break;
        }
        let i = i % n;
        if until >= lead || out.len() >= max {
            break;
        }
        out.push(i);
        until += segs.get(i).map_or(0.0, |s| s.dur);
    }
    out
}

/// The frame rate used to count frames in a timecode: 23.976 counts as 24,
/// 29.97 as 30 (non-drop-frame), anything unknown as 24.
pub(crate) fn nominal_fps(fps: f64) -> u64 {
    if fps.is_finite() && fps >= 1.0 {
        fps.round() as u64
    } else {
        24
    }
}

/// `t` seconds as `HH:MM:SS:FF` at `fps` (non-drop-frame: the frame
/// number is counted at the true rate, then split at the nominal one, the
/// way a camera's timecode counts 23.976 footage).
pub(crate) fn timecode(t: f64, fps: f64) -> String {
    let exact = if fps.is_finite() && fps >= 1.0 {
        fps
    } else {
        24.0
    };
    let nominal = nominal_fps(fps);
    // A hair of slack so 1.0 s at 24 fps is frame 24, not 23.999...
    let frames = (t.max(0.0) * exact + 1e-6).floor() as u64;
    let ff = frames % nominal;
    let secs = frames / nominal;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        secs / 3600,
        (secs / 60) % 60,
        secs % 60,
        ff
    )
}

/// The next shuttle rate after pressing J (`dir` −1) or L (`dir` +1) while
/// playing at `rate` (0 when paused): the same direction doubles up to ×8,
/// the other direction starts again at ×1.
pub(crate) fn shuttle(rate: f64, playing: bool, dir: f64) -> f64 {
    if playing && rate != 0.0 && rate.signum() == dir.signum() {
        (rate * 2.0).clamp(-8.0, 8.0)
    } else {
        dir.signum()
    }
}

/// Wraps or clamps a position that ran past `[lo, hi]`. Returns the new
/// position and whether playback should continue.
pub(crate) fn bound(pos: f64, lo: f64, hi: f64, looping: bool) -> (f64, bool) {
    if pos >= lo && pos <= hi {
        return (pos, true);
    }
    let span = (hi - lo).max(1e-3);
    if looping {
        (lo + (pos - lo).rem_euclid(span), true)
    } else {
        (pos.clamp(lo, hi), false)
    }
}

#[cfg(test)]
mod tests {
    use super::{Seg, bound, locate, seg_start, shuttle, timecode, total, upcoming};

    fn seg(t_in: f64, dur: f64) -> Seg {
        Seg {
            path: "a.mov".into(),
            t_in,
            dur,
            cx: 0.5,
        }
    }

    #[test]
    fn positions_map_to_segments() -> anyhow::Result<()> {
        let segs = [seg(10.0, 1.0), seg(50.0, 2.0), seg(5.0, 1.0)];
        assert!((total(&segs) - 4.0).abs() < 1e-9, "sum of lengths");
        assert_eq!(locate(&segs, 0.5), Some((0, 0.5)), "inside the first");
        let Some((i, w)) = locate(&segs, 1.5) else {
            anyhow::bail!("no segment at 1.5");
        };
        assert_eq!(i, 1, "second shot");
        assert!((w - 0.5).abs() < 1e-9, "half a second in");
        assert!(
            (segs[1].file_time(w) - 50.5).abs() < 1e-9,
            "file time follows the in-point"
        );
        assert_eq!(locate(&segs, 9.0), Some((2, 1.0)), "past the end clamps");
        assert_eq!(locate(&segs, -1.0), Some((0, 0.0)), "before the start");
        assert_eq!(locate(&[], 1.0), None, "nothing loaded");
        assert!(
            (seg_start(&segs, 2) - 3.0).abs() < 1e-9,
            "third starts at 3"
        );
        Ok(())
    }

    #[test]
    fn upcoming_shots_are_warmed_within_the_lead() -> anyhow::Result<()> {
        let segs = [seg(0.0, 1.0), seg(0.0, 1.0), seg(0.0, 1.0), seg(0.0, 5.0)];
        assert_eq!(
            upcoming(&segs, 0, 0.5, 2.0, false, 3),
            vec![1, 2],
            "shots starting within two seconds"
        );
        assert_eq!(upcoming(&segs, 0, 0.0, 9.0, false, 2), vec![1, 2], "capped");
        let none: Vec<usize> = Vec::new();
        assert_eq!(upcoming(&segs, 3, 4.0, 2.0, false, 3), none, "the end");
        assert_eq!(upcoming(&segs, 3, 4.0, 2.5, true, 3), vec![0, 1], "wraps");
        assert_eq!(
            upcoming(&segs[..1], 0, 0.0, 9.0, true, 3),
            none,
            "never itself"
        );
        Ok(())
    }

    #[test]
    fn timecode_counts_frames_at_the_source_rate() -> anyhow::Result<()> {
        assert_eq!(timecode(0.0, 23.976), "00:00:00:00", "zero");
        assert_eq!(timecode(1.0, 24.0), "00:00:01:00", "one second");
        assert_eq!(timecode(0.5, 24.0), "00:00:00:12", "half a second");
        assert_eq!(
            timecode(3661.0 + 23.0 / 24.0, 24.0),
            "01:01:01:23",
            "hours, minutes, seconds, last frame"
        );
        // 23.976: one real second is 23.976 frames, so frame 23 still.
        assert_eq!(timecode(1.0, 24000.0 / 1001.0), "00:00:00:23", "NTSC film");
        assert_eq!(timecode(-3.0, 30.0), "00:00:00:00", "never negative");
        assert_eq!(timecode(1.0, 0.0), "00:00:01:00", "unknown rate is 24");
        Ok(())
    }

    #[test]
    fn shuttle_doubles_and_reverses() -> anyhow::Result<()> {
        assert!((shuttle(0.0, false, 1.0) - 1.0).abs() < 1e-9, "L from stop");
        assert!((shuttle(1.0, true, 1.0) - 2.0).abs() < 1e-9, "L again");
        assert!((shuttle(8.0, true, 1.0) - 8.0).abs() < 1e-9, "capped at 8");
        assert!((shuttle(4.0, true, -1.0) + 1.0).abs() < 1e-9, "J reverses");
        assert!((shuttle(-2.0, true, -1.0) + 4.0).abs() < 1e-9, "J doubles");
        assert!(
            (shuttle(2.0, false, 1.0) - 1.0).abs() < 1e-9,
            "paused restarts"
        );
        Ok(())
    }

    #[test]
    fn running_off_the_end_loops_or_stops() -> anyhow::Result<()> {
        let (p, go) = bound(5.5, 1.0, 5.0, true);
        assert!(go && (p - 1.5).abs() < 1e-9, "looped back: {p}");
        let (p, go) = bound(5.5, 1.0, 5.0, false);
        assert!(!go && (p - 5.0).abs() < 1e-9, "stopped at the end: {p}");
        let (p, go) = bound(0.5, 1.0, 5.0, true);
        assert!(
            go && (p - 4.5).abs() < 1e-9,
            "reverse wraps to the end: {p}"
        );
        let (p, go) = bound(3.0, 1.0, 5.0, false);
        assert!(go && (p - 3.0).abs() < 1e-9, "inside is untouched");
        Ok(())
    }
}

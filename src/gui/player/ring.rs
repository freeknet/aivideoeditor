// ============================================================================
// Module:       gui::player::ring
// Description:  The small buffer of timestamped frames between a decoder
//               thread and the UI, and the rule for which frame to show.
//
// Dependencies: egui (ColorImage), std::sync
// ============================================================================

//! A bounded queue of decoded frames.
//!
//! The decoder thread [`Ring::push`]es frames in presentation order and
//! blocks while the ring is full, so a paused viewer holds a few frames of
//! look-ahead (stepping forward is then instant) and costs no CPU. The UI
//! asks [`Ring::take`] for the frame under the clock: frames older than it
//! are dropped, which frees room and lets the decoder run on. When the clock
//! is somewhere the ring cannot serve (before its first frame, or far past
//! its last), the answer is [`Pick::Restart`] and the caller starts a new
//! decoder at the clock.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A decoded frame and the file time it shows.
#[derive(Clone)]
pub(crate) struct Frame {
    /// File time, seconds.
    pub(crate) t: f64,
    /// The picture, ready to hand to a texture.
    pub(crate) image: Arc<egui::ColorImage>,
}

/// What the ring can do for a given clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// Show the frame at this index.
    Show(usize),
    /// Nothing yet; the decoder is still starting.
    Wait,
    /// The clock is outside what this decoder will ever produce.
    Restart,
}

/// How far past the newest frame the clock may run before the decoder is
/// judged unable to catch up, seconds. Generous: on a busy machine a
/// restart costs more than lagging (jumps restart explicitly instead).
const MAX_LAG: f64 = 3.0;

/// Chooses the frame to show at file time `t` from frames at `times`
/// (ascending), each `frame_dur` long. `ended` says the decoder has
/// produced its last frame.
pub(crate) fn pick(times: &[f64], t: f64, frame_dur: f64, ended: bool) -> Pick {
    let Some(&first) = times.first() else {
        return Pick::Wait;
    };
    // The first frame after a seek can land a frame or so late.
    if t < first - frame_dur * 1.5 {
        return Pick::Restart;
    }
    let edge = t + frame_dur * 0.5;
    let k = times.iter().rposition(|&ft| ft <= edge).unwrap_or(0);
    if k + 1 == times.len() && !ended && t > times[k] + MAX_LAG {
        return Pick::Restart;
    }
    Pick::Show(k)
}

/// The state behind the lock.
struct Inner {
    frames: VecDeque<Frame>,
    ended: bool,
    cancelled: bool,
}

/// A bounded frame queue shared by one decoder thread and the UI.
pub(crate) struct Ring {
    inner: Mutex<Inner>,
    room: Condvar,
    capacity: usize,
}

impl Ring {
    /// An empty ring holding at most `capacity` frames.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(Inner {
                frames: VecDeque::with_capacity(capacity),
                ended: false,
                cancelled: false,
            }),
            room: Condvar::new(),
            capacity: capacity.max(2),
        }
    }

    /// Adds a frame, waiting while the ring is full. Returns `false` once
    /// the ring is cancelled: the decoder should stop.
    pub(crate) fn push(&self, frame: Frame) -> bool {
        let Ok(mut inner) = self.inner.lock() else {
            return false;
        };
        loop {
            if inner.cancelled {
                return false;
            }
            if inner.frames.len() < self.capacity {
                inner.frames.push_back(frame);
                return true;
            }
            match self.room.wait_timeout(inner, Duration::from_millis(100)) {
                Ok((guard, _)) => inner = guard,
                Err(_) => return false,
            }
        }
    }

    /// Marks the stream finished: no more frames will come.
    pub(crate) fn end(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.ended = true;
        }
    }

    /// Tells the decoder to stop at its next frame.
    pub(crate) fn cancel(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.cancelled = true;
            inner.frames.clear();
        }
        self.room.notify_all();
    }

    /// Whether the decoder has stopped producing.
    pub(crate) fn ended(&self) -> bool {
        self.inner.lock().map(|i| i.ended).unwrap_or(true)
    }

    /// The frame for file time `t` (see [`pick`]); drops older frames.
    pub(crate) fn take(&self, t: f64, frame_dur: f64) -> (Pick, Option<Frame>) {
        let Ok(mut inner) = self.inner.lock() else {
            return (Pick::Restart, None);
        };
        let times: Vec<f64> = inner.frames.iter().map(|f| f.t).collect();
        let choice = pick(&times, t, frame_dur, inner.ended);
        let frame = match choice {
            Pick::Show(k) => {
                inner.frames.drain(..k);
                inner.frames.front().cloned()
            }
            Pick::Wait | Pick::Restart => None,
        };
        drop(inner);
        if matches!(choice, Pick::Show(k) if k > 0) {
            self.room.notify_all();
        }
        (choice, frame)
    }
}

#[cfg(test)]
mod tests {
    use super::{Frame, Pick, Ring, pick};
    use std::sync::Arc;

    const FD: f64 = 1.0 / 24.0;

    fn times(start: f64, n: usize) -> Vec<f64> {
        (0..n).map(|k| start + k as f64 * FD).collect()
    }

    #[test]
    fn the_frame_under_the_clock_is_chosen() -> anyhow::Result<()> {
        let ts = times(10.0, 8);
        assert_eq!(pick(&ts, 10.0, FD, false), Pick::Show(0), "first frame");
        assert_eq!(
            pick(&ts, 10.0 + 2.2 * FD, FD, false),
            Pick::Show(2),
            "third"
        );
        assert_eq!(
            pick(&ts, 10.0 + 2.6 * FD, FD, false),
            Pick::Show(3),
            "rounds to the nearest frame"
        );
        assert_eq!(pick(&[], 10.0, FD, false), Pick::Wait, "decoder starting");
        Ok(())
    }

    #[test]
    fn a_clock_the_ring_cannot_serve_asks_for_a_restart() -> anyhow::Result<()> {
        let ts = times(10.0, 8);
        assert_eq!(pick(&ts, 9.0, FD, false), Pick::Restart, "seeked back");
        assert_eq!(
            pick(&ts, 10.0 - FD, FD, false),
            Pick::Show(0),
            "a late first frame is tolerated"
        );
        assert_eq!(pick(&ts, 20.0, FD, false), Pick::Restart, "far ahead");
        assert_eq!(
            pick(&ts, 11.5, FD, false),
            Pick::Show(7),
            "a decoder a second behind keeps going"
        );
        assert_eq!(
            pick(&ts, 20.0, FD, true),
            Pick::Show(7),
            "an ended stream holds its last frame"
        );
        assert_eq!(
            pick(&ts, 10.0 + 7.0 * FD + 0.5, FD, false),
            Pick::Show(7),
            "a little behind: keep showing the newest"
        );
        Ok(())
    }

    #[test]
    fn taking_drops_older_frames_and_keeps_the_shown_one() -> anyhow::Result<()> {
        let ring = Ring::new(4);
        let image = Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::BLACK));
        for t in times(0.0, 4) {
            let pushed = ring.push(Frame {
                t,
                image: image.clone(),
            });
            assert!(pushed, "room for four");
        }
        let (choice, frame) = ring.take(2.0 * FD, FD);
        assert_eq!(choice, Pick::Show(2), "third frame");
        let shown = frame.map(|f| f.t).unwrap_or(-1.0);
        assert!((shown - 2.0 * FD).abs() < 1e-9, "returns it: {shown}");
        let (again, _) = ring.take(2.0 * FD, FD);
        assert_eq!(again, Pick::Show(0), "kept at the front for a paused clock");
        ring.cancel();
        let refused = ring.push(Frame { t: 9.0, image });
        assert!(!refused, "a cancelled ring stops its decoder");
        Ok(())
    }
}

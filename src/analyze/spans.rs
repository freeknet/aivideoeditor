// ============================================================================
// Module:       analyze::spans
// Description:  Finds short bad stretches (a focus hunt, a whip, a jolt, a
//               dark or blown moment) inside segments from the per-sample
//               measurements, and helps plans steer around them.
//
// Dependencies: crate::analyze::segments, crate::index (thresholds), serde
// ============================================================================

//! Bad spans.
//!
//! A segment's `flags` judge it whole: a 6 s segment whose median focus is
//! fine is not `soft`, even when the lens hunts for half a second in the
//! middle. Bad spans are those half seconds. Each sample (6 per second) is
//! marked against the same rules the segment flags use, with the night's
//! thresholds, but [`MARGIN`] past them: one sample is noisier than a
//! segment's mean, and a sample a hair under the line is not a moment an
//! editor would cut around.
//!
//! - `soft`: focus (median of three samples, so one noisy frame does not
//!   count) under `soft` x the night's median;
//! - `dark`: the lit share, lit-region mean or lit-region p95 under its
//!   threshold (`min_lit`, `dark`, `highlights`); the value is the worst of
//!   the three as a fraction of its threshold (0.7 = 70 % of the line);
//! - `blown`: clipped share over `blown`;
//! - `whip`: the camera classifier calls the sample a whip (too fast to
//!   read), even if the segmenter smoothed the run away as too short;
//! - `shaky`: handheld jitter over [`SHAKY`].
//!
//! Runs of one mark closer than [`MERGE_GAP`] merge, runs shorter than
//! [`MIN_LEN`] ([`MIN_WHIP`] for whips, which are brief by nature) are
//! dropped, and each run is cut to the segments it crosses (a piece under
//! [`MIN_PIECE`] left on the far side of a segment edge is dropped). A span whose
//! flag the whole segment already carries says nothing new and is left
//! out. A span keeps its worst value: lowest focus ratio or darkness
//! ratio, highest clipped share, pan speed or jitter.
//!
//! Spans are computed when the index is scored, from the cached samples, so
//! an old index gains them on the next `ave index` without re-measuring.

use super::Sample;
use super::segments::{Camera, Segment, classify};
use crate::index::Thresholds;

/// Shortest bad span, seconds (three samples at 6 Hz). Anything shorter is
/// a noisy frame, not a moment an editor would see.
pub(crate) const MIN_LEN: f32 = 0.5;

/// Shortest whip span, seconds (two samples): a whip pan is over fast and
/// the classifier already averages three samples of motion.
pub(crate) const MIN_WHIP: f32 = 0.33;

/// Runs of one flag separated by at most this many seconds (two samples)
/// merge into one span.
pub(crate) const MERGE_GAP: f32 = 0.34;

/// How far past a segment threshold one sample must be to count: 0.9
/// means 10 % under a lower bound (`soft`, `dark`) or 10 % over an upper
/// one (`blown`). Calibrated on 2026-11-11 (417 segments): with no margin
/// 38 spans (24 dark, mostly a hair under the line, e.g. lit 0.17 against
/// 0.18); at 0.9, 15 (6 dark, 9 soft, each a clear dip); at 0.85 only 7.
pub(crate) const MARGIN: f32 = 0.9;

/// Shortest piece of a run kept on one side of a segment edge, seconds
/// (two samples). A run cut by an edge leaves its real part on one side;
/// a lone sample on the other is the edge itself (often a cut).
pub(crate) const MIN_PIECE: f32 = 0.3;

/// Pan speed above which the camera classifier calls a sample a whip,
/// frame widths per second. Slower "whips" are texture loss with a colour
/// jump: blur.
const WHIP_SPEED: f32 = 0.9;

/// Jitter above which a sample is shaky; the same line the segment
/// classifier draws.
pub(crate) const SHAKY: f32 = 0.22;

/// How far a shot may overlap a span before it counts, seconds: the same
/// slack `plan::check` gives segment edges.
pub(crate) const SLACK: f32 = 0.05;

/// A short bad stretch inside a segment.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct BadSpan {
    /// Start, seconds in the clip.
    pub(crate) start: f32,
    /// End, seconds in the clip.
    pub(crate) end: f32,
    /// The flag (`soft`, `dark`, `blown`, `whip`, `shaky`).
    pub(crate) flag: String,
    /// The worst measurement inside the span (see the module docs).
    pub(crate) value: f32,
}

impl BadSpan {
    /// `whip 2.33-2.83` with the worst value in the unit its flag reads.
    pub(crate) fn text(&self) -> String {
        let value = match self.flag.as_str() {
            "soft" => format!(" focus {:.2}", self.value),
            "dark" => format!(" light {:.2}x threshold", self.value),
            "blown" => format!(" clipped {:.0}%", self.value * 100.0),
            "whip" if self.value < WHIP_SPEED => " blur".to_owned(),
            "whip" => format!(" speed {:.1}w/s", self.value),
            "shaky" => format!(" jitter {:.2}", self.value),
            _ => format!(" {:.2}", self.value),
        };
        format!("{} {:.2}-{:.2}{value}", self.flag, self.start, self.end)
    }

    /// Whether the window `t0..t1` overlaps the span by more than
    /// [`SLACK`].
    pub(crate) fn overlaps(&self, t0: f32, t1: f32) -> bool {
        self.end > t0 + SLACK && self.start < t1 - SLACK
    }
}

/// Whether a lower value is worse for `flag`.
fn lower_is_worse(flag: &str) -> bool {
    matches!(flag, "soft" | "dark")
}

/// The flags a sample could carry, in the order spans are reported.
const FLAGS: [&str; 5] = ["whip", "shaky", "soft", "dark", "blown"];

/// Per-sample marks of one clip: for each flag in [`FLAGS`], whether each
/// sample carries it and the value it was judged on.
pub(crate) struct Marks {
    /// Sample times.
    t: Vec<f32>,
    /// Sample period, seconds.
    dt: f32,
    /// `(flag, marked, value)` per flag.
    per_flag: Vec<(&'static str, Vec<bool>, Vec<f32>)>,
}

/// Marks every sample of a clip against `t`, with `sharp_ref` the night's
/// median focus.
pub(crate) fn marks(samples: &[Sample], sharp_ref: f32, t: &Thresholds) -> Marks {
    let n = samples.len();
    let dt = if n > 1 {
        samples[1].t - samples[0].t
    } else {
        1.0 / 6.0
    };
    let (classes, jitter) = classify(samples);
    let sharp_ref = sharp_ref.max(1e-6);
    let mut per_flag = Vec::new();
    for flag in FLAGS {
        let mut marked = Vec::with_capacity(n);
        let mut value = Vec::with_capacity(n);
        for i in 0..n {
            let s = &samples[i];
            let lo = i.saturating_sub(1);
            let hi = (i + 2).min(n);
            let (m, v) = match flag {
                "whip" => {
                    let k = (hi - lo) as f32;
                    let px = samples[lo..hi].iter().map(|x| x.pan_x).sum::<f32>() / k;
                    let py = samples[lo..hi].iter().map(|x| x.pan_y).sum::<f32>() / k;
                    (classes[i] == Camera::Whip, px.hypot(py))
                }
                "shaky" => (jitter[i] > SHAKY && classes[i] != Camera::Whip, jitter[i]),
                "soft" => {
                    let near: Vec<f32> = samples[lo..hi].iter().map(|x| x.sharp).collect();
                    let rel = super::segments::median(&near) / sharp_ref;
                    (rel < t.soft * MARGIN, rel)
                }
                "dark" => {
                    let ratio = |v: f32, line: f32| if line > 0.0 { v / line } else { 1.0 };
                    let worst = ratio(s.lit_frac, t.min_lit)
                        .min(ratio(s.lit, t.dark))
                        .min(ratio(s.lit_p95, t.highlights));
                    (worst < MARGIN, worst)
                }
                _ => (s.clipped * MARGIN > t.blown, s.clipped),
            };
            marked.push(m);
            value.push(v);
        }
        per_flag.push((flag, marked, value));
    }
    Marks {
        t: samples.iter().map(|s| s.t).collect(),
        dt,
        per_flag,
    }
}

/// Runs `lo..hi` (sample indices) of `marked` after merging gaps of at most
/// [`MERGE_GAP`] and dropping runs shorter than `min_len`.
fn runs(marked: &[bool], dt: f32, min_len: f32) -> Vec<(usize, usize)> {
    let gap = (MERGE_GAP / dt + 1e-3).floor() as usize;
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < marked.len() {
        if !marked[i] {
            i += 1;
            continue;
        }
        let start = i;
        while i < marked.len() && marked[i] {
            i += 1;
        }
        match out.last_mut() {
            Some(last) if start - last.1 <= gap => last.1 = i,
            Some(_) | None => out.push((start, i)),
        }
    }
    let min_n = (min_len / dt - 1e-3).ceil().max(1.0) as usize;
    out.retain(|(a, b)| b - a >= min_n);
    out
}

/// The bad spans inside `seg`, given its clip's `marks` and the flags the
/// whole segment already carries.
pub(crate) fn within(marks: &Marks, seg: &Segment, flags: &[String]) -> Vec<BadSpan> {
    let mut out = Vec::new();
    for (flag, marked, value) in &marks.per_flag {
        if flags.iter().any(|f| f == flag) {
            continue;
        }
        let min_len = if *flag == "whip" { MIN_WHIP } else { MIN_LEN };
        for (a, b) in runs(marked, marks.dt, min_len) {
            // The part of the run inside the segment.
            let idx: Vec<usize> = (a..b)
                .filter(|&k| marks.t[k] >= seg.start - 1e-4 && marks.t[k] < seg.end - 1e-4)
                .collect();
            let (Some(&first), Some(&last)) = (idx.first(), idx.last()) else {
                continue;
            };
            let cut_short = first > a || last + 1 < b;
            if cut_short && (idx.len() as f32) * marks.dt < MIN_PIECE {
                continue;
            }
            let vals = idx.iter().map(|&k| value[k]);
            let worst = if lower_is_worse(flag) {
                vals.fold(f32::INFINITY, f32::min)
            } else {
                vals.fold(0.0, f32::max)
            };
            let round = |x: f32| (x * 100.0).round() / 100.0;
            out.push(BadSpan {
                start: round(marks.t[first]),
                end: round((marks.t[last] + marks.dt).min(seg.end)),
                flag: (*flag).to_owned(),
                value: (worst * 1000.0).round() / 1000.0,
            });
        }
    }
    out.sort_by(|a, b| a.start.total_cmp(&b.start));
    out
}

/// The spans among `spans` whose flag is in `reject` and which overlap
/// `t0..t1`.
pub(crate) fn hits<'s>(
    spans: &'s [BadSpan],
    t0: f32,
    t1: f32,
    reject: &[String],
) -> impl Iterator<Item = &'s BadSpan> {
    spans
        .iter()
        .filter(move |s| reject.contains(&s.flag) && s.overlaps(t0, t1))
}

/// The in-point nearest `t_in`, inside `lo..=hi`, at which a `dur`-second
/// window overlaps none of `spans`; `None` when there is none. A window
/// that is clear already stays where it is.
pub(crate) fn dodge(spans: &[&BadSpan], t_in: f32, dur: f32, lo: f32, hi: f32) -> Option<f32> {
    let clear = |t: f32| spans.iter().all(|s| !s.overlaps(t, t + dur));
    if clear(t_in) {
        return Some(t_in);
    }
    spans
        .iter()
        .flat_map(|s| [s.end - SLACK * 0.5, s.start - dur + SLACK * 0.5])
        .filter(|&t| t >= lo && t <= hi && clear(t))
        .min_by(|a, b| (a - t_in).abs().total_cmp(&(b - t_in).abs()))
}

#[cfg(test)]
mod tests {
    use super::{BadSpan, dodge, marks, within};
    use crate::analyze::Sample;
    use crate::analyze::segments::Segment;
    use crate::index::Thresholds;

    /// A lit, sharp, steady sample at sample index `i`.
    fn good(i: usize) -> Sample {
        Sample {
            t: i as f32 / 6.0,
            sharp: 1.0,
            lit: 0.3,
            lit_frac: 0.05,
            lit_p95: 0.7,
            motion_ok: true,
            ..Sample::default()
        }
    }

    fn whole(samples: &[Sample]) -> Segment {
        Segment {
            start: 0.0,
            end: samples.len() as f32 / 6.0,
            ..Segment::default()
        }
    }

    #[test]
    fn a_focus_hunt_inside_a_good_segment_is_a_soft_span() -> anyhow::Result<()> {
        let s: Vec<Sample> = (0..36)
            .map(|i| Sample {
                sharp: if (12..17).contains(&i) { 0.2 } else { 1.0 },
                ..good(i)
            })
            .collect();
        let m = marks(&s, 1.0, &Thresholds::default());
        let spans = within(&m, &whole(&s), &[]);
        assert_eq!(spans.len(), 1, "one span: {spans:?}");
        let g = &spans[0];
        assert!(g.flag == "soft", "the hunt is soft: {g:?}");
        assert!(
            (g.start - 2.0).abs() < 0.2 && (g.end - 2.83).abs() < 0.2,
            "about 2.0-2.8 s: {g:?}"
        );
        assert!(g.value < 0.3, "worst focus kept: {g:?}");
        Ok(())
    }

    #[test]
    fn one_noisy_frame_is_not_a_span() -> anyhow::Result<()> {
        let s: Vec<Sample> = (0..36)
            .map(|i| Sample {
                sharp: if i == 10 { 0.1 } else { 1.0 },
                clipped: if i == 20 || i == 21 { 0.5 } else { 0.0 },
                ..good(i)
            })
            .collect();
        let m = marks(&s, 1.0, &Thresholds::default());
        let spans = within(&m, &whole(&s), &[]);
        assert!(spans.is_empty(), "too short to matter: {spans:?}");
        Ok(())
    }

    #[test]
    fn close_runs_merge_and_whole_segment_flags_are_not_repeated() -> anyhow::Result<()> {
        let s: Vec<Sample> = (0..36)
            .map(|i| Sample {
                clipped: if (6..9).contains(&i) || (10..13).contains(&i) {
                    0.3
                } else {
                    0.0
                },
                ..good(i)
            })
            .collect();
        let m = marks(&s, 1.0, &Thresholds::default());
        let spans = within(&m, &whole(&s), &[]);
        assert_eq!(spans.len(), 1, "a one-sample gap merges: {spans:?}");
        assert!(
            spans[0].flag == "blown" && (spans[0].end - spans[0].start - 1.17).abs() < 0.05,
            "one 1.17 s blown span: {spans:?}"
        );
        let flagged = within(&m, &whole(&s), &["blown".to_owned()]);
        assert!(flagged.is_empty(), "already flagged blown: {flagged:?}");
        Ok(())
    }

    #[test]
    fn a_short_whip_inside_a_static_segment_is_found() -> anyhow::Result<()> {
        let s: Vec<Sample> = (0..36)
            .map(|i| Sample {
                pan_x: if (18..21).contains(&i) { 2.0 } else { 0.0 },
                ..good(i)
            })
            .collect();
        let m = marks(&s, 1.0, &Thresholds::default());
        let spans = within(&m, &whole(&s), &[]);
        let whip = spans.iter().find(|g| g.flag == "whip");
        assert!(
            whip.is_some_and(|g| g.start >= 2.6 && g.end <= 3.7 && g.value > 1.0),
            "a whip around 3.0-3.5 s: {spans:?}"
        );
        Ok(())
    }

    #[test]
    fn spans_are_cut_to_their_segment() -> anyhow::Result<()> {
        let s: Vec<Sample> = (0..36)
            .map(|i| Sample {
                sharp: if (10..20).contains(&i) { 0.2 } else { 1.0 },
                ..good(i)
            })
            .collect();
        let m = marks(&s, 1.0, &Thresholds::default());
        let second = Segment {
            start: 2.5,
            end: 6.0,
            ..Segment::default()
        };
        let spans = within(&m, &second, &[]);
        assert!(
            spans.len() == 1 && spans[0].start >= 2.5 - 1e-3,
            "starts at the segment: {spans:?}"
        );
        Ok(())
    }

    /// A one-clip index (`A`, 120 s of good samples, one segment) whose
    /// segment carries a whip span at 59.5-60.0 s. Built through serde so
    /// fields added to the clip types later keep their defaults.
    fn whip_night() -> anyhow::Result<crate::index::Index> {
        let samples: Vec<Sample> = (0..720).map(good).collect();
        let span = BadSpan {
            start: 59.5,
            end: 60.0,
            flag: "whip".into(),
            value: 2.0,
        };
        let seg = Segment {
            start: 0.0,
            end: 120.0,
            lit: 0.4,
            lit_frac: 0.1,
            lit_p95: 0.8,
            sharp: 1.0,
            flame: 0.05,
            moving: 0.02,
            cx: 0.5,
            cy: 0.5,
            zoom_factor: 1.0,
            bad_spans: vec![span],
            ..Segment::default()
        };
        let clip: crate::analyze::ClipIndex = serde_json::from_value(serde_json::json!({
            "id": "A",
            "path": "A.mov",
            "stamp": [0, 0],
            "probe": {
                "duration": 120.0, "width": 1920, "height": 1080,
                "fps": [24000, 1001], "vcodec": "h264", "has_audio": false
            },
            "samples": samples,
            "segments": [seg.clone()],
            "hist": vec![0.0; 32],
        }))?;
        Ok(crate::index::Index {
            sharp_ref: 1.0,
            clips: vec![clip],
            segments: vec![crate::index::Scored {
                id: "A/1".into(),
                clip: "A".into(),
                seg,
                subject: "fire".into(),
                q: 0.9,
                ..crate::index::Scored::default()
            }],
            ..crate::index::Index::default()
        })
    }

    #[test]
    fn the_check_reports_a_shot_over_a_bad_span() -> anyhow::Result<()> {
        let index = whip_night()?;
        let map = crate::beats::MusicMap {
            bpm: 120.0,
            beats: (0..120).map(|i| i as f32 * 0.5).collect(),
            bars: (0..30)
                .map(|i| crate::beats::Bar {
                    t: i as f32 * 2.0,
                    rms: 1.0,
                    low: 1.0,
                })
                .collect(),
            ..crate::beats::MusicMap::default()
        };
        let plan_at = |t: f32| -> anyhow::Result<crate::plan::Plan> {
            Ok(toml::from_str(&format!(
                "name = \"t\"\naspect = \"16x9\"\nmusic = \"m.wav\"\nstart_bar = 0\ndrop_bar = 0\n\
                 card_bar = 1\nend_bar = 1\n[[shot]]\nclip = \"A\"\nin = {t}\nbeats = 4\n"
            ))?)
        };
        let people = crate::people::People::default();
        let over = crate::plan::check(&plan_at(59.0)?, &index, &map, &people);
        assert!(
            over.iter().any(|p| p.shot == 1
                && p.message.contains("bad span")
                && p.message.contains("whip")),
            "59-61 s crosses the whip: {over:?}"
        );
        let clear = crate::plan::check(&plan_at(62.0)?, &index, &map, &people);
        assert!(
            !clear.iter().any(|p| p.message.contains("bad span")),
            "62-64 s is clear: {clear:?}"
        );
        Ok(())
    }

    #[test]
    fn spans_survive_the_index_file_and_reach_the_chat_agent() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        whip_night()?.save(dir.path())?;
        let back = crate::index::Index::load(dir.path())?;
        assert_eq!(
            back.segments[0].seg.bad_spans.len(),
            1,
            "index.json keeps the span"
        );
        let shotlog = std::fs::read_to_string(dir.path().join("shotlog.md"))?;
        assert!(
            shotlog.contains("bad: whip 59.50-60.00 speed 2.0w/s"),
            "the shot log prints it: {shotlog}"
        );
        let out = crate::mcp::call(
            dir.path(),
            "segment_details",
            &serde_json::json!({"ids": ["A/1"]}),
            crate::mcp::Caller::Agent,
        )?;
        let text = out
            .first()
            .and_then(|c| c["text"].as_str())
            .unwrap_or_default()
            .to_owned();
        assert!(
            text.contains("bad spans") && text.contains("whip 59.50-60.00"),
            "segment_details shows it: {text}"
        );
        // An index written before bad spans existed still loads.
        let old = r#"{"start":0.0,"end":1.0,"camera":"static","after_cut":false,"luma":0.1,
            "p95":0.2,"clipped":0.0,"sharp":1.0,"flame":0.0,"led":0.0,"white":0.0,"moving":0.0,
            "cx":0.5,"cx_spread":0.0,"cy":0.5,"zoom_factor":1.0,"pan":0.0,"jitter":0.0}"#;
        let seg: Segment = serde_json::from_str(old)?;
        assert!(seg.bad_spans.is_empty(), "no spans in an old index");
        Ok(())
    }

    #[test]
    fn the_draft_shifts_its_window_off_a_bad_span() -> anyhow::Result<()> {
        let index = whip_night()?;
        let rules = crate::plan::Rules::default();
        let found = crate::draft::slot_candidates(&index, &rules, crate::plan::Aspect::Wide, 2.0);
        assert!(!found.is_empty(), "the clip still offers a window");
        let span = &index.segments[0].seg.bad_spans[0];
        assert!(
            found.iter().all(|c| !span.overlaps(c.t_in, c.t_in + 2.0)),
            "no window crosses 59.5-60.0: {:?}",
            found.iter().map(|c| c.t_in).collect::<Vec<_>>()
        );
        assert!(
            found.iter().any(|c| (c.t_in - 59.0).abs() < 1.2),
            "the centred window (59-61) moved just clear: {:?}",
            found.iter().map(|c| c.t_in).collect::<Vec<_>>()
        );
        Ok(())
    }

    #[test]
    fn a_window_dodges_a_span_or_gives_up() -> anyhow::Result<()> {
        let span = BadSpan {
            start: 4.0,
            end: 4.5,
            flag: "whip".into(),
            value: 2.0,
        };
        let spans = [&span];
        assert_eq!(
            dodge(&spans, 1.0, 2.0, 0.0, 8.0),
            Some(1.0),
            "a clear window stays"
        );
        let moved = dodge(&spans, 3.5, 2.0, 0.0, 8.0)
            .ok_or_else(|| anyhow::anyhow!("should find a clear window"))?;
        assert!(
            !span.overlaps(moved, moved + 2.0) && (moved - 3.5).abs() < 1.3,
            "moved just clear: {moved}"
        );
        assert_eq!(
            dodge(&spans, 3.5, 2.0, 3.0, 3.8),
            None,
            "nowhere to go inside 3.0-3.8"
        );
        Ok(())
    }
}

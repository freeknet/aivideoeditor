// ============================================================================
// Module:       analyze::cheer
// Description:  Crowd reaction from a clip's own audio: a 10 Hz loudness
//               envelope in three bands (cached per clip stamp) and the
//               cheers it shows, each with an onset time and a strength.
//
// Dependencies: crate::media, serde
// ============================================================================

//! Cheers: where the crowd reacts.
//!
//! A trick that lands gets a whoop. On the camera's microphone that is a
//! sustained rise in loudness over what came just before, and the rise is
//! *treble-heavy*: voices and claps fill 2-8 kHz, while the music at these
//! events (loud, compressed, steady) mostly lives in the bass and does not
//! move. The audio is decoded to 16 kHz mono and measured in 100 ms windows
//! ([`Envelope`]): full-band, low (< 250 Hz) and high (> 2 kHz) level in dB,
//! plus the crest factor. The envelope is cached under `<work>/audio/`
//! keyed by the clip's stamp, so a cached clip gains cheers without its
//! video being analysed again, and re-tuning [`detect`] costs no decoding.
//!
//! Each band is first smoothed by a median over [`SMOOTH`] windows: in
//! 100 ms windows a hi-hat or snare hit swings the treble by 15 dB, but it
//! fills one window, while a crowd fills them all. A window then *rises*
//! when its high band is at least [`RISE_DB`] over the rolling baseline
//! (the median of the previous [`BASE_WIN`] windows, ending [`BASE_GAP`]
//! windows back), its full band at least [`FULL_RISE_DB`] over its own
//! baseline, the high band rose at least [`TILT_DB`] more than the low
//! band (a song getting louder lifts the bass as much as the treble; a
//! crowd does not), and the low band did not fall more than
//! [`LOW_FALL_DB`] (a breakdown that drops the bass for a bright pad or a
//! riser is the song, not the crowd; a cheer adds, it never takes the bass
//! away). On the night these were tuned on (20 minutes of camera audio
//! under loud, steady house music), these leave only reactions with a
//! clear treble jump on top of the full band. A cheer is a run of
//! rising windows (one-window gaps bridged) at least [`MIN_RUN`] long
//! (0.4 s): a clap, a bump on the mic or a kick drum is shorter. Its onset
//! is refined to where the high band crossed halfway up the rise, and its
//! strength (0..1) grows with the size of the rise and how long it lasts.

use std::path::Path;

use anyhow::Result;

use crate::media;

/// Version of the envelope measurement; a cached envelope from another
/// version is measured again.
pub(crate) const AUDIO_VERSION: u32 = 1;

/// Decode rate, Hz.
const RATE: u32 = 16_000;

/// Envelope rate, windows per second.
pub(crate) const HZ: f32 = 10.0;

/// Samples per envelope window (100 ms).
const HOP: usize = (RATE as usize) / 10;

/// Rolling-baseline length, windows (5 s).
const BASE_WIN: usize = 50;

/// How far back the baseline ends, windows (0.3 s), so a rise does not
/// lift its own baseline.
const BASE_GAP: usize = 3;

/// Least history before a window can rise, windows (1.5 s): the first
/// second of a clip is the camera's level control settling.
const MIN_HISTORY: usize = 15;

/// High-band rise over the baseline that counts, dB.
pub(crate) const RISE_DB: f32 = 6.0;

/// Full-band rise over its baseline that counts too, dB.
const FULL_RISE_DB: f32 = 2.0;

/// How much more the high band must rise than the low band, dB.
const TILT_DB: f32 = 3.0;

/// How far the low band may fall while the treble rises, dB.
const LOW_FALL_DB: f32 = 3.0;

/// Median-smoothing width, windows (0.5 s).
const SMOOTH: usize = 5;

/// Shortest cheer, windows (0.4 s).
const MIN_RUN: usize = 4;

/// Floor for silent windows, dB.
const FLOOR_DB: f32 = -100.0;

/// The cached loudness envelope of one clip's audio.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Envelope {
    /// [`AUDIO_VERSION`] it was measured with.
    pub(crate) version: u32,
    /// The clip's stamp (size, mtime) when measured.
    pub(crate) stamp: (u64, u64),
    /// Full-band level per window, dB re full scale.
    pub(crate) full: Vec<f32>,
    /// Level below 250 Hz, dB.
    pub(crate) low: Vec<f32>,
    /// Level above 2 kHz, dB.
    pub(crate) high: Vec<f32>,
    /// Crest factor (peak over RMS), dB.
    pub(crate) crest: Vec<f32>,
}

/// One crowd reaction.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Cheer {
    /// Onset, seconds in the clip.
    pub(crate) t: f32,
    /// How long the rise lasts, seconds.
    pub(crate) dur: f32,
    /// Mean high-band rise over the baseline, dB.
    pub(crate) rise: f32,
    /// Strength 0..1.
    pub(crate) strength: f32,
}

/// A second-order section (RBJ cookbook), direct form I.
#[derive(Clone, Copy)]
struct Biquad {
    /// Feed-forward coefficients, normalised.
    b: [f32; 3],
    /// Feedback coefficients, normalised (a1, a2).
    a: [f32; 2],
    /// Last two inputs.
    x: [f32; 2],
    /// Last two outputs.
    y: [f32; 2],
}

impl Biquad {
    /// A Butterworth low-pass (`high` false) or high-pass at `f` Hz.
    fn new(f: f32, high: bool) -> Self {
        let w = std::f32::consts::TAU * f / RATE as f32;
        let alpha = w.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let c = w.cos();
        let a0 = 1.0 + alpha;
        let b = if high {
            [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0]
        } else {
            [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0]
        };
        Self {
            b: [b[0] / a0, b[1] / a0, b[2] / a0],
            a: [-2.0 * c / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    /// Filters one sample.
    fn step(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// Mean square to dB.
fn db(ms: f32) -> f32 {
    if ms > 1e-10 {
        10.0 * ms.log10()
    } else {
        FLOOR_DB
    }
}

/// Rounds to 0.1 dB, to keep the cache small.
fn tenth(x: f32) -> f32 {
    (x * 10.0).round() / 10.0
}

/// The envelope of 16 kHz mono `pcm` (version and stamp left default).
pub(crate) fn envelope(pcm: &[f32]) -> Envelope {
    // Fourth-order (two cascaded sections) for both bands.
    let mut lo = [Biquad::new(250.0, false); 2];
    let mut hi = [Biquad::new(2000.0, true); 2];
    let mut env = Envelope::default();
    for win in pcm.chunks(HOP) {
        if win.len() < HOP / 2 {
            break;
        }
        let (mut sf, mut sl, mut sh, mut peak) = (0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
        for &x in win {
            let l0 = lo[0].step(x);
            let l = lo[1].step(l0);
            let h0 = hi[0].step(x);
            let h = hi[1].step(h0);
            sf += x * x;
            sl += l * l;
            sh += h * h;
            peak = peak.max(x.abs());
        }
        let n = win.len() as f32;
        let full = db(sf / n);
        env.full.push(tenth(full));
        env.low.push(tenth(db(sl / n)));
        env.high.push(tenth(db(sh / n)));
        env.crest.push(tenth(db(peak * peak) - full));
    }
    env
}

/// Decodes `path`'s audio and measures its envelope. A clip with no audio
/// track gets an empty envelope (and so no cheers), not an error.
pub(crate) fn measure(path: &Path, stamp: (u64, u64), has_audio: bool) -> Result<Envelope> {
    let mut env = if has_audio {
        envelope(&media::audio_mono(path, RATE)?)
    } else {
        Envelope::default()
    };
    env.version = AUDIO_VERSION;
    env.stamp = stamp;
    Ok(env)
}

/// Loads the cached envelope of `path` from `dir`, or measures and caches
/// it when missing, stale or unreadable.
pub(crate) fn cached(
    dir: &Path,
    id: &str,
    path: &Path,
    has_audio: bool,
    force: bool,
) -> Result<Envelope> {
    let stamp = crate::index::stamp(path)?;
    let file = dir.join(format!("{id}.json"));
    if !force
        && let Ok(text) = std::fs::read_to_string(&file)
        && let Ok(env) = serde_json::from_str::<Envelope>(&text)
        && env.stamp == stamp
        && env.version == AUDIO_VERSION
    {
        return Ok(env);
    }
    let env = measure(path, stamp, has_audio)?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{id}.json.tmp"));
    std::fs::write(&tmp, serde_json::to_string(&env)?)?;
    std::fs::rename(&tmp, &file)?;
    Ok(env)
}

/// Median of `xs` (0 when empty).
fn median(xs: &[f32]) -> f32 {
    let mut v = xs.to_vec();
    v.sort_by(f32::total_cmp);
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

/// Each window replaced by the median of the [`SMOOTH`] windows around it:
/// a hi-hat or a snare fills one 100 ms window, a crowd fills them all.
fn smooth(band: &[f32]) -> Vec<f32> {
    let half = SMOOTH / 2;
    (0..band.len())
        .map(|i| median(&band[i.saturating_sub(half)..(i + half + 1).min(band.len())]))
        .collect()
}

/// The rolling baseline of `band` at window `i`, when there is history.
fn baseline(band: &[f32], i: usize) -> Option<f32> {
    let end = i.checked_sub(BASE_GAP)?;
    let start = end.saturating_sub(BASE_WIN);
    (end - start >= MIN_HISTORY).then(|| median(&band[start..end]))
}

/// The three bands, smoothed.
struct Bands {
    /// Full band.
    full: Vec<f32>,
    /// Below 250 Hz.
    low: Vec<f32>,
    /// Above 2 kHz.
    high: Vec<f32>,
}

impl Bands {
    /// The high-band rise of window `i`, when it counts as rising.
    fn rising(&self, i: usize) -> Option<f32> {
        let h = self.high[i] - baseline(&self.high, i)?;
        let f = self.full[i] - baseline(&self.full, i)?;
        let l = self.low[i] - baseline(&self.low, i)?;
        (h >= RISE_DB && f >= FULL_RISE_DB && h - l >= TILT_DB && l >= -LOW_FALL_DB).then_some(h)
    }
}

/// Strength 0..1 of a cheer rising `rise` dB for `dur` seconds: from 0.15
/// at the threshold, half again from the size (6 dB is nothing, 18 dB is
/// all) and the rest from the length (0.4 s is nothing, 2 s is all).
fn strength(rise: f32, dur: f32) -> f32 {
    let size = ((rise - RISE_DB) / 12.0).clamp(0.0, 1.0);
    let length = ((dur - 0.4) / 1.6).clamp(0.0, 1.0);
    let s = 0.15 + 0.5 * size + 0.35 * length;
    (s.min(1.0) * 100.0).round() / 100.0
}

/// The cheers in `env`, in time order.
pub(crate) fn detect(env: &Envelope) -> Vec<Cheer> {
    let n = env.full.len().min(env.low.len()).min(env.high.len());
    let bands = Bands {
        full: smooth(&env.full[..n]),
        low: smooth(&env.low[..n]),
        high: smooth(&env.high[..n]),
    };
    let rise: Vec<Option<f32>> = (0..n).map(|i| bands.rising(i)).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if rise[i].is_none() {
            i += 1;
            continue;
        }
        // Extend the run, bridging single-window gaps.
        let start = i;
        let mut end = i + 1;
        while end < n && (rise[end].is_some() || (end + 1 < n && rise[end + 1].is_some())) {
            end += 1;
        }
        if end - start >= MIN_RUN {
            let rises: Vec<f32> = rise[start..end].iter().flatten().copied().collect();
            let mean = rises.iter().sum::<f32>() / rises.len().max(1) as f32;
            let dur = (end - start) as f32 / HZ;
            out.push(Cheer {
                t: (onset(&env.high[..n], &bands.high, start, mean) * 1000.0).round() / 1000.0,
                dur,
                rise: tenth(mean),
                strength: strength(mean, dur),
            });
        }
        i = end;
    }
    out
}

/// Gives each segment the cheers whose onset falls inside it.
pub(crate) fn assign(segments: &mut [super::segments::Segment], cheers: &[Cheer]) {
    for seg in segments.iter_mut() {
        seg.cheers = cheers
            .iter()
            .filter(|c| c.t >= seg.start && c.t < seg.end)
            .copied()
            .collect();
    }
}

/// Seconds where the raw high band `high` first crossed halfway up a rise
/// of `rise` dB whose first (smoothed) rising window is `start`. The
/// median smoothing can move an edge by up to half its width, so the
/// crossing is looked for that far either side.
fn onset(high: &[f32], smoothed: &[f32], start: usize, rise: f32) -> f32 {
    let centre = |k: usize| (k as f32 + 0.5) / HZ;
    let Some(base) = baseline(smoothed, start) else {
        return centre(start);
    };
    let half = base + 0.5 * rise;
    let reach = SMOOTH / 2 + 1;
    let lo = start.saturating_sub(reach).max(1);
    let hi = (start + reach).min(high.len().saturating_sub(1));
    for k in lo..=hi {
        let (before, after) = (high[k - 1], high[k]);
        if before < half && after >= half {
            let span = after - before;
            let f = if span > f32::EPSILON {
                (half - before) / span
            } else {
                1.0
            };
            // Window k-1's centre plus the fraction of the step to k.
            return centre(k - 1) + f / HZ;
        }
    }
    centre(start)
}

#[cfg(test)]
mod tests {
    use super::{Envelope, HOP, RATE, detect, envelope};

    /// An envelope from per-window (full, low, high) levels.
    fn env(levels: &[(f32, f32, f32)]) -> Envelope {
        Envelope {
            full: levels.iter().map(|l| l.0).collect(),
            low: levels.iter().map(|l| l.1).collect(),
            high: levels.iter().map(|l| l.2).collect(),
            crest: vec![10.0; levels.len()],
            ..Envelope::default()
        }
    }

    /// Twenty seconds of loud steady music with a little wobble.
    fn music() -> Vec<(f32, f32, f32)> {
        (0..200)
            .map(|i| {
                let w = if i % 5 == 0 { 1.5 } else { 0.0 };
                (-14.0 + w, -16.0 + w, -30.0 + w)
            })
            .collect()
    }

    #[test]
    fn steady_music_has_no_cheers() -> anyhow::Result<()> {
        let c = detect(&env(&music()));
        assert!(c.is_empty(), "steady music is not a crowd: {c:?}");
        Ok(())
    }

    #[test]
    fn a_treble_rise_is_one_cheer_at_its_onset() -> anyhow::Result<()> {
        let mut l = music();
        // A whoop from 10.0 s to 11.5 s: treble up 12 dB, bass unmoved.
        for x in &mut l[100..115] {
            x.0 += 4.0;
            x.2 += 12.0;
        }
        let c = detect(&env(&l));
        assert_eq!(c.len(), 1, "one cheer: {c:?}");
        assert!((c[0].t - 10.0).abs() < 0.1, "onset at 10 s: {}", c[0].t);
        assert!((c[0].dur - 1.5).abs() < 0.15, "lasts 1.5 s: {}", c[0].dur);
        assert!(
            c[0].strength > 0.4 && c[0].strength <= 1.0,
            "a clear cheer is strong: {}",
            c[0].strength
        );
        Ok(())
    }

    #[test]
    fn the_song_getting_louder_is_not_a_cheer() -> anyhow::Result<()> {
        let mut l = music();
        // The drop: everything 8 dB louder from 10 s on, bass included.
        for x in &mut l[100..] {
            x.0 += 8.0;
            x.1 += 8.0;
            x.2 += 8.0;
        }
        let c = detect(&env(&l));
        assert!(c.is_empty(), "a louder song lifts the bass too: {c:?}");
        Ok(())
    }

    #[test]
    fn hi_hats_are_not_a_cheer() -> anyhow::Result<()> {
        let mut l = music();
        // From 10 s on, eighth-note hats (about every third window at
        // 128 BPM), 15 dB of treble each.
        for (i, x) in l.iter_mut().enumerate().skip(100) {
            if i % 3 == 0 {
                x.0 += 3.0;
                x.2 += 15.0;
            }
        }
        let c = detect(&env(&l));
        assert!(c.is_empty(), "single-window hits are the drums: {c:?}");
        Ok(())
    }

    #[test]
    fn a_breakdown_that_drops_the_bass_is_not_a_cheer() -> anyhow::Result<()> {
        let mut l = music();
        // Bass out, a bright riser in, the full band about level.
        for x in &mut l[100..130] {
            x.0 += 2.5;
            x.1 -= 12.0;
            x.2 += 12.0;
        }
        let c = detect(&env(&l));
        assert!(c.is_empty(), "the song took the bass away: {c:?}");
        Ok(())
    }

    #[test]
    fn a_short_clap_is_not_a_cheer() -> anyhow::Result<()> {
        let mut l = music();
        for x in &mut l[100..103] {
            x.0 += 6.0;
            x.2 += 15.0;
        }
        let c = detect(&env(&l));
        assert!(c.is_empty(), "0.3 s is a clap or a bump: {c:?}");
        Ok(())
    }

    #[test]
    fn the_envelope_splits_bass_from_treble() -> anyhow::Result<()> {
        let tone = |f: f32| -> Vec<f32> {
            (0..RATE as usize)
                .map(|n| 0.5 * (std::f32::consts::TAU * f * n as f32 / RATE as f32).sin())
                .collect()
        };
        let bass = envelope(&tone(80.0));
        let hiss = envelope(&tone(4000.0));
        assert_eq!(bass.full.len(), RATE as usize / HOP, "ten windows a second");
        let (b, h) = (bass.full.len() - 1, hiss.full.len() - 1);
        assert!(
            bass.low[b] > bass.high[b] + 30.0,
            "80 Hz is bass: low {} high {}",
            bass.low[b],
            bass.high[b]
        );
        assert!(
            hiss.high[h] > hiss.low[h] + 30.0,
            "4 kHz is treble: low {} high {}",
            hiss.low[h],
            hiss.high[h]
        );
        Ok(())
    }
}

// ============================================================================
// Module:       beats
// Description:  Music analysis: onset envelope, tempo, beat tracking, bars,
//               per-bar energy, drops and sections, plus the user's grid
//               corrections, applied on load.
//
// Dependencies: crate::media, rustfft, serde_json
// ============================================================================

//! The music map an edit is cut to.
//!
//! - **Onsets:** spectral flux of the log-magnitude STFT (hop 512 at
//!   22.05 kHz, about 23 ms).
//! - **Tempo:** the onset envelope's autocorrelation, searched over
//!   90-180 BPM with a mild preference for 120-150 (bass music sits at
//!   140; halftime feels 70 but the grid is 140).
//! - **Beats:** dynamic-programming beat tracking (Ellis 2007): each beat
//!   is an onset peak about one period after the previous one.
//! - **Bars:** beats grouped in fours, with the phase that puts the most
//!   low-frequency energy (kicks) on the downbeat.
//! - **Drops and sections:** see [`structure`]: the low-end jump into a bar,
//!   weighted by the build before it; sections (intro / build / drop /
//!   break / outro) from per-bar energy on 4-bar phrase lines.
//! - **Envelope:** loudness, low end and onset strength at ~93 ms, kept in
//!   the map so a corrected grid can re-measure its bars without decoding
//!   the track again.
//!
//! ## The user's corrections
//!
//! Detectors miss the "one" and sometimes the tempo, so `music.json` also
//! carries what the user fixed on the Music page: `grid_offset` (seconds),
//! `bpm_override`, `downbeat_shift` (beats), `drops_user` (bars marked or
//! unmarked), `sections_user` (renamed sections) and `ranges` (the song
//! range each deliverable uses). The file keeps the analysis as analysed;
//! [`MusicMap::load`] applies the corrections, so every consumer (planner,
//! renderer, timeline) sees the corrected `beats`, `bars`, `drops` and
//! `sections` without knowing they were corrected, and [`MusicMap::save`]
//! writes the analysis back untouched beside the corrections. Re-running
//! `ave beats` on the same track keeps them.
//!
//! ## Why not the `beat-this` crate (yet)
//!
//! `beat-this` 1.1 (MIT, pure-Rust `rten` inference) is the recommended
//! upgrade in `docs/research/03-techniques.md` §3, and its code would build
//! here, but the published crate ships **no model weights**: the mel front
//! end and the 10 MB small model (or the 83 MB full one) live only in its
//! git repository and releases. Using it means vendoring or downloading
//! those files and configuring their path, plus `rten`, `rubato` and
//! `symphonia` in the build. Until that is decided, the tracker sits behind
//! [`analyze`], so swapping in its beats and downbeats touches only the
//! beat-finding half of that function, and the user's grid fixes on the
//! Music page cover the cases the DSP gets wrong.

pub(crate) mod structure;

use std::path::Path;

use anyhow::{Context, Result, ensure};
use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use crate::media;

use structure::Feat;

/// Analysis sample rate.
const RATE: u32 = 22_050;
/// STFT window.
const WIN: usize = 2048;
/// STFT hop.
const HOP: usize = 512;
/// STFT frames per envelope value.
const ENV_GROUP: usize = 4;

/// One bar of the track.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Bar {
    /// Start time, seconds.
    pub(crate) t: f32,
    /// RMS loudness over the bar.
    pub(crate) rms: f32,
    /// Low-end (below ~150 Hz) energy, relative to the track's loudest bar.
    pub(crate) low: f32,
}

/// A drop candidate.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Drop {
    /// Bar index.
    pub(crate) bar: usize,
    /// Time, seconds.
    pub(crate) t: f32,
    /// How big the jump is, weighted by the build before it (higher is
    /// stronger).
    pub(crate) score: f32,
    /// Marked by the user rather than detected.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) user: bool,
}

/// A part of the song: intro, build, drop, break or outro.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Section {
    /// Its name (see [`structure::NAMES`]; the user may rename it).
    pub(crate) name: String,
    /// First bar.
    pub(crate) from_bar: usize,
    /// Last bar (inclusive).
    pub(crate) to_bar: usize,
}

/// A bar the user marked as a drop (`on`) or told is not one.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct DropMark {
    /// Bar index (on the corrected grid).
    pub(crate) bar: usize,
    /// A drop (true) or not (false).
    pub(crate) on: bool,
}

/// The part of the song a deliverable uses, e.g. `reel-30` → bars 13-31.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct SongRange {
    /// Deliverable name (`reel-15`, `feed-30`, `wide-60`, `recap`, …).
    pub(crate) name: String,
    /// First bar.
    pub(crate) from_bar: usize,
    /// Last bar (inclusive).
    pub(crate) to_bar: usize,
}

/// Loudness, low end and onset strength over the track, at `hop` seconds.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Envelope {
    /// Time of value 0's centre, seconds.
    pub(crate) t0: f32,
    /// Seconds per value.
    pub(crate) hop: f32,
    /// RMS loudness.
    pub(crate) rms: Vec<f32>,
    /// Low-end energy, 0..1 over the track.
    pub(crate) low: Vec<f32>,
    /// Onset strength, 0..1 over the track.
    pub(crate) onset: Vec<f32>,
}

impl Envelope {
    /// Index range of the values whose centres fall in `[a, b)` (at least
    /// one value when the envelope is not empty).
    fn span(&self, a: f32, b: f32) -> (usize, usize) {
        let n = self.rms.len();
        if n == 0 || self.hop <= 0.0 {
            return (0, 0);
        }
        let at = |t: f32| (((t - self.t0) / self.hop).max(0.0).round() as usize).min(n);
        let (ia, ib) = (at(a), at(b));
        if ib > ia {
            (ia, ib)
        } else {
            let i = ia.min(n - 1);
            (i, i + 1)
        }
    }

    /// RMS, low end and onset strength over `[a, b)`.
    fn stats(&self, a: f32, b: f32) -> (f32, f32, f32) {
        let (ia, ib) = self.span(a, b);
        if ib <= ia {
            return (0.0, 0.0, 0.0);
        }
        let k = (ib - ia) as f32;
        let rms = (self.rms[ia..ib].iter().map(|x| x * x).sum::<f32>() / k).sqrt();
        let pick = |v: &[f32]| v.get(ia..ib).map_or(0.0, |s| s.iter().sum::<f32>() / k);
        (rms, pick(&self.low), pick(&self.onset))
    }
}

/// The analysis as analysed, kept so corrections can be re-applied and
/// saved without compounding.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Raw {
    bpm: f32,
    beats: Vec<f32>,
    bars: Vec<Bar>,
    drops: Vec<Drop>,
    sections: Vec<Section>,
}

/// The whole map.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct MusicMap {
    /// Source file.
    pub(crate) path: String,
    /// Duration, seconds.
    pub(crate) duration: f32,
    /// Tempo.
    pub(crate) bpm: f32,
    /// Beat times, seconds.
    pub(crate) beats: Vec<f32>,
    /// Bars (four beats each).
    pub(crate) bars: Vec<Bar>,
    /// Drop candidates: the user's marks first, then detected ones
    /// strongest first.
    pub(crate) drops: Vec<Drop>,
    /// Song sections in order, covering every bar.
    #[serde(default)]
    pub(crate) sections: Vec<Section>,
    /// Loudness / low end / onsets over time (absent in older maps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) envelope: Option<Envelope>,
    /// User: seconds added to every beat.
    #[serde(default)]
    pub(crate) grid_offset: f32,
    /// User: the tempo, when detection got it wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) bpm_override: Option<f32>,
    /// User: beats to move the downbeat by (positive is later).
    #[serde(default)]
    pub(crate) downbeat_shift: i32,
    /// User: bars marked as drops, or detected drops dismissed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) drops_user: Vec<DropMark>,
    /// User: sections as the user named them (replaces the detected ones).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) sections_user: Vec<Section>,
    /// User: the song range each deliverable uses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) ranges: Vec<SongRange>,
    /// The analysis before corrections (set by [`MusicMap::apply`]).
    #[serde(skip)]
    pub(crate) raw: Option<Box<Raw>>,
}

impl MusicMap {
    /// Loads `<work>/music.json` with the user's corrections applied.
    pub(crate) fn load(work: &Path) -> Result<Self> {
        let path = work.join("music.json");
        let text = std::fs::read_to_string(&path).map_err(|e| {
            anyhow::anyhow!("reading {} (run `ave beats` first): {e}", path.display())
        })?;
        let mut map: Self =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        map.apply();
        Ok(map)
    }

    /// Writes `<work>/music.json`: the analysis as analysed plus the
    /// corrections, through a temp file and a rename.
    pub(crate) fn save(&self, work: &Path) -> Result<()> {
        let mut out = self.clone();
        if let Some(raw) = out.raw.take() {
            out.bpm = raw.bpm;
            out.beats = raw.beats;
            out.bars = raw.bars;
            out.drops = raw.drops;
            out.sections = raw.sections;
        }
        std::fs::create_dir_all(work)?;
        let path = work.join("music.json");
        let tmp = work.join("music.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&out)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Time of beat `k` counted from bar `bar`'s downbeat, extrapolating
    /// past the last detected beat at the track tempo.
    pub(crate) fn beat_time(&self, bar: usize, k: usize) -> f32 {
        let period = 60.0 / self.bpm;
        let first = bar * 4;
        let i = first + k;
        match self.beats.get(i) {
            Some(t) => *t,
            None => {
                let last = self.beats.len().saturating_sub(1);
                self.beats.last().copied().unwrap_or(0.0) + (i - last) as f32 * period
            }
        }
    }

    /// Seconds per beat.
    pub(crate) fn period(&self) -> f32 {
        60.0 / self.bpm.max(1.0)
    }

    /// When bar `i` ends (the next bar's start, or four beats on).
    pub(crate) fn bar_end(&self, i: usize) -> f32 {
        match (self.bars.get(i + 1), self.bars.get(i)) {
            (Some(next), _) => next.t,
            (None, Some(bar)) => (bar.t + 4.0 * self.period()).min(self.duration.max(bar.t)),
            (None, None) => self.duration,
        }
    }

    /// The bar playing at `t`, if any.
    pub(crate) fn bar_at(&self, t: f32) -> Option<usize> {
        let first = self.bars.first()?;
        if t < first.t || t >= self.bar_end(self.bars.len() - 1) {
            return None;
        }
        Some(self.bars.partition_point(|b| b.t <= t).saturating_sub(1))
    }

    /// The section bar `bar` is in.
    pub(crate) fn section_at(&self, bar: usize) -> Option<usize> {
        self.sections
            .iter()
            .position(|s| bar >= s.from_bar && bar <= s.to_bar)
    }

    /// Whether the grid is as detected (no offset, tempo or downbeat fix).
    pub(crate) fn grid_is_detected(&self) -> bool {
        self.grid_offset.abs() < 1e-6 && self.bpm_override.is_none() && self.downbeat_shift == 0
    }

    /// The tempo as detected.
    pub(crate) fn detected_bpm(&self) -> f32 {
        self.raw.as_ref().map_or(self.bpm, |r| r.bpm)
    }

    /// Re-derives `bpm`, `beats`, `bars`, `drops` and `sections` from the
    /// analysis and the corrections. Idempotent; call after changing any
    /// correction field.
    pub(crate) fn apply(&mut self) {
        let raw = match &self.raw {
            Some(raw) => (**raw).clone(),
            None => {
                let raw = Raw {
                    bpm: self.bpm,
                    beats: self.beats.clone(),
                    bars: self.bars.clone(),
                    drops: self.drops.clone(),
                    sections: self.sections.clone(),
                };
                self.raw = Some(Box::new(raw.clone()));
                raw
            }
        };
        let detected = self.grid_is_detected();
        let (beats, bpm, bars) = if detected {
            (raw.beats, raw.bpm, raw.bars.clone())
        } else {
            let (beats, bpm) = corrected_beats(
                &raw.beats,
                raw.bpm,
                self.duration,
                self.bpm_override,
                self.downbeat_shift,
                self.grid_offset,
            );
            let bars = measure_bars(
                &beats,
                bpm,
                self.duration,
                self.envelope.as_ref(),
                &raw.bars,
            );
            (beats, bpm, bars)
        };
        self.beats = beats;
        self.bpm = bpm;
        self.bars = bars;
        let feats = self.features();
        let bar_t: Vec<f32> = self.bars.iter().map(|b| b.t).collect();
        let mut drops = if detected {
            raw.drops
        } else {
            structure::drops(&feats, &bar_t)
        };
        drops
            .retain(|d| d.bar < self.bars.len() && !self.drops_user.iter().any(|m| m.bar == d.bar));
        let mut user: Vec<Drop> = self
            .drops_user
            .iter()
            .filter(|m| m.on)
            .filter_map(|m| {
                let bar = self.bars.get(m.bar)?;
                Some(Drop {
                    bar: m.bar,
                    t: bar.t,
                    score: structure::drop_score(&feats, m.bar).unwrap_or(0.0),
                    user: true,
                })
            })
            .collect();
        user.append(&mut drops);
        self.drops = user;
        self.sections = if self.sections_user.is_empty() {
            structure::sections(&feats, &self.drops)
        } else {
            let last = self.bars.len().saturating_sub(1);
            self.sections_user
                .iter()
                .filter(|s| s.from_bar <= last)
                .map(|s| Section {
                    to_bar: s.to_bar.min(last),
                    ..s.clone()
                })
                .collect()
        };
    }

    /// Per-bar energy, normalised over the track.
    fn features(&self) -> Vec<Feat> {
        let max_rms = self.bars.iter().map(|b| b.rms).fold(1e-12_f32, f32::max);
        let onset: Vec<f32> = match &self.envelope {
            Some(env) if !env.onset.is_empty() => (0..self.bars.len())
                .map(|i| env.stats(self.bars[i].t, self.bar_end(i)).2)
                .collect(),
            Some(_) | None => self.bars.iter().map(|b| b.rms).collect(),
        };
        let max_onset = onset.iter().copied().fold(1e-12_f32, f32::max);
        self.bars
            .iter()
            .zip(&onset)
            .map(|(b, o)| Feat {
                rms: b.rms / max_rms,
                low: b.low,
                onset: o / max_onset,
            })
            .collect()
    }

    /// Moves the downbeat by `beats` (positive is later).
    pub(crate) fn shift_downbeat(&mut self, beats: i32) {
        self.downbeat_shift += beats;
        self.apply();
    }

    /// Nudges the whole grid by `seconds`.
    pub(crate) fn nudge(&mut self, seconds: f32) {
        self.grid_offset = ((self.grid_offset + seconds) * 1000.0).round() / 1000.0;
        self.apply();
    }

    /// Sets the tempo by hand, or back to detected with `None`. Values
    /// outside 40-300 BPM are refused.
    pub(crate) fn set_bpm(&mut self, bpm: Option<f32>) {
        self.bpm_override = bpm.filter(|b| (40.0..=300.0).contains(b));
        self.apply();
    }

    /// Forgets the offset, tempo and downbeat corrections.
    pub(crate) fn reset_grid(&mut self) {
        self.grid_offset = 0.0;
        self.bpm_override = None;
        self.downbeat_shift = 0;
        self.apply();
    }

    /// Marks bar `bar` as a drop, or unmarks it if it is one.
    pub(crate) fn toggle_drop(&mut self, bar: usize) {
        let was = self.drops.iter().any(|d| d.bar == bar);
        self.drops_user.retain(|m| m.bar != bar);
        self.apply();
        let is = self.drops.iter().any(|d| d.bar == bar);
        if was == is {
            self.drops_user.push(DropMark { bar, on: !was });
            self.apply();
        }
    }

    /// Makes the drop at `bar` the one edits are built around.
    pub(crate) fn make_primary_drop(&mut self, bar: usize) {
        self.drops_user.retain(|m| m.bar != bar);
        self.drops_user.insert(0, DropMark { bar, on: true });
        self.apply();
    }

    /// Renames section `i` (the sections become the user's from then on).
    pub(crate) fn rename_section(&mut self, i: usize, name: &str) {
        let name = name.trim();
        if name.is_empty() || i >= self.sections.len() {
            return;
        }
        let mut sections = self.sections.clone();
        sections[i].name = name.to_owned();
        self.sections_user = sections;
        self.apply();
    }

    /// Goes back to the detected sections.
    pub(crate) fn reset_sections(&mut self) {
        self.sections_user.clear();
        self.apply();
    }

    /// Sets (or replaces) the song range called `name`.
    pub(crate) fn set_range(&mut self, name: &str, from_bar: usize, to_bar: usize) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let (from_bar, to_bar) = (from_bar.min(to_bar), from_bar.max(to_bar));
        let range = SongRange {
            name: name.to_owned(),
            from_bar,
            to_bar,
        };
        match self.ranges.iter_mut().find(|r| r.name == name) {
            Some(r) => *r = range,
            None => self.ranges.push(range),
        }
    }

    /// Removes the song range called `name`.
    pub(crate) fn remove_range(&mut self, name: &str) {
        self.ranges.retain(|r| r.name != name);
    }

    /// Copies the user's corrections from `old` when it maps the same track.
    pub(crate) fn carry_corrections(&mut self, old: &Self) {
        let norm = |p: &str| p.replace('\\', "/").to_lowercase();
        if norm(&old.path) != norm(&self.path) {
            return;
        }
        self.grid_offset = old.grid_offset;
        self.bpm_override = old.bpm_override;
        self.downbeat_shift = old.downbeat_shift;
        self.drops_user = old.drops_user.clone();
        self.sections_user = old.sections_user.clone();
        self.ranges = old.ranges.clone();
        self.apply();
    }
}

/// The beat grid after the user's tempo, downbeat and offset corrections,
/// and its tempo.
fn corrected_beats(
    beats: &[f32],
    bpm: f32,
    duration: f32,
    bpm_override: Option<f32>,
    shift: i32,
    offset: f32,
) -> (Vec<f32>, f32) {
    let bpm = bpm_override.unwrap_or(bpm).max(1.0);
    let period = 60.0 / bpm;
    let anchor = beats.first().copied().unwrap_or(0.0);
    let mut out: Vec<f32> = match bpm_override {
        Some(_) => (0..)
            .map(|k| anchor + k as f32 * period)
            .take_while(|t| *t < duration.max(anchor + period))
            .collect(),
        None => beats.to_vec(),
    };
    if shift >= 0 {
        let n = usize::try_from(shift).unwrap_or(0).min(out.len());
        out.drain(..n);
    } else {
        let n = shift.unsigned_abs() as usize;
        let first = out.first().copied().unwrap_or(0.0);
        if first - n as f32 * period >= -0.02 {
            let lead: Vec<f32> = (1..=n).rev().map(|k| first - k as f32 * period).collect();
            out.splice(0..0, lead);
        } else {
            // Not enough room before the first beat: the same phase, one
            // bar later.
            let n = usize::try_from(shift.rem_euclid(4))
                .unwrap_or(0)
                .min(out.len());
            out.drain(..n);
        }
    }
    for t in &mut out {
        *t = (*t + offset).max(0.0);
    }
    (out, bpm)
}

/// Bars on `beats` (fours), measured from the envelope, or from the
/// analysed bars when the map predates the envelope.
fn measure_bars(
    beats: &[f32],
    bpm: f32,
    duration: f32,
    env: Option<&Envelope>,
    analysed: &[Bar],
) -> Vec<Bar> {
    let period = 60.0 / bpm.max(1.0);
    let mut bars: Vec<Bar> = beats
        .chunks(4)
        .enumerate()
        .filter_map(|(k, chunk)| {
            let t = *chunk.first()?;
            let end = beats
                .get(4 * k + 4)
                .copied()
                .unwrap_or((t + 4.0 * period).min(duration.max(t)));
            Some((t, end))
        })
        .filter(|(t, end)| end > t)
        .map(|(t, end)| match env {
            Some(env) if !env.rms.is_empty() => {
                let (rms, low, _) = env.stats(t, end);
                Bar { t, rms, low }
            }
            Some(_) | None => {
                let mid = 0.5 * (t + end);
                let i = analysed.partition_point(|b| b.t <= mid).saturating_sub(1);
                analysed.get(i).map_or(
                    Bar {
                        t,
                        rms: 0.0,
                        low: 0.0,
                    },
                    |b| Bar {
                        t,
                        rms: b.rms,
                        low: b.low,
                    },
                )
            }
        })
        .collect();
    let max_low = bars.iter().map(|b| b.low).fold(1e-12_f32, f32::max);
    for b in &mut bars {
        b.low /= max_low;
    }
    bars
}

/// Spectral flux and low-band energy per STFT frame.
fn envelopes(samples: &[f32]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WIN);
    let window: Vec<f32> = (0..WIN)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / WIN as f32).cos())
        .collect();
    let low_bins = (150.0 * WIN as f32 / RATE as f32).ceil() as usize;
    let mut prev = vec![0.0_f32; WIN / 2];
    let (mut flux, mut low, mut rms) = (Vec::new(), Vec::new(), Vec::new());
    let mut buf = vec![Complex::new(0.0, 0.0); WIN];
    let mut start = 0;
    while start + WIN <= samples.len() {
        let frame = &samples[start..start + WIN];
        for (b, (s, w)) in buf.iter_mut().zip(frame.iter().zip(&window)) {
            *b = Complex::new(s * w, 0.0);
        }
        fft.process(&mut buf);
        let mut f = 0.0;
        let mut l = 0.0;
        for k in 1..WIN / 2 {
            let mag = buf[k].norm();
            let lm = (1.0 + 100.0 * mag).ln();
            f += (lm - prev[k]).max(0.0);
            prev[k] = lm;
            if k <= low_bins {
                l += mag * mag;
            }
        }
        flux.push(f);
        low.push(l);
        rms.push((frame.iter().map(|s| s * s).sum::<f32>() / WIN as f32).sqrt());
        start += HOP;
    }
    // Normalise and remove the local mean so the beat tracker sees peaks.
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len().max(1) as f32;
    let m = mean(&flux).max(1e-9);
    let flux: Vec<f32> = flux.iter().map(|f| f / m).collect();
    let smooth = crate::analyze::features::box_mean(&flux, flux.len(), 1, 8);
    let flux = flux
        .iter()
        .zip(&smooth)
        .map(|(f, s)| (f - s).max(0.0))
        .collect();
    (flux, low, rms)
}

/// The per-frame envelopes, grouped into the map's coarser [`Envelope`].
fn coarse_envelope(flux: &[f32], low: &[f32], rms: &[f32]) -> Envelope {
    let group =
        |v: &[f32], f: fn(&[f32]) -> f32| -> Vec<f32> { v.chunks(ENV_GROUP).map(f).collect() };
    let mean = |c: &[f32]| c.iter().sum::<f32>() / c.len().max(1) as f32;
    let rms_of = |c: &[f32]| (c.iter().map(|x| x * x).sum::<f32>() / c.len().max(1) as f32).sqrt();
    let normalise = |mut v: Vec<f32>| {
        let max = v.iter().copied().fold(1e-12_f32, f32::max);
        v.iter_mut()
            .for_each(|x| *x = (*x / max * 1e4).round() / 1e4);
        v
    };
    let rms = group(rms, rms_of)
        .into_iter()
        .map(|x| (x * 1e5).round() / 1e5)
        .collect();
    Envelope {
        t0: (WIN / 2 + (ENV_GROUP - 1) * HOP / 2) as f32 / RATE as f32,
        hop: (ENV_GROUP * HOP) as f32 / RATE as f32,
        rms,
        low: normalise(group(low, mean)),
        onset: normalise(group(flux, mean)),
    }
}

/// Tempo in frames per beat, from the onset envelope.
fn tempo(env: &[f32], fps: f32) -> f32 {
    let lag_of = |bpm: f32| 60.0 * fps / bpm;
    let (lo, hi) = (lag_of(180.0).floor() as usize, lag_of(90.0).ceil() as usize);
    let mut best = (0.0, lag_of(120.0));
    for lag in lo.max(1)..=hi {
        let mut ac = 0.0;
        for i in lag..env.len() {
            ac += env[i] * env[i - lag];
        }
        ac /= (env.len() - lag).max(1) as f32;
        let bpm = 60.0 * fps / lag as f32;
        let prior = (-0.5 * ((bpm / 135.0).log2() / 0.6).powi(2)).exp();
        let score = ac * prior;
        if score > best.0 {
            best = (score, lag as f32);
        }
    }
    // Refine with a parabola through the neighbours.
    let lag = best.1 as usize;
    let ac = |l: usize| -> f32 {
        if l == 0 || l >= env.len() {
            return 0.0;
        }
        (l..env.len()).map(|i| env[i] * env[i - l]).sum::<f32>() / (env.len() - l) as f32
    };
    let (a, b, c) = (ac(lag.saturating_sub(1)), ac(lag), ac(lag + 1));
    let denom = a - 2.0 * b + c;
    if denom.abs() > 1e-9 {
        lag as f32 + 0.5 * (a - c) / denom
    } else {
        lag as f32
    }
}

/// Dynamic-programming beat tracker; returns beat frame indices.
fn track(env: &[f32], period: f32) -> Vec<usize> {
    const TIGHTNESS: f32 = 100.0;
    let n = env.len();
    let mut score = vec![0.0_f32; n];
    let mut back = vec![usize::MAX; n];
    for t in 0..n {
        let lo = (t as f32 - 2.0 * period).max(0.0) as usize;
        let hi = (t as f32 - 0.5 * period).max(0.0) as usize;
        let mut best = (0.0_f32, usize::MAX);
        for (p, prev) in score.iter().enumerate().take(hi.min(t)).skip(lo) {
            let d = ((t - p) as f32 / period).ln();
            let s = prev - TIGHTNESS * d * d;
            if best.1 == usize::MAX || s > best.0 {
                best = (s, p);
            }
        }
        score[t] = env[t] + if best.1 == usize::MAX { 0.0 } else { best.0 };
        back[t] = best.1;
    }
    // Start from the best-scoring frame in the last period.
    let tail = n.saturating_sub(period.ceil() as usize);
    let mut t = (tail..n)
        .max_by(|&a, &b| score[a].total_cmp(&score[b]))
        .unwrap_or(0);
    let mut beats = Vec::new();
    while t != usize::MAX {
        beats.push(t);
        t = back[t];
    }
    beats.reverse();
    beats
}

/// The shift (seconds, within ±0.25) that puts the beat grid on the kicks.
///
/// Spectral flux peaks a little after an onset, by an amount that depends
/// on the window and the sound, so the tracked grid can sit late. The fix
/// is measured rather than assumed: a low-passed (under ~120 Hz) envelope
/// at 128-sample resolution, its rises scored at every beat for each
/// candidate shift, the best shift kept.
fn kick_offset(samples: &[f32], beats: &[f32]) -> f32 {
    const ENV_HOP: usize = 128;
    // Two one-pole low-passes at ~120 Hz.
    let a = (-2.0 * std::f32::consts::PI * 120.0 / RATE as f32).exp();
    let (mut y1, mut y2) = (0.0_f32, 0.0_f32);
    let mut env = Vec::with_capacity(samples.len() / ENV_HOP + 1);
    let mut acc = 0.0;
    for (i, s) in samples.iter().enumerate() {
        y1 = (1.0 - a) * s + a * y1;
        y2 = (1.0 - a) * y1 + a * y2;
        acc += y2 * y2;
        if (i + 1) % ENV_HOP == 0 {
            env.push((acc / ENV_HOP as f32).sqrt());
            acc = 0.0;
        }
    }
    let rise: Vec<f32> = env
        .iter()
        .enumerate()
        .map(|(i, v)| {
            if i == 0 {
                0.0
            } else {
                (v - env[i - 1]).max(0.0)
            }
        })
        .collect();
    let step = ENV_HOP as f32 / RATE as f32;
    let strength = |t: f32| -> f32 {
        let c = (t / step).round() as i64;
        (c - 3..=c + 3)
            .filter_map(|k| usize::try_from(k).ok().and_then(|k| rise.get(k)))
            .fold(0.0_f32, |m, v| m.max(*v))
    };
    let mut best = (f32::MIN, 0.0);
    let span = (0.25 / step) as i64;
    for k in -span..=span {
        let shift = k as f32 * step;
        let score: f32 = beats.iter().map(|&t| strength(t + shift)).sum();
        if score > best.0 {
            best = (score, shift);
        }
    }
    best.1
}

/// Analyses `path` into a music map (no corrections).
pub(crate) fn analyze(path: &Path) -> Result<MusicMap> {
    let samples = media::audio_mono(path, RATE)?;
    ensure!(
        samples.len() > RATE as usize * 4,
        "{} is shorter than 4 seconds",
        path.display()
    );
    let fps = RATE as f32 / HOP as f32;
    let (env, low, rms) = envelopes(&samples);
    let period = tempo(&env, fps);
    let beat_frames = track(&env, period);
    let to_t = |f: usize| (f * HOP + WIN / 2) as f32 / RATE as f32;
    let shift = kick_offset(
        &samples,
        &beat_frames.iter().map(|&f| to_t(f)).collect::<Vec<_>>(),
    );
    log::debug!("beat grid shifted {shift:+.3}s onto the kicks");
    let beats: Vec<f32> = beat_frames
        .iter()
        .map(|&f| (to_t(f) + shift).max(0.0))
        .collect();
    let bpm = 60.0 * fps / period;

    // Downbeat phase: the one whose beats carry the most low end.
    let low_at = |f: usize| -> f32 {
        let lo = f.saturating_sub(2);
        let hi = (f + 3).min(low.len());
        low[lo..hi].iter().sum()
    };
    let phase = (0..4)
        .max_by(|&a, &b| {
            let s = |ph: usize| {
                beat_frames
                    .iter()
                    .skip(ph)
                    .step_by(4)
                    .map(|&f| low_at(f))
                    .sum::<f32>()
            };
            s(a).total_cmp(&s(b))
        })
        .unwrap_or(0);
    let beats: Vec<f32> = beats.into_iter().skip(phase).collect();
    let frames: Vec<usize> = beat_frames.into_iter().skip(phase).collect();

    let mut bars = Vec::new();
    for chunk in frames.chunks(4) {
        let (Some(&a), Some(&last)) = (chunk.first(), chunk.last()) else {
            continue;
        };
        let b = (last + period.round() as usize).min(rms.len());
        if b <= a {
            continue;
        }
        let r = rms[a..b].iter().map(|x| x * x).sum::<f32>() / (b - a) as f32;
        let l = low[a..b].iter().sum::<f32>() / (b - a) as f32;
        bars.push(Bar {
            t: to_t(a) + shift,
            rms: r.sqrt(),
            low: l,
        });
    }
    let max_low = bars.iter().map(|b| b.low).fold(1e-12_f32, f32::max);
    for b in &mut bars {
        b.low /= max_low;
    }

    let mut map = MusicMap {
        path: path.display().to_string(),
        duration: samples.len() as f32 / RATE as f32,
        bpm,
        beats,
        bars,
        envelope: Some(coarse_envelope(&env, &low, &rms)),
        ..MusicMap::default()
    };
    let feats = map.features();
    let bar_t: Vec<f32> = map.bars.iter().map(|b| b.t).collect();
    map.drops = structure::drops(&feats, &bar_t);
    map.sections = structure::sections(&feats, &map.drops);
    Ok(map)
}

/// Analyses `track` into `<work>/music.json`, keeping the user's
/// corrections when the old map was of the same track; returns the map
/// with them applied.
pub(crate) fn reanalyze(track: &Path, work: &Path) -> Result<MusicMap> {
    let mut map = analyze(track)?;
    map.apply();
    if let Ok(old) = MusicMap::load(work) {
        map.carry_corrections(&old);
    }
    map.save(work)?;
    Ok(map)
}

/// The pace of a cut with no music: 120 BPM, so a bar (the planner's unit
/// of a shot) is two seconds.
pub(crate) const NO_MUSIC_BPM: f32 = 120.0;

/// How long the stand-in grid of a cut with no music runs: longer than any
/// deliverable.
const NO_MUSIC_SECONDS: f32 = 1800.0;

impl MusicMap {
    /// A map with no track behind it: a steady grid at [`NO_MUSIC_BPM`],
    /// flat loudness, no drops and no sections, so the planner paces the
    /// cut evenly and the render plays the clips' own sound. Its `path` is
    /// empty, which is what [`MusicMap::no_music`] reads.
    pub(crate) fn paced() -> Self {
        let period = 60.0 / NO_MUSIC_BPM;
        let count = (NO_MUSIC_SECONDS / period) as usize;
        let beats: Vec<f32> = (0..count).map(|i| i as f32 * period).collect();
        let bars = beats
            .iter()
            .step_by(4)
            .map(|&t| Bar {
                t,
                rms: 0.0,
                low: 0.0,
            })
            .collect();
        let mut map = Self {
            path: String::new(),
            duration: NO_MUSIC_SECONDS,
            bpm: NO_MUSIC_BPM,
            beats,
            bars,
            drops: Vec::new(),
            sections: Vec::new(),
            envelope: None,
            grid_offset: 0.0,
            bpm_override: None,
            downbeat_shift: 0,
            drops_user: Vec::new(),
            sections_user: Vec::new(),
            ranges: Vec::new(),
            raw: None,
        };
        map.apply();
        map
    }

    /// Whether this is the stand-in for a cut with no music ([`MusicMap::paced`]).
    pub(crate) fn no_music(&self) -> bool {
        self.path.is_empty()
    }
}

/// Writes the no-music stand-in ([`MusicMap::paced`]) as `<work>/music.json`.
pub(crate) fn write_no_music(work: &Path) -> Result<MusicMap> {
    let map = MusicMap::paced();
    map.save(work)?;
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::{Bar, DropMark, Envelope, MusicMap, Section, tempo, track};

    #[test]
    fn no_music_is_a_steady_two_second_grid_that_survives_a_save() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        super::write_no_music(dir.path())?;
        let map = MusicMap::load(dir.path())?;
        assert!(map.no_music(), "an empty path marks no music");
        assert!(map.drops.is_empty(), "no drops: {:?}", map.drops);
        let bar = map.beat_time(1, 0) - map.beat_time(0, 0);
        assert!((bar - 2.0).abs() < 1e-4, "a bar is two seconds: {bar}");
        assert!(
            map.bars.len() >= 400,
            "long enough for any cut: {}",
            map.bars.len()
        );
        Ok(())
    }

    #[test]
    fn a_click_track_is_found() -> anyhow::Result<()> {
        // 43 frames/s envelope, a click every 18 frames (~143 BPM).
        let fps = 43.07;
        let env: Vec<f32> = (0..2000)
            .map(|i| if i % 18 == 0 { 1.0 } else { 0.0 })
            .collect();
        let period = tempo(&env, fps);
        assert!(
            (period - 18.0).abs() < 0.6,
            "period ~18 frames, got {period}"
        );
        let beats = track(&env, period);
        let on_click = beats.iter().filter(|&&b| b % 18 == 0).count();
        assert!(
            on_click * 10 >= beats.len() * 9,
            "beats land on clicks: {on_click}/{}",
            beats.len()
        );
        Ok(())
    }

    /// A 120 BPM map, 64 bars: quiet for 16, loud from bar 16, with an
    /// envelope to re-measure from.
    fn synthetic() -> MusicMap {
        let period = 0.5;
        let beats: Vec<f32> = (0..256).map(|i| 1.0 + i as f32 * period).collect();
        let loud = |t: f32| t >= 33.0;
        let hop = 0.1;
        let n = (130.0 / hop) as usize;
        let ts: Vec<f32> = (0..n).map(|i| 0.05 + i as f32 * hop).collect();
        let env = Envelope {
            t0: 0.05,
            hop,
            rms: ts
                .iter()
                .map(|&t| if loud(t) { 0.5 } else { 0.1 })
                .collect(),
            low: ts
                .iter()
                .map(|&t| if loud(t) { 1.0 } else { 0.2 })
                .collect(),
            onset: ts
                .iter()
                .map(|&t| if loud(t) { 0.8 } else { 0.2 })
                .collect(),
        };
        let bars = (0..64)
            .map(|k| {
                let t = beats[4 * k];
                Bar {
                    t,
                    rms: if loud(t) { 0.5 } else { 0.1 },
                    low: if loud(t) { 1.0 } else { 0.2 },
                }
            })
            .collect();
        let mut map = MusicMap {
            path: "C:/m/track.wav".into(),
            duration: 129.5,
            bpm: 120.0,
            beats,
            bars,
            envelope: Some(env),
            ..MusicMap::default()
        };
        let bar_t: Vec<f32> = map.bars.iter().map(|b| b.t).collect();
        map.drops = super::structure::drops(&map.features(), &bar_t);
        map.apply();
        map
    }

    #[test]
    fn corrections_move_the_grid_and_survive_a_save() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut map = synthetic();
        assert!(
            map.drops.first().is_some_and(|d| d.bar == 16),
            "the loud entry is the drop: {:?}",
            map.drops
        );
        let first = map.beats.first().copied().unwrap_or(-1.0);

        map.shift_downbeat(1);
        assert!(
            (map.beats[0] - (first + 0.5)).abs() < 1e-4,
            "the downbeat moved one beat later"
        );
        map.shift_downbeat(-2);
        assert!(
            (map.beats[0] - (first - 0.5)).abs() < 1e-4,
            "and back past the first detected beat: {}",
            map.beats[0]
        );
        map.nudge(0.01);
        map.nudge(0.01);
        assert!(
            (map.beats[0] - (first - 0.5 + 0.02)).abs() < 1e-4,
            "two 10 ms nudges"
        );
        map.save(dir.path())?;
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("music.json"))?)?;
        let saved_first = raw["beats"][0].as_f64().unwrap_or(-1.0) as f32;
        assert!(
            (saved_first - first).abs() < 1e-4,
            "the file keeps the analysis as analysed"
        );
        let back = MusicMap::load(dir.path())?;
        assert_eq!(back.beats, map.beats, "load applies the corrections");
        assert_eq!(back.downbeat_shift, -1, "shift kept");
        assert!((back.grid_offset - 0.02).abs() < 1e-6, "offset kept");

        let mut map = back;
        map.reset_grid();
        assert!(
            (map.beats[0] - first).abs() < 1e-6,
            "reset returns to the detected grid"
        );
        Ok(())
    }

    #[test]
    fn a_tempo_override_rebuilds_the_grid() -> anyhow::Result<()> {
        let mut map = synthetic();
        map.set_bpm(Some(140.0));
        assert!((map.bpm - 140.0).abs() < 1e-4, "tempo set");
        let gap = map.beats[1] - map.beats[0];
        assert!((gap - 60.0 / 140.0).abs() < 1e-4, "beats at 140: {gap}");
        assert!(
            map.beats.last().is_some_and(|t| *t < map.duration),
            "within the track"
        );
        assert!(
            (map.beat_time(2, 0) - map.bars[2].t).abs() < 1e-6,
            "bars follow the grid"
        );
        map.set_bpm(Some(1000.0));
        assert!(map.bpm_override.is_none(), "absurd tempi are refused");
        Ok(())
    }

    #[test]
    fn user_drops_come_first_and_dismissals_stick() -> anyhow::Result<()> {
        let mut map = synthetic();
        map.toggle_drop(40);
        assert_eq!(
            map.drops.first().map(|d| (d.bar, d.user)),
            Some((40, true)),
            "the marked bar leads"
        );
        map.toggle_drop(16);
        assert!(
            map.drops.iter().all(|d| d.bar != 16),
            "the detected drop is dismissed"
        );
        assert!(
            map.drops_user.contains(&DropMark { bar: 16, on: false }),
            "and the dismissal is recorded"
        );
        map.toggle_drop(16);
        assert!(
            map.drops.iter().any(|d| d.bar == 16),
            "toggling again restores it"
        );
        map.toggle_drop(40);
        assert!(map.drops_user.is_empty(), "unmarking clears the mark");
        Ok(())
    }

    #[test]
    fn renames_and_ranges_are_kept() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut map = synthetic();
        assert!(!map.sections.is_empty(), "sections detected");
        map.rename_section(0, "  warmup ");
        map.set_range("reel-30", 31, 13);
        map.set_range("reel-30", 13, 30);
        map.set_range("feed-30", 16, 31);
        map.save(dir.path())?;
        let back = MusicMap::load(dir.path())?;
        assert_eq!(
            back.sections.first().map(|s| s.name.as_str()),
            Some("warmup"),
            "rename kept"
        );
        assert_eq!(back.ranges.len(), 2, "two ranges, the first replaced");
        assert_eq!(
            back.ranges.first().map(|r| (r.from_bar, r.to_bar)),
            Some((13, 30)),
            "replaced in place"
        );
        let mut again = synthetic();
        again.carry_corrections(&back);
        assert_eq!(again.ranges, back.ranges, "re-analysis keeps ranges");
        Ok(())
    }

    #[test]
    fn an_old_map_without_new_fields_loads() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let old = r#"{"path":"a.wav","duration":20.0,"bpm":120.0,
            "beats":[0.0,0.5,1.0,1.5,2.0,2.5,3.0,3.5],
            "bars":[{"t":0.0,"rms":0.1,"low":0.5},{"t":2.0,"rms":0.2,"low":1.0}],
            "drops":[{"bar":1,"t":2.0,"score":0.5}]}"#;
        std::fs::write(dir.path().join("music.json"), old)?;
        let mut map = MusicMap::load(dir.path())?;
        assert_eq!(map.drops.len(), 1, "drops as stored");
        assert_eq!(
            map.sections.last().map(|s: &Section| s.to_bar),
            Some(1),
            "sections derived for an old map"
        );
        map.shift_downbeat(1);
        assert_eq!(map.bars.len(), 2, "re-barred from the analysed bars");
        assert_eq!(map.bar_at(0.6), Some(0), "bar lookup");
        Ok(())
    }
}

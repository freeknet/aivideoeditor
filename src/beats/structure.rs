// ============================================================================
// Module:       beats::structure
// Description:  Song structure from per-bar energy: drop candidates scored
//               with their build-up, and phrase-aligned sections.
//
// Dependencies: none (pure functions over per-bar features)
// ============================================================================

//! Song structure.
//!
//! Everything here works on [`Feat`], one per bar, each feature normalised
//! to 0..1 over the track:
//!
//! - **Drops** ([`drops`]): the low-end jump from the four bars before to
//!   the four after (the old heuristic), multiplied up by the **build**
//!   that precedes it (loudness and onset density rising over the eight
//!   bars before, the low end pulled out just before the drop), with a
//!   small bonus for landing on a phrase start (bar 1 of 4 or 8). A bare
//!   kick re-entry with no build scores less than a real drop. A near-tie
//!   (within 5%) goes to the earlier drop, the one a reel is built on.
//! - **Sections** ([`sections`]): the track cut into 4-bar blocks phased so
//!   the strongest drop starts a block, each block labelled by its energy
//!   (low end, loudness, onset density): high blocks are `drop`; before
//!   the first drop the quiet opening is `intro` and the rest `build`;
//!   after the last drop `outro`; between drops `break`, except the rising
//!   blocks right before a drop, which become `build`. Neighbouring blocks with the same label merge, so
//!   boundaries always sit on 4-bar (and usually 8-bar) phrase lines.

use super::{Drop, Section};

/// One bar's energy, each part normalised to 0..1 over the track.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Feat {
    /// Loudness.
    pub(crate) rms: f32,
    /// Low-end (kick and bass) energy.
    pub(crate) low: f32,
    /// Onset density (how busy the bar is).
    pub(crate) onset: f32,
}

impl Feat {
    /// The blend sections are judged on.
    fn energy(self) -> f32 {
        0.45 * self.low + 0.35 * self.rms + 0.2 * self.onset
    }

    /// What rises through a build: loudness and busyness, not bass.
    fn lift(self) -> f32 {
        0.5 * self.rms + 0.5 * self.onset
    }
}

/// Section names, in the order a dance track usually has them.
pub(crate) const NAMES: [&str; 5] = ["intro", "build", "drop", "break", "outro"];

/// Mean of `f` over `feats[a..b]` (0 for an empty range).
fn mean(feats: &[Feat], a: usize, b: usize, f: impl Fn(Feat) -> f32) -> f32 {
    let b = b.min(feats.len());
    if b <= a {
        return 0.0;
    }
    feats[a..b].iter().map(|x| f(*x)).sum::<f32>() / (b - a) as f32
}

/// How much of a build leads into bar `i`: loudness and onsets rising over
/// the eight bars before, and the low end pulled out just before.
pub(crate) fn build_score(feats: &[Feat], i: usize) -> f32 {
    if i < 4 {
        return 0.0;
    }
    let start = i.saturating_sub(8);
    let half = start + (i - start) / 2;
    let rise = mean(feats, half, i, Feat::lift) - mean(feats, start, half, Feat::lift);
    let low_fall = mean(feats, start, half, |f| f.low) - mean(feats, i - 2, i, |f| f.low);
    rise.clamp(0.0, 1.0) + low_fall.clamp(0.0, 1.0)
}

/// The drop score of bar `i`, or `None` when the low end does not jump.
pub(crate) fn drop_score(feats: &[Feat], i: usize) -> Option<f32> {
    if i < 4 || i + 2 > feats.len() {
        return None;
    }
    let before = mean(feats, i - 4, i, |f| f.low);
    let after = mean(feats, i, i + 4, |f| f.low);
    let jump = after - before;
    if jump <= 0.15 {
        return None;
    }
    let phrase = if i.is_multiple_of(8) {
        0.1
    } else if i.is_multiple_of(4) {
        0.05
    } else {
        0.0
    };
    Some(jump * (1.0 + build_score(feats, i)) + phrase)
}

/// Drop candidates, strongest first, at least eight bars apart.
/// `bar_t` is each bar's start time.
pub(crate) fn drops(feats: &[Feat], bar_t: &[f32]) -> Vec<Drop> {
    let mut all: Vec<Drop> = (0..feats.len())
        .filter_map(|i| {
            drop_score(feats, i).map(|score| Drop {
                bar: i,
                t: bar_t.get(i).copied().unwrap_or(0.0),
                score,
                user: false,
            })
        })
        .collect();
    all.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Drop> = Vec::new();
    for d in all {
        if kept.iter().all(|k| k.bar.abs_diff(d.bar) >= 8) {
            kept.push(d);
        }
    }
    // A near-tie goes to the earlier drop: the first drop is the one a
    // reel is built on, and a repeat of it scores the same.
    let top = kept.first().map_or(0.0, |d| d.score);
    if let Some(i) = kept
        .iter()
        .enumerate()
        .filter(|(_, d)| d.score >= 0.95 * top)
        .min_by_key(|(_, d)| d.bar)
        .map(|(i, _)| i)
    {
        let first = kept.remove(i);
        kept.insert(0, first);
    }
    kept
}

/// Phrase-aligned sections covering every bar, in order.
pub(crate) fn sections(feats: &[Feat], drops: &[Drop]) -> Vec<Section> {
    let n = feats.len();
    if n == 0 {
        return Vec::new();
    }
    // Blocks of four bars, phased so the strongest drop starts one.
    let phase = drops.first().map_or(0, |d| d.bar % 4);
    let mut starts: Vec<usize> = Vec::new();
    if phase > 0 {
        starts.push(0);
    }
    starts.extend((phase..n).step_by(4));
    let blocks: Vec<(usize, usize)> = starts
        .iter()
        .enumerate()
        .map(|(k, &a)| (a, starts.get(k + 1).copied().unwrap_or(n)))
        .collect();
    let energy: Vec<f32> = blocks
        .iter()
        .map(|&(a, b)| mean(feats, a, b, Feat::energy))
        .collect();
    let lo = energy.iter().copied().fold(f32::MAX, f32::min);
    let hi = energy.iter().copied().fold(f32::MIN, f32::max);
    let span = (hi - lo).max(1e-6);
    let level: Vec<f32> = energy.iter().map(|e| (e - lo) / span).collect();
    let mut high: Vec<bool> = level.iter().map(|l| *l >= 0.6).collect();
    if hi - lo < 0.05 {
        // A flat track: one level all the way through.
        high.iter_mut().for_each(|h| *h = true);
    }
    // A strong (or user-marked) drop always opens a high block.
    let top = drops.iter().map(|d| d.score).fold(0.0_f32, f32::max);
    for d in drops.iter().filter(|d| d.user || d.score >= 0.75 * top) {
        if let Some(k) = blocks.iter().position(|&(a, b)| d.bar >= a && d.bar < b) {
            high[k] = true;
        }
    }
    let first = high.iter().position(|h| *h);
    let last = high.iter().rposition(|h| *h);
    let mut label: Vec<&str> = (0..blocks.len())
        .map(|k| match (first, last) {
            (Some(f), Some(l)) => {
                if high[k] {
                    "drop"
                } else if k < f {
                    // Quiet blocks open the track; once it picks up, the
                    // rest of the way to the first drop is the build.
                    if level[..=k].iter().all(|l| *l < 0.3) {
                        "intro"
                    } else {
                        "build"
                    }
                } else if k > l {
                    "outro"
                } else {
                    "break"
                }
            }
            (None, _) | (_, None) => "intro",
        })
        .collect();
    // Builds: up to four rising blocks right before each drop run.
    for s in 1..blocks.len() {
        if !(high[s] && !high[s - 1]) {
            continue;
        }
        let mut k = s - 1;
        let mut taken = 0;
        while k > 0 && taken < 4 && !high[k] {
            let (a, b) = blocks[k];
            let (pa, pb) = blocks[k - 1];
            let rising = mean(feats, a, b, Feat::lift) > mean(feats, pa, pb, Feat::lift) + 0.02;
            let bass_out = mean(feats, a, b, |f| f.low) < 0.8 * mean(feats, pa, pb, |f| f.low);
            let into_drop = taken == 0 && build_score(feats, blocks[s].0) > 0.15;
            if !(rising || bass_out || into_drop) {
                break;
            }
            label[k] = "build";
            taken += 1;
            k -= 1;
        }
    }
    // Merge neighbours with the same label.
    let mut out: Vec<Section> = Vec::new();
    for (k, &(a, b)) in blocks.iter().enumerate() {
        match out.last_mut() {
            Some(prev) if prev.name == label[k] => prev.to_bar = b - 1,
            Some(_) | None => out.push(Section {
                name: label[k].to_owned(),
                from_bar: a,
                to_bar: b - 1,
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Feat, build_score, drop_score, drops, sections};

    /// `n` bars of one level.
    fn level(n: usize, rms: f32, low: f32, onset: f32) -> Vec<Feat> {
        vec![Feat { rms, low, onset }; n]
    }

    /// intro 16 · build 8 · drop 16 · break 8 · build 8 · drop 16 · outro 8.
    fn dance_track() -> Vec<Feat> {
        let mut f = level(16, 0.3, 0.35, 0.2);
        // Build: loudness and onsets climb, the bass drains away.
        f.extend((0..8).map(|i| Feat {
            rms: 0.4 + 0.06 * i as f32,
            low: 0.3 - 0.03 * i as f32,
            onset: 0.3 + 0.08 * i as f32,
        }));
        f.extend(level(16, 1.0, 1.0, 0.8));
        f.extend(level(8, 0.25, 0.2, 0.2));
        f.extend((0..8).map(|i| Feat {
            rms: 0.4 + 0.06 * i as f32,
            low: 0.3 - 0.03 * i as f32,
            onset: 0.3 + 0.08 * i as f32,
        }));
        f.extend(level(16, 1.0, 1.0, 0.8));
        f.extend(level(8, 0.3, 0.3, 0.2));
        f
    }

    #[test]
    fn a_dance_track_gets_its_sections() -> anyhow::Result<()> {
        let f = dance_track();
        let t: Vec<f32> = (0..f.len()).map(|i| i as f32 * 1.7).collect();
        let d = drops(&f, &t);
        let bars: Vec<usize> = d.iter().map(|d| d.bar).collect();
        assert!(
            bars.contains(&24) && bars.contains(&56),
            "drops at bars 24 and 56: {bars:?}"
        );
        let s = sections(&f, &d);
        let names: Vec<&str> = s.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            ["intro", "build", "drop", "break", "build", "drop", "outro"],
            "sections in order: {s:?}"
        );
        assert!(
            s.iter().all(|s| s.from_bar % 4 == 0),
            "boundaries on 4-bar phrase lines: {s:?}"
        );
        assert_eq!(s.first().map(|s| s.from_bar), Some(0), "covers from bar 0");
        assert_eq!(
            s.last().map(|s| s.to_bar),
            Some(f.len() - 1),
            "covers to the last bar"
        );
        let drop = s.iter().find(|s| s.name == "drop");
        assert_eq!(
            drop.map(|s| (s.from_bar, s.to_bar)),
            Some((24, 39)),
            "the first drop is bars 24-39"
        );
        Ok(())
    }

    #[test]
    fn a_build_makes_a_drop_stronger() -> anyhow::Result<()> {
        // Same low-end jump at bar 16, with and without a build before it.
        let mut plain = level(16, 0.4, 0.3, 0.3);
        plain.extend(level(16, 1.0, 1.0, 0.8));
        let mut built = level(8, 0.4, 0.3, 0.3);
        built.extend((0..8).map(|i| Feat {
            rms: 0.4 + 0.07 * i as f32,
            low: if i < 4 { 0.3 } else { 0.05 },
            onset: 0.3 + 0.08 * i as f32,
        }));
        built.extend(level(16, 1.0, 1.0, 0.8));
        assert!(
            build_score(&plain, 16) < 0.01,
            "no build before a plain kick re-entry"
        );
        assert!(build_score(&built, 16) > 0.3, "a build is seen");
        let (Some(a), Some(b)) = (drop_score(&plain, 16), drop_score(&built, 16)) else {
            anyhow::bail!("both are drops");
        };
        assert!(b > a, "the built drop scores higher: {b} vs {a}");
        Ok(())
    }

    #[test]
    fn drops_prefer_phrase_starts_and_keep_apart() -> anyhow::Result<()> {
        let mut f = level(16, 0.3, 0.2, 0.2);
        f.extend(level(16, 1.0, 1.0, 0.8));
        let t: Vec<f32> = (0..f.len()).map(|i| i as f32).collect();
        let d = drops(&f, &t);
        assert_eq!(d.first().map(|d| d.bar), Some(16), "the jump is at bar 16");
        assert!(
            d.windows(2).all(|w| w[0].score >= w[1].score),
            "strongest first"
        );
        assert!(
            d.iter().all(|x| d
                .iter()
                .all(|y| x.bar == y.bar || x.bar.abs_diff(y.bar) >= 8)),
            "eight bars apart"
        );
        Ok(())
    }

    #[test]
    fn a_flat_track_is_one_section() -> anyhow::Result<()> {
        let f = level(20, 0.5, 0.5, 0.5);
        let s = sections(&f, &[]);
        assert_eq!(s.len(), 1, "one section: {s:?}");
        assert_eq!(s.first().map(|s| s.to_bar), Some(19), "to the end");
        assert!(sections(&[], &[]).is_empty(), "nothing for no bars");
        Ok(())
    }
}

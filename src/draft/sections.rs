// ============================================================================
// Module:       draft::sections
// Description:  Drafts a cut as one section per performer: each a few bars
//               long, changing on the phrase, its shots in recording order
//               and cut forward past the slow stretches.
//
// Dependencies: crate::index, crate::beats, crate::plan, crate::people
// ============================================================================

//! The sections structure ([`Structure::Sections`]).
//!
//! A montage of everyone's best seconds (the mix structure) reads as random
//! on a night with little footage: the same few faces keep coming back in a
//! different order. Sections give the cut a story instead:
//!
//! - **The breakdown** before the drop is one performer, the calmest
//!   striking one (LED, silks), a shot a bar, building 2 + 1 + 1 into the
//!   drop.
//! - **The drop** opens the section of the performer with the strongest
//!   fire, their biggest burst on the downbeat. The hero shot is the one
//!   shot pulled out of time; the rest of that section keeps to the clock.
//! - **The other performers** follow in recording order (the night as it
//!   happened), each for a few bars. Performers with more good footage get
//!   more bars, weak ones fewer, and sections change on whole pairs of bars
//!   so the switch lands on the music's phrase. Nobody comes back.
//! - **Inside a section** the shots run forward through the performer's
//!   footage, each a window on a good moment, picked together (a dynamic
//!   programme over the section) so the run gets the best total. Between
//!   two shots of one take the cut skips at least `min_skip` seconds: that
//!   is where the slow stretches go, so the set plays back sped up on the
//!   beat instead of in real time. Shot lengths follow the music's energy
//!   as in the mix ([`super::pace`]).
//! - The last bar is the finale and prefers fire within its section.
//! - Footage the people map calls `venue`, `crowd` or `none-…` is the room,
//!   not a performer, and gets no section.
//!
//! When a performer's footage cannot fill their bars, their section gives a
//! pair of bars to the strongest other section and the draft is tried
//! again; the cut eases (fewer cuts per bar) before that.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Result, bail};

use super::{Candidate, Layout, STAND_IN_FIT, Spec, Want, as_shot, candidates, fit, layout, pace};
use crate::beats::MusicMap;
use crate::index::Index;
use crate::people::People;
use crate::plan::{Finish, Memory, Plan, Shot};

/// Shot lengths a section can use, in beats.
const LENGTHS: [usize; 4] = [1, 2, 4, 8];

/// Bars a section changes on: a section is a whole number of these.
const UNIT: usize = 2;

/// Most retries of the bar allocation before giving up.
const ROUNDS: usize = 24;

/// Whether footage the people map names `who` is a performer's: the room
/// (`venue`, `crowd`) and the draft's `none-…` groups get no section.
fn performs(who: &str) -> bool {
    let who = who.trim().to_lowercase();
    !(matches!(who.as_str(), "venue" | "crowd" | "none") || who.starts_with("none-"))
}

/// One performer's footage as the drafter weighs it.
#[derive(Clone, Debug)]
struct Performer {
    name: String,
    /// Recording clock of their first good moment (seconds).
    first: f64,
    /// Mean score of their best few 4-beat windows.
    top: f32,
    /// Best hero fit (fire, burst on the cut) among their windows.
    hero: f32,
    /// Best breakdown fit among their windows.
    calm: f32,
    /// Distinct 2-beat moments their footage holds, cut forward.
    capacity: usize,
}

impl Performer {
    /// What a section of theirs is worth: quality, and enough of it.
    fn value(&self) -> f32 {
        self.top * (self.capacity as f32).sqrt()
    }
}

/// A slot of a section: its length, what it wants and where it sits.
#[derive(Clone, Copy, Debug)]
struct Slot {
    beats: usize,
    want: Want,
}

/// The candidate windows of every length, tagged with the performer.
struct Pools {
    by_len: BTreeMap<usize, Vec<(String, Candidate)>>,
    /// Recording clock of each clip's start (seconds); clips with no time
    /// fall back to their order in the index, far apart.
    clock: HashMap<String, f64>,
    beat: f32,
}

impl Pools {
    fn new(index: &Index, people: &People, spec: &Spec, beat: f32, memory: &Memory) -> Self {
        let mut by_len = BTreeMap::new();
        for beats in LENGTHS {
            let dur = beats as f32 * beat;
            let pool = candidates(index, &spec.rules, spec.aspect, dur, true, memory)
                .into_iter()
                .filter(|c| !matches!(c.kind, "venue" | "none"))
                .map(|c| (people.name_at(&c.clip, c.t_in), c))
                .filter(|(who, _)| performs(who))
                .collect();
            by_len.insert(beats, pool);
        }
        let clock = index
            .clips
            .iter()
            .enumerate()
            .map(|(k, c)| {
                let t = c
                    .probe
                    .recorded_at
                    .as_ref()
                    .and_then(crate::media::RecordedAt::unix)
                    .map_or(1.0e9 + k as f64 * 1.0e5, |u| u as f64);
                (c.id.clone(), t)
            })
            .collect();
        Self {
            by_len,
            clock,
            beat,
        }
    }

    /// When `c` starts on the night's clock.
    fn at(&self, c: &Candidate) -> f64 {
        self.clock.get(&c.clip).copied().unwrap_or(0.0) + f64::from(c.t_in)
    }

    /// The windows of `beats` showing `who`.
    fn of<'p>(&'p self, beats: usize, who: &'p str) -> impl Iterator<Item = &'p Candidate> {
        self.by_len
            .get(&beats)
            .into_iter()
            .flatten()
            .filter(move |(w, _)| w == who)
            .map(|(_, c)| c)
    }

    /// Everyone with footage, weighed.
    fn performers(&self, neutral: bool, min_skip: f32) -> Vec<Performer> {
        let mut names: Vec<&str> = self
            .by_len
            .get(&2)
            .into_iter()
            .flatten()
            .map(|(w, _)| w.as_str())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
            .into_iter()
            .filter_map(|name| {
                let mut short: Vec<&Candidate> = self.of(2, name).collect();
                short.sort_by(|a, b| self.at(a).total_cmp(&self.at(b)));
                let first = short.first().map(|c| self.at(c))?;
                let dur = 2.0 * self.beat;
                let mut capacity = 0;
                let mut last: Option<&Candidate> = None;
                for c in short {
                    if last.is_none_or(|l| follows(self, l, dur, c, min_skip)) {
                        capacity += 1;
                        last = Some(c);
                    }
                }
                let mut long: Vec<&Candidate> = self.of(4, name).collect();
                if long.is_empty() {
                    long = self.of(2, name).collect();
                }
                let mut scores: Vec<f32> = long.iter().map(|c| c.score).collect();
                scores.sort_by(|a, b| b.total_cmp(a));
                scores.truncate(4);
                let top = scores.iter().sum::<f32>() / scores.len().max(1) as f32;
                let best = |want: Want| {
                    long.iter()
                        .map(|c| c.score * fit(want, c, neutral))
                        .fold(0.0_f32, f32::max)
                };
                Some(Performer {
                    name: name.to_owned(),
                    first,
                    top,
                    hero: best(Want::Hero),
                    calm: best(Want::Calm),
                    capacity,
                })
            })
            .collect()
    }
}

/// Whether `b` may follow `a` (which lasts `a_dur`) in a section: later on
/// the clock, and in one take at least `min_skip` past `a`'s out-point.
fn follows(pools: &Pools, a: &Candidate, a_dur: f32, b: &Candidate, min_skip: f32) -> bool {
    if a.clip == b.clip {
        b.t_in >= a.t_in + a_dur + min_skip
    } else {
        pools.at(b) >= pools.at(a) + f64::from(a_dur)
    }
}

/// Whether `c` (lasting `dur`) shows frames a placed shot already shows.
fn overlaps(c: &Candidate, dur: f32, placed: &[(Shot, f32)]) -> bool {
    placed
        .iter()
        .any(|(s, d)| s.clip == c.clip && c.t_in < s.t_in + d && s.t_in < c.t_in + dur)
}

/// One performer's run of bars.
#[derive(Clone, Debug)]
struct Section {
    who: usize,
    /// First bar, counted from the cut's start bar.
    first: usize,
    bars: usize,
}

/// The slots of a section that starts `from_drop` bars after the drop
/// (`None`: the breakdown before it), at pacing ease `ease`.
fn slots(
    map: &MusicMap,
    drop_bar: usize,
    from_drop: Option<usize>,
    bars: usize,
    finale: bool,
    ease: usize,
) -> Vec<Slot> {
    let mut out = Vec::new();
    let Some(k0) = from_drop else {
        for b in 0..bars {
            if b + 1 == bars {
                out.extend([2, 1, 1].map(|beats| Slot {
                    beats,
                    want: Want::Build,
                }));
            } else {
                out.push(Slot {
                    beats: 4,
                    want: if b == 0 { Want::Hook } else { Want::Calm },
                });
            }
        }
        return out;
    };
    let mut first = 0;
    if k0 == 0 {
        out.push(Slot {
            beats: 4,
            want: Want::Hero,
        });
        first = 1;
    }
    let tail = usize::from(finale && bars > first);
    let n = bars - first - tail;
    for p in pace(map, drop_bar + k0 + first, k0 + first, n, ease) {
        for &beats in p.beats() {
            out.push(Slot {
                beats,
                want: Want::Other,
            });
        }
    }
    if tail == 1 {
        out.push(Slot {
            beats: 4,
            want: Want::Finale,
        });
    }
    out
}

/// What a candidate is worth in a slot of its performer's section.
fn worth(slot: Slot, c: &Candidate, neutral: bool) -> f32 {
    let mult = match slot.want {
        Want::Hero => fit(Want::Hero, c, neutral).max(STAND_IN_FIT),
        Want::Finale => fit(Want::Finale, c, neutral).max(0.3),
        Want::Hook | Want::Calm | Want::Build | Want::Fire | Want::Other => 1.0,
    };
    c.score * mult
}

/// Fills `slots` from `who`'s footage: the best-scoring run that keeps to
/// the clock and skips forward within a take, clear of `placed`. A
/// [`Want::Hero`] slot is filled first and on its own (the drop may come
/// from any point of their set). `None` when the footage cannot fill it.
fn fill<'p>(
    pools: &'p Pools,
    who: &'p str,
    slots: &[Slot],
    placed: &[(Shot, f32)],
    neutral: bool,
    min_skip: f32,
) -> Option<Vec<&'p Candidate>> {
    let mut taken: Vec<(Shot, f32)> = placed.to_vec();
    let mut hero = None;
    let mut hero_dur = 0.0;
    let mut rest = slots;
    if let Some((first, tail)) = slots.split_first()
        && first.want == Want::Hero
    {
        let dur = first.beats as f32 * pools.beat;
        let c = pools
            .of(first.beats, who)
            .filter(|c| !overlaps(c, dur, &taken))
            .max_by(|a, b| worth(*first, a, neutral).total_cmp(&worth(*first, b, neutral)))?;
        taken.push((shot_of(c, first.beats), dur));
        hero = Some(c);
        hero_dur = dur;
        rest = tail;
    }
    // best[k][i]: best total of slots 0..=k ending with option i of slot k.
    let options: Vec<Vec<&Candidate>> = rest
        .iter()
        .map(|s| {
            let dur = s.beats as f32 * pools.beat;
            let mut o: Vec<&Candidate> = pools
                .of(s.beats, who)
                .filter(|c| !overlaps(c, dur, &taken))
                .collect();
            o.sort_by(|a, b| pools.at(a).total_cmp(&pools.at(b)));
            o
        })
        .collect();
    let mut best: Vec<Vec<f32>> = Vec::with_capacity(rest.len());
    let mut back: Vec<Vec<usize>> = Vec::with_capacity(rest.len());
    for (k, slot) in rest.iter().enumerate() {
        let mut row = vec![f32::NEG_INFINITY; options[k].len()];
        let mut from = vec![usize::MAX; options[k].len()];
        for (i, c) in options[k].iter().enumerate() {
            let own = worth(*slot, c, neutral);
            if k == 0 {
                // After the hero (pulled out of time) the run may start
                // anywhere but a few seconds back in the same take.
                let clear = hero.is_none_or(|h: &Candidate| {
                    h.clip != c.clip
                        || follows(pools, h, hero_dur, c, min_skip)
                        || h.t_in - c.t_in >= crate::plan::JUMP_CUT_GAP
                });
                if clear {
                    row[i] = own;
                }
                continue;
            }
            let prev_dur = rest[k - 1].beats as f32 * pools.beat;
            for (j, p) in options[k - 1].iter().enumerate() {
                let total = best[k - 1][j] + own;
                if best[k - 1][j].is_finite()
                    && total > row[i]
                    && follows(pools, p, prev_dur, c, min_skip)
                {
                    row[i] = total;
                    from[i] = j;
                }
            }
        }
        best.push(row);
        back.push(from);
    }
    let mut picks = Vec::with_capacity(rest.len());
    if let Some(last) = best.last() {
        let mut i = last
            .iter()
            .enumerate()
            .filter(|(_, v)| v.is_finite())
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)?;
        for k in (0..rest.len()).rev() {
            picks.push(*options.get(k)?.get(i)?);
            if k > 0 {
                i = *back.get(k)?.get(i)?;
            }
        }
        picks.reverse();
    }
    let mut out: Vec<&Candidate> = hero.into_iter().collect();
    out.extend(picks);
    (out.len() == slots.len()).then_some(out)
}

/// A bare shot for the overlap bookkeeping.
fn shot_of(c: &Candidate, beats: usize) -> Shot {
    Shot {
        clip: c.clip.clone(),
        t_in: c.t_in,
        beats,
        cx: c.cx,
        who: String::new(),
        seg: String::new(),
        note: String::new(),
    }
}

/// The most [`UNIT`]s a performer whose footage holds `capacity` 2-beat
/// moments should get: about two cuts a bar, so a unit wants four moments,
/// and at most twice the section length.
fn cap(capacity: usize, section_bars: usize) -> usize {
    (capacity / (2 * UNIT)).clamp(1, (2 * section_bars / UNIT).max(1))
}

/// Splits the `after` bars following the drop among the hero and the
/// chosen others (`(value, capacity)` each): whole [`UNIT`]s, at least one
/// each, the rest by value up to what each one's footage holds. Units no
/// one has room for go where the most footage is to spare. `units[0]` is
/// the hero's.
fn allocate(after: usize, section_bars: usize, values: &[(f32, usize)]) -> Vec<usize> {
    let total = after / UNIT;
    let mut units = vec![1_usize; values.len().min(total.max(1))];
    let cap = |k: usize| cap(values[k].1, section_bars);
    let mut left = total.saturating_sub(units.len());
    // The hero's section first gets its full length.
    while left > 0 && units[0] * UNIT < section_bars && units[0] < cap(0) {
        units[0] += 1;
        left -= 1;
    }
    let sum: f32 = values
        .iter()
        .take(units.len())
        .map(|v| v.0)
        .sum::<f32>()
        .max(1e-6);
    while left > 0 {
        // The section furthest below its value share, among those with room.
        let pick = (0..units.len())
            .filter(|&k| units[k] < cap(k))
            .max_by(|&a, &b| {
                let need = |k: usize| values[k].0 / sum * total as f32 - units[k] as f32;
                need(a).total_cmp(&need(b))
            });
        // Everyone full: the most footage to spare per unit takes it.
        let k = pick.unwrap_or_else(|| {
            (0..units.len())
                .max_by(|&a, &b| {
                    let spare = |k: usize| values[k].1 as f32 / (units[k] + 1) as f32;
                    spare(a).total_cmp(&spare(b))
                })
                .unwrap_or(0)
        });
        units[k] += 1;
        left -= 1;
    }
    units
}

/// Drafts a sections cut for `spec`.
pub(super) fn draft(
    index: &Index,
    map: &MusicMap,
    people: &People,
    spec: &Spec,
    music: &str,
    memory: &Memory,
) -> Result<Plan> {
    let Layout {
        start_bar,
        drop_bar,
        after,
    } = layout(map, spec)?;
    let lead = drop_bar - start_bar;
    let rules = &spec.rules;
    let neutral = rules.drop_subject.is_empty();
    let beat = 60.0 / map.bpm;
    let pools = Pools::new(index, people, spec, beat, memory);
    let everyone = pools.performers(neutral, rules.min_skip);
    if everyone.is_empty() {
        bail!("no usable footage of any performer for a sections cut");
    }
    let section_bars = rules.section_bars.max(UNIT).div_ceil(UNIT) * UNIT;

    // The drop's performer: the strongest fire (the best shot when the
    // rules want no subject there).
    let by = |f: fn(&Performer) -> f32| move |a: &&Performer, b: &&Performer| f(a).total_cmp(&f(b));
    let hero = everyone
        .iter()
        .max_by(by(|p| {
            if p.hero > 0.0 {
                p.hero
            } else {
                p.value() * 1e-3
            }
        }))
        .map(|p| p.name.clone())
        .unwrap_or_default();
    // Breakdown candidates, best first; the hero only as a last resort.
    let mut intros: Vec<&Performer> = everyone.iter().filter(|p| p.name != hero).collect();
    intros.sort_by(|a, b| by(|p| p.calm)(b, a));
    intros.extend(everyone.iter().filter(|p| p.name == hero));

    let mut excluded: Vec<String> = Vec::new();
    let mut shrink: HashMap<String, usize> = HashMap::new();
    let mut intro_at = 0;
    for _ in 0..ROUNDS {
        let Some(intro) = intros.get(intro_at).map(|p| p.name.clone()) else {
            bail!(
                "no performer's footage can carry the {lead}-bar breakdown before the drop; \
                 try the mix structure (rules.structure = \"mix\") or a shorter cut"
            );
        };
        // The others: the best by value, as many as the bars allow, then in
        // recording order.
        let mut others: Vec<&Performer> = everyone
            .iter()
            .filter(|p| p.name != hero && p.name != intro && !excluded.contains(&p.name))
            .collect();
        others.sort_by(|a, b| by(Performer::value)(b, a));
        let left_of = |p: &Performer| {
            let cut = shrink.get(&p.name).copied().unwrap_or(0);
            p.capacity.saturating_sub(cut * 2 * UNIT)
        };
        let rest = after.saturating_sub(section_bars);
        let room = (rest + section_bars / 2) / section_bars.max(1);
        let mut take = if rest >= UNIT { room.max(1) } else { 0 };
        // More performers while the chosen ones' footage cannot fill the
        // bars: a thin night gets more, shorter sections.
        let held = |n: usize| -> usize {
            everyone
                .iter()
                .filter(|p| p.name == hero)
                .chain(others.iter().copied().take(n))
                .map(|p| cap(left_of(p), section_bars))
                .sum()
        };
        while take < others.len() && take < after / UNIT - 1 && held(take) < after / UNIT {
            take += 1;
        }
        others.truncate(take);
        others.sort_by(|a, b| a.first.total_cmp(&b.first));
        let Some(hero_p) = everyone.iter().find(|p| p.name == hero) else {
            bail!("the drop's performer vanished");
        };
        let mut lineup: Vec<&Performer> = vec![hero_p];
        lineup.extend(others.iter().copied());
        let values: Vec<(f32, usize)> = lineup.iter().map(|p| (p.value(), left_of(p))).collect();
        let units = allocate(after, section_bars, &values);
        let mut sections = Vec::new();
        if lead > 0 {
            sections.push(Section {
                who: usize::MAX,
                first: 0,
                bars: lead,
            });
        }
        let mut bar = lead;
        for (k, &u) in units.iter().enumerate() {
            let bars = if k + 1 == units.len() {
                lead + after - bar
            } else {
                u * UNIT
            };
            sections.push(Section {
                who: k,
                first: bar,
                bars,
            });
            bar += bars;
        }
        // Fill: the drop's section, then the breakdown, then the rest.
        let mut order: Vec<usize> = (0..sections.len()).collect();
        order.sort_by_key(|&s| match sections[s].who {
            0 => 0,
            usize::MAX => 1,
            _ => 2,
        });
        let mut placed: Vec<(Shot, f32)> = Vec::new();
        let mut filled: Vec<Option<Vec<(Slot, Candidate)>>> = vec![None; sections.len()];
        let mut failed = None;
        for s in order {
            let sec = &sections[s];
            let name = if sec.who == usize::MAX {
                intro.as_str()
            } else {
                lineup[sec.who].name.as_str()
            };
            let from_drop = (sec.who != usize::MAX).then(|| sec.first - lead);
            let finale = sec.first + sec.bars == lead + after && after >= 2;
            let got = (0..3).find_map(|ease| {
                let want = slots(map, drop_bar, from_drop, sec.bars, finale, ease);
                fill(&pools, name, &want, &placed, neutral, rules.min_skip)
                    .map(|picks| (want, picks))
            });
            let Some((want, picks)) = got else {
                failed = Some(s);
                break;
            };
            let mut run = Vec::new();
            for (slot, c) in want.into_iter().zip(picks) {
                placed.push((shot_of(c, slot.beats), slot.beats as f32 * beat));
                run.push((slot, c.clone()));
            }
            filled[s] = Some(run);
        }
        if let Some(s) = failed {
            log::debug!(
                "sections: {} could not fill {} bars from bar {} (lineup {:?}, units {units:?})",
                if sections[s].who == usize::MAX {
                    intro.as_str()
                } else {
                    lineup[sections[s].who].name.as_str()
                },
                sections[s].bars,
                sections[s].first,
                lineup
                    .iter()
                    .map(|p| (&p.name, p.capacity))
                    .collect::<Vec<_>>()
            );
            match sections[s].who {
                usize::MAX => intro_at += 1,
                0 if units.len() == 1 => {
                    bail!(
                        "`{hero}`'s footage cannot fill the {after} bars after the drop and no \
                         other performer is left; make a shorter cut"
                    );
                }
                k => {
                    let name = lineup[k].name.clone();
                    if units[k] <= 1 && k > 0 {
                        excluded.push(name);
                    } else {
                        *shrink.entry(name).or_insert(0) += 1;
                    }
                }
            }
            continue;
        }
        let names: Vec<String> = sections
            .iter()
            .map(|s| {
                if s.who == usize::MAX {
                    intro.clone()
                } else {
                    lineup[s.who].name.clone()
                }
            })
            .collect();
        return Ok(assemble(
            spec,
            music,
            (start_bar, drop_bar, after),
            &sections,
            &names,
            filled,
            people,
            &pools,
        ));
    }
    bail!(
        "could not share the {after} bars after the drop among the performers' footage; \
         try a shorter cut or the mix structure"
    )
}

/// The plan from the filled sections, with notes saying why each shot is
/// where it is.
#[expect(
    clippy::too_many_arguments,
    reason = "the draft's results; a struct would only rename them"
)]
fn assemble(
    spec: &Spec,
    music: &str,
    bars: (usize, usize, usize),
    sections: &[Section],
    names: &[String],
    filled: Vec<Option<Vec<(Slot, Candidate)>>>,
    people: &People,
    pools: &Pools,
) -> Plan {
    let (start_bar, drop_bar, after) = bars;
    let count = sections.len();
    let mut shots = Vec::new();
    for (k, run) in filled.into_iter().enumerate() {
        let run = run.unwrap_or_default();
        let n = run.len();
        let mut prev: Option<(Candidate, f32)> = None;
        for (i, (slot, c)) in run.into_iter().enumerate() {
            let mut shot = as_shot(&c, slot.beats, people);
            let mut how = String::new();
            if let Some((p, d)) = &prev {
                let skip = c.t_in - (p.t_in + d);
                if p.clip == c.clip && skip >= 0.0 {
                    how = format!(" · skips {skip:.1}s");
                } else if p.clip == c.clip {
                    how = format!(" · back to {:.1}s, their set from the top", c.t_in);
                } else if slot.want != Want::Hero {
                    how = " · next take".to_owned();
                }
            }
            if c.aligned && slot.want == Want::Hero {
                how.push_str(&format!(" · burst {:.1}% on the cut", c.burst * 100.0));
            }
            if c.cheer > 0.0 {
                how.push_str(&format!(" · ends on a cheer {:.2}", c.cheer));
            }
            let role = match slot.want {
                Want::Hero => "hero",
                Want::Finale => "finale",
                Want::Hook => "hook",
                Want::Build => "build",
                Want::Calm | Want::Fire | Want::Other => "",
            };
            let role = if role.is_empty() {
                String::new()
            } else {
                format!("{role} · ")
            };
            let who = names.get(k).map_or("", String::as_str);
            shot.note = format!(
                "{role}section {}/{count} {who} {}/{n} · {} {} · score {:.2}{how}",
                k + 1,
                i + 1,
                c.kind,
                c.framing,
                c.score
            )
            .to_lowercase();
            shot.who = who.to_owned();
            prev = Some((c, slot.beats as f32 * pools.beat));
            shots.push(shot);
        }
    }
    let card_bar = drop_bar + after;
    Plan {
        name: spec.name.clone(),
        aspect: spec.aspect,
        music: music.to_owned(),
        start_bar,
        drop_bar,
        card_bar,
        end_bar: card_bar + spec.card_bars,
        end_card: spec.end_card.clone(),
        rules: spec.rules.clone(),
        finish: Finish::default(),
        shots,
        comments: Vec::new(),
        accepted: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::allocate;
    use crate::draft::tests::{fireball_night, map};
    use crate::plan::{Rules, Structure};

    #[test]
    fn the_room_is_not_a_performer() -> anyhow::Result<()> {
        for who in ["venue", "Crowd", "none-04", "none"] {
            assert!(!super::performs(who), "{who} gets no section");
        }
        for who in ["fire-trio", "dj", "MVI_8210", "nonesuch"] {
            assert!(super::performs(who), "{who} does");
        }
        Ok(())
    }

    #[test]
    fn bars_go_by_value_in_whole_pairs() -> anyhow::Result<()> {
        // 16 bars: the hero takes its 4, the rest by value and capacity.
        let units = allocate(16, 4, &[(1.0, 40), (2.0, 40), (0.5, 40)]);
        assert_eq!(
            units.iter().sum::<usize>(),
            8,
            "all 16 bars, in pairs: {units:?}"
        );
        assert!(units[0] >= 2, "the hero gets its section: {units:?}");
        assert!(units[1] > units[2], "more value, more bars: {units:?}");
        // A performer with footage for one pair keeps to it.
        let thin = allocate(16, 4, &[(1.0, 40), (5.0, 4), (1.0, 40)]);
        assert_eq!(thin[1], 1, "capped by footage: {thin:?}");
        Ok(())
    }

    #[test]
    fn a_sections_cut_gives_each_performer_one_forward_run() -> anyhow::Result<()> {
        let (index, _) = fireball_night();
        let mut music = map(&[0.2; 8]
            .iter()
            .chain(&[0.9; 40])
            .copied()
            .collect::<Vec<_>>());
        music.drops.push(crate::beats::Drop {
            bar: 8,
            t: 0.0,
            score: 1.0,
            user: false,
        });
        let rules = Rules {
            structure: Structure::Sections,
            ..Rules::default()
        };
        let spec = crate::draft::Spec {
            name: "reel-30".into(),
            aspect: crate::plan::Aspect::Vertical,
            seconds: 30.0,
            end_card: String::new(),
            card_bars: 1,
            rules,
        };
        let people = crate::people::People::default();
        let plan = crate::draft::draft(&index, &music, &people, &spec, "m.wav")?;
        // Runs of one performer: each name once.
        let mut runs: Vec<&str> = Vec::new();
        for s in &plan.shots {
            if runs.last() != Some(&s.who.as_str()) {
                runs.push(&s.who);
            }
        }
        let mut seen = runs.clone();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), runs.len(), "nobody comes back: {runs:?}");
        assert!(runs.len() >= 3, "several sections: {runs:?}");
        // Sections change on whole pairs of bars after the drop.
        let lead = (plan.drop_bar - plan.start_bar) * 4;
        let mut beat = 0;
        for pair in plan.shots.windows(2) {
            beat += pair[0].beats;
            if pair[0].who != pair[1].who && beat > lead {
                assert_eq!(
                    (beat - lead) % 8,
                    0,
                    "a section starts on a pair at beat {beat}"
                );
            }
            if pair[0].clip == pair[1].clip && beat > lead + 4 {
                let dur = pair[0].beats as f32 * 60.0 / music.bpm;
                assert!(
                    pair[1].t_in >= pair[0].t_in + dur + spec.rules.min_skip - 0.01,
                    "one take cuts forward past the lull: {pair:?}"
                );
            }
        }
        assert!(
            plan.shots.iter().all(|s| s.note.contains("section")),
            "every note names its section"
        );
        let problems = crate::plan::check(&plan, &index, &music, &people);
        assert!(
            problems
                .iter()
                .all(|f| !matches!(f.id, "repeat-performer" | "jump-cut" | "same-stretch")),
            "the check agrees with the structure: {problems:?}"
        );
        assert!(
            problems
                .iter()
                .all(|f| f.severity != crate::plan::Severity::Hard),
            "no hard findings: {problems:?}"
        );
        Ok(())
    }
}

// ============================================================================
// Module:       refs
// Description:  References: typed pointers at footage, shots, music and people
//               that any page can hand to the assistant, with intent modifiers.
//
// Dependencies: crate::{index, plan, beats, project}
// ============================================================================

//! References ("refs").
//!
//! A reference names something precisely so a conversation can be about it:
//! "replace **shot 5**, keep **MVI_7987@24.0-27.5**, avoid **white-mask**, and
//! put something like **MVI_7925/3** on **bars 17-20**". Every page can make
//! them from its selection (shift/ctrl-click, a marked I/O range, a beat
//! range) and drop them into the assistant's composer, where they show as
//! chips and travel with the message.
//!
//! ## Kinds
//!
//! | Text form | Kind | Points at |
//! |---|---|---|
//! | `@shot:reel-30#5` | [`Target::Shot`] | slot 5 (1-based) of a plan |
//! | `@seg:MVI_7987/3` | [`Target::Segment`] | a shot-log segment |
//! | `@clip:MVI_7987@24.0-27.5` | [`Target::Range`] | a time range of a source clip |
//! | `@frame:MVI_7987@25.8` | [`Target::Frame`] | one moment of a source clip |
//! | `@bars:17-20` | [`Target::Bars`] | bars of the music (inclusive) |
//! | `@person:white-mask` | [`Target::Person`] | a performer from the people map |
//! | `@plan:reel-30` | [`Target::Plan`] | a whole plan |
//!
//! ## Modifiers
//!
//! A single prefix character says what the user wants done with the target:
//!
//! | Prefix | [`Modifier`] | Meaning |
//! |---|---|---|
//! | (none) | `About` | just "this one" |
//! | `+` | `Keep` | keep / pin it, do not change it |
//! | `!` | `Avoid` | never use it (a rule for this request) |
//! | `>` | `Replace` | swap it for something better |
//! | `~` | `Like` | find / use more like it |
//! | `*` | `Hero` | feature it: longer, on a strong beat, the drop |
//!
//! `+@shot:reel-30#1 >@shot:reel-30#5 ~@seg:MVI_7925/3 !@person:white-mask`
//! round-trips through [`Reference::parse`] and `Display`, so a reference
//! typed by hand and one made by clicking are the same thing.
//!
//! [`resolve`] turns a reference into the facts the assistant needs — the
//! source file and range, the shot-log numbers, the caption — so the model
//! never has to guess what "that shot" was.

use std::fmt;

use crate::project::Project;

/// What a reference points at.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// A slot of a plan, 1-based.
    Shot {
        /// Plan name (file stem).
        plan: String,
        /// Slot number, 1-based.
        index: usize,
    },
    /// A shot-log segment, e.g. `MVI_7987/3`.
    Segment {
        /// Segment id.
        id: String,
    },
    /// A range of a source clip, seconds.
    Range {
        /// Clip id.
        clip: String,
        /// Start.
        t0: f32,
        /// End.
        t1: f32,
    },
    /// One moment of a source clip.
    Frame {
        /// Clip id.
        clip: String,
        /// Time, seconds.
        t: f32,
    },
    /// Music bars, inclusive.
    Bars {
        /// First bar.
        from: usize,
        /// Last bar.
        to: usize,
    },
    /// A performer.
    Person {
        /// Name in the people map.
        name: String,
    },
    /// A whole plan.
    Plan {
        /// Plan name.
        name: String,
    },
}

/// What the user wants done with a target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Modifier {
    /// Just "this one".
    #[default]
    About,
    /// Keep / pin it.
    Keep,
    /// Never use it.
    Avoid,
    /// Swap it for something better.
    Replace,
    /// Use more like it.
    Like,
    /// Feature it.
    Hero,
}

impl Modifier {
    /// Every modifier, in menu order.
    pub const ALL: [Self; 6] = [
        Self::About,
        Self::Keep,
        Self::Avoid,
        Self::Replace,
        Self::Like,
        Self::Hero,
    ];

    /// The text prefix.
    #[must_use]
    pub fn prefix(self) -> &'static str {
        match self {
            Self::About => "",
            Self::Keep => "+",
            Self::Avoid => "!",
            Self::Replace => ">",
            Self::Like => "~",
            Self::Hero => "*",
        }
    }

    /// The word a chip and a menu show.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::About => "about",
            Self::Keep => "keep",
            Self::Avoid => "avoid",
            Self::Replace => "replace",
            Self::Like => "more like",
            Self::Hero => "feature",
        }
    }

    /// The modifier a prefix character stands for.
    fn from_prefix(c: char) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.prefix().starts_with(c) && !m.prefix().is_empty())
    }
}

/// A reference: a target and what to do with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    /// What it points at.
    pub target: Target,
    /// What to do with it.
    pub modifier: Modifier,
}

impl Reference {
    /// A plain reference to `target`.
    #[must_use]
    pub fn new(target: Target) -> Self {
        Self {
            target,
            modifier: Modifier::About,
        }
    }

    /// The same reference with `modifier`.
    #[must_use]
    pub fn with(mut self, modifier: Modifier) -> Self {
        self.modifier = modifier;
        self
    }

    /// A short label for a chip: `shot 5`, `MVI_7987 24.0–27.5`, `bars 17–20`.
    #[must_use]
    pub fn short(&self) -> String {
        match &self.target {
            Target::Shot { index, .. } => format!("shot {index}"),
            Target::Segment { id } => id.clone(),
            Target::Range { clip, t0, t1 } => format!("{clip} {t0:.1}–{t1:.1}"),
            Target::Frame { clip, t } => format!("{clip} @{t:.1}"),
            Target::Bars { from, to } if from == to => format!("bar {from}"),
            Target::Bars { from, to } => format!("bars {from}–{to}"),
            Target::Person { name } => name.clone(),
            Target::Plan { name } => name.clone(),
        }
    }

    /// Parses one reference token (`+@shot:reel-30#5`).
    #[must_use]
    pub fn parse(token: &str) -> Option<Self> {
        let token = token.trim();
        let mut chars = token.chars();
        let first = chars.next()?;
        let (modifier, rest) = match Modifier::from_prefix(first) {
            Some(m) => (m, chars.as_str()),
            None => (Modifier::About, token),
        };
        let rest = rest.strip_prefix('@')?;
        let (kind, value) = rest.split_once(':')?;
        let range = |s: &str| -> Option<(f32, f32)> {
            let (a, b) = s.split_once('-')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        };
        let target = match kind {
            "shot" => {
                let (plan, index) = value.rsplit_once('#')?;
                Target::Shot {
                    plan: plan.to_owned(),
                    index: index.parse().ok().filter(|i| *i > 0)?,
                }
            }
            "seg" => Target::Segment {
                id: value.to_owned(),
            },
            "clip" => {
                let (clip, r) = value.split_once('@')?;
                let (t0, t1) = range(r)?;
                Target::Range {
                    clip: clip.to_owned(),
                    t0,
                    t1,
                }
            }
            "frame" => {
                let (clip, t) = value.split_once('@')?;
                Target::Frame {
                    clip: clip.to_owned(),
                    t: t.parse().ok()?,
                }
            }
            "bars" => match value.split_once('-') {
                Some((a, b)) => Target::Bars {
                    from: a.parse().ok()?,
                    to: b.parse().ok()?,
                },
                None => {
                    let bar = value.parse().ok()?;
                    Target::Bars { from: bar, to: bar }
                }
            },
            "person" => Target::Person {
                name: value.to_owned(),
            },
            "plan" => Target::Plan {
                name: value.to_owned(),
            },
            _ => return None,
        };
        Some(Self { target, modifier })
    }

    /// Every reference token in `text`, in order (anything else is ignored).
    #[must_use]
    pub fn scan(text: &str) -> Vec<Self> {
        text.split_whitespace().filter_map(Self::parse).collect()
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.modifier.prefix())?;
        match &self.target {
            Target::Shot { plan, index } => write!(f, "@shot:{plan}#{index}"),
            Target::Segment { id } => write!(f, "@seg:{id}"),
            Target::Range { clip, t0, t1 } => write!(f, "@clip:{clip}@{t0:.2}-{t1:.2}"),
            Target::Frame { clip, t } => write!(f, "@frame:{clip}@{t:.2}"),
            Target::Bars { from, to } => write!(f, "@bars:{from}-{to}"),
            Target::Person { name } => write!(f, "@person:{name}"),
            Target::Plan { name } => write!(f, "@plan:{name}"),
        }
    }
}

/// What a reference resolves to in the open night.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Resolved {
    /// Source clip id, when the target is footage.
    pub clip: Option<String>,
    /// Source range, seconds.
    pub range: Option<(f32, f32)>,
    /// One line of facts for the assistant: numbers, caption, who.
    pub facts: String,
}

/// Resolves `reference` against `project`, or explains why it cannot.
pub(crate) fn resolve(reference: &Reference, project: &Project) -> Result<Resolved, String> {
    let index = project.index.as_ref();
    let segment_facts = |clip: &str, t0: f32, t1: f32| -> String {
        let Some(index) = index else {
            return String::new();
        };
        index
            .segments
            .iter()
            .filter(|s| s.clip == clip && s.seg.end > t0 && s.seg.start < t1)
            .map(|s| {
                let caption = s
                    .caption
                    .as_ref()
                    .map(|c| format!(" — {}", c.summary))
                    .unwrap_or_default();
                let flags = if s.flags.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", s.flags.join(" "))
                };
                format!("{} {} q{:.2}{flags}{caption}", s.id, s.subject, s.q)
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    match &reference.target {
        Target::Segment { id } => {
            let s = index
                .and_then(|i| i.segments.iter().find(|s| s.id == *id))
                .ok_or_else(|| format!("no segment {id}"))?;
            Ok(Resolved {
                clip: Some(s.clip.clone()),
                range: Some((s.seg.start, s.seg.end)),
                facts: segment_facts(&s.clip, s.seg.start, s.seg.end),
            })
        }
        Target::Range { clip, t0, t1 } => Ok(Resolved {
            clip: Some(clip.clone()),
            range: Some((*t0, *t1)),
            facts: segment_facts(clip, *t0, *t1),
        }),
        Target::Frame { clip, t } => Ok(Resolved {
            clip: Some(clip.clone()),
            range: Some((*t, *t)),
            facts: segment_facts(clip, *t, t + 0.01),
        }),
        Target::Shot { plan, index: slot } => {
            let path = project.work.join("plans").join(format!("{plan}.toml"));
            let p = crate::plan::Plan::load(&path).map_err(|e| format!("{e:#}"))?;
            let shot = p
                .shots
                .get(slot - 1)
                .ok_or_else(|| format!("{plan} has no shot {slot}"))?;
            let len = project
                .music
                .as_ref()
                .map_or(0.0, |m| p.timing(m).get(slot - 1).map_or(0.0, |(_, d)| *d));
            let facts = format!(
                "{plan} shot {slot}: {} @{:.2} for {} beats, cx {:.2}, who {}; note: {}; {}",
                shot.clip,
                shot.t_in,
                shot.beats,
                shot.cx,
                shot.who,
                shot.note,
                segment_facts(&shot.clip, shot.t_in, shot.t_in + len)
            );
            Ok(Resolved {
                clip: Some(shot.clip.clone()),
                range: Some((shot.t_in, shot.t_in + len)),
                facts,
            })
        }
        Target::Bars { from, to } => {
            let m = project
                .music
                .as_ref()
                .ok_or("the music is not mapped yet")?;
            let t0 = m.beat_time(*from, 0);
            let t1 = m.beat_time(to + 1, 0);
            Ok(Resolved {
                clip: None,
                range: Some((t0, t1)),
                facts: format!("bars {from}-{to} = {t0:.2}-{t1:.2}s of the track"),
            })
        }
        Target::Person { name } => {
            let clips: Vec<&String> = project
                .people
                .names
                .iter()
                .filter(|(_, n)| *n == name)
                .map(|(k, _)| k)
                .collect();
            if clips.is_empty() {
                return Err(format!("nobody called {name} in people.toml"));
            }
            Ok(Resolved {
                clip: None,
                range: None,
                facts: format!(
                    "{name} appears in {}",
                    clips
                        .iter()
                        .map(|c| c.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
        }
        Target::Plan { name } => {
            let path = project.work.join("plans").join(format!("{name}.toml"));
            let p = crate::plan::Plan::load(&path).map_err(|e| format!("{e:#}"))?;
            Ok(Resolved {
                clip: None,
                range: None,
                facts: format!(
                    "{name}: {} shots, bars {}-{}",
                    p.shots.len(),
                    p.start_bar,
                    p.end_bar
                ),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Modifier, Reference, Target};

    #[test]
    fn every_kind_round_trips_through_text() {
        let refs = [
            "@shot:reel-30#5",
            "+@seg:MVI_7987/3",
            "!@clip:MVI_7987@24.00-27.50",
            "~@frame:MVI_7925@13.50",
            ">@bars:17-20",
            "*@person:white-mask",
            "@plan:reel-30",
        ];
        for text in refs {
            let r = Reference::parse(text);
            assert_eq!(
                r.as_ref().map(ToString::to_string).as_deref(),
                Some(text),
                "{text}"
            );
        }
    }

    #[test]
    fn modifiers_and_kinds_parse() {
        let r = Reference::parse("!@person:white-mask");
        assert_eq!(
            r,
            Some(
                Reference::new(Target::Person {
                    name: "white-mask".into()
                })
                .with(Modifier::Avoid)
            ),
            "avoid a person"
        );
        assert_eq!(
            Reference::parse("@bars:17").map(|r| r.target),
            Some(Target::Bars { from: 17, to: 17 }),
            "a single bar"
        );
        assert!(
            Reference::parse("@shot:reel-30#0").is_none(),
            "shots are 1-based"
        );
        assert!(Reference::parse("plain words").is_none(), "not a reference");
    }

    #[test]
    fn scanning_a_message_finds_its_references() {
        let found =
            Reference::scan("swap >@shot:reel-30#5 for something like ~@seg:MVI_7925/3 please");
        assert_eq!(found.len(), 2, "two references: {found:?}");
        assert_eq!(found[0].modifier, Modifier::Replace, "first is a replace");
        assert_eq!(found[1].short(), "MVI_7925/3", "chip label");
    }
}

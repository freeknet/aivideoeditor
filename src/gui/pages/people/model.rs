// ============================================================================
// Module:       gui::pages::people::model
// Description:  The People tab's view of the night: who each segment shows,
//               grouped per person, with counts, props and sample frames.
//
// Dependencies: crate::{people, index, project}
// ============================================================================

//! What the People tab draws, derived from the people map and the index.
//!
//! A segment belongs to the person whose entry covers its start (a range
//! entry beats a whole-clip one, exactly as the planner reads the map).
//! Segments no entry covers, whose caption has someone performing in it,
//! are "Unassigned"; so are segments mapped to their own clip id, which is
//! what unlisted footage counts as anyway. Venue and crowd shots with no
//! entry belong to nobody and are not shown.
//!
//! The model is rebuilt only when the map or the index changes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::index::Index;
use crate::people::{ClipPerson, People, prop_kind};
use crate::project::Project;

/// Sample frames shown per person card.
pub(super) const STRIP: usize = 4;

/// One clip's segments under one person.
#[derive(Clone, Debug, Default)]
pub(super) struct ClipRow {
    /// Clip id.
    pub(super) clip: String,
    /// Source file.
    pub(super) path: PathBuf,
    /// The segments (indices into `index.segments`) this person has in it.
    pub(super) segs: Vec<usize>,
}

/// One person (or the Unassigned pile).
#[derive(Clone, Debug, Default)]
pub(super) struct Group {
    /// The name; `None` for Unassigned.
    pub(super) name: Option<String>,
    /// Their clips, in recording order.
    pub(super) clips: Vec<ClipRow>,
    /// Segments over all clips.
    pub(super) segs: usize,
    /// Prop categories seen in their captions, most frequent first.
    pub(super) props: Vec<(&'static str, usize)>,
    /// The best segment of up to [`STRIP`] different clips, for the card.
    pub(super) strip: Vec<usize>,
    /// The first description the captions give.
    pub(super) sample: String,
    /// Recording start of their first clip (seconds since the epoch).
    pub(super) start: f64,
}

impl Group {
    /// The label a card shows.
    pub(super) fn label(&self) -> &str {
        self.name.as_deref().unwrap_or("Unassigned")
    }

    /// Every segment, clip by clip.
    pub(super) fn all_segs(&self) -> impl Iterator<Item = usize> + '_ {
        self.clips.iter().flat_map(|c| c.segs.iter().copied())
    }
}

/// How the person grid is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Order {
    /// By when they were first recorded: the order of the night.
    #[default]
    Time,
    /// Alphabetically.
    Name,
    /// Most segments first.
    Size,
}

impl Order {
    /// Every order, in menu order.
    pub(super) const ALL: [Self; 3] = [Self::Time, Self::Name, Self::Size];

    /// The menu label.
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Time => "Recorded",
            Self::Name => "Name",
            Self::Size => "Segments",
        }
    }
}

/// The whole tab's model.
#[derive(Debug, Default)]
pub(super) struct Model {
    /// Named people.
    pub(super) groups: Vec<Group>,
    /// Performer footage nobody is mapped to.
    pub(super) unassigned: Group,
    /// What the captions say about each clip's main performer.
    pub(super) profiles: Vec<ClipPerson>,
    /// Every clip's segments (indices into `index.segments`), in time order.
    pub(super) clip_segs: BTreeMap<String, Vec<usize>>,
    /// Clip id to source file.
    pub(super) paths: BTreeMap<String, PathBuf>,
}

impl Model {
    /// Builds the model for `people` over `index`.
    pub(super) fn build(people: &People, index: Option<&Index>, order: Order) -> Self {
        let Some(index) = index else {
            // No index: the people still exist, with nothing to show.
            let groups = people
                .all_names()
                .into_iter()
                .map(|n| Group {
                    name: Some(n.to_owned()),
                    ..Group::default()
                })
                .collect();
            return Self {
                groups,
                ..Self::default()
            };
        };
        let profiles = crate::people::profiles(index);
        let mut clip_segs: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut buckets: BTreeMap<Option<String>, BTreeMap<String, Vec<usize>>> = BTreeMap::new();
        for name in people.all_names() {
            if index.clip(name).is_none() {
                buckets.entry(Some(name.to_owned())).or_default();
            }
        }
        for (i, seg) in index.segments.iter().enumerate() {
            clip_segs.entry(seg.clip.clone()).or_default().push(i);
            let owner = match people.entry_at(&seg.clip, seg.seg.start) {
                Some(name) if name != seg.clip => Some(Some(name.to_owned())),
                Some(_) => Some(None),
                None if performer(seg) => Some(None),
                None => None,
            };
            if let Some(owner) = owner {
                buckets
                    .entry(owner)
                    .or_default()
                    .entry(seg.clip.clone())
                    .or_default()
                    .push(i);
            }
        }
        let paths: BTreeMap<String, PathBuf> = index
            .clips
            .iter()
            .map(|c| (c.id.clone(), PathBuf::from(&c.path)))
            .collect();
        let start_of = |clip: &str| {
            profiles
                .iter()
                .find(|p| p.clip == clip)
                .map(|p| p.start)
                .or_else(|| {
                    index
                        .clip(clip)
                        .map(|c| c.stamp.1 as f64 - c.probe.duration)
                })
                .unwrap_or(f64::INFINITY)
        };

        let mut groups = Vec::new();
        let mut unassigned = Group::default();
        for (name, clips) in buckets {
            let mut rows: Vec<ClipRow> = clips
                .into_iter()
                .map(|(clip, segs)| ClipRow {
                    path: paths.get(&clip).cloned().unwrap_or_default(),
                    clip,
                    segs,
                })
                .collect();
            rows.sort_by(|a, b| start_of(&a.clip).total_cmp(&start_of(&b.clip)));
            let group = summarise(name, rows, index, &start_of);
            if group.name.is_some() {
                groups.push(group);
            } else {
                unassigned = group;
            }
        }
        sort(&mut groups, order);
        Self {
            groups,
            unassigned,
            profiles,
            clip_segs,
            paths,
        }
    }

    /// The person called `name`, or Unassigned for `None`.
    pub(super) fn group(&self, name: Option<&str>) -> Option<&Group> {
        match name {
            None => Some(&self.unassigned),
            Some(n) => self.groups.iter().find(|g| g.name.as_deref() == Some(n)),
        }
    }

    /// The source file of `clip`.
    pub(super) fn path(&self, clip: &str) -> &Path {
        self.paths.get(clip).map_or(Path::new(""), PathBuf::as_path)
    }

    /// `clip`'s segments as `(start, end)`, in order, for the map's edits.
    pub(super) fn spans(&self, clip: &str, index: &Index) -> Vec<(f32, f32)> {
        self.clip_segs
            .get(clip)
            .map(|segs| {
                segs.iter()
                    .filter_map(|&i| index.segments.get(i))
                    .map(|s| (s.seg.start, s.seg.end))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Whether a segment's caption shows someone performing (not an empty
/// venue or the crowd).
fn performer(seg: &crate::index::Scored) -> bool {
    seg.caption
        .as_ref()
        .is_some_and(|c| !c.people.is_empty() && !matches!(c.effective_moment(), "venue" | "crowd"))
}

/// Counts, props and the sample strip of one person.
fn summarise(
    name: Option<String>,
    clips: Vec<ClipRow>,
    index: &Index,
    start_of: &dyn Fn(&str) -> f64,
) -> Group {
    let mut props: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut sample = String::new();
    let mut best: Vec<(f32, usize)> = Vec::new();
    for row in &clips {
        let mut top: Option<(f32, usize)> = None;
        for &i in &row.segs {
            let Some(seg) = index.segments.get(i) else {
                continue;
            };
            if let Some(main) = seg.caption.as_ref().and_then(|c| c.people.first()) {
                let kind = prop_kind(&main.prop);
                if kind != "none" {
                    *props.entry(kind).or_default() += 1;
                }
                if sample.is_empty() && !main.appearance.is_empty() {
                    sample.clone_from(&main.appearance);
                }
            }
            if top.is_none_or(|(q, _)| seg.q > q) {
                top = Some((seg.q, i));
            }
        }
        best.extend(top);
    }
    best.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut props: Vec<(&'static str, usize)> = props.into_iter().collect();
    props.sort_by_key(|e| std::cmp::Reverse(e.1));
    Group {
        segs: clips.iter().map(|c| c.segs.len()).sum(),
        start: clips
            .iter()
            .map(|c| start_of(&c.clip))
            .fold(f64::INFINITY, f64::min),
        strip: best.into_iter().take(STRIP).map(|(_, i)| i).collect(),
        name,
        clips,
        props,
        sample,
    }
}

/// Orders the person grid.
fn sort(groups: &mut [Group], order: Order) {
    match order {
        Order::Time => groups.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.name.cmp(&b.name))),
        Order::Name => groups.sort_by(|a, b| a.name.cmp(&b.name)),
        Order::Size => groups.sort_by(|a, b| b.segs.cmp(&a.segs).then(a.name.cmp(&b.name))),
    }
}

/// The model, rebuilt when what it is built from changes.
#[derive(Debug, Default)]
pub(super) struct Cache {
    key: Option<(PathBuf, People, usize, Order)>,
    model: Model,
}

impl Cache {
    /// The model for the open night.
    pub(super) fn get(&mut self, project: &Project, order: Order) -> &Model {
        let segments = project.index.as_ref().map_or(0, |i| i.segments.len());
        let fresh = self.key.as_ref().is_some_and(|(work, people, n, o)| {
            *work == project.work && *people == project.people && *n == segments && *o == order
        });
        if !fresh {
            self.model = Model::build(&project.people, project.index.as_ref(), order);
            self.key = Some((
                project.work.clone(),
                project.people.clone(),
                segments,
                order,
            ));
        }
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::{Model, Order};
    use crate::people::People;

    #[test]
    fn people_without_footage_still_show() -> anyhow::Result<()> {
        let people: People = toml::from_str("[names]\n\"MVI_1\" = \"ava\"\n")?;
        let model = Model::build(&people, None, Order::Time);
        assert_eq!(model.groups.len(), 1, "one person");
        assert_eq!(model.groups[0].label(), "ava", "named");
        assert_eq!(model.unassigned.label(), "Unassigned", "the pile");
        Ok(())
    }
}

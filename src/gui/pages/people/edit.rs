// ============================================================================
// Module:       gui::pages::people::edit
// Description:  Every change the People tab makes to the people map: applied,
//               saved to people.toml, reloaded, and undoable.
//
// Dependencies: crate::{people, project}, super::model
// ============================================================================

//! People-map edits.
//!
//! An edit is a function over a copy of the map. [`History::apply`] runs it,
//! and when the map changed writes `people.toml` (atomically, with the
//! descriptive comments regenerated), reloads it into the project so every
//! page sees the same map, and keeps the map as it was for undo.

use crate::index::Index;
use crate::people::People;
use crate::project::Project;

use super::model::Model;

/// Edits kept for undo.
const DEPTH: usize = 30;

/// Undo and redo stacks of whole maps (they are small).
#[derive(Debug, Default)]
pub(super) struct History {
    undo: Vec<(String, People)>,
    redo: Vec<(String, People)>,
}

impl History {
    /// Applies `edit` to the open night's map; returns what happened, for
    /// the status line.
    pub(super) fn apply(
        &mut self,
        project: &mut Project,
        label: &str,
        edit: impl FnOnce(&mut People),
    ) -> String {
        let before = project.people.clone();
        let mut next = before.clone();
        edit(&mut next);
        if next == before {
            return format!("{label}: nothing changed");
        }
        if let Err(e) = write(project, &next) {
            return format!("{label} failed: {e:#}");
        }
        self.undo.push((label.to_owned(), before));
        if self.undo.len() > DEPTH {
            self.undo.remove(0);
        }
        self.redo.clear();
        format!("{label} · Ctrl+Z to undo")
    }

    /// Puts back the map as it was before the last edit.
    pub(super) fn undo(&mut self, project: &mut Project) -> String {
        let Some((label, map)) = self.undo.pop() else {
            return "Nothing to undo".to_owned();
        };
        let now = project.people.clone();
        if let Err(e) = write(project, &map) {
            self.undo.push((label, map));
            return format!("Undo failed: {e:#}");
        }
        self.redo.push((label.clone(), now));
        format!("Undid {label} · Ctrl+Shift+Z to redo")
    }

    /// Re-applies the last undone edit.
    pub(super) fn redo(&mut self, project: &mut Project) -> String {
        let Some((label, map)) = self.redo.pop() else {
            return "Nothing to redo".to_owned();
        };
        let now = project.people.clone();
        if let Err(e) = write(project, &map) {
            self.redo.push((label, map));
            return format!("Redo failed: {e:#}");
        }
        self.undo.push((label.clone(), now));
        format!("Redid {label}")
    }

    /// The label of the edit undo would revert.
    pub(super) fn last(&self) -> Option<&str> {
        self.undo.last().map(|(l, _)| l.as_str())
    }
}

/// Saves `map` as the night's `people.toml` and reloads it.
fn write(project: &mut Project, map: &People) -> anyhow::Result<()> {
    map.save(&project.work, project.index.as_ref())?;
    project.reload_people();
    Ok(())
}

/// Moves the segments `segs` (indices into `index.segments`) to `to`
/// (`None`: to nobody), clip by clip.
pub(super) fn move_segments(
    people: &mut People,
    model: &Model,
    index: &Index,
    segs: &[usize],
    to: Option<&str>,
) {
    let mut by_clip: std::collections::BTreeMap<&str, Vec<usize>> = Default::default();
    for &i in segs {
        let Some(seg) = index.segments.get(i) else {
            continue;
        };
        let Some(order) = model.clip_segs.get(&seg.clip) else {
            continue;
        };
        if let Some(pos) = order.iter().position(|&j| j == i) {
            by_clip.entry(seg.clip.as_str()).or_default().push(pos);
        }
    }
    for (clip, chosen) in by_clip {
        let spans = model.spans(clip, index);
        people.assign_segments(clip, &spans, &chosen, to);
    }
}

/// A name not yet in the map, based on `base`: `base`, `base-2`, `base-3`…
pub(super) fn fresh_name(people: &People, base: &str) -> String {
    let names = people.all_names();
    let base = base.trim();
    let base = if base.is_empty() { "person" } else { base };
    if !names.contains(base) {
        return base.to_owned();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|n| !names.contains(n.as_str()))
        .unwrap_or_else(|| format!("{base}-new"))
}

#[cfg(test)]
mod tests {
    use super::{History, fresh_name};
    use crate::people::People;
    use crate::project::Project;

    #[test]
    fn fresh_names_never_collide() -> anyhow::Result<()> {
        let p: People = toml::from_str("[names]\nA = \"ava\"\nB = \"ava-2\"\n")?;
        assert_eq!(fresh_name(&p, "ava"), "ava-3", "next free suffix");
        assert_eq!(fresh_name(&p, "bo"), "bo", "unused as is");
        assert_eq!(fresh_name(&p, " "), "person", "blank gets a word");
        Ok(())
    }

    #[test]
    fn edits_save_reload_and_undo() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(
            dir.path().join("people.toml"),
            "[names]\n\"MVI_1\" = \"a\"\n\"MVI_2\" = \"b\"\n",
        )?;
        let mut project = Project::open(dir.path())?;
        let mut history = History::default();
        let said = history.apply(&mut project, "Merge", |p| {
            p.merge(&["b".to_owned()], "a");
        });
        assert!(said.contains("undo"), "reported: {said}");
        assert_eq!(project.people.all_names().len(), 1, "merged in memory");
        assert_eq!(
            People::load(dir.path())?.all_names().len(),
            1,
            "merged on disk"
        );
        let same = history.apply(&mut project, "Nothing", |_| {});
        assert!(same.contains("nothing changed"), "no-op: {same}");
        history.undo(&mut project);
        assert_eq!(project.people.all_names().len(), 2, "undone");
        history.redo(&mut project);
        assert_eq!(project.people.all_names().len(), 1, "redone");
        Ok(())
    }
}

// ============================================================================
// Module:       gui::pages::people::queue
// Description:  The confirm queue: one unassigned or doubtful clip at a time
//               with the people map's best guess, answered Y / N / S.
//
// Dependencies: crate::people, super::model
// ============================================================================

//! The confirm queue, Apple Photos' "Confirm additional photos" for clips.
//!
//! The queue holds clips that need a human: those with a performer and no
//! entry, then those whose entry looks doubtful (the clip resembles another
//! person more than the one it is filed under). Each comes with the best
//! guess from [`People::guesses`]: `Y` files the clip there, `N` rules that
//! person out and offers the next guess later if one is plausible, `S`
//! skips (never confirm when unsure). It advances on every answer.

use std::collections::{BTreeSet, VecDeque};

use crate::people::People;

use super::model::Model;

/// Guesses scoring below this are not worth asking about.
pub(super) const PLAUSIBLE: f32 = 0.2;

/// A filed clip is doubtful when its own person scores below this and
/// someone else scores higher.
const DOUBT: f32 = 0.35;

/// One clip waiting for an answer.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Item {
    /// Clip id.
    pub(super) clip: String,
    /// Who it is filed under now (`None`: nobody).
    pub(super) current: Option<String>,
}

/// What the user answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Answer {
    /// It is the guessed person.
    Yes,
    /// It is not.
    No,
    /// Not sure.
    Skip,
}

/// The queue.
#[derive(Debug, Default)]
pub(super) struct Queue {
    /// Clips still to answer, front first.
    pub(super) items: VecDeque<Item>,
    /// `(clip, name)` pairs the user said no to.
    rejected: BTreeSet<(String, String)>,
    /// Answered so far.
    pub(super) answered: usize,
    /// Filed by a yes.
    pub(super) confirmed: usize,
}

/// A question: the clip and who it might be.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Question {
    /// The clip.
    pub(super) item: Item,
    /// The best guess and its score.
    pub(super) guess: (String, f32),
    /// The next few guesses.
    pub(super) others: Vec<(String, f32)>,
}

impl Queue {
    /// The queue for the night as it stands.
    pub(super) fn build(people: &People, model: &Model) -> Self {
        let mut items = VecDeque::new();
        let mut seen = BTreeSet::new();
        for row in &model.unassigned.clips {
            if seen.insert(row.clip.clone()) {
                items.push_back(Item {
                    clip: row.clip.clone(),
                    current: None,
                });
            }
        }
        for group in &model.groups {
            let Some(name) = group.name.as_deref() else {
                continue;
            };
            if group.clips.len() < 2 {
                continue;
            }
            for row in &group.clips {
                let guesses = people.guesses(&model.profiles, &row.clip);
                let own = guesses
                    .iter()
                    .find(|(n, _)| n == name)
                    .map_or(0.0, |(_, s)| *s);
                let better = guesses.first().is_some_and(|(n, s)| n != name && *s > own);
                if own < DOUBT && better && seen.insert(row.clip.clone()) {
                    items.push_back(Item {
                        clip: row.clip.clone(),
                        current: Some(name.to_owned()),
                    });
                }
            }
        }
        Self {
            items,
            ..Self::default()
        }
    }

    /// The question at the front, dropping clips with no plausible guess
    /// left.
    pub(super) fn question(&mut self, people: &People, model: &Model) -> Option<Question> {
        while let Some(item) = self.items.front().cloned() {
            let open: Vec<(String, f32)> = people
                .guesses(&model.profiles, &item.clip)
                .into_iter()
                .filter(|(n, s)| {
                    *s >= PLAUSIBLE
                        && Some(n) != item.current.as_ref()
                        && !self.rejected.contains(&(item.clip.clone(), n.clone()))
                })
                .collect();
            let mut open = open.into_iter();
            if let Some(guess) = open.next() {
                return Some(Question {
                    item,
                    guess,
                    others: open.take(3).collect(),
                });
            }
            self.items.pop_front();
        }
        None
    }

    /// Records the answer to `question`; returns the name to file the clip
    /// under when the answer was yes.
    pub(super) fn answer(&mut self, question: &Question, answer: Answer) -> Option<String> {
        self.answered += 1;
        let item = self.items.pop_front();
        match answer {
            Answer::Yes => {
                self.confirmed += 1;
                Some(question.guess.0.clone())
            }
            Answer::No => {
                self.rejected
                    .insert((question.item.clip.clone(), question.guess.0.clone()));
                // Ask about the next guess later, if there is one.
                if let Some(item) = item
                    && !question.others.is_empty()
                {
                    self.items.push_back(item);
                }
                None
            }
            Answer::Skip => None,
        }
    }

    /// Clips left, the current one included.
    pub(super) fn left(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{Answer, Item, Question, Queue};

    fn question(clip: &str, guess: &str, others: &[&str]) -> Question {
        Question {
            item: Item {
                clip: clip.to_owned(),
                current: None,
            },
            guess: (guess.to_owned(), 0.5),
            others: others.iter().map(|o| ((*o).to_owned(), 0.3)).collect(),
        }
    }

    fn queue(clips: &[&str]) -> Queue {
        Queue {
            items: clips
                .iter()
                .map(|c| Item {
                    clip: (*c).to_owned(),
                    current: None,
                })
                .collect(),
            ..Queue::default()
        }
    }

    #[test]
    fn yes_files_the_clip_and_advances() {
        let mut q = queue(&["A", "B"]);
        let filed = q.answer(&question("A", "ava", &[]), Answer::Yes);
        assert_eq!(filed.as_deref(), Some("ava"), "filed under the guess");
        assert_eq!(q.left(), 1, "advanced");
        assert_eq!(q.confirmed, 1, "counted");
    }

    #[test]
    fn no_comes_back_only_with_another_guess() {
        let mut q = queue(&["A", "B"]);
        assert!(
            q.answer(&question("A", "ava", &["bo"]), Answer::No)
                .is_none(),
            "nothing filed"
        );
        assert_eq!(
            q.items.back().map(|i| i.clip.as_str()),
            Some("A"),
            "asked again later"
        );
        let mut q = queue(&["A", "B"]);
        q.answer(&question("A", "ava", &[]), Answer::No);
        assert_eq!(q.left(), 1, "no other guess: dropped");
        q.answer(&question("B", "ava", &[]), Answer::Skip);
        assert_eq!(q.left(), 0, "skipped");
    }
}

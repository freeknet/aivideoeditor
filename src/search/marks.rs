// ============================================================================
// Module:       search::marks
// Description:  Review marks: pick / reject and a 1-5 rating per shot-log
//               segment, kept in <work>/review.toml.
//
// Dependencies: toml, anyhow
// ============================================================================

//! The human's verdicts on segments.
//!
//! Marks are a signal the planner can use: a picked segment is one the
//! editor has seen and wants, a rejected one must not be used whatever its
//! measured quality says, and a rating orders the picks. They are written by
//! the Shots browser (P / X / U, 1-5) and are plain text, so they can be
//! read, diffed and edited by hand.
//!
//! ## `<work>/review.toml`
//!
//! ```toml
//! # Review marks: verdict = "pick" | "reject", rating = 1..5.
//! [marks]
//! "MVI_7925/3" = { verdict = "pick", rating = 4 }
//! "MVI_7930/1" = { verdict = "reject" }
//! "MVI_7987/2" = { rating = 2 }
//! ```
//!
//! Short forms are accepted on read: `"MVI_7925/3" = "pick"` and
//! `"MVI_7925/3" = 4`, and a `[marks."MVI_7925/3"]` table works the same as
//! the inline one. Parsing is forgiving: an entry that does not make sense
//! (an unknown verdict, a rating of 9) is skipped with a warning and costs
//! only itself, never the file. Writes go through a temporary file and a
//! rename, so a crash mid-write cannot leave half a file.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};

/// The file name inside the work folder.
pub(crate) const FILE: &str = "review.toml";

/// Keep or drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Wanted.
    Pick,
    /// Never use.
    Reject,
}

impl Verdict {
    /// The word in the file.
    fn word(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Reject => "reject",
        }
    }

    /// The verdict a word in the file stands for.
    fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "pick" | "picked" | "keep" | "yes" => Some(Self::Pick),
            "reject" | "rejected" | "drop" | "no" => Some(Self::Reject),
            _ => None,
        }
    }
}

/// One segment's mark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Mark {
    /// Pick, reject, or not yet decided.
    pub(crate) verdict: Option<Verdict>,
    /// Stars 1..=5, 0 for none.
    pub(crate) rating: u8,
}

impl Mark {
    /// Whether the mark says nothing (and need not be stored).
    fn is_empty(self) -> bool {
        self.verdict.is_none() && self.rating == 0
    }
}

/// Every segment's mark.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Marks {
    marks: BTreeMap<String, Mark>,
}

impl Marks {
    /// Loads `<work>/review.toml`; a missing file is no marks. Problems come
    /// back as warnings next to whatever could be read.
    pub(crate) fn load(work: &Path) -> (Self, Vec<String>) {
        match std::fs::read_to_string(work.join(FILE)) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Self::default(), Vec::new()),
            Err(e) => (Self::default(), vec![format!("{FILE}: {e}")]),
        }
    }

    /// Parses the file's text, keeping every entry that makes sense.
    pub(crate) fn parse(text: &str) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut marks = BTreeMap::new();
        let table = match text.parse::<toml::Table>() {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("{FILE}: {e}"));
                return (Self { marks }, warnings);
            }
        };
        let Some(toml::Value::Table(entries)) = table.get("marks") else {
            return (Self { marks }, warnings);
        };
        for (id, value) in entries {
            match entry(value) {
                Ok(mark) if !mark.is_empty() => {
                    marks.insert(id.clone(), mark);
                }
                Ok(_) => {}
                Err(why) => warnings.push(format!("{FILE}: {id}: {why}")),
            }
        }
        (Self { marks }, warnings)
    }

    /// The file's text.
    pub(crate) fn to_toml(&self) -> String {
        let mut s = String::from(
            "# Review marks from ave's Shots browser: verdict = \"pick\" | \"reject\", rating = 1..5.\n\
             # The planner treats rejects as unusable and prefers picks, best rated first.\n\
             [marks]\n",
        );
        for (id, m) in &self.marks {
            let mut fields = Vec::new();
            if let Some(v) = m.verdict {
                fields.push(format!("verdict = \"{}\"", v.word()));
            }
            if m.rating > 0 {
                fields.push(format!("rating = {}", m.rating));
            }
            let key = toml::Value::String(id.clone()).to_string();
            let _ = writeln!(s, "{key} = {{ {} }}", fields.join(", "));
        }
        s
    }

    /// Writes `<work>/review.toml` through a temporary file and a rename.
    pub(crate) fn save(&self, work: &Path) -> Result<()> {
        let path = work.join(FILE);
        let tmp = work.join(format!("{FILE}.tmp"));
        std::fs::write(&tmp, self.to_toml())
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// The mark on a segment (empty when none).
    pub(crate) fn get(&self, id: &str) -> Mark {
        self.marks.get(id).copied().unwrap_or_default()
    }

    /// Sets or clears a segment's verdict.
    pub(crate) fn set_verdict(&mut self, id: &str, verdict: Option<Verdict>) {
        let mut m = self.get(id);
        m.verdict = verdict;
        self.put(id, m);
    }

    /// Sets a segment's rating (0 clears it; above 5 is 5).
    pub(crate) fn set_rating(&mut self, id: &str, rating: u8) {
        let mut m = self.get(id);
        m.rating = rating.min(5);
        self.put(id, m);
    }

    /// How many segments are picked and rejected.
    pub(crate) fn counts(&self) -> (usize, usize) {
        let picks = self
            .marks
            .values()
            .filter(|m| m.verdict == Some(Verdict::Pick))
            .count();
        let rejects = self
            .marks
            .values()
            .filter(|m| m.verdict == Some(Verdict::Reject))
            .count();
        (picks, rejects)
    }

    /// Stores a mark, dropping it when empty.
    fn put(&mut self, id: &str, mark: Mark) {
        if mark.is_empty() {
            self.marks.remove(id);
        } else {
            self.marks.insert(id.to_owned(), mark);
        }
    }
}

/// One `[marks]` entry to a mark.
fn entry(value: &toml::Value) -> Result<Mark, String> {
    let rating = |v: &toml::Value| -> Result<u8, String> {
        match v.as_integer() {
            Some(r @ 0..=5) => u8::try_from(r).map_err(|e| e.to_string()),
            Some(r) => Err(format!("rating {r} is not 0..5")),
            None => Err(format!("rating {v} is not a number")),
        }
    };
    match value {
        toml::Value::String(word) => Verdict::parse(word)
            .map(|v| Mark {
                verdict: Some(v),
                rating: 0,
            })
            .ok_or_else(|| format!("unknown verdict {word:?}")),
        toml::Value::Integer(_) => Ok(Mark {
            verdict: None,
            rating: rating(value)?,
        }),
        toml::Value::Table(t) => {
            let verdict = match t.get("verdict") {
                None => None,
                Some(toml::Value::String(w)) if w.trim().is_empty() || w == "none" => None,
                Some(toml::Value::String(w)) => {
                    Some(Verdict::parse(w).ok_or_else(|| format!("unknown verdict {w:?}"))?)
                }
                Some(other) => return Err(format!("verdict {other} is not a word")),
            };
            let rating = t.get("rating").map_or(Ok(0), rating)?;
            Ok(Mark { verdict, rating })
        }
        toml::Value::Float(_)
        | toml::Value::Boolean(_)
        | toml::Value::Datetime(_)
        | toml::Value::Array(_) => Err(format!("{value} is not a mark")),
    }
}

#[cfg(test)]
mod tests {
    use super::{Mark, Marks, Verdict};

    #[test]
    fn marks_round_trip_through_the_file() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut marks = Marks::default();
        marks.set_verdict("MVI_7925/3", Some(Verdict::Pick));
        marks.set_rating("MVI_7925/3", 4);
        marks.set_verdict("MVI_7930/1", Some(Verdict::Reject));
        marks.set_rating("MVI_7987/2", 9);
        marks.save(dir.path())?;
        let (back, warnings) = Marks::load(dir.path());
        assert!(warnings.is_empty(), "clean file: {warnings:?}");
        assert_eq!(back, marks, "round trip");
        assert_eq!(back.get("MVI_7987/2").rating, 5, "ratings cap at 5");
        assert_eq!(back.counts(), (1, 1), "one pick, one reject");
        assert!(
            !dir.path().join("review.toml.tmp").exists(),
            "the temporary file is renamed away"
        );
        Ok(())
    }

    #[test]
    fn clearing_a_mark_removes_it() {
        let mut marks = Marks::default();
        marks.set_verdict("a/1", Some(Verdict::Pick));
        marks.set_verdict("a/1", None);
        assert_eq!(marks, Marks::default(), "an empty mark is not stored");
        assert_eq!(marks.get("a/1"), Mark::default(), "absent reads as empty");
    }

    #[test]
    fn a_bad_entry_costs_only_itself() {
        let text = r#"
[marks]
"a/1" = "pick"
"a/2" = 3
"a/3" = { verdict = "maybe" }
"a/4" = { rating = 12 }
"a/5" = { verdict = "reject", rating = 1 }
"a/6" = 1.5

[marks."a/7"]
verdict = "keep"
"#;
        let (marks, warnings) = Marks::parse(text);
        assert_eq!(warnings.len(), 3, "three bad entries: {warnings:?}");
        assert_eq!(
            marks.get("a/1").verdict,
            Some(Verdict::Pick),
            "short verdict"
        );
        assert_eq!(marks.get("a/2").rating, 3, "short rating");
        assert_eq!(
            marks.get("a/5"),
            Mark {
                verdict: Some(Verdict::Reject),
                rating: 1
            },
            "inline table"
        );
        assert_eq!(marks.get("a/7").verdict, Some(Verdict::Pick), "table form");
        let (none, warnings) = Marks::parse("this is not toml = = =");
        assert_eq!(none, Marks::default(), "unparseable is empty");
        assert_eq!(warnings.len(), 1, "and says why");
    }
}

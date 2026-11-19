// ============================================================================
// Module:       gui::assistant::skills
// Description:  The agent's skills (agent/skills/<name>/SKILL.md, built into
//               the binary): their front matter, matching a message against
//               their triggers, and writing them where Claude Code can Read.
//
// Dependencies: anyhow
// ============================================================================

//! Skills.
//!
//! A skill is a step-by-step recipe in `agent/skills/<name>/SKILL.md` whose
//! front matter names it, describes it and lists trigger phrases. The files
//! are compiled in, so the app works wherever it is installed; before a
//! Claude Code session starts they are written to a folder the agent may
//! Read ([`install`]). Matching is ours, as in Selects: when a message
//! contains a trigger phrase, the per-turn reminder names the skill and its
//! file, and the system prompt tells the agent to Read it first.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The skill files, by folder name.
const FILES: [(&str, &str); 4] = [
    (
        "beat-sync-reel",
        include_str!("../../../agent/skills/beat-sync-reel/SKILL.md"),
    ),
    (
        "suggest-moments",
        include_str!("../../../agent/skills/suggest-moments/SKILL.md"),
    ),
    (
        "fix-check",
        include_str!("../../../agent/skills/fix-check/SKILL.md"),
    ),
    (
        "weekly-deliverables",
        include_str!("../../../agent/skills/weekly-deliverables/SKILL.md"),
    ),
];

/// One skill's front matter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Skill {
    /// Its name (the folder).
    pub(crate) name: String,
    /// One line: what it is for.
    pub(crate) description: String,
    /// Phrases that make it relevant, lowercase.
    pub(crate) triggers: Vec<String>,
}

/// Reads the `---` front matter of a SKILL.md: `name:`, `description:` and
/// `triggers: [a, b, c]`.
pub(crate) fn parse(folder: &str, text: &str) -> Skill {
    let mut skill = Skill {
        name: folder.to_owned(),
        description: String::new(),
        triggers: Vec::new(),
    };
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return skill;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "name" if !value.is_empty() => value.clone_into(&mut skill.name),
            "description" => value.clone_into(&mut skill.description),
            "triggers" => {
                skill.triggers = value
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .split(',')
                    .map(|t| t.trim().trim_matches('"').to_lowercase())
                    .filter(|t| !t.is_empty())
                    .collect();
            }
            _ => {}
        }
    }
    skill
}

/// Every built-in skill.
pub(crate) fn all() -> Vec<Skill> {
    FILES.iter().map(|(f, t)| parse(f, t)).collect()
}

/// The skills a message is about: those with a trigger phrase in it, as
/// whole words, most matching phrases first.
pub(crate) fn matching(message: &str) -> Vec<Skill> {
    let text = format!(" {} ", normalise(message));
    let mut hits: Vec<(usize, Skill)> = all()
        .into_iter()
        .filter_map(|s| {
            let n = s
                .triggers
                .iter()
                .filter(|t| text.contains(&format!(" {} ", normalise(t))))
                .count();
            (n > 0).then_some((n, s))
        })
        .collect();
    hits.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
    hits.into_iter().map(|(_, s)| s).collect()
}

/// Lowercase, punctuation to spaces, single spaces.
fn normalise(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Writes the skills to `<dir>/<name>/SKILL.md` (only files that differ)
/// and returns `dir`.
pub(crate) fn install(dir: &Path) -> Result<PathBuf> {
    for (folder, text) in FILES {
        let path = dir.join(folder).join("SKILL.md");
        if std::fs::read_to_string(&path).ok().as_deref() == Some(text) {
            continue;
        }
        let parent = dir.join(folder);
        std::fs::create_dir_all(&parent)
            .with_context(|| format!("creating {}", parent.display()))?;
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(dir.to_path_buf())
}

/// The file of skill `name` under `dir`.
pub(crate) fn file(dir: &Path, name: &str) -> PathBuf {
    dir.join(name).join("SKILL.md")
}

/// One line per skill for the system prompt.
pub(crate) fn listing(dir: &Path) -> String {
    all()
        .iter()
        .map(|s| {
            format!(
                "- {}: {} ({})",
                s.name,
                s.description,
                file(dir, &s.name).display()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_skill_has_front_matter() -> anyhow::Result<()> {
        let all = super::all();
        assert_eq!(all.len(), 4, "four skills");
        for s in &all {
            assert!(!s.description.is_empty(), "{} has a description", s.name);
            assert!(s.triggers.len() >= 3, "{} has triggers", s.name);
        }
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "beat-sync-reel",
                "suggest-moments",
                "fix-check",
                "weekly-deliverables"
            ],
            "names match folders"
        );
        Ok(())
    }

    #[test]
    fn messages_match_triggers_as_whole_phrases() -> anyhow::Result<()> {
        let names =
            |m: &str| -> Vec<String> { super::matching(m).into_iter().map(|s| s.name).collect() };
        assert_eq!(
            names("Fix the check, please!"),
            ["fix-check"],
            "punctuation ignored"
        );
        assert_eq!(
            names("Make a 30s promo reel cut to the drop"),
            ["beat-sync-reel"],
            "several phrases, one skill"
        );
        assert_eq!(
            names("who are the best moments per performer?"),
            ["suggest-moments"],
            "highlights"
        );
        assert!(names("the problemset").is_empty(), "no match inside a word");
        assert!(names("hello").is_empty(), "nothing relevant");
        Ok(())
    }

    #[test]
    fn install_writes_readable_files() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = super::install(dir.path())?;
        let text = std::fs::read_to_string(super::file(&root, "fix-check"))?;
        assert!(text.contains("name: fix-check"), "written whole");
        assert!(
            super::listing(&root).contains("weekly-deliverables"),
            "listed"
        );
        super::install(dir.path())?;
        Ok(())
    }

    #[test]
    fn front_matter_parses_and_tolerates_missing() -> anyhow::Result<()> {
        let s = super::parse(
            "x",
            "---\nname: y\ndescription: d\ntriggers: [A b, \"c\"]\n---\nbody",
        );
        assert_eq!(s.name, "y", "name");
        assert_eq!(s.triggers, ["a b", "c"], "lowercased, unquoted");
        let bare = super::parse("z", "no front matter");
        assert_eq!(bare.name, "z", "falls back to the folder");
        Ok(())
    }
}

// ============================================================================
// Module:       changelog (example)
// Description:  Regenerates CHANGELOG.md from the git history, grouping
//               conventional-commit subjects under the release tag that
//               shipped them.
//
// Dependencies: anyhow; std::process::Command (shells out to git). No crates
//               beyond what the library already pulls in — the calendar
//               conversion is inlined rather than adding a date crate.
// ============================================================================

//! Regenerates `CHANGELOG.md` — run it with:
//!
//! ```sh
//! cargo run --example changelog
//! ```
//!
//! check the committed file against the history without rewriting it:
//!
//! ```sh
//! cargo run --example changelog -- --check
//! ```
//!
//! or write the section for a release that is *about to be tagged*, so
//! the tag can be placed on a commit whose changelog is already final:
//!
//! ```sh
//! cargo run --example changelog -- --release v0.2.2
//! ```
//!
//! Write the release section before tagging so the tagged tree carries
//! the finished changelog. The commit that adds the generated changelog
//! cannot list itself (its hash does not exist while
//! the file is being written), which is why commits touching only
//! `CHANGELOG.md` are excluded from every section, on the generate and
//! check paths alike — the two must always agree.
//!
//! The changelog is derived, not authored. Every entry is a
//! conventional-commit subject (`fix:`, `feat:`, `refactor:` …) resolved
//! against `git tag`, so the released sections cannot say something the
//! history does not. That is the whole point: a hand-maintained changelog
//! drifts silently, and the drift is only ever discovered on release day.
//!
//! Consequences worth knowing before you edit the file by hand:
//!
//! - **Released sections are rewritten on every run, except their
//!   summary.** The `### ` lists under a release heading are regenerated
//!   from the history, and edits to them are lost. Prose between the
//!   heading and its first `### ` is the release's summary: it is read back
//!   from the file and kept, so a release can open with a paragraph a
//!   person wrote. `release.yml` publishes the whole section, summary and
//!   lists, as the release notes.
//! - **Releases from before the generator are kept whole.** A version in
//!   [`PRE_GENERATOR`] was released with a hand-written section; that
//!   section, heading and all, is copied from the file as it stands rather
//!   than regenerated, because what shipped in the tag is what it says.
//! - **A wrong subject is corrected in `CHANGELOG.corrections`**, never by
//!   rewording a pushed commit. See [`corrections`].
//! - **`--check` compares released sections only.** The `Unreleased`
//!   section is deliberately exempt, because its content changes with
//!   every merge — and a squash or rebase rewrites the very hashes it
//!   would be pinned to. Gating a pull request on it would fail every
//!   pull request for no signal.
//!
//! - **A version tag no branch can reach is a hard error under `--check`.**
//!   Each release is computed as `<previous>..<tag>`, so a tag left pointing
//!   at an amended-away commit makes every range from there on wrong. See
//!   [`orphan_report`] for the symptom and the fix.
//!
//! An example rather than a test because it writes into the source tree.
//! It is still built by `cargo clippy --all-targets` so it cannot rot silently.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The generated file, relative to the crate root.
const CHANGELOG: &str = "CHANGELOG.md";

/// Used to build the commit and compare links. Taken from `Cargo.toml`'s
/// `repository` field rather than guessed.
const REPO: &str = "https://github.com/freeknet/aivideoeditor";

/// Heading for commits that have not shipped in a tagged release yet.
const UNRELEASED: &str = "## [Unreleased]";

/// Versions released before this file was generated. Their sections were
/// written by hand and shipped that way in the tag, so they are kept
/// verbatim, heading included, rather than regenerated from the history.
/// Nothing is ever added here: every release after these is generated.
const PRE_GENERATOR: &[&str] = &["0.1.0"];

/// The file of corrected subjects, at the crate root.
const CORRECTIONS: &str = "CHANGELOG.corrections";

/// The line the generator writes for a tag with nothing under it. It is
/// output, not a summary, so it is never read back as one.
const NO_COMMITS: &str = "No commits recorded against this tag.";

/// Field separator for `git log --format`. `%x1f` is the ASCII unit
/// separator, which cannot occur in a commit subject.
const SEP: char = '\u{1f}';

/// Conventional-commit types, in the order their sections appear within a
/// release, mapped to the heading each lands under.
///
/// Anything whose subject does not parse as a conventional commit falls to
/// `Other`.
const SECTIONS: &[(&str, &[&str])] = &[
    ("Added", &["feat"]),
    ("Fixed", &["fix"]),
    ("Performance", &["perf"]),
    ("Changed", &["refactor", "style"]),
    ("Documentation", &["docs"]),
    ("Internal", &["build", "ci", "chore"]),
    ("Tests", &["test"]),
];

/// Heading for breaking changes, which lead a release regardless of the
/// type they were committed under.
const BREAKING: &str = "Breaking changes";

/// Heading for subjects that are not conventional commits.
const OTHER: &str = "Other";

/// One commit, already parsed into the pieces a changelog line needs.
struct Commit {
    hash: String,
    short: String,
    /// The heading this commit belongs under.
    section: &'static str,
    /// The `(scope)` of a conventional commit, if it carried one.
    scope: Option<String>,
    /// The subject with the `type(scope):` prefix stripped.
    summary: String,
}

/// What the committed file holds for a release that a regeneration keeps.
#[derive(Debug, PartialEq, Eq)]
enum Kept {
    /// A [`PRE_GENERATOR`] section: its heading line and body, as written.
    Verbatim { heading: String, body: String },
    /// The prose between a generated section's heading and its first
    /// `### `.
    Summary(String),
}

/// One tagged release and everything that landed since the previous tag.
struct Release {
    /// The tag as git spells it, e.g. `v0.2.0`.
    tag: String,
    /// The tag without its leading `v`, e.g. `0.2.0`.
    version: String,
    /// Committer date of the tagged commit, `YYYY-MM-DD`.
    date: String,
    /// The tag this release is compared against, if any.
    previous: Option<String>,
    commits: Vec<Commit>,
}

fn main() -> Result<()> {
    let mut check = false;
    let mut release: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => check = true,
            "--release" => {
                let Some(version) = args.next() else {
                    bail!("--release needs a version, e.g. `--release v0.2.2`");
                };
                release = Some(version);
            }
            _ => {
                bail!("unknown argument `{arg}`; usage: changelog [--check | --release <version>]")
            }
        }
    }
    if check && release.is_some() {
        bail!("--check and --release are mutually exclusive");
    }

    if !Path::new("Cargo.toml").is_file() {
        bail!("run this from the crate root, where Cargo.toml lives");
    }

    let mut releases = releases()?;
    // What the file already says that a regeneration keeps: summaries, and
    // the sections of releases older than the generator. A missing file
    // keeps nothing.
    let existing = fs::read_to_string(CHANGELOG).ok();
    let kept = existing.as_deref().map(kept_sections).unwrap_or_default();

    // An orphaned tag makes every range below wrong, so it is reported
    // before any mismatch it would itself have caused.
    let orphaned = orphaned_tags(&releases)?;

    if check {
        if !orphaned.is_empty() {
            bail!("{}", orphan_report(&orphaned));
        }
        let Some(existing) = existing else {
            bail!("{CHANGELOG} is missing; run `cargo run --example changelog`");
        };
        // A leading section whose tag does not exist yet is the pre-tag
        // release flow in progress, not drift — but it only passes if it
        // is exactly the release about to be cut: version newer than the
        // latest tag, matching Cargo.toml, content matching the history.
        if let Some(version) = pending_version(&existing, &releases) {
            let pending = pending_release(&version, &releases).with_context(|| {
                format!(
                    "{CHANGELOG} has a [{version}] section but no such tag exists, \
                     and it does not validate as a release about to be cut"
                )
            })?;
            releases.insert(0, pending);
        }
        let generated = render(&releases, &[], &kept);
        if released_only(&existing) == released_only(&generated) {
            println!("{CHANGELOG} matches the git history.");
            return Ok(());
        }
        bail!(
            "{CHANGELOG} is out of date with the git history.\n\
             Regenerate it with `cargo run --example changelog` and commit the result."
        );
    }

    let unreleased = match &release {
        // The pending release swallows everything that would have been
        // unreleased; the section it lands under is the one being cut.
        Some(version) => {
            let pending = pending_release(version, &releases)?;
            releases.insert(0, pending);
            Vec::new()
        }
        None => match releases.first() {
            Some(latest) => commits(&format!("{}..HEAD", latest.tag))?,
            None => commits("HEAD")?,
        },
    };
    let generated = render(&releases, &unreleased, &kept);

    // A warning rather than an error on this path: the file still gets
    // written, so you are not locked out of regenerating while you sort the
    // tag out. `--check` is where it becomes a build failure.
    if !orphaned.is_empty() {
        eprintln!("warning: {}", orphan_report(&orphaned));
    }

    fs::write(CHANGELOG, &generated).with_context(|| format!("failed to write {CHANGELOG}"))?;
    println!(
        "wrote {CHANGELOG}: {} release(s), {} unreleased commit(s).",
        releases.len(),
        unreleased.len()
    );
    Ok(())
}

/// Runs git and returns its stdout, turning a non-zero exit into an error
/// rather than an empty string that would silently produce a blank section.
fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to run `git {}`", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`git {}` failed: {}", args.join(" "), stderr.trim());
    }

    String::from_utf8(output.stdout)
        .with_context(|| format!("`git {}` printed invalid UTF-8", args.join(" ")))
}

/// Every version tag, newest release first.
fn releases() -> Result<Vec<Release>> {
    let mut tags: Vec<(String, (u64, u64, u64))> = Vec::new();
    for line in git(&["tag", "--list"])?.lines() {
        let tag = line.trim();
        if tag.is_empty() {
            continue;
        }
        if let Some(version) = parse_version(tag) {
            tags.push((tag.to_string(), version));
        }
    }

    // Oldest first, so each tag knows the one it follows.
    tags.sort_by_key(|(_, version)| *version);

    let mut releases = Vec::new();
    for (index, (tag, _)) in tags.iter().enumerate() {
        let previous = index
            .checked_sub(1)
            .and_then(|i| tags.get(i))
            .map(|(t, _)| t.clone());
        let range = match &previous {
            Some(prev) => format!("{prev}..{tag}"),
            // The first release reaches back to the root commit.
            None => tag.clone(),
        };
        releases.push(Release {
            version: tag.trim_start_matches('v').to_string(),
            date: tag_date(tag)?,
            previous,
            commits: commits(&range)?,
            tag: tag.clone(),
        });
    }

    // Newest first, which is the order a changelog is read in.
    releases.reverse();
    Ok(releases)
}

/// Every ref a tag is reachable from.
///
/// Empty means the tag points into history that no branch can reach — the
/// tagged commit was amended or rebased afterwards, leaving the tag
/// pointing at discarded history and making its release range misleading.
fn refs_containing(tag: &str) -> Result<Vec<String>> {
    let refs = git(&[
        "for-each-ref",
        "--contains",
        tag,
        "--format=%(refname)",
        "refs/heads",
        "refs/remotes",
    ])?;
    Ok(refs
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Version tags pointing at unreachable commits.
///
/// A checkout with no branch refs at all cannot answer the question, so
/// this reports nothing rather than condemning every tag — better to skip
/// the check than to fail a build for the shape of its clone.
fn orphaned_tags(releases: &[Release]) -> Result<Vec<String>> {
    let all_refs = git(&[
        "for-each-ref",
        "--format=%(refname)",
        "refs/heads",
        "refs/remotes",
    ])?;
    if all_refs.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut orphaned = Vec::new();
    for release in releases {
        if refs_containing(&release.tag)?.is_empty() {
            orphaned.push(release.tag.clone());
        }
    }
    Ok(orphaned)
}

/// What to tell someone who has just grown an orphaned tag.
fn orphan_report(tags: &[String]) -> String {
    let list = tags.join(", ");
    format!(
        "{list} point(s) at a commit no branch can reach.\n\
         A tag was placed, then the commit under it was amended or rebased, so the tag \
         now describes history that was thrown away. Every range computed from it is wrong.\n\
         \n\
         Re-point it at the commit that actually shipped, then force-push:\n\
         \n\
         \x20   git tag -f -a <tag> <commit> -m \"aivideoeditor <tag>\"\n\
         \x20   git push --force origin refs/tags/<tag>\n\
         \n\
         Pushing a `v*` tag re-triggers .github/workflows/release.yml, which ends in\n\
         `gh release upload --clobber`. If the release is already published and you do not\n\
         want its assets rebuilt, cancel the run: `gh run cancel <id>`."
    )
}

/// `v1.2.3` or `1.2.3` into a comparable tuple. Anything else is not a
/// release tag and is skipped rather than guessed at.
fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Committer time of the commit a tag points at, as a UTC date.
/// `^{commit}` dereferences an annotated tag, so both tag styles give
/// the same answer.
///
/// UTC rather than the committer's recorded offset, so the date
/// `--release` writes *before* the tag exists (from the wall clock, in
/// UTC) is the same date this reads back after the tag exists — with
/// local dates, a release cut near midnight in a non-UTC timezone would
/// fail its own `--check`.
fn tag_date(tag: &str) -> Result<String> {
    let spec = format!("{tag}^{{commit}}");
    let seconds: u64 = git(&["log", "-1", "--format=%ct", &spec])?
        .trim()
        .parse()
        .with_context(|| format!("{tag} has an unreadable committer timestamp"))?;
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    Ok(utc_date(time))
}

/// A `SystemTime` as a `YYYY-MM-DD` UTC date.
///
/// UTC on both the generate and the check path, so a tag pushed on one
/// side of midnight cannot disagree with a check run on the other. The
/// check compares headings with the date stripped anyway, but a
/// regeneration that flipped the date for no reason would show up as a
/// diff in every release section at once.
fn utc_date(time: std::time::SystemTime) -> String {
    let Ok(elapsed) = time.duration_since(std::time::UNIX_EPOCH) else {
        // Before 1970. Not reachable from a git committer timestamp on
        // this repository, and a wrong date is better than a panic in a
        // tool whose whole job is to be run unattended in CI.
        return "1970-01-01".to_owned();
    };
    let days = (elapsed.as_secs() / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Public-domain "days since the Unix epoch" into (year, month, day) on
/// the proleptic Gregorian calendar, by Howard Hinnant.
///
/// Reproduced here rather than pulled from a date crate: this is the
/// only place in the project that needs a calendar, and `chrono` or
/// `time` would be a dependency of the *library* — audited by
/// `cargo deny`, carried into every build — for one function used by one
/// example.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// The section for a release that is about to be tagged: everything
/// since the latest tag, dated today, under the version the tag will
/// carry — so the tag can then be placed on a tree whose changelog is
/// already finished. Generate this before placing the release tag.
fn pending_release(version: &str, releases: &[Release]) -> Result<Release> {
    let tag = if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    };
    let Some(parsed) = parse_version(&tag) else {
        bail!("`{version}` is not a version; expected something like v0.2.2");
    };
    if git(&["tag", "--list", &tag])?.trim() == tag {
        bail!("{tag} already exists — regenerate without `--release` instead");
    }
    if let Some(latest) = releases.first().filter(|latest| {
        parse_version(&latest.tag).is_some_and(|latest_version| parsed <= latest_version)
    }) {
        bail!("{tag} is not newer than the latest tag, {}", latest.tag);
    }
    // The version bump commits first (see CONTRIBUTING.md), so a
    // mismatch here means the release is being cut against the wrong
    // tree — with the bump forgotten, or under the wrong number.
    let manifest = fs::read_to_string("Cargo.toml").context("failed to read Cargo.toml")?;
    let expected = format!("version = \"{}\"", tag.trim_start_matches('v'));
    let Some(version_line) = manifest
        .lines()
        .find(|l| l.trim().starts_with("version = "))
    else {
        bail!("Cargo.toml has no `version =` line");
    };
    if version_line.trim() != expected {
        bail!(
            "Cargo.toml says `{}` but the release being cut is {tag}; bump the version first",
            version_line.trim()
        );
    }

    let range = match releases.first() {
        Some(latest) => format!("{}..HEAD", latest.tag),
        None => "HEAD".to_string(),
    };
    Ok(Release {
        version: tag.trim_start_matches('v').to_string(),
        date: utc_date(std::time::SystemTime::now()),
        previous: releases.first().map(|r| r.tag.clone()),
        commits: commits(&range)?,
        tag,
    })
}

/// Parsed commits in a revision range, newest first. Merges are excluded —
/// they carry no subject of their own worth listing. So are commits that
/// touch only `CHANGELOG.md`: the changelog commit a release produces
/// cannot list itself (its hash does not exist while the file is being
/// generated), so such commits are defined out of every section instead,
/// which keeps the generate and check paths in exact agreement.
fn commits(range: &str) -> Result<Vec<Commit>> {
    let format = format!("--format=%H{SEP}%h{SEP}%s");
    // `--full-history` matters: a pathspec switches on git's history
    // simplification; full history keeps every commit whose own diff
    // touches the kept paths, which is the semantics actually wanted:
    // "skip a commit only when CHANGELOG.md is all it changed".
    let log = git(&[
        "log",
        "--no-merges",
        "--full-history",
        &format,
        range,
        "--",
        ".",
        ":(exclude)CHANGELOG.md",
    ])?;

    let corrected = corrections()?;
    let mut commits = Vec::new();
    for line in log.lines() {
        let mut fields = line.split(SEP);
        let (Some(hash), Some(short), Some(subject)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let subject = corrected.get(hash).map_or(subject, String::as_str);
        commits.push(parse_subject(hash, short, subject));
    }
    Ok(commits)
}

/// Subjects to use instead of a commit's own, by full hash. A pushed
/// commit cannot be reworded without rewriting every hash after it, which
/// the release rules forbid, so a subject that is wrong is corrected here
/// instead. One correction per line, `<40-hex hash> <subject>`; `#` starts
/// a comment. No file means no corrections.
fn corrections() -> Result<BTreeMap<String, String>> {
    let Ok(text) = fs::read_to_string(CORRECTIONS) else {
        return Ok(BTreeMap::new());
    };
    parse_corrections(&text)
}

/// [`corrections`]'s parser.
fn parse_corrections(text: &str) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((hash, subject)) = line.split_once(' ') else {
            bail!("{CORRECTIONS}:{}: expected `<hash> <subject>`", number + 1);
        };
        if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            bail!(
                "{CORRECTIONS}:{}: `{hash}` is not a full commit hash",
                number + 1
            );
        }
        out.insert(hash.to_ascii_lowercase(), subject.trim().to_owned());
    }
    Ok(out)
}

/// What a regeneration keeps from the committed file, by version: the
/// whole section of a [`PRE_GENERATOR`] release, and the summary prose of
/// any other release that has one.
fn kept_sections(text: &str) -> BTreeMap<String, Kept> {
    let mut sections: Vec<(String, String, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        // The link references that close the file end the last section.
        if line.starts_with("[Unreleased]:") {
            break;
        }
        if let Some(rest) = line.strip_prefix("## [") {
            let version = rest.split_once(']').map_or("", |(version, _)| version);
            sections.push((version.to_owned(), line.to_owned(), Vec::new()));
            continue;
        }
        if let Some((_, _, body)) = sections.last_mut() {
            body.push(line);
        }
    }

    let mut kept = BTreeMap::new();
    for (version, heading, body) in sections {
        if version == "Unreleased" || version.is_empty() {
            continue;
        }
        if PRE_GENERATOR.contains(&version.as_str()) {
            let body = trim_blank_lines(&body);
            kept.insert(version, Kept::Verbatim { heading, body });
            continue;
        }
        let prose: Vec<&str> = body
            .iter()
            .take_while(|line| !line.starts_with("### "))
            .filter(|line| line.trim() != NO_COMMITS)
            .copied()
            .collect();
        let summary = trim_blank_lines(&prose);
        if !summary.is_empty() {
            kept.insert(version, Kept::Summary(summary));
        }
    }
    kept
}

/// `lines` joined, without the blank lines at either end.
fn trim_blank_lines(lines: &[&str]) -> String {
    let start = lines.iter().position(|line| !line.trim().is_empty());
    let end = lines.iter().rposition(|line| !line.trim().is_empty());
    match (start, end) {
        (Some(start), Some(end)) => lines
            .get(start..=end)
            .map(|kept| kept.join("\n"))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Splits a `type(scope)!: summary` subject into its parts, falling back to
/// the `Other` section for anything that is not a conventional commit.
fn parse_subject(hash: &str, short: &str, subject: &str) -> Commit {
    let plain = Commit {
        hash: hash.to_string(),
        short: short.to_string(),
        section: OTHER,
        scope: None,
        summary: subject.trim().to_string(),
    };

    let Some(colon) = subject.find(':') else {
        return plain;
    };
    let (head, rest) = subject.split_at(colon);
    let summary = rest[1..].trim();
    if summary.is_empty() {
        return plain;
    }

    // A trailing `!` is the conventional-commit marker for a breaking change.
    let breaking = head.ends_with('!');
    let head = head.strip_suffix('!').unwrap_or(head);

    let (kind, scope) = match head.split_once('(') {
        Some((kind, scope)) => match scope.strip_suffix(')') {
            Some(scope) if !scope.is_empty() => (kind, Some(scope.to_string())),
            // `feat(unclosed: ...` is not a conventional commit.
            _ => return plain,
        },
        None => (head, None),
    };

    if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_lowercase()) {
        return plain;
    }

    let section = if breaking {
        BREAKING
    } else {
        let mut found = OTHER;
        for (heading, kinds) in SECTIONS {
            if kinds.contains(&kind) {
                found = heading;
                break;
            }
        }
        found
    };

    Commit {
        hash: hash.to_string(),
        short: short.to_string(),
        section,
        scope,
        summary: summary.to_string(),
    }
}

/// The full file, preamble through link references.
fn render(releases: &[Release], unreleased: &[Commit], kept: &BTreeMap<String, Kept>) -> String {
    let mut out = String::new();

    out.push_str("# Changelog\n\n");
    out.push_str("All notable changes to AI Video Editor, newest first.\n\n");
    out.push_str(
        "**This file is generated** from the conventional-commit subjects in the\n\
         git history by `cargo run --example changelog`, and CI checks it. The\n\
         lists under a release are rewritten on every run; a paragraph between a\n\
         release's heading and its first list is its summary, and is kept. A\n\
         wrong subject is corrected in `CHANGELOG.corrections`. Releases from\n\
         before the generator keep the section they shipped with. See\n\
         `CONTRIBUTING.md`.\n\n",
    );
    out.push_str(
        "The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),\n\
         and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).\n",
    );

    out.push('\n');
    out.push_str(UNRELEASED);
    out.push('\n');
    if unreleased.is_empty() {
        out.push_str("\nNothing yet — the latest release is the tip of `main`.\n");
    } else {
        push_sections(&mut out, unreleased);
    }

    for release in releases {
        let summary = match kept.get(&release.version) {
            Some(Kept::Verbatim { heading, body }) => {
                out.push_str(&format!("\n{heading}\n"));
                if !body.is_empty() {
                    out.push_str(&format!("\n{body}\n"));
                }
                continue;
            }
            Some(Kept::Summary(summary)) => Some(summary),
            None => None,
        };
        out.push_str(&format!("\n## [{}] - {}\n", release.version, release.date));
        if let Some(summary) = summary {
            out.push_str(&format!("\n{summary}\n"));
        }
        if release.commits.is_empty() {
            out.push_str("\nNo commits recorded against this tag.\n");
        } else {
            push_sections(&mut out, &release.commits);
        }
    }

    push_links(&mut out, releases);
    out
}

/// The `### Heading` blocks for one release, in `SECTIONS` order, skipping
/// any heading nothing landed under.
fn push_sections(out: &mut String, commits: &[Commit]) {
    let mut order: Vec<&str> = vec![BREAKING];
    order.extend(SECTIONS.iter().map(|(heading, _)| *heading));
    order.push(OTHER);

    for heading in order {
        let matching: Vec<&Commit> = commits.iter().filter(|c| c.section == heading).collect();
        if matching.is_empty() {
            continue;
        }
        out.push_str(&format!("\n### {heading}\n\n"));
        for commit in matching {
            let scope = match &commit.scope {
                Some(scope) => format!("**{scope}:** "),
                None => String::new(),
            };
            out.push_str(&format!(
                "- {scope}{} ([`{}`]({REPO}/commit/{}))\n",
                commit.summary, commit.short, commit.hash
            ));
        }
    }
}

/// Markdown link references, so each heading above is a link to the diff
/// that produced it.
fn push_links(out: &mut String, releases: &[Release]) {
    out.push('\n');
    match releases.first() {
        Some(latest) => out.push_str(&format!(
            "[Unreleased]: {REPO}/compare/{}...HEAD\n",
            latest.tag
        )),
        None => out.push_str(&format!("[Unreleased]: {REPO}/commits/HEAD\n")),
    }

    for release in releases {
        match &release.previous {
            Some(previous) => out.push_str(&format!(
                "[{}]: {REPO}/compare/{previous}...{}\n",
                release.version, release.tag
            )),
            None => out.push_str(&format!(
                "[{}]: {REPO}/releases/tag/{}\n",
                release.version, release.tag
            )),
        }
    }
}

/// The version of the newest release section in the committed file when
/// no tag of that name exists — the pre-tag state of the release flow.
/// `None` when the newest section's tag exists (or there are no release
/// sections at all). Only the newest section may be pending: an older
/// section without a tag is plain drift and fails the comparison.
fn pending_version(existing: &str, releases: &[Release]) -> Option<String> {
    for line in existing.lines() {
        let Some(rest) = line.strip_prefix("## [") else {
            continue;
        };
        let Some((version, _)) = rest.split_once(']') else {
            continue;
        };
        if version == "Unreleased" {
            continue;
        }
        return (!releases.iter().any(|r| r.version == version)).then(|| version.to_string());
    }
    None
}

/// The file with its `Unreleased` section removed and release-heading
/// dates stripped, which is what `--check` compares. The `Unreleased`
/// exemption is in the module docs; dates are stripped because the
/// pre-tag section is dated by the wall clock and the post-tag one by
/// the tag, and a release cut close to UTC midnight must not fail its
/// own check over which side of it the tag landed. The next
/// regeneration refreshes the written date to the tag's.
fn released_only(text: &str) -> String {
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.starts_with(UNRELEASED) {
            skipping = true;
            continue;
        }
        // Any other `## ` heading ends the section, as do the link
        // references that close the file.
        if skipping && (line.starts_with("## ") || line.starts_with("[Unreleased]:")) {
            skipping = false;
        }
        if !skipping {
            kept.push(undated_heading(line));
        }
    }
    kept.join("\n")
}

/// A release heading without its ` - YYYY-MM-DD` tail; every other line
/// unchanged. Entry bullets start `- `, never `## [`, so only headings
/// can match.
fn undated_heading(line: &str) -> &str {
    if line.starts_with("## [") {
        line.split(" - ").next().unwrap_or(line)
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit_of(subject: &str) -> Commit {
        parse_subject("0123456789abcdef", "0123456", subject)
    }

    fn release_of(version: &str, subjects: &[&str]) -> Release {
        Release {
            tag: format!("v{version}"),
            version: version.to_owned(),
            date: "2026-12-01".to_owned(),
            previous: None,
            commits: subjects.iter().map(|subject| commit_of(subject)).collect(),
        }
    }

    #[test]
    fn corrections_map_full_hashes_to_subjects_and_refuse_short_ones() -> Result<()> {
        let parsed = parse_corrections(
            "# a comment\n\n81404F1E2ED2EDD1087F6B209B60517C2F13F105 fix: right\n",
        )?;
        assert_eq!(
            parsed
                .get("81404f1e2ed2edd1087f6b209b60517c2f13f105")
                .map(String::as_str),
            Some("fix: right"),
            "a full hash, lower-cased, to its subject"
        );
        assert!(
            parse_corrections("81404f1 fix: short").is_err(),
            "an abbreviated hash would be ambiguous one day"
        );
        assert!(
            parse_corrections("81404f1e2ed2edd1087f6b209b60517c2f13f105").is_err(),
            "a hash without a subject corrects nothing"
        );
        Ok(())
    }

    #[test]
    fn a_pre_generator_section_is_kept_whole_and_a_summary_is_kept() {
        let file = "# Changelog\n\n## [Unreleased]\n\nNothing yet.\n\n\
                    ## [0.2.0] - 2026-12-01\n\nA summary.\n\nTwo lines.\n\n### Fixed\n\n- x\n\n\
                    ## [0.1.0] - 2026-11-25\n\nHand-written.\n\n### Added\n\n- **A**: b\n\n\
                    [Unreleased]: https://example.invalid\n";
        let kept = kept_sections(file);
        assert_eq!(
            kept.get("0.2.0"),
            Some(&Kept::Summary("A summary.\n\nTwo lines.".to_owned())),
            "the prose before the first list is the summary"
        );
        assert_eq!(
            kept.get("0.1.0"),
            Some(&Kept::Verbatim {
                heading: "## [0.1.0] - 2026-11-25".to_owned(),
                body: "Hand-written.\n\n### Added\n\n- **A**: b".to_owned(),
            }),
            "a release older than the generator keeps its heading and every line"
        );
        assert!(
            !kept.contains_key("Unreleased"),
            "Unreleased is regenerated, never kept"
        );
    }

    #[test]
    fn a_section_without_prose_keeps_no_summary() {
        let file = "## [0.3.0] - 2026-12-01\n\nNo commits recorded against this tag.\n\n\
                    ## [0.2.0] - 2026-12-01\n\n### Fixed\n\n- x\n";
        let kept = kept_sections(file);
        assert!(
            kept.is_empty(),
            "the generator's own placeholder and a bare list are not summaries: {kept:?}"
        );
    }

    #[test]
    fn rendering_keeps_what_was_kept_and_regenerates_the_rest() {
        let releases = [
            release_of("0.2.0", &["fix: a fix"]),
            release_of("0.1.0", &["feat: lost in the hand-written section"]),
        ];
        let mut kept = BTreeMap::new();
        kept.insert("0.2.0".to_owned(), Kept::Summary("Summary.".to_owned()));
        kept.insert(
            "0.1.0".to_owned(),
            Kept::Verbatim {
                heading: "## [0.1.0] - 2026-11-25".to_owned(),
                body: "As shipped.".to_owned(),
            },
        );
        let out = render(&releases, &[], &kept);
        assert!(
            out.contains("## [0.2.0] - 2026-12-01\n\nSummary.\n\n### Fixed\n\n- a fix"),
            "the summary leads the generated lists: {out}"
        );
        assert!(
            out.contains("## [0.1.0] - 2026-11-25\n\nAs shipped.\n\n[Unreleased]:"),
            "the hand-written section is copied as it is: {out}"
        );
        assert!(
            !out.contains("lost in the hand-written section"),
            "a kept section is not regenerated: {out}"
        );
        assert_eq!(
            kept_sections(&out),
            kept,
            "rendering then reading back keeps the same things"
        );
    }

    #[test]
    fn the_unreleased_section_and_release_dates_are_ignored_by_the_check() {
        let committed = "# C\n\n## [Unreleased]\n\n- new\n\n## [0.1.0] - 2026-11-25\n\nx\n";
        let generated = "# C\n\n## [Unreleased]\n\nNothing yet.\n\n## [0.1.0] - 2026-11-26\n\nx\n";
        assert_eq!(
            released_only(committed),
            released_only(generated),
            "only released sections, without their dates, are compared"
        );
    }
}

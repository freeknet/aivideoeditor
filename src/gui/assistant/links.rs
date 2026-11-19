// ============================================================================
// Module:       gui::assistant::links
// Description:  `ave://` links in the assistant's text: parsing them into
//               references, and splitting a message into prose and chips.
//
// Dependencies: crate::refs
// ============================================================================

//! Links out of the model.
//!
//! The agent cites what it talks about as markdown links with `ave://`
//! targets; the panel shows each as a chip that selects the thing and loads
//! it in the viewer:
//!
//! | Link | Reference |
//! |---|---|
//! | `ave://seg/MVI_7987/3` | segment `MVI_7987/3` |
//! | `ave://shot/reel-30/5` | slot 5 of plan `reel-30` |
//! | `ave://clip/MVI_7987@24.0-27.5` | that range of the clip |
//! | `ave://clip/MVI_7987@24.0` | that moment |
//! | `ave://bars/17-20` | bars 17 to 20 |
//! | `ave://plan/reel-30` | the plan |
//! | `ave://person/fans-02` | a performer |

use crate::refs::{Reference, Target};

/// The scheme.
const SCHEME: &str = "ave://";

/// A link target as a reference, if it is a valid `ave://` link.
pub(crate) fn parse(url: &str) -> Option<Reference> {
    let rest = url.trim().strip_prefix(SCHEME)?;
    let (kind, path) = rest.split_once('/')?;
    let path = path.trim_end_matches('/');
    let target = match kind {
        "seg" | "segment" => {
            let (clip, n) = path.rsplit_once('/')?;
            n.parse::<usize>().ok()?;
            (!clip.is_empty()).then(|| Target::Segment {
                id: path.to_owned(),
            })?
        }
        "shot" => {
            let (plan, n) = path.rsplit_once('/')?;
            let index = n.parse::<usize>().ok().filter(|i| *i >= 1)?;
            (!plan.is_empty()).then(|| Target::Shot {
                plan: plan.to_owned(),
                index,
            })?
        }
        "clip" => {
            let (clip, times) = path.split_once('@')?;
            if clip.is_empty() {
                return None;
            }
            match times.split_once('-') {
                Some((a, b)) => {
                    let (t0, t1) = (a.parse::<f32>().ok()?, b.parse::<f32>().ok()?);
                    (t1 >= t0).then(|| Target::Range {
                        clip: clip.to_owned(),
                        t0,
                        t1,
                    })?
                }
                None => Target::Frame {
                    clip: clip.to_owned(),
                    t: times.parse().ok()?,
                },
            }
        }
        "bars" | "bar" => {
            let (a, b) = path.split_once('-').unwrap_or((path, path));
            let (from, to) = (a.parse::<usize>().ok()?, b.parse::<usize>().ok()?);
            (to >= from).then_some(Target::Bars { from, to })?
        }
        "plan" if !path.is_empty() && !path.contains('/') => Target::Plan {
            name: path.to_owned(),
        },
        "person" if !path.is_empty() => Target::Person {
            name: path.to_owned(),
        },
        _ => return None,
    };
    Some(Reference::new(target))
}

/// A piece of a message: prose, or a link with its label.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Piece {
    /// Text to render as markdown.
    Text(String),
    /// A chip.
    Link {
        /// What the model wrote as the label.
        label: String,
        /// Where it points.
        target: Reference,
    },
}

/// Splits `text` at `[label](ave://…)` links and bare `ave://…` URLs.
/// Links that do not parse stay as text.
pub(crate) fn split(text: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut prose = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        let md = rest.find("](ave://").and_then(|close| {
            let open = rest[..close].rfind('[')?;
            let end = close + rest[close..].find(')')?;
            Some((open, close, end))
        });
        let bare = rest.find(SCHEME);
        match (md, bare) {
            (Some((open, close, end)), Some(b)) if open <= b => {
                let label = &rest[open + 1..close];
                let url = &rest[close + 2..end];
                prose.push_str(&rest[..open]);
                match parse(url) {
                    Some(target) => {
                        flush(&mut out, &mut prose);
                        out.push(Piece::Link {
                            label: label.to_owned(),
                            target,
                        });
                    }
                    None => prose.push_str(label),
                }
                rest = &rest[end + 1..];
            }
            (_, Some(b)) => {
                let len = rest[b..]
                    .find(|c: char| c.is_whitespace() || matches!(c, ')' | ',' | ';'))
                    .unwrap_or(rest.len() - b);
                let mut url = &rest[b..b + len];
                // A full stop ends a sentence, not a link.
                url = url.trim_end_matches('.');
                prose.push_str(&rest[..b]);
                match parse(url) {
                    Some(target) => {
                        flush(&mut out, &mut prose);
                        let label = url.trim_start_matches(SCHEME).to_owned();
                        out.push(Piece::Link { label, target });
                    }
                    None => prose.push_str(url),
                }
                rest = &rest[b + url.len()..];
            }
            (_, None) => {
                prose.push_str(rest);
                rest = "";
            }
        }
    }
    flush(&mut out, &mut prose);
    out
}

/// Moves pending prose into `out`.
fn flush(out: &mut Vec<Piece>, prose: &mut String) {
    if !prose.is_empty() {
        out.push(Piece::Text(std::mem::take(prose)));
    }
}

#[cfg(test)]
mod tests {
    use super::{Piece, parse, split};
    use crate::refs::Target;

    #[test]
    fn every_link_kind_parses() -> anyhow::Result<()> {
        let t = |u: &str| parse(u).map(|r| r.target);
        assert_eq!(
            t("ave://seg/MVI_7987/3"),
            Some(Target::Segment {
                id: "MVI_7987/3".into()
            }),
            "segment"
        );
        assert_eq!(
            t("ave://shot/reel-30/5"),
            Some(Target::Shot {
                plan: "reel-30".into(),
                index: 5
            }),
            "shot"
        );
        assert_eq!(
            t("ave://clip/MVI_7987@24.0-27.5"),
            Some(Target::Range {
                clip: "MVI_7987".into(),
                t0: 24.0,
                t1: 27.5
            }),
            "range"
        );
        assert_eq!(
            t("ave://clip/MVI_7987@24.5"),
            Some(Target::Frame {
                clip: "MVI_7987".into(),
                t: 24.5
            }),
            "frame"
        );
        assert_eq!(
            t("ave://bars/17-20"),
            Some(Target::Bars { from: 17, to: 20 }),
            "bars"
        );
        assert_eq!(
            t("ave://bars/9"),
            Some(Target::Bars { from: 9, to: 9 }),
            "one bar"
        );
        assert_eq!(
            t("ave://plan/reel-15"),
            Some(Target::Plan {
                name: "reel-15".into()
            }),
            "plan"
        );
        for bad in [
            "ave://shot/reel-30/0",
            "ave://shot/reel-30",
            "ave://bars/20-17",
            "ave://clip/MVI@3-1",
            "ave://seg/MVI_7987/x",
            "https://example.com",
            "ave://nope/1",
        ] {
            assert!(parse(bad).is_none(), "{bad} is refused");
        }
        Ok(())
    }

    #[test]
    fn messages_split_into_prose_and_chips() -> anyhow::Result<()> {
        let pieces = split(
            "Put [shot 5](ave://shot/reel-30/5) on the drop; see ave://seg/MVI_7987/3. \
             [broken](ave://shot/x/0) stays text.",
        );
        assert_eq!(pieces.len(), 5, "{pieces:?}");
        assert_eq!(pieces[0], Piece::Text("Put ".into()), "lead");
        assert!(
            matches!(&pieces[1], Piece::Link { label, .. } if label == "shot 5"),
            "markdown link: {:?}",
            pieces[1]
        );
        assert!(
            matches!(&pieces[3], Piece::Link { label, .. } if label == "seg/MVI_7987/3"),
            "bare link, full stop left out: {:?}",
            pieces[3]
        );
        assert!(
            matches!(&pieces[4], Piece::Text(t) if t.starts_with(". broken stays")),
            "unparseable link keeps its label: {:?}",
            pieces[4]
        );
        assert_eq!(split("no links"), [Piece::Text("no links".into())], "plain");
        Ok(())
    }
}

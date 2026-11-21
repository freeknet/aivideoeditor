// ============================================================================
// Module:       gui::assistant::complete
// Description:  The composer's `@` autocomplete: finding the reference token
//               under the cursor, ranking candidates (shots, people, bars,
//               plans, segments by id or caption words) and inserting one.
//
// Dependencies: crate::refs
// ============================================================================

//! Reference autocomplete.
//!
//! Typing `@` (optionally after an intent prefix `+ ! > ~ *`) opens a list
//! of things to point at; picking one replaces the partial token with the
//! full text form (`>@shot:reel-30#5 `). Tokens typed in full are the same
//! as chips: [`merge`] combines both into the message's references.

use crate::refs::{Modifier, Reference};

/// The `@` token being typed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Active {
    /// First char of the token (including its prefix), char index.
    pub(crate) start: usize,
    /// One past its last char.
    pub(crate) end: usize,
    /// The intent prefix typed before `@`.
    pub(crate) modifier: Modifier,
    /// What follows `@`, lower-cased.
    pub(crate) query: String,
}

/// The `@` token around char position `cursor`, if any.
pub(crate) fn active(text: &str, cursor: usize) -> Option<Active> {
    let chars: Vec<char> = text.chars().collect();
    let cursor = cursor.min(chars.len());
    let mut start = cursor;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = cursor;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    let word: String = chars[start..end].iter().collect();
    let (modifier, rest) = match word.chars().next() {
        Some(c) if c != '@' => {
            let m = Modifier::ALL
                .into_iter()
                .find(|m| m.prefix().starts_with(c) && !m.prefix().is_empty())?;
            (m, &word[c.len_utf8()..])
        }
        Some(_) => (Modifier::About, word.as_str()),
        None => return None,
    };
    let query = rest.strip_prefix('@')?;
    Some(Active {
        start,
        end,
        modifier,
        query: query.to_lowercase(),
    })
}

/// Something that can be referenced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// The token without its prefix, e.g. `@shot:reel-30#5`.
    pub(crate) token: String,
    /// A short label.
    pub(crate) label: String,
    /// One line of detail.
    pub(crate) detail: String,
}

/// Everything the composer can offer.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sources {
    /// Shots of the current plan: plan, slot, description.
    pub(crate) shots: Vec<(String, usize, String)>,
    /// Performers.
    pub(crate) people: Vec<String>,
    /// Bar ranges worth naming: from, to, label.
    pub(crate) bars: Vec<(usize, usize, String)>,
    /// Plans.
    pub(crate) plans: Vec<String>,
    /// Segments: id and caption.
    pub(crate) segments: Vec<(String, String)>,
}

/// Candidates for `query`, best first, at most `limit`.
pub(crate) fn candidates(query: &str, sources: &Sources, limit: usize) -> Vec<Candidate> {
    let q = query.to_lowercase();
    // `shot:5`, `seg:MVI`, `person:x`: the kind narrows, the rest filters.
    let (kind, needle) = match q.split_once(':') {
        Some((k, v)) => (Some(k.to_owned()), v.to_owned()),
        None => (None, q.clone()),
    };
    let wants = |k: &str| {
        kind.as_deref()
            .is_none_or(|w| k.starts_with(w) || w.starts_with(k))
    };
    let hit = |hay: &str| needle.is_empty() || hay.to_lowercase().contains(&needle);
    let mut out = Vec::new();
    if wants("shot") {
        for (plan, slot, desc) in &sources.shots {
            let label = format!("shot {slot}");
            if hit(&format!("{label} #{slot} {desc}")) || needle == slot.to_string() {
                out.push(Candidate {
                    token: format!("@shot:{plan}#{slot}"),
                    label,
                    detail: desc.clone(),
                });
            }
        }
        // An exact slot number goes first.
        out.sort_by_key(|c: &Candidate| c.label != format!("shot {needle}"));
    }
    if wants("person") {
        for name in &sources.people {
            if hit(name) {
                out.push(Candidate {
                    token: format!("@person:{name}"),
                    label: name.clone(),
                    detail: "performer".to_owned(),
                });
            }
        }
    }
    if wants("bars") {
        for (from, to, label) in &sources.bars {
            if hit(&format!("bars {from}-{to} {label}")) {
                out.push(Candidate {
                    token: format!("@bars:{from}-{to}"),
                    label: format!("bars {from}–{to}"),
                    detail: label.clone(),
                });
            }
        }
    }
    if wants("plan") {
        for name in &sources.plans {
            if hit(name) {
                out.push(Candidate {
                    token: format!("@plan:{name}"),
                    label: name.clone(),
                    detail: "plan".to_owned(),
                });
            }
        }
    }
    if wants("seg") && (!needle.is_empty() || kind.is_some()) {
        let words: Vec<&str> = needle.split(['_', ' ']).filter(|w| !w.is_empty()).collect();
        for (id, caption) in &sources.segments {
            let hay = format!("{id} {caption}").to_lowercase();
            if hay.contains(&needle) || (!words.is_empty() && words.iter().all(|w| hay.contains(w)))
            {
                out.push(Candidate {
                    token: format!("@seg:{id}"),
                    label: id.clone(),
                    detail: caption.chars().take(80).collect(),
                });
            }
            if out.len() >= limit * 3 {
                break;
            }
        }
    }
    out.truncate(limit);
    out
}

/// `text` with chars `start..end` replaced by `token` and a space; returns
/// the new text and the cursor after the space.
pub(crate) fn insert(text: &str, start: usize, end: usize, token: &str) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let (start, end) = (start.min(chars.len()), end.min(chars.len()));
    let mut out: String = chars[..start].iter().collect();
    out.push_str(token);
    out.push(' ');
    let cursor = out.chars().count();
    let tail: String = chars[end..].iter().collect();
    out.push_str(tail.trim_start());
    (out, cursor)
}

/// Where a reference in the composer comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// A staged chip (index into the staged list).
    Chip(usize),
    /// A token typed in the text.
    Typed,
}

/// The message's references: staged chips, then typed tokens not already
/// staged (by target, so `+@shot:x#1` typed over a chip changes nothing).
pub(crate) fn merge(chips: &[Reference], text: &str) -> Vec<(Reference, Origin)> {
    let mut out: Vec<(Reference, Origin)> = chips
        .iter()
        .enumerate()
        .map(|(k, r)| (r.clone(), Origin::Chip(k)))
        .collect();
    for r in Reference::scan(text) {
        if !out.iter().any(|(o, _)| o.target == r.target) {
            out.push((r, Origin::Typed));
        }
    }
    out
}

/// Byte ranges of the reference tokens in `text`, in order: what the
/// composer and the sent bubble paint in the key colour (Selects shows
/// mentions as coloured text, not chips).
pub(crate) fn mention_spans(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut spans = Vec::new();
    let mut start = None;
    for (at, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        match (c.is_whitespace(), start) {
            (true, Some(from)) => {
                if text
                    .get(from..at)
                    .is_some_and(|word| Reference::parse(word).is_some())
                {
                    spans.push(from..at);
                }
                start = None;
            }
            (false, None) => start = Some(at),
            (true, None) | (false, Some(_)) => {}
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::{Origin, Sources};
    use crate::refs::{Modifier, Reference, Target};

    fn sources() -> Sources {
        Sources {
            shots: vec![
                ("reel-30".into(), 1, "MVI_7925 silks".into()),
                ("reel-30".into(), 5, "MVI_7987 fire fans".into()),
            ],
            people: vec!["white-mask".into(), "fans-02".into()],
            bars: vec![(17, 20, "drop".into())],
            plans: vec!["reel-30".into()],
            segments: vec![
                ("MVI_7925/3".into(), "silks in pink light".into()),
                ("MVI_7987/2".into(), "fire fans spinning close".into()),
            ],
        }
    }

    #[test]
    fn the_token_under_the_cursor() -> anyhow::Result<()> {
        let a = super::active("swap >@sho", 10);
        assert_eq!(
            a.as_ref().map(|a| (a.start, a.modifier, a.query.as_str())),
            Some((5, Modifier::Replace, "sho")),
            "replace token: {a:?}"
        );
        assert!(super::active("plain words", 5).is_none(), "no token");
        assert_eq!(
            super::active("@", 1).map(|a| a.query),
            Some(String::new()),
            "a bare @ opens the list"
        );
        Ok(())
    }

    #[test]
    fn candidates_by_kind_and_words() -> anyhow::Result<()> {
        let s = sources();
        let all = super::candidates("", &s, 20);
        assert!(
            all.iter().any(|c| c.token == "@shot:reel-30#5")
                && !all.iter().any(|c| c.token.starts_with("@seg")),
            "empty query lists shots etc., not every segment: {all:?}"
        );
        let fire = super::candidates("fire", &s, 20);
        assert!(
            fire.iter().any(|c| c.token == "@shot:reel-30#5")
                && fire.iter().any(|c| c.token == "@seg:MVI_7987/2"),
            "caption words: {fire:?}"
        );
        let seg = super::candidates("seg:7925", &s, 20);
        assert_eq!(seg.len(), 1, "kind narrows: {seg:?}");
        let five = super::candidates("5", &s, 20);
        assert_eq!(
            five.first().map(|c| c.label.as_str()),
            Some("shot 5"),
            "slot number"
        );
        Ok(())
    }

    #[test]
    fn inserting_and_merging_tokens() -> anyhow::Result<()> {
        let (text, cursor) = super::insert("swap >@sho please", 5, 10, ">@shot:reel-30#5");
        assert_eq!(text, "swap >@shot:reel-30#5 please", "replaced");
        assert_eq!(cursor, 22, "cursor after the space");

        let chip = Reference::new(Target::Shot {
            plan: "reel-30".into(),
            index: 5,
        });
        let merged = super::merge(
            std::slice::from_ref(&chip),
            "keep +@shot:reel-30#5 avoid !@person:white-mask",
        );
        assert_eq!(merged.len(), 2, "duplicate target folded: {merged:?}");
        assert_eq!(merged[0].1, Origin::Chip(0), "chip first");
        assert_eq!(merged[1].1, Origin::Typed, "then typed");
        Ok(())
    }

    #[test]
    fn mention_spans_cover_whole_tokens_only() -> anyhow::Result<()> {
        let text = "swap >@shot:reel-30#5 for\n@person:white-mask, not @nothing";
        let spans = super::mention_spans(text);
        let words: Vec<&str> = spans.iter().filter_map(|r| text.get(r.clone())).collect();
        assert_eq!(
            words,
            [">@shot:reel-30#5", "@person:white-mask,"]
                .iter()
                .filter(|w| Reference::parse(w).is_some())
                .copied()
                .collect::<Vec<_>>(),
            "every parsable token, prefix included, and nothing else"
        );
        assert!(
            words.first() == Some(&">@shot:reel-30#5"),
            "the shot token with its intent prefix: {words:?}"
        );
        assert!(
            super::mention_spans("").is_empty() && super::mention_spans("   ").is_empty(),
            "no text, no spans"
        );
        Ok(())
    }
}

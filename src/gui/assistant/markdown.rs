// ============================================================================
// Module:       gui::assistant::markdown
// Description:  The little markdown the assistant writes (bold, inline code,
//               bullets, headings) parsed into styled spans for the panel.
//
// Dependencies: none
// ============================================================================

//! Markdown-ish text.
//!
//! The model is told to write plain sentences with **bold** and bullets; this
//! turns that into lines of [`Span`]s the panel lays out. Anything it does
//! not understand stays literal text, so nothing is ever lost.

/// What kind of line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Ordinary text.
    Para,
    /// A bullet, with its nesting depth.
    Bullet(usize),
    /// A heading.
    Heading,
}

/// A run of text with one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    /// The text.
    pub(crate) text: String,
    /// Bold.
    pub(crate) bold: bool,
    /// Inline code.
    pub(crate) code: bool,
}

/// One line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Line {
    /// Its kind.
    pub(crate) kind: Kind,
    /// Its spans.
    pub(crate) spans: Vec<Span>,
}

/// Parses `text` into lines.
pub(crate) fn parse(text: &str) -> Vec<Line> {
    text.lines()
        .map(|raw| {
            let indent = raw.len() - raw.trim_start().len();
            let t = raw.trim_start();
            let (kind, body) = if let Some(b) = t
                .strip_prefix("- ")
                .or_else(|| t.strip_prefix("* "))
                .or_else(|| t.strip_prefix("• "))
            {
                (Kind::Bullet(indent / 2), b)
            } else if t.starts_with('#') {
                (Kind::Heading, t.trim_start_matches('#').trim_start())
            } else {
                (Kind::Para, t)
            };
            Line {
                kind,
                spans: spans(body),
            }
        })
        .collect()
}

/// Splits one line into styled spans.
fn spans(text: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut bold = false;
    let mut code = false;
    let mut cur = String::new();
    let mut rest = text;
    let flush = |out: &mut Vec<Span>, cur: &mut String, bold: bool, code: bool| {
        if !cur.is_empty() {
            out.push(Span {
                text: std::mem::take(cur),
                bold,
                code,
            });
        }
    };
    while let Some(c) = rest.chars().next() {
        if !code && rest.starts_with("**") && (bold || rest[2..].contains("**")) {
            flush(&mut out, &mut cur, bold, code);
            bold = !bold;
            rest = &rest[2..];
        } else if c == '`' && (code || rest[1..].contains('`')) {
            flush(&mut out, &mut cur, bold, code);
            code = !code;
            rest = &rest[1..];
        } else {
            cur.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    flush(&mut out, &mut cur, bold, code);
    out
}

#[cfg(test)]
mod tests {
    use super::{Kind, Span};

    fn s(text: &str, bold: bool, code: bool) -> Span {
        Span {
            text: text.into(),
            bold,
            code,
        }
    }

    #[test]
    fn bold_code_bullets_and_headings() -> anyhow::Result<()> {
        let lines = super::parse("Swap **shot 5** for `MVI_7925/3`.\n- one\n  - two\n## Head");
        assert_eq!(lines.len(), 4, "four lines");
        assert_eq!(
            lines[0].spans,
            vec![
                s("Swap ", false, false),
                s("shot 5", true, false),
                s(" for ", false, false),
                s("MVI_7925/3", false, true),
                s(".", false, false)
            ],
            "styled spans"
        );
        assert_eq!(lines[1].kind, Kind::Bullet(0), "bullet");
        assert_eq!(lines[2].kind, Kind::Bullet(1), "nested bullet");
        assert_eq!(lines[3].kind, Kind::Heading, "heading");
        let lone = super::parse("2 ** 3 and a ` tick");
        assert_eq!(
            lone[0].spans,
            vec![s("2 ** 3 and a ` tick", false, false)],
            "unpaired markers stay literal"
        );
        Ok(())
    }
}

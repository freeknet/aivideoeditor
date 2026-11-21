// ============================================================================
// Module:       gui::assistant::model
// Description:  The conversation as data: messages made of blocks (text,
//               thinking, tool calls and results), and the lines of the
//               per-plan chat log that persist it.
//
// Dependencies: serde, serde_json; super::ops
// ============================================================================

//! Conversation data.
//!
//! A [`Msg`] is one turn of the API conversation, kept losslessly: thinking
//! blocks carry their signature and anything the provider returned that we
//! do not model is kept as [`Block::Opaque`] and replayed unchanged, so the
//! history can be resent append-only (which keeps the prompt cache warm and
//! the model's earlier reasoning valid).
//!
//! A chat log (`<work>/chat/chats/<id>.jsonl`) is a list of [`LogLine`]s: the
//! messages, the proposals the model made, what became of each, and the
//! timing of every turn. Reading it back rebuilds the panel exactly.

use serde_json::Value;

use super::ops::EditOp;
use crate::plan::Plan;

/// Who said it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Role {
    /// The person (and the tool results sent on their behalf).
    User,
    /// The model.
    Assistant,
}

/// One piece of a message.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Block {
    /// Prose.
    Text {
        /// The text.
        text: String,
    },
    /// The night's state sent with a user message (plan, check, references).
    /// Sent as text, hidden in the panel.
    Context {
        /// The snapshot.
        text: String,
    },
    /// The model's reasoning (a summary). An empty signature means it came
    /// from a provider that cannot take it back, so it is never resent.
    Thinking {
        /// The summary.
        thinking: String,
        /// The provider's signature, replayed unchanged.
        #[serde(default)]
        signature: String,
    },
    /// A tool call.
    ToolUse {
        /// Call id.
        id: String,
        /// Tool name.
        name: String,
        /// Arguments.
        input: Value,
    },
    /// A tool's answer.
    ToolResult {
        /// The call it answers.
        tool_use_id: String,
        /// What the tool returned.
        content: String,
        /// Whether it failed.
        #[serde(default)]
        is_error: bool,
    },
    /// A block we do not model (redacted thinking, fallback markers),
    /// replayed exactly as received.
    Opaque {
        /// The block as the provider sent it.
        raw: Value,
    },
}

/// One message.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Msg {
    /// Who.
    pub(crate) role: Role,
    /// What.
    pub(crate) blocks: Vec<Block>,
    /// When, Unix seconds.
    #[serde(default)]
    pub(crate) ts: u64,
}

impl Msg {
    /// A message stamped now.
    pub(crate) fn new(role: Role, blocks: Vec<Block>) -> Self {
        Self {
            role,
            blocks,
            ts: now(),
        }
    }

    /// The text the person typed (or the model wrote), without context.
    pub(crate) fn text(&self) -> String {
        self.blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } => Some(text.as_str()),
                Block::Context { .. }
                | Block::Thinking { .. }
                | Block::ToolUse { .. }
                | Block::ToolResult { .. }
                | Block::Opaque { .. } => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The tool calls in this message.
    pub(crate) fn tool_uses(&self) -> impl Iterator<Item = (&str, &str, &Value)> {
        self.blocks.iter().filter_map(|b| match b {
            Block::ToolUse { id, name, input } => Some((id.as_str(), name.as_str(), input)),
            Block::Text { .. }
            | Block::Context { .. }
            | Block::Thinking { .. }
            | Block::ToolResult { .. }
            | Block::Opaque { .. } => None,
        })
    }
}

/// What became of a proposal.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum Status {
    /// Waiting for the person.
    Pending,
    /// Saved as plan version `version`; `ops` are the ones applied.
    Applied {
        /// The version written.
        version: usize,
        /// Indices into the proposal's ops.
        ops: Vec<usize>,
        /// When.
        ts: u64,
    },
    /// Thrown away.
    Discarded,
    /// Undone: the plan before it was restored as `version`.
    Reverted {
        /// The version written by the revert.
        version: usize,
        /// When.
        ts: u64,
    },
}

/// A proposed edit, as the model made it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Proposal {
    /// The tool call it came from.
    pub(crate) id: String,
    /// The plan it edits.
    pub(crate) plan: String,
    /// One line saying what and why.
    pub(crate) summary: String,
    /// The edit.
    pub(crate) ops: Vec<EditOp>,
    /// The plan the ops apply to, as it was when proposed.
    pub(crate) base: Plan,
    /// The plan file's version when proposed (`crate::mcp::plan_version`);
    /// Apply refuses if the file moved since. Empty for old proposals.
    #[serde(default)]
    pub(crate) base_version: String,
    /// When.
    pub(crate) ts: u64,
}

/// One line of the chat log.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum LogLine {
    /// A conversation message.
    Msg {
        /// The message.
        msg: Msg,
    },
    /// A proposal (its card sits under the message that made it).
    Proposal {
        /// The proposal.
        proposal: Box<Proposal>,
    },
    /// A proposal changed state.
    Status {
        /// Proposal id.
        id: String,
        /// New state.
        status: Status,
    },
    /// A turn finished.
    Turn {
        /// Wall time, seconds.
        secs: f32,
        /// Model that answered.
        model: String,
        /// Input tokens, all steps.
        #[serde(default)]
        input_tokens: u64,
        /// Output tokens, all steps.
        #[serde(default)]
        output_tokens: u64,
        /// Input tokens read from the prompt cache.
        #[serde(default)]
        cached_tokens: u64,
        /// What the turn cost, when the runtime reports it (Claude Code).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
        /// The last request's prompt, cache included: how full the context
        /// was at the end of the turn (0: not reported).
        #[serde(default, skip_serializing_if = "is_zero")]
        context_tokens: u64,
        /// The model's context window, when the runtime said (0: not said).
        #[serde(default, skip_serializing_if = "is_zero")]
        context_window: u64,
    },
    /// An image a tool returned (a `look` contact sheet), saved beside the
    /// log and shown under the call.
    Image {
        /// The call it came from.
        tool_use_id: String,
        /// The file, relative to the chat folder.
        file: String,
    },
    /// Something went wrong (shown, not sent).
    Notice {
        /// What.
        text: String,
        /// Whether it is an error rather than a note.
        #[serde(default)]
        error: bool,
    },
}

/// For serde: leave a zero count out.
fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Seconds since the Unix epoch.
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `YYYY-MM-DD HH:MM` (UTC) for Unix seconds.
pub(crate) fn stamp(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// The messages of a log, in order.
pub(crate) fn messages(log: &[LogLine]) -> Vec<Msg> {
    log.iter()
        .filter_map(|l| match l {
            LogLine::Msg { msg } => Some(msg.clone()),
            LogLine::Proposal { .. }
            | LogLine::Status { .. }
            | LogLine::Turn { .. }
            | LogLine::Notice { .. }
            | LogLine::Image { .. } => None,
        })
        .collect()
}

/// The latest status of proposal `id` in a log.
pub(crate) fn status_of(log: &[LogLine], id: &str) -> Status {
    log.iter()
        .rev()
        .find_map(|l| match l {
            LogLine::Status { id: i, status } if i == id => Some(status.clone()),
            LogLine::Msg { .. }
            | LogLine::Proposal { .. }
            | LogLine::Status { .. }
            | LogLine::Turn { .. }
            | LogLine::Notice { .. }
            | LogLine::Image { .. } => None,
        })
        .unwrap_or(Status::Pending)
}

/// Tool calls in the last assistant message that no later message answers,
/// as `(id, name, input)`.
pub(crate) fn unanswered(msgs: &[Msg]) -> Vec<(String, String, Value)> {
    let Some(pos) = msgs.iter().rposition(|m| m.role == Role::Assistant) else {
        return Vec::new();
    };
    let answered: Vec<&str> = msgs[pos + 1..]
        .iter()
        .flat_map(|m| m.blocks.iter())
        .filter_map(|b| match b {
            Block::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            Block::Text { .. }
            | Block::Context { .. }
            | Block::Thinking { .. }
            | Block::ToolUse { .. }
            | Block::Opaque { .. } => None,
        })
        .collect();
    msgs[pos]
        .tool_uses()
        .filter(|(id, _, _)| !answered.contains(id))
        .map(|(id, name, input)| (id.to_owned(), name.to_owned(), input.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Block, LogLine, Msg, Role, Status};

    #[test]
    fn stamps_are_civil_dates() -> anyhow::Result<()> {
        assert_eq!(super::stamp(0), "1970-01-01 00:00", "epoch");
        assert_eq!(
            super::stamp(1_795_000_000),
            "2026-11-18 11:06",
            "a known date"
        );
        Ok(())
    }

    #[test]
    fn log_lines_round_trip_as_json() -> anyhow::Result<()> {
        let line = LogLine::Msg {
            msg: Msg::new(
                Role::Assistant,
                vec![
                    Block::Thinking {
                        thinking: "hm".into(),
                        signature: "sig".into(),
                    },
                    Block::ToolUse {
                        id: "t1".into(),
                        name: "get_plan".into(),
                        input: serde_json::json!({}),
                    },
                    Block::Opaque {
                        raw: serde_json::json!({"type": "redacted_thinking", "data": "x"}),
                    },
                ],
            ),
        };
        let text = serde_json::to_string(&line)?;
        let back: LogLine = serde_json::from_str(&text)?;
        assert_eq!(back, line, "round trip: {text}");
        Ok(())
    }

    #[test]
    fn unanswered_calls_and_latest_status() -> anyhow::Result<()> {
        let call = |id: &str| Block::ToolUse {
            id: id.into(),
            name: "search_shots".into(),
            input: serde_json::json!({"query": "fire"}),
        };
        let msgs = vec![
            Msg::new(Role::User, vec![Block::Text { text: "hi".into() }]),
            Msg::new(Role::Assistant, vec![call("a"), call("b")]),
            Msg::new(
                Role::User,
                vec![Block::ToolResult {
                    tool_use_id: "a".into(),
                    content: "ok".into(),
                    is_error: false,
                }],
            ),
        ];
        // The last assistant message is the one with calls; only b is open.
        let open = super::unanswered(&msgs);
        assert_eq!(open.len(), 1, "one open call: {open:?}");
        assert_eq!(open[0].0, "b", "b is open");

        let log = vec![
            LogLine::Status {
                id: "p".into(),
                status: Status::Discarded,
            },
            LogLine::Status {
                id: "p".into(),
                status: Status::Applied {
                    version: 2,
                    ops: vec![0],
                    ts: 0,
                },
            },
        ];
        assert!(
            matches!(
                super::status_of(&log, "p"),
                Status::Applied { version: 2, .. }
            ),
            "latest wins"
        );
        assert_eq!(super::status_of(&log, "q"), Status::Pending, "unknown");
        Ok(())
    }
}

// ============================================================================
// Module:       gui::assistant::tools
// Description:  The tools the in-process loop offers: the `ave mcp` tool set
//               (run in-process through `crate::mcp`) plus `ask_user`, with
//               staged plan edits turned into proposal cards.
//
// Dependencies: serde_json; super::{ops, model, provider}; crate::{mcp, plan}
// ============================================================================

//! Tools.
//!
//! The Claude API and local providers see exactly the tools Claude Code sees
//! over MCP ([`crate::mcp::api_tools`]), run by the same code
//! ([`crate::mcp::call`]), plus `ask_user`, which only an in-process loop can
//! serve. A `plan_edit` without `commit` comes back as a proposal card; one
//! with `commit` is refused unless the person allowed the assistant to apply
//! edits, and then shows up as an applied, revertible card.
//!
//! Calls run on the worker thread against an [`Env`] taken when the message
//! was sent; the open plan's scope is enforced here before the edit reaches
//! the engine.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};

use super::model::{Proposal, now};
use super::ops::{self, EditOp, Night, Scope};
use crate::index::Index;
use crate::mcp::{self, Caller};
use crate::people::People;
use crate::plan::Plan;

/// The night as the tools see it.
#[derive(Clone, Debug)]
pub(crate) struct Env {
    /// The night's work folder.
    pub(crate) work: PathBuf,
    /// The plan the conversation is about, as sent.
    pub(crate) plan: Plan,
    /// The full index, for checking edits against the scope.
    pub(crate) index: Option<Arc<Index>>,
    /// The people map.
    pub(crate) people: People,
    /// What the request may touch in that plan.
    pub(crate) scope: Scope,
}

/// What a tool call produced.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    /// A result to send back, with any images it carried (MIME type and
    /// base64 data) for the panel.
    Answer {
        /// The text.
        content: String,
        /// Whether the call failed.
        is_error: bool,
        /// Images for the panel.
        images: Vec<(String, String)>,
    },
    /// A staged or committed edit for the panel, and the result to send back.
    Proposal {
        /// The text.
        content: String,
        /// The proposal.
        proposal: Box<Proposal>,
        /// The version saved, when the agent committed it.
        committed: Option<usize>,
    },
    /// A question for the person; the turn ends until they answer.
    Ask {
        /// The question.
        question: String,
        /// Two or three answers to offer as buttons.
        options: Vec<String>,
    },
}

/// The name of the clarifying-question tool.
pub(crate) const ASK: &str = "ask_user";

/// The name of the plan-editing tool.
pub(crate) const EDIT: &str = "plan_edit";

/// The prefix Claude Code puts before our MCP tools' names.
pub(crate) const MCP_PREFIX: &str = "mcp__ave__";

/// The tool definitions, in the Messages API's shape.
pub(crate) fn definitions() -> Value {
    let mut tools = mcp::api_tools();
    if let Some(a) = tools.as_array_mut() {
        a.push(json!({
            "name": ASK,
            "description": "Ask the person one clarifying question when the request is \
                ambiguous, with two or three short, concrete options shown as buttons. Ends \
                your turn; their answer comes back as this tool's result.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "question": {"type": "string"},
                    "options": {"type": "array", "items": {"type": "string"}, "description": "Two or three answers."}
                },
                "required": ["question", "options"],
                "additionalProperties": false
            }
        }));
    }
    tools
}

/// A short description of a call for the panel ("Searching shots: fire").
/// `live` is the present tense, for a call still running.
pub(crate) fn describe(name: &str, input: &Value, live: bool) -> String {
    let name = name.strip_prefix(MCP_PREFIX).unwrap_or(name);
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or_default();
    let (now, past, what) = match name {
        "night_overview" => ("Reading the night", "Read the night", String::new()),
        "search_shots" => ("Searching shots", "Searched shots", s("query").to_owned()),
        "segment_details" => (
            "Reading segments",
            "Read segments",
            input["ids"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default(),
        ),
        "look" => (
            "Making a contact sheet",
            "Made a contact sheet",
            String::new(),
        ),
        "music_map" => ("Reading the music", "Read the music", String::new()),
        "plan_get" => ("Reading the plan", "Read the plan", s("name").to_owned()),
        EDIT if input["commit"].as_bool().unwrap_or(false) => {
            ("Saving an edit", "Saved an edit", s("reason").to_owned())
        }
        EDIT => ("Staging an edit", "Staged an edit", s("reason").to_owned()),
        "plan_draft" => ("Drafting a plan", "Drafted a plan", s("name").to_owned()),
        "alternatives" => (
            "Ranking alternatives",
            "Ranked alternatives",
            format!("{} #{}", s("plan"), input["index"]),
        ),
        "plan_versions" => ("Listing versions", "Listed versions", s("name").to_owned()),
        "plan_revert" => (
            "Reverting",
            "Reverted",
            format!("{} to v{}", s("name"), input["version"]),
        ),
        "people_map" => (
            "Reading the people map",
            "Read the people map",
            String::new(),
        ),
        "Read" => ("Reading", "Read", file_name(s("file_path"))),
        "Glob" | "Grep" => (
            "Looking up files",
            "Looked up files",
            s("pattern").to_owned(),
        ),
        ASK => ("Asking", "Asked", s("question").to_owned()),
        mcp::RUN_STEP => (
            "Asking to start a step",
            "Asked to start a step",
            [s("step"), s("plan")]
                .into_iter()
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
        ),
        other => {
            return format!("{} {other}", if live { "Calling" } else { "Called" });
        }
    };
    let verb = if live { now } else { past };
    if what.is_empty() {
        verb.to_owned()
    } else {
        format!("{verb}: {what}")
    }
}

/// The last two parts of a path (`fix-check/SKILL.md`).
fn file_name(path: &str) -> String {
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|p| !p.is_empty()).collect();
    parts[parts.len().saturating_sub(2)..].join("/")
}

/// Runs one call.
pub(crate) fn run(env: &Env, id: &str, name: &str, input: &Value) -> Outcome {
    if let Some(raw) = input.get(super::provider::INVALID_JSON) {
        return error(format!(
            "INVALID_JSON: the arguments were not valid JSON ({}); send them again",
            raw.as_str()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>()
        ));
    }
    match name {
        ASK => ask(input),
        EDIT => edit(env, id, input),
        other => match mcp::call(&env.work, other, input, Caller::Agent) {
            Ok(content) => {
                let (text, images) = split_content(&content);
                Outcome::Answer {
                    content: text,
                    is_error: false,
                    images,
                }
            }
            Err(e) => error(format!("{e:#}")),
        },
    }
}

/// Text and images of an MCP result's content blocks.
pub(crate) fn split_content(content: &[Value]) -> (String, Vec<(String, String)>) {
    let mut text = Vec::new();
    let mut images = Vec::new();
    for c in content {
        match c["type"].as_str() {
            Some("text") => text.push(c["text"].as_str().unwrap_or_default().to_owned()),
            Some("image") => {
                let mime = c["mimeType"]
                    .as_str()
                    .or_else(|| c["source"]["media_type"].as_str())
                    .unwrap_or("image/jpeg");
                let data = c["data"]
                    .as_str()
                    .or_else(|| c["source"]["data"].as_str())
                    .unwrap_or_default();
                if !data.is_empty() {
                    images.push((mime.to_owned(), data.to_owned()));
                    text.push("(a contact sheet, shown to the person)".to_owned());
                }
            }
            Some(_) | None => {}
        }
    }
    (text.join("\n"), images)
}

/// `ask_user`.
fn ask(input: &Value) -> Outcome {
    let question = input
        .get("question")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let options: Vec<String> = input
        .get("options")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|o| !o.is_empty())
                .take(3)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    if question.is_empty() {
        return error("ask_user needs a question".to_owned());
    }
    Outcome::Ask { question, options }
}

/// `plan_edit`: scope-checked, then run by the engine; a staged edit becomes
/// a card, a committed one an applied card.
fn edit(env: &Env, id: &str, input: &Value) -> Outcome {
    let plan_name = input
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let ops: Vec<EditOp> = match input
        .get("ops")
        .cloned()
        .map(serde_json::from_value::<Vec<EditOp>>)
    {
        Some(Ok(ops)) if !ops.is_empty() => ops,
        Some(Ok(_)) => return error("an edit needs at least one op".to_owned()),
        Some(Err(e)) => return error(format!("the ops do not match the schema: {e}")),
        None => return error("plan_edit needs ops".to_owned()),
    };
    let path = env.work.join("plans").join(format!("{plan_name}.toml"));
    let (base, base_version) = match (Plan::load(&path), mcp::plan_version(&path)) {
        (Ok(p), Ok(v)) => (p, v),
        (Err(e), _) | (_, Err(e)) => return error(format!("cannot read plan {plan_name}: {e:#}")),
    };
    if plan_name == env.plan.name {
        let night = Night {
            index: env.index.as_deref(),
            people: &env.people,
        };
        if let Err(e) = ops::apply(&base, &ops, &env.scope, night) {
            return error(format!("refused, nothing was staged: {e}"));
        }
    }
    let content = match mcp::call(&env.work, EDIT, input, Caller::Agent) {
        Ok(c) => split_content(&c).0,
        Err(e) => return error(format!("{e:#}")),
    };
    let committed = content
        .split("committed as v")
        .nth(1)
        .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|n| n.parse().ok());
    Outcome::Proposal {
        content,
        committed,
        proposal: Box::new(Proposal {
            id: id.to_owned(),
            plan: plan_name,
            summary: input
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned(),
            ops,
            base,
            base_version,
            ts: now(),
        }),
    }
}

/// A failed call.
fn error(content: String) -> Outcome {
    Outcome::Answer {
        content,
        is_error: true,
        images: Vec::new(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::{Env, Outcome};
    use crate::gui::assistant::ops::Scope;

    /// A night on disk with plan `t` (3 shots) and the env for it; keep the
    /// folder alive as long as the env.
    pub(crate) fn env() -> anyhow::Result<(tempfile::TempDir, Env)> {
        let dir = crate::mcp::tests::night(3)?;
        let env = Env {
            work: dir.path().to_path_buf(),
            plan: crate::gui::assistant::ops::tests::plan(3),
            index: None,
            people: crate::people::People::default(),
            scope: Scope::Whole,
        };
        Ok((dir, env))
    }

    #[test]
    fn the_tool_set_is_the_mcp_set_plus_ask() -> anyhow::Result<()> {
        let defs = super::definitions();
        let names: Vec<&str> = defs
            .as_array()
            .map(|a| a.iter().filter_map(|d| d["name"].as_str()).collect())
            .unwrap_or_default();
        let mcp = crate::mcp::tools();
        assert_eq!(
            names.len(),
            mcp.as_array().map_or(0, Vec::len) + 1,
            "every MCP tool plus ask_user"
        );
        assert!(
            names.contains(&"plan_edit") && names.contains(&"ask_user"),
            "{names:?}"
        );
        assert!(
            defs.as_array()
                .is_some_and(|a| a.iter().all(|d| d["input_schema"]["type"] == "object")),
            "object schemas"
        );
        Ok(())
    }

    #[test]
    fn staged_edits_become_cards_and_scope_is_enforced() -> anyhow::Result<()> {
        let (_dir, mut env) = env()?;
        let good = json!({"name": "t", "reason": "note 2", "ops": [
            {"op": "set_note", "index": 2, "note": "LED hoop q 0.95"}
        ]});
        let out = super::run(&env, "p1", "plan_edit", &good);
        let Outcome::Proposal {
            proposal,
            content,
            committed,
        } = out
        else {
            anyhow::bail!("expected a proposal: {out:?}");
        };
        assert_eq!(proposal.id, "p1", "id is the call id");
        assert_eq!(
            proposal.base_version.len(),
            16,
            "the base version rides along"
        );
        assert_eq!(committed, None, "staged, not saved");
        assert!(content.contains("staged"), "says so: {content}");

        env.scope = Scope::Shots(std::iter::once(1).collect());
        let out = super::run(&env, "p2", "plan_edit", &good);
        assert!(
            matches!(&out, Outcome::Answer { is_error: true, content, .. } if content.contains("refused")),
            "out of scope: {out:?}"
        );

        let commit = json!({"name": "t", "reason": "x", "commit": true, "base_version": "0",
            "ops": [{"op": "set_note", "index": 1, "note": "n"}]});
        env.scope = Scope::Whole;
        let out = super::run(&env, "p3", "plan_edit", &commit);
        assert!(
            matches!(&out, Outcome::Answer { is_error: true, content, .. } if content.contains("COMMIT_REFUSED")),
            "commits refused by default: {out:?}"
        );
        Ok(())
    }

    #[test]
    fn other_tools_run_in_the_engine_and_questions_end_the_turn() -> anyhow::Result<()> {
        let (_dir, env) = env()?;
        let out = super::run(&env, "1", "plan_get", &json!({"name": "t"}));
        assert!(
            matches!(&out, Outcome::Answer { content, is_error: false, .. } if content.contains("version:")),
            "plan_get: {out:?}"
        );
        let out = super::run(&env, "2", "search_shots", &json!({"query": "fire"}));
        assert!(
            matches!(out, Outcome::Answer { is_error: true, .. }),
            "unindexed night is an error"
        );
        let ask =
            json!({"question": "Punchier how?", "options": ["shorter shots", "more fire", ""]});
        let out = super::run(&env, "q", "ask_user", &ask);
        assert!(
            matches!(&out, Outcome::Ask { options, .. } if options.len() == 2),
            "empty options dropped: {out:?}"
        );
        assert_eq!(
            super::describe("mcp__ave__search_shots", &json!({"query": "fire"}), true),
            "Searching shots: fire",
            "described live"
        );
        assert_eq!(
            super::describe(
                "Read",
                &json!({"file_path": "C:\\x\\skills\\fix-check\\SKILL.md"}),
                false
            ),
            "Read: fix-check/SKILL.md",
            "skill reads"
        );
        Ok(())
    }

    #[test]
    fn images_split_from_text() -> anyhow::Result<()> {
        let (text, images) = super::split_content(&[
            json!({"type": "image", "mimeType": "image/jpeg", "data": "AAAA"}),
            json!({"type": "text", "text": "1 A @1.00"}),
        ]);
        assert_eq!(images.len(), 1, "one image");
        assert!(text.contains("1 A @1.00"), "text kept: {text}");
        Ok(())
    }
}

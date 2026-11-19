// ============================================================================
// Module:       gui::assistant::tools
// Description:  The tools the model may call (search the shot log, read a
//               segment or the plan, propose an edit, ask the person), their
//               JSON schemas, and running them against a snapshot of the night.
//
// Dependencies: serde_json; super::{context, ops, model}; crate::{plan, index}
// ============================================================================

//! Tools.
//!
//! The tools run on the worker thread against an [`Env`]: a snapshot of the
//! night taken when the message was sent, so the person can keep working
//! while the model thinks. None of them changes anything on disk:
//! `propose_edit` validates the operations on a copy of the plan, predicts
//! the rule check and hands the proposal to the panel as a diff card.

use std::fmt::Write as _;
use std::sync::Arc;

use serde_json::{Value, json};

use super::context::{self, ShotLog};
use super::model::{Proposal, now};
use super::ops::{self, EditOp, Night, Scope};
use crate::beats::MusicMap;
use crate::index::Index;
use crate::people::People;
use crate::plan::Plan;

/// The night as the tools see it.
#[derive(Clone, Debug)]
pub(crate) struct Env {
    /// The plan being edited.
    pub(crate) plan: Plan,
    /// The shot log.
    pub(crate) log: ShotLog,
    /// The full index, for the rule check.
    pub(crate) index: Option<Arc<Index>>,
    /// The music map.
    pub(crate) music: Option<MusicMap>,
    /// The people map.
    pub(crate) people: People,
    /// What the request may touch.
    pub(crate) scope: Scope,
}

impl Env {
    /// The rule check of `plan` in this night.
    fn check(&self, plan: &Plan) -> Option<Vec<crate::plan::Problem>> {
        ops::check(
            plan,
            self.index.as_deref(),
            self.music.as_ref(),
            &self.people,
        )
    }
}

/// What a tool call produced.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    /// A result to send back.
    Answer {
        /// The text.
        content: String,
        /// Whether the call failed.
        is_error: bool,
    },
    /// A proposal for the panel, and the result to send back.
    Proposal {
        /// The text.
        content: String,
        /// The proposal.
        proposal: Box<Proposal>,
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

/// The name of the proposal tool.
pub(crate) const PROPOSE: &str = "propose_edit";

/// The tool definitions, in the Messages API's shape.
pub(crate) fn definitions() -> Value {
    json!([
        {
            "name": "search_shots",
            "description": "Search the night's shot log. Returns matching segments, best first, \
                one line each with measured numbers, flags, caption, who, and whether the plan \
                already uses it. Words match captions, subjects, people and ids. Facets: \
                fire>0.05, led>0.02, q>0.7, len>2 (seconds), subject:fire, moment:performing, \
                who:<name>, clip:<prefix>, -<flag> to exclude a flag, clean to exclude every \
                bad flag, transition/idle moments, blocked views and blur/focus issues.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Words and facets, e.g. 'fire fans close clean q>0.7'."},
                    "limit": {"type": "integer", "description": "Most results to return (default 12, max 40)."}
                },
                "required": ["query"],
                "additionalProperties": false
            },
            "eager_input_streaming": true
        },
        {
            "name": "get_segment",
            "description": "One shot-log segment by id (e.g. MVI_7925/3) with its neighbours in \
                the same clip.",
            "input_schema": {
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"],
                "additionalProperties": false
            },
            "eager_input_streaming": true
        },
        {
            "name": "get_plan",
            "description": "The current plan (one line per shot with its note), the music \
                window and the rule check.",
            "input_schema": {"type": "object", "properties": {}, "additionalProperties": false},
            "eager_input_streaming": true
        },
        {
            "name": PROPOSE,
            "description": "Propose an edit to the plan. Nothing is applied: the person sees a \
                diff card with your reasons and the predicted check, and accepts or discards \
                it. Ops run in order; each index is a 1-based shot number in the plan as the \
                earlier ops left it, so put remove_shot and move_shot last. Ops: replace_shot \
                {index, clip, in, cx?, reason} puts other footage in a slot of the same \
                length (reason becomes the note; cx defaults to the segment's action centre); \
                set_in {index, in}; set_beats {index, beats} (balance with another set_beats: \
                the beat total is fixed); move_shot {from, to} (slot lengths stay); \
                remove_shot {index} (its beats go to the shot before); set_cx {index, cx}; \
                set_note {index, note}. Returns the predicted check or why the ops were refused.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "summary": {"type": "string", "description": "One sentence: what changes and why."},
                    "ops": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "op": {"type": "string", "enum": ["replace_shot", "set_in", "set_beats", "move_shot", "remove_shot", "set_cx", "set_note"]},
                                "index": {"type": "integer"},
                                "clip": {"type": "string"},
                                "in": {"type": "number"},
                                "cx": {"type": "number"},
                                "reason": {"type": "string"},
                                "beats": {"type": "integer"},
                                "from": {"type": "integer"},
                                "to": {"type": "integer"},
                                "note": {"type": "string"}
                            },
                            "required": ["op"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["summary", "ops"],
                "additionalProperties": false
            },
            "eager_input_streaming": true
        },
        {
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
            },
            "eager_input_streaming": true
        }
    ])
}

/// A short description of a call for the panel: `search_shots fire close`.
pub(crate) fn describe(name: &str, input: &Value) -> String {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or_default();
    match name {
        "search_shots" => format!("Searched the shot log: {}", s("query")),
        "get_segment" => format!("Read segment {}", s("id")),
        "get_plan" => "Read the plan".to_owned(),
        PROPOSE => format!("Proposed: {}", s("summary")),
        ASK => format!("Asked: {}", s("question")),
        other => format!("Called {other}"),
    }
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
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or_default();
    match name {
        "search_shots" => {
            let limit = input
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(12, |n| n.clamp(1, 40) as usize);
            let hits = env.log.search(s("query"), limit, Some(&env.plan));
            if hits.is_empty() {
                return answer(format!(
                    "no segments match `{}` (of {}); try fewer words or looser facets",
                    s("query"),
                    env.log.rows.len()
                ));
            }
            let mut out = String::new();
            for r in hits {
                let used = context::used_in(&env.plan, r)
                    .map(|k| format!(" · IN PLAN as shot {k}"))
                    .unwrap_or_default();
                let _ = writeln!(out, "{}{used}", r.line());
            }
            answer(out)
        }
        "get_segment" => {
            let id = s("id");
            let Some(row) = env.log.get(id) else {
                return error(format!("no segment {id}; ids look like MVI_7925/3"));
            };
            let mut out = format!("{}\nsame clip:\n", row.line());
            for r in env
                .log
                .rows
                .iter()
                .filter(|r| r.clip == row.clip && r.id != row.id)
            {
                let _ = writeln!(out, "- {}", r.line());
            }
            answer(out)
        }
        "get_plan" => {
            let mut out = context::plan_text(&env.plan, env.music.as_ref());
            if let Some(m) = &env.music {
                let _ = writeln!(out, "{}", context::music_text(&env.plan, m));
            }
            let problems = env.check(&env.plan);
            let _ = write!(out, "{}", context::problems_text(problems.as_deref()));
            answer(out)
        }
        PROPOSE => propose(env, id, input),
        ASK => {
            let question = s("question").trim().to_owned();
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
        other => error(format!("there is no tool called {other}")),
    }
}

/// `propose_edit`.
fn propose(env: &Env, id: &str, input: &Value) -> Outcome {
    let summary = input
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let ops: Vec<EditOp> = match input
        .get("ops")
        .cloned()
        .map(serde_json::from_value::<Vec<EditOp>>)
    {
        Some(Ok(ops)) => ops,
        Some(Err(e)) => return error(format!("the ops do not match the schema: {e}")),
        None => return error("propose_edit needs ops".to_owned()),
    };
    if ops.is_empty() {
        return error("a proposal needs at least one op".to_owned());
    }
    let night = Night {
        index: env.index.as_deref(),
        people: &env.people,
    };
    let after = match ops::apply(&env.plan, &ops, &env.scope, night) {
        Ok((after, _)) => after,
        Err(e) => return error(format!("refused, nothing was shown to the person: {e}")),
    };
    let verdict = match (env.check(&env.plan), env.check(&after)) {
        (Some(before), Some(now)) => {
            let (new, fixed) = ops::compare(&before, &now);
            let mut v = if now.is_empty() {
                "predicted check: passes".to_owned()
            } else {
                format!(
                    "predicted check: {} problems ({} new, {fixed} fixed)",
                    now.len(),
                    new.len()
                )
            };
            for p in &new {
                let _ = write!(v, "\n- NEW shot {}: {}", p.shot, p.message);
            }
            v
        }
        (None, _) | (_, None) => "predicted check: unavailable".to_owned(),
    };
    Outcome::Proposal {
        content: format!(
            "Shown to the person as a diff card; not applied until they accept.\n{verdict}"
        ),
        proposal: Box::new(Proposal {
            id: id.to_owned(),
            plan: env.plan.name.clone(),
            summary,
            ops,
            base: env.plan.clone(),
            ts: now(),
        }),
    }
}

/// A successful answer.
fn answer(content: String) -> Outcome {
    Outcome::Answer {
        content,
        is_error: false,
    }
}

/// A failed call.
fn error(content: String) -> Outcome {
    Outcome::Answer {
        content,
        is_error: true,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::json;

    use super::{Env, Outcome};
    use crate::gui::assistant::ops::Scope;

    pub(crate) fn env() -> Env {
        Env {
            plan: crate::gui::assistant::ops::tests::plan(3),
            log: crate::gui::assistant::context::tests::log(),
            index: None,
            music: None,
            people: crate::people::People::default(),
            scope: Scope::Whole,
        }
    }

    #[test]
    fn every_definition_has_a_schema() -> anyhow::Result<()> {
        let defs = super::definitions();
        let names: Vec<&str> = defs
            .as_array()
            .map(|a| a.iter().filter_map(|d| d["name"].as_str()).collect())
            .unwrap_or_default();
        assert_eq!(
            names,
            [
                "search_shots",
                "get_segment",
                "get_plan",
                "propose_edit",
                "ask_user"
            ],
            "the tool set"
        );
        assert!(
            defs.as_array()
                .is_some_and(|a| a.iter().all(|d| d["input_schema"]["type"] == "object")),
            "object schemas"
        );
        Ok(())
    }

    #[test]
    fn search_and_segment_answer_with_shot_log_lines() -> anyhow::Result<()> {
        let env = env();
        let out = super::run(&env, "1", "search_shots", &json!({"query": "fire -dark"}));
        assert!(
            matches!(&out, Outcome::Answer { content, is_error: false } if content.contains("A/1") && !content.contains("B/1")),
            "search: {out:?}"
        );
        let out = super::run(&env, "2", "get_segment", &json!({"id": "nope"}));
        assert!(
            matches!(out, Outcome::Answer { is_error: true, .. }),
            "unknown segment is an error"
        );
        Ok(())
    }

    #[test]
    fn proposals_validate_and_questions_end_the_turn() -> anyhow::Result<()> {
        let env = env();
        let good = json!({"summary": "swap 2", "ops": [
            {"op": "replace_shot", "index": 2, "clip": "C", "in": 1.0, "reason": "LED hoop q 0.95"}
        ]});
        let out = super::run(&env, "p1", "propose_edit", &good);
        let Outcome::Proposal { proposal, content } = out else {
            anyhow::bail!("expected a proposal: {out:?}");
        };
        assert_eq!(proposal.id, "p1", "id is the call id");
        assert_eq!(proposal.ops.len(), 1, "one op");
        assert!(content.contains("not applied"), "says so: {content}");

        let bad = json!({"summary": "x", "ops": [{"op": "set_note", "index": 7, "note": "n"}]});
        let out = super::run(&env, "p2", "propose_edit", &bad);
        assert!(
            matches!(&out, Outcome::Answer { is_error: true, content } if content.contains("no shot 7")),
            "refused: {out:?}"
        );

        let ask =
            json!({"question": "Punchier how?", "options": ["shorter shots", "more fire", ""]});
        let out = super::run(&env, "q", "ask_user", &ask);
        assert!(
            matches!(&out, Outcome::Ask { options, .. } if options.len() == 2),
            "empty options dropped: {out:?}"
        );
        assert_eq!(
            super::describe("search_shots", &json!({"query": "fire"})),
            "Searched the shot log: fire",
            "described"
        );
        Ok(())
    }
}

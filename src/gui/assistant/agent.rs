// ============================================================================
// Module:       gui::assistant::agent
// Description:  The assistant's turn: call the model, run the tools it asks
//               for, feed the results back, until it answers, proposes and
//               finishes, asks the person something, or fails.
//
// Dependencies: serde_json; super::{provider, tools, model}
// ============================================================================

//! The tool loop.
//!
//! [`run`] is one turn, run on a worker thread. Everything it produces goes
//! out as [`Event`]s, in order: streamed text, every message to append to
//! the conversation, proposals for the panel, a question, notices, and a
//! final [`Event::Done`]. The panel appends and persists what arrives, so a
//! turn interrupted half way leaves a conversation that can still be resent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use serde_json::Value;

use super::model::{Block, Msg, Proposal, Role};
use super::provider::{Delta, Provider, Request, Stop, Usage};
use super::tools::{self, Env, Outcome};

/// Most model calls in one turn.
const MAX_STEPS: usize = 12;

/// Something the turn produced.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Event {
    /// Streamed text or thinking.
    Delta(Delta),
    /// A message to append to the conversation.
    Msg(Msg),
    /// A proposal to show.
    Proposal(Box<Proposal>),
    /// A question for the person; `held` are the other results of the same
    /// step, to send with the answer.
    Ask {
        /// The `ask_user` call id.
        id: String,
        /// The question.
        question: String,
        /// Answers to offer.
        options: Vec<String>,
        /// Results of the step's other calls.
        held: Vec<Block>,
    },
    /// A note or an error for the panel.
    Notice {
        /// What.
        text: String,
        /// Whether it is an error.
        error: bool,
    },
    /// The turn ended.
    Done {
        /// Wall time, seconds.
        secs: f32,
        /// The model that answered.
        model: String,
        /// Tokens, all steps.
        usage: Usage,
    },
}

/// Everything a turn needs.
pub(crate) struct Job {
    /// The model.
    pub(crate) provider: Box<dyn Provider>,
    /// Standing instructions.
    pub(crate) system: String,
    /// Tool definitions.
    pub(crate) tools: Value,
    /// The conversation, ending with the person's new message.
    pub(crate) msgs: Vec<Msg>,
    /// The night, for the tools.
    pub(crate) env: Env,
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("provider", &self.provider.label())
            .field("msgs", &self.msgs.len())
            .finish_non_exhaustive()
    }
}

/// Runs one turn.
pub(crate) fn run(mut job: Job, emit: &mut dyn FnMut(Event), cancel: &AtomicBool) {
    let started = Instant::now();
    let mut usage = Usage::default();
    let mut model = job.provider.label();
    for step in 0..MAX_STEPS {
        let request = Request {
            system: &job.system,
            msgs: &job.msgs,
            tools: &job.tools,
        };
        let reply = match job
            .provider
            .send(&request, &mut |d| emit(Event::Delta(d)), cancel)
        {
            Ok(r) => r,
            Err(e) => {
                emit(Event::Notice {
                    text: e,
                    error: true,
                });
                break;
            }
        };
        usage.input += reply.usage.input;
        usage.cached += reply.usage.cached;
        usage.output += reply.usage.output;
        if !reply.model.is_empty() {
            model.clone_from(&reply.model);
        }
        let msg = Msg::new(Role::Assistant, reply.blocks);
        job.msgs.push(msg.clone());
        emit(Event::Msg(msg.clone()));

        let calls: Vec<(String, String, Value)> = msg
            .tool_uses()
            .map(|(id, name, input)| (id.to_owned(), name.to_owned(), input.clone()))
            .collect();
        if calls.is_empty() {
            match reply.stop {
                Stop::EndTurn | Stop::ToolUse => {}
                Stop::MaxTokens => emit(Event::Notice {
                    text: "The reply hit the output limit and was cut short.".to_owned(),
                    error: true,
                }),
                Stop::Refusal(why) => emit(Event::Notice {
                    text: format!("The model declined: {why}"),
                    error: true,
                }),
                Stop::Other(why) => emit(Event::Notice {
                    text: format!("The model stopped ({why})."),
                    error: false,
                }),
            }
            break;
        }
        if matches!(reply.stop, Stop::MaxTokens) {
            // A call cut off mid-arguments must not run; answer every call so
            // the conversation stays valid, and end the turn.
            let results = calls
                .iter()
                .map(|(id, _, _)| Block::ToolResult {
                    tool_use_id: id.clone(),
                    content: "not run: the reply was cut off by the output limit".to_owned(),
                    is_error: true,
                })
                .collect();
            let m = Msg::new(Role::User, results);
            job.msgs.push(m.clone());
            emit(Event::Msg(m));
            emit(Event::Notice {
                text: "The reply hit the output limit mid tool call; ask again, more narrowly."
                    .to_owned(),
                error: true,
            });
            break;
        }

        let mut results = Vec::new();
        let mut ask: Option<(String, String, Vec<String>)> = None;
        for (id, name, input) in &calls {
            let outcome = if ask.is_some() && name == tools::ASK {
                Outcome::Answer {
                    content: "only one question per turn".to_owned(),
                    is_error: true,
                }
            } else {
                tools::run(&job.env, id, name, input)
            };
            match outcome {
                Outcome::Answer { content, is_error } => results.push(Block::ToolResult {
                    tool_use_id: id.clone(),
                    content,
                    is_error,
                }),
                Outcome::Proposal { content, proposal } => {
                    emit(Event::Proposal(proposal));
                    results.push(Block::ToolResult {
                        tool_use_id: id.clone(),
                        content,
                        is_error: false,
                    });
                }
                Outcome::Ask { question, options } => ask = Some((id.clone(), question, options)),
            }
        }
        if let Some((id, question, options)) = ask {
            emit(Event::Ask {
                id,
                question,
                options,
                held: results,
            });
            break;
        }
        let m = Msg::new(Role::User, results);
        job.msgs.push(m.clone());
        emit(Event::Msg(m));
        if cancel.load(Ordering::Relaxed) {
            emit(Event::Notice {
                text: "Stopped.".to_owned(),
                error: false,
            });
            break;
        }
        if step + 1 == MAX_STEPS {
            emit(Event::Notice {
                text: format!("Stopped after {MAX_STEPS} steps; say \"go on\" to continue."),
                error: false,
            });
        }
    }
    emit(Event::Done {
        secs: started.elapsed().as_secs_f32(),
        model,
        usage,
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    use serde_json::json;

    use super::{Event, Job};
    use crate::gui::assistant::model::{Block, Msg, Role};
    use crate::gui::assistant::provider::{Delta, Provider, Reply, Request, Stop, Usage};

    /// Plays back scripted replies and records how long each request was.
    struct Mock {
        replies: Mutex<Vec<Reply>>,
        seen: std::sync::Arc<Mutex<Vec<usize>>>,
    }

    impl Provider for Mock {
        fn label(&self) -> String {
            "mock".into()
        }

        fn send(
            &self,
            request: &Request<'_>,
            on: &mut dyn FnMut(Delta),
            _cancel: &AtomicBool,
        ) -> Result<Reply, String> {
            if let Ok(mut s) = self.seen.lock() {
                s.push(request.msgs.len());
            }
            let mut replies = self.replies.lock().map_err(|e| e.to_string())?;
            if replies.is_empty() {
                return Err("script ran out".into());
            }
            let r = replies.remove(0);
            on(Delta::Text("…".into()));
            Ok(r)
        }
    }

    fn reply(blocks: Vec<Block>, stop: Stop) -> Reply {
        Reply {
            blocks,
            stop,
            usage: Usage {
                input: 10,
                cached: 5,
                output: 3,
            },
            model: "mock-1".into(),
        }
    }

    fn call(id: &str, name: &str, input: serde_json::Value) -> Block {
        Block::ToolUse {
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    fn make_job(replies: Vec<Reply>) -> (Job, std::sync::Arc<Mutex<Vec<usize>>>) {
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let job = Job {
            provider: Box::new(Mock {
                replies: Mutex::new(replies),
                seen: seen.clone(),
            }),
            system: "sys".into(),
            tools: crate::gui::assistant::tools::definitions(),
            msgs: vec![Msg::new(
                Role::User,
                vec![Block::Text {
                    text: "swap shot 2 for something with more fire".into(),
                }],
            )],
            env: crate::gui::assistant::tools::tests::env(),
        };
        (job, seen)
    }

    #[test]
    fn a_turn_searches_proposes_and_finishes() -> anyhow::Result<()> {
        let (job, seen) = make_job(vec![
            reply(
                vec![call("s", "search_shots", json!({"query": "fire"}))],
                Stop::ToolUse,
            ),
            reply(
                vec![call(
                    "p",
                    "propose_edit",
                    json!({"summary": "fire in 2", "ops": [
                        {"op": "replace_shot", "index": 2, "clip": "A", "in": 1.0, "reason": "fire 8%"}
                    ]}),
                )],
                Stop::ToolUse,
            ),
            reply(
                vec![Block::Text {
                    text: "Proposed **shot 2** → A.".into(),
                }],
                Stop::EndTurn,
            ),
        ]);
        let mut events = Vec::new();
        super::run(job, &mut |e| events.push(e), &AtomicBool::new(false));
        let msgs = events.iter().filter(|e| matches!(e, Event::Msg(_))).count();
        assert_eq!(msgs, 5, "3 replies + 2 result messages: {events:?}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Proposal(p) if p.id == "p")),
            "the proposal surfaced"
        );
        assert!(
            matches!(events.last(), Some(Event::Done { usage, model, .. }) if usage.input == 30 && model == "mock-1"),
            "done with summed usage: {:?}",
            events.last()
        );
        let lens = seen.lock().map(|s| s.clone()).unwrap_or_default();
        assert_eq!(lens, [1, 3, 5], "the history grows append-only");
        Ok(())
    }

    #[test]
    fn a_question_ends_the_turn_and_holds_other_results() -> anyhow::Result<()> {
        let (job, _) = make_job(vec![reply(
            vec![
                call("g", "get_plan", json!({})),
                call(
                    "q",
                    "ask_user",
                    json!({"question": "Which part?", "options": ["the build", "the drop"]}),
                ),
            ],
            Stop::ToolUse,
        )]);
        let mut events = Vec::new();
        super::run(job, &mut |e| events.push(e), &AtomicBool::new(false));
        let ask = events.iter().find_map(|e| match e {
            Event::Ask {
                id, options, held, ..
            } => Some((id.clone(), options.len(), held.len())),
            Event::Delta(_)
            | Event::Msg(_)
            | Event::Proposal(_)
            | Event::Notice { .. }
            | Event::Done { .. } => None,
        });
        assert_eq!(ask, Some(("q".into(), 2, 1)), "asked, plan result held");
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, Event::Msg(m) if m.role == Role::User)),
            "no results sent before the answer"
        );
        Ok(())
    }

    #[test]
    fn failures_and_cut_off_calls_end_cleanly() -> anyhow::Result<()> {
        let (job, _) = make_job(Vec::new());
        let mut events = Vec::new();
        super::run(job, &mut |e| events.push(e), &AtomicBool::new(false));
        assert!(
            matches!(&events[0], Event::Notice { error: true, text } if text.contains("ran out")),
            "provider error shown: {events:?}"
        );
        let (job, _) = make_job(vec![reply(
            vec![call("s", "search_shots", json!({"query": "fi"}))],
            Stop::MaxTokens,
        )]);
        let mut events = Vec::new();
        super::run(job, &mut |e| events.push(e), &AtomicBool::new(false));
        assert!(
            events.iter().any(|e| matches!(e, Event::Msg(m)
                if m.blocks.iter().any(|b| matches!(b, Block::ToolResult { is_error: true, .. })))),
            "the cut-off call is answered with an error: {events:?}"
        );
        Ok(())
    }
}

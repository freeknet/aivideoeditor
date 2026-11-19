// ============================================================================
// Module:       gui::assistant::provider
// Description:  The model behind the panel: Claude over the Messages API
//               (streamed) or a local llama.cpp OpenAI-compatible server, both
//               behind one trait so the loop and its tests do not care which.
//
// Dependencies: ureq, serde_json; super::model
// ============================================================================

//! Providers.
//!
//! [`Provider::send`] takes the system prompt, the conversation and the tool
//! definitions and returns the model's next message, calling `on` with text
//! and thinking as they stream so the panel can show them live.
//!
//! - [`Claude`]: `POST /v1/messages` with server-sent events, adaptive
//!   thinking with summaries, the system prompt and the conversation cached,
//!   and the server-side refusal fallback on the models that have it.
//! - [`Local`]: llama.cpp's `/v1/chat/completions`, not streamed. Small local
//!   models plan poorly; it is the offline fallback, not the default.
//!
//! The request and response shapes are pure functions ([`claude_body`],
//! [`Sse`], [`local_body`], [`local_reply`]) so they are tested without a
//! network.

use std::io::BufRead as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use super::model::{Block, Msg, Role};

/// The Messages API version header.
const API_VERSION: &str = "2026-10-21";

/// The server-side refusal fallback beta.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// Key under which a tool call's unparseable arguments are kept.
pub(crate) const INVALID_JSON: &str = "__invalid_json";

/// Something that streamed in.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Delta {
    /// Answer text.
    Text(String),
    /// Thinking summary text.
    Thinking(String),
    /// A tool call started.
    Tool(String),
}

/// Why the model stopped.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Stop {
    /// It finished.
    EndTurn,
    /// It wants tool results.
    ToolUse,
    /// It ran out of output tokens.
    MaxTokens,
    /// It declined.
    Refusal(String),
    /// Anything else.
    Other(String),
}

/// Token counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    /// Input tokens, including cached ones.
    pub(crate) input: u64,
    /// Of which read from the cache.
    pub(crate) cached: u64,
    /// Output tokens.
    pub(crate) output: u64,
}

/// The model's next message.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reply {
    /// Its blocks.
    pub(crate) blocks: Vec<Block>,
    /// Why it stopped.
    pub(crate) stop: Stop,
    /// Token counts.
    pub(crate) usage: Usage,
    /// The model that answered.
    pub(crate) model: String,
}

/// One request.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Request<'a> {
    /// The standing instructions.
    pub(crate) system: &'a str,
    /// The conversation so far.
    pub(crate) msgs: &'a [Msg],
    /// Tool definitions (Messages API shape).
    pub(crate) tools: &'a Value,
}

/// A model.
pub(crate) trait Provider: Send {
    /// A label for the panel.
    fn label(&self) -> String;

    /// The next message, streaming deltas to `on`; stops early when
    /// `cancel` is set.
    fn send(
        &self,
        request: &Request<'_>,
        on: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<Reply, String>;
}

/// Claude over the Messages API.
#[derive(Clone, Debug)]
pub(crate) struct Claude {
    /// API key.
    pub(crate) key: String,
    /// Model id.
    pub(crate) model: String,
    /// API base URL.
    pub(crate) base: String,
}

/// Whether `model` offers the server-side refusal fallback.
fn has_fallback(model: &str) -> bool {
    !model.contains("haiku")
}

/// The Messages API request body.
pub(crate) fn claude_body(model: &str, request: &Request<'_>) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": 64000,
        "stream": true,
        "system": [{"type": "text", "text": request.system, "cache_control": {"type": "ephemeral"}}],
        "tools": request.tools,
        "thinking": {"type": "adaptive", "display": "summarized"},
        "output_config": {"effort": "medium"},
        "cache_control": {"type": "ephemeral"},
        "messages": claude_messages(request.msgs),
    });
    if has_fallback(model) {
        body["fallbacks"] = json!("default");
    }
    body
}

/// The conversation in the Messages API's shape: consecutive messages of
/// one role merged, blocks that cannot be resent dropped.
fn claude_messages(msgs: &[Msg]) -> Vec<Value> {
    let mut out: Vec<(Role, Vec<Value>)> = Vec::new();
    for m in msgs {
        let blocks: Vec<Value> = m
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text { text } | Block::Context { text } => {
                    (!text.trim().is_empty()).then(|| json!({"type": "text", "text": text}))
                }
                Block::Thinking {
                    thinking,
                    signature,
                } => (!signature.is_empty()).then(
                    || json!({"type": "thinking", "thinking": thinking, "signature": signature}),
                ),
                Block::ToolUse { id, name, input } => {
                    Some(json!({"type": "tool_use", "id": id, "name": name, "input": input}))
                }
                Block::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } => Some(json!({"type": "tool_result", "tool_use_id": tool_use_id,
                    "content": content, "is_error": is_error})),
                Block::Opaque { raw } => Some(raw.clone()),
            })
            .collect();
        if blocks.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some((role, existing)) if *role == m.role => existing.extend(blocks),
            Some(_) | None => out.push((m.role, blocks)),
        }
    }
    out.into_iter()
        .map(|(role, content)| {
            json!({"role": match role { Role::User => "user", Role::Assistant => "assistant" },
                "content": content})
        })
        .collect()
}

/// A block being streamed.
#[derive(Clone, Debug)]
enum Partial {
    /// Text.
    Text(String),
    /// Thinking and its signature.
    Thinking(String, String),
    /// A tool call: id, name, argument JSON so far.
    Tool(String, String, String),
    /// Anything else, kept whole.
    Opaque(Value),
}

/// Assembles a streamed Messages API response from its server-sent events.
#[derive(Clone, Debug, Default)]
pub(crate) struct Sse {
    /// Blocks by index.
    blocks: Vec<Partial>,
    /// The stop reason.
    stop: Option<Stop>,
    /// Token counts.
    usage: Usage,
    /// The model.
    model: String,
    /// An error event's message.
    error: Option<String>,
}

impl Sse {
    /// Feeds one line of the event stream.
    pub(crate) fn line(&mut self, line: &str, on: &mut dyn FnMut(Delta)) {
        let Some(data) = line.strip_prefix("data:") else {
            return;
        };
        let Ok(event) = serde_json::from_str::<Value>(data.trim()) else {
            return;
        };
        let s = |v: &Value, k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        match event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "message_start" => {
                let m = &event["message"];
                self.model = s(m, "model");
                let u = &m["usage"];
                self.usage.cached = n(u, "cache_read_input_tokens");
                self.usage.input =
                    n(u, "input_tokens") + self.usage.cached + n(u, "cache_creation_input_tokens");
                self.usage.output = n(u, "output_tokens");
            }
            "content_block_start" => {
                let b = &event["content_block"];
                let partial = match b.get("type").and_then(Value::as_str).unwrap_or_default() {
                    "text" => Partial::Text(s(b, "text")),
                    "thinking" => Partial::Thinking(s(b, "thinking"), s(b, "signature")),
                    "tool_use" => {
                        on(Delta::Tool(s(b, "name")));
                        Partial::Tool(s(b, "id"), s(b, "name"), String::new())
                    }
                    _ => Partial::Opaque(b.clone()),
                };
                let at = usize::try_from(n(&event, "index")).unwrap_or(self.blocks.len());
                if at >= self.blocks.len() {
                    self.blocks.resize(at + 1, Partial::Text(String::new()));
                }
                self.blocks[at] = partial;
            }
            "content_block_delta" => {
                let at = usize::try_from(n(&event, "index")).unwrap_or(usize::MAX);
                let d = &event["delta"];
                let Some(block) = self.blocks.get_mut(at) else {
                    return;
                };
                match (
                    d.get("type").and_then(Value::as_str).unwrap_or_default(),
                    block,
                ) {
                    ("text_delta", Partial::Text(t)) => {
                        let piece = s(d, "text");
                        t.push_str(&piece);
                        on(Delta::Text(piece));
                    }
                    ("thinking_delta", Partial::Thinking(t, _)) => {
                        let piece = s(d, "thinking");
                        t.push_str(&piece);
                        on(Delta::Thinking(piece));
                    }
                    ("signature_delta", Partial::Thinking(_, sig)) => {
                        sig.push_str(&s(d, "signature"))
                    }
                    ("input_json_delta", Partial::Tool(_, _, json)) => {
                        json.push_str(&s(d, "partial_json"));
                    }
                    (
                        _,
                        Partial::Text(_)
                        | Partial::Thinking(..)
                        | Partial::Tool(..)
                        | Partial::Opaque(_),
                    ) => {}
                }
            }
            "message_delta" => {
                let reason = s(&event["delta"], "stop_reason");
                if !reason.is_empty() {
                    self.stop = Some(match reason.as_str() {
                        "end_turn" | "stop_sequence" => Stop::EndTurn,
                        "tool_use" => Stop::ToolUse,
                        "max_tokens" => Stop::MaxTokens,
                        "refusal" => Stop::Refusal(
                            event["delta"]["stop_details"]["explanation"]
                                .as_str()
                                .unwrap_or("the model declined this request")
                                .to_owned(),
                        ),
                        other => Stop::Other(other.to_owned()),
                    });
                }
                let out = n(&event["usage"], "output_tokens");
                if out > 0 {
                    self.usage.output = out;
                }
            }
            "error" => {
                self.error = Some(s(&event["error"], "message"));
            }
            _ => {}
        }
    }

    /// The assembled message, or the stream's error.
    pub(crate) fn finish(self) -> Result<Reply, String> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let blocks = self
            .blocks
            .into_iter()
            .filter_map(|p| match p {
                Partial::Text(text) => (!text.is_empty()).then_some(Block::Text { text }),
                Partial::Thinking(thinking, signature) => Some(Block::Thinking {
                    thinking,
                    signature,
                }),
                Partial::Tool(id, name, args) => Some(Block::ToolUse {
                    id,
                    name,
                    input: parse_args(&args),
                }),
                Partial::Opaque(raw) => Some(Block::Opaque { raw }),
            })
            .collect();
        let stop = self
            .stop
            .ok_or_else(|| "the response ended before the model finished".to_owned())?;
        Ok(Reply {
            blocks,
            stop,
            usage: self.usage,
            model: self.model,
        })
    }
}

/// Tool arguments as JSON, or the raw text under [`INVALID_JSON`].
fn parse_args(args: &str) -> Value {
    if args.trim().is_empty() {
        return json!({});
    }
    match serde_json::from_str::<Value>(args) {
        Ok(v) if v.is_object() => v,
        Ok(_) | Err(_) => json!({ INVALID_JSON: args }),
    }
}

/// An HTTP agent that leaves status handling to us, over the system's TLS.
fn agent(read_timeout: Option<Duration>) -> ureq::Agent {
    ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_response(read_timeout)
        .build()
        .into()
}

/// The `message` of an API error body, or the body itself.
fn error_message(status: u16, body: &str) -> String {
    let msg = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(300).collect());
    match status {
        401 => format!("the API key was rejected ({msg}); check it in the assistant settings"),
        403 => format!("this key may not use that model ({msg})"),
        429 => format!("rate limited, try again in a moment ({msg})"),
        529 => format!("the API is overloaded, try again in a moment ({msg})"),
        _ => format!("HTTP {status}: {msg}"),
    }
}

impl Provider for Claude {
    fn label(&self) -> String {
        self.model.clone()
    }

    fn send(
        &self,
        request: &Request<'_>,
        on: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<Reply, String> {
        let body = claude_body(&self.model, request);
        let agent = agent(Some(Duration::from_secs(180)));
        let mut attempt = 0;
        loop {
            let mut req = agent
                .post(&format!("{}/v1/messages", self.base.trim_end_matches('/')))
                .header("x-api-key", &self.key)
                .header("anthropic-version", API_VERSION)
                .header("content-type", "application/json");
            if has_fallback(&self.model) {
                req = req.header("anthropic-beta", FALLBACK_BETA);
            }
            let response = req
                .send_json(&body)
                .map_err(|e| format!("could not reach the Claude API: {e}"))?;
            let status = response.status().as_u16();
            if status != 200 {
                let text = response.into_body().read_to_string().unwrap_or_default();
                let retry = matches!(status, 429 | 500 | 502 | 503 | 504 | 529);
                if retry && attempt < 2 && !cancel.load(Ordering::Relaxed) {
                    attempt += 1;
                    std::thread::sleep(Duration::from_secs(2 * attempt * attempt));
                    continue;
                }
                return Err(error_message(status, &text));
            }
            let reader = std::io::BufReader::new(response.into_body().into_reader());
            let mut sse = Sse::default();
            for line in reader.lines() {
                if cancel.load(Ordering::Relaxed) {
                    return Err("stopped".to_owned());
                }
                let line = line.map_err(|e| format!("the stream broke: {e}"))?;
                sse.line(&line, on);
            }
            return sse.finish();
        }
    }
}

/// A local llama.cpp server.
#[derive(Clone, Debug)]
pub(crate) struct Local {
    /// Server URL.
    pub(crate) url: String,
}

/// The chat-completions request body.
pub(crate) fn local_body(request: &Request<'_>) -> Value {
    let mut messages = vec![json!({"role": "system", "content": request.system})];
    for m in request.msgs {
        match m.role {
            Role::User => {
                let mut text = Vec::new();
                for b in &m.blocks {
                    match b {
                        Block::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => messages.push(
                            json!({"role": "tool", "tool_call_id": tool_use_id, "content": content}),
                        ),
                        Block::Text { text: t } | Block::Context { text: t } => {
                            text.push(t.clone());
                        }
                        Block::Thinking { .. } | Block::ToolUse { .. } | Block::Opaque { .. } => {}
                    }
                }
                if !text.is_empty() {
                    messages.push(json!({"role": "user", "content": text.join("\n\n")}));
                }
            }
            Role::Assistant => {
                let text = m.text();
                let calls: Vec<Value> = m
                    .tool_uses()
                    .map(|(id, name, input)| {
                        json!({"id": id, "type": "function",
                            "function": {"name": name, "arguments": input.to_string()}})
                    })
                    .collect();
                let mut msg = json!({"role": "assistant", "content": text});
                if !calls.is_empty() {
                    msg["tool_calls"] = Value::Array(calls);
                }
                messages.push(msg);
            }
        }
    }
    let tools: Vec<Value> = request
        .tools
        .as_array()
        .map(|a| {
            a.iter()
                .map(|t| {
                    json!({"type": "function", "function": {
                        "name": t["name"], "description": t["description"],
                        "parameters": t["input_schema"]}})
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "messages": messages,
        "tools": tools,
        "temperature": 0.3,
        "max_tokens": 4096,
        "stream": false
    })
}

/// Parses a chat-completions response.
pub(crate) fn local_reply(v: &Value) -> Result<Reply, String> {
    let choice = &v["choices"][0];
    let msg = &choice["message"];
    if msg.is_null() {
        return Err(format!(
            "the local server's reply has no message: {}",
            v.to_string().chars().take(300).collect::<String>()
        ));
    }
    let mut blocks = Vec::new();
    let mut content = msg["content"].as_str().unwrap_or_default().to_owned();
    let mut thinking = msg["reasoning_content"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if let Some(start) = content.find("<think>") {
        let end = content[start..]
            .find("</think>")
            .map_or(content.len(), |e| start + e + "</think>".len());
        thinking.push_str(
            content[start..end]
                .trim_start_matches("<think>")
                .trim_end_matches("</think>"),
        );
        content.replace_range(start..end, "");
    }
    if !thinking.trim().is_empty() {
        blocks.push(Block::Thinking {
            thinking: thinking.trim().to_owned(),
            signature: String::new(),
        });
    }
    if !content.trim().is_empty() {
        blocks.push(Block::Text {
            text: content.trim().to_owned(),
        });
    }
    let mut calls = 0;
    for (k, call) in msg["tool_calls"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let f = &call["function"];
        let id = call["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map_or_else(|| format!("call_{k}"), str::to_owned);
        let args = &f["arguments"];
        let input = if let Some(s) = args.as_str() {
            parse_args(s)
        } else if args.is_object() {
            args.clone()
        } else {
            json!({})
        };
        blocks.push(Block::ToolUse {
            id,
            name: f["name"].as_str().unwrap_or_default().to_owned(),
            input,
        });
        calls += 1;
    }
    let stop = match choice["finish_reason"].as_str().unwrap_or_default() {
        "length" => Stop::MaxTokens,
        _ if calls > 0 => Stop::ToolUse,
        _ => Stop::EndTurn,
    };
    let n = |k: &str| v["usage"][k].as_u64().unwrap_or(0);
    Ok(Reply {
        blocks,
        stop,
        usage: Usage {
            input: n("prompt_tokens"),
            cached: 0,
            output: n("completion_tokens"),
        },
        model: v["model"].as_str().unwrap_or("local").to_owned(),
    })
}

impl Provider for Local {
    fn label(&self) -> String {
        format!("local ({})", self.url)
    }

    fn send(
        &self,
        request: &Request<'_>,
        on: &mut dyn FnMut(Delta),
        cancel: &AtomicBool,
    ) -> Result<Reply, String> {
        let agent = agent(Some(Duration::from_secs(300)));
        let response = agent
            .post(&format!(
                "{}/v1/chat/completions",
                self.url.trim_end_matches('/')
            ))
            .send_json(local_body(request))
            .map_err(|e| {
                format!(
                    "the local model server at {} is not answering ({e}); start llama-server or \
                     set AVE_LLM_URL",
                    self.url
                )
            })?;
        if cancel.load(Ordering::Relaxed) {
            return Err("stopped".to_owned());
        }
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|e| format!("reading the local server's reply: {e}"))?;
        if status != 200 {
            return Err(error_message(status, &text));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| format!("the local server's reply is not JSON: {e}"))?;
        let reply = local_reply(&v)?;
        for b in &reply.blocks {
            match b {
                Block::Text { text } => on(Delta::Text(text.clone())),
                Block::Thinking { thinking, .. } => on(Delta::Thinking(thinking.clone())),
                Block::ToolUse { name, .. } => on(Delta::Tool(name.clone())),
                Block::Context { .. } | Block::ToolResult { .. } | Block::Opaque { .. } => {}
            }
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Delta, Request, Sse, Stop};
    use crate::gui::assistant::model::{Block, Msg, Role};

    fn convo() -> Vec<Msg> {
        vec![
            Msg::new(
                Role::User,
                vec![
                    Block::Context {
                        text: "<context>plan</context>".into(),
                    },
                    Block::Text {
                        text: "swap shot 5".into(),
                    },
                ],
            ),
            Msg::new(
                Role::Assistant,
                vec![
                    Block::Thinking {
                        thinking: "local thought".into(),
                        signature: String::new(),
                    },
                    Block::Thinking {
                        thinking: "claude thought".into(),
                        signature: "sig".into(),
                    },
                    Block::ToolUse {
                        id: "t1".into(),
                        name: "get_plan".into(),
                        input: json!({}),
                    },
                ],
            ),
            Msg::new(
                Role::User,
                vec![Block::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "the plan".into(),
                    is_error: false,
                }],
            ),
            Msg::new(
                Role::User,
                vec![Block::Text {
                    text: "and?".into(),
                }],
            ),
        ]
    }

    #[test]
    fn claude_bodies_cache_merge_and_drop_unsigned_thinking() -> anyhow::Result<()> {
        let msgs = convo();
        let tools = json!([]);
        let req = Request {
            system: "sys",
            msgs: &msgs,
            tools: &tools,
        };
        let body = super::claude_body("claude-opus-5-5", &req);
        assert_eq!(
            body["system"][0]["cache_control"]["type"], "ephemeral",
            "system cached"
        );
        assert_eq!(body["thinking"]["type"], "adaptive", "adaptive thinking");
        assert_eq!(body["fallbacks"], "default", "fallback on opus");
        let messages = body["messages"].as_array().map(Vec::len);
        assert_eq!(messages, Some(3), "the two trailing user messages merge");
        let assistant = &body["messages"][1]["content"];
        assert_eq!(
            assistant.as_array().map(Vec::len),
            Some(2),
            "unsigned thinking dropped: {assistant}"
        );
        assert_eq!(
            body["messages"][2]["content"][0]["type"], "tool_result",
            "result first"
        );
        let haiku = super::claude_body("claude-haiku-5-5", &req);
        assert!(haiku.get("fallbacks").is_none(), "no fallback on haiku");
        Ok(())
    }

    #[test]
    fn a_streamed_reply_assembles() -> anyhow::Result<()> {
        let events = [
            r#"data: {"type":"message_start","message":{"model":"claude-opus-5-5","usage":{"input_tokens":10,"cache_read_input_tokens":90,"output_tokens":1}}}"#,
            r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Look at shot 5."}}"#,
            r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"abc"}}"#,
            r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}"#,
            r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Searching"}}"#,
            r#"data: {"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"search_shots","input":{}}}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"query\":"}}"#,
            r#"data: {"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"fire\"}"}}"#,
            r#"data: {"type":"content_block_start","index":3,"content_block":{"type":"redacted_thinking","data":"xyz"}}"#,
            r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":42}}"#,
            "event: message_stop",
        ];
        let mut sse = Sse::default();
        let mut seen = Vec::new();
        for e in events {
            sse.line(e, &mut |d| seen.push(d));
        }
        let reply = sse.finish().map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(reply.stop, Stop::ToolUse, "stop reason");
        assert_eq!(reply.usage.input, 100, "input counts cache reads");
        assert_eq!(reply.usage.output, 42, "output");
        assert_eq!(reply.blocks.len(), 4, "four blocks: {:?}", reply.blocks);
        assert_eq!(
            reply.blocks[0],
            Block::Thinking {
                thinking: "Look at shot 5.".into(),
                signature: "abc".into()
            },
            "thinking with signature"
        );
        assert_eq!(
            reply.blocks[2],
            Block::ToolUse {
                id: "t1".into(),
                name: "search_shots".into(),
                input: json!({"query": "fire"})
            },
            "tool call parsed"
        );
        assert!(
            matches!(&reply.blocks[3], Block::Opaque { raw } if raw["type"] == "redacted_thinking"),
            "unknown blocks kept"
        );
        assert!(
            seen.contains(&Delta::Text("Searching".into())),
            "text streamed"
        );
        assert!(
            seen.contains(&Delta::Tool("search_shots".into())),
            "tool announced"
        );
        Ok(())
    }

    #[test]
    fn stream_errors_and_bad_arguments() -> anyhow::Result<()> {
        let mut sse = Sse::default();
        sse.line(
            r#"data: {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            &mut |_| {},
        );
        assert_eq!(
            sse.finish().err().as_deref(),
            Some("Overloaded"),
            "error event"
        );
        assert!(
            super::parse_args("{\"query\": ")
                .get(super::INVALID_JSON)
                .is_some(),
            "truncated arguments are flagged"
        );
        assert!(
            Sse::default().finish().is_err(),
            "no stop reason is an error"
        );
        Ok(())
    }

    #[test]
    fn local_shapes_round_trip() -> anyhow::Result<()> {
        let msgs = convo();
        let tools = crate::gui::assistant::tools::definitions();
        let req = Request {
            system: "sys",
            msgs: &msgs,
            tools: &tools,
        };
        let body = super::local_body(&req);
        let roles: Vec<&str> = body["messages"]
            .as_array()
            .map(|a| a.iter().filter_map(|m| m["role"].as_str()).collect())
            .unwrap_or_default();
        assert_eq!(
            roles,
            ["system", "user", "assistant", "tool", "user"],
            "roles"
        );
        assert_eq!(body["tools"][0]["type"], "function", "function tools");
        let reply = super::local_reply(&json!({
            "model": "qwen",
            "choices": [{"finish_reason": "tool_calls", "message": {
                "content": "<think>hmm</think>Let me look.",
                "tool_calls": [{"id": "", "function": {"name": "get_plan", "arguments": "{}"}}]
            }}],
            "usage": {"prompt_tokens": 5, "completion_tokens": 7}
        }))
        .map_err(|e| anyhow::anyhow!(e))?;
        assert_eq!(reply.stop, Stop::ToolUse, "tool stop");
        assert_eq!(
            reply.blocks.len(),
            3,
            "thinking, text, call: {:?}",
            reply.blocks
        );
        assert!(
            matches!(&reply.blocks[2], Block::ToolUse { id, .. } if id == "call_0"),
            "missing ids are made up"
        );
        assert!(
            super::error_message(401, "{}").contains("rejected"),
            "401 explained"
        );
        Ok(())
    }
}

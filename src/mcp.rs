// ============================================================================
// Module:       mcp
// Description:  `ave mcp`: the engine as Model Context Protocol tools over
//               stdio, so an agent (Claude Code) can work inside a night.
//
// Dependencies: serde_json, toml, image (JPEG contact sheets); crate::{project,
//               search, plan, draft, alternatives, refs, media}
// ============================================================================

//! The MCP server.
//!
//! This is how the chat panel's agent works: the app starts Claude Code with
//! `ave mcp --work <night>/_ave` as an MCP server, and the agent reads the
//! night, looks at frames and edits plans through these tools. The same
//! server works from any MCP client (Claude Desktop, Claude Code in a
//! terminal), so the editing knowledge lives in one place.
//!
//! **Editorial decisions belong to the agent; the tools measure, show and
//! apply.** Edits follow a working-copy model: `plan_edit` and `plan_draft`
//! return the validated result (the diff and the rule check) without saving
//! unless `commit: true`, and every commit is a new version that
//! `plan_revert` can undo. Nothing here deletes footage or overwrites a
//! version.
//!
//! Protocol: newline-delimited JSON-RPC 2.0 on stdin/stdout (`initialize`,
//! `tools/list`, `tools/call`, `ping`); logs go to stderr.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::gui::assistant::{ops, store};
use crate::plan::Plan;
use crate::project::Project;

/// The protocol revision this server speaks.
const PROTOCOL: &str = "2025-06-18";

/// The `[pages]` table in `ave.toml` that holds the assistant's options.
pub(crate) const PAGE_KEY: &str = "assistant";

/// The per-night option that lets the agent save edits itself.
pub(crate) const ALLOW_COMMIT: &str = "allow_commit";

/// The tool that asks to start a slow engine step (index, caption, render).
/// It runs nothing itself: the app turns the call into an approval card.
pub(crate) const RUN_STEP: &str = "run_step";

/// How `run_step` answers: the request is with the person.
pub(crate) const APPROVAL_REQUESTED: &str = "APPROVAL_REQUESTED";

/// How `plan_draft` answers the agent when it names no length or aspect:
/// the person picks the first cut's settings on a card.
pub(crate) const SETTINGS_NEEDED: &str = "SETTINGS_NEEDED";

/// Who is calling a tool: the agent, whose saves the person must allow, or
/// the person (Apply on a card), who may always save.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Caller {
    /// The model.
    Agent,
    /// The person, through the panel.
    Person,
}

/// Serves MCP on stdin/stdout until stdin closes.
pub(crate) fn serve(work: &Path) -> Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(msg) => handle(work, &msg),
            Err(e) => Some(error_reply(
                &Value::Null,
                -32700,
                &format!("parse error: {e}"),
            )),
        };
        if let Some(reply) = reply {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
    }
    Ok(())
}

/// A JSON-RPC error reply.
fn error_reply(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Handles one message; notifications get no reply.
fn handle(work: &Path, msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let method = msg["method"].as_str().unwrap_or_default();
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": msg["params"]["protocolVersion"].as_str().unwrap_or(PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "ave", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS.replace(
                "{about}",
                &crate::recipe::Recipe::load(work).0.about,
            ),
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or_default();
            let args = msg["params"]["arguments"].clone();
            Ok(match call(work, name, &args, Caller::Agent) {
                Ok(content) => json!({"content": content, "isError": false}),
                Err(e) => {
                    json!({"content": [{"type": "text", "text": format!("{e:#}")}], "isError": true})
                }
            })
        }
        other => Err(format!("unknown method {other}")),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => error_reply(&id, -32601, &e),
    })
}

/// What the server tells the agent about itself on connect.
const INSTRUCTIONS: &str = "ave edits {about} into cuts (beat-synced when there is music). \
Start with night_overview. Find footage with search_shots (free text plus facets such as \
fire>0.3 -dark person:x move:zoom-in pick) and read exact facts with segment_details. Use look to see frames when a \
decision depends on the picture. Plans are storyboards on the music's beat grid: plan_get shows one with its rule \
check (each finding is `<where> [<hard|soft> <id>] <message>`: hard ones, such as bad-span or beat-count, make the \
plan invalid and must be fixed; soft ones, such as jump-cut, exposure-jump or repeat-performer, are taste calls); \
plan_edit applies ops to a working copy and returns the diff and check without saving until commit:true; \
alternatives ranks replacements for a slot; plan_fix lists every finding with what it means and its automatic \
fixes (what each does to the check, and the ops to stage it with plan_edit). Rules: no repeated shots or performers, best moments only, the drop on \
the hero fire shot, no slow motion or digital zoom. You own the editorial decisions; say why for each shot in its \
note. A commit needs the base_version plan_get returned and is refused unless the person allowed the assistant to \
apply edits; otherwise stage and let them apply. Every commit is a new version (plan_versions, plan_revert).";

/// The tool definitions in the Messages API's shape (`input_schema`), for
/// the in-process loop: the same tools Claude Code sees over MCP.
pub(crate) fn api_tools() -> Value {
    Value::Array(
        tools()
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|t| {
                        json!({"name": t["name"], "description": t["description"],
                            "input_schema": t["inputSchema"]})
                    })
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// The tool definitions.
pub(crate) fn tools() -> Value {
    let ops_schema = json!({
        "type": "array",
        "description": "Edits, applied in order; slots and marker numbers are 1-based and each op sees the plan as earlier ops left it. add_marker {time, note, color} pins a marker (a review comment) at time seconds on the cut; remove_marker / resolve_marker {marker} take the number plan_get lists (m3 is marker 3).",
        "items": {"type": "object", "required": ["op"], "properties": {
            "op": {"type": "string", "enum": ["replace_shot", "set_in", "set_beats", "move_shot", "remove_shot", "set_cx", "set_note", "add_marker", "remove_marker", "resolve_marker"]},
            "index": {"type": "integer"}, "clip": {"type": "string"}, "in": {"type": "number"},
            "cx": {"type": "number"}, "beats": {"type": "integer"}, "from": {"type": "integer"},
            "to": {"type": "integer"}, "note": {"type": "string"}, "reason": {"type": "string"},
            "time": {"type": "number", "description": "add_marker: seconds from the plan's start."},
            "color": {"type": "string", "enum": crate::plan::COMMENT_COLORS},
            "marker": {"type": "integer", "description": "remove_marker / resolve_marker: the marker's number."}
        }}
    });
    json!([
        {"name": "night_overview", "description": "The open night: clips, segments and flags, captions, the night's sets (acts by recording time and setting, with their clips), music grid and drops, people, plans with their rule-check status.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "search_shots", "description": "Search the shot log. Free text matches captions/people/settings/clip ids; facets: fire>0.3 led>0.2 move>0.5 focus<0.8 q>0.7 dark soft flag:blown move:pan|zoom-in|zoom-out|static person:<name> clip:MVI_79 set:3 (the clips of the night's third act; night_overview lists the sets) moment:performing framing:close pick reject like:<segment id>; '-' negates. Shares are relative to the night (1 = its strongest). The first line says whether the results are complete: PARTIAL means some segments in scope have no caption, so free text and caption facets could not see them. Each hit's focal time is its best moment (seconds in the clip): the frame to look at or cut around.",
         "inputSchema": {"type": "object", "required": ["query"], "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "default": 25}}}},
        {"name": "segment_details", "description": "Exact facts for segments (ids like MVI_7987/3): times, camera, exposure, focus, fire/LED/moving light, action centre, quality, flags with thresholds, bad spans (short bad stretches inside the segment: focus hunt, whip, jolt, dark or blown moment), moments (fire bursts, and crowd cheers from the camera audio, with onset times), caption, who, plans using it.",
         "inputSchema": {"type": "object", "required": ["ids"], "properties": {"ids": {"type": "array", "items": {"type": "string"}}}}},
        {"name": "look", "description": "See frames: a contact sheet (JPEG) of up to 12 moments, left to right, top to bottom. Each item is a segment id (its mid time, or 'at') or a clip id with 'at' seconds. Use it when a choice depends on the picture.",
         "inputSchema": {"type": "object", "required": ["items"], "properties": {
            "items": {"type": "array", "items": {"type": "object", "properties": {"segment": {"type": "string"}, "clip": {"type": "string"}, "at": {"type": "number"}}}},
            "width": {"type": "integer", "default": 320}}}},
        {"name": "music_map", "description": "The track: bpm, bars with energy (low end, rms), sections, drop candidates (strongest first), song ranges.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "plan_get", "description": "A plan: bars, aspect, every shot with its time on the cut, beats, clip, in-point, crop, who and note, its numbered markers (review comments: time, colour, author, note, resolved), plus the rule-check problems and its version (for base_version).",
         "inputSchema": {"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}}},
        {"name": "plan_edit", "description": "Apply edit ops to a plan's working copy. Returns the diff, the rule check (new problems, problems fixed) and the base_version. Without commit the edit is staged: the person sees it as a card and applies it. commit:true saves a new version, only when the person allowed the assistant to apply edits and only if base_version still matches the plan file.",
         "inputSchema": {"type": "object", "required": ["name", "ops", "reason"], "properties": {
            "name": {"type": "string"}, "ops": ops_schema, "reason": {"type": "string", "description": "One sentence: what changes and why (the card's title)."},
            "commit": {"type": "boolean", "default": false},
            "base_version": {"type": "string", "description": "The version plan_get or the staged plan_edit returned; required with commit."}}}},
        {"name": "plan_draft", "description": "Draft a plan from the shot log and the music (hook, breakdown, build, hero on the drop, phrase pacing, finale). Returns it with its check; saves only with commit:true.",
         "inputSchema": {"type": "object", "required": ["name"], "properties": {
            "name": {"type": "string"}, "aspect": {"type": "string", "enum": ["9x16", "4x5", "16x9"], "default": "9x16"},
            "seconds": {"type": "number", "default": 30}, "end_card": {"type": "string"}, "commit": {"type": "boolean", "default": false}}}},
        {"name": "alternatives", "description": "Ranked footage that could replace shot `index` (1-based) of a plan without breaking the rules, each with why it fits.",
         "inputSchema": {"type": "object", "required": ["plan", "index"], "properties": {"plan": {"type": "string"}, "index": {"type": "integer"}, "limit": {"type": "integer", "default": 8}}}},
        {"name": "plan_fix", "description": "Every rule-check finding of a plan with what it means in plain words and its automatic fixes, best first: each says what it changes and what it does to the check (clears the finding, never adds a hard one; new soft findings named) and carries the ops to stage it with plan_edit. Findings the person accepted are marked. Nothing is saved.",
         "inputSchema": {"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}, "limit": {"type": "integer", "default": 3, "description": "Fixes per finding."}}}},
        {"name": "plan_versions", "description": "A plan's saved versions, oldest first, with what changed.",
         "inputSchema": {"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}}},
        {"name": "plan_revert", "description": "Restore a saved version (saved as a new version, nothing is lost).",
         "inputSchema": {"type": "object", "required": ["name", "version"], "properties": {"name": {"type": "string"}, "version": {"type": "integer"}}}},
        {"name": "people_map", "description": "Performers: name → clips/ranges, as the no-repeated-performer rule sees them.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": RUN_STEP, "description": "Ask to start a slow engine step: index the footage, caption the shot log, or render a plan to video. Nothing runs here: the ave app shows the person an approval card with the estimated time and, if they approve, runs it as a job with live progress in the chat. End your turn after calling it.",
         "inputSchema": {"type": "object", "required": ["step"], "properties": {
            "step": {"type": "string", "enum": ["index", "caption", "render"]},
            "plan": {"type": "string", "description": "The plan to render (render only)."}}}}
    ])
}

/// A text content block.
fn text(s: impl Into<String>) -> Vec<Value> {
    vec![json!({"type": "text", "text": s.into()})]
}

/// Runs one tool for `caller`; the content blocks of its result.
pub(crate) fn call(work: &Path, name: &str, args: &Value, caller: Caller) -> Result<Vec<Value>> {
    let project = Project::open(work)?;
    match name {
        "night_overview" => Ok(text(overview(&project))),
        "search_shots" => search(&project, args),
        "segment_details" => details(&project, args),
        "look" => look(&project, args),
        "music_map" => music(&project),
        "plan_get" => plan_get(&project, args),
        "plan_edit" => plan_edit(&project, args, caller),
        "plan_draft" => plan_draft(&project, args, caller),
        "alternatives" => alternatives(&project, args),
        "plan_fix" => plan_fix(&project, args),
        "plan_versions" => {
            let name = str_arg(args, "name")?;
            let lines: Vec<String> = store::versions(&project.work, name)
                .iter()
                .map(|v| format!("v{} {}", v.n, v.header))
                .collect();
            Ok(text(if lines.is_empty() {
                "no versions yet".to_owned()
            } else {
                lines.join("\n")
            }))
        }
        "plan_revert" => {
            let name = str_arg(args, "name")?;
            let n = args["version"].as_u64().context("version is required")? as usize;
            gate(&project, caller, "the revert")?;
            let v = store::versions(&project.work, name)
                .into_iter()
                .find(|v| v.n == n)
                .with_context(|| format!("{name} has no v{n}"))?;
            let mut plan = Plan::load(&v.path)?;
            // The live plan's markers (review comments) survive a revert.
            if let Ok(live) = Plan::load(&plan_path(&project, name)) {
                plan.comments = live.comments;
            }
            let saved =
                store::save_version(&project.work, name, &plan, &format!("revert to v{n}"))?;
            plan.save(&plan_path(&project, name))?;
            Ok(text(format!(
                "{name} restored from v{n} (saved as v{saved})"
            )))
        }
        RUN_STEP => run_step(&project, args),
        "people_map" => {
            let mut by: std::collections::BTreeMap<&str, Vec<&str>> =
                std::collections::BTreeMap::new();
            for (k, n) in &project.people.names {
                by.entry(n.as_str()).or_default().push(k.as_str());
            }
            let lines: Vec<String> = by
                .iter()
                .map(|(n, ks)| format!("{n}: {}", ks.join(", ")))
                .collect();
            Ok(text(if lines.is_empty() {
                "no people map yet".to_owned()
            } else {
                lines.join("\n")
            }))
        }
        other => anyhow::bail!("unknown tool {other}"),
    }
}

/// A required string argument.
fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args[key]
        .as_str()
        .with_context(|| format!("`{key}` is required"))
}

/// `<work>/plans/<name>.toml`.
fn plan_path(project: &Project, name: &str) -> PathBuf {
    project.work.join("plans").join(format!("{name}.toml"))
}

/// The night in a few lines.
fn overview(p: &Project) -> String {
    let mut s = format!("night {} ({})\n", p.name(), p.work.display());
    match &p.index {
        Some(i) => {
            let flagged = i.segments.iter().filter(|g| !g.flags.is_empty()).count();
            let captioned = i.segments.iter().filter(|g| g.caption.is_some()).count();
            let minutes: f64 = i.clips.iter().map(|c| c.probe.duration).sum::<f64>() / 60.0;
            s.push_str(&format!(
                "footage: {} clips, {minutes:.1} min, {} segments ({flagged} flagged, {captioned} captioned)\n",
                i.clips.len(),
                i.segments.len()
            ));
            s.push_str(&sets_section(i));
        }
        None => s.push_str("footage: not indexed yet\n"),
    }
    match &p.music {
        Some(m) => {
            let drops: Vec<String> = m
                .drops
                .iter()
                .take(3)
                .map(|d| format!("bar {} ({:.1}s)", d.bar, d.t))
                .collect();
            s.push_str(&format!(
                "music: {} · {:.1} BPM · {} bars · drops {}\n",
                m.path,
                m.bpm,
                m.bars.len(),
                drops.join(", ")
            ));
        }
        None => s.push_str("music: not mapped yet\n"),
    }
    s.push_str(&format!("people: {} map entries\n", p.people.names.len()));
    for path in &p.plans {
        let stem = path
            .file_stem()
            .map(|x| x.to_string_lossy().into_owned())
            .unwrap_or_default();
        let status = match (Plan::load(path), &p.index, &p.music) {
            (Ok(plan), Some(i), Some(m)) => {
                let problems = crate::plan::check(&plan, i, m, &p.people);
                format!(
                    "{} shots, {:?}, {}",
                    plan.shots.len(),
                    plan.aspect,
                    check_summary(&problems)
                )
            }
            (Err(e), _, _) => format!("unreadable: {e:#}"),
            _ => "not checkable yet".to_owned(),
        };
        s.push_str(&format!("plan {stem}: {status}\n"));
    }
    s
}

/// The night's acts for `night_overview`: one line per set with its
/// clips, so the agent can plan by act (open on the first, close on the
/// last) before searching shots.
fn sets_section(i: &crate::index::Index) -> String {
    let sets = crate::analyze::sets::group(i);
    if sets.is_empty() {
        return "sets: no recording times yet (re-run index to read them)\n".to_owned();
    }
    let mut s = format!(
        "sets ({}, the night's acts in recording order: split by pauses over {} min and lasting indoor/outdoor changes):\n",
        sets.len(),
        crate::analyze::sets::GAP_SECS / 60
    );
    for set in &sets {
        s.push_str(&format!(
            "  {} — clips {}\n",
            set.heading(),
            set.clips.join(" ")
        ));
    }
    s
}

/// `search_shots`.
fn search(p: &Project, args: &Value) -> Result<Vec<Value>> {
    let index = p.index.as_ref().context("the night is not indexed yet")?;
    let query = crate::search::Query::parse(str_arg(args, "query")?);
    let limit = args["limit"].as_u64().unwrap_or(25) as usize;
    let (marks, _) = crate::search::marks::Marks::load(&p.work);
    let night = crate::search::Night::new(index, &p.people, &marks);
    let results = crate::search::search(&query, &night);
    let hits = &results.hits;
    let lines: Vec<String> = hits
        .iter()
        .take(limit)
        .filter_map(|h| {
            let s = index.segments.get(h.seg)?;
            let cap = s
                .caption
                .as_ref()
                .map(|c| format!(" — {}", c.summary))
                .unwrap_or_default();
            Some(format!(
                "{} {:.1}-{:.1}s focal {:.1}s score {:.2}: {}{cap}",
                s.id, s.seg.start, s.seg.end, h.focal, h.score, h.reason
            ))
        })
        .collect();
    let coverage = if results.complete {
        "complete: every segment in scope was searchable".to_owned()
    } else {
        format!(
            "PARTIAL: {} segments in scope have no caption, so the caption terms could not see them \
             (complete=false, uncaptioned_in_scope={}); caption the night or use measured facets",
            results.uncaptioned_in_scope, results.uncaptioned_in_scope
        )
    };
    Ok(text(format!(
        "{} matches{} · {coverage}\n{}",
        hits.len(),
        if hits.len() > limit {
            format!(", first {limit}")
        } else {
            String::new()
        },
        lines.join("\n")
    )))
}

/// `segment_details`.
fn details(p: &Project, args: &Value) -> Result<Vec<Value>> {
    let ids = args["ids"].as_array().context("`ids` is required")?;
    let mut out = Vec::new();
    for id in ids.iter().filter_map(Value::as_str) {
        let r = crate::refs::Reference::new(crate::refs::Target::Segment { id: id.to_owned() });
        let facts = crate::refs::resolve(&r, p).map_or_else(|e| e, |x| x.facts);
        let seg = p
            .index
            .as_ref()
            .and_then(|i| i.segments.iter().find(|s| s.id == id));
        let extra = seg.map_or_else(String::new, |s| {
            let g = &s.seg;
            let who = p.people.name_at(&s.clip, g.start);
            let caption = s.caption.as_ref().map_or_else(String::new, |c| {
                let people: Vec<String> = c.people.iter().map(|x| format!("{} [{}]", x.appearance, x.prop)).collect();
                format!("\n  caption: {} | moment {} | framing {} | setting {} | people: {} | issues: {}", c.summary, c.effective_moment(), c.framing, c.setting, people.join("; "), c.issues.join(", "))
            });
            let mut moments = String::new();
            if !g.moments.is_empty() {
                let at: Vec<String> = g.moments.iter().map(|t| format!("{t:.2}")).collect();
                moments.push_str(&format!("\n  moments: fire burst {:.3} at {}", g.burst, at.join(", ")));
            }
            if !g.cheers.is_empty() {
                let at: Vec<String> = g
                    .cheers
                    .iter()
                    .map(|c| format!("{:.2}s (strength {:.2}, treble +{:.1} dB for {:.1}s)", c.t, c.strength, c.rise, c.dur))
                    .collect();
                moments.push_str(&format!("\n  moments: crowd cheer at {}", at.join(", ")));
            }
            let bad = if g.bad_spans.is_empty() {
                String::new()
            } else {
                let spans: Vec<String> = g.bad_spans.iter().map(|b| b.text()).collect();
                format!(" · bad spans (avoid these seconds): {}", spans.join(", "))
            };
            format!(
                "\n  camera {:?} · exposure {:.2}/{:.2} · focus {:.2} · fire {:.3} · led {:.3} · moving {:.3} · cx {:.2}±{:.2} · q {:.2} · flags {:?}{bad} · who {who}{moments}{caption}",
                g.camera, g.luma, g.p95, g.sharp, g.flame, g.led, g.moving, g.cx, g.cx_spread / 2.0, s.q, s.flags
            )
        });
        out.push(format!("{id}: {facts}{extra}"));
    }
    Ok(text(out.join("\n")))
}

/// `music_map`.
fn music(p: &Project) -> Result<Vec<Value>> {
    let m = p.music.as_ref().context("the music is not mapped yet")?;
    let mut s = format!(
        "{} · {:.2} BPM · {} bars · {:.1}s\n",
        m.path,
        m.bpm,
        m.bars.len(),
        m.duration
    );
    for d in &m.drops {
        s.push_str(&format!(
            "drop bar {} at {:.2}s (score {:.2})\n",
            d.bar, d.t, d.score
        ));
    }
    s.push_str("bars (index: start s, low-end energy, rms):\n");
    for (i, b) in m.bars.iter().enumerate() {
        s.push_str(&format!("{i}: {:.2} {:.2} {:.3}\n", b.t, b.low, b.rms));
    }
    Ok(text(s))
}

/// One line for a check result: `ok`, `ok, 3 soft` or `FAILS: 1 hard, 3 soft`.
/// Only hard findings fail; soft ones are taste warnings.
fn check_summary(findings: &[crate::plan::Finding]) -> String {
    let hard = crate::plan::hard_count(findings);
    let soft = findings.len() - hard;
    match (hard, soft) {
        (0, 0) => "ok".to_owned(),
        (0, _) => format!("ok, {soft} soft"),
        (_, _) => format!("FAILS: {hard} hard, {soft} soft"),
    }
}

/// A plan's markers (review comments), numbered as `remove_marker` and
/// `resolve_marker` take them; empty when it has none.
pub(crate) fn markers_text(plan: &Plan) -> String {
    if plan.comments.is_empty() {
        return String::new();
    }
    let mut s = format!("markers ({}):\n", plan.comments.len());
    for (i, c) in plan.comments.iter().enumerate() {
        s.push_str(&format!(
            "m{} {:.2}s {} · {}{} · {}\n",
            i + 1,
            c.time,
            c.color,
            c.author,
            if c.resolved { " · resolved" } else { "" },
            if c.note.is_empty() {
                "(no note)"
            } else {
                c.note.trim()
            }
        ));
    }
    s
}

/// A plan as text, with its check.
fn describe_plan(p: &Project, plan: &Plan) -> String {
    let mut s = format!(
        "{} · {:?} · bars {}..{} (drop {}, card {}) · {} shots\n",
        plan.name,
        plan.aspect,
        plan.start_bar,
        plan.end_bar,
        plan.drop_bar,
        plan.card_bar,
        plan.shots.len()
    );
    let timing = p.music.as_ref().map(|m| plan.timing(m)).unwrap_or_default();
    for (i, shot) in plan.shots.iter().enumerate() {
        let at = timing
            .get(i)
            .map_or(String::new(), |(t, d)| format!("{t:5.2}s +{d:.2}s "));
        s.push_str(&format!(
            "#{} {at}{} beats · {} @{:.2} cx {:.2} · {} · {}\n",
            i + 1,
            shot.beats,
            shot.clip,
            shot.t_in,
            shot.cx,
            shot.who,
            shot.note
        ));
    }
    s.push_str(&markers_text(plan));
    match ops::check(plan, p.index.as_ref(), p.music.as_ref(), &p.people) {
        Some(problems) => {
            s.push_str(&format!("check: {}\n", check_summary(&problems)));
            for pr in problems {
                s.push_str(&format!("  {}\n", pr.text()));
            }
        }
        None => s.push_str("check: needs the index and the music map\n"),
    }
    s
}

/// `plan_get`.
fn plan_get(p: &Project, args: &Value) -> Result<Vec<Value>> {
    let name = str_arg(args, "name")?;
    let path = plan_path(p, name);
    let plan = Plan::load(&path)?;
    let mut s = describe_plan(p, &plan);
    s.push_str(&format!(
        "version: {} (pass as base_version to plan_edit)\n",
        plan_version(&path)?
    ));
    Ok(text(s))
}

/// A plan file's version: a hash of its bytes, so any change to the file
/// (by the person, the timeline or another agent) gives a new one.
pub(crate) fn plan_version(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(version_of(&bytes))
}

/// FNV-1a, 64 bits, as hex.
fn version_of(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Whether the person lets the agent save edits in this night
/// (`[pages.assistant] allow_commit` in `ave.toml`, the panel's "Let the
/// assistant apply edits").
pub(crate) fn commits_allowed(p: &Project) -> bool {
    p.settings
        .pages
        .get(PAGE_KEY)
        .and_then(|t| t.get(ALLOW_COMMIT))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
}

/// Refuses a save the person has not allowed.
fn gate(p: &Project, caller: Caller, what: &str) -> Result<()> {
    match caller {
        Caller::Person => Ok(()),
        Caller::Agent if commits_allowed(p) => Ok(()),
        Caller::Agent => anyhow::bail!(
            "COMMIT_REFUSED: {what} was not saved. The person has not turned on \"Let the \
             assistant apply edits\" for this night, so stage the edit (plan_edit without commit) \
             and let them apply it from the card."
        ),
    }
}

/// What a `plan_edit` did.
struct Edited {
    /// The report for the agent.
    text: String,
    /// The version saved, when it committed.
    saved: Option<usize>,
}

/// `plan_edit`: working copy, diff and check; saves only with commit.
fn plan_edit(p: &Project, args: &Value, caller: Caller) -> Result<Vec<Value>> {
    Ok(text(edit(p, args, caller)?.text))
}

/// Applies a `plan_edit` call.
fn edit(p: &Project, args: &Value, caller: Caller) -> Result<Edited> {
    let name = str_arg(args, "name")?;
    let reason = args["reason"].as_str().unwrap_or("edit");
    let commit = args["commit"].as_bool().unwrap_or(false);
    let edit_ops: Vec<ops::EditOp> =
        serde_json::from_value(args["ops"].clone()).context("`ops` does not match the schema")?;
    let path = plan_path(p, name);
    let version = plan_version(&path)?;
    if commit {
        gate(p, caller, "the edit")?;
        let based = args["base_version"].as_str().context(
            "a commit needs base_version: the version plan_get or the staged plan_edit returned",
        )?;
        anyhow::ensure!(
            based == version,
            "STALE: {name} changed since you read it (it is now version {version}, you based the \
             edit on {based}); call plan_get, re-stage the edit and commit again"
        );
    }
    let base = Plan::load(&path)?;
    let night = ops::Night {
        index: p.index.as_ref(),
        people: &p.people,
    };
    let (edited, effects) = ops::apply(&base, &edit_ops, &ops::Scope::Whole, night)
        .map_err(|e| anyhow::anyhow!("{}", e.message))?;
    let before =
        ops::check(&base, p.index.as_ref(), p.music.as_ref(), &p.people).unwrap_or_default();
    let after =
        ops::check(&edited, p.index.as_ref(), p.music.as_ref(), &p.people).unwrap_or_default();
    let (new, fixed) = ops::compare(&before, &after);
    let mut s = String::new();
    for e in &effects {
        let side = |x: &Option<(usize, crate::plan::Shot)>| {
            x.as_ref().map_or("—".to_owned(), |(i, sh)| {
                format!("#{i} {} @{:.2}", sh.clip, sh.t_in)
            })
        };
        s.push_str(&format!(
            "{}: {} → {}{}\n",
            e.label,
            side(&e.before),
            side(&e.after),
            if e.reason.is_empty() {
                String::new()
            } else {
                format!(" ({})", e.reason)
            }
        ));
    }
    s.push_str(&format!(
        "check: {} problems after ({} new, {fixed} fixed); {}\n",
        after.len(),
        new.len(),
        check_summary(&after)
    ));
    for pr in &new {
        s.push_str(&format!("  new: {}\n", pr.text()));
    }
    let mut saved = None;
    if commit {
        let v = store::save_version(&p.work, name, &edited, reason)?;
        saved = Some(v);
        s.push_str(&format!(
            "committed as v{v}; version now {}\n",
            plan_version(&path)?
        ));
    } else {
        s.push_str(&format!(
            "staged, not saved: the person sees it as a card to apply. base_version: {version}\n"
        ));
    }
    Ok(Edited { text: s, saved })
}

/// Commits ops the person applied from a card: the same path as the agent's
/// `plan_edit` with `commit`, with the same stale-version check. Returns the
/// version saved.
pub(crate) fn commit_edit(
    work: &Path,
    name: &str,
    edit_ops: &[ops::EditOp],
    reason: &str,
    base_version: &str,
) -> Result<usize> {
    let project = Project::open(work)?;
    let args = json!({
        "name": name, "ops": edit_ops, "reason": reason, "commit": true,
        "base_version": base_version,
    });
    edit(&project, &args, Caller::Person)?
        .saved
        .context("the edit was not saved")
}

/// `run_step`: checked, never run here (see [`RUN_STEP`]).
fn run_step(p: &Project, args: &Value) -> Result<Vec<Value>> {
    let what = match str_arg(args, "step")? {
        "index" => {
            anyhow::ensure!(
                p.settings.footage.is_some(),
                "the night has no footage folder; the person sets it on the night page"
            );
            "index the footage".to_owned()
        }
        "caption" => {
            anyhow::ensure!(p.index.is_some(), "index the night first");
            "caption the shot log".to_owned()
        }
        "render" => {
            let name = str_arg(args, "plan")?;
            anyhow::ensure!(plan_path(p, name).exists(), "there is no plan {name}");
            format!("render {name}")
        }
        other => anyhow::bail!("step must be index, caption or render, not {other}"),
    };
    Ok(text(format!(
        "{APPROVAL_REQUESTED}: the person is asked to approve: {what}. The ave app shows them \
         the estimated time and, if they approve, runs it as a job with live progress. Nothing \
         has started yet. End your turn now and say you asked (outside the ave app nothing runs)."
    )))
}

/// `plan_draft`.
fn plan_draft(p: &Project, args: &Value, caller: Caller) -> Result<Vec<Value>> {
    let index = p.index.as_ref().context("the night is not indexed yet")?;
    let music = p.music.as_ref().context("the music is not mapped yet")?;
    let name = str_arg(args, "name")?;
    if caller == Caller::Agent && args.get("seconds").is_none() && args.get("aspect").is_none() {
        return Ok(text(format!(
            "{SETTINGS_NEEDED}: nothing was drafted. The person picks the first cut's length, \
             aspect, pacing, hero shot and rules on a card in the ave app; end your turn now and \
             their choice comes back as the next message. Outside the app, ask them for the \
             length and aspect, then call plan_draft again with seconds and aspect."
        )));
    }
    let aspect: crate::plan::Aspect =
        serde_json::from_value(args.get("aspect").cloned().unwrap_or(json!("9x16")))?;
    let end_card = args["end_card"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let key = match aspect {
                crate::plan::Aspect::Vertical => "9x16",
                crate::plan::Aspect::Feed => "4x5",
                crate::plan::Aspect::Wide => "16x9",
            };
            p.settings
                .end_cards
                .get(key)
                .map(|x| x.display().to_string())
                .unwrap_or_default()
        });
    let spec = crate::draft::Spec {
        name: name.to_owned(),
        aspect,
        seconds: args["seconds"].as_f64().unwrap_or(30.0) as f32,
        end_card,
        card_bars: 2,
        rules: crate::recipe::Recipe::load(&p.work).0.rules,
    };
    let plan = crate::draft::draft(index, music, &p.people, &spec, &music.path)?;
    let mut s = describe_plan(p, &plan);
    if args["commit"].as_bool().unwrap_or(false) {
        let path = plan_path(p, name);
        // A new plan overwrites nothing; redrafting one that exists does.
        if path.exists() {
            gate(p, caller, "the redraft")?;
        }
        if path.exists()
            && let Ok(old) = Plan::load(&path)
            && store::versions(&p.work, name).is_empty()
        {
            store::save_version(&p.work, name, &old, "before redraft")?;
        }
        let v = store::save_version(&p.work, name, &plan, "draft")?;
        plan.save(&path)?;
        s.push_str(&format!("committed as v{v}\n"));
    } else {
        s.push_str("not saved; call again with commit:true to save\n");
    }
    Ok(text(s))
}

/// `alternatives`.
fn alternatives(p: &Project, args: &Value) -> Result<Vec<Value>> {
    let index = p.index.as_ref().context("the night is not indexed yet")?;
    let music = p.music.as_ref().context("the music is not mapped yet")?;
    let plan = Plan::load(&plan_path(p, str_arg(args, "plan")?))?;
    let k = args["index"].as_u64().context("`index` is required")? as usize;
    anyhow::ensure!(k >= 1, "index is 1-based");
    let limit = args["limit"].as_u64().unwrap_or(8) as usize;
    let alts = crate::alternatives::rank(&plan, k - 1, index, music, &p.people, limit);
    let lines: Vec<String> = alts
        .iter()
        .map(|a| {
            format!(
                "{} ({}) @{:.2} cx {:.2} · {} · score {:.2} · {}",
                a.clip, a.seg, a.t_in, a.cx, a.who, a.score, a.reason
            )
        })
        .collect();
    Ok(text(if lines.is_empty() {
        "no alternatives keep the rules".to_owned()
    } else {
        lines.join("\n")
    }))
}

/// `plan_fix`: each finding with its advice and automatic fixes, as ops.
fn plan_fix(p: &Project, args: &Value) -> Result<Vec<Value>> {
    let index = p.index.as_ref().context("the night is not indexed yet")?;
    let music = p.music.as_ref().context("the music is not mapped yet")?;
    let name = str_arg(args, "name")?;
    let path = plan_path(p, name);
    let plan = Plan::load(&path)?;
    let limit = args["limit"].as_u64().unwrap_or(3).clamp(1, 8) as usize;
    let memory = crate::plan::Memory::for_index(index);
    let night = crate::fix::Night {
        index,
        map: music,
        people: &p.people,
        memory: &memory,
    };
    let findings = night.check(&plan);
    let mut s = format!("{name}: check {}\n", check_summary(&findings));
    for f in &findings {
        let accepted = plan.is_accepted(f);
        s.push_str(&format!(
            "{}{}\n  what: {}\n",
            f.text(),
            if accepted {
                " (accepted by the person)"
            } else {
                ""
            },
            crate::fix::advice(f.id)
        ));
        if accepted {
            continue;
        }
        let fixes = crate::fix::fixes(&plan, f, night, limit);
        if fixes.is_empty() {
            s.push_str(if crate::fix::fixable(f.id) {
                "  no safe automatic fix: edit by hand (alternatives, plan_edit)\n"
            } else {
                "  no automatic fix: the person's call\n"
            });
        }
        for (k, fix) in fixes.iter().enumerate() {
            s.push_str(&format!(
                "  fix {}: {}\n    ops: {}\n",
                k + 1,
                fix.preview(),
                serde_json::to_string(&fix.ops)?
            ));
        }
    }
    s.push_str(&format!(
        "stage a fix with plan_edit (name, ops, reason); base_version: {}\n",
        plan_version(&path)?
    ));
    Ok(text(s))
}

/// `look`: a JPEG contact sheet of up to 12 moments.
fn look(p: &Project, args: &Value) -> Result<Vec<Value>> {
    use base64::Engine as _;
    let index = p.index.as_ref().context("the night is not indexed yet")?;
    let items = args["items"].as_array().context("`items` is required")?;
    let width = args["width"].as_u64().unwrap_or(320).clamp(160, 640) as u32;
    let mut frames = Vec::new();
    let mut labels = Vec::new();
    for item in items.iter().take(12) {
        let (clip, t) = if let Some(seg) = item["segment"].as_str() {
            let s = index
                .segments
                .iter()
                .find(|s| s.id == seg)
                .with_context(|| format!("no segment {seg}"))?;
            (
                s.clip.clone(),
                item["at"]
                    .as_f64()
                    .unwrap_or(f64::from(s.seg.start + s.seg.end) / 2.0),
            )
        } else {
            (
                str_arg(item, "clip")?.to_owned(),
                item["at"].as_f64().unwrap_or(0.0),
            )
        };
        let c = index
            .clip(&clip)
            .with_context(|| format!("no clip {clip}"))?;
        let f = crate::media::frame_rgb(Path::new(&c.path), t, width)?;
        labels.push(format!("{} {clip} @{t:.2}", labels.len() + 1));
        frames.push(f);
    }
    anyhow::ensure!(!frames.is_empty(), "nothing to look at");
    let cols = frames.len().min(4);
    let rows = frames.len().div_ceil(cols);
    let cell_h = frames.iter().map(|f| f.height).max().unwrap_or(1);
    let cell_w = width as usize;
    let gap = 4;
    let (w, h) = (
        cols * cell_w + (cols - 1) * gap,
        rows * cell_h + (rows - 1) * gap,
    );
    let mut sheet = image::RgbImage::new(w as u32, h as u32);
    for (i, f) in frames.iter().enumerate() {
        let (x0, y0) = ((i % cols) * (cell_w + gap), (i / cols) * (cell_h + gap));
        for y in 0..f.height {
            for x in 0..f.width.min(cell_w) {
                let o = (y * f.width + x) * 3;
                sheet.put_pixel(
                    (x0 + x) as u32,
                    (y0 + y) as u32,
                    image::Rgb([f.rgb[o], f.rgb[o + 1], f.rgb[o + 2]]),
                );
            }
        }
    }
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 82).encode_image(&sheet)?;
    Ok(vec![
        json!({"type": "image", "mimeType": "image/jpeg", "data": base64::engine::general_purpose::STANDARD.encode(&jpeg)}),
        json!({"type": "text", "text": format!("{}\n(left to right, top to bottom)", labels.join("\n"))}),
    ])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{handle, tools};
    use serde_json::json;

    #[test]
    fn it_speaks_the_handshake_and_lists_tools() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let init = handle(
            dir.path(),
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
        );
        let init = init.ok_or_else(|| anyhow::anyhow!("initialize must reply"))?;
        assert_eq!(init["result"]["serverInfo"]["name"], "ave", "server name");
        assert!(
            handle(
                dir.path(),
                &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
            )
            .is_none(),
            "notifications get no reply"
        );
        let list = handle(
            dir.path(),
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .ok_or_else(|| anyhow::anyhow!("no list"))?;
        let n = list["result"]["tools"].as_array().map_or(0, Vec::len);
        assert_eq!(
            n,
            tools().as_array().map_or(0, Vec::len),
            "every tool listed"
        );
        Ok(())
    }

    /// A night folder holding plan `t` with `n` shots.
    pub(crate) fn night(n: usize) -> anyhow::Result<tempfile::TempDir> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir_all(dir.path().join("plans"))?;
        crate::gui::assistant::ops::tests::plan(n).save(&dir.path().join("plans/t.toml"))?;
        Ok(dir)
    }

    fn first_text(content: &[serde_json::Value]) -> String {
        content
            .first()
            .and_then(|c| c["text"].as_str())
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn commits_need_permission_and_a_fresh_base() -> anyhow::Result<()> {
        use super::{Caller, call};
        let dir = night(3)?;
        let work = dir.path();
        let ops = json!([{"op": "set_note", "index": 2, "note": "better"}]);
        let staged = call(
            work,
            "plan_edit",
            &json!({"name": "t", "ops": ops, "reason": "note"}),
            Caller::Agent,
        )?;
        let staged = first_text(&staged);
        let base = staged
            .split("base_version: ")
            .nth(1)
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();
        assert_eq!(base.len(), 16, "staged edits report the version: {staged}");
        let got = first_text(&call(
            work,
            "plan_get",
            &json!({"name": "t"}),
            Caller::Agent,
        )?);
        assert!(
            got.contains(&base),
            "plan_get reports the same version: {got}"
        );

        let commit = json!({"name": "t", "ops": ops, "reason": "note", "commit": true, "base_version": base});
        let refused = call(work, "plan_edit", &commit, Caller::Agent);
        assert!(
            refused
                .as_ref()
                .is_err_and(|e| e.to_string().contains("COMMIT_REFUSED")),
            "the agent may not commit by default: {refused:?}"
        );
        let stale = json!({"name": "t", "ops": ops, "reason": "note", "commit": true, "base_version": "0000000000000000"});
        let refused = call(work, "plan_edit", &stale, Caller::Person);
        assert!(
            refused
                .as_ref()
                .is_err_and(|e| e.to_string().contains("STALE")),
            "a stale base is refused: {refused:?}"
        );

        std::fs::write(
            work.join("ave.toml"),
            "[pages.assistant]\nallow_commit = true\n",
        )?;
        let done = first_text(&call(work, "plan_edit", &commit, Caller::Agent)?);
        assert!(
            done.contains("committed as v2"),
            "allowed commit saves: {done}"
        );
        let again = call(work, "plan_edit", &commit, Caller::Agent);
        assert!(
            again.is_err(),
            "the same base cannot commit twice: the plan moved"
        );
        let ops_typed: Vec<crate::gui::assistant::ops::EditOp> =
            serde_json::from_value(json!([{"op": "set_note", "index": 1, "note": "x"}]))?;
        let base = super::plan_version(&work.join("plans/t.toml"))?;
        let v = super::commit_edit(work, "t", &ops_typed, "from a card", &base)?;
        assert_eq!(v, 3, "the card's Apply takes the same path");
        Ok(())
    }

    #[test]
    fn plan_fix_lists_each_finding_with_its_fixes_as_ops() -> anyhow::Result<()> {
        use super::{Caller, call};
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("2026-11-13").join("_ave");
        std::fs::create_dir_all(work.join("plans"))?;
        let mut index = crate::fix::tests::index();
        crate::fix::tests::paint(&mut index, "C", 0.3, [0.0, 0.05, 0.95]);
        index.save(&work)?;
        crate::fix::tests::map().save(&work)?;
        let mut plan = crate::fix::tests::plan();
        plan.name = "reel-x".into();
        plan.save(&work.join("plans").join("reel-x.toml"))?;
        let got = first_text(&call(
            &work,
            "plan_fix",
            &json!({"name": "reel-x"}),
            Caller::Agent,
        )?);
        assert!(
            got.contains("cut 2|3 [soft colour-jump]")
                && got.contains("what: The mid-tone colour jumps")
                && got.contains("fix 1: Swap shot 3 C/2 → ")
                && got.contains("ops: [{\"op\":\"replace_shot\",\"index\":3,"),
            "finding, advice, fix and ops: {got}"
        );
        let ops_line = got
            .lines()
            .find_map(|l| l.trim().strip_prefix("ops: "))
            .ok_or_else(|| anyhow::anyhow!("no ops line: {got}"))?;
        let ops: serde_json::Value = serde_json::from_str(ops_line)?;
        let staged = first_text(&call(
            &work,
            "plan_edit",
            &json!({"name": "reel-x", "ops": ops, "reason": "fix the colour jump"}),
            Caller::Agent,
        )?);
        assert!(
            staged.contains("0 problems after") && staged.contains("1 fixed"),
            "the ops stage cleanly with plan_edit: {staged}"
        );
        Ok(())
    }

    #[test]
    fn markers_are_read_by_plan_get_and_edited_by_plan_edit() -> anyhow::Result<()> {
        use super::{Caller, call};
        let dir = night(2)?;
        let work = dir.path();
        std::fs::write(
            work.join("ave.toml"),
            "[pages.assistant]\nallow_commit = true\n",
        )?;
        let get = || -> anyhow::Result<String> {
            Ok(first_text(&call(
                work,
                "plan_get",
                &json!({"name": "t"}),
                Caller::Agent,
            )?))
        };
        let before = get()?;
        assert!(!before.contains("markers"), "no markers yet: {before}");
        let base = super::plan_version(&work.join("plans/t.toml"))?;
        let add = json!({"name": "t", "reason": "review", "commit": true, "base_version": base, "ops": [
            {"op": "add_marker", "time": 1.5, "note": "hold the hoop longer", "color": "yellow"},
            {"op": "add_marker", "time": 3.0}
        ]});
        let done = first_text(&call(work, "plan_edit", &add, Caller::Agent)?);
        assert!(done.contains("committed"), "markers commit: {done}");
        let got = get()?;
        assert!(
            got.contains("markers (2):")
                && got.contains("m1 1.50s yellow · assistant · hold the hoop longer")
                && got.contains("m2 3.00s green · assistant · (no note)"),
            "plan_get lists numbered markers: {got}"
        );
        let saved = crate::plan::Plan::load(&work.join("plans/t.toml"))?;
        assert_eq!(saved.comments.len(), 2, "saved in the plan file");

        let base = super::plan_version(&work.join("plans/t.toml"))?;
        let edit = json!({"name": "t", "reason": "done", "commit": true, "base_version": base, "ops": [
            {"op": "resolve_marker", "marker": 1},
            {"op": "remove_marker", "marker": 2}
        ]});
        call(work, "plan_edit", &edit, Caller::Agent)?;
        let got = get()?;
        assert!(
            got.contains("markers (1):") && got.contains("m1 1.50s yellow · assistant · resolved"),
            "resolved kept, the other removed: {got}"
        );

        let bad = json!({"name": "t", "reason": "x", "ops": [{"op": "add_marker", "time": 1.0, "color": "purple"}]});
        let refused = call(work, "plan_edit", &bad, Caller::Agent);
        assert!(
            refused
                .as_ref()
                .is_err_and(|e| e.to_string().contains("purple")),
            "an unknown colour is refused: {refused:?}"
        );

        // A revert brings back old shots but keeps today's markers.
        let reverted = first_text(&call(
            work,
            "plan_revert",
            &json!({"name": "t", "version": 1}),
            Caller::Person,
        )?);
        assert!(reverted.contains("restored"), "reverted: {reverted}");
        let after = crate::plan::Plan::load(&work.join("plans/t.toml"))?;
        assert_eq!(after.comments.len(), 1, "markers survive the revert");
        Ok(())
    }

    #[test]
    fn api_tools_mirror_the_mcp_tools() -> anyhow::Result<()> {
        let api = super::api_tools();
        let names = |v: &serde_json::Value| -> Vec<String> {
            v.as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t["name"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        assert_eq!(names(&api), names(&tools()), "same tools, same order");
        assert!(
            api.as_array()
                .is_some_and(|a| a.iter().all(|t| t["input_schema"]["type"] == "object")),
            "Messages API shape"
        );
        Ok(())
    }

    #[test]
    fn run_step_asks_and_runs_nothing() -> anyhow::Result<()> {
        use super::{Caller, call};
        let dir = night(2)?;
        let work = dir.path();
        let asked = first_text(&call(
            work,
            super::RUN_STEP,
            &json!({"step": "render", "plan": "t"}),
            Caller::Agent,
        )?);
        assert!(
            asked.starts_with(super::APPROVAL_REQUESTED) && asked.contains("render t"),
            "the request goes to the person: {asked}"
        );
        assert!(!work.join("renders").exists(), "nothing was rendered");
        for bad in [
            json!({"step": "render", "plan": "nope"}),
            json!({"step": "caption"}),
            json!({"step": "index"}),
            json!({"step": "dance"}),
        ] {
            assert!(
                call(work, super::RUN_STEP, &bad, Caller::Agent).is_err(),
                "refused up front: {bad}"
            );
        }
        Ok(())
    }

    /// Writes an `index.json` of segments `(id, caption summary)` into `work`.
    fn write_index(work: &std::path::Path, segs: &[(&str, Option<&str>)]) -> anyhow::Result<()> {
        let mut segments = Vec::new();
        for (k, (id, summary)) in segs.iter().enumerate() {
            let mut s = crate::index::Scored {
                id: (*id).to_owned(),
                clip: id.split('/').next().unwrap_or(id).to_owned(),
                q: 0.5,
                caption: summary.map(|t| crate::caption::Caption {
                    summary: t.to_owned(),
                    ..crate::caption::Caption::default()
                }),
                ..crate::index::Scored::default()
            };
            s.seg.start = k as f32 * 4.0;
            s.seg.end = s.seg.start + 3.0;
            s.seg.peak_t = s.seg.start + 2.0;
            segments.push(serde_json::to_value(s)?);
        }
        let index = json!({"source": "", "sharp_ref": 1.0, "clips": [], "segments": segments});
        std::fs::write(work.join("index.json"), index.to_string())?;
        Ok(())
    }

    #[test]
    fn search_says_when_its_results_are_partial() -> anyhow::Result<()> {
        use super::{Caller, call};
        let dir = tempfile::tempdir()?;
        let work = dir.path();
        write_index(
            work,
            &[
                ("A/1", Some("Fire spinner by the stage.")),
                ("A/2", None),
                ("B/1", None),
            ],
        )?;
        let partial = first_text(&call(
            work,
            "search_shots",
            &json!({"query": "spinner"}),
            Caller::Agent,
        )?);
        assert!(
            partial.contains("PARTIAL") && partial.contains("uncaptioned_in_scope=2"),
            "a caption query names what it could not see: {partial}"
        );
        assert!(
            partial.contains("A/1 0.0-3.0s focal 2.0s"),
            "each hit carries its focal time: {partial}"
        );
        let whole = first_text(&call(
            work,
            "search_shots",
            &json!({"query": "clip:B"}),
            Caller::Agent,
        )?);
        assert!(
            whole.contains("complete") && !whole.contains("PARTIAL"),
            "a measured facet sees everything: {whole}"
        );
        Ok(())
    }

    #[test]
    fn the_overview_lists_the_nights_sets() -> anyhow::Result<()> {
        use crate::analyze::sets::tests::{clip, night};
        let index = night(vec![
            clip("A", 0, 10.0, "outdoor parking lot at night", "fire"),
            clip("B", 3600, 10.0, "indoor venue", "led"),
        ]);
        let text = super::sets_section(&index);
        assert!(
            text.starts_with("sets (2,")
                && text.contains("Set 1: Outdoor parking lot · fire · 19:00-19:00 · 1 clips")
                && text.contains("Set 2: Indoor venue · LED")
                && text.contains("(60 min gap) — clips B"),
            "{text}"
        );
        let old = crate::index::Index::default();
        assert!(
            super::sets_section(&old).contains("no recording times"),
            "an index without times says so"
        );
        Ok(())
    }

    #[test]
    fn tool_errors_are_results_not_protocol_errors() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let r = handle(dir.path(), &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "search_shots", "arguments": {"query": "fire"}}}))
            .ok_or_else(|| anyhow::anyhow!("no reply"))?;
        assert_eq!(
            r["result"]["isError"], true,
            "an unindexed night is a tool error: {r}"
        );
        let overview = handle(dir.path(), &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "night_overview", "arguments": {}}}))
            .ok_or_else(|| anyhow::anyhow!("no reply"))?;
        assert!(
            overview["result"]["content"][0]["text"]
                .as_str()
                .is_some_and(|t| t.contains("not indexed")),
            "overview works on an empty night"
        );
        Ok(())
    }
}

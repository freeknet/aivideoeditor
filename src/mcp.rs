// ============================================================================
// Module:       mcp
// Description:  `ave mcp`: the engine as Model Context Protocol tools over
//               stdio, so an agent (Claude Code) can work inside a night.
//
// Dependencies: serde_json, image (JPEG contact sheets); crate::{project,
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
            "instructions": INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = msg["params"]["name"].as_str().unwrap_or_default();
            let args = msg["params"]["arguments"].clone();
            Ok(match call(work, name, &args) {
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
const INSTRUCTIONS: &str = "ave edits a night of event footage (fire spinning, LED props, silks, DJs) into \
beat-synced promo cuts. Start with night_overview. Find footage with search_shots (free text plus facets such as \
fire>0.3 -dark person:x move:zoom-in pick) and read exact facts with segment_details. Use look to see frames when a \
decision depends on the picture. Plans are storyboards on the music's beat grid: plan_get shows one with its rule \
check; plan_edit applies ops to a working copy and returns the diff and check without saving until commit:true; \
alternatives ranks replacements for a slot. Rules: no repeated shots or performers, best moments only, the drop on \
the hero fire shot, no slow motion or digital zoom. You own the editorial decisions; say why for each shot in its \
note. Every commit is a new version (plan_versions, plan_revert).";

/// The tool definitions.
fn tools() -> Value {
    let ops_schema = json!({
        "type": "array",
        "description": "Edits, applied in order; slots are 1-based and each op sees the plan as earlier ops left it.",
        "items": {"type": "object", "required": ["op"], "properties": {
            "op": {"type": "string", "enum": ["replace_shot", "set_in", "set_beats", "move_shot", "remove_shot", "set_cx", "set_note"]},
            "index": {"type": "integer"}, "clip": {"type": "string"}, "in": {"type": "number"},
            "cx": {"type": "number"}, "beats": {"type": "integer"}, "from": {"type": "integer"},
            "to": {"type": "integer"}, "note": {"type": "string"}, "reason": {"type": "string"}
        }}
    });
    json!([
        {"name": "night_overview", "description": "The open night: clips, segments and flags, captions, music grid and drops, people, plans with their rule-check status.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "search_shots", "description": "Search the shot log. Free text matches captions/people/settings/clip ids; facets: fire>0.3 led>0.2 move>0.5 focus<0.8 q>0.7 dark soft flag:blown move:pan|zoom-in|zoom-out|static person:<name> clip:MVI_79 moment:performing framing:close pick reject like:<segment id>; '-' negates. Shares are relative to the night (1 = its strongest).",
         "inputSchema": {"type": "object", "required": ["query"], "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "default": 25}}}},
        {"name": "segment_details", "description": "Exact facts for segments (ids like MVI_7987/3): times, camera, exposure, focus, fire/LED/moving light, action centre, quality, flags with thresholds, burst moments, caption, who, plans using it.",
         "inputSchema": {"type": "object", "required": ["ids"], "properties": {"ids": {"type": "array", "items": {"type": "string"}}}}},
        {"name": "look", "description": "See frames: a contact sheet (JPEG) of up to 12 moments, left to right, top to bottom. Each item is a segment id (its mid time, or 'at') or a clip id with 'at' seconds. Use it when a choice depends on the picture.",
         "inputSchema": {"type": "object", "required": ["items"], "properties": {
            "items": {"type": "array", "items": {"type": "object", "properties": {"segment": {"type": "string"}, "clip": {"type": "string"}, "at": {"type": "number"}}}},
            "width": {"type": "integer", "default": 320}}}},
        {"name": "music_map", "description": "The track: bpm, bars with energy (low end, rms), sections, drop candidates (strongest first), song ranges.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "plan_get", "description": "A plan: bars, aspect, every shot with its time on the cut, beats, clip, in-point, crop, who and note, plus the rule-check problems.",
         "inputSchema": {"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}}},
        {"name": "plan_edit", "description": "Apply edit ops to a plan's working copy. Returns the diff and the rule check (new problems, problems fixed). Saves a new version only with commit:true.",
         "inputSchema": {"type": "object", "required": ["name", "ops", "reason"], "properties": {
            "name": {"type": "string"}, "ops": ops_schema, "reason": {"type": "string"}, "commit": {"type": "boolean", "default": false}}}},
        {"name": "plan_draft", "description": "Draft a plan from the shot log and the music (hook, breakdown, build, hero on the drop, phrase pacing, finale). Returns it with its check; saves only with commit:true.",
         "inputSchema": {"type": "object", "required": ["name"], "properties": {
            "name": {"type": "string"}, "aspect": {"type": "string", "enum": ["9x16", "4x5", "16x9"], "default": "9x16"},
            "seconds": {"type": "number", "default": 30}, "end_card": {"type": "string"}, "commit": {"type": "boolean", "default": false}}}},
        {"name": "alternatives", "description": "Ranked footage that could replace shot `index` (1-based) of a plan without breaking the rules, each with why it fits.",
         "inputSchema": {"type": "object", "required": ["plan", "index"], "properties": {"plan": {"type": "string"}, "index": {"type": "integer"}, "limit": {"type": "integer", "default": 8}}}},
        {"name": "plan_versions", "description": "A plan's saved versions, oldest first, with what changed.",
         "inputSchema": {"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}}},
        {"name": "plan_revert", "description": "Restore a saved version (saved as a new version, nothing is lost).",
         "inputSchema": {"type": "object", "required": ["name", "version"], "properties": {"name": {"type": "string"}, "version": {"type": "integer"}}}},
        {"name": "people_map", "description": "Performers: name → clips/ranges, as the no-repeated-performer rule sees them.",
         "inputSchema": {"type": "object", "properties": {}}}
    ])
}

/// A text content block.
fn text(s: impl Into<String>) -> Vec<Value> {
    vec![json!({"type": "text", "text": s.into()})]
}

/// Runs one tool.
fn call(work: &Path, name: &str, args: &Value) -> Result<Vec<Value>> {
    let project = Project::open(work)?;
    match name {
        "night_overview" => Ok(text(overview(&project))),
        "search_shots" => search(&project, args),
        "segment_details" => details(&project, args),
        "look" => look(&project, args),
        "music_map" => music(&project),
        "plan_get" => plan_get(&project, args),
        "plan_edit" => plan_edit(&project, args),
        "plan_draft" => plan_draft(&project, args),
        "alternatives" => alternatives(&project, args),
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
            let v = store::versions(&project.work, name)
                .into_iter()
                .find(|v| v.n == n)
                .with_context(|| format!("{name} has no v{n}"))?;
            let plan = Plan::load(&v.path)?;
            let saved =
                store::save_version(&project.work, name, &plan, &format!("revert to v{n}"))?;
            plan.save(&plan_path(&project, name))?;
            Ok(text(format!(
                "{name} restored from v{n} (saved as v{saved})"
            )))
        }
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
                    if problems.is_empty() {
                        "check ok".to_owned()
                    } else {
                        format!("{} problems", problems.len())
                    }
                )
            }
            (Err(e), _, _) => format!("unreadable: {e:#}"),
            _ => "not checkable yet".to_owned(),
        };
        s.push_str(&format!("plan {stem}: {status}\n"));
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
    let hits = crate::search::search(&query, &night);
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
                "{} {:.1}-{:.1}s score {:.2}: {}{cap}",
                s.id, s.seg.start, s.seg.end, h.score, h.reason
            ))
        })
        .collect();
    Ok(text(format!(
        "{} matches{}\n{}",
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
            format!(
                "\n  camera {:?} · exposure {:.2}/{:.2} · focus {:.2} · fire {:.3} · led {:.3} · moving {:.3} · cx {:.2}±{:.2} · q {:.2} · flags {:?} · who {who}{caption}",
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
    match ops::check(plan, p.index.as_ref(), p.music.as_ref(), &p.people) {
        Some(problems) if problems.is_empty() => s.push_str("check: ok\n"),
        Some(problems) => {
            s.push_str(&format!("check: {} problems\n", problems.len()));
            for pr in problems {
                s.push_str(&format!("  shot {}: {}\n", pr.shot, pr.message));
            }
        }
        None => s.push_str("check: needs the index and the music map\n"),
    }
    s
}

/// `plan_get`.
fn plan_get(p: &Project, args: &Value) -> Result<Vec<Value>> {
    let name = str_arg(args, "name")?;
    let plan = Plan::load(&plan_path(p, name))?;
    Ok(text(describe_plan(p, &plan)))
}

/// `plan_edit`: working copy, diff and check; saves only with commit.
fn plan_edit(p: &Project, args: &Value) -> Result<Vec<Value>> {
    let name = str_arg(args, "name")?;
    let reason = args["reason"].as_str().unwrap_or("edit");
    let edit_ops: Vec<ops::EditOp> =
        serde_json::from_value(args["ops"].clone()).context("`ops` does not match the schema")?;
    let base = Plan::load(&plan_path(p, name))?;
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
        "check: {} problems after ({} new, {fixed} fixed)\n",
        after.len(),
        new.len()
    ));
    for pr in &new {
        s.push_str(&format!("  new: shot {}: {}\n", pr.shot, pr.message));
    }
    if args["commit"].as_bool().unwrap_or(false) {
        if store::versions(&p.work, name).is_empty() {
            store::save_version(&p.work, name, &base, "original")?;
        }
        let v = store::save_version(&p.work, name, &edited, reason)?;
        edited.save(&plan_path(p, name))?;
        s.push_str(&format!("committed as v{v}\n"));
    } else {
        s.push_str("not saved (working copy); call again with commit:true to save\n");
    }
    Ok(text(s))
}

/// `plan_draft`.
fn plan_draft(p: &Project, args: &Value) -> Result<Vec<Value>> {
    let index = p.index.as_ref().context("the night is not indexed yet")?;
    let music = p.music.as_ref().context("the music is not mapped yet")?;
    let name = str_arg(args, "name")?;
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
    };
    let plan = crate::draft::draft(index, music, &p.people, &spec, &music.path)?;
    let mut s = describe_plan(p, &plan);
    if args["commit"].as_bool().unwrap_or(false) {
        let path = plan_path(p, name);
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
mod tests {
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

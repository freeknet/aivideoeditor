// ============================================================================
// Module:       caption
// Description:  Captions segments with a local vision-language model (Qwen3-VL
//               on llama.cpp's OpenAI-compatible server), schema-constrained.
//
// Dependencies: crate::index, crate::media, crate::commands (progress), ureq,
//               base64, serde_json
// ============================================================================

//! Vision captions.
//!
//! Two stills per segment (at 30 % and 70 % of it) go to the server in one
//! request, with a JSON schema the server enforces. The answer becomes the
//! segment's [`Caption`], printed under it in the shot log.
//!
//! The server is `AVE_VLM_URL` (default `http://127.0.0.1:8089`). If it is
//! not answering and `AVE_LLAMA_DIR` and `AVE_VLM_MODEL` / `AVE_VLM_MMPROJ`
//! are set, `ave caption` starts it and stops it when done.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use rayon::prelude::*;

use crate::commands::{ItemState, Progress, Report};
use crate::index::{Index, Scored};
use crate::media;

/// What the vision model saw in a segment.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Caption {
    /// One-sentence description.
    pub(crate) summary: String,
    /// The people in frame and what they hold.
    #[serde(default)]
    pub(crate) people: Vec<Person>,
    /// `performing`, `transition`, `idle`, `crowd` or `venue`.
    pub(crate) moment: String,
    /// A prop is lit and moving in both frames.
    #[serde(default)]
    pub(crate) props_active: bool,
    /// Something stands between the camera and the subject.
    #[serde(default)]
    pub(crate) blocked: bool,
    /// `close`, `medium` or `wide`.
    pub(crate) framing: String,
    /// Where it is.
    pub(crate) setting: String,
    /// Problems the model noticed (blur, obstruction, darkness...).
    #[serde(default)]
    pub(crate) issues: Vec<String>,
}

/// One person in a caption.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Person {
    /// Clothing, hair, build: enough to recognise them in another clip.
    pub(crate) appearance: String,
    /// Prop, or `none`.
    pub(crate) prop: String,
}

impl Caption {
    /// The moment, corrected where the summary contradicts it: small vision
    /// models answer `performing` for an empty chandelier shot while the
    /// summary says "no performers visible".
    pub(crate) fn effective_moment(&self) -> &str {
        let s = self.summary.to_lowercase();
        let nobody = [
            "no performer",
            "no people",
            "no person",
            "no one ",
            "nobody",
            "empty",
        ]
        .iter()
        .any(|p| s.contains(p));
        if nobody {
            "venue"
        } else {
            self.moment.as_str()
        }
    }

    /// The caption on one line, for the shot log.
    pub(crate) fn one_line(&self) -> String {
        let people: Vec<String> = self
            .people
            .iter()
            .map(|p| {
                if p.prop.is_empty() || p.prop == "none" {
                    p.appearance.clone()
                } else {
                    format!("{} [{}]", p.appearance, p.prop)
                }
            })
            .collect();
        let mut s = format!(
            "_{}_ · {} · {} · {}",
            self.summary, self.moment, self.framing, self.setting
        );
        if !people.is_empty() {
            s.push_str(&format!(" · who: {}", people.join("; ")));
        }
        if self.blocked {
            s.push_str(" · BLOCKED view");
        }
        if !self.props_active && self.moment != "venue" && self.moment != "crowd" {
            s.push_str(" · props not active");
        }
        if !self.issues.is_empty() {
            s.push_str(&format!(" · issues: {}", self.issues.join(", ")));
        }
        s
    }
}

/// The schema the server is asked to enforce.
fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "summary": {"type": "string", "maxLength": 140},
            "people": {"type": "array", "maxItems": 3, "items": {"type": "object", "properties": {
                "appearance": {"type": "string", "maxLength": 90}, "prop": {"type": "string", "maxLength": 30}},
                "required": ["appearance", "prop"]}},
            "moment": {"type": "string", "enum": ["performing", "transition", "idle", "crowd", "venue"]},
            "props_active": {"type": "boolean"},
            "blocked": {"type": "boolean"},
            "framing": {"type": "string", "enum": ["close", "medium", "wide"]},
            "setting": {"type": "string", "maxLength": 60},
            "issues": {"type": "array", "maxItems": 4, "items": {"type": "string", "maxLength": 40}}
        },
        "required": ["summary", "people", "moment", "props_active", "blocked", "framing", "setting", "issues"]
    })
}

/// The instruction sent with every pair of stills.
const PROMPT: &str = "Two frames, about a second apart, from event footage (a night of fire spinning, \nLED props, aerial silks and DJs). Answer tersely. summary: one short sentence. people: at most the 3 most \nprominent people, each with appearance (clothing, colours, hair, build: enough to recognise them in another clip, \nunder 15 words) and prop (fire fans, fire staff, fire poi, fire hoop, LED hoop, LED poi, silks, DJ decks, none). \nmoment: performing = actively doing a trick with the prop in motion; transition = walking in, climbing, \nwrapping, lighting or picking up props, adjusting, fumbling, turning away; idle = standing still or watching; \ncrowd = mainly audience; venue = no person in focus. props_active: is a prop lit AND moving in both frames? \nblocked: is a person, head or object between the camera and the main subject? framing: size of the main \nsubject. setting: where, in a few words. issues: only real problems (out of focus, motion blur, too dark, \nblown out), else empty. Be literal; never guess names.";

/// Why captions cannot run here, or `None` when they look set up: a server
/// of the person's own (`AVE_VLM_URL`), or llama-server and both model
/// files where the `AVE_LLAMA_DIR` / `AVE_VLM_MODEL` / `AVE_VLM_MMPROJ`
/// settings say. No network call, so a screen can ask while it draws; a
/// server on the default port without `AVE_VLM_URL` set still works when
/// the step runs, it is just not promised here.
pub(crate) fn not_set_up() -> Option<String> {
    let set = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
    if set("AVE_VLM_URL").is_some() {
        return None;
    }
    let (Some(dir), Some(model), Some(mmproj)) = (
        set("AVE_LLAMA_DIR"),
        set("AVE_VLM_MODEL"),
        set("AVE_VLM_MMPROJ"),
    ) else {
        return Some("needs the local vision model: see Captions in the README".to_owned());
    };
    let exe = Path::new(&dir).join(format!("llama-server{}", std::env::consts::EXE_SUFFIX));
    [exe, PathBuf::from(model), PathBuf::from(mmproj)]
        .into_iter()
        .find(|p| !p.is_file())
        .map(|missing| format!("{} is missing", missing.display()))
}

/// The server URL.
fn server_url() -> String {
    std::env::var("AVE_VLM_URL").unwrap_or_else(|_| "http://127.0.0.1:8089".to_owned())
}

/// Whether the server answers `/health`.
fn healthy(agent: &ureq::Agent, url: &str) -> bool {
    agent
        .get(&format!("{url}/health"))
        .call()
        .is_ok_and(|r| r.status().is_success())
}

/// A llama-server this process started, killed on drop.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts llama-server from the `AVE_LLAMA_DIR` / `AVE_VLM_MODEL` /
/// `AVE_VLM_MMPROJ` settings, or explains what is missing.
fn start_server(agent: &ureq::Agent, url: &str) -> Result<Server> {
    let need = |k: &str| {
        std::env::var(k).with_context(|| {
            format!("the vision server at {url} is not answering and {k} is not set to start one")
        })
    };
    let dir = PathBuf::from(need("AVE_LLAMA_DIR")?);
    let model = need("AVE_VLM_MODEL")?;
    let mmproj = need("AVE_VLM_MMPROJ")?;
    let port = url
        .rsplit(':')
        .next()
        .unwrap_or("8089")
        .trim_end_matches('/')
        .to_owned();
    let exe = dir.join(format!("llama-server{}", std::env::consts::EXE_SUFFIX));
    log::info!("starting {} on port {port}", exe.display());
    let mut cmd = Command::new(&exe);
    crate::media::quiet(&mut cmd);
    let child = cmd
        .args([
            "-m", &model, "--mmproj", &mmproj, "-ngl", "99", "-c", "16384", "-np", "4",
        ])
        .args(["--host", "127.0.0.1", "--port", &port])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("starting {}", exe.display()))?;
    let server = Server(child);
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        if healthy(agent, url) {
            return Ok(server);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("llama-server did not become healthy within 2 minutes")
}

/// Captions one segment.
fn caption_one(
    agent: &ureq::Agent,
    url: &str,
    index: &Index,
    seg: &Scored,
    tmp: &Path,
) -> Result<Caption> {
    let clip = index
        .clip(&seg.clip)
        .with_context(|| format!("segment {} has no clip", seg.id))?;
    let g = &seg.seg;
    let mut content = Vec::new();
    for (k, frac) in [0.3_f32, 0.7].iter().enumerate() {
        let t = f64::from(g.start + (g.end - g.start) * frac);
        let file = tmp.join(format!("{}_{k}.jpg", seg.id.replace('/', "_")));
        media::still(Path::new(&clip.path), t, 576, None, &file)?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&file)?);
        let _ = std::fs::remove_file(&file);
        content.push(serde_json::json!({"type": "image_url",
            "image_url": {"url": format!("data:image/jpeg;base64,{b64}")}}));
    }
    content.push(serde_json::json!({"type": "text", "text": PROMPT}));
    let body = serde_json::json!({
        "temperature": 0.1,
        "max_tokens": 360,
        "messages": [{"role": "user", "content": content}],
        "response_format": {"type": "json_schema",
            "json_schema": {"name": "caption", "strict": true, "schema": schema()}}
    });
    let mut response = agent
        .post(&format!("{url}/v1/chat/completions"))
        .send_json(&body)
        .with_context(|| format!("captioning {}", seg.id))?;
    let reply: serde_json::Value = response.body_mut().read_json()?;
    let text = reply["choices"][0]["message"]["content"]
        .as_str()
        .context("the server's reply has no message content")?;
    serde_json::from_str(text).with_context(|| {
        format!(
            "the caption for {} is not the expected JSON: {text}",
            seg.id
        )
    })
}

/// Captions every segment of `work`'s index that passes `min_q` and has no
/// caption yet (or every one, with `redo`). Saves as it goes, reports every
/// segment to `progress` (`caption 120/417 MVI_7925/3: done 6.8s eta 2210s`)
/// and stops after the batch in flight when it is cancelled.
pub(crate) fn run(
    work: &Path,
    min_q: f32,
    redo: bool,
    parallel: usize,
    progress: &Progress,
) -> Result<usize> {
    let mut index = Index::load(work)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(180)))
        .build()
        .into();
    let url = server_url();
    let _server = if healthy(&agent, &url) {
        None
    } else {
        Some(start_server(&agent, &url)?)
    };
    let tmp = work.join("tmp");
    std::fs::create_dir_all(&tmp)?;

    let todo: Vec<usize> = index
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            (redo || s.caption.is_none()) && s.q >= min_q && s.seg.end - s.seg.start >= 1.0
        })
        .map(|(i, _)| i)
        .collect();
    log::info!("captioning {} segments ({parallel} at a time)", todo.len());
    progress.stage("caption", format!("{} segments to caption", todo.len()));
    let started = Instant::now();
    let total = todo.len();
    let done = Mutex::new(0_usize);
    let snapshot = index.clone();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(parallel.max(1))
        .build()?;
    let mut ok = 0;
    // Batches, saving after each, so a long run shows progress in the shot
    // log and an interrupted (or cancelled) one keeps what it finished.
    for batch in todo.chunks(24) {
        let results: Vec<(usize, Result<Caption>)> = pool.install(|| {
            batch
                .par_iter()
                .map(|&i| {
                    if let Err(e) = progress.check() {
                        return (i, Err(e));
                    }
                    let seg = &snapshot.segments[i];
                    let n = done.lock().map_or(0, |n| *n);
                    progress.report(&Report::item(
                        "caption",
                        n,
                        total,
                        seg.id.as_str(),
                        ItemState::Started,
                    ));
                    let began = Instant::now();
                    let r = caption_one(&agent, &url, &snapshot, seg, &tmp);
                    let n = done.lock().map_or(0, |mut n| {
                        *n += 1;
                        *n
                    });
                    let elapsed = started.elapsed().as_secs_f32();
                    let report = match &r {
                        Ok(_) => {
                            Report::item("caption", n, total, seg.id.as_str(), ItemState::Done)
                                .took(began.elapsed().as_secs_f32())
                                .remaining(crate::commands::eta(elapsed, n, total))
                        }
                        Err(e) => {
                            Report::item("caption", n, total, seg.id.as_str(), ItemState::Failed)
                                .because(&format!("{e:#}"))
                        }
                    };
                    progress.report(&report);
                    (i, r)
                })
                .collect()
        });
        for (i, r) in results {
            match r {
                Ok(c) => {
                    index.segments[i].caption = Some(c);
                    ok += 1;
                }
                Err(e) if !progress.cancelled() => {
                    log::warn!("{}: {e:#}", index.segments[i].id);
                }
                Err(_) => {}
            }
        }
        index.save(work)?;
        progress.check()?;
    }
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::{Caption, Person, schema};

    #[test]
    fn a_caption_prints_on_one_line() -> anyhow::Result<()> {
        let c = Caption {
            summary: "a woman spins fire fans".to_owned(),
            people: vec![Person {
                appearance: "black sleeveless top".to_owned(),
                prop: "fire fans".to_owned(),
            }],
            moment: "performing".to_owned(),
            props_active: true,
            blocked: false,
            framing: "medium".to_owned(),
            setting: "parking lot".to_owned(),
            issues: Vec::new(),
        };
        let line = c.one_line();
        assert!(!line.contains('\n'), "one line: {line}");
        assert!(line.contains("[fire fans]"), "prop shown: {line}");
        assert!(
            schema()["required"]
                .as_array()
                .is_some_and(|r| r.len() == 8),
            "every field required"
        );
        Ok(())
    }
}

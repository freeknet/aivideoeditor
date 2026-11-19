// ============================================================================
// Module:       gui::assistant::context
// Description:  What the model is told: the standing system prompt, the
//               per-message snapshot of the plan, music, check and
//               references, and a compact searchable copy of the shot log.
//
// Dependencies: crate::{plan, index, people, beats}
// ============================================================================

//! Context for the model.
//!
//! [`SYSTEM`] never changes, so it is cached once per conversation. Every
//! user message carries a [`snapshot`] of the night as it is now: the plan
//! (one line per shot, with its note), the music window, the rule check and
//! the references the person attached, each resolved to facts. The shot log
//! itself is too big to send whole; the model pulls lines from it with the
//! `search_shots` and `get_segment` tools, served by [`ShotLog`].

use std::fmt::Write as _;

use crate::beats::MusicMap;
use crate::index::Index;
use crate::people::People;
use crate::plan::{Plan, Problem};

/// The standing instructions.
pub(crate) const SYSTEM: &str = "You are the editing assistant inside ave, a desktop video \
editor for a weekly night of dark, dialogue-free event footage (fire spinning, LED props, aerial \
silks, DJs). Footage is indexed into a shot log: one line per segment with measured numbers \
(exposure, focus relative to the shoot, fire and LED share of the frame, moving light, action \
centre, quality q 0-1, flags) and a vision caption. Plans are beat-synced storyboards: a music \
window in bars and one shot per slot with a clip, an in-point, a length in beats, a crop centre \
cx and a note saying why the shot is there.

You never see the footage. Judge only from the shot log and the numbers; do not claim to have \
watched anything.

How to work:
- Read the plan and check in the <context> of the latest message. Use search_shots and \
get_segment to find footage; quote the shot-log facts you rely on.
- Change the plan only through propose_edit. Nothing is applied until the person accepts, so \
never say a change is done. Keep proposals local: touch only the shots the request is about and \
stay inside the stated scope. Never regenerate the whole plan for a local request.
- Every replace_shot carries a reason that will become the shot's note: what the shot is and \
why it beats the old one, with numbers (\"fire 9% · q 0.91 · performing close\").
- The music is the clock: slot lengths come from the bars. Keep the beat total; balance any \
set_beats with another.
- After proposing, say in one or two sentences what you changed and what the predicted check \
says. If the check gets worse, say so plainly.
- If a request is ambiguous (\"make it punchier\", \"fix the start\") and the references do not \
settle it, call ask_user with one short question and two or three concrete options instead of \
guessing. Ask at most one question per turn.
- Answer questions about the footage or the plan directly, without proposing.

The edit rules (the check enforces them; respect them in every proposal):
- No repeated shots: two shots from one clip must start at least same_clip_gap seconds apart.
- No repeated performers: the same person may not appear again within person_gap shots.
- Best moments only: avoid segments flagged dark, soft, whip, shaky or blown, captions whose \
moment is transition or idle, and blocked views.
- The drop lands on the hero shot: the shot starting exactly on the drop bar must be the \
strongest fire shot available (subject fire, high q, performing, close or medium).
- Prefer action centred in the crop: for vertical cuts the crop is a third of the frame, so \
segments whose action drifts widely read badly.

References: the person points at things with tokens like @shot:reel-30#5 (slot 5), \
@seg:MVI_7925/3 (a shot-log segment), @clip:MVI_7987@24.0-27.5 (a range), @bars:17-20, \
@person:name, @plan:name. A prefix states intent: + keep it unchanged, ! never use it, > replace \
it, ~ find more like it, * feature it (longer, on a strong beat or the drop). Treat ! as a hard \
rule for the request and + as untouchable.

Style: short. Plain sentences, **bold** for shot numbers and clip ids that matter, bullets for \
lists. No headings, no tables.";

/// One shot-log segment, flattened for search and quoting.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    /// Segment id, `MVI_7925/3`.
    pub(crate) id: String,
    /// Clip id.
    pub(crate) clip: String,
    /// Start, seconds.
    pub(crate) start: f32,
    /// End, seconds.
    pub(crate) end: f32,
    /// `fire`, `led`, `light`, `none`.
    pub(crate) subject: String,
    /// Quality 0..1.
    pub(crate) q: f32,
    /// Flags.
    pub(crate) flags: Vec<String>,
    /// Camera behaviour.
    pub(crate) camera: String,
    /// Fire share of the frame.
    pub(crate) fire: f32,
    /// LED share of the frame.
    pub(crate) led: f32,
    /// Action centre and drift.
    pub(crate) cx: (f32, f32),
    /// Caption on one line, if captioned.
    pub(crate) caption: String,
    /// The caption's moment (corrected).
    pub(crate) moment: String,
    /// Who, from the people map.
    pub(crate) who: String,
}

impl Row {
    /// The row as one shot-log line.
    pub(crate) fn line(&self) -> String {
        let flags = if self.flags.is_empty() {
            String::new()
        } else {
            format!(" · flags {}", self.flags.join(" "))
        };
        let caption = if self.caption.is_empty() {
            String::new()
        } else {
            format!(" · {}", self.caption)
        };
        format!(
            "{} {:.1}-{:.1} ({:.1}s) · {} · {} · fire {:.1}% led {:.1}% · cx {:.2}±{:.2} · q {:.2} · who {}{flags}{caption}",
            self.id,
            self.start,
            self.end,
            self.end - self.start,
            self.camera,
            self.subject,
            self.fire * 100.0,
            self.led * 100.0,
            self.cx.0,
            self.cx.1 / 2.0,
            self.q,
            self.who,
        )
    }
}

/// A search filter.
type Test = Box<dyn Fn(&Row) -> bool>;

/// A compact copy of the shot log the worker thread can own.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ShotLog {
    /// Every segment.
    pub(crate) rows: Vec<Row>,
}

impl ShotLog {
    /// Builds it from the index and the people map.
    pub(crate) fn new(index: &Index, people: &People) -> Self {
        let rows = index
            .segments
            .iter()
            .map(|s| Row {
                id: s.id.clone(),
                clip: s.clip.clone(),
                start: s.seg.start,
                end: s.seg.end,
                subject: s.subject.clone(),
                q: s.q,
                flags: s.flags.clone(),
                camera: s.seg.camera.label().to_owned(),
                fire: s.seg.flame,
                led: s.seg.led,
                cx: (s.seg.cx, s.seg.cx_spread),
                caption: s
                    .caption
                    .as_ref()
                    .map(|c| c.one_line().replace('_', ""))
                    .unwrap_or_default(),
                moment: s
                    .caption
                    .as_ref()
                    .map(|c| c.effective_moment().to_owned())
                    .unwrap_or_default(),
                who: people.name_at(&s.clip, s.seg.start),
            })
            .collect();
        Self { rows }
    }

    /// The row with id `id`.
    pub(crate) fn get(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }

    /// Segments matching `query`, best first.
    ///
    /// Words match the id, subject, caption and who. Facets: `fire>0.05`,
    /// `led>0.02`, `q>0.7`, `len>2` (seconds), `subject:fire`,
    /// `moment:performing`, `who:name` / `person:name`, `clip:MVI_79`, and
    /// `-dark` / `!soft` to exclude a flag (`clean` excludes every bad flag).
    pub(crate) fn search(&self, query: &str, limit: usize, plan: Option<&Plan>) -> Vec<&Row> {
        let mut words = Vec::new();
        let mut tests: Vec<Test> = Vec::new();
        for token in query.split_whitespace() {
            let t = token.to_lowercase();
            if let Some((k, v)) = t.split_once('>')
                && let Ok(v) = v.parse::<f32>()
            {
                let key = k.to_owned();
                tests.push(Box::new(move |r: &Row| match key.as_str() {
                    "fire" => r.fire > v,
                    "led" => r.led > v,
                    "q" => r.q > v,
                    "len" => r.end - r.start > v,
                    _ => true,
                }));
            } else if let Some((k, v)) = t.split_once(':') {
                let (key, val) = (k.to_owned(), v.to_owned());
                tests.push(Box::new(move |r: &Row| match key.as_str() {
                    "subject" => r.subject == val,
                    "moment" => r.moment == val,
                    "who" | "person" => r.who.to_lowercase().contains(&val),
                    "clip" => r.clip.to_lowercase().starts_with(&val),
                    _ => true,
                }));
            } else if let Some(flag) = t.strip_prefix('-').or_else(|| t.strip_prefix('!')) {
                let flag = flag.to_owned();
                tests.push(Box::new(move |r: &Row| !r.flags.contains(&flag)));
            } else if t == "clean" {
                tests.push(Box::new(|r: &Row| {
                    !r.flags.iter().any(|f| {
                        ["dark", "soft", "whip", "shaky", "blown", "short"].contains(&f.as_str())
                    }) && r.moment != "transition"
                        && r.moment != "idle"
                        && !r.caption.contains("BLOCKED")
                        && !r
                            .caption
                            .split("issues:")
                            .nth(1)
                            .is_some_and(|i| i.contains("focus") || i.contains("blur"))
                }));
            } else {
                words.push(t);
            }
        }
        let mut hits: Vec<(f32, &Row)> = self
            .rows
            .iter()
            .filter(|r| tests.iter().all(|t| t(r)))
            .filter_map(|r| {
                let hay = format!("{} {} {} {}", r.id, r.subject, r.caption, r.who).to_lowercase();
                let matched = words.iter().filter(|w| hay.contains(w.as_str())).count();
                if !words.is_empty() && matched == 0 {
                    return None;
                }
                let used = plan.and_then(|p| used_in(p, r)).is_some();
                let score = matched as f32 + r.q - if used { 0.5 } else { 0.0 };
                Some((score, r))
            })
            .collect();
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        hits.into_iter().take(limit).map(|(_, r)| r).collect()
    }
}

/// The slot (1-based) of `plan` using footage of `row`, if any.
pub(crate) fn used_in(plan: &Plan, row: &Row) -> Option<usize> {
    plan.shots
        .iter()
        .position(|s| s.clip == row.clip && s.t_in < row.end && s.t_in + 0.5 > row.start)
        .map(|k| k + 1)
}

/// The plan, one line per shot.
pub(crate) fn plan_text(plan: &Plan, music: Option<&MusicMap>) -> String {
    let mut s = String::new();
    let drop_beat = (plan.drop_bar.saturating_sub(plan.start_bar)) * 4;
    let _ = writeln!(
        s,
        "plan {} · {} · bars {}-{} (drop at bar {}, end card at bar {}) · {} shots · {} beats",
        plan.name,
        aspect(plan),
        plan.start_bar,
        plan.end_bar,
        plan.drop_bar,
        plan.card_bar,
        plan.shots.len(),
        plan.footage_beats()
    );
    let r = &plan.rules;
    let _ = writeln!(
        s,
        "rules: same_clip_gap {:.0}s · person_gap {} · reject flags {} · reject moments {} · drop subject {}",
        r.same_clip_gap,
        r.person_gap,
        r.reject_flags.join(" "),
        r.reject_moments.join(" "),
        if r.drop_subject.is_empty() {
            "any"
        } else {
            &r.drop_subject
        }
    );
    let timing = music.map(|m| plan.timing(m));
    let mut beat = 0;
    for (k, shot) in plan.shots.iter().enumerate() {
        let when = timing
            .as_ref()
            .and_then(|t| t.get(k))
            .map(|(a, d)| format!(" · {a:.1}+{d:.1}s"))
            .unwrap_or_default();
        let drop = if beat == drop_beat { " · DROP" } else { "" };
        let _ = writeln!(
            s,
            "#{} {}@{:.2} · {} beats{when}{drop} · cx {:.2} · who {} · seg {} · note: {}",
            k + 1,
            shot.clip,
            shot.t_in,
            shot.beats,
            shot.cx,
            if shot.who.is_empty() { "?" } else { &shot.who },
            if shot.seg.is_empty() { "?" } else { &shot.seg },
            shot.note
        );
        beat += shot.beats;
    }
    s
}

/// The aspect key.
fn aspect(plan: &Plan) -> &'static str {
    match plan.aspect {
        crate::plan::Aspect::Vertical => "9x16",
        crate::plan::Aspect::Feed => "4x5",
        crate::plan::Aspect::Wide => "16x9",
    }
}

/// The music in a few lines: tempo, the plan window, drops, bar energy.
pub(crate) fn music_text(plan: &Plan, music: &MusicMap) -> String {
    let mut s = format!(
        "music: {:.1} BPM, {} bars, {:.0}s",
        music.bpm,
        music.bars.len(),
        music.duration
    );
    let drops: Vec<String> = music
        .drops
        .iter()
        .map(|d| format!("bar {} ({:.1}s)", d.bar, d.t))
        .collect();
    if !drops.is_empty() {
        let _ = write!(s, " · drop candidates {}", drops.join(", "));
    }
    let window: Vec<String> = music
        .bars
        .iter()
        .enumerate()
        .skip(plan.start_bar)
        .take(plan.end_bar.saturating_sub(plan.start_bar))
        .map(|(k, b)| format!("{k}:{:.2}", b.rms))
        .collect();
    if !window.is_empty() {
        let _ = write!(
            s,
            "\nbar loudness in the window (bar:rms): {}",
            window.join(" ")
        );
    }
    s
}

/// The check as the model reads it.
pub(crate) fn problems_text(problems: Option<&[Problem]>) -> String {
    match problems {
        None => "check: cannot run yet (the night needs its index and music map)".to_owned(),
        Some([]) => "check: passes".to_owned(),
        Some(ps) => {
            let mut s = format!("check: {} problems", ps.len());
            for p in ps {
                if p.shot == 0 {
                    let _ = write!(s, "\n- plan: {}", p.message);
                } else {
                    let _ = write!(s, "\n- shot {}: {}", p.shot, p.message);
                }
            }
            s
        }
    }
}

/// Everything a user message carries besides its text.
#[derive(Clone, Debug, Default)]
pub(crate) struct Snapshot {
    /// The plan as text, unless unchanged since the last message.
    pub(crate) plan: Option<String>,
    /// The music summary.
    pub(crate) music: Option<String>,
    /// The check.
    pub(crate) check: String,
    /// What the request may touch.
    pub(crate) scope: String,
    /// Attached references: token and resolved facts.
    pub(crate) refs: Vec<(String, String)>,
    /// What is selected in the app.
    pub(crate) selection: Vec<String>,
}

/// The snapshot as the text block sent to the model.
pub(crate) fn snapshot(s: &Snapshot) -> String {
    let mut out = String::from("<context>\n");
    match &s.plan {
        Some(p) => out.push_str(p),
        None => out.push_str("plan: unchanged since the previous message\n"),
    }
    if let Some(m) = &s.music {
        let _ = writeln!(out, "{m}");
    }
    let _ = writeln!(out, "{}", s.check);
    let _ = writeln!(out, "scope: {}", s.scope);
    if !s.refs.is_empty() {
        let _ = writeln!(out, "references:");
        for (token, facts) in &s.refs {
            let _ = writeln!(out, "- {token} = {facts}");
        }
    }
    if !s.selection.is_empty() {
        let _ = writeln!(out, "selected in the app: {}", s.selection.join(" "));
    }
    out.push_str("</context>");
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{Row, ShotLog, Snapshot};

    /// A row with the given basics.
    pub(crate) fn row(id: &str, subject: &str, q: f32, flags: &[&str], caption: &str) -> Row {
        let clip = id.split('/').next().unwrap_or(id).to_owned();
        Row {
            id: id.into(),
            clip,
            start: 0.0,
            end: 4.0,
            subject: subject.into(),
            q,
            flags: flags.iter().map(|f| (*f).to_owned()).collect(),
            camera: "static".into(),
            fire: if subject == "fire" { 0.08 } else { 0.0 },
            led: if subject == "led" { 0.05 } else { 0.0 },
            cx: (0.5, 0.1),
            caption: caption.into(),
            moment: "performing".into(),
            who: "fans-02".into(),
        }
    }

    pub(crate) fn log() -> ShotLog {
        ShotLog {
            rows: vec![
                row("A/1", "fire", 0.9, &[], "fire fans spinning close"),
                row("B/1", "fire", 0.6, &["dark"], "fire staff in the dark"),
                row("C/1", "led", 0.95, &[], "LED hoop on stage"),
            ],
        }
    }

    #[test]
    fn search_matches_words_and_facets() -> anyhow::Result<()> {
        let log = log();
        let hits = log.search("fire", 10, None);
        assert_eq!(hits.len(), 2, "two fire rows: {hits:?}");
        assert_eq!(hits[0].id, "A/1", "best first");
        let hits = log.search("fire -dark", 10, None);
        assert_eq!(hits.len(), 1, "dark excluded");
        let hits = log.search("q>0.9", 10, None);
        assert_eq!(hits.len(), 1, "facet only: {hits:?}");
        assert_eq!(hits[0].id, "C/1", "the LED row");
        let hits = log.search("subject:fire clean", 10, None);
        assert_eq!(hits.len(), 1, "clean drops flagged rows");
        assert!(log.get("C/1").is_some(), "lookup by id");
        assert!(
            hits[0].line().contains("q 0.90"),
            "a line quotes the numbers: {}",
            hits[0].line()
        );
        Ok(())
    }

    #[test]
    fn snapshots_carry_plan_check_scope_and_refs() -> anyhow::Result<()> {
        let plan = super::super::ops::tests::plan(2);
        let s = Snapshot {
            plan: Some(super::plan_text(&plan, None)),
            music: None,
            check: super::problems_text(Some(&[])),
            scope: "the whole plan".into(),
            refs: vec![(">@shot:t#2".into(), "t shot 2: B@10".into())],
            selection: Vec::new(),
        };
        let text = super::snapshot(&s);
        assert!(text.contains("#2 B@10.00 · 4 beats"), "shot line: {text}");
        assert!(text.contains("DROP"), "drop marked: {text}");
        assert!(text.contains("check: passes"), "check: {text}");
        assert!(text.contains("- >@shot:t#2 = t shot 2"), "refs: {text}");
        Ok(())
    }
}

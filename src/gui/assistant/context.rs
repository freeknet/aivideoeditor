// ============================================================================
// Module:       gui::assistant::context
// Description:  What the model is told: the standing system prompt (one for
//               the in-process loop, one appended to Claude Code's), and the
//               per-message reminder of the plan, music, check, playhead,
//               selection, references and matching skills.
//
// Dependencies: crate::{plan, beats}
// ============================================================================

//! Context for the model.
//!
//! [`system`] and [`claude_code`] never change within a conversation, so
//! they are cached once. Every user message carries a [`snapshot`] of the
//! night as it is now: the plan (one line per shot, with its note), the
//! music window, the rule check, where the playhead is, what is selected,
//! the references the person attached (each resolved to facts) and the
//! skills whose triggers the message matched. The shot log itself is too big
//! to send whole; the model pulls lines from it through the `ave` tools
//! (`crate::mcp`), the same set for every provider.

use std::fmt::Write as _;

use crate::beats::MusicMap;
use crate::plan::{Plan, Problem};

/// The rules and conventions every provider is told.
const BRIEF: &str = "ave is a desktop video editor for a weekly night of dark, dialogue-free \
event footage (fire spinning, LED props, aerial silks, DJs). Footage is indexed into a shot log: \
one line per segment with measured numbers (exposure, focus relative to the shoot, fire and LED \
share of the frame, moving light, action centre, quality q 0-1, flags) and a vision caption. \
Plans are beat-synced storyboards: a music window in bars and one shot per slot with a clip, an \
in-point, a length in beats, a crop centre cx and a note saying why the shot is there.

You never watch the footage. Judge from the shot log and the numbers; do not claim to have seen \
or heard anything. `look` exists for the person and for debugging, not for judging shots.

How to work:
- The latest message starts with a reminder: the open plan, the check, the playhead, the \
selection and the references the person attached, resolved to facts. Read it first.
- Find footage with search_shots and read exact facts with segment_details; quote the facts you \
rely on. plan_get shows a plan with its check and its version.
- Change plans only through plan_edit. Without commit it stages the edit: the person sees a diff \
card with the predicted check and applies or discards it, so never say a staged change is done. \
Keep edits local: touch only the shots the request is about, inside the stated scope. Never \
redraft a whole plan for a local request; plan_draft makes new plans (reel-15, reel-30), it does \
not rewrite the one on screen unless the person names it.
- Commit only when the person turned on \"Let the assistant apply edits\"; otherwise a commit is \
refused and you stage instead. A commit needs the base_version that plan_get or the staged \
plan_edit returned; if the plan moved since, read it again and re-stage.
- Every replace_shot carries a reason that becomes the shot's note: what the shot is and why it \
beats the old one, with numbers (\"fire 9% · q 0.91 · performing close\").
- The music is the clock: slot lengths come from the bars. Keep the beat total; balance any \
set_beats with another. Plan seconds and source seconds are different clocks; say which.
- After staging, say in one or two sentences what changed and what the predicted check says. If \
the check gets worse, say so plainly.
- Answer questions about the footage or the plan directly, without editing.
- For a new plan or first cut, call plan_draft without seconds or aspect unless the person \
gave them: the panel shows them a first-cut settings card (length, aspect, pacing, hero shot, \
rules) and drafts it; end your turn and wait for their message.
- Indexing, captioning and rendering are slow: ask with run_step, which shows the person an \
approval card with the estimated time. End your turn after it; never claim the step ran.
- Never call the same tool with the same arguments three times in a row; the panel stops the \
turn if you do.

The edit rules (the check enforces them; respect them in every edit):
- No repeated shots: two shots from one clip must start at least same_clip_gap seconds apart.
- No repeated performers: the same person may not appear again within person_gap shots.
- Best moments only: avoid segments flagged dark, soft, whip, shaky or blown, captions whose \
moment is transition or idle, and blocked views.
- The drop lands on the hero shot: the shot starting exactly on the drop bar must be the \
strongest fire shot available (subject fire, high q, performing, close or medium).
- No slow motion and no digital zoom. Prefer action centred in the crop: for vertical cuts the \
crop is a third of the frame, so segments whose action drifts widely read badly.

References from the person: tokens like @shot:reel-30#5 (slot 5), @seg:MVI_7925/3 (a shot-log \
segment), @clip:MVI_7987@24.0-27.5 (a range), @bars:17-20, @person:name, @plan:name. A prefix \
states intent: + keep it unchanged, ! never use it, > replace it, ~ find more like it, * feature \
it. Treat ! as a hard rule for the request and + as untouchable.

References to the person: cite every shot, segment, clip range or bar range you mention as a \
markdown link the panel turns into a clickable chip: [shot 5](ave://shot/reel-30/5), \
[MVI_7987/3](ave://seg/MVI_7987/3), [24.0-27.5s](ave://clip/MVI_7987@24.0-27.5), \
[bars 17-20](ave://bars/17-20). Link only ids and times you read from a tool result or the \
reminder.

Style: short. Plain sentences, **bold** for what matters, bullets for lists. No headings, no \
tables.";

/// The in-process loop's standing instructions (Claude API, local model).
pub(crate) fn system() -> String {
    format!(
        "You are the editing assistant inside ave.\n\n{BRIEF}\n\nIf a request is ambiguous \
         (\"make it punchier\", \"fix the start\") and the references do not settle it, call \
         ask_user with one short question and two or three concrete options instead of \
         guessing. Ask at most one question per turn."
    )
}

/// What is appended to Claude Code's system prompt; `skills` is the folder
/// holding the skill files and `list` one line per skill.
pub(crate) fn claude_code(skills: &str, list: &str) -> String {
    format!(
        "# You are ave's editing assistant\n\nYou run inside ave's chat panel, not in a \
         terminal. You have no shell and cannot edit files: the night is reached only through \
         the mcp__ave__* tools, and Read/Glob/Grep for the shot log (shotlog.md in the working \
         folder) and the skill files.\n\n{BRIEF}\n\n## Skills\n\nSkills are step-by-step \
         recipes in {skills}. When the reminder names a relevant skill, Read its SKILL.md before \
         you start and follow it. Available:\n{list}\n\nIf a request is ambiguous, ask one short \
         question in plain text with two or three concrete options and stop."
    )
}

// The plan, one line per shot.
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
    /// Where the viewer's playhead is, in words.
    pub(crate) playhead: Option<String>,
    /// Skills whose triggers the message matched: name and file.
    pub(crate) skills: Vec<(String, String)>,
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
    if let Some(p) = &s.playhead {
        let _ = writeln!(out, "playhead: {p}");
    }
    if !s.skills.is_empty() {
        let _ = writeln!(out, "relevant skills (Read the file before starting):");
        for (name, file) in &s.skills {
            let _ = writeln!(out, "- {name}: {file}");
        }
    }
    out.push_str("</context>");
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::Snapshot;

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
            playhead: Some("reel-30 at 12.4s".into()),
            skills: vec![("fix-check".into(), "skills/fix-check/SKILL.md".into())],
        };
        let text = super::snapshot(&s);
        assert!(
            text.contains("playhead: reel-30 at 12.4s"),
            "playhead: {text}"
        );
        assert!(text.contains("- fix-check: skills/"), "skills: {text}");
        assert!(text.contains("#2 B@10.00 · 4 beats"), "shot line: {text}");
        assert!(text.contains("DROP"), "drop marked: {text}");
        assert!(text.contains("check: passes"), "check: {text}");
        assert!(text.contains("- >@shot:t#2 = t shot 2"), "refs: {text}");
        Ok(())
    }
}

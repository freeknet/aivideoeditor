// ============================================================================
// Module:       gui::pages::inspector::details
// Description:  What the inspector shows for each kind of reference: segment
//               numbers and flags with their thresholds, caption, who, plan
//               back-links, the clip's segment strip; shots, bars, people.
//
// Dependencies: egui; crate::{index, caption, people, plan, refs, project}
// ============================================================================

//! The inspector's per-kind views.
//!
//! Everything is read from the open night: the shot log (`index.json`) for
//! the measurements, its captions, the people map for who, the music map
//! for bars, and the plans for where footage is used. Clicking a link here
//! changes the app-wide selection, so the inspector doubles as a way to
//! walk from a shot to its segment, its clip, its performer and back.

use std::path::{Path, PathBuf};

use egui::{Rect, Sense, Ui, vec2};

use crate::analyze::segments::Camera;
use crate::gui::thumbs::Thumbs;
use crate::gui::ui::theme;
use crate::index::Scored;
use crate::project::Project;
use crate::refs::{Reference, Target};
use crate::theme::Palette;

use super::rows::{self, SMALL, text};
use super::usage::{self, Plans};

/// Something a click in the inspector asked for.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Op {
    /// Make this the selection.
    Select(Reference),
    /// Load this clip range in the viewer.
    Open(PathBuf, f64, f64, f32),
}

/// What every view reads from.
pub(super) struct View<'a> {
    /// The palette.
    pub(super) palette: &'a Palette,
    /// The open night.
    pub(super) project: &'a Project,
    /// The night's plans.
    pub(super) plans: &'a Plans,
}

impl View<'_> {
    /// The source file of `clip`.
    fn path(&self, clip: &str) -> PathBuf {
        self.project
            .index
            .as_ref()
            .and_then(|i| i.clip(clip))
            .map(|c| PathBuf::from(&c.path))
            .unwrap_or_default()
    }

    /// The segment containing `t` seconds of `clip` (or the nearest one).
    fn segment_at(&self, clip: &str, t: f32) -> Option<&Scored> {
        let index = self.project.index.as_ref()?;
        let segs = index.segments.iter().filter(|s| s.clip == clip);
        segs.clone()
            .find(|s| (s.seg.start..s.seg.end).contains(&t))
            .or_else(|| {
                segs.min_by(|a, b| {
                    let d = |s: &Scored| (s.seg.start - t).abs().min((s.seg.end - t).abs());
                    d(a).total_cmp(&d(b))
                })
            })
    }
}

/// Why a flag is set: the measured value against the threshold
/// `index::score` applies (kept in step with it by hand).
pub(super) fn flag_reason(flag: &str, seg: &Scored, sharp_ref: f32) -> String {
    let g = &seg.seg;
    match flag {
        "dark" if g.p95 < 0.30 => format!("highlights p95 {:.2} < 0.30", g.p95),
        "dark" => format!("mean luma {:.3} < 0.035", g.luma),
        "soft" => format!(
            "focus {:.2}× the night's median < 0.60",
            g.sharp / sharp_ref.max(1e-6)
        ),
        "shaky" => "handheld jitter with no steady direction".to_owned(),
        "whip" => "camera moving too fast to read".to_owned(),
        "blown" => format!("{:.0}% of the frame clipped > 12%", g.clipped * 100.0),
        "no-action" => format!("moving light {:.2}% < 0.08%", g.moving * 100.0),
        "short" => format!("{:.2} s long < 1.0 s", g.end - g.start),
        other => other.to_owned(),
    }
}

/// A frame of `clip` at `t`, full width, 16:9; click loads the range in
/// the viewer.
fn picture(
    v: &View<'_>,
    ui: &mut Ui,
    thumbs: &mut Thumbs,
    clip: &str,
    (t0, t1): (f32, f32),
    cx: f32,
    ops: &mut Vec<Op>,
) {
    // Selects' rows stop 8 px short of the panel's right edge.
    let width = ui.available_width() - theme::SPACE_SM;
    let height = (width * 9.0 / 16.0).min(220.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let path = v.path(clip);
    let mid = f64::from(t0 + t1) / 2.0;
    thumbs.paint(ui, rect, &path, mid, 0.5, theme::rgb(v.palette.raised));
    // Where the action is: a hairline at cx.
    let x = rect.left() + rect.width() * cx.clamp(0.0, 1.0);
    ui.painter().vline(
        x,
        rect.y_range(),
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(110)),
    );
    if response
        .on_hover_text("Open in the viewer (the line marks the action centre)")
        .clicked()
    {
        ops.push(Op::Open(path, f64::from(t0), f64::from(t1), cx));
    }
}

/// Everything about one segment.
pub(super) fn segment(
    v: &View<'_>,
    ui: &mut Ui,
    thumbs: &mut Thumbs,
    seg: &Scored,
    ops: &mut Vec<Op>,
) {
    let g = &seg.seg;
    picture(v, ui, thumbs, &seg.clip, (g.start, g.end), g.cx, ops);
    rows::title(
        ui,
        v.palette,
        &seg.id,
        &format!(
            "{} · {:.2}–{:.2} s ({:.1} s)",
            seg.clip,
            g.start,
            g.end,
            g.end - g.start
        ),
    );
    measurements(v, ui, seg);
    caption(v, ui, seg);
    who(v, ui, &seg.clip, g.start, ops);
    used_in(v, ui, &seg.clip, g.start, g.end, ops);
    clip_strip(v, ui, &seg.clip, Some(&seg.id), ops);
}

/// The shot-log numbers and flags.
fn measurements(v: &View<'_>, ui: &mut Ui, seg: &Scored) {
    let p = v.palette;
    let g = &seg.seg;
    let sharp_ref = v.project.index.as_ref().map_or(1.0, |i| i.sharp_ref);
    rows::section(ui, p, "Shot log");
    rows::row_meter(ui, p, "Quality", seg.q, &format!("{:.2}", seg.q));
    let mut camera = g.camera.label().to_owned();
    match g.camera {
        Camera::ZoomIn | Camera::ZoomOut => camera.push_str(&format!(" ×{:.2}", g.zoom_factor)),
        Camera::Pan => camera.push_str(&format!(" {:+.2} widths", g.pan)),
        Camera::Static | Camera::Whip | Camera::Shaky | Camera::Unknown => {}
    }
    if g.after_cut {
        camera.push_str(" · after a cut");
    }
    rows::row(ui, p, "Camera", camera);
    rows::row(
        ui,
        p,
        "Exposure",
        format!(
            "mean {:.2} · p95 {:.2} · clipped {:.1}%",
            g.luma,
            g.p95,
            g.clipped * 100.0
        ),
    );
    rows::row(
        ui,
        p,
        "Focus",
        format!("{:.2}× the night's median", g.sharp / sharp_ref.max(1e-6)),
    );
    rows::row(ui, p, "Subject", seg.subject.as_str());
    rows::row(
        ui,
        p,
        "Light",
        format!(
            "fire {:.1}% · LED {:.1}% · white {:.1}%",
            g.flame * 100.0,
            g.led * 100.0,
            g.white * 100.0
        ),
    );
    rows::row(
        ui,
        p,
        "Moving",
        format!("{:.2}% of the frame", g.moving * 100.0),
    );
    rows::row(
        ui,
        p,
        "Action",
        format!("cx {:.2} ± {:.2}, cy {:.2}", g.cx, g.cx_spread / 2.0, g.cy),
    );
    if seg.flags.is_empty() {
        rows::row_colored(ui, p, "Flags", "none", theme::rgb(p.success));
    } else {
        for (k, flag) in seg.flags.iter().enumerate() {
            rows::row_colored(
                ui,
                p,
                if k == 0 { "Flags" } else { "" },
                format!("{flag}: {}", flag_reason(flag, seg, sharp_ref)),
                theme::rgb(p.warning),
            );
        }
    }
}

/// The vision caption, every field.
fn caption(v: &View<'_>, ui: &mut Ui, seg: &Scored) {
    let p = v.palette;
    rows::section(ui, p, "Caption");
    let Some(c) = &seg.caption else {
        rows::row_colored(
            ui,
            p,
            "",
            "Not captioned (below the quality cut, or captioning has not run).",
            theme::rgb(p.text_muted),
        );
        return;
    };
    ui.add(egui::Label::new(text(&c.summary, SMALL, theme::rgb(p.text))).wrap());
    ui.add_space(2.0);
    let moment = c.effective_moment();
    if moment == c.moment {
        rows::row(ui, p, "Moment", moment);
    } else {
        rows::row(
            ui,
            p,
            "Moment",
            format!(
                "{moment} (model said {}; the summary says nobody)",
                c.moment
            ),
        );
    }
    rows::row(ui, p, "Framing", c.framing.as_str());
    rows::row(ui, p, "Setting", c.setting.as_str());
    for (k, person) in c.people.iter().enumerate() {
        let prop = if person.prop.is_empty() || person.prop == "none" {
            String::new()
        } else {
            format!(" [{}]", person.prop)
        };
        rows::row(
            ui,
            p,
            if k == 0 { "People" } else { "" },
            format!("{}{prop}", person.appearance),
        );
    }
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    rows::row(ui, p, "Props lit", yes_no(c.props_active));
    if c.blocked {
        rows::row_colored(
            ui,
            p,
            "Blocked",
            "yes: something is in front of the subject",
            theme::rgb(p.warning),
        );
    } else {
        rows::row(ui, p, "Blocked", "no");
    }
    if !c.issues.is_empty() {
        rows::row_colored(ui, p, "Issues", c.issues.join(", "), theme::rgb(p.warning));
    }
}

/// Who the people map says this is.
fn who(v: &View<'_>, ui: &mut Ui, clip: &str, t: f32, ops: &mut Vec<Op>) {
    let p = v.palette;
    rows::section(ui, p, "Who");
    match v.project.people.entry_at(clip, t) {
        Some(name) if name != clip => {
            if rows::row_link(ui, p, "Person", name) {
                ops.push(Op::Select(Reference::new(Target::Person {
                    name: name.to_owned(),
                })));
            }
        }
        Some(_) | None => rows::row_colored(
            ui,
            p,
            "Person",
            "not in people.toml (counts as its own person)",
            theme::rgb(p.text_muted),
        ),
    }
}

/// The plan shots that use this footage.
fn used_in(v: &View<'_>, ui: &mut Ui, clip: &str, t0: f32, t1: f32, ops: &mut Vec<Op>) {
    let p = v.palette;
    let uses = v.plans.uses(clip, t0, t1, v.project.music.as_ref());
    rows::section(ui, p, "Used in");
    if uses.is_empty() {
        rows::row_colored(ui, p, "", "no plan uses it", theme::rgb(p.text_muted));
        return;
    }
    for u in uses {
        let label = format!("{} shot {}", u.plan, u.shot);
        let at = format!("@{:.1}–{:.1}", u.range.0, u.range.1);
        if rows::row_link(ui, p, &at, &label) {
            ops.push(Op::Select(Reference::new(Target::Shot {
                plan: u.plan,
                index: u.shot,
            })));
        }
    }
}

/// The clip's segments as a strip proportional to time, coloured by
/// quality; the current one outlined. Click one to select it.
fn clip_strip(v: &View<'_>, ui: &mut Ui, clip: &str, current: Option<&str>, ops: &mut Vec<Op>) {
    let p = v.palette;
    let Some(index) = v.project.index.as_ref() else {
        return;
    };
    let Some(info) = index.clip(clip) else {
        return;
    };
    let segs: Vec<&Scored> = index.segments.iter().filter(|s| s.clip == clip).collect();
    rows::section_with(ui, p, "Clip", &format!("{} segments", segs.len()));
    rows::row(
        ui,
        p,
        "Source",
        format!(
            "{} · {:.1} s · {}×{} · {:.2} fps · {}",
            clip,
            info.probe.duration,
            info.probe.width,
            info.probe.height,
            info.probe.fps_f64(),
            info.probe.vcodec
        ),
    );
    let total = info
        .probe
        .duration
        .max(f64::from(segs.last().map_or(0.0, |s| s.seg.end)))
        .max(0.1) as f32;
    ui.add_space(theme::SPACE_XS);
    let width = ui.available_width() - theme::SPACE_SM;
    let (rect, _) = ui.allocate_exact_size(vec2(width, theme::COMPACT_HEIGHT), Sense::hover());
    for s in &segs {
        let x0 = rect.left() + rect.width() * (s.seg.start / total);
        let x1 = rect.left() + rect.width() * (s.seg.end / total);
        let r = Rect::from_x_y_ranges(x0..=(x1 - 1.0).max(x0 + 1.0), rect.y_range());
        // Brighter is better: a neutral ramp, so the key colour stays for
        // what is selected.
        let fill = p.raised.lerp(p.text_muted, s.q.clamp(0.0, 1.0));
        let id = ui.scope_id().with(("strip", &s.id));
        let response = ui.interact(r, id, Sense::click());
        ui.painter()
            .rect_filled(r, egui::CornerRadius::same(2), theme::rgb(fill));
        if !s.flags.is_empty() {
            ui.painter().hline(
                r.x_range(),
                r.bottom() - 1.0,
                egui::Stroke::new(2.0, theme::rgb(p.warning)),
            );
        }
        if Some(s.id.as_str()) == current {
            ui.painter().rect_stroke(
                r,
                egui::CornerRadius::same(2),
                egui::Stroke::new(1.5, theme::rgb(p.accent)),
                egui::StrokeKind::Inside,
            );
        }
        let hover = format!(
            "{} · {:.1}–{:.1} s · q{:.2}{}\nBrighter is higher quality; an amber underline is a flag.",
            s.id,
            s.seg.start,
            s.seg.end,
            s.q,
            if s.flags.is_empty() {
                String::new()
            } else {
                format!(" · {}", s.flags.join(" "))
            }
        );
        if response.on_hover_text(hover).clicked() {
            ops.push(Op::Select(Reference::new(Target::Segment {
                id: s.id.clone(),
            })));
            ops.push(Op::Open(
                v.path(clip),
                f64::from(s.seg.start),
                f64::from(s.seg.end),
                s.seg.cx,
            ));
        }
    }
}

/// A range or frame of a clip: the picture, then the segment under it.
pub(super) fn range(
    v: &View<'_>,
    ui: &mut Ui,
    thumbs: &mut Thumbs,
    clip: &str,
    (t0, t1): (f32, f32),
    ops: &mut Vec<Op>,
) {
    let seg = v.segment_at(clip, (t0 + t1) / 2.0);
    let cx = seg.map_or(0.5, |s| s.seg.cx);
    picture(v, ui, thumbs, clip, (t0, t1.max(t0 + 0.04)), cx, ops);
    let title = if (t1 - t0).abs() < 0.01 {
        format!("{clip} @ {t0:.2} s")
    } else {
        format!("{clip} {t0:.2}–{t1:.2} s ({:.1} s)", t1 - t0)
    };
    let under = seg.map_or_else(String::new, |seg| {
        format!(
            "Segment {} ({:.1}–{:.1} s)",
            seg.id, seg.seg.start, seg.seg.end
        )
    });
    rows::title(ui, v.palette, &title, &under);
    if let Some(index) = v.project.index.as_ref() {
        let over: Vec<&Scored> = index
            .segments
            .iter()
            .filter(|s| s.clip == clip && s.seg.end > t0 && s.seg.start < t1.max(t0 + 0.01))
            .collect();
        if over.len() > 1 {
            rows::section(ui, v.palette, "Spans");
            for s in over {
                if rows::row_link(ui, v.palette, &format!("q{:.2}", s.q), &s.id) {
                    ops.push(Op::Select(Reference::new(Target::Segment {
                        id: s.id.clone(),
                    })));
                }
            }
        }
    }
    if let Some(seg) = seg {
        measurements(v, ui, seg);
        caption(v, ui, seg);
    }
    who(v, ui, clip, t0, ops);
    used_in(v, ui, clip, t0, t1, ops);
    clip_strip(v, ui, clip, seg.map(|s| s.id.as_str()), ops);
}

/// A plan shot: its fields, where it sits on the beat grid, its footage.
pub(super) fn shot(
    v: &View<'_>,
    ui: &mut Ui,
    thumbs: &mut Thumbs,
    plan_name: &str,
    slot: usize,
    ops: &mut Vec<Op>,
) {
    let p = v.palette;
    let Some(plan) = v.plans.get(plan_name) else {
        rows::row_colored(
            ui,
            p,
            "",
            format!("No plan called {plan_name}"),
            theme::rgb(p.warning),
        );
        return;
    };
    let Some(shot) = slot.checked_sub(1).and_then(|k| plan.shots.get(k)) else {
        rows::row_colored(
            ui,
            p,
            "",
            format!("{plan_name} has no shot {slot}"),
            theme::rgb(p.warning),
        );
        return;
    };
    let lengths = usage::shot_lengths(plan, v.project.music.as_ref());
    let len = lengths.get(slot - 1).copied().unwrap_or(0.0);
    picture(
        v,
        ui,
        thumbs,
        &shot.clip,
        (shot.t_in, shot.t_in + len.max(0.04)),
        shot.cx,
        ops,
    );
    rows::title(ui, p, &format!("{plan_name} · shot {slot}"), "");
    let beats_before: usize = plan.shots.iter().take(slot - 1).map(|s| s.beats).sum();
    let bar = plan.start_bar + beats_before / 4;
    let beat = beats_before % 4 + 1;
    let at: f32 = lengths.iter().take(slot - 1).sum();
    rows::section(ui, p, "Shot");
    if rows::row_link(ui, p, "Clip", &shot.clip) {
        ops.push(Op::Select(Reference::new(Target::Range {
            clip: shot.clip.clone(),
            t0: shot.t_in,
            t1: shot.t_in + len,
        })));
    }
    rows::row(
        ui,
        p,
        "In",
        format!("{:.2} s (to {:.2} s)", shot.t_in, shot.t_in + len),
    );
    rows::row(
        ui,
        p,
        "Beats",
        format!(
            "{} ({len:.2} s) from bar {bar} beat {beat}, {at:.2} s in",
            shot.beats
        ),
    );
    if bar == plan.drop_bar && beat == 1 {
        rows::row(ui, p, "", "lands on the drop");
    }
    rows::row(ui, p, "Crop cx", format!("{:.2}", shot.cx));
    let who = v.project.people.who(shot);
    let source = if shot.who.is_empty() {
        "from people.toml"
    } else {
        "set in the plan"
    };
    if rows::row_link(ui, p, "Who", &who) {
        ops.push(Op::Select(Reference::new(Target::Person {
            name: who.clone(),
        })));
    }
    rows::row_colored(ui, p, "", source, theme::rgb(p.text_faint));
    if !shot.seg.is_empty() && rows::row_link(ui, p, "Segment", &shot.seg) {
        ops.push(Op::Select(Reference::new(Target::Segment {
            id: shot.seg.clone(),
        })));
    }
    rows::row(
        ui,
        p,
        "Note",
        if shot.note.is_empty() {
            "(no note: say why this shot is here)"
        } else {
            shot.note.as_str()
        },
    );
    if let Some(seg) = v.segment_at(&shot.clip, shot.t_in) {
        measurements(v, ui, seg);
        caption(v, ui, seg);
        used_in(v, ui, &shot.clip, shot.t_in, shot.t_in + len, ops);
        clip_strip(v, ui, &shot.clip, Some(&seg.id), ops);
    }
}

/// Music bars: times, tempo, energy per bar, drops.
pub(super) fn bars(v: &View<'_>, ui: &mut Ui, from: usize, to: usize) {
    let p = v.palette;
    let Some(m) = v.project.music.as_ref() else {
        rows::row_colored(
            ui,
            p,
            "",
            "The music is not mapped yet.",
            theme::rgb(p.text_muted),
        );
        return;
    };
    let t0 = m.beat_time(from, 0);
    let t1 = m.beat_time(to + 1, 0);
    let title = if from == to {
        format!("Bar {from}")
    } else {
        format!("Bars {from}–{to}")
    };
    rows::title(
        ui,
        p,
        &title,
        &format!(
            "{t0:.2}–{t1:.2} s ({:.2} s) of {}",
            t1 - t0,
            Path::new(&m.path)
                .file_name()
                .map_or_else(|| m.path.clone(), |n| n.to_string_lossy().into_owned())
        ),
    );
    rows::section(ui, p, "Music");
    rows::row(
        ui,
        p,
        "Tempo",
        format!(
            "{:.1} bpm · {} beats",
            m.bpm,
            (to + 1 - from.min(to + 1)) * 4
        ),
    );
    let loudest = m
        .bars
        .iter()
        .map(|b| b.rms)
        .fold(0.0_f32, f32::max)
        .max(1e-6);
    rows::section(ui, p, "Energy");
    for bar in from..=to.min(from + 63) {
        let Some(b) = m.bars.get(bar) else {
            break;
        };
        let drop = m.drops.iter().find(|d| d.bar == bar);
        let label = match drop {
            Some(_) => format!("{bar} drop"),
            None => format!("{bar}"),
        };
        rows::row_meter(
            ui,
            p,
            &label,
            b.rms / loudest,
            &format!(
                "{:.0}% · low {:.0}%",
                b.rms / loudest * 100.0,
                b.low * 100.0
            ),
        );
    }
    let drops: Vec<String> = m
        .drops
        .iter()
        .filter(|d| (from..=to).contains(&d.bar))
        .map(|d| format!("bar {} @{:.1}s (score {:.2})", d.bar, d.t, d.score))
        .collect();
    if !drops.is_empty() {
        rows::row(ui, p, "Drops", drops.join(" · "));
    }
}

/// A performer: their entries, segments and the shots that show them.
pub(super) fn person(
    v: &View<'_>,
    ui: &mut Ui,
    thumbs: &mut Thumbs,
    name: &str,
    ops: &mut Vec<Op>,
) {
    let p = v.palette;
    let keys = v.project.people.keys_of(name);
    let segs: Vec<&Scored> = v
        .project
        .index
        .as_ref()
        .map(|i| {
            i.segments
                .iter()
                .filter(|s| v.project.people.entry_at(&s.clip, s.seg.start) == Some(name))
                .collect()
        })
        .unwrap_or_default();
    rows::title(
        ui,
        p,
        name,
        &format!("{} entries · {} segments", keys.len(), segs.len()),
    );
    ui.add_space(theme::SPACE_XS);
    // The best few frames.
    let mut best = segs.clone();
    best.sort_by(|a, b| b.q.total_cmp(&a.q));
    let mut clips_seen = Vec::new();
    best.retain(|s| {
        let fresh = !clips_seen.contains(&s.clip);
        clips_seen.push(s.clip.clone());
        fresh
    });
    if !best.is_empty() {
        let width = ui.available_width();
        let cell = ((width - 6.0) / 4.0).max(10.0);
        let (rect, _) = ui.allocate_exact_size(vec2(width, cell * 1.25), Sense::hover());
        for (k, s) in best.iter().take(4).enumerate() {
            let r = Rect::from_min_size(
                rect.min + vec2(k as f32 * (cell + 2.0), 0.0),
                vec2(cell, cell * 1.25),
            );
            let mid = f64::from(s.seg.start + s.seg.end) / 2.0;
            thumbs.paint(ui, r, &v.path(&s.clip), mid, s.seg.cx, theme::rgb(p.hover));
            let response = ui.interact(r, ui.scope_id().with(("person-frame", k)), Sense::click());
            if response.on_hover_text(&s.id).clicked() {
                ops.push(Op::Select(Reference::new(Target::Segment {
                    id: s.id.clone(),
                })));
            }
        }
    }
    let props: Vec<String> = {
        let mut counts: std::collections::BTreeMap<&'static str, usize> = Default::default();
        for s in &segs {
            if let Some(main) = s.caption.as_ref().and_then(|c| c.people.first()) {
                let kind = crate::people::prop_kind(&main.prop);
                if kind != "none" {
                    *counts.entry(kind).or_default() += 1;
                }
            }
        }
        let mut v: Vec<(&str, usize)> = counts.into_iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v.into_iter().map(|(k, n)| format!("{k} {n}")).collect()
    };
    rows::section(ui, p, "Person");
    if !props.is_empty() {
        rows::row(ui, p, "Props", props.join(" · "));
    }
    if let Some(sample) = segs
        .iter()
        .find_map(|s| s.caption.as_ref().and_then(|c| c.people.first()))
    {
        rows::row(ui, p, "Looks like", sample.appearance.as_str());
    }
    rows::section(ui, p, "Entries");
    for key in &keys {
        let (clip, range) = crate::people::parse_key(key);
        let (t0, t1) = range.unwrap_or((0.0, 0.0));
        let label = if range.is_some() { "range" } else { "clip" };
        if rows::row_link(ui, p, label, key) {
            let target = match range {
                Some(_) => Target::Range {
                    clip: clip.to_owned(),
                    t0,
                    t1,
                },
                None => Target::Range {
                    clip: clip.to_owned(),
                    t0: 0.0,
                    t1: v
                        .project
                        .index
                        .as_ref()
                        .and_then(|i| i.clip(clip))
                        .map_or(1.0, |c| c.probe.duration as f32),
                },
            };
            ops.push(Op::Select(Reference::new(target)));
        }
    }
    let uses = v
        .plans
        .uses_of_person(name, &v.project.people, v.project.music.as_ref());
    rows::section(ui, p, "Used in");
    if uses.is_empty() {
        rows::row_colored(ui, p, "", "no plan shows them", theme::rgb(p.text_muted));
    } else {
        rows::row(ui, p, "Shots", usage::summary(&uses));
    }
}

/// A whole plan.
pub(super) fn plan(v: &View<'_>, ui: &mut Ui, name: &str, ops: &mut Vec<Op>) {
    let p = v.palette;
    rows::title(ui, p, name, "");
    let Some(plan) = v.plans.get(name) else {
        rows::row_colored(
            ui,
            p,
            "",
            "Could not read this plan.",
            theme::rgb(p.warning),
        );
        return;
    };
    rows::section(ui, p, "Plan");
    let (w, h) = plan.aspect.size();
    rows::row(ui, p, "Frame", format!("{w}×{h}"));
    rows::row(
        ui,
        p,
        "Bars",
        format!(
            "{}–{} · drop {} · card {}",
            plan.start_bar, plan.end_bar, plan.drop_bar, plan.card_bar
        ),
    );
    let lengths = usage::shot_lengths(plan, v.project.music.as_ref());
    rows::row(
        ui,
        p,
        "Shots",
        format!(
            "{} · {:.1} s of footage",
            plan.shots.len(),
            lengths.iter().sum::<f32>()
        ),
    );
    rows::section(ui, p, "Shots");
    for (k, shot) in plan.shots.iter().enumerate() {
        let label = format!("#{}", k + 1);
        let value = format!(
            "{} @{:.1} · {}b · {}",
            shot.clip,
            shot.t_in,
            shot.beats,
            v.project.people.who(shot)
        );
        if rows::row_link(ui, p, &label, &value) {
            ops.push(Op::Select(Reference::new(Target::Shot {
                plan: name.to_owned(),
                index: k + 1,
            })));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::flag_reason;
    use crate::analyze::segments::{Camera, Segment};
    use crate::index::Scored;

    #[test]
    fn flags_explain_their_threshold() {
        let seg = Scored {
            id: "C/1".to_owned(),
            clip: "C".to_owned(),
            seg: Segment {
                start: 0.0,
                end: 0.5,
                camera: Camera::Static,
                after_cut: false,
                luma: 0.1,
                p95: 0.2,
                clipped: 0.2,
                sharp: 0.3,
                flame: 0.0,
                led: 0.0,
                white: 0.0,
                moving: 0.0001,
                cx: 0.5,
                cx_spread: 0.1,
                cy: 0.5,
                zoom_factor: 1.0,
                pan: 0.0,
                jitter: 0.0,
                ..Default::default()
            },
            subject: "none".to_owned(),
            q: 0.1,
            flags: Vec::new(),
            caption: None,
            ..Default::default()
        };
        assert!(
            flag_reason("dark", &seg, 1.0).contains("0.30"),
            "p95 threshold"
        );
        assert!(
            flag_reason("soft", &seg, 1.0).contains("0.60"),
            "focus threshold"
        );
        assert!(
            flag_reason("blown", &seg, 1.0).contains("12%"),
            "clip threshold"
        );
        assert!(flag_reason("short", &seg, 1.0).contains("1.0 s"), "length");
        assert!(
            flag_reason("no-action", &seg, 1.0).contains("0.08%"),
            "motion"
        );
    }
}

// ============================================================================
// Module:       gui::pages::plan::strips
// Description:  The rows under the tracks: the selected shot (note, framing,
//               alternatives button), the alternatives strip, the check bar,
//               and the banner shown while viewing an old version.
//
// Dependencies: egui; crate::gui::pages::plan
// ============================================================================

//! The rows under the tracks.

use std::collections::BTreeSet;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Layout, Rect, Sense, Stroke, Ui, UiBuilder, pos2,
    vec2,
};

use super::{Drag, Io, Shown, State, edit};
use crate::gui::ui::theme;

/// Lays out `add` left to right inside `rect`.
fn row<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
        add,
    )
    .inner
}

/// The banner over the tracks while an old version is on show.
pub(super) fn viewing_banner(
    state: &mut State,
    s: &Shown<'_>,
    ui: &mut Ui,
    n: u32,
    saved: &str,
    change: &str,
) {
    let t = s.theme;
    egui::Frame::new()
        .fill(theme::translucent(t.warning, 40))
        .stroke(Stroke::new(1.0, theme::translucent(t.warning, 140)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!("Viewing v{n} · {saved} · {change} — read-only"))
                        .size(11.5)
                        .color(theme::rgb(t.text)),
                );
                if ui.button("Restore this version").clicked()
                    && let Some(v) = state.viewing.take()
                {
                    let plan = v.plan;
                    state.apply(format!("restore v{n}"), |p| {
                        *p = plan;
                        true
                    });
                }
                if ui.button("Back to current").clicked() {
                    state.viewing = None;
                }
            });
        });
}

/// The row under the tracks: what is selected, its note, its framing.
pub(super) fn selection_bar(
    state: &mut State,
    s: &Shown<'_>,
    io: &mut Io<'_>,
    ui: &mut Ui,
    rect: Rect,
) {
    let t = s.theme;
    ui.painter()
        .rect_filled(rect, CornerRadius::same(3), theme::rgb(t.raised));
    let inner = rect.shrink2(vec2(6.0, 2.0));
    let muted = theme::rgb(t.text_muted);
    let Some(k) = s.single() else {
        let text = if s.sel.is_empty() {
            "Click a shot · Shift/Ctrl add · drag to reorder · drag an edit to roll · Alt-drag to slip · A alternatives · Del lift · R reference in chat".to_owned()
        } else {
            let list: Vec<String> = s.sel.iter().map(|k| (k + 1).to_string()).collect();
            format!(
                "{} shots selected ({}) · R references them in chat · Del lifts them",
                s.sel.len(),
                list.join(", ")
            )
        };
        row(ui, inner, |ui| {
            ui.label(egui::RichText::new(text).size(11.0).color(muted));
        });
        return;
    };
    let Some(shot) = s.plan.shots.get(k) else {
        return;
    };
    let dur = s.timing.get(k).map_or(0.0, |x| x.1);
    let frame_w = (inner.height() - 4.0) * 16.0 / 9.0;
    let alt_w = 118.0;
    let text_rect = Rect::from_min_max(
        inner.min,
        pos2(inner.right() - frame_w - alt_w - 16.0, inner.bottom()),
    );
    row(ui, text_rect, |ui| {
        ui.label(
            egui::RichText::new(format!("#{}", k + 1))
                .strong()
                .color(theme::rgb(t.accent)),
        );
        ui.label(
            egui::RichText::new(format!(
                "{} @ {:.2}s · {}b {dur:.2}s",
                shot.clip, shot.t_in, shot.beats
            ))
            .size(11.5)
            .color(theme::rgb(t.text)),
        );
        if !shot.who.is_empty() {
            ui.label(egui::RichText::new(&shot.who).size(11.0).color(muted));
        }
        // The note: the reason the shot is there.
        let buf = match &mut state.note_edit {
            Some((n, text)) if *n == k => text,
            other => {
                *other = Some((k, shot.note.clone()));
                match other {
                    Some((_, text)) => text,
                    None => return,
                }
            }
        };
        let resp = ui.add_enabled(
            s.editable,
            egui::TextEdit::singleline(buf)
                .hint_text("why this shot is here")
                .font(FontId::proportional(11.5))
                .desired_width(ui.available_width()),
        );
        if !resp.has_focus() && *buf != shot.note {
            if resp.lost_focus() {
                let note = buf.clone();
                state.apply(format!("note shot {}", k + 1), |p| {
                    p.shots.get_mut(k).is_some_and(|sh| {
                        sh.note = note;
                        true
                    })
                });
            } else {
                *buf = shot.note.clone();
            }
        }
    });

    // Framing: the crop for the plan's aspect over the source frame.
    let fr = Rect::from_min_size(
        pos2(inner.right() - frame_w - alt_w - 8.0, inner.top() + 2.0),
        vec2(frame_w, inner.height() - 4.0),
    );
    framing(state, s, io, ui, fr, k);
    let ab = Rect::from_min_size(
        pos2(inner.right() - alt_w, inner.top() + 1.0),
        vec2(alt_w, inner.height() - 2.0),
    );
    row(ui, ab, |ui| {
        if ui
            .add_enabled(s.editable, egui::Button::new("Alternatives  A"))
            .on_hover_text("Rank footage that fits this slot without breaking a rule")
            .clicked()
        {
            super::open_alternatives(state, s, k);
        }
    });
}

/// The framing strip: drag the crop window to move the shot's `cx`.
fn framing(state: &mut State, s: &Shown<'_>, io: &mut Io<'_>, ui: &mut Ui, rect: Rect, k: usize) {
    let t = s.theme;
    let Some(shot) = s.plan.shots.get(k) else {
        return;
    };
    if let Some(path) = s.clip_path(&shot.clip) {
        io.thumbs.paint(
            ui,
            rect,
            &path,
            f64::from(shot.t_in),
            0.5,
            theme::rgb(t.hover),
        );
    } else {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(2), theme::rgb(t.hover));
    }
    let crop = s.plan.aspect.crop_frac();
    let resp = ui
        .interact(
            rect,
            egui::Id::unique("plan_framing").with(k),
            Sense::click_and_drag(),
        )
        .on_hover_text(if crop < 1.0 {
            format!(
                "{} crop, cx {:.2}: drag to reframe",
                super::aspect_key(s.plan.aspect),
                shot.cx
            )
        } else {
            "16x9 uses the whole frame".to_owned()
        });
    let p = ui.painter().with_clip_rect(rect);
    if crop < 1.0 {
        let w = rect.width() * crop;
        let cx = match (&state.drag, state.plan.as_ref()) {
            (Some(Drag::Frame { .. }), Some(plan)) => plan.shots.get(k).map_or(shot.cx, |sh| sh.cx),
            _ => shot.cx,
        };
        let x0 = rect.left() + cx * rect.width() - w / 2.0;
        let window = Rect::from_min_size(pos2(x0, rect.top()), vec2(w, rect.height()));
        let shade = Color32::from_black_alpha(150);
        p.rect_filled(
            Rect::from_min_max(rect.min, pos2(window.left(), rect.bottom())),
            CornerRadius::ZERO,
            shade,
        );
        p.rect_filled(
            Rect::from_min_max(pos2(window.right(), rect.top()), rect.max),
            CornerRadius::ZERO,
            shade,
        );
        p.rect_stroke(
            window,
            CornerRadius::ZERO,
            Stroke::new(1.5, theme::rgb(t.accent)),
            egui::StrokeKind::Inside,
        );
        if s.editable {
            if resp.drag_started()
                && let Some(plan) = state.plan.clone()
            {
                state.drag = Some(Drag::Frame {
                    shot: k,
                    before: plan,
                });
            }
            if (resp.dragged() || resp.clicked())
                && let Some(pos) = resp.interact_pointer_pos()
            {
                let cx = (pos.x - rect.left()) / rect.width();
                if resp.clicked() {
                    state.apply(format!("reframe shot {}", k + 1), |plan| {
                        edit::reframe(plan, k, cx)
                    });
                } else if state
                    .plan
                    .as_mut()
                    .is_some_and(|plan| edit::reframe(plan, k, cx))
                {
                    state.rev += 1;
                }
            }
            if resp.drag_stopped()
                && let Some(Drag::Frame { shot, before }) = state.drag.take()
            {
                state.commit_drag(before, format!("reframe shot {}", shot + 1));
            }
        }
    }
}

/// The alternatives strip for the open slot.
pub(super) fn alternatives(
    state: &mut State,
    s: &Shown<'_>,
    io: &mut Io<'_>,
    ui: &mut Ui,
    rect: Rect,
) {
    let t = s.theme;
    let Some(alts) = state.alts.as_ref() else {
        return;
    };
    let slot = alts.slot;
    if alts.rev != s.rev && s.plan.shots.get(slot).is_some() {
        super::open_alternatives(state, s, slot);
    }
    let Some(alts) = state.alts.as_ref() else {
        return;
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(3),
        theme::rgb(t.panel),
        Stroke::new(1.0, theme::rgb(t.border)),
        egui::StrokeKind::Inside,
    );
    let head = Rect::from_min_size(rect.min + vec2(6.0, 2.0), vec2(rect.width() - 12.0, 16.0));
    let want = s
        .project
        .index
        .as_ref()
        .map(|i| crate::alternatives::want_for(&s.plan, slot, i).label())
        .unwrap_or_default();
    let beats = s.plan.shots.get(slot).map_or(0, |sh| sh.beats);
    let mut close = false;
    row(ui, head, |ui| {
        ui.label(
            egui::RichText::new(format!(
                "ALTERNATIVES FOR SHOT {} · {want} · {beats} beats",
                slot + 1
            ))
            .size(10.0)
            .strong()
            .color(theme::rgb(t.text_muted)),
        );
        ui.label(
            egui::RichText::new("1–8 or click swaps · Esc closes")
                .size(10.0)
                .color(theme::rgb(t.text_faint)),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("Close").clicked() {
                close = true;
            }
        });
    });
    if close {
        state.alts = None;
        return;
    }
    let items = alts.items.clone();
    let body = Rect::from_min_max(
        pos2(rect.left() + 4.0, head.bottom() + 2.0),
        rect.max - vec2(4.0, 4.0),
    );
    if items.is_empty() {
        row(ui, body, |ui| {
            ui.label(
                egui::RichText::new("Nothing else fits this slot without breaking a rule. Loosen the rules or free up a clip.")
                    .size(11.0)
                    .color(theme::rgb(t.text_muted)),
            );
        });
        return;
    }
    let ratio = super::aspect_ratio(s.plan.aspect);
    let dur = s.timing.get(slot).map_or(1.0, |x| x.1);
    let mut take = None;
    ui.scope_builder(UiBuilder::new().max_rect(body), |ui| {
        egui::ScrollArea::horizontal()
            .id_salt("plan_alts")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (n, alt) in items.iter().enumerate() {
                        let (card, resp) = ui
                            .allocate_exact_size(vec2(200.0, body.height() - 2.0), Sense::click());
                        let hovered = resp.hovered();
                        ui.painter().rect(
                            card,
                            CornerRadius::same(3),
                            theme::rgb(if hovered { t.hover } else { t.raised }),
                            Stroke::new(1.0, theme::rgb(if hovered { t.accent } else { t.border })),
                            egui::StrokeKind::Inside,
                        );
                        let th_h = card.height() - 6.0;
                        let th = Rect::from_min_size(
                            card.min + vec2(3.0, 3.0),
                            vec2((th_h * ratio).min(80.0), th_h),
                        );
                        if let Some(path) = s.clip_path(&alt.clip) {
                            io.thumbs.paint(
                                ui,
                                th,
                                &path,
                                f64::from(alt.t_in),
                                alt.cx,
                                theme::rgb(t.hover),
                            );
                        }
                        let badge = Rect::from_min_size(th.min, vec2(14.0, 14.0));
                        ui.painter().rect_filled(
                            badge,
                            CornerRadius::same(2),
                            theme::rgb(t.accent),
                        );
                        ui.painter().text(
                            badge.center(),
                            Align2::CENTER_CENTER,
                            (n + 1).to_string(),
                            FontId::monospace(10.0),
                            theme::rgb(t.text_on_accent),
                        );
                        let tr = Rect::from_min_max(
                            pos2(th.right() + 5.0, card.top() + 3.0),
                            card.max - vec2(3.0, 3.0),
                        );
                        let p = ui.painter().with_clip_rect(tr);
                        p.text(
                            tr.min,
                            Align2::LEFT_TOP,
                            format!("{} @ {:.1}s", alt.clip, alt.t_in),
                            FontId::proportional(11.0),
                            theme::rgb(t.text),
                        );
                        let galley = ui.painter().layout(
                            alt.reason.clone(),
                            FontId::proportional(9.5),
                            theme::rgb(t.text_muted),
                            tr.width(),
                        );
                        p.galley(tr.min + vec2(0.0, 15.0), galley, theme::rgb(t.text_muted));
                        let resp = resp.on_hover_text(format!(
                            "{}\n{}{}\nClick to swap in · double-click previews",
                            alt.seg,
                            alt.reason,
                            if alt.who.is_empty() {
                                String::new()
                            } else {
                                format!("\n{}", alt.who)
                            }
                        ));
                        if resp.double_clicked() {
                            if let Some(path) = s.clip_path(&alt.clip) {
                                io.player.open_clip(
                                    &path,
                                    f64::from(alt.t_in),
                                    f64::from(alt.t_in + dur),
                                    alt.cx,
                                );
                                io.player.play();
                            }
                        } else if resp.clicked() {
                            take = Some(n);
                        }
                    }
                });
            });
    });
    if let Some(n) = take {
        super::take_alternative(state, s, io, n);
    }
}

/// The check bar: counts, the first problem, and the full list.
pub(super) fn check_bar(
    state: &mut State,
    s: &Shown<'_>,
    io: &mut Io<'_>,
    ui: &mut Ui,
    rect: Rect,
) {
    let t = s.theme;
    ui.painter()
        .rect_filled(rect, CornerRadius::same(3), theme::rgb(t.panel));
    let errors = state.problems.iter().filter(|p| super::is_error(p)).count();
    let warnings = state.problems.len() - errors;
    let clean = (0..s.plan.shots.len())
        .filter(|k| !s.issues.contains_key(k))
        .count();
    let mut jump = None;
    row(ui, rect.shrink2(vec2(6.0, 1.0)), |ui| {
        if s.project.index.is_none() {
            ui.label(
                egui::RichText::new("No shot log: the plan cannot be checked.")
                    .size(11.0)
                    .color(theme::rgb(t.text_muted)),
            );
            return;
        }
        let count = |ui: &mut Ui, glyph: &str, n: usize, color: Color32, what: &str| {
            let (r, _) = ui.allocate_exact_size(vec2(11.0, 11.0), Sense::hover());
            let (c, st) = (r.center(), Stroke::new(1.6, color));
            let p = ui.painter();
            match glyph {
                "ok" => {
                    p.line_segment([c + vec2(-4.0, 0.0), c + vec2(-1.0, 3.5)], st);
                    p.line_segment([c + vec2(-1.0, 3.5), c + vec2(4.5, -4.0)], st);
                }
                "warn" => {
                    p.add(egui::Shape::closed_line(
                        vec![c + vec2(0.0, -5.0), c + vec2(5.0, 4.5), c + vec2(-5.0, 4.5)],
                        Stroke::new(1.3, color),
                    ));
                    p.vline(c.x, (c.y - 1.5)..=(c.y + 1.5), Stroke::new(1.3, color));
                }
                _ => {
                    p.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], st);
                    p.line_segment([c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)], st);
                }
            }
            ui.label(
                egui::RichText::new(n.to_string())
                    .size(11.5)
                    .strong()
                    .color(color),
            )
            .on_hover_text(what);
        };
        count(
            ui,
            "ok",
            clean,
            theme::rgb(t.success),
            "shots that pass every rule",
        );
        count(
            ui,
            "warn",
            warnings,
            if warnings > 0 {
                theme::rgb(t.warning)
            } else {
                theme::rgb(t.text_faint)
            },
            "content rules bent (repeats, flags, captions)",
        );
        count(
            ui,
            "err",
            errors,
            if errors > 0 {
                theme::rgb(t.danger)
            } else {
                theme::rgb(t.text_faint)
            },
            "structural errors (timing, ranges, crop, drop)",
        );
        if state.problems.is_empty() {
            ui.label(
                egui::RichText::new("ready to render")
                    .size(11.0)
                    .color(theme::rgb(t.text_muted)),
            );
            return;
        }
        ui.menu_button(egui::RichText::new("Problems").size(11.0), |ui| {
            ui.set_min_width(420.0);
            egui::ScrollArea::vertical()
                .max_height(260.0)
                .show(ui, |ui| {
                    for p in &state.problems {
                        let color = if super::is_error(p) {
                            t.danger
                        } else {
                            t.warning
                        };
                        let where_ = if p.shot == 0 {
                            "plan".to_owned()
                        } else {
                            format!("shot {}", p.shot)
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format!("{where_}: {}", p.message))
                                        .size(11.0)
                                        .color(theme::rgb(color)),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            jump = Some(p.shot);
                            ui.close();
                        }
                    }
                });
        });
        if let Some(first) = state.problems.first() {
            let where_ = if first.shot == 0 {
                "plan".to_owned()
            } else {
                format!("shot {}", first.shot)
            };
            let text = format!("{where_}: {}", first.message);
            if ui
                .add(
                    egui::Label::new(
                        egui::RichText::new(text)
                            .size(11.0)
                            .color(theme::rgb(t.text_muted)),
                    )
                    .truncate()
                    .sense(Sense::click()),
                )
                .on_hover_text("Select that shot")
                .clicked()
            {
                jump = Some(first.shot);
            }
        }
    });
    if let Some(shot) = jump.filter(|&n| n > 0) {
        let k = shot - 1;
        io.select(&s.name, &BTreeSet::from([k]));
        state.anchor = Some(k);
        state.view.reveal = s.timing.get(k).map(|&(a, d)| (a, a + d));
    }
}

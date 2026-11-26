// ============================================================================
// Module:       gui::pages::plan::strips
// Description:  The rows under the tracks: the selected shot (note, framing,
//               alternatives), the alternatives strip, the check bar, and the
//               banner shown while viewing an old version.
//
// Dependencies: egui; crate::gui::pages::plan, crate::gui::ui::{icons,
//               theme, widgets}
// ============================================================================

//! The rows under the tracks.
//!
//! Each is a flat row with a hairline above it, as Selects separates the
//! parts of a panel. Nothing here is a hint line: what a key does is in the
//! control's tooltip and on the shortcut sheet.

use crate::gui::ui::hint::Hint as _;
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2,
    vec2,
};

use super::{Drag, Io, Shown, State, edit};
use crate::gui::ui::{icons, theme, widgets};
use crate::refs::{Reference, Target};

/// Lays out `add` left to right inside `rect`.
fn row<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = theme::SPACE_SM;
            add(ui)
        },
    )
    .inner
}

/// The hairline along the top of a row.
fn top_line(ui: &Ui, s: &Shown<'_>, rect: Rect) {
    ui.painter()
        .hline(rect.x_range(), rect.top() + 0.5, theme::ink_line(s.theme));
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
    super::banner(
        ui,
        theme::translucent(t.warning, 26),
        icons::HISTORY,
        theme::rgb(t.warning),
        |ui| {
            ui.label(
                egui::RichText::new(format!("Viewing v{n} · {saved} · {change} · read-only"))
                    .font(theme::font(theme::BODY_SIZE))
                    .color(theme::rgb(t.text)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
                if widgets::ghost_button(ui, t, "Back to current").clicked() {
                    state.viewing = None;
                }
                if widgets::button_with(
                    ui,
                    t,
                    widgets::ButtonKind::Outline,
                    Some(icons::ROTATE_CCW),
                    "Restore this version",
                )
                .clicked()
                    && let Some(v) = state.viewing.take()
                {
                    let mut plan = v.plan;
                    state.apply(format!("restore v{n}"), |p| {
                        // The live plan's markers survive a restore.
                        plan.comments = std::mem::take(&mut p.comments);
                        *p = plan;
                        true
                    });
                }
            });
        },
    );
}

/// The row under the tracks while shots are selected: the shot, its note
/// and framing, and what can be done with it.
pub(super) fn selection_bar(
    state: &mut State,
    s: &Shown<'_>,
    io: &mut Io<'_>,
    ui: &mut Ui,
    rect: Rect,
) {
    let t = s.theme;
    top_line(ui, s, rect);
    let inner = rect.shrink2(vec2(theme::SPACE_SM, theme::SPACE_XS));
    let muted = theme::rgb(t.text_muted);
    let actions_w = 300.0;
    let actions = Rect::from_min_max(pos2(inner.right() - actions_w, inner.top()), inner.max);
    // Reference in chat and lift, for one shot or many.
    row(ui, actions, |ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let lift = ui
                .add_enabled_ui(s.editable && s.plan.shots.len() > s.sel.len(), |ui| {
                    widgets::icon_btn(
                        ui,
                        t,
                        icons::TRASH,
                        "Lift (beats go to the shot before)",
                        "Del",
                    )
                })
                .inner;
            if lift.clicked() {
                super::lift_selected(state, s, io);
            }
            let mention = |io: &mut Io<'_>, modifier: crate::refs::Modifier| {
                for &k in &s.sel {
                    io.actions.push(crate::gui::app::Action::AddRef(
                        Reference::new(Target::Shot {
                            plan: s.name.clone(),
                            index: k + 1,
                        })
                        .with(modifier),
                    ));
                }
            };
            if widgets::icon_btn(ui, t, icons::MESSAGE_SQUARE, "Comment at this shot", "")
                .clicked()
                && let Some(&k) = s.sel.iter().next()
            {
                let at = s.timing.get(k).map_or(0.0, |&(a, _)| a);
                super::comments::add_at(state, at);
            }
            // How the chat should take them: keep, avoid, replace, more like.
            ui.menu_button("as{2026}", |ui| {
                for modifier in crate::refs::Modifier::ALL {
                    if widgets::menu_item(ui, t, None, modifier.label(), modifier.prefix(), false)
                        .clicked()
                    {
                        mention(io, modifier);
                        ui.close();
                    }
                }
            })
            .response
            .hint("Point the chat at them as keep, avoid, replace, more like this or feature (Shift+R)");
            if widgets::button_with(
                ui,
                t,
                widgets::ButtonKind::Outline,
                Some(icons::AT_SIGN),
                "Ask the chat",
            )
            .hint("Add the selected shots to the chat message, to ask about or change them (R)")
            .clicked()
            {
                mention(io, crate::refs::Modifier::About);
            }
        });
    });
    let Some(k) = s.single() else {
        let list: Vec<String> = s.sel.iter().map(|k| (k + 1).to_string()).collect();
        row(ui, inner, |ui| {
            ui.label(
                egui::RichText::new(format!("{} shots selected", s.sel.len()))
                    .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                    .color(theme::rgb(t.text)),
            );
            ui.label(
                egui::RichText::new(list.join(", "))
                    .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
                    .color(muted),
            );
        });
        return;
    };
    let Some(shot) = s.plan.shots.get(k) else {
        return;
    };
    let dur = s.timing.get(k).map_or(0.0, |x| x.1);
    let frame_w = (inner.height() - 4.0) * 16.0 / 9.0;
    let alt_w = 120.0;
    let text_rect = Rect::from_min_max(
        inner.min,
        pos2(
            actions.left() - frame_w - alt_w - theme::SPACE_SM * 3.0,
            inner.bottom(),
        ),
    );
    row(ui, text_rect, |ui| {
        ui.label(
            egui::RichText::new(format!("Shot {}", k + 1))
                .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                .color(theme::rgb(t.text)),
        );
        ui.label(
            egui::RichText::new(format!(
                "{} @ {:.2}s · {} beats · {dur:.2}s",
                shot.clip, shot.t_in, shot.beats
            ))
            .font(theme::tabular(ui.ctx(), theme::BODY_SIZE))
            .color(muted),
        );
        if !shot.who.is_empty() {
            widgets::pill(ui, t, &shot.who, widgets::Tone::Neutral);
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
        let field = Rect::from_min_size(
            pos2(
                ui.cursor().left(),
                inner.center().y - theme::CONTROL_HEIGHT / 2.0,
            ),
            vec2(ui.available_width().max(60.0), theme::CONTROL_HEIGHT),
        );
        let resp = ui
            .add_enabled_ui(s.editable, |ui| {
                widgets::field_frame(ui, t, field, buf, "Why this shot is here", Some(icons::PEN))
            })
            .inner;
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
        pos2(
            actions.left() - frame_w - alt_w - theme::SPACE_SM * 2.0,
            inner.top() + 2.0,
        ),
        vec2(frame_w, inner.height() - 4.0),
    );
    framing(state, s, io, ui, fr, k);
    let ab = Rect::from_min_size(
        pos2(actions.left() - alt_w - theme::SPACE_SM, inner.top()),
        vec2(alt_w, inner.height()),
    );
    row(ui, ab, |ui| {
        let resp = ui
            .add_enabled_ui(s.editable, |ui| {
                widgets::button_with(
                    ui,
                    t,
                    widgets::ButtonKind::Outline,
                    Some(icons::SPARKLES),
                    "Alternatives",
                )
            })
            .inner;
        if widgets::tip(
            resp,
            t,
            "Rank footage that fits this slot without breaking a rule",
            "A",
        )
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
            theme::rgb(t.raised),
        );
    } else {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS_XS),
            theme::rgb(t.raised),
        );
    }
    let crop = s.plan.aspect.crop_frac();
    let resp = ui
        .interact(
            rect,
            egui::Id::unique("plan_framing").with(k),
            Sense::click_and_drag(),
        )
        .hint(if crop < 1.0 {
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
            Stroke::new(1.5, theme::rgb(t.text)),
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
    top_line(ui, s, rect);
    let head = Rect::from_min_size(
        rect.min + vec2(theme::SPACE_SM, 2.0),
        vec2(rect.width() - theme::SPACE_SM * 2.0, theme::CONTROL_HEIGHT),
    );
    let want = s
        .project
        .index
        .as_ref()
        .map(|i| crate::alternatives::want_for(&s.plan, slot, i).label())
        .unwrap_or_default();
    let beats = s.plan.shots.get(slot).map_or(0, |sh| sh.beats);
    let mut close = false;
    row(ui, head, |ui| {
        icons::icon(ui, icons::SPARKLES, icons::SMALL, theme::rgb(t.text_muted));
        ui.label(
            egui::RichText::new(format!("Alternatives for shot {}", slot + 1))
                .font(theme::medium(ui.ctx(), theme::BODY_SIZE))
                .color(theme::rgb(t.text)),
        );
        ui.label(
            egui::RichText::new(format!("{want} · {beats} beats"))
                .font(theme::font(theme::BODY_SIZE))
                .color(theme::rgb(t.text_muted)),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::icon_btn(ui, t, icons::X, "Close alternatives", "Esc").clicked() {
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
        pos2(rect.left() + theme::SPACE_SM, head.bottom() + 2.0),
        rect.max - vec2(theme::SPACE_SM, theme::SPACE_SM),
    );
    if items.is_empty() {
        row(ui, body, |ui| {
            ui.label(
                egui::RichText::new(
                    "Nothing else fits this slot without breaking a rule. Loosen the rules or free up a clip.",
                )
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
                    ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
                    for (n, alt) in items.iter().enumerate() {
                        let (card, resp) =
                            ui.allocate_exact_size(vec2(220.0, body.height()), Sense::click());
                        let hovered = resp.hovered();
                        ui.painter().rect(
                            card,
                            CornerRadius::same(theme::RADIUS_MD),
                            if hovered {
                                theme::ink(t, theme::INK_08)
                            } else {
                                theme::ink(t, theme::INK_03)
                            },
                            theme::ink_line(t),
                            egui::StrokeKind::Inside,
                        );
                        let th_h = card.height() - 8.0;
                        let th = Rect::from_min_size(
                            card.min + vec2(4.0, 4.0),
                            vec2((th_h * ratio).min(80.0), th_h),
                        );
                        if let Some(path) = s.clip_path(&alt.clip) {
                            io.thumbs.paint(
                                ui,
                                th,
                                &path,
                                f64::from(alt.t_in),
                                alt.cx,
                                theme::rgb(t.raised),
                            );
                        }
                        let badge = Rect::from_min_size(th.min + vec2(3.0, 3.0), vec2(16.0, 16.0));
                        ui.painter().rect_filled(
                            badge,
                            CornerRadius::same(theme::RADIUS),
                            theme::popover_fill(t),
                        );
                        ui.painter().text(
                            badge.center(),
                            Align2::CENTER_CENTER,
                            (n + 1).to_string(),
                            theme::tabular(ui.ctx(), theme::TINY_SIZE),
                            theme::rgb(t.text),
                        );
                        let tr = Rect::from_min_max(
                            pos2(th.right() + theme::SPACE_SM, card.top() + 6.0),
                            card.max - vec2(6.0, 4.0),
                        );
                        let p = ui.painter().with_clip_rect(tr);
                        p.text(
                            tr.min,
                            Align2::LEFT_TOP,
                            format!("{} @ {:.1}s", alt.clip, alt.t_in),
                            theme::medium(ui.ctx(), theme::BODY_SIZE),
                            theme::rgb(t.text),
                        );
                        let galley = ui.painter().layout(
                            alt.reason.clone(),
                            theme::font(theme::SMALL_SIZE),
                            theme::rgb(t.text_muted),
                            tr.width(),
                        );
                        p.galley(tr.min + vec2(0.0, 18.0), galley, theme::rgb(t.text_muted));
                        let tip = format!(
                            "{}\n{}{}",
                            alt.seg,
                            alt.reason,
                            if alt.who.is_empty() {
                                String::new()
                            } else {
                                format!("\n{}", alt.who)
                            }
                        );
                        let resp = widgets::tip(
                            resp,
                            t,
                            &format!("{tip}\nClick swaps it in; double-click previews"),
                            &(n + 1).to_string(),
                        );
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

/// A count with its icon on a tinted pill: `tone` when there are any,
/// neutral at zero.
fn count_pill(
    ui: &mut Ui,
    s: &Shown<'_>,
    glyph: char,
    n: usize,
    color: Color32,
    tip: &str,
    sense: Sense,
) -> egui::Response {
    let t = s.theme;
    let (fill, ink) = if n > 0 {
        (color.gamma_multiply(0.15), color)
    } else {
        (theme::ink(t, theme::INK_05), theme::rgb(t.text_muted))
    };
    let galley = ui.painter().layout_no_wrap(
        n.to_string(),
        theme::tabular(ui.ctx(), theme::BODY_SIZE),
        ink,
    );
    let size = vec2(
        theme::SPACE_SM + icons::SMALL + theme::SPACE_XS + galley.size().x + theme::SPACE_SM,
        theme::COMPACT_HEIGHT,
    );
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let role = if sense.senses_click() {
        egui::Role::Button
    } else {
        egui::Role::Label
    };
    resp.widget_info(|| egui::WidgetInfo::labeled(role, true, format!("{n} {tip}")));
    if sense.senses_click() && resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(theme::RADIUS),
            theme::ink(t, theme::INK_08),
        );
    }
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
    icons::paint(
        p,
        Rect::from_min_size(
            pos2(
                rect.left() + theme::SPACE_SM,
                rect.center().y - icons::SMALL / 2.0,
            ),
            Vec2::splat(icons::SMALL),
        ),
        glyph,
        icons::SMALL,
        ink,
    );
    p.galley(
        pos2(
            rect.left() + theme::SPACE_SM + icons::SMALL + theme::SPACE_XS,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        ink,
    );
    widgets::tip(resp, t, tip, "")
}

/// The check bar: shots clean, soft and hard counts as pills (from each
/// finding's severity; accepted soft findings do not count), "Ready to
/// render" unless a hard finding stops it, then the worst finding and the
/// Findings list ([`super::findings`]), where each finding can be fixed,
/// handed to the chat or accepted. A click on the soft or hard count opens
/// the list; clicking a finding selects its shot, or both shots around a
/// seam finding's cut with the playhead on it.
pub(super) fn check_bar(
    state: &mut State,
    s: &Shown<'_>,
    io: &mut Io<'_>,
    ui: &mut Ui,
    rect: Rect,
) {
    let t = s.theme;
    top_line(ui, s, rect);
    let errors = crate::plan::hard_count(&state.problems);
    let accepted = crate::plan::accepted_count(&s.plan, &state.problems);
    let warnings = state.problems.len() - errors - accepted;
    let clean = (0..s.plan.shots.len())
        .filter(|k| !s.issues.contains_key(k))
        .count();
    // The worst open finding: hard first, each group in check order.
    let first = state
        .problems
        .iter()
        .filter(|f| !s.plan.is_accepted(f))
        .min_by_key(|f| f.severity)
        .cloned();
    let any = !state.problems.is_empty();
    let mut act: Option<super::findings::Act> = None;
    row(ui, rect.shrink2(vec2(theme::SPACE_SM, 0.0)), |ui| {
        if s.project.index.is_none() {
            icons::icon(ui, icons::INFO, icons::SMALL, theme::rgb(t.text_muted));
            ui.label(
                egui::RichText::new("Not analysed yet: the cut cannot be checked.")
                    .color(theme::rgb(t.text_muted)),
            );
            return;
        }
        ui.spacing_mut().item_spacing.x = theme::SPACE_XS;
        count_pill(
            ui,
            s,
            icons::CIRCLE_CHECK,
            clean,
            theme::rgb(t.success),
            "shots pass every rule",
            Sense::hover(),
        );
        let soft_tip = if accepted > 0 {
            format!(
                "soft: taste rules bent (repeats, flags, jump cuts, exposure and colour jumps); {accepted} accepted. Click to fix them"
            )
        } else {
            "soft: taste rules bent (repeats, flags, jump cuts, exposure and colour jumps). Click to fix them".to_owned()
        };
        let list_sense = if !any { Sense::hover() } else { Sense::click() };
        let soft = count_pill(
            ui,
            s,
            icons::TRIANGLE_ALERT,
            warnings,
            theme::rgb(t.warning),
            &soft_tip,
            list_sense,
        );
        let hard = count_pill(
            ui,
            s,
            icons::CIRCLE_X,
            errors,
            theme::rgb(t.danger),
            "hard: the plan is invalid or would render wrong (timing, ranges, crop, drop). Click to fix them",
            list_sense,
        );
        let toggle = soft.clicked() || hard.clicked();
        ui.add_space(theme::SPACE_XS);
        if errors == 0 {
            ui.label(
                egui::RichText::new("Ready to render")
                    .font(theme::font(theme::BODY_SIZE))
                    .color(theme::rgb(t.text_muted)),
            );
        }
        if !any {
            return;
        }
        let list = widgets::button_with(
            ui,
            t,
            widgets::ButtonKind::Ghost,
            Some(icons::LIST),
            "Findings",
        );
        let list = widgets::tip(
            list,
            t,
            "Every finding the check makes, hard first, with a fix where there is one",
            "",
        );
        act = super::findings::popover(state, s, &list, toggle);
        if let Some(first) = &first {
            let resp = ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("{} · {}", first.place(), first.message))
                        .font(theme::font(theme::BODY_SIZE))
                        .color(theme::rgb(t.text_muted)),
                )
                .truncate()
                .sense(Sense::click()),
            );
            let what = if first.seam.is_some() {
                "Select the shots around that cut"
            } else {
                "Select that shot"
            };
            if widgets::tip(resp, t, &format!("{what} ({})", first.id), "").clicked() {
                act = Some(super::findings::Act::Select(first.clone()));
            }
        }
    });
    if let Some(act) = act {
        super::findings::act(state, s, io, act);
    }
}

/// Selects what finding `f` is about (its shot, or both shots around its
/// cut), scrolls it into view and, for a cut, puts the playhead on it.
pub(super) fn show_finding(
    state: &mut State,
    s: &Shown<'_>,
    io: &mut Io<'_>,
    f: &crate::plan::Finding,
) {
    let Some((shots, cut)) = super::finding_target(f, s.plan.shots.len()) else {
        return;
    };
    let (Some(&lo), Some(&hi)) = (shots.first(), shots.last()) else {
        return;
    };
    io.select(&s.name, &shots);
    state.anchor = Some(lo);
    state.view.reveal = match (s.timing.get(lo), s.timing.get(hi)) {
        (Some(&(a, _)), Some(&(b, d))) => Some((a, b + d)),
        (Some(_) | None, _) => None,
    };
    if let Some(&(at, _)) = cut.and_then(|k| s.timing.get(k)) {
        io.seek(s, at);
    }
}

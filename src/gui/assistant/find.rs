// ============================================================================
// Module:       gui::assistant::find
// Description:  Find in chat: where a query matches the conversation, the
//               "2/5" counter and next / previous, the find bar that replaces
//               the header's title, and the highlight painted on matches.
//
// Dependencies: egui (LayoutJob); crate::gui::ui::{icons, keymap, theme,
//               widgets}; super (the panel's state)
// ============================================================================

//! Find in chat.
//!
//! Selects replaces the chat title with a search field, a match counter and
//! up / down buttons (`docs/research/07-selects-visual-chat-project.md`
//! §3.1). Here the same bar opens with the `chat-find` chord (Ctrl+Shift+F:
//! Ctrl+F is the workspace search) or the header's search button.
//!
//! Matching is case-insensitive for ASCII and runs over exactly the text the
//! conversation lays out (the person's bubbles and the assistant's prose), in
//! the order it is drawn, so the counter and the highlights agree: the
//! drawing code hands each laid-out message to [`Hits::mark`], which counts
//! as it goes and paints the current match stronger than the others.

use std::ops::Range;

use egui::text::{LayoutJob, LayoutSection};
use egui::{Align, Layout, RichText, Ui, Vec2};

use super::{PageCtx, State};
use crate::gui::ui::theme::{self, SPACE_XS};
use crate::gui::ui::{icons, widgets};
use crate::theme::Palette;

/// The open find bar.
#[derive(Clone, Debug, Default)]
pub(super) struct Find {
    /// What is searched for.
    pub(super) query: String,
    /// The current match, 0-based in drawing order.
    pub(super) current: usize,
    /// Give the field the keyboard next frame.
    pub(super) focus: bool,
    /// Scroll the current match into view next time it is drawn.
    pub(super) scroll: bool,
}

impl Find {
    /// A bar that takes the keyboard.
    pub(super) fn open() -> Self {
        Self {
            focus: true,
            ..Self::default()
        }
    }

    /// Moves to the next (or previous) of `total` matches, wrapping.
    pub(super) fn step(&mut self, forward: bool, total: usize) {
        if total == 0 {
            self.current = 0;
            return;
        }
        self.current = if forward {
            (self.current + 1) % total
        } else {
            (self.current + total - 1) % total
        };
        self.scroll = true;
    }
}

/// Where `query` occurs in `text`, ignoring ASCII case, without overlaps.
pub(super) fn ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    // ASCII lowercasing keeps every byte offset where it was.
    let hay = text.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = hay.get(at..).and_then(|h| h.find(&needle)) {
        let start = at + i;
        out.push(start..start + needle.len());
        at = start + needle.len();
    }
    out
}

/// How many times `query` occurs across `texts`.
pub(super) fn count(texts: &[String], query: &str) -> usize {
    texts.iter().map(|t| ranges(t, query).len()).sum()
}

/// The counter's text: `2/5`, or `0/0` with nothing found.
pub(super) fn counter(current: usize, total: usize) -> String {
    if total == 0 {
        "0/0".to_owned()
    } else {
        format!("{}/{total}", current.min(total - 1) + 1)
    }
}

/// Matches being painted during one draw of the conversation.
#[derive(Debug)]
pub(super) struct Hits {
    /// The query.
    query: String,
    /// The current match.
    current: usize,
    /// Matches painted so far this draw.
    seen: usize,
    /// Still to scroll the current match into view.
    pub(super) scroll: bool,
    /// A match's fill.
    fill: egui::Color32,
    /// The current match's fill.
    strong: egui::Color32,
}

impl Hits {
    /// The hits of the open find bar, if it has a query.
    pub(super) fn of(find: Option<&Find>, theme: &Palette) -> Option<Self> {
        let find = find.filter(|f| !f.query.trim().is_empty())?;
        Some(Self {
            query: find.query.clone(),
            current: find.current,
            seen: 0,
            scroll: find.scroll,
            fill: theme::translucent(theme.accent, 70),
            strong: theme::translucent(theme.accent, 170),
        })
    }

    /// Paints the matches in `job`'s text; returns whether the current
    /// match is among them.
    pub(super) fn mark(&mut self, job: &mut LayoutJob) -> bool {
        let found = ranges(&job.text, &self.query);
        if found.is_empty() {
            return false;
        }
        let first = self.seen;
        let mut sections = Vec::with_capacity(job.sections.len() + found.len() * 2);
        for s in job.sections.drain(..) {
            let span = usize::from(s.byte_range.start)..usize::from(s.byte_range.end);
            let mut cuts: Vec<usize> = found
                .iter()
                .flat_map(|r| [r.start, r.end])
                .filter(|p| *p > span.start && *p < span.end)
                .collect();
            cuts.sort_unstable();
            cuts.dedup();
            cuts.push(span.end);
            let mut at = span.start;
            for (n, end) in cuts.into_iter().enumerate() {
                let mut format = s.format.clone();
                if let Some(k) = found.iter().position(|r| r.start <= at && at < r.end) {
                    format.background = if first + k == self.current {
                        self.strong
                    } else {
                        self.fill
                    };
                }
                sections.push(LayoutSection {
                    leading_space: if n == 0 { s.leading_space } else { 0.0 },
                    byte_range: at.into()..end.into(),
                    format,
                });
                at = end;
            }
        }
        job.sections = sections;
        self.seen += found.len();
        (first..self.seen).contains(&self.current)
    }

    /// Scrolls `rect` into view if it holds the current match and the bar
    /// asked for it.
    pub(super) fn reveal(&mut self, ui: &Ui, holds_current: bool, rect: egui::Rect) {
        if holds_current && self.scroll {
            ui.scroll_to_rect(rect, Some(Align::Center));
            self.scroll = false;
        }
    }
}

/// The find bar in the header: the field, the counter, previous / next and
/// close. Enter goes to the next match, Shift+Enter to the previous one,
/// Escape closes.
pub(super) fn bar(ctx: &PageCtx<'_>, state: &mut State, ui: &mut Ui) {
    let theme = ctx.theme;
    let total = state
        .chat
        .as_ref()
        .zip(state.find.as_ref())
        .map_or(0, |(chat, f)| {
            count(&super::find_texts(&chat.log), &f.query)
        });
    let Some(find) = state.find.as_mut() else {
        return;
    };
    let mut close = false;
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if widgets::icon_toggle_sized(ui, theme, icons::X, "Close find", "Escape", false, 24.0)
            .clicked()
        {
            close = true;
        }
        let enabled = total > 0;
        let next = ui
            .add_enabled_ui(enabled, |ui| {
                widgets::icon_toggle_sized(
                    ui,
                    theme,
                    icons::CHEVRON_DOWN,
                    "Next match",
                    "Enter",
                    false,
                    24.0,
                )
            })
            .inner;
        if widgets::why_disabled(next, "No matches").clicked() {
            find.step(true, total);
        }
        let previous = ui
            .add_enabled_ui(enabled, |ui| {
                widgets::icon_toggle_sized(
                    ui,
                    theme,
                    icons::CHEVRON_UP,
                    "Previous match",
                    "Shift+Enter",
                    false,
                    24.0,
                )
            })
            .inner;
        if widgets::why_disabled(previous, "No matches").clicked() {
            find.step(false, total);
        }
        ui.add_space(SPACE_XS);
        ui.label(
            RichText::new(counter(find.current, total))
                .font(theme::tabular(ui.ctx(), theme::SMALL_SIZE))
                .color(theme::rgb(theme.text_muted)),
        );
        ui.add_space(SPACE_XS);
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            let size = Vec2::new(ui.available_width().max(60.0), 24.0);
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let field = widgets::field_frame(
                ui,
                theme,
                rect,
                &mut find.query,
                "Find in chat",
                Some(icons::SEARCH),
            );
            if find.focus {
                field.request_focus();
                find.focus = false;
            }
            if field.changed() {
                find.current = 0;
                find.scroll = true;
            }
            if field.lost_focus() {
                let (enter, shift, escape) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::Enter),
                        i.modifiers.shift,
                        i.key_pressed(egui::Key::Escape),
                    )
                });
                if escape {
                    close = true;
                } else if enter {
                    find.step(!shift, total);
                    find.focus = true;
                }
            }
        });
    });
    if close {
        state.find = None;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn matches_ignore_case_and_count_across_messages() {
        assert_eq!(
            super::ranges("Drop it on the DROP, drop", "drop"),
            vec![0..4, 15..19, 21..25],
            "three, any case"
        );
        assert!(
            super::ranges("anything", "  ").is_empty(),
            "blank finds nothing"
        );
        let texts = vec![
            "the drop".to_owned(),
            "no".to_owned(),
            "drop drop".to_owned(),
        ];
        assert_eq!(super::count(&texts, "DROP"), 3, "counted across messages");
        assert_eq!(super::counter(0, 3), "1/3", "first of three");
        assert_eq!(super::counter(0, 0), "0/0", "nothing found");
        let mut find = super::Find::open();
        find.step(false, 3);
        assert_eq!(find.current, 2, "previous wraps to the last");
        find.step(true, 3);
        assert_eq!(find.current, 0, "next wraps to the first");
    }

    #[test]
    fn marking_splits_sections_at_matches() -> anyhow::Result<()> {
        let mut job = egui::text::LayoutJob::default();
        job.append("a drop and ", 0.0, egui::TextFormat::default());
        job.append("drop", 0.0, egui::TextFormat::default());
        let find = super::Find {
            query: "drop".into(),
            current: 1,
            ..super::Find::default()
        };
        let theme = crate::theme::signature();
        let mut hits = super::Hits::of(Some(&find), &theme)
            .ok_or_else(|| anyhow::anyhow!("a query makes hits"))?;
        assert!(hits.mark(&mut job), "the current match is in this job");
        let marked = job
            .sections
            .iter()
            .filter(|s| s.format.background != egui::Color32::TRANSPARENT)
            .count();
        assert_eq!(marked, 2, "both matches painted");
        assert_eq!(job.sections.len(), 4, "split around the first match");
        Ok(())
    }
}

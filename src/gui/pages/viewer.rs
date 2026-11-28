// ============================================================================
// Module:       gui::pages::viewer
// Description:  The viewer in the workspace centre, as Selects' Preview: the
//               shared player letterboxed on the panel, safe-area guides, an
//               info overlay, the transport with a settings menu, and the
//               viewer's keys.
//
// Dependencies: egui; crate::gui::{pages, player, ui}, crate::refs
// ============================================================================

//! The viewer region.
//!
//! No header row (Selects has none): the picture fills the panel above a
//! 48 px transport whose top edge is the scrub bar. The framing options
//! (aspect 16:9 / 9:16 / 4:5, guides, the info overlay, preview quality and
//! the volume) live in the gear menu at the right of the transport.
//! Narrow aspects show the crop around the source's `cx`; *Full frame*
//! shows the whole frame with the outside of the crop dimmed instead.
//! Options persist per night in `ave.toml` under `[pages.viewer]`. The top
//! 48 px of both corners stay free: the folded browser's and inspector's
//! glass buttons float there.
//!
//! Keys (when no text field has focus): the player's transport keys (Space,
//! J/K/L, ←/→, Shift+←/→, I, O, Alt+X, `/`, Ctrl+L), plus `Alt+R` to
//! reference the marked range (or the frame under the playhead) in chat and
//! `Ctrl+Shift+I` for the info overlay. A complete I/O range also becomes
//! the app-wide selection, so the inspector and `R` see it.

mod overlays;

use crate::gui::ui::hint::Hint as _;
use std::path::Path;

use super::PageCtx;
use crate::gui::app::Action;
use crate::gui::player::{self, Framing, Player, Source, clock, transport};
use crate::gui::ui::{icons, keymap, theme, widgets};
use crate::refs::{Reference, Target};
use crate::theme::Palette;

/// The output aspect the viewer frames for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Aspect {
    /// 16:9, the master.
    #[default]
    Wide,
    /// 9:16, reels and stories.
    Vertical,
    /// 4:5, the feed.
    Feed,
}

impl Aspect {
    /// Every aspect, in menu order.
    const ALL: [Self; 3] = [Self::Wide, Self::Vertical, Self::Feed];

    /// Width over height.
    fn ratio(self) -> f32 {
        match self {
            Self::Wide => 16.0 / 9.0,
            Self::Vertical => 9.0 / 16.0,
            Self::Feed => 4.0 / 5.0,
        }
    }

    /// Menu label (also the settings value).
    fn label(self) -> &'static str {
        match self {
            Self::Wide => "16:9",
            Self::Vertical => "9:16",
            Self::Feed => "4:5",
        }
    }

    /// The icon beside the label in the menu.
    fn icon(self) -> char {
        match self {
            Self::Wide => icons::LANDSCAPE,
            Self::Vertical => icons::SMARTPHONE,
            Self::Feed => icons::PORTRAIT,
        }
    }

    /// What the aspect is for.
    fn hint(self) -> &'static str {
        match self {
            Self::Wide => "The master",
            Self::Vertical => "Reels and stories",
            Self::Feed => "The feed",
        }
    }

    /// Parses a settings value.
    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.label() == text)
    }
}

/// Preview widths offered, pixels.
const QUALITIES: [(u32, &str); 4] = [
    (640, "Draft 640"),
    (960, "Preview 960"),
    (1280, "High 1280"),
    (1920, "Full 1920"),
];

/// The viewer's own state.
#[derive(Debug, Default)]
pub(crate) struct State {
    aspect: Aspect,
    /// Title/action-safe outlines.
    safe: bool,
    /// The 9:16 interface zones.
    social: bool,
    /// Whole frame with the crop outlined.
    ghost: bool,
    /// The info overlay.
    info: bool,
    /// Decoded preview width.
    width: u32,
    /// The night whose options are loaded.
    loaded_for: Option<std::path::PathBuf>,
    /// The marked range last pushed into the selection.
    selected: Option<(f64, f64)>,
}

impl State {
    /// Reads the options from the night's settings (once per night).
    fn load(&mut self, ctx: &PageCtx<'_>) {
        let work = ctx.project.as_ref().map(|p| p.work.clone());
        if work == self.loaded_for {
            return;
        }
        self.loaded_for = work;
        let table = ctx
            .project
            .as_ref()
            .and_then(|p| p.settings.pages.get("viewer"));
        let flag = |key: &str, default: bool| {
            table
                .and_then(|t| t.get(key))
                .and_then(toml::Value::as_bool)
                .unwrap_or(default)
        };
        self.aspect = table
            .and_then(|t| t.get("aspect"))
            .and_then(toml::Value::as_str)
            .and_then(Aspect::parse)
            .unwrap_or_default();
        self.safe = flag("safe", false);
        self.social = flag("social", true);
        self.ghost = flag("ghost", false);
        self.info = flag("info", false);
        self.width = table
            .and_then(|t| t.get("preview_width"))
            .and_then(toml::Value::as_integer)
            .and_then(|w| u32::try_from(w).ok())
            .unwrap_or(player::PREVIEW_WIDTH);
    }

    /// Writes the options back to `ave.toml`.
    fn save(&self, ctx: &mut PageCtx<'_>) {
        let Some(project) = ctx.project.as_mut() else {
            return;
        };
        let table = project
            .settings
            .pages
            .entry("viewer".to_owned())
            .or_default();
        table.insert("aspect".into(), self.aspect.label().into());
        table.insert("safe".into(), self.safe.into());
        table.insert("social".into(), self.social.into());
        table.insert("ghost".into(), self.ghost.into());
        table.insert("info".into(), self.info.into());
        table.insert("preview_width".into(), i64::from(self.width).into());
        if let Err(err) = project.save_settings() {
            log::warn!("could not save the viewer options: {err:#}");
        }
    }
}

/// Room left under the picture for the scrub bar, which overlaps it.
const SCRUB_ROOM: f32 = theme::SPACE_XS;

/// Draws the viewer into the centre.
pub(crate) fn draw(ctx: &mut PageCtx<'_>, state: &mut State, ui: &mut egui::Ui) {
    state.load(ctx);
    if state.width == 0 {
        state.width = player::PREVIEW_WIDTH;
    }
    ctx.player.set_preview_width(state.width);
    let size = egui::vec2(
        ui.available_width(),
        (ui.available_height() - transport::reserved(ui) - SCRUB_ROOM).max(80.0),
    );
    // A frame still decoding shows as the video surface, forced dark.
    let surface = theme::surface(ui.ctx(), theme::Surface::Canvas);
    let framing = Framing {
        aspect: state.aspect.ratio(),
        ghost: state.ghost && state.aspect != Aspect::Wide,
    };
    let shown = ctx
        .player
        .show_framed(ui, ctx.thumbs, size, framing, surface);
    let stage = shown.response.rect;
    let has_picture = matches!(
        ctx.player.source(),
        Source::Clip { .. } | Source::Sequence { .. }
    );
    let painter = ui.painter_at(stage);
    if has_picture {
        if state.safe {
            overlays::paint_safe(&painter, shown.frame);
        }
        if state.social && state.aspect == Aspect::Vertical {
            overlays::paint_social(&painter, shown.frame);
        }
        if state.width < player::PREVIEW_WIDTH {
            overlays::paint_reduced(&painter, shown.frame);
        }
    }
    if state.info {
        overlays::paint_info(&painter, shown.frame, &info_lines(ctx.player));
    }
    if matches!(ctx.player.source(), Source::Empty) {
        let mut empty = ui.new_child(egui::UiBuilder::new().max_rect(stage));
        widgets::empty_state(
            &mut empty,
            ctx.theme,
            icons::CLAPPERBOARD,
            "Nothing to play",
            "Select a shot, a cut or a stretch of music to play it here.",
            |_| {},
        );
    }
    ui.add_space(SCRUB_ROOM);
    let palette = ctx.theme;
    let mut changed = false;
    transport::bar(ctx.player, ui, palette, |ui, player| {
        changed = settings(ui, palette, state, player);
    });
    if changed {
        state.save(ctx);
    }
    if ctx.keys_free(ui) {
        ctx.player.handle_keys(ui, ctx.keymap);
        let preset = ctx.keymap;
        let (alt_r, info) = ui.input(|i| {
            (
                keymap::pressed(i, "mention-range", preset),
                keymap::pressed(i, "info-overlay", preset),
            )
        });
        if alt_r && let Some(reference) = reference_here(ctx.player) {
            ctx.actions.push(Action::AddRef(reference));
        }
        if info {
            state.info = !state.info;
            state.save(ctx);
        }
    }
    sync_selection(ctx, state);
}

/// The gear at the right of the transport and its menu: aspect, guides,
/// the info overlay, preview quality and volume, the current ones checked.
/// Returns whether an option that persists changed.
fn settings(ui: &mut egui::Ui, theme: &Palette, state: &mut State, p: &mut Player) -> bool {
    let id = ui.scope_id().with("viewer-settings");
    let open = widgets::menu_is_open(ui.ctx(), id);
    let response = transport::button(ui, theme, icons::SETTINGS, "Preview settings", "", open);
    let mut changed = false;
    // The response is not needed: drawing it is the point.
    let _ = widgets::menu(&response)
        .id(id)
        .width(240.0)
        .gap(theme::SPACE_XS)
        .align(egui::RectAlign::TOP_END)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            heading(ui, theme, "Frame");
            for aspect in Aspect::ALL {
                let on = state.aspect == aspect;
                if widgets::menu_item(ui, theme, Some(aspect.icon()), aspect.label(), "", on)
                    .hint(aspect.hint())
                    .clicked()
                    && !on
                {
                    state.aspect = aspect;
                    changed = true;
                }
            }
            widgets::menu_separator(ui, theme);
            heading(ui, theme, "Guides");
            changed |= check(ui, theme, "Safe margins", "", &mut state.safe)
                .hint("Title and action safe areas")
                .clicked();
            if state.aspect == Aspect::Vertical {
                changed |= check(ui, theme, "UI zones", "", &mut state.social)
                    .hint("Where TikTok and Reels draw over a vertical video")
                    .clicked();
            }
            if state.aspect != Aspect::Wide {
                changed |= check(ui, theme, "Full frame", "", &mut state.ghost)
                    .hint("Show the whole frame with the crop outlined")
                    .clicked();
            }
            changed |= check(ui, theme, "Info overlay", "Ctrl+Shift+I", &mut state.info).clicked();
            widgets::menu_separator(ui, theme);
            heading(ui, theme, "Preview quality");
            for (width, label) in QUALITIES {
                let on = state.width == width;
                if widgets::menu_item(ui, theme, None, label, "", on)
                    .hint("Decode width of the preview")
                    .clicked()
                    && !on
                {
                    state.width = width;
                    changed = true;
                }
            }
            widgets::menu_separator(ui, theme);
            transport::volume_rows(ui, theme, p);
        });
    changed
}

/// A menu section's label, inset to line up with the items' text.
fn heading(ui: &mut egui::Ui, theme: &Palette, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(theme::SPACE_SM);
        widgets::menu_heading(ui, theme, text);
    });
}

/// A menu item that flips `on` when clicked, checked while on.
fn check(
    ui: &mut egui::Ui,
    theme: &Palette,
    label: &str,
    keys: &str,
    on: &mut bool,
) -> egui::Response {
    let response = widgets::menu_item(ui, theme, None, label, keys, *on);
    if response.clicked() {
        *on = !*on;
    }
    response
}

/// What is loaded, for the info overlay.
fn source_label(player: &Player) -> String {
    match player.source() {
        Source::Empty => "Viewer".to_owned(),
        Source::Clip { path, .. } => clip_id(path),
        Source::Sequence { shots, music } => {
            let shot = player.sequence_shot().map_or(0, |(i, _)| i + 1);
            let over = if music.is_some() {
                " · over music"
            } else {
                ""
            };
            format!("Cut · shot {shot} of {}{over}", shots.len())
        }
        Source::Audio { path, .. } => format!("Music · {}", clip_id(path)),
    }
}

/// The info overlay's lines.
fn info_lines(player: &Player) -> Vec<String> {
    let fps = player.fps();
    let mut lines = vec![source_label(player)];
    if let Some(here) = player.here() {
        lines.push(format!("clip   {}", clip_id(&here.path)));
        lines.push(format!(
            "source {}  {:.3}s",
            clock::timecode(here.file_t, fps),
            here.file_t
        ));
        if matches!(player.source(), Source::Sequence { .. }) {
            lines.push(format!("shot   {} / {}", here.seg + 1, here.segs));
        }
    } else if let Source::Audio { path, t0, .. } = player.source() {
        lines.push(format!("track  {}", clip_id(path)));
        lines.push(format!("at     {:.3}s", t0 + player.position()));
    }
    lines.push(format!("fps    {fps:.3}"));
    if let Some((i, o)) = player.marked() {
        lines.push(format!("marked {:.2}s", o - i));
    }
    lines
}

/// A clip id: the file stem (the index names clips by their stems).
fn clip_id(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// What `Alt+R` references: the marked range, else the playhead frame.
fn reference_here(player: &Player) -> Option<Reference> {
    target_here(player).map(Reference::new)
}

/// The marked range as a clip range (clamped to the in-mark's shot for a
/// cut), else the frame under the playhead.
fn target_here(player: &Player) -> Option<Target> {
    if let Some(range) = marked_target(player) {
        return Some(range);
    }
    let here = player.here()?;
    Some(Target::Frame {
        clip: clip_id(&here.path),
        t: here.file_t as f32,
    })
}

/// The marked range as a `Range` target, when both marks are set.
fn marked_target(player: &Player) -> Option<Target> {
    let (i, o) = player.marked()?;
    let a = player.here_at(i)?;
    let b = player.here_at(o)?;
    let t1 = if a.seg == b.seg { b.file_t } else { a.seg_out };
    Some(Target::Range {
        clip: clip_id(&a.path),
        t0: a.file_t as f32,
        t1: t1 as f32,
    })
}

/// A newly completed I/O range replaces the app-wide selection.
fn sync_selection(ctx: &mut PageCtx<'_>, state: &mut State) {
    let marked = ctx.player.marked();
    if marked == state.selected {
        return;
    }
    state.selected = marked;
    if marked.is_some()
        && let Some(target) = marked_target(ctx.player)
    {
        *ctx.selection = vec![Reference::new(target)];
    }
}

#[cfg(test)]
mod tests {
    use super::{Aspect, clip_id, target_here};
    use crate::gui::player::{Player, SeqShot};
    use crate::refs::Target;
    use std::path::Path;

    #[test]
    fn aspects_round_trip_through_settings() -> anyhow::Result<()> {
        for a in Aspect::ALL {
            assert_eq!(Aspect::parse(a.label()), Some(a), "{a:?}");
        }
        assert!(Aspect::Vertical.ratio() < 1.0, "9:16 is narrow");
        Ok(())
    }

    #[test]
    fn references_are_ranges_with_marks_and_frames_without() -> anyhow::Result<()> {
        let mut p = Player::default();
        p.open_clip(Path::new("C:/night/MVI_7925.MOV"), 10.0, 20.0, 0.5);
        p.seek(2.0);
        let Some(Target::Frame { clip, t }) = target_here(&p) else {
            anyhow::bail!("no frame target");
        };
        assert_eq!(clip, "MVI_7925", "clip id is the file stem");
        assert!((t - 12.0).abs() < 1e-4, "file time: {t}");
        p.mark_in();
        p.seek(5.0);
        p.mark_out();
        let Some(Target::Range { t0, t1, .. }) = target_here(&p) else {
            anyhow::bail!("no range target");
        };
        assert!(
            (t0 - 12.0).abs() < 1e-4 && (t1 - 15.0).abs() < 1e-4,
            "{t0}-{t1}"
        );
        Ok(())
    }

    #[test]
    fn a_range_across_shots_stops_at_the_first_shots_end() -> anyhow::Result<()> {
        let shot = |name: &str, t_in| SeqShot {
            path: name.into(),
            t_in,
            dur: 2.0,
            cx: 0.5,
        };
        let mut p = Player::default();
        p.open_sequence(vec![shot("A.MOV", 30.0), shot("B.MOV", 5.0)], None);
        p.seek(1.0);
        p.mark_in();
        p.seek(3.0);
        p.mark_out();
        let Some(Target::Range { clip, t0, t1 }) = target_here(&p) else {
            anyhow::bail!("no range target");
        };
        assert_eq!(clip, "A", "the in-mark's shot");
        assert!(
            (t0 - 31.0).abs() < 1e-4 && (t1 - 32.0).abs() < 1e-4,
            "{t0}-{t1}"
        );
        assert_eq!(clip_id(Path::new("x/y.mov")), "y", "stem");
        Ok(())
    }
}

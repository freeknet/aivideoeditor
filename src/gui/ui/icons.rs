// ============================================================================
// Module:       gui::ui::icons
// Description:  The Lucide icon font by name: a constant per icon the app uses,
//               and the helpers that draw one on its own or inside text.
//
// Dependencies: egui; crate::gui::font (the icon family), super::theme
// ============================================================================

//! Lucide icons, by name.
//!
//! Selects draws every icon from [Lucide](https://lucide.dev); so does
//! this app, from the icon font bundled in `assets/fonts/Lucide.ttf`
//! (`lucide-static` 1.60.0, ISC; see `crate::gui::font`). Each constant
//! below is that version's codepoint for the icon of the same name, read
//! from the package's `font/codepoints.json`. **Use these, never a Unicode
//! symbol** (`▶ ✕ ⚙ ▾`): a symbol is drawn by whatever system font has
//! it, at its own weight and size, and looks pasted in.
//!
//! An icon is a `char`, so it composes with text:
//!
//! ```ignore
//! ui.label(format!("{} Export", icons::DOWNLOAD)); // inline, at text size
//! icons::icon(ui, icons::PLAY, icons::SIZE, color);  // on its own, 16 px
//! ```
//!
//! Lucide sits in every text family behind the text faces, so the first
//! form works in any label; [`icon`] and [`paint`] draw from the family
//! with Lucide first, centred in a square, which is what a button wants.
//!
//! To add an icon: find its name at lucide.dev, look its codepoint up in
//! `https://unpkg.com/lucide-static@1.60.0/font/codepoints.json`, add a
//! constant here (alphabetical) and a line to `docs/DESIGN.md`.

use super::theme;
use egui::{Color32, Painter, Rect, Response, Sense, Ui, Vec2};

/// The size an icon is drawn at beside 12 px text, and in a 28 px button.
pub const SIZE: f32 = 16.0;

/// The size of a small icon: inside a chip, a menu row, a 20 px control.
pub const SMALL: f32 = 14.0;

/// The size of an empty state's or a feature card's icon.
pub const LARGE: f32 = 24.0;

/// `activity`: a pulse line.
pub const ACTIVITY: char = '\u{e038}';
/// `arrow-up`: send (the composer's send button).
pub const ARROW_UP: char = '\u{e04a}';
/// `arrow-left-right`: swap, slip.
pub const ARROW_LEFT_RIGHT: char = '\u{e24a}';
/// `arrow-up-down`: sort.
pub const ARROW_UP_DOWN: char = '\u{e37d}';
/// `at-sign`: mention.
pub const AT_SIGN: char = '\u{e04e}';
/// `audio-lines`: an audio track, music.
pub const AUDIO_LINES: char = '\u{e55a}';
/// `audio-waveform`: a waveform.
pub const AUDIO_WAVEFORM: char = '\u{e55b}';
/// `bookmark`.
pub const BOOKMARK: char = '\u{e060}';
/// `bot`: the assistant.
pub const BOT: char = '\u{e1bb}';
/// `captions`: captions, the caption track.
pub const CAPTIONS: char = '\u{e3a4}';
/// `check`: done, the chosen option.
pub const CHECK: char = '\u{e06c}';
/// `chevron-down`: a menu or a section opens downwards.
pub const CHEVRON_DOWN: char = '\u{e06d}';
/// `chevron-left`.
pub const CHEVRON_LEFT: char = '\u{e06e}';
/// `chevron-right`: a submenu, a collapsed section.
pub const CHEVRON_RIGHT: char = '\u{e06f}';
/// `chevron-up`.
pub const CHEVRON_UP: char = '\u{e070}';
/// `chevrons-up-down`: a select.
pub const CHEVRONS_UP_DOWN: char = '\u{e211}';
/// `circle`: an empty status dot.
pub const CIRCLE: char = '\u{e076}';
/// `circle-alert`: an error.
pub const CIRCLE_ALERT: char = '\u{e077}';
/// `circle-check`: passed.
pub const CIRCLE_CHECK: char = '\u{e226}';
/// `circle-dot`: recording, the current item.
pub const CIRCLE_DOT: char = '\u{e345}';
/// `circle-play`.
pub const CIRCLE_PLAY: char = '\u{e080}';
/// `circle-x`: failed.
pub const CIRCLE_X: char = '\u{e084}';
/// `clapperboard`: a take, a scene.
pub const CLAPPERBOARD: char = '\u{e29b}';
/// `clock`: duration.
pub const CLOCK: char = '\u{e087}';
/// `columns-2`: side by side, compare.
pub const COLUMNS: char = '\u{e098}';
/// `columns-3`: three columns.
pub const COLUMNS_3: char = '\u{e099}';
/// `command`: the command palette.
pub const COMMAND: char = '\u{e09a}';
/// `copy`.
pub const COPY: char = '\u{e09e}';
/// `crop`: aspect, reframe.
pub const CROP: char = '\u{e0ab}';
/// `download`: export, save a file.
pub const DOWNLOAD: char = '\u{e0b2}';
/// `ellipsis`: more actions.
pub const ELLIPSIS: char = '\u{e0b6}';
/// `ellipsis-vertical`: more actions, in a narrow column.
pub const ELLIPSIS_VERTICAL: char = '\u{e0b7}';
/// `external-link`: opens outside the app.
pub const EXTERNAL_LINK: char = '\u{e0b9}';
/// `eye`: visible, preview.
pub const EYE: char = '\u{e0ba}';
/// `eye-off`: hidden.
pub const EYE_OFF: char = '\u{e0bb}';
/// `file-text`: a document, a script.
pub const FILE_TEXT: char = '\u{e0cc}';
/// `file-video`: a clip on disk.
pub const FILE_VIDEO: char = '\u{e321}';
/// `film`: shots, footage.
pub const FILM: char = '\u{e0d0}';
/// `filter`.
pub const FILTER: char = '\u{e0dc}';
/// `flag`: a check flag, a marker.
pub const FLAG: char = '\u{e0d1}';
/// `flame`: fire, the hero shot.
pub const FLAME: char = '\u{e0d2}';
/// `folder`: a night, a project.
pub const FOLDER: char = '\u{e0d7}';
/// `folder-open`: open a night.
pub const FOLDER_OPEN: char = '\u{e247}';
/// `gauge`: quality, a score.
pub const GAUGE: char = '\u{e1bf}';
/// `git-compare-arrows`: compare two versions.
pub const COMPARE: char = '\u{e553}';
/// `grid-3x3`: a grid view.
pub const GRID: char = '\u{e0e9}';
/// `hand`: the hand tool.
pub const HAND: char = '\u{e1d7}';
/// `headphones`: solo a track.
pub const HEADPHONES: char = '\u{e0f1}';
/// `heart`.
pub const HEART: char = '\u{e0f2}';
/// `history`: versions, recent.
pub const HISTORY: char = '\u{e1f5}';
/// `house`: home.
pub const HOME: char = '\u{e0f5}';
/// `image`: a frame, a still.
pub const IMAGE: char = '\u{e0f6}';
/// `info`.
pub const INFO: char = '\u{e0f9}';
/// `keyboard`: shortcuts.
pub const KEYBOARD: char = '\u{e284}';
/// `layers`: tracks, a stack.
pub const LAYERS: char = '\u{e529}';
/// `layout-grid`: a card grid.
pub const LAYOUT_GRID: char = '\u{e0ff}';
/// `list`: a list view.
pub const LIST: char = '\u{e106}';
/// `list-filter`: filter a list.
pub const LIST_FILTER: char = '\u{e460}';
/// `loader-circle`: working (spin it).
pub const LOADER: char = '\u{e10a}';
/// `lock`: locked.
pub const LOCK: char = '\u{e10b}';
/// `lock-open`: unlocked.
pub const LOCK_OPEN: char = '\u{e10c}';
/// `magnet`: snapping.
pub const MAGNET: char = '\u{e2b5}';
/// `maximize`: full screen.
pub const MAXIMIZE: char = '\u{e112}';
/// `maximize-2`: expand.
pub const MAXIMIZE_2: char = '\u{e113}';
/// `menu`: the application menu.
pub const MENU: char = '\u{e115}';
/// `message-square`: chat.
pub const MESSAGE_SQUARE: char = '\u{e117}';
/// `message-square-text`: a comment.
pub const MESSAGE_SQUARE_TEXT: char = '\u{e575}';
/// `minimize-2`: collapse.
pub const MINIMIZE_2: char = '\u{e11b}';
/// `minus`: remove, zoom out a step.
pub const MINUS: char = '\u{e11c}';
/// `monitor`: the viewer, 16:9.
pub const MONITOR: char = '\u{e11d}';
/// `moon`: a dark theme.
pub const MOON: char = '\u{e11e}';
/// `mouse-pointer-2`: the select tool.
pub const POINTER: char = '\u{e1c3}';
/// `move-horizontal`: roll, trim.
pub const MOVE_HORIZONTAL: char = '\u{e1c6}';
/// `music`: music, the soundtrack.
pub const MUSIC: char = '\u{e122}';
/// `panel-bottom`: the timeline.
pub const PANEL_BOTTOM: char = '\u{e42c}';
/// `panel-bottom-close`: fold the timeline.
pub const PANEL_BOTTOM_CLOSE: char = '\u{e42d}';
/// `panel-bottom-open`: unfold the timeline.
pub const PANEL_BOTTOM_OPEN: char = '\u{e42f}';
/// `panel-left`: a left panel (the browser).
pub const PANEL_LEFT: char = '\u{e12a}';
/// `panel-left-close`: fold a left panel.
pub const PANEL_LEFT_CLOSE: char = '\u{e21c}';
/// `panel-left-open`: unfold a left panel.
pub const PANEL_LEFT_OPEN: char = '\u{e21d}';
/// `panel-right`: the inspector.
pub const PANEL_RIGHT: char = '\u{e431}';
/// `panel-right-close`: fold the inspector.
pub const PANEL_RIGHT_CLOSE: char = '\u{e432}';
/// `panel-right-open`: unfold the inspector.
pub const PANEL_RIGHT_OPEN: char = '\u{e434}';
/// `panels-left-bottom`: the view-mode menu.
pub const PANELS_LEFT_BOTTOM: char = '\u{e12b}';
/// `paperclip`: attach.
pub const PAPERCLIP: char = '\u{e12d}';
/// `pause`.
pub const PAUSE: char = '\u{e12e}';
/// `pen`: rename (the top bar's title).
pub const PEN: char = '\u{e12f}';
/// `pencil`: edit, rename.
pub const PENCIL: char = '\u{e1f9}';
/// `pin`.
pub const PIN: char = '\u{e259}';
/// `play`.
pub const PLAY: char = '\u{e13c}';
/// `plus`: add, new.
pub const PLUS: char = '\u{e13d}';
/// `ratio`: aspect ratio.
pub const RATIO: char = '\u{e4e8}';
/// `rectangle-horizontal`: a 16:9 frame.
pub const LANDSCAPE: char = '\u{e376}';
/// `rectangle-vertical`: a 9:16 frame.
pub const PORTRAIT: char = '\u{e377}';
/// `redo-2`.
pub const REDO: char = '\u{e2a0}';
/// `refresh-cw`: reload, re-run.
pub const REFRESH: char = '\u{e145}';
/// `repeat`: loop playback.
pub const REPEAT: char = '\u{e146}';
/// `rotate-ccw`: reset.
pub const ROTATE_CCW: char = '\u{e148}';
/// `scissors`: blade, cut.
pub const SCISSORS: char = '\u{e14e}';
/// `search`.
pub const SEARCH: char = '\u{e151}';
/// `send`: send a message.
pub const SEND: char = '\u{e152}';
/// `send-horizontal`.
pub const SEND_HORIZONTAL: char = '\u{e4f2}';
/// `settings`: a settings page.
pub const SETTINGS: char = '\u{e154}';
/// `settings-2`: preferences (the top bar's button, as in Selects).
pub const SETTINGS_2: char = '\u{e245}';
/// `share`: hand off.
pub const SHARE: char = '\u{e155}';
/// `skip-back`: to the start, the previous shot.
pub const SKIP_BACK: char = '\u{e15f}';
/// `skip-forward`: to the end, the next shot.
pub const SKIP_FORWARD: char = '\u{e160}';
/// `sliders-horizontal`: adjust, options.
pub const SLIDERS: char = '\u{e29a}';
/// `smartphone`: a vertical reel.
pub const SMARTPHONE: char = '\u{e163}';
/// `sparkle`: one spark.
pub const SPARKLE: char = '\u{e47e}';
/// `sparkles`: AI, suggestions.
pub const SPARKLES: char = '\u{e412}';
/// `split`: split a clip.
pub const SPLIT: char = '\u{e440}';
/// `square`: stop (the composer's send button while a reply streams).
pub const SQUARE: char = '\u{e167}';
/// `star`: a pick, a rating.
pub const STAR: char = '\u{e176}';
/// `step-back`: one frame back.
pub const STEP_BACK: char = '\u{e3e9}';
/// `step-forward`: one frame on.
pub const STEP_FORWARD: char = '\u{e3ea}';
/// `sun`: a light theme.
pub const SUN: char = '\u{e178}';
/// `tag`: a label.
pub const TAG: char = '\u{e17f}';
/// `thumbs-down`: reject.
pub const THUMBS_DOWN: char = '\u{e189}';
/// `thumbs-up`: keep.
pub const THUMBS_UP: char = '\u{e18a}';
/// `timer`.
pub const TIMER: char = '\u{e1e0}';
/// `trash-2`: delete.
pub const TRASH: char = '\u{e18e}';
/// `triangle-alert`: a warning.
pub const TRIANGLE_ALERT: char = '\u{e193}';
/// `type`: text, a title.
pub const TYPE: char = '\u{e198}';
/// `undo-2`.
pub const UNDO: char = '\u{e2a1}';
/// `upload`: import.
pub const UPLOAD: char = '\u{e19e}';
/// `user`: one person.
pub const USER: char = '\u{e19f}';
/// `users`: people.
pub const USERS: char = '\u{e1a4}';
/// `video`: a clip, the camera.
pub const VIDEO: char = '\u{e1a5}';
/// `volume-2`: sound on.
pub const VOLUME: char = '\u{e1ab}';
/// `volume-x`: muted.
pub const VOLUME_X: char = '\u{e1ac}';
/// `wand-sparkles`: generate, auto-fix.
pub const WAND: char = '\u{e357}';
/// `x`: close, dismiss, remove.
pub const X: char = '\u{e1b2}';
/// `zap`: fast, a drop.
pub const ZAP: char = '\u{e1b4}';
/// `zoom-in`.
pub const ZOOM_IN: char = '\u{e1b6}';
/// `zoom-out`.
pub const ZOOM_OUT: char = '\u{e1b7}';

/// The font an icon is drawn from at `size`: the family with Lucide first
/// (the regular face on the very first frame; see `crate::gui::font`).
#[must_use]
pub fn font(ctx: &egui::Context, size: f32) -> egui::FontId {
    crate::gui::font::weight(ctx, crate::gui::font::ICONS, size)
}

/// Paints `glyph` centred in `rect`, `size` points tall, in `color`.
pub fn paint(painter: &Painter, rect: Rect, glyph: char, size: f32, color: Color32) {
    let galley = painter.layout_no_wrap(glyph.to_string(), font(painter.ctx(), size), color);
    let at = rect.center() - galley.size() / 2.0;
    painter.galley(at, galley, color);
}

/// Allocates a `size` square at the cursor and draws `glyph` in it.
pub fn icon(ui: &mut Ui, glyph: char, size: f32, color: Color32) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    paint(ui.painter(), rect, glyph, size, color);
    response
}

/// `glyph` as rich text at `size`, from the icon family: for a label,
/// a button or a menu entry built from text.
#[must_use]
pub fn icon_text(ctx: &egui::Context, glyph: char, size: f32) -> egui::RichText {
    egui::RichText::new(glyph.to_string()).font(font(ctx, size))
}

/// `glyph`, two spaces, then `label`: an icon leading a menu entry or a
/// button made from text. The icon draws from the text family's Lucide
/// fallback, at the text's size.
#[must_use]
pub fn with_label(glyph: char, label: &str) -> String {
    format!("{glyph}  {label}")
}

// The sizes have a hierarchy, checked when the crate is compiled.
const _: () = {
    assert!(SMALL < SIZE && SIZE < LARGE);
    assert!(SIZE < theme::CONTROL_HEIGHT, "an icon fits in a control");
};

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame of `ctx`, discarding its output (textures included,
    /// which epaint insists are handled).
    fn run_frame(ctx: &egui::Context, run: impl FnMut(&mut egui::Ui)) {
        let mut output = ctx.run_ui(egui::RawInput::default(), run);
        output.textures_delta.clear();
    }

    #[test]
    fn every_icon_is_in_lucides_private_use_block() {
        for glyph in [
            PLAY,
            PAUSE,
            SKIP_BACK,
            SKIP_FORWARD,
            SCISSORS,
            MESSAGE_SQUARE,
            SPARKLES,
            SEARCH,
            SETTINGS,
            FOLDER,
            FILM,
            USERS,
            MUSIC,
            LAYERS,
            PANEL_LEFT,
            PANEL_RIGHT,
            PANEL_BOTTOM,
            X,
            PLUS,
            CHEVRON_DOWN,
            CHEVRON_RIGHT,
            CHECK,
            TRIANGLE_ALERT,
            UNDO,
            REDO,
            EYE,
            LOCK,
            SEND,
            PAPERCLIP,
            STAR,
            FLAG,
            GRID,
            LIST,
            COLUMNS,
            MAXIMIZE,
            VOLUME,
            REPEAT,
        ] {
            let code = u32::from(glyph);
            assert!(
                (0xE000..=0xF8FF).contains(&code),
                "U+{code:04X} is outside the private use area"
            );
        }
    }

    #[test]
    fn the_icon_font_draws_every_named_icon() {
        let ctx = egui::Context::default();
        run_frame(&ctx, |ui| {
            let _ = crate::gui::font::setup(ui.ctx(), None);
        });
        let mut missing = Vec::new();
        run_frame(&ctx, |ui| {
            let family = egui::FontFamily::Name(crate::gui::font::ICONS.into());
            ui.ctx().fonts_mut(|fonts| {
                for glyph in [PLAY, SEARCH, SETTINGS, PANEL_BOTTOM, X, SPARKLES, ZOOM_OUT] {
                    if !fonts.has_glyph(&egui::FontId::new(16.0, family.clone()), glyph) {
                        missing.push(glyph);
                    }
                }
            });
        });
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }
}

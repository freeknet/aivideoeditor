// ============================================================================
// Module:       gui::ui::modal
// Description:  The one modal dialog: egui-material3's dialog on its scrim,
//               closed by its close button, Escape or a click outside.
//
// Dependencies: egui; egui_material3 (MaterialDialog); crate::theme::Palette
// ============================================================================

//! Modal dialogs.
//!
//! **One modal shape, not several.** Every dialog in the app is this one
//! with different contents, so there are no six dialogs of six widths, none
//! aligned, three movable for no reason. It is Material 3's basic dialog
//! (egui-material3's `MaterialDialog`): the high container on a scrim, the
//! extra-large shape, a headline, a close button, focus kept inside while
//! it is open and handed back when it closes, and Material's enter and exit
//! motion.
//!
//! Escape and a click on the scrim both dismiss, because a dialog that can
//! only be closed with the mouse is a dialog that interrupts typing.

use crate::theme::Palette;
use egui::Ui;

/// What a modal reports back after a frame.
pub struct Outcome<R> {
    /// What the contents returned, if they were drawn this frame.
    pub inner: Option<R>,
    /// Whether the user asked to dismiss it (the close button, Escape or
    /// the scrim).
    pub dismissed: bool,
}

/// Draws a modal dialog titled `title`, `width` wide, while `open` (and
/// while it animates out after closing).
pub fn show<R>(
    ctx: &egui::Context,
    _theme: &Palette,
    title: &str,
    open: bool,
    width: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> Outcome<R> {
    let id = egui::Id::unique("modal").with(title);
    // The caller owns whether the dialog is open; the library's flag is a
    // copy, and its closing is reported as `dismissed`.
    let mut still_open = open;
    let response = egui_material3::MaterialDialog::new(id, title, &mut still_open)
        .close_button(true)
        // The contents keep the app's own fonts and colours rather than
        // Material's supporting-text style.
        .content_style(egui_material3::dialog::DialogContentStyle::Inherit)
        .dismiss_on_escape(true)
        .dismiss_on_scrim(true)
        .min_width(width)
        .max_width(width)
        .content(move |ui, _close| contents(ui))
        .show(ctx);
    Outcome {
        inner: response.inner,
        dismissed: open && response.close_reason.is_some(),
    }
}

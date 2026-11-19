// ============================================================================
// Module:       gui
// Description:  The desktop front end: the window's size limits, and the seam
//               between the app and the runtime that owns the window.
//
// Dependencies: egui; see the submodules
// ============================================================================

//! The desktop front end.
//!
//! [`run::run`] opens the window and drives [`app::App`] until it closes.
//! Everything inside the window is [`app`] and [`ui`], which use egui and
//! nothing else, so the runtime in [`run`] can be swapped without either of
//! them noticing.

pub mod app;
pub(crate) mod assistant;
pub mod font;
pub mod icons;
pub(crate) mod pages;
pub(crate) mod player;
pub mod run;
pub(crate) mod thumbs;
pub mod ui;

/// The window's initial inner size, in logical points, when no size was
/// saved.
pub const DEFAULT_SIZE: [f32; 2] = [1180.0, 760.0];

/// The smallest the window may be dragged to. Below this the sidebar and a
/// column of cards stop fitting side by side, and a window that can be made
/// unusable will be, by accident, once.
pub const MIN_SIZE: [f32; 2] = [720.0, 480.0];

// Relations between constants, checked when the crate is compiled.
const _: () = {
    assert!(MIN_SIZE[0] < DEFAULT_SIZE[0] && MIN_SIZE[1] < DEFAULT_SIZE[1]);
    assert!(
        MIN_SIZE[0] > ui::theme::SIDEBAR_WIDTH * 2.0,
        "at this width the sidebar would be most of the window"
    );
};

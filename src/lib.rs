// ============================================================================
// Module:       aivideoeditor (library crate root)
// Description:  Module graph: the portable core, the Win32 layer, and the
//               desktop front end built on top of both.
//
// Dependencies: anyhow; see the individual modules. ffmpeg and llama.cpp at
//               run time.
// ============================================================================

//! AI Video Editor: Index footage, plan beat-synced edits, render and export timelines.
//!
//! The crate is in three layers, and the split is what keeps it testable.
//!
//! The **portable core** is plain Rust with no window in it: [`color`] is
//! the colour maths, [`theme`] the palette and its readability rules,
//! [`motion`] the easing curves and durations,
//! [`icon`] the icon geometry,
//! [`brand`] the application mark,
//! [`config`] the preferences file,
//! and [`cli`] the command line. Its tests run on any machine, without a
//! GPU or a display.
//!
//! [`gui`] is the desktop front end. It draws with egui and nothing else,
//! so the runtime that owns the window (`gui::run`) can change without the
//! app noticing.
//!
//! `win` is the only place that talks to Win32. Every call is wrapped in a
//! safe leaf function, so nothing outside it contains `unsafe`.
//!
//! The **engine** turns footage into an edit and needs no window either:
//! `media` talks to ffmpeg, `analyze` and `index` measure every clip into a
//! shot log, `caption` adds vision-model captions, `beats` maps the music,
//! `people` says who is in which clip, `plan` / `draft` build and check the
//! storyboard, and `render` / `otio` turn it into a video or a timeline.
//! [`cli`] runs any engine step headless.
//!
//! See `docs/ARCHITECTURE.md` for the module map.

mod alternatives;
mod analyze;
mod beats;
pub mod brand;
mod caption;
pub mod cli;
pub mod color;
pub mod command_palette;
mod commands;
pub mod config;
mod draft;
pub mod gui;
pub mod icon;
mod index;
mod jobs;
mod logging;
mod mcp;
mod media;
pub mod motion;
mod otio;
mod people;
mod plan;
mod project;
mod recipe;
pub mod refs;
mod render;
mod search;
pub mod theme;

#[cfg(windows)]
pub mod win;

/// The product name, as it appears in the window title and the about page.
pub const NAME: &str = "AI Video Editor";

/// The one-line description, for `--help` and the about page.
pub const TAGLINE: &str = "Index footage, plan beat-synced edits, render and export timelines.";

/// The colour the project was scaffolded with: the seed for the built-in
/// theme's accent and for the mark's colours.
///
/// Parsed at compile time. A malformed value falls back to cyan here and
/// fails `the_accent_seed_parses`, rather than painting black.
pub const ACCENT: color::Rgb = match color::Rgb::from_hex("#ff3fb4") {
    Some(accent) => accent,
    None => color::Rgb::new(0x00, 0xe5, 0xff),
};

/// The application id: config directory, single-instance mutex, tray host.
pub const APP_ID: &str = "aivideoeditor";

/// Installs the logger (stderr, `RUST_LOG`, default info). The window
/// calls this too, so engine and player warnings reach a terminal.
pub fn init_logging() {
    logging::init();
}

/// Makes a panic in the window app visible: written to a crash log next to
/// the config and shown in a dialog, before the abort closes the window.
pub fn install_crash_report() {
    logging::install_crash_report();
}

/// Runs the headless subcommand in `cli`, writing its report to `out`.
///
/// # Errors
///
/// When there is no subcommand, or the command fails.
pub fn run_command<W: std::io::Write>(cli: Cli, out: &mut W) -> anyhow::Result<()> {
    logging::init();
    let Some(command) = cli.command else {
        anyhow::bail!("no subcommand given");
    };
    commands::execute(command, out)
}

use cli::Cli;

#[cfg(test)]
mod tests {
    #[test]
    fn the_accent_seed_parses() {
        assert_eq!(
            super::color::Rgb::from_hex("#ff3fb4"),
            Some(super::ACCENT),
            "the scaffolded accent must be a #rrggbb colour"
        );
    }
}

#[cfg(test)]
mod header_check;
#[cfg(test)]
mod unsafe_check;

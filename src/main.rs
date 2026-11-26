// ============================================================================
// Module:       aivideoeditor (binary crate root)
// Description:  Entry point: parses the command line, then either runs a
//               pipeline step headless or opens the desktop front end.
//
// Dependencies: anyhow; aivideoeditor::{cli, gui}
// ============================================================================

// No console window behind the GUI in a release build: Windows gives a
// console-subsystem binary a terminal whether or not it writes to one, so
// launching from Explorer would open an empty black window beside the app.
// Debug builds keep the console, which is where panics go while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Entry point for `ave`.
//!
//! Deliberately thin: it parses the command line, loads the saved
//! preferences and hands off. Everything else belongs to the library.
//!
//! ## Failures are reported in a dialog on Windows
//!
//! A release build has no console, so anything written to stderr goes
//! nowhere and a startup failure would look like the program refusing to
//! open. See [`report`].

use aivideoeditor::cli::Cli;

fn main() -> std::process::ExitCode {
    // Every process the app starts (ffmpeg, mostly) ends when the app does,
    // closed, killed or crashed; held for the whole run.
    #[cfg(windows)]
    let _children = aivideoeditor::win::job::kill_children_on_exit();
    // A release build has no console of its own; join the terminal we were
    // started from so subcommands, --help and argument errors can print.
    #[cfg(windows)]
    if std::env::args_os().len() > 1 {
        let _ = aivideoeditor::win::console::attach_parent();
    }
    let cli = {
        use clap::Parser as _;
        Cli::parse()
    };
    if cli.is_headless() {
        return match aivideoeditor::run_command(cli, &mut std::io::stdout().lock()) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error:#}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    // A second launch hands focus to the first window and leaves, rather
    // than opening a second copy that fights the first over the config file.
    #[cfg(windows)]
    let _instance = match aivideoeditor::win::single_instance::acquire(aivideoeditor::APP_ID) {
        Some(guard) => guard,
        None => {
            aivideoeditor::win::single_instance::focus_existing(aivideoeditor::NAME);
            return std::process::ExitCode::SUCCESS;
        }
    };

    match run(cli) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            report(&format!("{error:#}"));
            std::process::ExitCode::FAILURE
        }
    }
}

/// Applies the command line to the saved preferences and opens the window.
fn run(cli: Cli) -> anyhow::Result<()> {
    aivideoeditor::init_logging();
    aivideoeditor::install_crash_report();
    let mut config = if cli.reset {
        aivideoeditor::config::Config::default()
    } else {
        aivideoeditor::config::load()
    };
    if let Some(theme) = cli.theme {
        config.theme = Some(theme);
    }
    if let Some(font) = cli.font {
        config.font = Some(font);
    }
    let result = aivideoeditor::gui::run::run(config, cli.open);
    aivideoeditor::flush_logs(std::time::Duration::from_secs(2));
    result
}

/// Reports a startup failure somewhere the user will actually see it.
///
/// Written to stderr for a terminal, and on Windows also shown in a
/// message box, because a release build has no console at all.
fn report(message: &str) {
    eprintln!("{message}");
    #[cfg(windows)]
    aivideoeditor::win::dialog::show_error(message, aivideoeditor::NAME);
}

// ============================================================================
// Module:       aivideoeditor (library crate root)
// Description:  The whole application behind one testable entry point:
//               argument parsing, logging setup, and command dispatch.
//
// Dependencies: anyhow; clap, configuration, HTTP and SQLite when selected
// ============================================================================

//! AI Video Editor: Index footage, plan beat-synced edits, render and export timelines.
//!
//! Everything the program does lives in this library, and `src/main.rs`
//! only hands it the process arguments and standard output. That split is
//! what makes the binary testable: `tests/cli.rs` calls [`run`] with its
//! own argument list and an in-memory buffer, so the commands are checked
//! without spawning a process, finding a compiled executable, or scraping
//! a terminal.
//!
//! [`run`] reports through its return value rather than exiting. A usage
//! error is exit code 2 (the convention `clap` and most Unix tools share),
//! help and version requests are success, and an operational failure is an
//! `Err` whose context chain the binary prints on the way out.

mod cli;
pub mod config;
mod logging;

/// Runs one invocation of the program and returns its exit code.
///
/// `args` includes the program name as its first item, exactly as
/// [`std::env::args_os`] yields it. Normal output goes to `out`; usage
/// errors, help, and logs go where the terminal expects them.
///
/// # Errors
///
/// Returns an error when a command fails (an unwritable `out`, or a
/// failure in the command itself). Refused arguments are not an error:
/// they print a usage message and return exit code 2.
pub fn run<I, T, W>(args: I, out: &mut W) -> anyhow::Result<std::process::ExitCode>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
    W: std::io::Write,
{
    logging::init();
    let options = match cli::parse(args) {
        Ok(options) => options,
        Err(code) => return Ok(code),
    };
    cli::execute(options, out)?;
    Ok(std::process::ExitCode::SUCCESS)
}

// Test-only guard modules, when enabled, go last: rustfmt sorts adjacent
// `mod` lines, and these must not interleave with the ones above.
#[cfg(test)]
mod header_check;

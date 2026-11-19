// ============================================================================
// Module:       cli (integration test)
// Description:  End-to-end checks of the headless subcommands, through the
//               same entry point the binary calls.
//
// Dependencies: aivideoeditor (the library), anyhow, clap, tempfile
// ============================================================================

//! End-to-end checks of the headless command line.
//!
//! These parse an argument list and call [`aivideoeditor::run_command`]
//! with an in-memory buffer, so they need no window, no shell and no path
//! to a compiled executable.

use aivideoeditor::cli::Cli;
use clap::Parser as _;

/// Parses `args` after the program name.
fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("ave").chain(args.iter().copied()))
}

#[test]
fn a_bare_run_opens_the_window_rather_than_a_command() -> anyhow::Result<()> {
    let cli = parse(&[])?;
    assert!(!cli.is_headless(), "no subcommand means the GUI");
    Ok(())
}

#[test]
fn show_without_an_index_explains_itself() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let work = dir.path().to_string_lossy().into_owned();
    let cli = parse(&["show", "--work", &work])?;
    let mut out = Vec::new();
    let result = aivideoeditor::run_command(cli, &mut out);
    let message = format!(
        "{:#}",
        result
            .err()
            .ok_or_else(|| anyhow::anyhow!("expected an error"))?
    );
    assert!(
        message.contains("ave index"),
        "points at the fix: {message}"
    );
    Ok(())
}

#[test]
fn an_unknown_subcommand_is_a_usage_error() -> anyhow::Result<()> {
    let error = parse(&["frobnicate"])
        .err()
        .ok_or_else(|| anyhow::anyhow!("expected an error"))?;
    assert!(error.use_stderr(), "usage errors go to stderr");
    Ok(())
}

#[test]
fn version_is_not_an_error() -> anyhow::Result<()> {
    let error = parse(&["--version"])
        .err()
        .ok_or_else(|| anyhow::anyhow!("clap reports --version as an early exit"))?;
    assert!(
        !error.use_stderr(),
        "--version prints to stdout and exits 0"
    );
    Ok(())
}

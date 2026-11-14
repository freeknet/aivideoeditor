// ============================================================================
// Module:       cli (integration test)
// Description:  End-to-end checks of the command line, through the same
//               entry point the binary calls.
//
// Dependencies: aivideoeditor (the library), anyhow
// ============================================================================

//! End-to-end checks of the command line.
//!
//! These call [`aivideoeditor::run`] with an argument list and an in-memory
//! buffer instead of spawning the compiled binary, so they need no shell,
//! no path to an executable, and no scraping of a real terminal.

use std::process::ExitCode;

/// Exit code 0, named for the assertions below.
const SUCCESS: ExitCode = ExitCode::SUCCESS;

/// Runs the program with `args` and returns its exit code and stdout.
fn run(args: &[&str]) -> anyhow::Result<(ExitCode, String)> {
    let mut out = Vec::new();
    let code = aivideoeditor::run(args.iter().copied(), &mut out)?;
    Ok((code, String::from_utf8(out)?))
}

#[test]
fn the_default_action_greets() -> anyhow::Result<()> {
    let (code, out) = run(&["aivideoeditor"])?;
    assert_eq!(code, SUCCESS, "a bare run succeeds");
    assert!(out.ends_with("!\n"), "one line: {out:?}");
    Ok(())
}

#[test]
fn a_name_is_greeted() -> anyhow::Result<()> {
    let (code, out) = run(&["aivideoeditor", "--name", "Ferris"])?;
    assert_eq!(code, SUCCESS, "a valid flag succeeds");
    assert!(out.contains("Ferris"), "--name is used: {out:?}");
    Ok(())
}

#[test]
fn an_unknown_flag_is_a_usage_error() -> anyhow::Result<()> {
    let (code, out) = run(&["aivideoeditor", "--no-such-flag"])?;
    assert_eq!(code, ExitCode::from(2), "usage errors exit 2");
    assert!(out.is_empty(), "nothing ran: {out:?}");
    Ok(())
}

#[test]
fn version_is_not_an_error() -> anyhow::Result<()> {
    let (code, _) = run(&["aivideoeditor", "--version"])?;
    assert_eq!(code, SUCCESS, "--version exits 0");
    Ok(())
}

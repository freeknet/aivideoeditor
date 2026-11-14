// ============================================================================
// Module:       cli
// Description:  The command line and what each command does.
//
// Dependencies: clap (derive) when selected; std::io for output
// ============================================================================

//! The command line, and the commands it dispatches to.
//!
//! Commands write to the `out` they are handed instead of reaching for
//! standard output themselves. The binary passes the real stdout; tests
//! pass a `Vec<u8>` and read back exactly what a user would have seen.
//!
//! Keep the surface small. Every flag added here is a flag that has to
//! keep working, so each one should earn its place by being the only way
//! to reach something.

use std::io::Write;

/// Who the default action greets when nobody is named.
const DEFAULT_NAME: &str = "world";

/// The word the default action greets with when no preference is saved.
const DEFAULT_GREETING: &str = "Hello";

/// The command line.
///
/// `about` comes from the `description` in `Cargo.toml`, so `--help` and
/// the published crate metadata cannot drift apart.
#[derive(clap::Parser, Debug)]
#[command(name = "aivideoeditor", version, about)]
pub(crate) struct Cli {
    /// Who to greet. Overrides the saved preference for this run.
    #[arg(long, value_name = "NAME")]
    pub(crate) name: Option<String>,
}

/// Parses `args`, or prints why it could not and yields the exit code.
///
/// Help and `--version` are "errors" to clap but successes to the user,
/// so they exit 0; anything clap refused exits 2.
pub(crate) fn parse<I, T>(args: I) -> Result<Cli, std::process::ExitCode>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    use clap::Parser as _;
    Cli::try_parse_from(args).map_err(|error| {
        // Printing is best effort: if the terminal is gone there is no one
        // left to tell, and the exit code still carries the outcome.
        let _ = error.print();
        if error.use_stderr() {
            std::process::ExitCode::from(2)
        } else {
            std::process::ExitCode::SUCCESS
        }
    })
}

/// Runs the command `options` selects.
pub(crate) fn execute(options: Cli, out: &mut impl Write) -> anyhow::Result<()> {
    greet(out, options.name.as_deref())
}

/// The default action: a greeting, so a fresh scaffold visibly does
/// something. Replace it with whatever the program is for.
pub(crate) fn greet(out: &mut impl Write, name: Option<&str>) -> anyhow::Result<()> {
    let saved = crate::config::load();
    let greeting = saved.greeting.as_deref().unwrap_or(DEFAULT_GREETING);
    let name = name.or(saved.name.as_deref()).unwrap_or(DEFAULT_NAME);
    log::debug!("greeting {name} with {greeting:?}");
    writeln!(out, "{greeting}, {name}!")?;
    Ok(())
}

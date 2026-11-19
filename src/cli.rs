// ============================================================================
// Module:       cli
// Description:  The command line: window switches for the desktop app, and the
//               pipeline subcommands that run headless instead.
//
// Dependencies: clap (derive); crate::commands
// ============================================================================

//! The command line.
//!
//! `ave` with no subcommand opens the window; the switches below only adjust
//! it. `ave index …`, `ave plan …` and the other pipeline subcommands run in
//! the terminal without a window, so the same binary serves batch scripts,
//! agents and the GUI (see [`crate::commands`]).

/// The parsed command line.
#[derive(clap::Parser, Debug, Default, PartialEq)]
#[command(name = "ave", version, about = crate::TAGLINE)]
pub struct Cli {
    /// Select this theme for the session (and save it as the preference).
    #[arg(long, value_name = "ID")]
    pub theme: Option<String>,
    /// Set the UI in this installed font family, by name.
    #[arg(long, value_name = "FAMILY")]
    pub font: Option<String>,
    /// Start from the built-in defaults and overwrite the saved preferences
    /// on exit: the escape hatch for a config that made the app unusable.
    #[arg(long)]
    pub reset: bool,
    /// Open this work folder (a night's `_ave` folder) on start.
    #[arg(long, value_name = "DIR")]
    pub open: Option<std::path::PathBuf>,
    /// A pipeline step to run headless instead of opening the window.
    #[command(subcommand)]
    pub(crate) command: Option<crate::commands::Cmd>,
}

impl Cli {
    /// Whether a headless subcommand was given (no window should open).
    #[must_use]
    pub fn is_headless(&self) -> bool {
        self.command.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser as _;

    /// Parses `args` after a program name.
    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("ave").chain(args.iter().copied()))
    }

    #[test]
    fn the_bare_invocation_parses_to_the_defaults() -> anyhow::Result<()> {
        let cli = parse(&[]).map_err(|error| anyhow::anyhow!("{error}"))?;
        assert_eq!(cli, Cli::default(), "double-clicked from Explorer");
        assert!(!cli.is_headless(), "no subcommand opens the window");
        Ok(())
    }

    #[test]
    fn an_unknown_flag_is_rejected_rather_than_ignored() {
        // A typo'd flag that is silently ignored appears not to work.
        assert!(parse(&["--nope"]).is_err(), "unknown flags are errors");
    }

    #[test]
    fn a_subcommand_runs_headless() -> anyhow::Result<()> {
        let cli = parse(&["show", "--work", "x"]).map_err(|error| anyhow::anyhow!("{error}"))?;
        assert!(cli.is_headless(), "a pipeline step needs no window");
        Ok(())
    }

    #[test]
    fn window_switches_are_taken() -> anyhow::Result<()> {
        let cli = parse(&["--font", "Inter", "--theme", "nebula", "--reset"])
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        assert_eq!(cli.font.as_deref(), Some("Inter"), "font");
        assert_eq!(cli.theme.as_deref(), Some("nebula"), "theme");
        assert!(cli.reset, "--reset is a flag");
        Ok(())
    }
}

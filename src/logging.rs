// ============================================================================
// Module:       logging
// Description:  Logging setup: diagnostics to stderr, filtered by level, so
//               stdout stays clean for the program's real output.
//
// Dependencies: the selected logging crate
// ============================================================================

//! Logging setup.
//!
//! Logs go to stderr and output goes to stdout, so piping the program into
//! another one never mixes the two. The level defaults to `info`;
//! `RUST_LOG` overrides it where the logger supports that.
//!
//! [`init`] may run more than once (every test that calls the entry point
//! runs it), and something else may have installed a logger first. Both
//! are fine: the first logger installed wins and later attempts are no-ops
//! rather than errors.

/// Installs the logger, unless one is already installed.
pub(crate) fn init() {
    let env = env_logger::Env::default().default_filter_or("info");
    let _ = env_logger::Builder::from_env(env).try_init();
}

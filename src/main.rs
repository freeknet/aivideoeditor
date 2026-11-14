// ============================================================================
// Module:       aivideoeditor (binary crate root)
// Description:  Entry point: hands the process arguments and stdout to the
//               library and turns its result into an exit code.
//
// Dependencies: aivideoeditor (the library), anyhow
// ============================================================================

//! Entry point for `aivideoeditor`.
//!
//! Deliberately thin. Everything that can be tested lives in the library
//! (see `src/lib.rs`); this file only connects it to the process. An `Err`
//! returned from `main` prints its whole context chain to stderr and exits
//! with code 1.

fn main() -> anyhow::Result<std::process::ExitCode> {
    aivideoeditor::run(std::env::args_os(), &mut std::io::stdout().lock())
}

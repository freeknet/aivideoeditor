// ============================================================================
// Module:       logging
// Description:  Logging setup: diagnostics to stderr, filtered by level, so
//               stdout stays clean for the program's real output.
//
// Dependencies: the selected logging crate, dirs; win::dialog (crash report)
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

/// Where [`install_crash_report`] writes: next to the config, so it
/// survives the window closing.
pub(crate) fn crash_log_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join(crate::APP_ID).join("crash.log"))
}

/// Makes a panic visible in the window app.
///
/// The release build has no console and aborts on panic, so without this a
/// bug looks like the window vanishing with no word of why (the message
/// went to a stderr nobody has). The hook appends the message, where it
/// happened and a backtrace to [`crash_log_path`], shows it in a dialog on
/// Windows, then lets the default hook run (the abort follows).
pub(crate) fn install_crash_report() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let report = format!(
            "{} crashed on thread '{}': {info}\n\n{}",
            crate::NAME,
            thread.name().unwrap_or("unnamed"),
            std::backtrace::Backtrace::force_capture()
        );
        let written = crash_log_path().and_then(|path| {
            use std::io::Write as _;
            let _ = std::fs::create_dir_all(path.parent()?);
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()?;
            writeln!(file, "---- {:?}\n{report}", std::time::SystemTime::now()).ok()?;
            Some(path)
        });
        #[cfg(windows)]
        {
            // Everything before the backtrace: where, then the message.
            let first = report.split("\n\n").next().unwrap_or_default();
            let saved = written.map_or_else(String::new, |path| {
                format!("\n\nThe details were saved to {}", path.display())
            });
            crate::win::dialog::show_error(
                &format!("{first}{saved}\n\nPlease report this."),
                crate::NAME,
            );
        }
        #[cfg(not(windows))]
        let _ = written;
        default(info);
    }));
}

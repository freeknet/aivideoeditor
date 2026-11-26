// ============================================================================
// Module:       logging
// Description:  The `log` backend: RUST_LOG-filtered lines to stderr, a
//               rotating per-session file for the window app, and an
//               in-memory ring; plus the crash report.
//
// Dependencies: log, env_filter, dirs; win::dialog (crash report)
// ============================================================================

//! Logging.
//!
//! Logs go to stderr and output goes to stdout, so piping the program into
//! another one never mixes the two. `RUST_LOG` filters them with
//! env_logger's syntax (`warn,aivideoeditor::render=debug`); without it the
//! level is `info`.
//!
//! The window app also writes every line to a **session file**,
//! `ave-<UTC time>-<pid>.log` under [`logs_dir`] (`%LOCALAPPDATA%\
//! aivideoeditor\logs` on Windows). A release build has no console, so
//! without it a warning from the engine or the player goes nowhere. The
//! newest [`KEEP_LOGS`] sessions are kept, oldest by modification time
//! deleted first: names carry the clock, and a clock that jumped backwards
//! would make name order delete the newest. A file stops at
//! [`MAX_FILE_BYTES`], so something logging on every frame cannot fill the
//! disk. Headless commands (`ave index`, and `ave mcp`, which the assistant
//! starts once per chat) log to stderr only: a session file each would
//! rotate the window's logs away.
//!
//! The thread that logs may be the UI thread, so the file write happens on
//! a writer thread: a record is formatted once, pushed into the ring (a
//! mutex held for one `push_back`) and handed over with a `try_send` that
//! never blocks. If the writer falls behind, lines are counted and dropped,
//! and the file says how many. [`flush`] waits for the writer, with a
//! timeout; the crash report and a clean exit call it.
//!
//! The ring keeps the last [`RING_LINES`] records, for the crash report now
//! and a log panel in the window later.
//!
//! [`init`] may run more than once (every test that calls the entry point
//! runs it), and something else may have installed a logger first. Both
//! are fine: the first logger installed wins and later attempts are no-ops.

use log::{LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime};

/// Records kept in the in-memory ring.
const RING_LINES: usize = 2000;

/// Session logs kept on disk, including the current one.
const KEEP_LOGS: usize = 10;

/// The most a session file grows to. An evening's editing logs well under
/// a megabyte; past this, something is logging per frame.
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Records that may wait for the writer thread before new ones are dropped.
const CHANNEL_DEPTH: usize = 8192;

/// How long the crash report waits for the writer to flush.
const CRASH_FLUSH: Duration = Duration::from_millis(500);

/// Replaces the base directory for logs and the crash report, so a test
/// or a portable run keeps them out of the user's profile.
const DATA_ENV: &str = "AVE_DATA_DIR";

/// The per-user directory for what ave writes about itself (logs, the
/// crash report): `%LOCALAPPDATA%\aivideoeditor` on Windows. Local, not
/// roaming, because logs are about this machine. Not created here.
pub(crate) fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os(DATA_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(crate::APP_ID)
}

/// Where session logs are written.
pub(crate) fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

/// Where [`install_crash_report`] appends.
pub(crate) fn crash_log_path() -> PathBuf {
    data_dir().join("crash.log")
}

/// Where the logger writes.
struct Config {
    /// The `RUST_LOG` value, if any.
    spec: Option<String>,
    /// Directory for the session file; `None` keeps logs in memory and on
    /// stderr only.
    dir: Option<PathBuf>,
    /// Whether to mirror lines to stderr.
    stderr: bool,
    /// Bytes after which the session file takes no more lines.
    max_file_bytes: u64,
}

impl Config {
    fn from_env(dir: Option<PathBuf>) -> Self {
        Self {
            spec: std::env::var("RUST_LOG")
                .ok()
                .filter(|spec| !spec.trim().is_empty()),
            dir,
            stderr: true,
            max_file_bytes: MAX_FILE_BYTES,
        }
    }
}

/// A message for the writer thread.
enum Message {
    Line(Arc<str>),
    Flush(SyncSender<()>),
}

/// The `log::Log` implementation.
struct Logger {
    filter: env_filter::Filter,
    /// What the filter was built from, for the session header.
    spec: String,
    ring: Mutex<VecDeque<Arc<str>>>,
    sink: Option<SyncSender<Message>>,
    stderr: bool,
    dropped: Arc<AtomicU64>,
    path: Option<PathBuf>,
    started: Instant,
}

impl Logger {
    /// Builds a logger, opens the session file and starts the writer
    /// thread. A file that cannot be opened degrades to memory and stderr,
    /// and the first line says why.
    fn new(config: Config) -> Self {
        let spec = config.spec.unwrap_or_else(|| "info".to_owned());
        let mut builder = env_filter::Builder::new();
        builder.filter_level(LevelFilter::Info);
        if let Err(error) = builder.try_parse(&spec) {
            // The filter is half-built only for valid input; start over at
            // the default rather than guess what was meant.
            builder = env_filter::Builder::new();
            builder.filter_level(LevelFilter::Info);
            eprintln!("RUST_LOG `{spec}` ignored: {error}");
        }
        let mut notes = Vec::new();
        let opened = config
            .dir
            .as_deref()
            .and_then(|dir| match open_session_file(dir) {
                Ok(opened) => Some(opened),
                Err(error) => {
                    notes.push(format!(
                        "log file unavailable in {}: {error}",
                        dir.display()
                    ));
                    None
                }
            });
        let path = opened.as_ref().map(|(_, path)| path.clone());
        let dropped = Arc::new(AtomicU64::new(0));
        let sink = match opened {
            Some((file, _)) => {
                let file = SessionFile::new(file, config.max_file_bytes);
                match spawn_writer(file, Arc::clone(&dropped)) {
                    Ok(sink) => Some(sink),
                    Err(error) => {
                        notes.push(format!("log writer thread failed: {error}"));
                        None
                    }
                }
            }
            None => None,
        };
        let logger = Self {
            filter: builder.build(),
            spec,
            ring: Mutex::new(VecDeque::with_capacity(RING_LINES)),
            sink,
            stderr: config.stderr,
            dropped,
            path,
            started: Instant::now(),
        };
        if logger.path.is_some() {
            logger.to_file_and_ring(
                format!(
                    "=== {} {} session started {} (log times are seconds since start) ===",
                    crate::NAME,
                    env!("CARGO_PKG_VERSION"),
                    utc_iso(SystemTime::now())
                )
                .into(),
            );
            logger.to_file_and_ring(format!("log filter: {}", logger.spec).into());
        }
        for note in notes {
            logger.push(format!("logging: {note}").into());
        }
        logger
    }

    /// The newest `count` records, oldest first.
    fn recent(&self, count: usize) -> Vec<String> {
        let ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
        let skip = ring.len().saturating_sub(count);
        ring.iter()
            .skip(skip)
            .map(|line| line.to_string())
            .collect()
    }

    /// [`Logger::recent`] without waiting for the ring's lock: the crash
    /// report may run on the thread that holds it.
    fn try_recent(&self, count: usize) -> Option<Vec<String>> {
        let ring = self.ring.try_lock().ok()?;
        let skip = ring.len().saturating_sub(count);
        Some(
            ring.iter()
                .skip(skip)
                .map(|line| line.to_string())
                .collect(),
        )
    }

    /// Asks the writer to flush the file and waits up to `timeout`.
    /// Returns whether the flush completed.
    fn flush_within(&self, timeout: Duration) -> bool {
        let Some(sink) = &self.sink else {
            return true;
        };
        let (ack_tx, ack_rx) = sync_channel(1);
        if sink.try_send(Message::Flush(ack_tx)).is_err() {
            return false;
        }
        match ack_rx.recv_timeout(timeout) {
            Ok(()) => true,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => false,
        }
    }

    /// A record: to the ring and the file, and to stderr when mirrored.
    /// Stderr is written here, not on the writer thread, so a line logged
    /// just before a command exits is not lost with the thread.
    fn push(&self, text: Arc<str>) {
        if self.stderr {
            // Nowhere to report a failed write to stderr.
            writeln!(std::io::stderr(), "{text}").ok();
        }
        self.to_file_and_ring(text);
    }

    /// A line for the ring and the file only: the session header, which a
    /// terminal does not need.
    fn to_file_and_ring(&self, text: Arc<str>) {
        {
            let mut ring = self.ring.lock().unwrap_or_else(PoisonError::into_inner);
            if ring.len() == RING_LINES {
                ring.pop_front();
            }
            ring.push_back(Arc::clone(&text));
        }
        if let Some(sink) = &self.sink
            && sink.try_send(Message::Line(text)).is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.filter.enabled(metadata)
    }

    fn log(&self, record: &Record<'_>) {
        if !self.filter.matches(record) {
            return;
        }
        let text = format_record(
            self.started.elapsed(),
            record.level(),
            std::thread::current().name().unwrap_or("?"),
            record.target(),
            &record.args().to_string(),
        );
        self.push(text.into());
    }

    fn flush(&self) {
        self.flush_within(Duration::from_secs(2));
    }
}

/// One record as a line:
///
/// ```text
///    12.345s INFO  [main] aivideoeditor::render: wrote cut.mp4
/// ```
///
/// Continuation lines of a multi-line message are indented so a record
/// stays visually one unit.
fn format_record(
    uptime: Duration,
    level: log::Level,
    thread: &str,
    target: &str,
    message: &str,
) -> String {
    let body = message.replace('\n', "\n    ");
    format!(
        "{:>9.3}s {:<5} [{thread}] {target}: {body}",
        uptime.as_secs_f64(),
        level.as_str()
    )
}

/// Creates the session file, first deleting the oldest logs so at most
/// [`KEEP_LOGS`] remain afterwards.
fn open_session_file(dir: &Path) -> std::io::Result<(File, PathBuf)> {
    fs::create_dir_all(dir)?;
    prune_old_logs(dir, KEEP_LOGS.saturating_sub(1));
    let name = format!(
        "ave-{}-{}.log",
        utc_stamp(SystemTime::now()),
        std::process::id()
    );
    let path = dir.join(name);
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    Ok((file, path))
}

/// Deletes all but the newest `keep` session logs in `dir`, by
/// modification time, returning how many went. A log still open elsewhere
/// (another window's session) cannot be deleted and is skipped.
fn prune_old_logs(dir: &Path, keep: usize) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let logs: Vec<(PathBuf, SystemTime)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("ave-") && name.ends_with(".log"))
        })
        .map(|path| {
            let time = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (path, time)
        })
        .collect();
    beyond_newest(logs, keep)
        .iter()
        .filter(|path| fs::remove_file(path).is_ok())
        .count()
}

/// The items to delete so only the newest `keep` remain, oldest first.
/// Items modified at the same instant are told apart by their own order,
/// so the result does not depend on directory listing order.
fn beyond_newest<T: Ord>(mut items: Vec<(T, SystemTime)>, keep: usize) -> Vec<T> {
    items.sort_by(|(a_item, a_time), (b_item, b_time)| {
        a_time.cmp(b_time).then_with(|| a_item.cmp(b_item))
    });
    let excess = items.len().saturating_sub(keep);
    items
        .into_iter()
        .take(excess)
        .map(|(item, _)| item)
        .collect()
}

/// The session file as the writer thread sees it: buffered, and taking
/// lines until its cap.
struct SessionFile {
    file: BufWriter<File>,
    written: u64,
    limit: u64,
    full: bool,
}

impl SessionFile {
    fn new(file: File, limit: u64) -> Self {
        Self {
            file: BufWriter::new(file),
            written: 0,
            limit,
            full: false,
        }
    }

    /// Writes one record and its newline, unless the cap is reached; the
    /// first record that would cross it is replaced by a note saying why
    /// the file ends there.
    fn line(&mut self, text: &str) {
        if self.full {
            return;
        }
        let length = u64::try_from(text.len()).map_or(u64::MAX, |bytes| bytes.saturating_add(1));
        if self.written.saturating_add(length) > self.limit {
            self.full = true;
            // The logger cannot log its own failure, and this is the
            // file's last line either way.
            writeln!(
                self.file,
                "logging: this session log reached its cap of {} bytes; later lines are not \
                 written here",
                self.limit
            )
            .ok();
            // As above: nothing more is written to this file.
            self.file.flush().ok();
            return;
        }
        if writeln!(self.file, "{text}").is_ok() {
            self.written = self.written.saturating_add(length);
        }
    }

    fn flush(&mut self) {
        // The logger cannot log its own failure; the next write tries again.
        self.file.flush().ok();
    }
}

fn spawn_writer(
    file: SessionFile,
    dropped: Arc<AtomicU64>,
) -> std::io::Result<SyncSender<Message>> {
    let (tx, rx) = sync_channel(CHANNEL_DEPTH);
    std::thread::Builder::new()
        .name("ave-log".to_owned())
        .spawn(move || writer_loop(&rx, file, &dropped))?;
    Ok(tx)
}

/// Drains the channel into the file, flushing whenever it catches up.
fn writer_loop(rx: &Receiver<Message>, mut file: SessionFile, dropped: &AtomicU64) {
    while let Ok(first) = rx.recv() {
        let mut next = Some(first);
        while let Some(message) = next.take() {
            match message {
                Message::Line(text) => file.line(&text),
                Message::Flush(ack) => {
                    file.flush();
                    // The flusher stopped waiting; the flush happened anyway.
                    ack.send(()).ok();
                }
            }
            next = rx.try_recv().ok();
        }
        let lost = dropped.swap(0, Ordering::Relaxed);
        if lost > 0 {
            file.line(&format!(
                "logging: {lost} log lines dropped while the writer was behind"
            ));
        }
        file.flush();
    }
}

/// `time` as `YYYYMMDDTHHMMSSZ`, for a file name.
fn utc_stamp(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(time);
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

/// `time` as `YYYY-MM-DDTHH:MM:SSZ`, for the session header.
fn utc_iso(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = utc_parts(time);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// A `SystemTime` as UTC calendar parts. Before 1970 reads as the epoch:
/// a wrong stamp in a log name is better than no log.
fn utc_parts(time: SystemTime) -> (i64, u32, u32, u64, u64, u64) {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    (
        year,
        month,
        day,
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60,
    )
}

/// Days since the Unix epoch into (year, month, day) on the proleptic
/// Gregorian calendar: Howard Hinnant's public-domain algorithm, here
/// rather than a date crate for one log name.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        year,
        u32::try_from(month).unwrap_or(1),
        u32::try_from(day).unwrap_or(1),
    )
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Installs `config`'s logger unless one is installed already.
fn install(config: Config) {
    let logger = LOGGER.get_or_init(|| Logger::new(config));
    // Err means another logger is installed (a test harness's, or ours
    // from an earlier call): the first one wins, by design.
    if log::set_logger(logger).is_ok() {
        log::set_max_level(logger.filter.filter());
    }
}

/// Installs the stderr logger, unless one is already installed. For
/// headless commands and tests: no session file.
pub(crate) fn init() {
    install(Config::from_env(None));
}

/// Installs the window app's logger: stderr, the ring, and a session file
/// under [`logs_dir`]. Unless one is already installed.
pub(crate) fn init_session() {
    install(Config::from_env(Some(logs_dir())));
}

/// This session's log file, if the logger writes one.
pub(crate) fn log_file() -> Option<PathBuf> {
    LOGGER.get()?.path.clone()
}

/// The newest `count` log records, oldest first. Empty before a logger is
/// installed.
#[expect(
    dead_code,
    reason = "the window's log panel reads the ring; it is not drawn yet"
)]
pub(crate) fn recent_lines(count: usize) -> Vec<String> {
    LOGGER
        .get()
        .map_or_else(Vec::new, |logger| logger.recent(count))
}

/// Flushes the session file, waiting up to `timeout`. Returns whether the
/// writer confirmed. For a clean exit and the crash report.
pub(crate) fn flush(timeout: Duration) -> bool {
    LOGGER
        .get()
        .is_none_or(|logger| logger.flush_within(timeout))
}

/// Makes a panic visible in the window app.
///
/// The release build has no console and aborts on panic, so without this a
/// bug looks like the window vanishing with no word of why (the message
/// went to a stderr nobody has). The hook appends the message, where it
/// happened, the last log lines and a backtrace to [`crash_log_path`],
/// flushes the session log, shows it in a dialog on Windows, then lets the
/// default hook run (the abort follows).
pub(crate) fn install_crash_report() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let recent = LOGGER
            .get()
            .and_then(|logger| logger.try_recent(40))
            .unwrap_or_default();
        let report = format!(
            "{} {} crashed on thread '{}': {info}\n\nLast log lines:\n{}\n\n{}",
            crate::NAME,
            env!("CARGO_PKG_VERSION"),
            thread.name().unwrap_or("unnamed"),
            recent.join("\n"),
            std::backtrace::Backtrace::force_capture()
        );
        log::error!("{}", report.split("\n\n").next().unwrap_or_default());
        // A crash cannot wait long for a disk; what made it is written.
        flush(CRASH_FLUSH);
        let path = crash_log_path();
        let written = append_crash(&path, &report).then_some(path);
        #[cfg(windows)]
        {
            // Everything before the log lines: where, then the message.
            let first = report.split("\n\n").next().unwrap_or_default();
            let saved = written.map_or_else(String::new, |path| {
                format!("\n\nThe details were saved to {}", path.display())
            });
            let session = log_file().map_or_else(String::new, |path| {
                format!("\nThe session log is {}", path.display())
            });
            crate::win::dialog::show_error(
                &format!("{first}{saved}{session}\n\nPlease report this."),
                crate::NAME,
            );
        }
        #[cfg(not(windows))]
        drop(written);
        default(info);
    }));
}

/// Appends one report to the crash log. Returns whether it was written.
fn append_crash(path: &Path, report: &str) -> bool {
    let Some(parent) = path.parent() else {
        return false;
    };
    if fs::create_dir_all(parent).is_err() {
        return false;
    }
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return false;
    };
    writeln!(file, "---- {}\n{report}", utc_iso(SystemTime::now())).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, bail};

    fn file_logger(dir: &Path, spec: &str, cap: u64) -> Logger {
        Logger::new(Config {
            spec: Some(spec.to_owned()),
            dir: Some(dir.to_path_buf()),
            stderr: false,
            max_file_bytes: cap,
        })
    }

    fn record_at(logger: &Logger, level: log::Level, target: &str, message: &str) {
        logger.log(
            &Record::builder()
                .level(level)
                .target(target)
                .args(format_args!("{message}"))
                .build(),
        );
    }

    #[test]
    fn a_record_is_one_line_with_its_continuations_indented() {
        let line = format_record(
            Duration::from_millis(12_345),
            log::Level::Warn,
            "main",
            "aivideoeditor::render",
            "first\nsecond",
        );
        assert_eq!(
            line, "   12.345s WARN  [main] aivideoeditor::render: first\n    second",
            "uptime, level, thread, target, indented body"
        );
    }

    #[test]
    fn the_session_file_takes_filtered_lines_and_a_header() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let logger = file_logger(
            dir.path(),
            "warn,aivideoeditor::render=debug",
            MAX_FILE_BYTES,
        );
        record_at(&logger, log::Level::Info, "aivideoeditor::index", "hidden");
        record_at(&logger, log::Level::Debug, "aivideoeditor::render", "shown");
        record_at(&logger, log::Level::Error, "eframe", "also shown");
        assert!(
            logger.flush_within(Duration::from_secs(5)),
            "the writer flushed"
        );
        let Some(path) = logger.path.clone() else {
            bail!("no session file was opened in {}", dir.path().display());
        };
        let text = fs::read_to_string(&path)?;
        assert!(
            text.starts_with("=== AI Video Editor"),
            "a header first: {text}"
        );
        assert!(
            text.contains("log filter: warn,aivideoeditor::render=debug"),
            "{text}"
        );
        assert!(text.contains("aivideoeditor::render: shown"), "{text}");
        assert!(text.contains("eframe: also shown"), "{text}");
        assert!(!text.contains("hidden"), "filtered out by RUST_LOG: {text}");
        let file_lines: Vec<String> = text.lines().skip(2).map(str::to_owned).collect();
        assert_eq!(
            logger.recent(2),
            file_lines,
            "the ring holds the same lines as the file"
        );
        Ok(())
    }

    #[test]
    fn a_bad_filter_falls_back_to_info() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let logger = file_logger(dir.path(), "aivideoeditor=loud", MAX_FILE_BYTES);
        record_at(&logger, log::Level::Info, "x", "info passes");
        record_at(&logger, log::Level::Debug, "x", "debug does not");
        let lines = logger.recent(10).join("\n");
        assert!(lines.contains("info passes"), "{lines}");
        assert!(!lines.contains("debug does not"), "{lines}");
        Ok(())
    }

    #[test]
    fn the_session_file_stops_at_its_cap_and_says_so() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let logger = file_logger(dir.path(), "info", 400);
        for index in 0..40 {
            record_at(&logger, log::Level::Info, "x", &format!("line {index}"));
        }
        assert!(
            logger.flush_within(Duration::from_secs(5)),
            "the writer flushed"
        );
        let Some(path) = logger.path.clone() else {
            bail!("no session file");
        };
        let text = fs::read_to_string(&path)?;
        assert!(
            text.len() < 600,
            "capped near 400 bytes, got {}",
            text.len()
        );
        assert!(
            text.trim_end().ends_with("written here"),
            "ends saying why: {text}"
        );
        assert!(
            logger.recent(1).join("").contains("line 39"),
            "the ring keeps going after the file stops"
        );
        Ok(())
    }

    #[test]
    fn the_ring_keeps_the_newest_records() -> Result<()> {
        let logger = Logger::new(Config {
            spec: None,
            dir: None,
            stderr: false,
            max_file_bytes: MAX_FILE_BYTES,
        });
        for index in 0..(RING_LINES + 5) {
            record_at(&logger, log::Level::Info, "x", &format!("n{index}"));
        }
        let all = logger.recent(usize::MAX);
        assert_eq!(all.len(), RING_LINES, "bounded");
        assert!(
            all.first().is_some_and(|line| line.ends_with("n5")),
            "oldest evicted"
        );
        assert!(logger.path.is_none(), "no directory, no file");
        Ok(())
    }

    #[test]
    fn only_the_newest_sessions_are_kept_by_time_not_name() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for name in ["ave-b.log", "ave-a.log", "ave-c.log", "notes.txt"] {
            fs::write(dir.path().join(name), "x")?;
        }
        // `ave-c` is the oldest by time though it sorts last by name.
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        File::options()
            .write(true)
            .open(dir.path().join("ave-c.log"))?
            .set_modified(old)?;
        assert_eq!(
            prune_old_logs(dir.path(), 2),
            1,
            "one beyond the newest two"
        );
        assert!(
            !dir.path().join("ave-c.log").exists(),
            "the oldest by time went"
        );
        assert!(dir.path().join("notes.txt").exists(), "not a session log");
        Ok(())
    }

    #[test]
    fn equal_times_fall_back_to_name_order() {
        let at = SystemTime::UNIX_EPOCH;
        let items = vec![("c", at), ("a", at), ("b", at)];
        assert_eq!(beyond_newest(items, 1), vec!["a", "b"], "a then b go");
    }

    #[test]
    fn utc_stamps_match_known_dates() {
        let at = |seconds: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds);
        assert_eq!(utc_stamp(at(0)), "19700101T000000Z", "the epoch");
        assert_eq!(
            utc_iso(at(951_782_400)),
            "2000-02-29T00:00:00Z",
            "a leap day"
        );
        assert_eq!(
            utc_iso(at(1_795_665_661)),
            "2026-11-26T04:01:01Z",
            "a recent evening"
        );
    }

    #[test]
    fn logs_and_the_crash_report_sit_under_the_data_directory() {
        let base = data_dir();
        assert!(logs_dir().starts_with(&base), "logs outside {base:?}");
        assert!(
            crash_log_path().starts_with(&base),
            "crash log outside {base:?}"
        );
    }
}

// ============================================================================
// Module:       diagnose
// Description:  `ave --diagnose`: which build this is, what this machine has
//               (GPU adapters, ffmpeg, llama-server, claude) and where ave
//               keeps its settings, logs and crash report.
//
// Dependencies: eframe::wgpu (adapter list); crate::{media, caption, config,
//               logging, gui::assistant}; build.rs (AVE_BUILD_*)
// ============================================================================

//! The diagnose report.
//!
//! One block of plain text answering "what is this build and this machine?",
//! for a bug report and for the package check, which runs it from an
//! unpacked release zip. It never opens a window and never changes
//! anything: each probe runs a tool's version flag or reads a setting.
//!
//! The commit and its dirty flag are fixed when the binary is built
//! (`build.rs`), so a report from a user's copy names the exact source it
//! came from.

use std::fmt::Write as _;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

/// The crate version.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Abbreviated commit hash, or `unknown` when git was unavailable.
const GIT_HASH: &str = env!("AVE_BUILD_GIT_HASH");

/// Whether tracked files differed from `HEAD` when this was built.
const GIT_DIRTY: bool = matches!(env!("AVE_BUILD_GIT_DIRTY").as_bytes(), b"1");

/// The target triple.
const TARGET: &str = env!("AVE_BUILD_TARGET");

/// The cargo profile by name (`debug`, `release`, `release-fast`).
const PROFILE: &str = env!("AVE_BUILD_PROFILE");

/// Environment variables that change what ave does, reported set or not.
const ENV_FLAGS: [&str; 9] = [
    "RUST_LOG",
    "AVE_DATA_DIR",
    "AVE_FFMPEG_DIR",
    "AVE_LLAMA_DIR",
    "AVE_VLM_URL",
    "AVE_VLM_MODEL",
    "AVE_VLM_MMPROJ",
    "AVE_CLAUDE",
    "RUST_BACKTRACE",
];

/// How long the GPU adapter list may take before the report goes on
/// without it. Creating a Vulkan and a D3D12 instance is usually well
/// under a second; a broken driver must not hang a diagnosis.
const GPU_BUDGET: Duration = Duration::from_secs(10);

/// One line naming the build, such as
/// `0.1.0 (3f9a1c2b4d5e+dirty, release, x86_64-pc-windows-msvc)`.
pub(crate) fn build_summary() -> String {
    let dirty = if GIT_DIRTY { "+dirty" } else { "" };
    format!("{VERSION} ({GIT_HASH}{dirty}, {PROFILE}, {TARGET})")
}

/// The whole report, probing the machine now. Takes a moment: it starts
/// ffmpeg, ffprobe and the GPU drivers.
pub(crate) fn report() -> String {
    let mut text = String::new();
    section(&mut text, "Build");
    row(&mut text, "version", VERSION);
    let dirty = if GIT_DIRTY {
        " (uncommitted changes)"
    } else {
        ""
    };
    row(&mut text, "commit", &format!("{GIT_HASH}{dirty}"));
    row(&mut text, "profile", PROFILE);
    row(&mut text, "target", TARGET);
    row(&mut text, "os", std::env::consts::OS);

    section(&mut text, "Graphics");
    let adapters = gpu_adapters();
    if adapters.is_empty() {
        row(&mut text, "adapter", "none reported");
    }
    for adapter in adapters {
        row(&mut text, "adapter", &adapter);
    }

    section(&mut text, "Tools");
    for name in ["ffmpeg", "ffprobe"] {
        let found = tool_version(&crate::media::tool(name));
        row(
            &mut text,
            name,
            &found.unwrap_or_else(|| "not found".to_owned()),
        );
    }
    row(
        &mut text,
        "captions",
        &crate::caption::not_set_up().unwrap_or_else(|| "set up".to_owned()),
    );
    row(
        &mut text,
        "claude",
        &crate::gui::assistant::claude_code_exe().map_or_else(
            || "not found on PATH".to_owned(),
            |path| path.display().to_string(),
        ),
    );

    section(&mut text, "Environment");
    for name in ENV_FLAGS {
        let value = std::env::var(name).unwrap_or_else(|_| "(unset)".to_owned());
        row(&mut text, name, &value);
    }

    section(&mut text, "Directories");
    let shown = |path: Option<std::path::PathBuf>| {
        path.map_or_else(|| "(none)".to_owned(), |path| path.display().to_string())
    };
    row(&mut text, "config", &shown(crate::config::config_path()));
    row(
        &mut text,
        "data",
        &crate::logging::data_dir().display().to_string(),
    );
    row(
        &mut text,
        "logs",
        &crate::logging::logs_dir().display().to_string(),
    );
    row(
        &mut text,
        "crash log",
        &crate::logging::crash_log_path().display().to_string(),
    );
    row(&mut text, "this log", &shown(crate::logging::log_file()));
    text
}

/// The first line of `<exe> -version`, or `None` when it does not run.
fn tool_version(exe: &Path) -> Option<String> {
    let mut command = std::process::Command::new(exe);
    command
        .arg("-version")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    crate::media::quiet(&mut command);
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::to_owned)
}

/// Every GPU adapter wgpu can see, as `name (backend, type)`, or a line
/// saying why there is no list. Runs on its own thread so a driver that
/// hangs costs [`GPU_BUDGET`], not the report.
fn gpu_adapters() -> Vec<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("ave-diagnose-gpu".to_owned())
        .spawn(move || {
            // The receiver gives up after the budget; a late answer is
            // simply not wanted any more.
            tx.send(list_adapters()).ok();
        });
    if let Err(error) = spawned {
        return vec![format!("not listed: {error}")];
    }
    rx.recv_timeout(GPU_BUDGET).unwrap_or_else(|_| {
        vec![format!(
            "not listed: the drivers took longer than {} s",
            GPU_BUDGET.as_secs()
        )]
    })
}

/// [`gpu_adapters`]' probe. On native platforms wgpu's adapter list is
/// ready the first time its future is polled, so it is polled once rather
/// than run on an executor.
fn list_adapters() -> Vec<String> {
    use eframe::wgpu;
    use std::future::Future as _;
    let instance = wgpu::Instance::default();
    let future = std::pin::pin!(instance.enumerate_adapters(wgpu::Backends::all()));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.poll(&mut context) {
        std::task::Poll::Ready(adapters) => adapters
            .iter()
            .map(|adapter| {
                let info = adapter.get_info();
                format!("{} ({:?}, {:?})", info.name, info.backend, info.device_type)
            })
            .collect(),
        std::task::Poll::Pending => vec!["not listed: the adapter list was not ready".to_owned()],
    }
}

fn section(text: &mut String, title: &str) {
    if !text.is_empty() {
        text.push('\n');
    }
    writeln!(text, "{title}\n{}", "-".repeat(title.len())).ok();
}

fn row(text: &mut String, key: &str, value: &str) {
    writeln!(text, "  {key:<14} {value}").ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_names_version_commit_profile_and_target() {
        let line = build_summary();
        for part in [VERSION, GIT_HASH, PROFILE, TARGET] {
            assert!(line.contains(part), "`{part}` missing from {line}");
        }
    }

    #[test]
    fn a_row_and_a_section_line_up() {
        let mut text = String::new();
        section(&mut text, "Tools");
        row(&mut text, "ffmpeg", "not found");
        assert_eq!(
            text, "Tools\n-----\n  ffmpeg         not found\n",
            "a ruled title, then indented key-value rows"
        );
    }

    #[test]
    fn a_missing_tool_has_no_version() {
        assert_eq!(
            tool_version(Path::new("ave-no-such-tool-anywhere")),
            None,
            "a tool that does not start reports nothing, not an error"
        );
    }
}

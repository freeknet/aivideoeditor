// ============================================================================
// Module:       build (build script)
// Description:  Embeds the Windows resources (the application icon, the
//               manifest, the version information Explorer shows) and
//               records which commit the binary was built from.
//
// Dependencies: winresource (does nothing for a non-Windows target);
//               std::process (git)
// ============================================================================

//! Embeds the Windows resources into the executable, and the build's
//! identity into the program.
//!
//! Three resources, and all of them are invisible until they are missing.
//!
//! - **The icon.** The window icon is set at runtime, but Explorer, the
//!   taskbar and a pinned shortcut read an icon *resource* out of the
//!   `.exe`; without one they show the generic blank-page icon.
//!   `assets/aivideoeditor.ico` is regenerated from `src/brand.rs` by
//!   `cargo run --example brand_assets`.
//! - **The manifest** (`assets/aivideoeditor.manifest`). Without one, Windows
//!   bitmap-scales the window on a high-DPI display (a blurry app), reports
//!   Windows 8 to every version check, and gives the process the legacy
//!   code page instead of UTF-8.
//! - **The version information** the Properties dialog and Task Manager
//!   read, taken from `Cargo.toml` so it cannot disagree with the crate.
//!
//! The build identity (`AVE_BUILD_GIT_HASH`, `AVE_BUILD_GIT_DIRTY`,
//! `AVE_BUILD_TARGET`, `AVE_BUILD_PROFILE`) is what `ave --diagnose`
//! prints, so a report from a user's copy names the exact source. A
//! checkout without git (a source archive) is not an error: the hash is
//! `unknown` and the build reads as clean. `AVE_GIT_HASH` overrides the
//! lookup for a pipeline that builds an exported tree.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/aivideoeditor.ico");
    println!("cargo:rerun-if-changed=assets/aivideoeditor.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    build_identity();
    resources();
}

/// The stdout of `git <args>`, trimmed, or `None` when git is missing or
/// fails.
fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|text| text.trim().to_owned())
}

/// Records the commit, the dirty flag, the target and the profile.
fn build_identity() {
    println!("cargo:rerun-if-env-changed=AVE_GIT_HASH");
    let hash = std::env::var("AVE_GIT_HASH")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| git(&["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_owned());
    // Tracked files only: build outputs and scratch files are not "dirty".
    // `--no-optional-locks` keeps `git status` from rewriting the index.
    let dirty = git(&[
        "--no-optional-locks",
        "status",
        "--porcelain",
        "--untracked-files=no",
    ])
    .is_some_and(|status| !status.is_empty());

    // Re-run when the commit moves: HEAD changes on a checkout, and its
    // reflog gains a line on every commit, reset or merge. Not the index,
    // which git rewrites whenever anything refreshes its stat cache, and
    // every rewrite would relink the app. Only paths that exist are named,
    // since cargo treats a missing one as always changed. `--git-path`
    // resolves worktrees, where `.git` is a file.
    for name in ["HEAD", "logs/HEAD"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]).map(PathBuf::from)
            && path.exists()
        {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    println!("cargo:rustc-env=AVE_BUILD_GIT_HASH={hash}");
    println!("cargo:rustc-env=AVE_BUILD_GIT_DIRTY={}", u8::from(dirty));
    println!(
        "cargo:rustc-env=AVE_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned())
    );
    println!("cargo:rustc-env=AVE_BUILD_PROFILE={}", profile());
}

/// The cargo profile by name. Cargo's `PROFILE` says only `release` or
/// `debug`, so a `release-fast` build would report itself as the shipping
/// one. The name is the folder cargo builds into,
/// `target/[<triple>/]<profile>/build/<package>-<hash>/out`, three levels
/// above `OUT_DIR`; `PROFILE` when that cannot be read.
fn profile() -> String {
    let broad = std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_owned());
    std::env::var_os("OUT_DIR")
        .map(PathBuf::from)
        .and_then(|out| {
            out.ancestors()
                .nth(3)
                .and_then(|dir| dir.file_name())
                .map(|name| name.to_string_lossy().into_owned())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or(broad)
}

/// Compiles the icon, manifest and version block into the executable.
fn resources() {
    // The *target*, not the host: `cfg(windows)` in a build script describes
    // the machine running it.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("assets/aivideoeditor.ico");
    resource.set_manifest_file("assets/aivideoeditor.manifest");
    // `FileDescription` is what Task Manager and Explorer show as the
    // program's name, so it holds the product name, not a sentence.
    resource.set("ProductName", "AI Video Editor");
    resource.set("FileDescription", "AI Video Editor");
    resource.set("Comments", env!("CARGO_PKG_DESCRIPTION"));
    resource.set("LegalCopyright", "Copyright (c) 2026 Lawrence Norton");

    if let Err(error) = resource.compile() {
        if std::env::var("PROFILE").as_deref() == Ok("release") {
            eprintln!("could not embed the Windows resources: {error}");
            std::process::exit(1);
        }
        println!("cargo:warning=could not embed the Windows resources: {error}");
    }
}

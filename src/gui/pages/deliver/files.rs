// ============================================================================
// Module:       gui::pages::deliver::files
// Description:  Showing delivered files in Explorer and copying renders to
//               their delivery names off the UI thread.
//
// Dependencies: std::process (explorer), crossbeam-channel
// ============================================================================

//! Files: reveal, open folder, and the background copy of a finished render
//! to the recipe's output folder under its delivery name.
//!
//! Explorer is started with `std::process::Command` (no shell, no unsafe).
//! `/select,` needs the path quoted as one token after the comma, which
//! Rust's own argument quoting does not produce, so on Windows the argument
//! is passed raw.

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;

/// A path with Windows separators, for Explorer.
#[cfg(windows)]
fn native(path: &Path) -> String {
    path.display().to_string().replace('/', "\\")
}

/// Opens `dir` in the file manager.
pub(crate) fn open_folder(dir: &Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer")
        .arg(native(dir))
        .spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
}

/// Shows `file` selected in its folder.
pub(crate) fn reveal(file: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        let _ = std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", native(file)))
            .spawn();
    }
    #[cfg(not(windows))]
    if let Some(dir) = file.parent() {
        open_folder(dir);
    }
}

/// A copy running on its own thread.
#[derive(Debug)]
pub(crate) struct Copying {
    /// Where it is going.
    pub(crate) to: PathBuf,
    /// The result: bytes copied, or the error.
    rx: Receiver<Result<u64, String>>,
}

impl Copying {
    /// Starts copying `from` to `to` (creating `to`'s folder, replacing
    /// `to`) through a temporary file, so a half-copied video never sits
    /// under the delivery name.
    pub(crate) fn start(from: PathBuf, to: PathBuf, ctx: Option<egui::Context>) -> Self {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let dest = to.clone();
        std::thread::spawn(move || {
            let result = copy(&from, &dest).map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        Self { to, rx }
    }

    /// The result, once the copy has finished.
    pub(crate) fn poll(&self) -> Option<Result<u64, String>> {
        self.rx.try_recv().ok()
    }
}

/// Copies `from` to `to` through `to.part` and a rename.
pub(crate) fn copy(from: &Path, to: &Path) -> anyhow::Result<u64> {
    use anyhow::Context as _;
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let part = to.with_extension("part");
    let bytes = std::fs::copy(from, &part)
        .with_context(|| format!("copying {} to {}", from.display(), part.display()))?;
    if to.exists() {
        std::fs::remove_file(to).with_context(|| format!("replacing {}", to.display()))?;
    }
    std::fs::rename(&part, to).with_context(|| format!("naming {}", to.display()))?;
    Ok(bytes)
}

/// A file size as people read it.
pub(crate) fn size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let mb = bytes as f64 / MB;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else if mb >= 1.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{} KB", bytes / 1024)
    }
}

#[cfg(test)]
mod tests {
    use super::{copy, size};

    #[test]
    fn a_copy_replaces_the_old_file_and_leaves_no_part() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let from = dir.path().join("renders").join("reel-30.mp4");
        std::fs::create_dir_all(dir.path().join("renders"))?;
        std::fs::write(&from, b"new")?;
        let to = dir.path().join("videos").join("reel-30.mp4");
        std::fs::create_dir_all(dir.path().join("videos"))?;
        std::fs::write(&to, b"old render")?;
        let bytes = copy(&from, &to)?;
        assert_eq!(bytes, 3, "bytes copied");
        assert_eq!(std::fs::read(&to)?, b"new", "replaced");
        assert!(!to.with_extension("part").exists(), "no temporary left");
        Ok(())
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size(2048), "2 KB");
        assert_eq!(size(50 * 1024 * 1024), "50 MB");
        assert_eq!(size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}

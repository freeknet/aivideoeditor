// ============================================================================
// Module:       gui::pages::deliver::files
// Description:  Showing delivered files in Explorer, copying renders to
//               their delivery names off the UI thread, and the manifest of
//               the files ave delivered.
//
// Dependencies: std::process (explorer), crossbeam-channel, serde_json
// ============================================================================

//! Files: reveal, open folder, the background copy of a finished render
//! to the recipe's output folder under its delivery name, and the
//! [`Manifest`] that tells a video ave delivered from one it did not make.
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

/// The manifest's file name inside `<work>/renders`.
const MANIFEST: &str = "delivered.json";

/// A delivered file as ave left it: its size and modification time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Stamp {
    /// Size in bytes.
    bytes: u64,
    /// Modification time, whole seconds since the Unix epoch.
    secs: u64,
    /// And the nanoseconds past them.
    nanos: u32,
}

impl Stamp {
    /// The stamp of the file at `path` now, if it is a file.
    fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
        let since = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        Some(Self {
            bytes: meta.len(),
            secs: since.as_secs(),
            nanos: since.subsec_nanos(),
        })
    }
}

/// Who a delivery name's file belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// Nothing there yet.
    Absent,
    /// ave copied it there and it has not changed since.
    Ave,
    /// Something else is there: the user's own edit, an export from another
    /// program, or an ave render changed since. Never replaced unasked.
    Other,
}

/// The files ave delivered, in `<work>/renders/delivered.json`, each with
/// the size and time it had when ave wrote it. A file in the output folder
/// that is not listed here, or no longer matches, is someone else's, so a
/// render never silently replaces a video the user made by hand.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Manifest {
    /// Delivered files by full path.
    #[serde(default)]
    files: std::collections::BTreeMap<String, Stamp>,
}

impl Manifest {
    /// Where the manifest of the night at `work` lives.
    pub(crate) fn path(work: &Path) -> PathBuf {
        work.join("renders").join(MANIFEST)
    }

    /// The manifest of the night at `work`; empty when there is none or it
    /// does not parse (which only costs the "delivered by ave" marks).
    pub(crate) fn load(work: &Path) -> Self {
        std::fs::read_to_string(Self::path(work))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Writes the manifest through a temporary file and a rename.
    pub(crate) fn save(&self, work: &Path) -> anyhow::Result<()> {
        use anyhow::Context as _;
        let path = Self::path(work);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Notes `file` as delivered by ave, as it is now.
    pub(crate) fn record(&mut self, file: &Path) {
        if let Some(stamp) = Stamp::of(file) {
            self.files.insert(key(file), stamp);
        }
    }

    /// Who the file at `file` belongs to.
    pub(crate) fn owner(&self, file: &Path) -> Owner {
        match Stamp::of(file) {
            None => Owner::Absent,
            Some(now) if self.files.get(&key(file)) == Some(&now) => Owner::Ave,
            Some(_) => Owner::Other,
        }
    }

    /// A name beside `dest` that replaces nothing ave did not make:
    /// `reel-30-ave.mp4`, else `reel-30-ave-2.mp4`, and so on.
    pub(crate) fn free_name(&self, dest: &Path) -> PathBuf {
        let stem = dest
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = dest
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let dir = dest.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut n = 1_u32;
        loop {
            let name = if n == 1 {
                format!("{stem}-ave{ext}")
            } else {
                format!("{stem}-ave-{n}{ext}")
            };
            let candidate = dir.join(name);
            if self.owner(&candidate) != Owner::Other || n >= 99 {
                return candidate;
            }
            n += 1;
        }
    }
}

/// A path as the manifest keys it: separators made one kind, so a work
/// folder written with forward slashes finds the same entry.
fn key(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
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
    fn only_a_file_ave_delivered_and_left_alone_is_its_own() -> anyhow::Result<()> {
        use super::{Manifest, Owner};
        let dir = tempfile::tempdir()?;
        let work = dir.path().join("_ave");
        let videos = dir.path().join("videos");
        std::fs::create_dir_all(&videos)?;
        let ours = videos.join("reel-30.mp4");
        let theirs = videos.join("reel-15.mp4");
        std::fs::write(&ours, b"render")?;
        std::fs::write(&theirs, b"hand edit")?;
        let mut manifest = Manifest::default();
        manifest.record(&ours);
        manifest.save(&work)?;
        let back = Manifest::load(&work);
        assert_eq!(back, manifest, "round trip");
        assert_eq!(back.owner(&ours), Owner::Ave, "delivered by ave");
        assert_eq!(back.owner(&theirs), Owner::Other, "not in the manifest");
        assert_eq!(
            back.owner(&videos.join("recap.mp4")),
            Owner::Absent,
            "nothing there"
        );
        std::fs::write(&ours, b"edited since")?;
        assert_eq!(
            back.owner(&ours),
            Owner::Other,
            "changed since ave wrote it"
        );
        assert_eq!(
            back.free_name(&theirs),
            videos.join("reel-15-ave.mp4"),
            "a name beside it"
        );
        std::fs::write(videos.join("reel-15-ave.mp4"), b"also theirs")?;
        assert_eq!(
            back.free_name(&theirs),
            videos.join("reel-15-ave-2.mp4"),
            "past a taken one"
        );
        Ok(())
    }

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size(2048), "2 KB");
        assert_eq!(size(50 * 1024 * 1024), "50 MB");
        assert_eq!(size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}

// ============================================================================
// Module:       package (example)
// Description:  Builds the release zip and its symbols zip from a build, and
//               checks a zip by unpacking it outside the checkout and running
//               the program it holds.
//
// Dependencies: anyhow; Windows' tar.exe (zip) and PowerShell (the version
//               resource); cargo (the credits example, for the notices)
// ============================================================================

//! The release package, built and checked by the same code in CI and in
//! the release workflow.
//!
//! ```sh
//! cargo build --release --locked --bins
//! cargo run --example package -- package --out dist
//! cargo run --example package -- check dist/aivideoeditor-v0.1.0-dev-x86_64-pc-windows-msvc.zip \
//!     --symbols dist/aivideoeditor-v0.1.0-dev-x86_64-pc-windows-msvc-symbols.zip
//! ```
//!
//! `package` stages `ave.exe`, the docs, the theme catalog, the font
//! licences, `THIRD_PARTY.md` and the generated `THIRD-PARTY-NOTICES.txt`
//! into one folder and zips it, and puts `ave.pdb` in a zip of its own.
//!
//! `check` takes any zip, unpacks it into a fresh folder in the system temp
//! directory (never the checkout, so nothing it runs can find a file a user
//! would not have), and checks:
//!
//! - **the file list**: exactly what [`EXPECTED`] names, nothing missing and
//!   nothing extra;
//! - **the notices** are the full generated file, not an empty one;
//! - **the version resource** on `ave.exe` names the version in the zip's
//!   name, so Explorer's Properties says which build it is;
//! - **the imports**: every DLL `ave.exe` imports is part of Windows, so a
//!   clean machine without the Visual C++ runtime starts it;
//! - **the symbols** zip holds `ave.pdb`;
//! - **it runs**: `ave.exe --version` and `ave.exe --diagnose` from the
//!   unpacked folder exit cleanly and say what they should.
//!
//! Windows only: the package is a Windows package, zipped with Windows' own
//! `tar.exe` (bsdtar, which writes and reads zip).

use anyhow::{Context as _, Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The platform the package is for.
const TRIPLE: &str = "x86_64-pc-windows-msvc";

/// Every entry at the top of the unpacked package.
const EXPECTED: [&str; 7] = [
    "ave.exe",
    "README.md",
    "LICENSE",
    "themes.toml",
    "fonts",
    "THIRD_PARTY.md",
    "THIRD-PARTY-NOTICES.txt",
];

fn main() -> Result<()> {
    if !Path::new("Cargo.toml").is_file() {
        bail!("run this from the crate root, where Cargo.toml lives");
    }
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        bail!("usage: package package [options] | package check <zip> [--symbols <zip>]");
    };
    let rest: Vec<String> = args.collect();
    match command.as_str() {
        "package" => {
            let (zip, symbols) = package(&PackageArgs::parse(&rest)?)?;
            println!("{}\n{}", zip.display(), symbols.display());
            Ok(())
        }
        "check" => run_check(&rest),
        other => bail!("unknown command `{other}`; expected `package` or `check`"),
    }
}

/// Options for `package`.
struct PackageArgs {
    /// The cargo profile whose `ave.exe` and `ave.pdb` are packaged.
    profile: String,
    /// The target the build was made for with `--target`, if any; the
    /// binaries are then under `target/<target>/<profile>`.
    target: Option<String>,
    /// The version in the archive's name, such as `v0.2.0`. Defaults to the
    /// crate version with a `-dev` suffix.
    tag: Option<String>,
    /// Where the staging folders and the two zips go.
    out: PathBuf,
}

impl PackageArgs {
    fn parse(args: &[String]) -> Result<Self> {
        let mut parsed = Self {
            profile: "release".to_owned(),
            target: None,
            tag: None,
            out: PathBuf::from("dist"),
        };
        let mut args = args.iter();
        while let Some(flag) = args.next() {
            let mut value = || {
                args.next()
                    .cloned()
                    .ok_or_else(|| anyhow!("{flag} needs a value"))
            };
            match flag.as_str() {
                "--profile" => parsed.profile = value()?,
                "--target" => parsed.target = Some(value()?),
                "--tag" => parsed.tag = Some(value()?),
                "--out" => parsed.out = PathBuf::from(value()?),
                other => bail!("unknown option `{other}`"),
            }
        }
        Ok(parsed)
    }
}

/// Stages the package and its symbols from a build and zips both. Returns
/// the two zips.
fn package(args: &PackageArgs) -> Result<(PathBuf, PathBuf)> {
    if !cfg!(windows) {
        bail!("the package is a Windows package; build it on Windows");
    }
    let tag = args
        .tag
        .clone()
        .unwrap_or_else(|| format!("v{}-dev", env!("CARGO_PKG_VERSION")));
    let triple = args.target.as_deref().unwrap_or(TRIPLE);
    let name = format!("aivideoeditor-{tag}-{triple}");
    let bin_dir = match &args.target {
        Some(target) => Path::new("target").join(target).join(&args.profile),
        None => Path::new("target").join(&args.profile),
    };

    std::fs::create_dir_all(&args.out)?;
    let out = std::path::absolute(&args.out)?;
    let staging = out.join(&name);
    let symbols = out.join(format!("{name}-symbols"));
    for folder in [&staging, &symbols] {
        if folder.exists() {
            // A leftover from an earlier run must not slip into this package.
            std::fs::remove_dir_all(folder)
                .with_context(|| format!("cannot clear {}", folder.display()))?;
        }
        std::fs::create_dir_all(folder)?;
    }

    copy(&bin_dir.join("ave.exe"), &staging.join("ave.exe"))?;
    copy(&bin_dir.join("ave.pdb"), &symbols.join("ave.pdb"))?;
    for doc in ["README.md", "LICENSE", "THIRD_PARTY.md"] {
        copy(Path::new(doc), &staging.join(doc))?;
    }
    // The theme catalog is compiled in, so the app runs without it, but it
    // is also the worked example anyone writing a theme starts from.
    copy(
        Path::new("assets/themes.toml"),
        &staging.join("themes.toml"),
    )?;
    // The bundled fonts are compiled in; their licences (OFL, ISC, MIT)
    // require the notices to travel with the binary.
    let fonts = staging.join("fonts");
    std::fs::create_dir_all(&fonts)?;
    for entry in std::fs::read_dir("assets/fonts")? {
        let path = entry?.path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
        let Some(name) = name else { continue };
        if name == "NOTICE" || name.ends_with(".txt") {
            copy(&path, &fonts.join(&name))?;
        }
    }
    notices(&staging.join("THIRD-PARTY-NOTICES.txt"))?;

    let zip = out.join(format!("{name}.zip"));
    let symbols_zip = out.join(format!("{name}-symbols.zip"));
    for (folder, archive) in [
        (name.clone(), &zip),
        (format!("{name}-symbols"), &symbols_zip),
    ] {
        if archive.exists() {
            // tar adds to an existing archive, so an old one goes first.
            std::fs::remove_file(archive)?;
        }
        tar(&["-a", "-c", "-f"], archive, &out, Some(&folder))?;
    }
    Ok((zip, symbols_zip))
}

/// Writes the full third-party notices with the credits example.
fn notices(path: &Path) -> Result<()> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .args([
            "run",
            "--locked",
            "--quiet",
            "--example",
            "credits",
            "--",
            "--notices",
        ])
        .arg(path)
        .status()
        .context("cannot run the credits example")?;
    if !status.success() {
        bail!("the credits example exited with {status}");
    }
    Ok(())
}

/// Copies one file, naming it on failure.
fn copy(from: &Path, to: &Path) -> Result<()> {
    std::fs::copy(from, to)
        .map(|_| ())
        .with_context(|| format!("cannot copy {} to {}", from.display(), to.display()))
}

/// Windows' own `tar.exe`, not whatever `tar` is first on `PATH`: Git
/// Bash's GNU tar cannot write a zip.
fn tar_exe() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("tar.exe"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("tar"))
}

/// Runs `tar <flags> <archive> -C <dir> [member]`. `-a` with a `.zip` name
/// writes a zip; `-x` reads one.
fn tar(flags: &[&str], archive: &Path, dir: &Path, member: Option<&str>) -> Result<()> {
    let mut command = Command::new(tar_exe());
    command.args(flags).arg(archive).arg("-C").arg(dir);
    if let Some(member) = member {
        command.arg(member);
    }
    let status = command.status().context("failed to run tar")?;
    if !status.success() {
        bail!("tar exited with {status} for {}", archive.display());
    }
    Ok(())
}

/// What a check found.
#[derive(Default)]
struct Checked {
    passed: Vec<String>,
    problems: Vec<String>,
}

impl Checked {
    /// Records a check as passed or as a problem.
    fn note(&mut self, ok: bool, passed: String, problem: String) {
        if ok {
            self.passed.push(passed);
        } else {
            self.problems.push(problem);
        }
    }
}

/// `check <zip> [--symbols <zip>]`: fails on any problem.
fn run_check(args: &[String]) -> Result<()> {
    let mut zip: Option<PathBuf> = None;
    let mut symbols: Option<PathBuf> = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--symbols" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow!("--symbols needs a zip"))?;
                symbols = Some(PathBuf::from(value));
            }
            other if zip.is_none() && !other.starts_with('-') => zip = Some(PathBuf::from(other)),
            other => bail!("unexpected argument `{other}`"),
        }
    }
    let Some(zip) = zip else {
        bail!("usage: package check <zip> [--symbols <zip>]");
    };
    let zip = std::path::absolute(&zip)?;
    let symbols = symbols.map(std::path::absolute).transpose()?;
    let work = scratch("package-check")?;
    let checked = check(&zip, symbols.as_deref(), &work)?;
    for line in &checked.passed {
        println!("ok       {line}");
    }
    for line in &checked.problems {
        println!("PROBLEM  {line}");
    }
    if !checked.problems.is_empty() {
        // The unpacked folder stays for a look at what went wrong.
        bail!(
            "{} problem(s) with {} (unpacked in {})",
            checked.problems.len(),
            zip.display(),
            work.display()
        );
    }
    std::fs::remove_dir_all(&work).with_context(|| format!("cannot remove {}", work.display()))?;
    println!("\n{} is what a user should get.", zip.display());
    Ok(())
}

/// A fresh folder in the system temp directory, outside the checkout.
fn scratch(name: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("ave-{name}-{}", std::process::id()));
    if dir.exists() {
        // A leftover from a crashed run with this id is not this run's.
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Unpacks `zip` (and its symbols) under `work` and checks it.
fn check(zip: &Path, symbols: Option<&Path>, work: &Path) -> Result<Checked> {
    if !cfg!(windows) {
        bail!("the package is a Windows package; check it on Windows");
    }
    let unpacked = work.join("package");
    std::fs::create_dir_all(&unpacked)?;
    tar(&["-x", "-f"], zip, &unpacked, None)
        .with_context(|| format!("cannot unpack {}", zip.display()))?;
    let tops: Vec<PathBuf> = std::fs::read_dir(&unpacked)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    let [root] = tops.as_slice() else {
        bail!(
            "{} should hold one folder, and holds {}",
            zip.display(),
            tops.len()
        );
    };
    let mut checked = Checked::default();

    // The file list, both ways.
    let present: Vec<String> = std::fs::read_dir(root)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let missing: Vec<&str> = EXPECTED
        .iter()
        .copied()
        .filter(|name| !present.iter().any(|p| p == name))
        .collect();
    let extra: Vec<&String> = present
        .iter()
        .filter(|p| !EXPECTED.contains(&p.as_str()))
        .collect();
    checked.note(
        missing.is_empty() && extra.is_empty(),
        format!("the file list ({} entries)", EXPECTED.len()),
        format!("missing {missing:?}, unexpected {extra:?}"),
    );
    let font_notice = root.join("fonts").join("NOTICE").is_file();
    checked.note(
        font_notice,
        "the font licences are in fonts/".to_owned(),
        "fonts/NOTICE is missing".to_owned(),
    );

    // The notices are the generated file.
    let notices = std::fs::read_to_string(root.join("THIRD-PARTY-NOTICES.txt")).unwrap_or_default();
    checked.note(
        notices.len() > 10_000 && notices.contains("egui"),
        "the third-party notices are the full generated file".to_owned(),
        format!(
            "THIRD-PARTY-NOTICES.txt is {} bytes and does not look generated",
            notices.len()
        ),
    );

    // The version resource names the release the zip says it is.
    let exe = root.join("ave.exe");
    let named = zip
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(version_in_name);
    let expected = named.unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned());
    let version = file_version(&exe).unwrap_or_else(|error| format!("unreadable ({error:#})"));
    checked.note(
        version == expected || version.starts_with(&format!("{expected}.")),
        format!("ave.exe's version resource says {version}"),
        format!("ave.exe's version resource says `{version}`, the package is {expected}"),
    );

    // Every DLL the program imports is part of Windows. One that is not
    // (the MSVC runtime, say) would make a clean machine refuse to start
    // it, which no test on a developer's machine would notice.
    match imports(&exe) {
        Ok(names) => {
            let foreign: Vec<&String> = names.iter().filter(|name| !is_windows_dll(name)).collect();
            checked.note(
                foreign.is_empty(),
                format!(
                    "ave.exe's {} direct imports are all Windows' own",
                    names.len()
                ),
                format!("ave.exe imports DLLs Windows does not have: {foreign:?}"),
            );
        }
        Err(error) => checked
            .problems
            .push(format!("ave.exe's import table unreadable: {error:#}")),
    }

    // The symbols travel separately.
    if let Some(symbols) = symbols {
        let into = work.join("symbols");
        std::fs::create_dir_all(&into)?;
        tar(&["-x", "-f"], symbols, &into, None)
            .with_context(|| format!("cannot unpack {}", symbols.display()))?;
        let pdb = find(&into, "ave.pdb");
        let size = pdb
            .as_deref()
            .and_then(|path| std::fs::metadata(path).ok())
            .map_or(0, |meta| meta.len());
        checked.note(
            size > 0,
            format!("the symbols zip holds ave.pdb ({} KiB)", size / 1024),
            "the symbols zip holds no ave.pdb".to_owned(),
        );
    }

    // It runs, from where a user would unpack it, with its data kept in
    // the scratch folder rather than the profile of whoever runs this.
    let data = work.join("data");
    match run(&exe, "--version", &data) {
        Ok(output) => checked.note(
            output.contains(env!("CARGO_PKG_VERSION")) || named_matches(&output, zip),
            format!("`ave --version` says {}", output.trim()),
            format!("`ave --version` printed `{}`", output.trim()),
        ),
        Err(error) => checked
            .problems
            .push(format!("`ave --version` failed: {error:#}")),
    }
    match run(&exe, "--diagnose", &data) {
        Ok(report) => {
            println!("{report}");
            checked.note(
                report.contains("Build") && report.contains("Directories"),
                "`ave --diagnose` ran and printed its report".to_owned(),
                format!("`ave --diagnose` printed an incomplete report:\n{report}"),
            );
        }
        Err(error) => checked
            .problems
            .push(format!("`ave --diagnose` failed: {error:#}")),
    }
    Ok(checked)
}

/// Whether `output` names the version in the zip's name (a release
/// checked from an older checkout).
fn named_matches(output: &str, zip: &Path) -> bool {
    zip.file_name()
        .and_then(|name| name.to_str())
        .and_then(version_in_name)
        .is_some_and(|version| output.contains(&version))
}

/// The `MAJOR.MINOR.PATCH` in a package's name,
/// `aivideoeditor-v0.2.1-<triple>.zip`; a `-dev` suffix is not part of it.
fn version_in_name(name: &str) -> Option<String> {
    let rest = name.strip_prefix("aivideoeditor-v")?;
    let version: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let parts = version.split('.').filter(|p| !p.is_empty()).count();
    (parts == 3).then_some(version.trim_end_matches('.').to_owned())
}

/// The first file called `name` under `dir`.
fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    let mut pending = vec![dir.to_path_buf()];
    while let Some(folder) = pending.pop() {
        for entry in std::fs::read_dir(&folder).ok()?.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if entry.file_name() == name {
                return Some(path);
            }
        }
    }
    None
}

/// Runs `exe <flag>` from its own folder and returns its stdout, giving up
/// after a minute.
fn run(exe: &Path, flag: &str, data: &Path) -> Result<String> {
    std::fs::create_dir_all(data)?;
    let out = data.join(format!("{}.out", flag.trim_start_matches('-')));
    let mut child = Command::new(exe)
        .arg(flag)
        .current_dir(exe.parent().context("the program has no folder")?)
        .env("AVE_DATA_DIR", data)
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&out)?)
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("cannot start {}", exe.display()))?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > Duration::from_secs(60) {
            // It is being abandoned; the error says so whether or not the
            // kill landed.
            child.kill().ok();
            bail!("still running after 60 s; stopped");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    if !status.success() {
        bail!("exited with {status}:\n{text}");
    }
    Ok(text)
}

/// The `ProductVersion` of a Windows executable's version resource, read
/// by PowerShell, which knows the resource format.
fn file_version(exe: &Path) -> Result<String> {
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "(Get-Item -LiteralPath '{}').VersionInfo.ProductVersion",
            exe.display()
        ))
        .output()
        .context("failed to run PowerShell")?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if !output.status.success() || text.is_empty() {
        bail!("no version resource");
    }
    Ok(text)
}

/// A little-endian `u32` at `at`, or an error naming what was cut short.
fn u32_at(bytes: &[u8], at: usize, what: &str) -> Result<u32> {
    bytes
        .get(at..at.saturating_add(4))
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| anyhow!("{what}: cut short at byte {at}"))
}

/// A little-endian `u16` at `at`.
fn u16_at(bytes: &[u8], at: usize, what: &str) -> Result<u16> {
    bytes
        .get(at..at.saturating_add(2))
        .and_then(|b| b.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| anyhow!("{what}: cut short at byte {at}"))
}

/// A `u32` read from a file as a `usize` offset.
fn offset(bytes: &[u8], at: usize, what: &str) -> Result<usize> {
    usize::try_from(u32_at(bytes, at, what)?).context("offset does not fit")
}

/// A PE image's bytes and where its data directories and sections are.
struct Pe {
    bytes: Vec<u8>,
    /// File offset of the first data directory.
    directories: usize,
    /// File offset of the section table, and how many sections it holds.
    sections: (usize, usize),
}

impl Pe {
    /// Reads and checks the headers of a PE file.
    fn read(path: &Path) -> Result<Self> {
        Self::parse(std::fs::read(path)?)
    }

    fn parse(bytes: Vec<u8>) -> Result<Self> {
        let header = offset(&bytes, 0x3c, "the DOS header")?;
        if bytes.get(header..header.saturating_add(4)) != Some(b"PE\0\0") {
            bail!("not a PE file");
        }
        let coff = header + 4;
        let count = usize::from(u16_at(&bytes, coff + 2, "the COFF header")?);
        let optional_size = usize::from(u16_at(&bytes, coff + 16, "the COFF header")?);
        let optional = coff + 20;
        // The data directories start after the fields that differ between
        // PE32 (0x10b) and PE32+ (0x20b).
        let directories = optional
            + match u16_at(&bytes, optional, "the optional header")? {
                0x20b => 112,
                0x10b => 96,
                other => bail!("unknown optional header magic {other:#x}"),
            };
        Ok(Self {
            bytes,
            directories,
            sections: (optional + optional_size, count),
        })
    }

    /// Data directory `index`'s RVA.
    fn directory(&self, index: usize) -> Result<usize> {
        offset(
            &self.bytes,
            self.directories + index * 8,
            "a data directory",
        )
    }

    /// The file offset an RVA maps to, through the section table.
    fn file_offset(&self, rva: usize) -> Result<Option<usize>> {
        let (table, count) = self.sections;
        for index in 0..count {
            let section = table + index * 40;
            let size = offset(&self.bytes, section + 8, "a section")?;
            let start = offset(&self.bytes, section + 12, "a section")?;
            let raw = offset(&self.bytes, section + 20, "a section")?;
            if (start..start.saturating_add(size)).contains(&rva) {
                return Ok(Some(raw + (rva - start)));
            }
        }
        Ok(None)
    }

    /// The NUL-terminated ASCII string at an RVA.
    fn string_at(&self, rva: usize) -> Result<String> {
        let at = self
            .file_offset(rva)?
            .context("a name outside every section")?;
        let tail = self.bytes.get(at..).context("a name past the end")?;
        let end = tail
            .iter()
            .position(|b| *b == 0)
            .context("an unterminated name")?;
        let name = tail.get(..end).context("a name past the end")?;
        Ok(String::from_utf8_lossy(name).into_owned())
    }
}

/// The DLLs a PE file imports directly, from its import directory (the
/// second data directory): one `IMAGE_IMPORT_DESCRIPTOR` of 20 bytes per
/// DLL, its name's RVA at byte 12, ended by an all-zero descriptor.
fn imports(exe: &Path) -> Result<Vec<String>> {
    let image = Pe::read(exe)?;
    let rva = image.directory(1)?;
    if rva == 0 {
        return Ok(Vec::new());
    }
    let mut at = image
        .file_offset(rva)?
        .context("the import directory is in no section")?;
    let mut names = Vec::new();
    loop {
        let descriptor = image
            .bytes
            .get(at..at.saturating_add(20))
            .context("an import descriptor past the end")?;
        if descriptor.iter().all(|b| *b == 0) {
            break;
        }
        names.push(image.string_at(offset(&image.bytes, at + 12, "an import name")?)?);
        at += 20;
    }
    Ok(names)
}

/// Whether a DLL a program imports is part of Windows itself, on every
/// supported version: the API sets, and the system libraries the Rust
/// standard library, winit, wgpu, cpal and the TLS stack link. Anything
/// else must ship in the package or be a documented prerequisite.
fn is_windows_dll(name: &str) -> bool {
    const SYSTEM: [&str; 34] = [
        "advapi32.dll",
        "bcrypt.dll",
        "bcryptprimitives.dll",
        "combase.dll",
        "comctl32.dll",
        "comdlg32.dll",
        "crypt32.dll",
        "d3d11.dll",
        "d3d12.dll",
        "d3dcompiler_47.dll",
        "dbghelp.dll",
        "dwmapi.dll",
        "dxgi.dll",
        "gdi32.dll",
        "imm32.dll",
        "kernel32.dll",
        "mmdevapi.dll",
        "ncrypt.dll",
        "ntdll.dll",
        "ole32.dll",
        "oleaut32.dll",
        "opengl32.dll",
        "propsys.dll",
        "secur32.dll",
        "setupapi.dll",
        "shell32.dll",
        "shlwapi.dll",
        "user32.dll",
        "userenv.dll",
        "uxtheme.dll",
        "version.dll",
        "winmm.dll",
        "ws2_32.dll",
        "ksuser.dll",
    ];
    let lower = name.to_ascii_lowercase();
    // `api-ms-win-*` and `ext-ms-win-*` are API sets the loader resolves
    // inside Windows. `api-ms-win-crt-*` is the Universal CRT, also part of
    // Windows 10 and later, but its presence means the C runtime was linked
    // dynamically, which `+crt-static` rules out, so it is not accepted.
    let api_set = (lower.starts_with("api-ms-win-") && !lower.starts_with("api-ms-win-crt-"))
        || lower.starts_with("ext-ms-win-");
    api_set || SYSTEM.contains(&lower.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_is_read_from_a_package_name() {
        assert_eq!(
            version_in_name("aivideoeditor-v0.2.1-x86_64-pc-windows-msvc.zip").as_deref(),
            Some("0.2.1"),
            "a release name"
        );
        assert_eq!(
            version_in_name("aivideoeditor-v0.1.0-dev-x86_64-pc-windows-msvc.zip").as_deref(),
            Some("0.1.0"),
            "a CI package's -dev suffix is not part of the version"
        );
        assert_eq!(version_in_name("other.zip"), None, "not a package name");
    }

    #[test]
    fn the_c_runtime_is_not_a_windows_dll() {
        for dll in [
            "vcruntime140.dll",
            "msvcp140.dll",
            "api-ms-win-crt-runtime-l1-1-0.dll",
        ] {
            assert!(!is_windows_dll(dll), "{dll} means a dynamic C runtime");
        }
        for dll in [
            "KERNEL32.dll",
            "api-ms-win-core-synch-l1-2-0.dll",
            "user32.dll",
        ] {
            assert!(is_windows_dll(dll), "{dll} is part of Windows");
        }
    }

    #[test]
    fn a_file_that_is_not_pe_is_refused() {
        assert!(
            Pe::parse(vec![0; 16]).is_err(),
            "too short for a DOS header"
        );
        let mut bytes = vec![0_u8; 0x80];
        bytes[0x3c] = 0x40;
        assert!(Pe::parse(bytes).is_err(), "no PE signature");
    }
}

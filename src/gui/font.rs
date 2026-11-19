// ============================================================================
// Module:       gui::font
// Description:  Installs the bundled UI face (Inter), or a typeface named in
//               the config from the machine's own font directories.
//
// Dependencies: egui (FontDefinitions); std::fs for the font directories;
//               assets/fonts/Inter-Regular.ttf (SIL OFL 1.1)
// ============================================================================

//! The UI typeface.
//!
//! The window draws in **Inter** (`assets/fonts/Inter-Regular.ttf`, from
//! the project's own GitHub release, v4.1), compiled in with
//! `include_bytes!`. It is under the SIL Open Font License 1.1, whose text
//! ships beside it as `assets/fonts/OFL.txt`; the OFL allows bundling a
//! font inside software as long as the licence travels with it and the font
//! is not sold on its own. Inter is what Selects uses for its numbers and is
//! drawn for small sizes on screens, which is where an editor's dense 12 px
//! chrome lives.
//!
//! A face *installed on the machine* can still be put in front of it, by
//! name, through the config's `font` key: reading an installed font is what
//! every other application on the machine does.
//!
//! ## The bundled faces stay underneath
//!
//! The named face is *prepended* to the proportional family rather than
//! replacing it. Many faces cover Latin and little else; replacing the
//! family would silently drop Greek, Cyrillic and every glyph a user's own
//! text might contain. Prepending means the named face is asked first and
//! the bundled one answers for everything it lacks.
//!
//! ## Failure is silence, on purpose
//!
//! A family that is not installed, or is spelled the way a font menu spells
//! it rather than the way the file is named, leaves the app in its bundled
//! face. That is a normal outcome, not an error worth showing anybody.

use std::path::{Path, PathBuf};

/// The name the loaded face is registered under.
const KEY: &str = "configured-ui-face";

/// The bundled UI face: Inter Regular, SIL OFL 1.1 (see the module docs).
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// The name the bundled face is registered under.
const INTER_KEY: &str = "inter";

/// Files tried, in order, for symbols the bundled faces lack (✕ ▶ ⏸ ✔ ★ …):
/// Segoe UI Symbol on Windows, Noto / DejaVu on Linux, Apple Symbols on macOS.
const SYMBOL_FALLBACKS: [&str; 4] = [
    "seguisym.ttf",
    "NotoSansSymbols2-Regular.ttf",
    "DejaVuSans.ttf",
    "Apple Symbols.ttf",
];

/// The name the symbol fallback is registered under.
const SYMBOLS: &str = "symbol-fallback";

/// Installs the fonts: Inter in front of egui's own faces, `family` (when
/// given and found) in front of Inter, and a system symbol face behind
/// them all, so a symbol in a label draws instead of showing an empty box.
///
/// Returns the file `family` was loaded from, if it was.
pub fn setup(ctx: &egui::Context, family: Option<&str>) -> Option<PathBuf> {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        INTER_KEY.to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(INTER)),
    );
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, INTER_KEY.to_owned());
    let face = family.and_then(locate);
    if let Some(path) = &face
        && let Ok(bytes) = std::fs::read(path)
    {
        fonts.font_data.insert(
            KEY.to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, KEY.to_owned());
    }
    let symbols = directories()
        .into_iter()
        .flat_map(|dir| SYMBOL_FALLBACKS.iter().map(move |f| dir.join(f)))
        .find_map(|p| std::fs::read(p).ok());
    if let Some(bytes) = symbols {
        fonts.font_data.insert(
            SYMBOLS.to_owned(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push(SYMBOLS.to_owned());
        }
    }
    ctx.set_fonts(fonts);
    face
}

/// Installs `family` as the proportional face, if it can be found (the
/// symbol fallback comes with it; see [`setup`]).
///
/// Returns the file it loaded; `None` leaves the bundled face in front.
pub fn install(ctx: &egui::Context, family: &str) -> Option<PathBuf> {
    setup(ctx, Some(family))
}

/// The directories fonts are installed into, the per-user one first: it is
/// where a font installed without administrator rights lands.
fn directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        directories.push(
            Path::new(&local)
                .join("Microsoft")
                .join("Windows")
                .join("Fonts"),
        );
    }
    if let Some(root) = std::env::var_os("SystemRoot") {
        directories.push(Path::new(&root).join("Fonts"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        directories.push(Path::new(&home).join(".local").join("share").join("fonts"));
        directories.push(Path::new(&home).join("Library").join("Fonts"));
    }
    directories.push(PathBuf::from("/usr/share/fonts/truetype"));
    directories.push(PathBuf::from("/Library/Fonts"));
    directories
}

/// Finds the file holding `family`'s regular weight.
fn locate(family: &str) -> Option<PathBuf> {
    directories().into_iter().find_map(|directory| {
        let files: Vec<PathBuf> = std::fs::read_dir(&directory)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_a_font_file(path))
            .collect();
        pick(&files, family).cloned()
    })
}

/// Whether a path names a font egui can load: TrueType or OpenType.
fn is_a_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "ttf" | "otf" | "ttc"
            )
        })
}

/// Picks the file for `family`'s regular weight out of `files`.
///
/// A family installs as several files that all begin with its name
/// (`Inter Regular.ttf`, `Inter Bold.ttf` …), and a directory listing has
/// no defined order, so the preference is explicit: the exact name, then
/// `<family> Regular`, then any match that is neither bold nor italic.
fn pick<'a>(files: &'a [PathBuf], family: &str) -> Option<&'a PathBuf> {
    let wanted = family.trim().to_ascii_lowercase();
    // `starts_with("")` is true of everything, so a blank name would
    // otherwise install the first font in the directory.
    if wanted.is_empty() {
        return None;
    }
    let stem = |path: &PathBuf| -> Option<String> {
        Some(path.file_stem()?.to_str()?.to_ascii_lowercase())
    };
    let regular = format!("{wanted} regular");
    files
        .iter()
        .find(|path| stem(path).is_some_and(|stem| stem == wanted))
        .or_else(|| {
            files
                .iter()
                .find(|path| stem(path).is_some_and(|stem| stem == regular))
        })
        .or_else(|| {
            files.iter().find(|path| {
                stem(path).is_some_and(|stem| {
                    stem.starts_with(&wanted) && !stem.contains("bold") && !stem.contains("italic")
                })
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn the_bundled_face_is_a_truetype_font() {
        assert_eq!(
            INTER.get(..4),
            Some(&[0x00, 0x01, 0x00, 0x00][..]),
            "assets/fonts/Inter-Regular.ttf must be a TrueType file"
        );
    }

    #[test]
    fn the_regular_weight_wins_over_the_bold_and_the_italic() {
        let candidates = files(&[
            "Inter Bold Italic.ttf",
            "Inter Bold.ttf",
            "Inter Italic.ttf",
            "Inter Regular.ttf",
        ]);
        assert_eq!(
            pick(&candidates, "Inter"),
            Some(&PathBuf::from("Inter Regular.ttf"))
        );
    }

    #[test]
    fn a_family_whose_file_is_named_after_it_is_matched_exactly() {
        let candidates = files(&["Outfit Bold.ttf", "Outfit.ttf"]);
        assert_eq!(
            pick(&candidates, "Outfit"),
            Some(&PathBuf::from("Outfit.ttf"))
        );
    }

    #[test]
    fn the_match_ignores_case_and_finds_nothing_for_a_missing_family() {
        let candidates = files(&["Inter Regular.ttf"]);
        assert!(pick(&candidates, "inter").is_some(), "case must not matter");
        assert!(pick(&candidates, "Helvetica").is_none());
    }

    #[test]
    fn a_blank_name_matches_nothing_rather_than_everything() {
        let candidates = files(&["Arial.ttf"]);
        assert!(pick(&candidates, "").is_none());
        assert!(pick(&candidates, "   ").is_none());
    }

    #[test]
    fn a_family_with_only_a_bold_face_is_not_used() {
        assert!(pick(&files(&["Inter Bold.ttf"]), "Inter").is_none());
    }

    #[test]
    fn only_truetype_and_opentype_files_are_offered() {
        assert!(is_a_font_file(Path::new("Inter.TTF")));
        assert!(is_a_font_file(Path::new("Inter.otf")));
        assert!(!is_a_font_file(Path::new("vgasys.fon")));
        assert!(!is_a_font_file(Path::new("marlett")));
    }
}

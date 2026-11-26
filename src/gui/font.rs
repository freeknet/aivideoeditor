// ============================================================================
// Module:       gui::font
// Description:  Installs the bundled faces (Mona Sans for the UI in three
//               weights, Roboto Mono for timecode, Lucide for icons, Inter
//               underneath), or a typeface named in the config.
//
// Dependencies: egui (FontDefinitions); egui_material3 (its weight family
//               names); std::fs for the font directories;
//               assets/fonts/*.ttf (SIL OFL 1.1 and ISC, see assets/fonts/NOTICE)
// ============================================================================

//! The UI typefaces.
//!
//! The window draws in the faces Selects draws in, compiled in with
//! `include_bytes!` from `assets/fonts/` (licences beside them, listed in
//! `assets/fonts/NOTICE`):
//!
//! | Face | Use | Licence |
//! |---|---|---|
//! | **Mona Sans** Regular, Medium, SemiBold (v2.0.27) | every label; Medium for headers, SemiBold for buttons | SIL OFL 1.1 |
//! | **Roboto Mono** Regular | timecode, tokens, code ([`egui::FontFamily::Monospace`]) | SIL OFL 1.1 |
//! | **Lucide** (`lucide-static` 1.60.0) | icons, by codepoint (`gui::ui::icons`) | ISC (some glyphs MIT, from Feather) |
//! | **Inter** Regular (v4.1) | fallback for anything Mona Sans lacks | SIL OFL 1.1 |
//!
//! The OFL allows bundling a font inside software as long as the licence
//! travels with it and the font is not sold on its own; ISC and MIT ask
//! only that the notice travels with it.
//!
//! ## Weights are families
//!
//! egui has no font weights: a [`egui::FontId`] is a size and a family.
//! So each weight is registered as its own named family, [`MEDIUM`] and
//! [`SEMIBOLD`], each with the regular chain behind it as fallback. Ask for
//! one through [`weight`], never by building `FontFamily::Name` yourself:
//! egui panics on a family it does not know, and fonts installed by
//! [`setup`] only arrive at the start of the *next* frame, so the first
//! frame must fall back to the regular face. [`weight`] does.
//!
//! egui-material3's widgets ask for their weights by its own names
//! (`plain-400`, `plain-500`, `plain-700`); those are registered too, as
//! Mona Sans Regular, Medium and SemiBold, so the library draws in the
//! app's face rather than falling back to egui's default.
//!
//! Lucide sits in every chain behind the text faces, so an icon codepoint
//! in an ordinary label draws as the icon; [`ICONS`] is the family with
//! Lucide first, for drawing an icon on its own.
//!
//! ## A face from the machine
//!
//! A face *installed on the machine* can still be put in front, by name,
//! through the config's `font` key. It is *prepended* to the proportional
//! family rather than replacing it: many faces cover Latin and little else,
//! and the bundled faces answer for everything it lacks. A family that is
//! not installed leaves the app in its bundled face; that is a normal
//! outcome, not an error worth showing anybody.

use egui_material3::tokens::typography::{WEIGHT_BOLD, WEIGHT_MEDIUM, WEIGHT_REGULAR};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The name the configured face is registered under.
const KEY: &str = "configured-ui-face";

/// Mona Sans Regular: the UI face (SIL OFL 1.1).
const MONA: &[u8] = include_bytes!("../../assets/fonts/MonaSans-Regular.ttf");

/// Mona Sans Medium: panel headers and titles (SIL OFL 1.1).
const MONA_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/MonaSans-Medium.ttf");

/// Mona Sans SemiBold: buttons, as Selects sets them (SIL OFL 1.1).
const MONA_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/MonaSans-SemiBold.ttf");

/// Roboto Mono Regular: timecode and tokens (SIL OFL 1.1).
const ROBOTO_MONO: &[u8] = include_bytes!("../../assets/fonts/RobotoMono-Regular.ttf");

/// The Lucide icon font, `lucide-static` 1.60.0 (ISC).
const LUCIDE: &[u8] = include_bytes!("../../assets/fonts/Lucide.ttf");

/// Inter Regular: the fallback text face (SIL OFL 1.1).
const INTER: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");

/// The family for panel headers and titles: Mona Sans Medium.
pub const MEDIUM: &str = "mona-medium";

/// The family for buttons: Mona Sans SemiBold.
pub const SEMIBOLD: &str = "mona-semibold";

/// The family with Lucide first, for drawing an icon on its own.
pub const ICONS: &str = "lucide";

/// The family for timecode and numbers: Inter first (Selects'
/// `font-tabular`).
pub const TABULAR: &str = "inter-tabular";

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

/// A font id in the named weight `family` ([`MEDIUM`], [`SEMIBOLD`] or
/// [`ICONS`]) at `size`, or the regular proportional face while the
/// bundled faces are not installed yet (the first frame; see the module
/// docs). Never panics on an unknown family.
#[must_use]
pub fn weight(ctx: &egui::Context, family: &str, size: f32) -> egui::FontId {
    let named = egui::FontFamily::Name(family.into());
    let known = ctx.fonts(|fonts| fonts.definitions().families.contains_key(&named));
    if known {
        egui::FontId::new(size, named)
    } else {
        egui::FontId::proportional(size)
    }
}

/// The family egui-material3 asks for at `weight` (its type scale names
/// each weight as a family; see the module docs).
fn material_weight(weight: u16) -> egui::FontFamily {
    egui::FontFamily::Name(egui_material3::tokens::typography::weight_family_name(weight).into())
}

/// Installs the fonts: Mona Sans, Inter and Lucide in front of egui's own
/// faces; `family` (when given and found) in front of those; Roboto Mono
/// for monospace; the weights and the icon family by name; and a system
/// symbol face behind them all, so a symbol in a label draws instead of
/// showing an empty box.
///
/// Returns the file `family` was loaded from, if it was.
pub fn setup(ctx: &egui::Context, family: Option<&str>) -> Option<PathBuf> {
    let mut fonts = egui::FontDefinitions::default();
    let mut add = |key: &str, data: egui::FontData| {
        fonts.font_data.insert(key.to_owned(), Arc::new(data));
    };
    add("mona", egui::FontData::from_static(MONA));
    add(MEDIUM, egui::FontData::from_static(MONA_MEDIUM));
    add(SEMIBOLD, egui::FontData::from_static(MONA_SEMIBOLD));
    add("roboto-mono", egui::FontData::from_static(ROBOTO_MONO));
    add("inter", egui::FontData::from_static(INTER));
    add(
        ICONS,
        egui::FontData::from_static(LUCIDE).tweak(egui::FontTweak {
            // Lucide's glyphs sit on the em square, a little high of the
            // text faces' x-height centre: nudged so an icon centres on a
            // label beside it.
            y_offset_factor: 0.06,
            ..Default::default()
        }),
    );
    let face = family.and_then(locate);
    if let Some(path) = &face
        && let Ok(bytes) = std::fs::read(path)
    {
        add(KEY, egui::FontData::from_owned(bytes));
    }
    let configured: Vec<String> = face.iter().map(|_| KEY.to_owned()).collect();

    // egui's own faces stay at the back of each chain: they cover Greek,
    // Cyrillic and emoji the bundled faces do not.
    let egui_proportional = fonts
        .families
        .get(&egui::FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let egui_monospace = fonts
        .families
        .get(&egui::FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    let chain = |front: &[&str]| -> Vec<String> {
        configured
            .iter()
            .cloned()
            .chain(front.iter().map(|key| (*key).to_owned()))
            .chain(["inter".to_owned(), ICONS.to_owned()])
            .chain(egui_proportional.iter().cloned())
            .collect()
    };
    let families = [
        (egui::FontFamily::Proportional, chain(&["mona"])),
        (egui::FontFamily::Name(MEDIUM.into()), chain(&[MEDIUM])),
        (egui::FontFamily::Name(SEMIBOLD.into()), chain(&[SEMIBOLD])),
        // egui-material3's type scale asks for its weights by these names
        // (`plain-400`, `plain-500`, `plain-700`); pointing them at Mona
        // Sans keeps the library's widgets in the app's face.
        (material_weight(WEIGHT_REGULAR), chain(&["mona"])),
        (material_weight(WEIGHT_MEDIUM), chain(&[MEDIUM])),
        (material_weight(WEIGHT_BOLD), chain(&[SEMIBOLD])),
        (
            egui::FontFamily::Name(TABULAR.into()),
            chain(&["inter", "mona"]),
        ),
        (
            egui::FontFamily::Name(ICONS.into()),
            std::iter::once(ICONS.to_owned())
                .chain(chain(&["mona"]))
                .collect(),
        ),
        (
            egui::FontFamily::Monospace,
            std::iter::once("roboto-mono".to_owned())
                .chain([ICONS.to_owned()])
                .chain(egui_monospace)
                .collect(),
        ),
    ];
    for (name, list) in families {
        // Each face once, at its first (highest) place in the chain.
        let mut seen = std::collections::HashSet::new();
        let list: Vec<String> = list
            .into_iter()
            .filter(|key| seen.insert(key.clone()))
            .collect();
        fonts.families.insert(name, list);
    }

    // Every fallback found, in order: one file rarely has every symbol.
    let mut seen = std::collections::HashSet::new();
    let symbols: Vec<Vec<u8>> = directories()
        .into_iter()
        .flat_map(|dir| SYMBOL_FALLBACKS.iter().map(move |f| dir.join(f)))
        .filter_map(|p| {
            let bytes = std::fs::read(&p).ok()?;
            // The same file in two folders (the per-user one and the
            // system one) is loaded once.
            seen.insert(p.file_name()?.to_owned()).then_some(bytes)
        })
        .collect();
    for (i, bytes) in symbols.into_iter().enumerate() {
        let key = format!("{SYMBOLS}-{i}");
        fonts
            .font_data
            .insert(key.clone(), Arc::new(egui::FontData::from_owned(bytes)));
        for list in fonts.families.values_mut() {
            list.push(key.clone());
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
    // Linux keeps each family in its own folder under truetype/ or opentype/.
    for dir in [
        "/usr/share/fonts/truetype",
        "/usr/share/fonts/truetype/dejavu",
        "/usr/share/fonts/truetype/noto",
        "/usr/share/fonts/opentype/noto",
        "/usr/share/fonts/TTF",
        "/usr/share/fonts/noto",
        "/Library/Fonts",
        "/System/Library/Fonts",
    ] {
        directories.push(PathBuf::from(dir));
    }
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

    /// Runs one frame of `ctx`, discarding its output (textures included,
    /// which epaint insists are handled).
    fn run_frame(ctx: &egui::Context, run: impl FnMut(&mut egui::Ui)) {
        let mut output = ctx.run_ui(egui::RawInput::default(), run);
        output.textures_delta.clear();
    }

    fn files(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn every_bundled_face_is_a_truetype_font() {
        for (name, bytes) in [
            ("Inter-Regular", INTER),
            ("MonaSans-Regular", MONA),
            ("MonaSans-Medium", MONA_MEDIUM),
            ("MonaSans-SemiBold", MONA_SEMIBOLD),
            ("RobotoMono-Regular", ROBOTO_MONO),
            ("Lucide", LUCIDE),
        ] {
            assert_eq!(
                bytes.get(..4),
                Some(&[0x00, 0x01, 0x00, 0x00][..]),
                "assets/fonts/{name}.ttf must be a TrueType file"
            );
        }
    }

    #[test]
    fn a_weight_falls_back_to_regular_until_the_faces_are_installed() {
        let ctx = egui::Context::default();
        let mut before = None;
        run_frame(&ctx, |ui| {
            before = Some(weight(ui.ctx(), MEDIUM, 12.0));
            let _ = setup(ui.ctx(), None);
        });
        assert_eq!(
            before,
            Some(egui::FontId::proportional(12.0)),
            "the first frame has no named families yet"
        );
        let mut after = None;
        run_frame(&ctx, |ui| {
            after = Some(weight(ui.ctx(), MEDIUM, 12.0));
        });
        assert_eq!(
            after,
            Some(egui::FontId::new(
                12.0,
                egui::FontFamily::Name(MEDIUM.into())
            )),
            "the next frame has them"
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

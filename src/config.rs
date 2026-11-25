// ============================================================================
// Module:       config
// Description:  Persisted preferences, loaded once at startup and saved on a
//               normal quit, parsed forgivingly field by field.
//
// Dependencies: serde + toml (the on-disk format), dirs (the location)
// ============================================================================

//! The preferences that survive a restart.
//!
//! All of it is convenience state: a missing, partial or unreadable file
//! means "use the defaults", never a hard failure.
//!
//! ## Parsed one field at a time
//!
//! [`parse`] pulls each field out of a `toml::Table` individually rather
//! than deserializing the whole struct at once. Whole-struct
//! deserialization fails on the first mistyped value, and the caller can
//! only fall back to the defaults — so one bad field would silently throw
//! away every other preference. Per field, a bad value costs that one
//! preference and the next save writes a valid file again.
//!
//! ## Written atomically
//!
//! [`save`] writes a temporary file beside the target, flushes it, and
//! renames it over the old one, so a crash mid-write leaves the previous
//! config intact rather than a half-written one that parses to nothing.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Everything the app remembers between runs.
///
/// Every field is `Option`, and `None` means "the app decides". That lets a
/// default change in a later release and reach users who never touched the
/// setting, while respecting the choice of anyone who did.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Config {
    /// A theme `id`. An id that no longer exists falls back to the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The UI typeface, by family name, looked up among installed fonts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font: Option<String>,
    /// Which page was open, by its stable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// Whether the sidebar shows only icons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_collapsed: Option<bool>,
    /// Whether animations are switched off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<bool>,
    /// The window's inner size in logical points, as `[width, height]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_size: Option<[f32; 2]>,
    /// The night that was open, reopened on start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_project: Option<String>,
    /// Recently opened nights, newest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_projects: Option<Vec<String>>,
    /// The workspace view mode, by its stable name (`default`, `vertical`,
    /// `comment`, `preview`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_mode: Option<String>,
    /// The keyboard preset, by its stable name (`ave`, `premiere`,
    /// `final-cut`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keymap: Option<String>,
    /// The default view's panel sizes, as fractions of their parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel_sizes: Option<PanelSizes>,
    /// The panels folded to rails in the default view, by stable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folded_panels: Option<Vec<String>>,
    /// The top bar's recent searches, newest first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_searches: Option<Vec<String>>,
    /// How many clips analysis reads at once (each runs its own ffmpeg).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analysis_jobs: Option<usize>,
    /// The folder ffmpeg and ffprobe are in, when not on `PATH`
    /// (`AVE_FFMPEG_DIR` wins when set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ffmpeg_dir: Option<String>,
    /// Captions: the folder holding llama.cpp's llama-server (`AVE_LLAMA_DIR`
    /// wins when set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_llama_dir: Option<String>,
    /// Captions: the vision model's .gguf file (`AVE_VLM_MODEL` wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_model: Option<String>,
    /// Captions: the model's projector .gguf file (`AVE_VLM_MMPROJ` wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption_mmproj: Option<String>,
}

/// The workspace's splits, each a fraction of the region it divides.
///
/// Fractions rather than pixels, so a layout chosen on one monitor keeps its
/// proportions on another and when the window is resized.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct PanelSizes {
    /// The chat's share of the window width.
    pub chat: f32,
    /// The browser's share of the editor width (right of the chat).
    pub browser: f32,
    /// The inspector's share of the editor width.
    pub inspector: f32,
    /// The timeline's share of the editor height.
    pub timeline: f32,
}

impl PanelSizes {
    /// Whether every fraction is a usable share: finite and inside (0, 1).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        [self.chat, self.browser, self.inspector, self.timeline]
            .iter()
            .all(|f| f.is_finite() && *f > 0.0 && *f < 1.0)
    }
}

/// Where the config file lives: `<config dir>/aivideoeditor/config.toml`.
#[must_use]
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(crate::APP_ID).join("config.toml"))
}

/// Loads the config, or the defaults if there is nothing readable.
#[must_use]
pub fn load() -> Config {
    config_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// Parses a config, forgiving per field. See the module docs.
#[must_use]
pub fn parse(text: &str) -> Config {
    let Ok(table) = text.parse::<toml::Table>() else {
        return Config::default();
    };
    /// Reads one field, `None` for anything that does not deserialize.
    fn field<T: serde::de::DeserializeOwned>(table: &toml::Table, key: &str) -> Option<T> {
        table
            .get(key)
            .cloned()
            .and_then(|value| value.try_into().ok())
    }
    Config {
        theme: field(&table, "theme"),
        font: field(&table, "font"),
        page: field(&table, "page"),
        sidebar_collapsed: field(&table, "sidebar_collapsed"),
        reduce_motion: field(&table, "reduce_motion"),
        window_size: field::<[f32; 2]>(&table, "window_size")
            .filter(|[width, height]| width.is_finite() && height.is_finite()),
        last_project: field(&table, "last_project"),
        recent_projects: field(&table, "recent_projects"),
        view_mode: field(&table, "view_mode"),
        keymap: field(&table, "keymap"),
        panel_sizes: field::<PanelSizes>(&table, "panel_sizes").filter(PanelSizes::is_valid),
        folded_panels: field(&table, "folded_panels"),
        analysis_jobs: field::<usize>(&table, "analysis_jobs").filter(|n| (1..=32).contains(n)),
        ffmpeg_dir: field(&table, "ffmpeg_dir"),
        caption_llama_dir: field(&table, "caption_llama_dir"),
        caption_model: field(&table, "caption_model"),
        caption_mmproj: field(&table, "caption_mmproj"),
        recent_searches: field(&table, "recent_searches"),
    }
}

/// Writes the config to [`config_path`], atomically. See the module docs.
///
/// # Errors
///
/// Returned rather than swallowed, so the caller can say the preferences
/// were not saved: no config directory, or an I/O failure.
pub fn save(config: &Config) -> std::io::Result<()> {
    let path = config_path()
        .ok_or_else(|| std::io::Error::other("no configuration directory could be found"))?;
    write(config, &path)
}

/// Writes `config` to `path` through a temporary file and a rename.
fn write(config: &Config, path: &std::path::Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(config).map_err(std::io::Error::other)?;
    let temporary = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    // `rename` replaces an existing file on every platform std supports.
    std::fs::rename(&temporary, path).inspect_err(|_| {
        // Do not leave the temporary behind to be mistaken for a config.
        let _ = std::fs::remove_file(&temporary);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_or_garbled_config_yields_the_defaults() {
        assert_eq!(parse(""), Config::default());
        assert_eq!(parse("}{ not toml <<<"), Config::default());
    }

    #[test]
    fn one_bad_field_costs_only_that_field() {
        let config = parse(
            r#"
            theme = "nebula"
            sidebar_collapsed = "yes please"
            window_size = [1280.0, 800.0]
            "#,
        );
        assert_eq!(config.theme.as_deref(), Some("nebula"), "before survives");
        assert_eq!(config.window_size, Some([1280.0, 800.0]), "after survives");
        assert_eq!(config.sidebar_collapsed, None, "the bad field is dropped");
    }

    #[test]
    fn a_config_round_trips() -> anyhow::Result<()> {
        let original = Config {
            theme: Some("midnight".to_string()),
            font: Some("Inter".to_string()),
            page: Some("review".to_string()),
            sidebar_collapsed: Some(true),
            reduce_motion: Some(false),
            window_size: Some([1440.0, 900.0]),
            last_project: Some("C:/nights/x/_ave".to_string()),
            recent_projects: Some(vec!["C:/nights/x/_ave".to_string()]),
            view_mode: Some("vertical".to_string()),
            keymap: Some("premiere".to_string()),
            panel_sizes: Some(PanelSizes {
                chat: 0.25,
                browser: 0.2,
                inspector: 0.2,
                timeline: 0.4,
            }),
            folded_panels: Some(vec!["browser".to_string()]),
            analysis_jobs: Some(6),
            ffmpeg_dir: Some("C:/ffmpeg/bin".to_string()),
            caption_llama_dir: Some("C:/llama".to_string()),
            caption_model: Some("C:/models/vl.gguf".to_string()),
            caption_mmproj: Some("C:/models/mmproj.gguf".to_string()),
            recent_searches: Some(vec!["fire>0.3".to_string()]),
        };
        let text = toml::to_string_pretty(&original)?;
        assert_eq!(parse(&text), original, "what was written must read back");
        Ok(())
    }

    #[test]
    fn panel_sizes_outside_zero_to_one_are_dropped() {
        let config = parse(
            r#"
            theme = "nebula"
            [panel_sizes]
            chat = 1.5
            browser = 0.2
            inspector = 0.2
            timeline = 0.4
            "#,
        );
        assert_eq!(config.panel_sizes, None, "a share above 1 is not a layout");
        assert_eq!(config.theme.as_deref(), Some("nebula"), "the rest survives");
    }

    #[test]
    fn an_untouched_config_writes_nothing() -> anyhow::Result<()> {
        let text = toml::to_string_pretty(&Config::default())?;
        assert!(text.trim().is_empty(), "got {text:?}");
        Ok(())
    }

    #[test]
    fn a_save_replaces_the_previous_file_and_leaves_no_temporary() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!("aivideoeditor-config-{}", std::process::id()));
        let path = dir.join("config.toml");
        for theme in ["first", "second"] {
            let config = Config {
                theme: Some(theme.to_string()),
                ..Config::default()
            };
            write(&config, &path)?;
        }
        let read = parse(&std::fs::read_to_string(&path)?);
        let leftovers = std::fs::read_dir(&dir)?.count();
        std::fs::remove_dir_all(&dir)?;
        assert_eq!(read.theme.as_deref(), Some("second"), "the save replaced");
        assert_eq!(leftovers, 1, "only the config itself is left behind");
        Ok(())
    }
}

// ============================================================================
// Module:       recipe
// Description:  Weekly recipes: the deliverables a night produces (lengths,
//               aspects, end cards, naming) and how each maps to a plan and
//               a render.
//
// Dependencies: serde, toml, dirs; crate::{plan, project, render}
// ============================================================================

//! Weekly recipes.
//!
//! The same job comes round every week: a 15, 30 and 60 second reel, a
//! story, a 4:5 feed cut, a 16:9 cut and a recap, each with its end card and
//! a fixed file name. A [`Recipe`] writes that down once, in
//! `<work>/recipe.toml`, so a new night only needs its footage and its track.
//!
//! ## One recipe per night, carried forward
//!
//! Saving a night's recipe also saves it as the user-level default
//! (`<config dir>/aivideoeditor/recipe.toml`), and [`new_night`] copies the
//! previous night's recipe (or that default) into the new work folder: the
//! "new night from last week" of `docs/research/02-ui-and-workflow.md` §6.
//!
//! ## Parsed forgivingly
//!
//! Like `config`, [`parse`] reads each field on its own and each deliverable
//! on its own: one mistyped deliverable costs that row, with a warning, not
//! the whole recipe.
//!
//! ## File names
//!
//! A deliverable's file name comes from a template (default `{id}.mp4`)
//! with the tokens `{night}`, `{id}`, `{aspect}`, `{version}` and `{date}`
//! (see [`Recipe::filename`]). The house rule is that a file name says what
//! the file is and nothing else (`reel-30.mp4`), so the default carries no
//! date and no platform; the tokens are there for recipes that want them.
//! A second version of a plan that would land on the same name gets
//! `-<version>` before the extension, so one render never silently
//! overwrites another.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::plan::Aspect;

/// The recipe's file name inside a work folder.
pub(crate) const FILE: &str = "recipe.toml";

/// The version a plan named exactly after its deliverable carries.
pub(crate) const MAIN_VERSION: &str = "v1";

/// Which encoder renders a deliverable. Mirrors [`crate::render::Encoder`],
/// which is a command-line enum rather than a file format.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum EncoderChoice {
    /// libx264 on the CPU.
    #[default]
    X264,
    /// AMD AMF on the GPU.
    Amf,
    /// NVIDIA NVENC on the GPU.
    Nvenc,
}

impl EncoderChoice {
    /// Every choice, in the order a picker shows them.
    pub(crate) const ALL: [Self; 3] = [Self::X264, Self::Nvenc, Self::Amf];

    /// The render encoder this choice selects.
    pub(crate) fn encoder(self) -> crate::render::Encoder {
        match self {
            Self::X264 => crate::render::Encoder::X264,
            Self::Amf => crate::render::Encoder::Amf,
            Self::Nvenc => crate::render::Encoder::Nvenc,
        }
    }

    /// The name a narrow picker shows.
    pub(crate) fn short_label(self) -> &'static str {
        match self {
            Self::X264 => "x264",
            Self::Amf => "AMF",
            Self::Nvenc => "NVENC",
        }
    }

    /// The name a picker shows.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::X264 => "x264 (CPU)",
            Self::Amf => "AMF (AMD GPU)",
            Self::Nvenc => "NVENC (NVIDIA GPU)",
        }
    }
}

/// The key an aspect goes by in file names and `ave.toml`'s end cards.
pub(crate) fn aspect_key(aspect: Aspect) -> &'static str {
    match aspect {
        Aspect::Vertical => "9x16",
        Aspect::Feed => "4x5",
        Aspect::Wide => "16x9",
    }
}

/// Every aspect, in the order a picker shows them.
pub(crate) const ASPECTS: [Aspect; 3] = [Aspect::Vertical, Aspect::Feed, Aspect::Wide];

/// One output of the night.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Deliverable {
    /// Short name, which is also the plan's name: `reel-30`.
    pub(crate) id: String,
    /// Frame shape.
    pub(crate) aspect: Aspect,
    /// Target length in seconds.
    pub(crate) seconds: f32,
    /// Which of `ave.toml`'s end cards it closes on; `None` is the one for
    /// its aspect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) end_card: Option<String>,
    /// Bars the end card holds.
    #[serde(default = "default_card_bars")]
    pub(crate) card_bars: usize,
    /// File name template; `None` uses the recipe's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) filename: Option<String>,
    /// Encoder; `None` uses the recipe's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) encoder: Option<EncoderChoice>,
}

/// The default bars an end card holds.
fn default_card_bars() -> usize {
    2
}

impl Deliverable {
    /// A deliverable with the recipe-wide defaults for everything optional.
    pub(crate) fn new(id: &str, aspect: Aspect, seconds: f32) -> Self {
        Self {
            id: id.to_owned(),
            aspect,
            seconds,
            end_card: None,
            card_bars: default_card_bars(),
            filename: None,
            encoder: None,
        }
    }

    /// The `ave.toml` end card key it uses.
    pub(crate) fn end_card_key(&self) -> &str {
        self.end_card
            .as_deref()
            .filter(|k| !k.trim().is_empty())
            .unwrap_or_else(|| aspect_key(self.aspect))
    }
}

/// The night's deliverables and how they are named and rendered.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Recipe {
    /// File name template for every deliverable without its own.
    pub(crate) filename: String,
    /// Output folder; `None` is `<night>/videos`. Relative paths are taken
    /// from the night's folder; `{night}` and `{date}` expand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) output_dir: Option<String>,
    /// Encoder for every deliverable without its own.
    pub(crate) encoder: EncoderChoice,
    /// Whether a render also writes an OpenTimelineIO timeline.
    pub(crate) otio: bool,
    /// The outputs, in the order they are listed and rendered.
    #[serde(rename = "deliverable", default)]
    pub(crate) deliverables: Vec<Deliverable>,
    /// The rules every draft keeps (repeats, people, flags, the drop).
    #[serde(default, skip_serializing_if = "is_default_rules")]
    pub(crate) rules: crate::plan::Rules,
    /// The template this recipe started from (its name; informational).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) template: String,
    /// What the footage is, in a few words, for the captioning model and the
    /// chat: `video footage`, `a night of fire spinning, LED props and DJs`.
    #[serde(default = "default_about")]
    pub(crate) about: String,
    /// Props people in the footage may hold, for captions (empty: whatever
    /// they are holding).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) props: Vec<String>,
    /// The four measurements the shot browser shows as bars, by name
    /// (`fire`, `led`, `light`, `move`, `q`, `focus`, `exp`, `luma`).
    #[serde(default = "default_metrics")]
    pub(crate) metrics: Vec<String>,
}

/// What footage is, when a recipe does not say.
pub(crate) const DEFAULT_ABOUT: &str = "video footage";

fn default_about() -> String {
    DEFAULT_ABOUT.to_owned()
}

/// The browser's bars when a recipe does not say: general measures.
pub(crate) const DEFAULT_METRICS: [&str; 4] = ["q", "focus", "exp", "move"];

fn default_metrics() -> Vec<String> {
    DEFAULT_METRICS.iter().map(|m| (*m).to_owned()).collect()
}

/// Whether `rules` are the built-in ones (left out of the file).
fn is_default_rules(rules: &crate::plan::Rules) -> bool {
    *rules == crate::plan::Rules::default()
}

/// The default file name template.
pub(crate) const DEFAULT_FILENAME: &str = "{id}.mp4";

impl Default for Recipe {
    /// The General template: a 30-second wide cut and a 30-second vertical
    /// one, rules that favour no kind of footage.
    fn default() -> Self {
        Self {
            filename: DEFAULT_FILENAME.to_owned(),
            output_dir: None,
            encoder: EncoderChoice::X264,
            otio: true,
            deliverables: vec![
                Deliverable::new("wide-30", Aspect::Wide, 30.0),
                Deliverable::new("vertical-30", Aspect::Vertical, 30.0),
            ],
            rules: crate::plan::Rules {
                drop_subject: String::new(),
                reject_moments: Vec::new(),
                ..crate::plan::Rules::default()
            },
            template: "General".to_owned(),
            about: default_about(),
            props: Vec::new(),
            metrics: default_metrics(),
        }
    }
}

impl Recipe {
    /// The Flow Night template: the weekly promo set (three reels, a story,
    /// a feed cut, a wide cut and a recap), fire on the drop, and captions
    /// that know the props.
    pub(crate) fn flow_night() -> Self {
        Self {
            deliverables: vec![
                Deliverable::new("reel-15", Aspect::Vertical, 15.0),
                Deliverable::new("reel-30", Aspect::Vertical, 30.0),
                Deliverable::new("reel-60", Aspect::Vertical, 60.0),
                Deliverable::new("story-15", Aspect::Vertical, 15.0),
                Deliverable::new("feed-30", Aspect::Feed, 30.0),
                Deliverable::new("wide-60", Aspect::Wide, 60.0),
                Deliverable::new("recap", Aspect::Wide, 180.0),
            ],
            rules: crate::plan::Rules::default(),
            template: "Flow Night".to_owned(),
            about: "a night of fire spinning, LED props, aerial silks and DJs".to_owned(),
            props: [
                "fire fans",
                "fire staff",
                "fire poi",
                "fire hoop",
                "LED hoop",
                "LED poi",
                "silks",
                "DJ decks",
            ]
            .iter()
            .map(|p| (*p).to_owned())
            .collect(),
            metrics: ["fire", "led", "move", "focus"]
                .iter()
                .map(|m| (*m).to_owned())
                .collect(),
            ..Self::default()
        }
    }
}

/// Where a loaded recipe came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// The night's own `recipe.toml`.
    Night,
    /// The user-level default (last saved recipe).
    UserDefault,
    /// Nothing on disk: the built-in week.
    BuiltIn,
}

/// What the tokens in a template expand to, apart from the deliverable's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Names {
    /// The night's name (its folder).
    pub(crate) night: String,
    /// The night's date, `YYYY-MM-DD`.
    pub(crate) date: String,
}

impl Names {
    /// The names for the night whose work folder is `work`: the night is the
    /// folder above `_ave`; the date is the one the name starts with, or
    /// today's.
    pub(crate) fn for_work(work: &Path) -> Self {
        let night = night_dir(work)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let date = night_date(&night).unwrap_or_else(today);
        Self { night, date }
    }
}

/// The night's own folder: the parent of an `_ave` work folder, else the
/// work folder itself.
pub(crate) fn night_dir(work: &Path) -> PathBuf {
    if work.file_name().is_some_and(|n| n == "_ave")
        && let Some(parent) = work.parent()
    {
        return parent.to_path_buf();
    }
    work.to_path_buf()
}

/// The `YYYY-MM-DD` a night's name starts with, if it does.
pub(crate) fn night_date(name: &str) -> Option<String> {
    let head: String = name.chars().take(10).collect();
    let bytes = head.as_bytes();
    let shaped = bytes.len() == 10
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit());
    shaped.then_some(head)
}

/// Today's date (UTC), `YYYY-MM-DD`.
pub(crate) fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (y, m, d) = civil_from_days(i64::try_from(secs / 86_400).unwrap_or(0));
    format!("{y:04}-{m:02}-{d:02}")
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

/// Expands `{night}`, `{id}`, `{aspect}`, `{version}` and `{date}` in
/// `template`; anything else in braces is left as written.
pub(crate) fn expand(
    template: &str,
    names: &Names,
    id: &str,
    aspect: Aspect,
    version: &str,
) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, from) = rest.split_at(open);
        out.push_str(before);
        let Some(close) = from.find('}') else {
            out.push_str(from);
            return out;
        };
        let token = from.get(1..close).unwrap_or_default();
        match token {
            "night" => out.push_str(&names.night),
            "id" => out.push_str(id),
            "aspect" => out.push_str(aspect_key(aspect)),
            "version" => out.push_str(version),
            "date" => out.push_str(&names.date),
            _ => out.push_str(from.get(..=close).unwrap_or_default()),
        }
        rest = from.get(close + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// A file name safe on Windows: no path separators, no reserved characters,
/// no trailing dots or spaces, and an extension (`.mp4` when it has none).
pub(crate) fn sanitize(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']).to_owned();
    let named = if trimmed.trim_matches(['-', '.']).is_empty() {
        fallback.to_owned()
    } else {
        trimmed
    };
    if Path::new(&named).extension().is_some() {
        named
    } else {
        format!("{named}.mp4")
    }
}

/// Inserts `-<version>` before the extension of `name`.
fn with_version(name: &str, version: &str) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => {
            let (stem, ext) = name.split_at(dot);
            format!("{stem}-{version}{ext}")
        }
        Some(_) | None => format!("{name}-{version}"),
    }
}

/// Which deliverable a plan named `stem` is a version of, and the version:
/// `reel-30` is `reel-30`'s main version, `reel-30-v2` / `reel-30_alt` /
/// `reel-30.b` are other versions. The longest matching id wins, so with
/// both `reel` and `reel-30` in the recipe, `reel-30` is not a `reel`.
pub(crate) fn assign(ids: &[&str], stem: &str) -> Option<(usize, String)> {
    let mut best: Option<(usize, String, usize)> = None;
    for (i, id) in ids.iter().enumerate() {
        let version = if stem == *id {
            Some(MAIN_VERSION.to_owned())
        } else {
            stem.strip_prefix(id)
                .and_then(|rest| rest.strip_prefix(['-', '_', '.']))
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        if let Some(version) = version
            && best.as_ref().is_none_or(|(_, _, len)| id.len() > *len)
        {
            best = Some((i, version, id.len()));
        }
    }
    best.map(|(i, v, _)| (i, v))
}

/// A plan file matched to one of the recipe's deliverables.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Matched {
    /// Index into [`Recipe::deliverables`].
    pub(crate) deliverable: usize,
    /// Its version ([`MAIN_VERSION`] for the plan named after the deliverable).
    pub(crate) version: String,
    /// The plan file.
    pub(crate) path: PathBuf,
}

impl Recipe {
    /// The plan files that are versions of a deliverable, by deliverable,
    /// main version first. Plans named after nothing in the recipe are left
    /// out.
    pub(crate) fn match_plans(&self, plans: &[PathBuf]) -> Vec<Matched> {
        let ids: Vec<&str> = self.deliverables.iter().map(|d| d.id.as_str()).collect();
        let mut out: Vec<Matched> = plans
            .iter()
            .filter_map(|path| {
                let stem = path.file_stem()?.to_string_lossy();
                let (deliverable, version) = assign(&ids, &stem)?;
                Some(Matched {
                    deliverable,
                    version,
                    path: path.clone(),
                })
            })
            .collect();
        out.sort_by(|a, b| {
            (a.deliverable, a.version != MAIN_VERSION, &a.version).cmp(&(
                b.deliverable,
                b.version != MAIN_VERSION,
                &b.version,
            ))
        });
        out
    }

    /// The `ave plan` step that drafts deliverable `d` for the night at
    /// `work`, closing on the end card `settings` has for it.
    pub(crate) fn plan_cmd(
        d: &Deliverable,
        work: &Path,
        settings: &crate::project::Settings,
    ) -> crate::commands::Cmd {
        crate::commands::Cmd::Plan {
            name: d.id.clone(),
            work: work.to_path_buf(),
            aspect: aspect_key(d.aspect).to_owned(),
            seconds: d.seconds,
            end_card: settings
                .end_cards
                .get(d.end_card_key())
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            card_bars: d.card_bars,
        }
    }

    /// The deliverable with this id.
    pub(crate) fn get(&self, id: &str) -> Option<&Deliverable> {
        self.deliverables.iter().find(|d| d.id == id)
    }

    /// The encoder `d` renders with.
    pub(crate) fn encoder_for(&self, d: &Deliverable) -> EncoderChoice {
        d.encoder.unwrap_or(self.encoder)
    }

    /// The file name version `version` of `d` is delivered under.
    pub(crate) fn filename(&self, d: &Deliverable, version: &str, names: &Names) -> String {
        let template = d
            .filename
            .as_deref()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or(self.filename.as_str());
        let expanded = expand(template, names, &d.id, d.aspect, version);
        let fallback = format!("{}.mp4", d.id);
        let name = sanitize(&expanded, &fallback);
        if version != MAIN_VERSION && !template.contains("{version}") {
            with_version(&name, version)
        } else {
            name
        }
    }

    /// The folder deliverables are copied to for the night at `work`.
    pub(crate) fn output_dir(&self, work: &Path, names: &Names) -> PathBuf {
        let night = night_dir(work);
        match self.output_dir.as_deref().filter(|d| !d.trim().is_empty()) {
            Some(template) => {
                let expanded = expand(template, names, "", Aspect::Vertical, MAIN_VERSION);
                let path = PathBuf::from(expanded.trim());
                if path.is_absolute() {
                    path
                } else {
                    night.join(path)
                }
            }
            None => night.join("videos"),
        }
    }

    /// Loads the recipe for the night at `work`: its own `recipe.toml`, else
    /// the user-level default, else the built-in week. Warnings say what was
    /// skipped while parsing.
    pub(crate) fn load(work: &Path) -> (Self, Vec<String>, Source) {
        if let Ok(text) = std::fs::read_to_string(work.join(FILE)) {
            let (recipe, warnings) = parse(&text);
            return (recipe, warnings, Source::Night);
        }
        if let Some(text) = default_path().and_then(|p| std::fs::read_to_string(p).ok()) {
            let (recipe, warnings) = parse(&text);
            return (recipe, warnings, Source::UserDefault);
        }
        (Self::default(), Vec::new(), Source::BuiltIn)
    }

    /// [`Recipe::load`] for screens that ask every frame: the file is read
    /// again only when it changed, checked at most once a second.
    pub(crate) fn cached(work: &Path) -> Self {
        type Slot = Option<(
            PathBuf,
            Option<std::time::SystemTime>,
            std::time::Instant,
            Recipe,
        )>;
        static CACHE: std::sync::Mutex<Slot> = std::sync::Mutex::new(None);
        let modified = || {
            std::fs::metadata(work.join(FILE))
                .and_then(|m| m.modified())
                .ok()
        };
        let Ok(mut slot) = CACHE.lock() else {
            return Self::load(work).0;
        };
        if let Some((w, mtime, checked, recipe)) = slot.as_mut()
            && w.as_path() == work
        {
            if checked.elapsed() < std::time::Duration::from_secs(1) {
                return recipe.clone();
            }
            *checked = std::time::Instant::now();
            let now = modified();
            if now == *mtime {
                return recipe.clone();
            }
            *mtime = now;
            *recipe = Self::load(work).0;
            return recipe.clone();
        }
        let recipe = Self::load(work).0;
        *slot = Some((
            work.to_path_buf(),
            modified(),
            std::time::Instant::now(),
            recipe.clone(),
        ));
        recipe
    }

    /// Writes `<work>/recipe.toml`, and the same recipe as the user-level
    /// default so next week starts from it.
    pub(crate) fn save(&self, work: &Path) -> Result<()> {
        self.save_to(&work.join(FILE))?;
        if let Some(path) = default_path() {
            // The night's copy is what matters; a missing config directory
            // only costs next week's head start.
            let _ = self.save_to(&path);
        }
        Ok(())
    }

    /// Writes the recipe to `path` through a temporary file and a rename.
    pub(crate) fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let body = toml::to_string_pretty(self)?;
        let text = format!(
            "# The night's deliverables (ave recipe).\n\
             # filename tokens: {{night}} {{id}} {{aspect}} {{version}} {{date}}\n\
             # output_dir: absolute, or relative to the night's folder (default: videos)\n\n{body}"
        );
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }
}

/// The user-level default recipe: `<config dir>/aivideoeditor/recipe.toml`.
pub(crate) fn default_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(crate::APP_ID).join(FILE))
}

/// Parses a recipe, forgiving per field and per deliverable. A field that
/// is missing or mistyped takes its default; a deliverable that does not
/// parse, or repeats an id, is skipped with a warning.
pub(crate) fn parse(text: &str) -> (Recipe, Vec<String>) {
    let mut warnings = Vec::new();
    let table = match text.parse::<toml::Table>() {
        Ok(table) => table,
        Err(e) => {
            warnings.push(format!(
                "recipe.toml does not parse, using the default: {e}"
            ));
            return (Recipe::default(), warnings);
        }
    };
    let defaults = Recipe::default();
    let field = |key: &str| table.get(key).cloned();
    let filename = field("filename")
        .and_then(|v| v.try_into::<String>().ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(defaults.filename);
    let output_dir = field("output_dir")
        .and_then(|v| v.try_into::<String>().ok())
        .filter(|s| !s.trim().is_empty());
    let encoder = match field("encoder") {
        Some(v) => v.try_into::<EncoderChoice>().unwrap_or_else(|_| {
            warnings.push("recipe: encoder must be \"x264\", \"nvenc\" or \"amf\"".to_owned());
            defaults.encoder
        }),
        None => defaults.encoder,
    };
    let otio = field("otio")
        .and_then(|v| v.try_into::<bool>().ok())
        .unwrap_or(defaults.otio);
    let text_field = |key: &str| field(key).and_then(|v| v.try_into::<String>().ok());
    let list_field = |key: &str| field(key).and_then(|v| v.try_into::<Vec<String>>().ok());
    let template = text_field("template").unwrap_or_default();
    let about = text_field("about")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| defaults.about.clone());
    let props = list_field("props").unwrap_or_default();
    let metrics = list_field("metrics")
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| defaults.metrics.clone());
    let rules = match table.get("rules") {
        None => defaults.rules.clone(),
        Some(v) => v
            .clone()
            .try_into::<crate::plan::Rules>()
            .unwrap_or_else(|e| {
                let message = e.to_string();
                let first = message.lines().next().unwrap_or_default();
                warnings.push(format!(
                    "recipe: rules skipped, using the built-in ones: {first}"
                ));
                defaults.rules.clone()
            }),
    };
    let deliverables = match table.get("deliverable") {
        None => defaults.deliverables,
        Some(toml::Value::Array(items)) => {
            let mut out: Vec<Deliverable> = Vec::new();
            for (n, item) in items.iter().enumerate() {
                match item.clone().try_into::<Deliverable>() {
                    Ok(d) if d.id.trim().is_empty() => {
                        warnings.push(format!("recipe: deliverable {} has no id", n + 1));
                    }
                    Ok(d) if out.iter().any(|o| o.id == d.id) => {
                        warnings.push(format!("recipe: {} is listed twice", d.id));
                    }
                    Ok(d) if !(d.seconds.is_finite() && d.seconds > 0.0) => {
                        warnings.push(format!("recipe: {} needs a length in seconds", d.id));
                    }
                    Ok(d) => out.push(d),
                    Err(e) => {
                        let message = e.to_string();
                        let first = message.lines().next().unwrap_or_default();
                        warnings.push(format!("recipe: deliverable {} skipped: {first}", n + 1));
                    }
                }
            }
            out
        }
        Some(_) => {
            warnings.push("recipe: `deliverable` must be a list of [[deliverable]]".to_owned());
            defaults.deliverables
        }
    };
    let mut recipe = Recipe {
        filename,
        output_dir,
        encoder,
        otio,
        deliverables,
        rules,
        template,
        about,
        props,
        metrics,
    };
    // A recipe saved before templates existed, listing the Flow Night cuts,
    // was a Flow Night recipe: what it leaves out comes from that template,
    // not from General, so an old project keeps cutting the way it did.
    const FLOW_NIGHT_CUTS: [&str; 6] = [
        "reel-15", "reel-30", "reel-60", "story-15", "feed-30", "wide-60",
    ];
    if recipe.template.is_empty()
        && recipe
            .deliverables
            .iter()
            .any(|d| FLOW_NIGHT_CUTS.contains(&d.id.as_str()))
    {
        let flow = Recipe::flow_night();
        let missing = |key: &str| !table.contains_key(key);
        if missing("rules") {
            recipe.rules = flow.rules;
        }
        if missing("about") {
            recipe.about = flow.about;
        }
        if missing("props") {
            recipe.props = flow.props;
        }
        if missing("metrics") {
            recipe.metrics = flow.metrics;
        }
        recipe.template = flow.template;
    }
    (recipe, warnings)
}

/// Audio files a track can be picked from.
const AUDIO: [&str; 7] = ["wav", "mp3", "flac", "m4a", "aac", "ogg", "opus"];

/// The first audio file directly in `dir`, by name.
fn find_track(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .map(|x| x.to_string_lossy().to_ascii_lowercase())
                    .is_some_and(|x| AUDIO.contains(&x.as_str()))
        })
        .collect();
    found.sort();
    found.into_iter().next()
}

/// End card images in the night's folder or its `_work` folder, by aspect:
/// a png/jpg whose name has `endcard` / `end_card` / `end-card` and an
/// aspect key (`endcard_9x16.png`).
fn find_end_cards(night: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for dir in [night.to_path_buf(), night.join("_work")] {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for path in paths {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            let image = name.ends_with(".png") || name.ends_with(".jpg") || name.ends_with(".jpeg");
            let card = ["endcard", "end_card", "end-card"]
                .iter()
                .any(|k| name.contains(k));
            if !(image && card) {
                continue;
            }
            for aspect in ASPECTS {
                let key = aspect_key(aspect);
                if name.contains(key) && !out.iter().any(|(k, _)| k == key) {
                    out.push((key.to_owned(), path.clone()));
                }
            }
        }
    }
    out
}

/// What [`new_night`] set up.
#[derive(Debug, PartialEq)]
pub(crate) struct NewNight {
    /// The new night's work folder.
    pub(crate) work: PathBuf,
    /// One line per thing carried over or found, for the user.
    pub(crate) notes: Vec<String>,
}

/// Reads `ave.toml` from `work`, or the defaults.
fn read_settings(work: &Path) -> crate::project::Settings {
    std::fs::read_to_string(work.join("ave.toml"))
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// Sets up a night for the footage folder `footage`, carrying over what
/// the previous night (work folder `previous`) had: its recipe, end cards,
/// track and page options. A track or end cards found in the new night's
/// own folder win over last week's. Nothing that already exists in the new
/// work folder is replaced, so running it on a started night only fills
/// gaps.
pub(crate) fn new_night(footage: &Path, previous: Option<&Path>) -> Result<NewNight> {
    let work = crate::project::work_dir_for(footage);
    std::fs::create_dir_all(&work).with_context(|| format!("creating {}", work.display()))?;
    let night = night_dir(&work);
    let mut notes = Vec::new();
    let previous = previous.filter(|p| *p != work.as_path());
    let last = previous.map(read_settings).unwrap_or_default();
    let mut settings = read_settings(&work);

    if settings.footage.is_none() && footage != work.as_path() {
        settings.footage = Some(footage.to_path_buf());
    }
    if settings.track.is_none() {
        if let Some(track) = find_track(&night) {
            notes.push(format!(
                "track: {}",
                track.file_name().unwrap_or_default().to_string_lossy()
            ));
            settings.track = Some(track);
        } else if let Some(track) = last.track.clone() {
            notes.push("track: last week's".to_owned());
            settings.track = Some(track);
        }
    }
    let found = find_end_cards(&night);
    for (key, path) in &found {
        if !settings.end_cards.contains_key(key) {
            settings.end_cards.insert(key.clone(), path.clone());
        }
    }
    if !found.is_empty() {
        notes.push(format!(
            "{} end card(s) found in the night's folder",
            found.len()
        ));
    }
    let mut carried = 0;
    for (key, path) in &last.end_cards {
        if !settings.end_cards.contains_key(key) {
            settings.end_cards.insert(key.clone(), path.clone());
            carried += 1;
        }
    }
    if carried > 0 {
        notes.push(format!("{carried} end card(s) from last week"));
    }
    for (page, options) in &last.pages {
        settings
            .pages
            .entry(page.clone())
            .or_insert_with(|| options.clone());
    }
    let text = toml::to_string_pretty(&settings)?;
    std::fs::write(work.join("ave.toml"), text)
        .with_context(|| format!("writing {}", work.join("ave.toml").display()))?;

    if !work.join(FILE).exists() {
        let from_last = previous
            .map(|p| p.join(FILE))
            .and_then(|p| std::fs::read_to_string(p).ok());
        let (recipe, source) = match from_last {
            Some(text) => (parse(&text).0, "last week's recipe"),
            None => match Recipe::load(&work) {
                (recipe, _, Source::UserDefault) => (recipe, "your saved recipe"),
                (recipe, _, Source::Night | Source::BuiltIn) => (recipe, "the default recipe"),
            },
        };
        recipe.save_to(&work.join(FILE))?;
        notes.push(format!(
            "{} deliverables from {source}",
            recipe.deliverables.len()
        ));
    }
    Ok(NewNight { work, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_flow_night_recipe_keeps_cutting_the_way_it_did() -> anyhow::Result<()> {
        let (old, _) = parse(
            "[[deliverable]]
id = \"reel-30\"
aspect = \"9x16\"
seconds = 30.0
",
        );
        assert_eq!(old.template, "Flow Night", "recognised by its cuts");
        assert_eq!(old.rules.drop_subject, "fire", "its rules, not General's");
        assert_eq!(old.metrics, Recipe::flow_night().metrics, "its measures");
        let (other, _) = parse(
            "[[deliverable]]
id = \"recap\"
aspect = \"16x9\"
seconds = 180.0
",
        );
        assert!(other.template.is_empty(), "a recap alone says nothing");
        assert!(other.rules.drop_subject.is_empty(), "so General's rules");
        Ok(())
    }

    #[test]
    fn the_rules_are_the_projects_and_a_partial_table_keeps_the_rest() -> anyhow::Result<()> {
        let (recipe, warnings) = parse(
            "[rules]
person_gap = 3
reject_flags = [\"dark\"]
",
        );
        assert!(warnings.is_empty(), "parses cleanly: {warnings:?}");
        assert_eq!(recipe.rules.person_gap, 3, "set");
        assert_eq!(recipe.rules.reject_flags, vec!["dark".to_owned()], "set");
        assert_eq!(
            recipe.rules.same_clip_gap,
            crate::plan::Rules::default().same_clip_gap,
            "left out, so built in"
        );
        let dir = tempfile::tempdir()?;
        recipe.save_to(&dir.path().join(FILE))?;
        let (back, _) = parse(&std::fs::read_to_string(dir.path().join(FILE))?);
        assert_eq!(back.rules, recipe.rules, "survives a save");
        let (bad, warnings) = parse(
            "[rules]
person_gap = \"lots\"
",
        );
        assert_eq!(
            bad.rules,
            Recipe::default().rules,
            "a bad table costs only the rules"
        );
        assert_eq!(warnings.len(), 1, "and says so: {warnings:?}");
        Ok(())
    }

    fn names() -> Names {
        Names {
            night: "2026-11-11".to_owned(),
            date: "2026-11-11".to_owned(),
        }
    }

    #[test]
    fn templates_expand_every_token_and_keep_unknown_ones() {
        let out = expand(
            "{night}_{id}_{aspect}_{version}_{date}_{other}.mp4",
            &names(),
            "reel-30",
            Aspect::Vertical,
            "v2",
        );
        assert_eq!(
            out, "2026-11-11_reel-30_9x16_v2_2026-11-11_{other}.mp4",
            "every token"
        );
        assert_eq!(
            expand("{id", &names(), "x", Aspect::Feed, "v1"),
            "{id",
            "an unclosed brace stays"
        );
    }

    #[test]
    fn the_default_name_is_just_what_the_file_is() {
        let recipe = Recipe::flow_night();
        let names = names();
        let reel = recipe.get("reel-30");
        let name = reel.map(|d| recipe.filename(d, MAIN_VERSION, &names));
        assert_eq!(name.as_deref(), Some("reel-30.mp4"), "house naming rule");
    }

    #[test]
    fn a_second_version_never_lands_on_the_first_ones_name() -> anyhow::Result<()> {
        let recipe = Recipe::flow_night();
        let names = names();
        let reel = recipe
            .get("reel-30")
            .ok_or_else(|| anyhow::anyhow!("no reel-30 in the default recipe"))?;
        assert_eq!(recipe.filename(reel, "v2", &names), "reel-30-v2.mp4");
        let mut own = reel.clone();
        own.filename = Some("{id}@{version}".to_owned());
        assert_eq!(
            recipe.filename(&own, "v2", &names),
            "reel-30@v2.mp4",
            "a template with {{version}} is left alone, and gets an extension"
        );
        Ok(())
    }

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(sanitize("a/b:c?.mp4", "x.mp4"), "a-b-c-.mp4");
        assert_eq!(sanitize("  ...  ", "x.mp4"), "x.mp4", "empty falls back");
        assert_eq!(sanitize("clip", "x.mp4"), "clip.mp4", "gets an extension");
    }

    #[test]
    fn plans_are_assigned_to_the_longest_matching_deliverable() {
        let ids = ["reel", "reel-30", "recap"];
        assert_eq!(assign(&ids, "reel-30"), Some((1, "v1".to_owned())));
        assert_eq!(assign(&ids, "reel-30-v2"), Some((1, "v2".to_owned())));
        assert_eq!(assign(&ids, "reel"), Some((0, "v1".to_owned())));
        assert_eq!(assign(&ids, "recap_alt"), Some((2, "alt".to_owned())));
        assert_eq!(assign(&ids, "feed-30"), None, "not in the recipe");
        assert_eq!(assign(&ids, "reelx"), None, "needs a separator");
    }

    #[test]
    fn the_recipe_round_trips_through_its_file() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let mut recipe = Recipe {
            encoder: EncoderChoice::Amf,
            output_dir: Some("out/{date}".to_owned()),
            ..Recipe::default()
        };
        if let Some(d) = recipe.deliverables.first_mut() {
            d.filename = Some("{night}-{id}.mp4".to_owned());
            d.end_card = Some("9x16-alt".to_owned());
        }
        recipe.save_to(&dir.path().join(FILE))?;
        let (back, warnings, source) = Recipe::load(dir.path());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(source, Source::Night, "the night's own file");
        assert_eq!(back, recipe, "round trip");
        Ok(())
    }

    #[test]
    fn one_bad_deliverable_costs_only_that_row() {
        let text = r#"
            filename = "{id}.mp4"
            encoder = "quicksync"
            [[deliverable]]
            id = "reel-30"
            aspect = "9x16"
            seconds = 30
            [[deliverable]]
            id = "broken"
            aspect = "3x2"
            seconds = 10
            [[deliverable]]
            id = "reel-30"
            aspect = "4x5"
            seconds = 30
            [[deliverable]]
            id = "recap"
            aspect = "16x9"
            seconds = 180.5
            card_bars = 4
        "#;
        let (recipe, warnings) = parse(text);
        let ids: Vec<&str> = recipe.deliverables.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["reel-30", "recap"], "bad and repeated rows skipped");
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(recipe.encoder, EncoderChoice::X264, "bad encoder: default");
        assert_eq!(recipe.get("recap").map(|d| d.card_bars), Some(4));
        assert_eq!(recipe.get("reel-30").map(|d| d.card_bars), Some(2));
    }

    #[test]
    fn garbage_is_the_default_recipe_with_a_warning() {
        let (recipe, warnings) = parse("this is = = not toml");
        assert_eq!(recipe, Recipe::default(), "defaults");
        assert_eq!(warnings.len(), 1, "says why");
    }

    #[test]
    fn the_output_folder_defaults_to_the_nights_videos() {
        let work = Path::new("C:/nights/2026-11-11/_ave");
        let names = Names::for_work(work);
        assert_eq!(names.night, "2026-11-11");
        assert_eq!(names.date, "2026-11-11", "date from the name");
        let mut recipe = Recipe::default();
        assert_eq!(
            recipe.output_dir(work, &names),
            Path::new("C:/nights/2026-11-11").join("videos")
        );
        recipe.output_dir = Some("deliver/{night}".to_owned());
        assert_eq!(
            recipe.output_dir(work, &names),
            Path::new("C:/nights/2026-11-11").join("deliver/2026-11-11"),
            "relative to the night"
        );
    }

    #[test]
    fn dates_come_from_names_or_the_calendar() {
        assert_eq!(
            night_date("2026-11-04_flow-night").as_deref(),
            Some("2026-11-04")
        );
        assert_eq!(night_date("11-4-2026"), None);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_773), (2026, 11, 16));
        assert_eq!(today().len(), 10, "YYYY-MM-DD");
    }

    #[test]
    fn a_new_night_carries_last_weeks_recipe_and_cards() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let last = dir.path().join("2026-11-04").join("_ave");
        std::fs::create_dir_all(&last)?;
        let mut recipe = Recipe::default();
        recipe.deliverables.truncate(2);
        recipe.save_to(&last.join(FILE))?;
        std::fs::write(
            last.join("ave.toml"),
            "track = \"old.wav\"\n[end_cards]\n\"9x16\" = \"old_9x16.png\"\n\"16x9\" = \"old_16x9.png\"\n",
        )?;
        let night = dir.path().join("2026-11-11");
        let video = night.join("originals").join("video");
        std::fs::create_dir_all(night.join("_work"))?;
        std::fs::create_dir_all(&video)?;
        std::fs::write(night.join("song.mp3"), b"")?;
        std::fs::write(night.join("_work").join("endcard_9x16.png"), b"")?;

        let made = new_night(&video, Some(&last))?;
        assert_eq!(
            made.work,
            night.join("_ave"),
            "work folder beside the footage"
        );
        let settings = read_settings(&made.work);
        assert_eq!(settings.footage.as_deref(), Some(video.as_path()));
        assert_eq!(
            settings.track.as_deref(),
            Some(night.join("song.mp3").as_path()),
            "this week's track wins"
        );
        assert_eq!(
            settings.end_cards.get("9x16").map(PathBuf::as_path),
            Some(night.join("_work").join("endcard_9x16.png").as_path()),
            "this week's end card wins"
        );
        assert_eq!(
            settings.end_cards.get("16x9").map(PathBuf::as_path),
            Some(Path::new("old_16x9.png")),
            "last week's fills the gap"
        );
        let (copied, _, source) = Recipe::load(&made.work);
        assert_eq!(source, Source::Night, "the night has its own recipe");
        assert_eq!(copied.deliverables.len(), 2, "last week's recipe");

        // Running it again changes nothing that exists.
        std::fs::write(made.work.join(FILE), "filename = \"{night}.mp4\"\n")?;
        let again = new_night(&video, Some(&last))?;
        let (kept, _, _) = Recipe::load(&again.work);
        assert_eq!(kept.filename, "{night}.mp4", "existing recipe kept");
        Ok(())
    }
}

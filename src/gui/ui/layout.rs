// ============================================================================
// Module:       gui::ui::layout
// Description:  The workspace's arrangement as data: view modes, which panels
//               are folded to rails, and each split as a fraction.
//
// Dependencies: crate::config (PanelSizes, persistence)
// ============================================================================

//! How the workspace is arranged.
//!
//! Selects lays its editor out as nested splitters sized in percentages,
//! with foldable panels that leave a labelled rail, and offers view modes
//! that rearrange the same panels. This module is that state, kept apart
//! from the drawing so it can be tested and persisted:
//!
//! ```text
//! chat 22% │ browser 21% │ viewer 58% │ inspector 21%   (upper row)
//!          │ timeline 45% of the height, under the three  (lower row)
//! ```
//!
//! Each [`ViewMode`] has its own folds and its own fractions, so folding the
//! browser to review a cut in Comment view does not fold it in Default.
//! Only the Default view's arrangement is written to the config; the other
//! views start from their own defaults each run, which is what makes them
//! predictable to switch to.

use crate::config::{Config, PanelSizes};

/// The width of a folded panel's rail (and the height of the timeline's).
pub const RAIL: f32 = 24.0;

/// The least width the chat may be dragged to before it should be folded.
pub const MIN_CHAT: f32 = 240.0;

/// The least width of the browser.
pub const MIN_BROWSER: f32 = 200.0;

/// The least width of the inspector.
pub const MIN_INSPECTOR: f32 = 220.0;

/// The least height of the timeline.
pub const MIN_TIMELINE: f32 = 140.0;

/// The least width left to the viewer, whatever the sides are dragged to.
pub const MIN_VIEWER: f32 = 240.0;

/// The largest share of its parent a side panel may take.
pub const MAX_SHARE: f32 = 0.45;

/// A workspace arrangement, picked from the top bar or with `Alt+1…4`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ViewMode {
    /// Chat, browser, viewer and inspector over the timeline.
    #[default]
    Default,
    /// The viewer as a tall 9:16 column in the centre, for reels.
    Vertical,
    /// Viewer and timeline large, chat narrow, the browser folded: for
    /// reviewing a cut and leaving timecoded comments.
    Comment,
    /// The viewer alone; every other panel folded to its rail.
    Preview,
}

impl ViewMode {
    /// Every mode, in menu order.
    pub const ALL: [Self; 4] = [Self::Default, Self::Vertical, Self::Comment, Self::Preview];

    /// The name the menu shows.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Vertical => "Vertical",
            Self::Comment => "Comment",
            Self::Preview => "Preview",
        }
    }

    /// One line on what the mode is for.
    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            Self::Default => "Chat, browser, viewer, inspector and timeline",
            Self::Vertical => "A tall 9:16 viewer in the centre, for reels",
            Self::Comment => "Viewer and timeline large, for reviewing a cut",
            Self::Preview => "The viewer alone",
        }
    }

    /// The stable name the config persists. Never rename one.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Vertical => "vertical",
            Self::Comment => "comment",
            Self::Preview => "preview",
        }
    }

    /// The mode with this stable name.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.key() == key)
    }

    /// The mode's position in [`Self::ALL`].
    fn index(self) -> usize {
        match self {
            Self::Default => 0,
            Self::Vertical => 1,
            Self::Comment => 2,
            Self::Preview => 3,
        }
    }

    /// What the mode starts with: which panels are open, and the splits.
    #[must_use]
    pub fn defaults(self) -> (Folds, PanelSizes) {
        let all = Folds::ALL_OPEN;
        match self {
            Self::Default => (all, DEFAULT_SIZES),
            Self::Vertical => (
                all,
                PanelSizes {
                    chat: 0.2,
                    timeline: 0.34,
                    ..DEFAULT_SIZES
                },
            ),
            Self::Comment => (
                Folds {
                    browser: false,
                    ..all
                },
                PanelSizes {
                    chat: 0.16,
                    inspector: 0.18,
                    timeline: 0.5,
                    ..DEFAULT_SIZES
                },
            ),
            Self::Preview => (Folds::ALL_FOLDED, DEFAULT_SIZES),
        }
    }
}

/// Selects' proportions: chat 22 %, browser and inspector 21 % of the
/// editor each, timeline 45 % of the editor's height.
pub const DEFAULT_SIZES: PanelSizes = PanelSizes {
    chat: 0.22,
    browser: 0.21,
    inspector: 0.21,
    timeline: 0.45,
};

/// A panel that folds to a rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Region {
    /// The chat, leftmost.
    Chat,
    /// The browser (Shots, People, Music), upper left.
    Browser,
    /// The inspector, upper right.
    Inspector,
    /// The timeline, along the foot.
    Timeline,
}

impl Region {
    /// Every foldable region.
    pub const ALL: [Self; 4] = [Self::Chat, Self::Browser, Self::Inspector, Self::Timeline];

    /// The rail's label and the toggle's name.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Browser => "Browser",
            Self::Inspector => "Inspector",
            Self::Timeline => "Timeline",
        }
    }

    /// The stable name the config persists. Never rename one.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Browser => "browser",
            Self::Inspector => "inspector",
            Self::Timeline => "timeline",
        }
    }

    /// The region with this stable name.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|region| region.key() == key)
    }

    /// The id of the command that shows or hides it.
    #[must_use]
    pub fn toggle_command(self) -> &'static str {
        match self {
            Self::Chat => "toggle-chat",
            Self::Browser => "toggle-browser",
            Self::Inspector => "toggle-inspector",
            Self::Timeline => "toggle-timeline",
        }
    }
}

/// Which panels are open (`true`) rather than folded to a rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Folds {
    /// The chat.
    pub chat: bool,
    /// The browser.
    pub browser: bool,
    /// The inspector.
    pub inspector: bool,
    /// The timeline.
    pub timeline: bool,
}

impl Folds {
    /// Every panel open.
    pub const ALL_OPEN: Self = Self {
        chat: true,
        browser: true,
        inspector: true,
        timeline: true,
    };

    /// Every panel folded.
    pub const ALL_FOLDED: Self = Self {
        chat: false,
        browser: false,
        inspector: false,
        timeline: false,
    };

    /// Whether `region` is open.
    #[must_use]
    pub fn get(self, region: Region) -> bool {
        match region {
            Region::Chat => self.chat,
            Region::Browser => self.browser,
            Region::Inspector => self.inspector,
            Region::Timeline => self.timeline,
        }
    }

    /// Opens or folds `region`.
    pub fn set(&mut self, region: Region, open: bool) {
        match region {
            Region::Chat => self.chat = open,
            Region::Browser => self.browser = open,
            Region::Inspector => self.inspector = open,
            Region::Timeline => self.timeline = open,
        }
    }
}

/// The whole arrangement: the mode, and each mode's folds and splits.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// The mode on screen.
    pub mode: ViewMode,
    /// Each mode's folds, by [`ViewMode::ALL`] position.
    folds: [Folds; 4],
    /// Each mode's splits, by [`ViewMode::ALL`] position.
    sizes: [PanelSizes; 4],
}

impl Default for Layout {
    fn default() -> Self {
        let folds = ViewMode::ALL.map(|mode| mode.defaults().0);
        let sizes = ViewMode::ALL.map(|mode| mode.defaults().1);
        Self {
            mode: ViewMode::Default,
            folds,
            sizes,
        }
    }
}

impl Layout {
    /// The arrangement the config remembers, defaults for the rest.
    #[must_use]
    pub fn from_config(config: &Config) -> Self {
        let mut layout = Self::default();
        if let Some(mode) = config.view_mode.as_deref().and_then(ViewMode::from_key) {
            layout.mode = mode;
        }
        if let Some(sizes) = config.panel_sizes.filter(PanelSizes::is_valid) {
            layout.sizes[ViewMode::Default.index()] = sizes;
        }
        if let Some(folded) = &config.folded_panels {
            let folds = &mut layout.folds[ViewMode::Default.index()];
            for region in folded.iter().filter_map(|key| Region::from_key(key)) {
                folds.set(region, false);
            }
        }
        layout
    }

    /// Writes what the config remembers: the mode and the Default view.
    pub fn write(&self, config: &mut Config) {
        config.view_mode = Some(self.mode.key().to_owned());
        let index = ViewMode::Default.index();
        let sizes = self.sizes.get(index).copied().unwrap_or(DEFAULT_SIZES);
        config.panel_sizes = Some(sizes).filter(PanelSizes::is_valid);
        let folds = self.folds.get(index).copied().unwrap_or(Folds::ALL_OPEN);
        config.folded_panels = Some(
            Region::ALL
                .into_iter()
                .filter(|region| !folds.get(*region))
                .map(|region| region.key().to_owned())
                .collect(),
        );
    }

    /// The current mode's folds.
    #[must_use]
    pub fn folds(&self) -> Folds {
        self.folds
            .get(self.mode.index())
            .copied()
            .unwrap_or(Folds::ALL_OPEN)
    }

    /// Whether `region` is open in the current mode.
    #[must_use]
    pub fn is_open(&self, region: Region) -> bool {
        self.folds().get(region)
    }

    /// Opens or folds `region` in the current mode.
    pub fn set_open(&mut self, region: Region, open: bool) {
        if let Some(folds) = self.folds.get_mut(self.mode.index()) {
            folds.set(region, open);
        }
    }

    /// Folds `region` if it is open, opens it if folded.
    pub fn toggle(&mut self, region: Region) {
        let open = self.is_open(region);
        self.set_open(region, !open);
    }

    /// The current mode's splits.
    #[must_use]
    pub fn sizes(&self) -> PanelSizes {
        self.sizes
            .get(self.mode.index())
            .copied()
            .unwrap_or(DEFAULT_SIZES)
    }

    /// The current mode's splits, to update after a drag.
    pub fn sizes_mut(&mut self) -> Option<&mut PanelSizes> {
        self.sizes.get_mut(self.mode.index())
    }

    /// Puts the current mode back to its own defaults.
    pub fn reset(&mut self) {
        let (folds, sizes) = self.mode.defaults();
        let index = self.mode.index();
        if let Some(slot) = self.folds.get_mut(index) {
            *slot = folds;
        }
        if let Some(slot) = self.sizes.get_mut(index) {
            *slot = sizes;
        }
    }
}

/// A side panel's width in points: `fraction` of `total`, kept between
/// `min` and [`MAX_SHARE`] of the parent (and never more than the parent).
#[must_use]
pub fn span(fraction: f32, total: f32, min: f32) -> f32 {
    if !total.is_finite() || total <= 0.0 {
        return 0.0;
    }
    let fraction = if fraction.is_finite() {
        fraction
    } else {
        DEFAULT_SIZES.browser
    };
    let max = (total * MAX_SHARE).max(min.min(total));
    (fraction * total).clamp(min.min(max), max)
}

/// The Vertical view's sides: the width each of browser and inspector gets
/// so that the viewer between them is a 9:16 column `height` tall, given
/// the editor's `width`. Never less than their minimums.
#[must_use]
pub fn vertical_sides(width: f32, height: f32) -> f32 {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return MIN_BROWSER;
    }
    let viewer = (height * 9.0 / 16.0).max(MIN_VIEWER);
    ((width - viewer) / 2.0).max(MIN_BROWSER.max(MIN_INSPECTOR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_and_region_keys_round_trip() {
        for mode in ViewMode::ALL {
            assert_eq!(ViewMode::from_key(mode.key()), Some(mode), "{mode:?}");
        }
        for region in Region::ALL {
            assert_eq!(Region::from_key(region.key()), Some(region), "{region:?}");
        }
    }

    #[test]
    fn the_default_view_has_selects_proportions() {
        let sizes = Layout::default().sizes();
        assert_eq!(sizes, DEFAULT_SIZES);
        let upper = sizes.browser + sizes.inspector;
        assert!(
            (upper - 0.42).abs() < 1e-6,
            "browser and inspector leave the viewer 58 %, got {upper}"
        );
    }

    #[test]
    fn preview_folds_everything_and_comment_folds_the_browser() {
        let mut layout = Layout {
            mode: ViewMode::Preview,
            ..Layout::default()
        };
        assert_eq!(layout.folds(), Folds::ALL_FOLDED, "preview");
        layout.mode = ViewMode::Comment;
        assert!(
            !layout.is_open(Region::Browser),
            "comment folds the browser"
        );
        assert!(
            layout.is_open(Region::Timeline),
            "comment keeps the timeline"
        );
    }

    #[test]
    fn folding_in_one_mode_leaves_the_others_alone() {
        let mut layout = Layout::default();
        layout.toggle(Region::Chat);
        assert!(!layout.is_open(Region::Chat), "folded in default");
        layout.mode = ViewMode::Vertical;
        assert!(layout.is_open(Region::Chat), "still open in vertical");
    }

    #[test]
    fn the_default_view_survives_the_config() {
        let mut layout = Layout::default();
        layout.toggle(Region::Inspector);
        if let Some(sizes) = layout.sizes_mut() {
            sizes.chat = 0.3;
        }
        layout.mode = ViewMode::Comment;
        let mut config = Config::default();
        layout.write(&mut config);
        let back = Layout::from_config(&config);
        assert_eq!(back.mode, ViewMode::Comment, "the mode is remembered");
        let default = Layout {
            mode: ViewMode::Default,
            ..back
        };
        assert!(
            !default.is_open(Region::Inspector),
            "the fold is remembered"
        );
        assert!((default.sizes().chat - 0.3).abs() < 1e-6, "the split too");
    }

    #[test]
    fn a_span_stays_inside_its_limits() {
        assert!((span(0.21, 1000.0, 200.0) - 210.0).abs() < 1e-3, "plain");
        assert!((span(0.05, 1000.0, 200.0) - 200.0).abs() < 1e-3, "minimum");
        assert!((span(0.9, 1000.0, 200.0) - 450.0).abs() < 1e-3, "maximum");
        assert!(
            span(0.3, 150.0, 200.0) <= 150.0,
            "never wider than the parent"
        );
        assert_eq!(span(0.3, f32::NAN, 200.0), 0.0, "no parent, no panel");
    }

    #[test]
    fn the_vertical_viewer_is_nine_by_sixteen() {
        let side = vertical_sides(1600.0, 640.0);
        let viewer = 1600.0 - side * 2.0;
        assert!(
            (viewer - 360.0).abs() < 1e-3,
            "a 640-high column is 360 wide"
        );
        assert!(
            vertical_sides(500.0, 900.0) >= MIN_BROWSER,
            "the sides keep their minimum"
        );
    }
}

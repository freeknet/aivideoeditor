// ============================================================================
// Module:       gui::ui::keymap
// Description:  The one command table: every command's id, label, region and
//               keys per preset (ave, Premiere, Final Cut). It drives the
//               shortcuts, the command palette and the shortcut sheet.
//
// Dependencies: egui (Key, Modifiers, KeyboardShortcut); crate::gui::app
// ============================================================================

//! The keyboard, as data.
//!
//! [`COMMANDS`] is the single place a shortcut is stated. The shell reads
//! the global ones from it each frame ([`dispatch`]), the command palette
//! lists it, and the shortcut sheet (`?`) prints it, so the three cannot
//! drift apart: a key changed here changes in all of them.
//!
//! ## Presets
//!
//! Selects asks which editor you come from and picks its shortcut table to
//! match. [`Preset`] does the same: **ave**'s own keys, **Premiere** (which
//! is also what Selects defaults to) and **Final Cut Pro**. A row states
//! the default keys and, where a preset differs, that preset's keys; a
//! preset that states nothing inherits the default.
//!
//! ## Global and region commands
//!
//! A [`Scope::Global`] command has an [`Action`] and is dispatched here,
//! before any region draws, so it wins over a region's own keys. The other
//! scopes (viewer, timeline, browser, chat, export) are read by their regions while
//! they have the keyboard; their rows are here so the sheet and the palette
//! show them, and [`chord`] is the lookup a region uses to adopt a preset.
//!
//! ## Exact matching
//!
//! egui's own `consume_shortcut` ignores an extra Shift or Alt, so a plain
//! `R` would also swallow the viewer's `Alt+R`. Global keys are therefore
//! matched exactly (see [`matches`]); the only leniency is for keys such as
//! `?` that need Shift to be typed at all.

use crate::gui::app::{Action, Page};
use crate::gui::ui::layout::{Region, ViewMode};
use egui::{Key, KeyboardShortcut, Modifiers};

/// Which editor's keyboard habits the shortcuts follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Preset {
    /// ave's own keys.
    #[default]
    Ave,
    /// Adobe Premiere Pro (Selects' default preset).
    Premiere,
    /// Final Cut Pro.
    FinalCut,
}

impl Preset {
    /// Every preset, in the order the picker shows them.
    pub const ALL: [Self; 3] = [Self::Ave, Self::Premiere, Self::FinalCut];

    /// The name shown in Settings.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Ave => "ave",
            Self::Premiere => "Premiere Pro",
            Self::FinalCut => "Final Cut Pro",
        }
    }

    /// The stable name the config persists. Never rename one.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Ave => "ave",
            Self::Premiere => "premiere",
            Self::FinalCut => "final-cut",
        }
    }

    /// The preset with this stable name.
    #[must_use]
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.key() == key)
    }
}

/// Where a command's keys are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Anywhere in the workspace, read by the shell.
    Global,
    /// The browser (Shots, People, Music).
    Browser,
    /// The viewer and its transport.
    Viewer,
    /// The timeline.
    Timeline,
    /// The chat composer.
    Chat,
    /// The export panel.
    Export,
}

impl Scope {
    /// Every scope, in the order the sheet lists them.
    pub const ALL: [Self; 6] = [
        Self::Global,
        Self::Viewer,
        Self::Timeline,
        Self::Browser,
        Self::Chat,
        Self::Export,
    ];

    /// The heading the sheet shows.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "Workspace",
            Self::Browser => "Browser",
            Self::Viewer => "Viewer",
            Self::Timeline => "Timeline",
            Self::Chat => "Chat",
            Self::Export => "Export",
        }
    }
}

/// The keys a command is bound to in one preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    /// Not bound: reachable from the palette only.
    Unbound,
    /// One chord.
    Chord(KeyboardShortcut),
    /// A family of keys shown as text (`0…5`, `J / K / L`), read by a
    /// region rather than dispatched here.
    Text(&'static str),
}

/// One row of the table.
#[derive(Clone, Copy, Debug)]
pub struct Command {
    /// Stable id: what a region looks its keys up by. Never rename one.
    pub id: &'static str,
    /// What the command does, as the palette and the sheet say it.
    pub label: &'static str,
    /// Where its keys are read.
    pub scope: Scope,
    /// The keys under the ave preset, and under any preset that states none.
    pub default: Keys,
    /// The keys under the Premiere preset, when they differ.
    pub premiere: Option<Keys>,
    /// The keys under the Final Cut preset, when they differ.
    pub final_cut: Option<Keys>,
    /// What running it does, for a command the shell dispatches.
    pub run: Option<fn() -> Action>,
}

impl Command {
    /// The keys this command has under `preset`.
    #[must_use]
    pub fn keys(&self, preset: Preset) -> Keys {
        let stated = match preset {
            Preset::Ave => None,
            Preset::Premiere => self.premiere,
            Preset::FinalCut => self.final_cut,
        };
        stated.unwrap_or(self.default)
    }

    /// The keys under `preset`, as the sheet and the palette print them;
    /// empty when unbound.
    #[must_use]
    pub fn hint(&self, preset: Preset) -> String {
        match self.keys(preset) {
            Keys::Unbound => String::new(),
            Keys::Chord(chord) => format_chord(chord),
            Keys::Text(text) => text.to_owned(),
        }
    }
}

/// A chord with no modifier.
const fn key(key: Key) -> Keys {
    Keys::Chord(KeyboardShortcut::new(Modifiers::NONE, key))
}

/// A chord with modifiers.
const fn with(modifiers: Modifiers, key: Key) -> Keys {
    Keys::Chord(KeyboardShortcut::new(modifiers, key))
}

/// Ctrl (Cmd on a Mac).
const CMD: Modifiers = Modifiers::COMMAND;
/// Ctrl+Shift.
const CMD_SHIFT: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
/// Ctrl+Alt.
const CMD_ALT: Modifiers = Modifiers::COMMAND.plus(Modifiers::ALT);

/// A global row: dispatched by the shell.
const fn global(
    id: &'static str,
    label: &'static str,
    default: Keys,
    premiere: Option<Keys>,
    final_cut: Option<Keys>,
    run: fn() -> Action,
) -> Command {
    Command {
        id,
        label,
        scope: Scope::Global,
        default,
        premiere,
        final_cut,
        run: Some(run),
    }
}

/// A region row: read by the region, listed here.
const fn region(id: &'static str, label: &'static str, scope: Scope, default: Keys) -> Command {
    Command {
        id,
        label,
        scope,
        default,
        premiere: None,
        final_cut: None,
        run: None,
    }
}

/// A region row whose keys differ under a preset.
const fn region_in(
    id: &'static str,
    label: &'static str,
    scope: Scope,
    default: Keys,
    premiere: Option<Keys>,
    final_cut: Option<Keys>,
) -> Command {
    Command {
        id,
        label,
        scope,
        default,
        premiere,
        final_cut,
        run: None,
    }
}

/// Every command, global ones first. See the module docs.
pub const COMMANDS: &[Command] = &[
    // ---- the workspace ---------------------------------------------------
    global(
        "palette",
        "Open the command palette",
        with(CMD, Key::K),
        None,
        None,
        || Action::OpenCommands,
    ),
    global(
        "shortcuts",
        "Show the keyboard shortcuts",
        key(Key::Questionmark),
        None,
        None,
        || Action::ToggleShortcuts,
    ),
    global(
        "settings",
        "Open settings",
        with(CMD, Key::Comma),
        None,
        None,
        || Action::OpenSettings,
    ),
    global(
        "open-night",
        "Open a night",
        with(CMD, Key::O),
        None,
        None,
        || Action::PickProject,
    ),
    global(
        "search",
        "Search shots, captions, moments",
        with(CMD, Key::F),
        None,
        None,
        || Action::FocusSearch,
    ),
    global(
        "toggle-chat",
        "Show or hide the chat",
        with(CMD, Key::J),
        None,
        None,
        || Action::TogglePanel(Region::Chat),
    ),
    global(
        "toggle-browser",
        "Show or hide the browser",
        with(CMD, Key::B),
        None,
        None,
        || Action::TogglePanel(Region::Browser),
    ),
    global(
        "toggle-inspector",
        "Show or hide the inspector",
        with(CMD, Key::I),
        None,
        None,
        || Action::TogglePanel(Region::Inspector),
    ),
    global(
        "toggle-timeline",
        "Show or hide the timeline",
        with(CMD, Key::T),
        None,
        None,
        || Action::TogglePanel(Region::Timeline),
    ),
    global(
        "export",
        "Show or hide export",
        with(CMD, Key::E),
        Some(with(CMD, Key::M)),
        None,
        || Action::ToggleExport,
    ),
    // Selects' panel focus keys: Shift+1..4 (Premiere), Cmd+1..4 (FCP).
    global(
        "focus-browser",
        "Focus the browser",
        with(Modifiers::SHIFT, Key::Num1),
        None,
        Some(with(CMD, Key::Num1)),
        || Action::FocusRegion(Region::Browser),
    ),
    global(
        "focus-chat",
        "Focus the chat",
        with(Modifiers::SHIFT, Key::Num2),
        None,
        Some(with(CMD, Key::Num2)),
        || Action::FocusRegion(Region::Chat),
    ),
    global(
        "focus-timeline",
        "Focus the timeline",
        with(Modifiers::SHIFT, Key::Num3),
        None,
        Some(with(CMD, Key::Num3)),
        || Action::FocusRegion(Region::Timeline),
    ),
    global(
        "focus-inspector",
        "Focus the inspector",
        with(Modifiers::SHIFT, Key::Num4),
        None,
        Some(with(CMD, Key::Num4)),
        || Action::FocusRegion(Region::Inspector),
    ),
    global(
        "view-default",
        "View: Default",
        with(Modifiers::ALT, Key::Num1),
        None,
        None,
        || Action::SetViewMode(ViewMode::Default),
    ),
    global(
        "view-vertical",
        "View: Vertical (9:16)",
        with(Modifiers::ALT, Key::Num2),
        None,
        None,
        || Action::SetViewMode(ViewMode::Vertical),
    ),
    global(
        "view-comment",
        "View: Comment",
        with(Modifiers::ALT, Key::Num3),
        None,
        None,
        || Action::SetViewMode(ViewMode::Comment),
    ),
    global(
        "view-preview",
        "View: Preview",
        with(Modifiers::ALT, Key::Num4),
        None,
        None,
        || Action::SetViewMode(ViewMode::Preview),
    ),
    // The page numbers predate the single workspace; under Final Cut the
    // plain Cmd+digits belong to the panel focus keys above.
    global(
        "go-home",
        "Go to Home",
        with(CMD, Key::Num1),
        None,
        Some(with(CMD_ALT, Key::Num1)),
        || Action::Go(Page::Night),
    ),
    global(
        "go-shots",
        "Go to Shots",
        with(CMD, Key::Num2),
        None,
        Some(with(CMD_ALT, Key::Num2)),
        || Action::Go(Page::Review),
    ),
    global(
        "go-people",
        "Go to People",
        with(CMD, Key::Num3),
        None,
        Some(with(CMD_ALT, Key::Num3)),
        || Action::Go(Page::People),
    ),
    global(
        "go-music",
        "Go to Music",
        with(CMD, Key::Num4),
        None,
        Some(with(CMD_ALT, Key::Num4)),
        || Action::Go(Page::Music),
    ),
    global(
        "go-plan",
        "Go to the timeline",
        with(CMD, Key::Num5),
        None,
        Some(with(CMD_ALT, Key::Num5)),
        || Action::Go(Page::Plan),
    ),
    global(
        "go-export",
        "Go to Export",
        with(CMD, Key::Num6),
        None,
        Some(with(CMD_ALT, Key::Num6)),
        || Action::Go(Page::Deliver),
    ),
    global(
        "stage-selection",
        "Mention the selection in chat",
        key(Key::R),
        None,
        None,
        || Action::StageSelection,
    ),
    global(
        "stage-selection-as",
        "Mention the selection as keep / avoid / replace / more like / feature",
        with(Modifiers::SHIFT, Key::R),
        None,
        None,
        || Action::ChooseSelectionIntent,
    ),
    global(
        "reload-night",
        "Reload the night from disk",
        Keys::Unbound,
        None,
        None,
        || Action::ReloadProject,
    ),
    global("quit", "Quit", with(CMD, Key::Q), None, None, || {
        Action::Quit
    }),
    // ---- the viewer ------------------------------------------------------
    region("play", "Play / pause", Scope::Viewer, key(Key::Space)),
    region(
        "shuttle-back",
        "Shuttle back (again: faster)",
        Scope::Viewer,
        key(Key::J),
    ),
    region("shuttle-stop", "Stop", Scope::Viewer, key(Key::K)),
    region(
        "shuttle-forward",
        "Shuttle forward (again: faster)",
        Scope::Viewer,
        key(Key::L),
    ),
    region(
        "frame-back",
        "Frame back",
        Scope::Viewer,
        key(Key::ArrowLeft),
    ),
    region(
        "frame-forward",
        "Frame forward",
        Scope::Viewer,
        key(Key::ArrowRight),
    ),
    region(
        "second-back",
        "One second back",
        Scope::Viewer,
        with(Modifiers::SHIFT, Key::ArrowLeft),
    ),
    region(
        "second-forward",
        "One second forward",
        Scope::Viewer,
        with(Modifiers::SHIFT, Key::ArrowRight),
    ),
    region("mark-in", "Mark in", Scope::Viewer, key(Key::I)),
    region("mark-out", "Mark out", Scope::Viewer, key(Key::O)),
    // Premiere clears marks with Ctrl+Shift+X and plays In to Out with
    // Ctrl+Shift+Space; Final Cut matches ave (Option+X, `/`).
    region_in(
        "clear-in-out",
        "Clear in and out",
        Scope::Viewer,
        with(Modifiers::ALT, Key::X),
        Some(with(CMD_SHIFT, Key::X)),
        None,
    ),
    region_in(
        "play-range",
        "Play the marked range",
        Scope::Viewer,
        key(Key::Slash),
        Some(with(CMD_SHIFT, Key::Space)),
        None,
    ),
    region("loop", "Loop", Scope::Viewer, with(CMD, Key::L)),
    region(
        "mention-range",
        "Mention the marked range or frame in chat",
        Scope::Viewer,
        with(Modifiers::ALT, Key::R),
    ),
    region(
        "info-overlay",
        "Show the clip's numbers over the picture",
        Scope::Viewer,
        with(CMD_SHIFT, Key::I),
    ),
    // ---- the timeline ----------------------------------------------------
    region("undo", "Undo", Scope::Timeline, with(CMD, Key::Z)),
    region("redo", "Redo", Scope::Timeline, with(CMD_SHIFT, Key::Z)),
    region(
        "lift",
        "Lift the selected shots",
        Scope::Timeline,
        Keys::Text("Delete / Backspace"),
    ),
    region(
        "alternatives",
        "Alternatives for the shot",
        Scope::Timeline,
        key(Key::A),
    ),
    region(
        "take-alternative",
        "Take alternative 1…8",
        Scope::Timeline,
        Keys::Text("1…8"),
    ),
    region("tool-select", "Select tool", Scope::Timeline, key(Key::V)),
    region("tool-slip", "Slip tool", Scope::Timeline, key(Key::T)),
    region(
        "zoom-fit",
        "Fit the timeline",
        Scope::Timeline,
        with(Modifiers::SHIFT, Key::Z),
    ),
    region("deselect", "Deselect", Scope::Timeline, key(Key::Escape)),
    region(
        "plan-compare",
        "Compare with the previous version (or the next plan)",
        Scope::Timeline,
        with(Modifiers::SHIFT, Key::C),
    ),
    region(
        "plan-compare-side",
        "Play A or B in the compare",
        Scope::Timeline,
        key(Key::B),
    ),
    // ---- the browser -----------------------------------------------------
    region("pick", "Pick", Scope::Browser, key(Key::P)),
    region("reject", "Reject", Scope::Browser, key(Key::X)),
    region("unmark", "Clear the mark", Scope::Browser, key(Key::U)),
    region("rate", "Rate 0…5 stars", Scope::Browser, Keys::Text("0…5")),
    region(
        "more-like",
        "More like this",
        Scope::Browser,
        with(Modifiers::SHIFT, Key::F),
    ),
    region(
        "play-shot",
        "Play the shot",
        Scope::Browser,
        key(Key::Enter),
    ),
    region(
        "select-all",
        "Select all",
        Scope::Browser,
        with(CMD, Key::A),
    ),
    region(
        "move-cursor",
        "Move (Shift extends)",
        Scope::Browser,
        Keys::Text("Arrows"),
    ),
    region(
        "review-tape",
        "Tape: play every result back to back",
        Scope::Browser,
        key(Key::T),
    ),
    region(
        "review-fast",
        "Fast review: long shots play faster on the tape",
        Scope::Browser,
        with(Modifiers::SHIFT, Key::L),
    ),
    region(
        "review-compare",
        "Compare the select with a candidate",
        Scope::Browser,
        key(Key::C),
    ),
    region(
        "review-compare-candidate",
        "Compare: previous / next candidate",
        Scope::Browser,
        Keys::Text("Left / Right"),
    ),
    region(
        "review-compare-promote",
        "Compare: make the candidate the select",
        Scope::Browser,
        key(Key::ArrowUp),
    ),
    region(
        "review-survey",
        "Survey the selection side by side",
        Scope::Browser,
        key(Key::G),
    ),
    region(
        "review-survey-remove",
        "Survey: drop the focused shot",
        Scope::Browser,
        key(Key::X),
    ),
    region(
        "review-next-group",
        "Survey: next group",
        Scope::Browser,
        key(Key::CloseBracket),
    ),
    region(
        "review-prev-group",
        "Survey: previous group",
        Scope::Browser,
        key(Key::OpenBracket),
    ),
    region(
        "review-exit",
        "Leave tape, compare or survey",
        Scope::Browser,
        key(Key::Escape),
    ),
    // ---- the browser: People ---------------------------------------------
    region(
        "people-yes",
        "People queue: yes, this is them",
        Scope::Browser,
        key(Key::Y),
    ),
    region("people-no", "People queue: no", Scope::Browser, key(Key::N)),
    region(
        "people-skip",
        "People queue: skip",
        Scope::Browser,
        key(Key::S),
    ),
    region(
        "people-queue",
        "People: start the confirm queue",
        Scope::Browser,
        key(Key::Space),
    ),
    region(
        "people-new-person",
        "People: move the picked faces to a new person",
        Scope::Browser,
        with(CMD_SHIFT, Key::M),
    ),
    // Under Premiere Ctrl+M is Export; group with Ctrl+G as Premiere does.
    region_in(
        "people-merge",
        "People: merge the selected people",
        Scope::Browser,
        with(CMD, Key::M),
        Some(with(CMD, Key::G)),
        None,
    ),
    region(
        "people-undo",
        "People: undo",
        Scope::Browser,
        with(CMD, Key::Z),
    ),
    region(
        "people-redo",
        "People: redo",
        Scope::Browser,
        with(CMD_SHIFT, Key::Z),
    ),
    region(
        "people-redo-alt",
        "People: redo (alternative)",
        Scope::Browser,
        with(CMD, Key::Y),
    ),
    region(
        "people-rename",
        "People: rename the person",
        Scope::Browser,
        key(Key::F2),
    ),
    region(
        "people-rename-alt",
        "People: rename the person (alternative)",
        Scope::Browser,
        key(Key::Enter),
    ),
    // ---- the browser: Music ----------------------------------------------
    region(
        "music-grid",
        "Music: show or hide the beat grid",
        Scope::Browser,
        key(Key::Num0),
    ),
    region(
        "music-downbeat-earlier",
        "Music: downbeat one beat earlier",
        Scope::Browser,
        key(Key::Comma),
    ),
    region(
        "music-downbeat-later",
        "Music: downbeat one beat later",
        Scope::Browser,
        key(Key::Period),
    ),
    region(
        "music-nudge-earlier",
        "Music: nudge the grid 10 ms earlier",
        Scope::Browser,
        with(Modifiers::SHIFT, Key::Comma),
    ),
    region(
        "music-nudge-later",
        "Music: nudge the grid 10 ms later",
        Scope::Browser,
        with(Modifiers::SHIFT, Key::Period),
    ),
    // Premiere and Final Cut both drop a marker with M.
    region_in(
        "music-drop",
        "Music: mark or unmark a drop",
        Scope::Browser,
        key(Key::D),
        Some(key(Key::M)),
        Some(key(Key::M)),
    ),
    region(
        "music-from",
        "Music: select bars from here",
        Scope::Browser,
        key(Key::I),
    ),
    region(
        "music-to",
        "Music: select bars to here",
        Scope::Browser,
        key(Key::O),
    ),
    // Final Cut zooms with Cmd+= / Cmd+-.
    region_in(
        "music-zoom-in",
        "Music: zoom in",
        Scope::Browser,
        key(Key::Equals),
        None,
        Some(with(CMD, Key::Equals)),
    ),
    region_in(
        "music-zoom-out",
        "Music: zoom out",
        Scope::Browser,
        key(Key::Minus),
        None,
        Some(with(CMD, Key::Minus)),
    ),
    region(
        "music-deselect",
        "Music: clear the bar selection",
        Scope::Browser,
        key(Key::Escape),
    ),
    // ---- the chat --------------------------------------------------------
    region("send", "Send", Scope::Chat, key(Key::Enter)),
    region(
        "pick-mention",
        "Pick a mention after @",
        Scope::Chat,
        Keys::Text("Up / Down, Enter or Tab"),
    ),
    region("chat-new", "New chat", Scope::Chat, with(CMD_SHIFT, Key::N)),
    // Ctrl+F is the workspace's search, dispatched before the chat sees it.
    region(
        "chat-find",
        "Find in chat",
        Scope::Chat,
        with(CMD_SHIFT, Key::F),
    ),
    region(
        "chat-find-step",
        "Find in chat: next / previous match",
        Scope::Chat,
        Keys::Text("Enter / Shift+Enter"),
    ),
    region(
        "recall-prompt",
        "Recall an earlier prompt with its references",
        Scope::Chat,
        Keys::Text("Up on an empty composer"),
    ),
    // ---- export ----------------------------------------------------------
    region(
        "deliver-render",
        "Render the ticked versions",
        Scope::Export,
        with(CMD, Key::Enter),
    ),
];

/// The command with this id.
#[must_use]
pub fn command(id: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.id == id)
}

/// The chord bound to `id` under `preset`, for a region adopting the table.
#[must_use]
pub fn chord(id: &str, preset: Preset) -> Option<KeyboardShortcut> {
    match command(id)?.keys(preset) {
        Keys::Chord(chord) => Some(chord),
        Keys::Unbound | Keys::Text(_) => None,
    }
}

/// Whether a key event (`key` with `pressed` modifiers) fires the command
/// `id` under `preset`: for a region that collects its key events itself.
#[must_use]
pub fn fires(id: &str, preset: Preset, key: Key, pressed: Modifiers) -> bool {
    chord(id, preset).is_some_and(|chord| matches(chord, key, pressed))
}

/// Whether the command `id` was pressed this frame under `preset`
/// (auto-repeat included, like `InputState::key_pressed`). Leaves the
/// event in place.
#[must_use]
pub fn pressed(input: &egui::InputState, id: &str, preset: Preset) -> bool {
    input.events.iter().any(|event| {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            return false;
        };
        fires(id, preset, *key, *modifiers)
    })
}

/// Like [`pressed`], but removes the matching presses so nothing drawn
/// later reads them too.
pub fn consume(input: &mut egui::InputState, id: &str, preset: Preset) -> bool {
    let before = input.events.len();
    input.events.retain(|event| {
        let egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } = event
        else {
            return true;
        };
        !fires(id, preset, *key, *modifiers)
    });
    input.events.len() != before
}

/// A chord as the sheet prints it: `Ctrl+Shift+Z`, `?`, `Alt+1`.
#[must_use]
pub fn format_chord(chord: KeyboardShortcut) -> String {
    let mut text = egui::ModifierNames::NAMES.format(&chord.modifiers, cfg!(target_os = "macos"));
    if !text.is_empty() {
        text.push('+');
    }
    // Arrows print as words: their symbols are not in every UI face.
    let key = chord.logical_key;
    let arrow = matches!(
        key,
        Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight
    );
    text.push_str(if arrow {
        key.name()
    } else {
        key.symbol_or_name()
    });
    text
}

/// Whether a key event with `pressed` modifiers fires `chord`: exactly the
/// chord's modifiers, except that Shift is not asked of a key (such as
/// `?`) that cannot be typed without it.
#[must_use]
pub fn matches(chord: KeyboardShortcut, key: Key, pressed: Modifiers) -> bool {
    if key != chord.logical_key {
        return false;
    }
    let needs_shift = matches!(
        key,
        Key::Questionmark
            | Key::Exclamationmark
            | Key::Colon
            | Key::Plus
            | Key::Pipe
            | Key::OpenCurlyBracket
            | Key::CloseCurlyBracket
    );
    let pressed = if needs_shift && !chord.modifiers.shift {
        Modifiers {
            shift: false,
            ..pressed
        }
    } else {
        pressed
    };
    pressed.matches_exact(chord.modifiers)
}

/// Reads this frame's key presses against the global commands under
/// `preset`, removes the ones that fired from the input (so no region also
/// acts on them) and returns their actions.
pub fn dispatch(input: &mut egui::InputState, preset: Preset) -> Vec<Action> {
    let mut actions = Vec::new();
    input.events.retain(|event| {
        let egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } = event
        else {
            return true;
        };
        let fired = COMMANDS.iter().find_map(|command| {
            let run = command.run?;
            match command.keys(preset) {
                Keys::Chord(chord) if matches(chord, *key, *modifiers) => Some(run),
                Keys::Chord(_) | Keys::Unbound | Keys::Text(_) => None,
            }
        });
        match fired {
            Some(run) => {
                actions.push(run());
                false
            }
            None => true,
        }
    });
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique() {
        let mut seen = HashSet::new();
        for command in COMMANDS {
            assert!(seen.insert(command.id), "{} is listed twice", command.id);
        }
    }

    #[test]
    fn no_two_global_commands_share_a_chord_in_any_preset() {
        for preset in Preset::ALL {
            let mut seen = HashSet::new();
            for command in COMMANDS.iter().filter(|c| c.scope == Scope::Global) {
                if let Keys::Chord(chord) = command.keys(preset) {
                    assert!(
                        seen.insert(format_chord(chord)),
                        "{} reuses {} under {preset:?}",
                        command.id,
                        format_chord(chord)
                    );
                }
            }
        }
    }

    #[test]
    fn a_global_chord_never_shadows_a_region_chord() {
        for preset in Preset::ALL {
            let global: HashSet<String> = COMMANDS
                .iter()
                .filter(|c| c.scope == Scope::Global)
                .filter_map(|c| match c.keys(preset) {
                    Keys::Chord(chord) => Some(format_chord(chord)),
                    Keys::Unbound | Keys::Text(_) => None,
                })
                .collect();
            for command in COMMANDS.iter().filter(|c| c.scope != Scope::Global) {
                if let Keys::Chord(chord) = command.keys(preset) {
                    assert!(
                        !global.contains(&format_chord(chord)),
                        "{} ({}) is taken by a global command under {preset:?}",
                        command.id,
                        format_chord(chord)
                    );
                }
            }
        }
    }

    #[test]
    fn every_global_command_runs_something_and_regions_do_not() {
        for command in COMMANDS {
            assert_eq!(
                command.run.is_some(),
                command.scope == Scope::Global,
                "{}: the shell dispatches exactly the global rows",
                command.id
            );
        }
    }

    #[test]
    fn every_page_is_reachable_by_a_command() {
        for page in Page::ALL {
            assert!(
                COMMANDS
                    .iter()
                    .filter_map(|c| c.run)
                    .any(|run| run() == Action::Go(page)),
                "{page:?} has no command"
            );
        }
    }

    #[test]
    fn presets_round_trip_and_inherit_the_default() -> anyhow::Result<()> {
        for preset in Preset::ALL {
            assert_eq!(Preset::from_key(preset.key()), Some(preset), "{preset:?}");
        }
        let settings = command("settings").ok_or_else(|| anyhow::anyhow!("no settings"))?;
        assert_eq!(
            settings.hint(Preset::FinalCut),
            settings.hint(Preset::Ave),
            "a preset that states nothing inherits the default"
        );
        assert_eq!(
            chord("focus-chat", Preset::FinalCut),
            Some(KeyboardShortcut::new(Modifiers::COMMAND, Key::Num2)),
            "Final Cut focuses panels with Cmd+digits"
        );
        Ok(())
    }

    #[test]
    fn matching_is_exact_except_for_shifted_symbols() -> anyhow::Result<()> {
        let r = chord("stage-selection", Preset::Ave).ok_or_else(|| anyhow::anyhow!("no R"))?;
        assert!(matches(r, Key::R, Modifiers::NONE), "plain R");
        assert!(!matches(r, Key::R, Modifiers::ALT), "Alt+R is the viewer's");
        assert!(!matches(r, Key::R, Modifiers::SHIFT), "Shift+R is its own");
        let sheet = chord("shortcuts", Preset::Ave).ok_or_else(|| anyhow::anyhow!("no ?"))?;
        assert!(
            matches(sheet, Key::Questionmark, Modifiers::SHIFT),
            "? is typed with Shift"
        );
        Ok(())
    }

    #[test]
    fn dispatch_consumes_what_it_fires_and_leaves_the_rest() {
        let mut input = egui::InputState::default();
        let press = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        input.events = vec![
            press(Key::J, Modifiers::COMMAND),
            press(Key::R, Modifiers::ALT),
        ];
        let actions = dispatch(&mut input, Preset::Ave);
        assert_eq!(actions, vec![Action::TogglePanel(Region::Chat)]);
        assert_eq!(input.events.len(), 1, "Alt+R is left for the viewer");
    }

    #[test]
    fn a_preset_changes_which_key_fires_a_region_command() {
        assert!(
            fires("music-drop", Preset::Ave, Key::D, Modifiers::NONE),
            "ave marks a drop with D"
        );
        assert!(
            !fires("music-drop", Preset::Premiere, Key::D, Modifiers::NONE),
            "D is not Premiere's"
        );
        assert!(
            fires("music-drop", Preset::Premiere, Key::M, Modifiers::NONE),
            "Premiere marks with M"
        );
        let mut input = egui::InputState::default();
        input.events = vec![egui::Event::Key {
            key: Key::G,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        }];
        assert!(
            !pressed(&input, "people-merge", Preset::Ave),
            "ave merges with Ctrl+M"
        );
        assert!(
            consume(&mut input, "people-merge", Preset::Premiere),
            "Premiere merges with Ctrl+G (its Ctrl+M is Export)"
        );
        assert!(input.events.is_empty(), "consume removes the press");
    }

    #[test]
    fn every_region_key_a_page_reads_is_a_chord_in_the_table() {
        for id in [
            "play",
            "shuttle-back",
            "shuttle-stop",
            "shuttle-forward",
            "frame-back",
            "frame-forward",
            "second-back",
            "second-forward",
            "mark-in",
            "mark-out",
            "clear-in-out",
            "play-range",
            "loop",
            "mention-range",
            "info-overlay",
            "people-yes",
            "people-no",
            "people-skip",
            "people-queue",
            "people-new-person",
            "people-merge",
            "people-undo",
            "people-redo",
            "people-redo-alt",
            "people-rename",
            "people-rename-alt",
            "music-grid",
            "music-downbeat-earlier",
            "music-downbeat-later",
            "music-nudge-earlier",
            "music-nudge-later",
            "music-drop",
            "music-from",
            "music-to",
            "music-zoom-in",
            "music-zoom-out",
            "music-deselect",
            "deliver-render",
        ] {
            for preset in Preset::ALL {
                assert!(
                    chord(id, preset).is_some(),
                    "{id} has no chord under {preset:?}"
                );
            }
        }
    }

    #[test]
    fn chords_print_the_way_menus_spell_them() {
        let chord = KeyboardShortcut::new(CMD_SHIFT, Key::Z);
        if !cfg!(target_os = "macos") {
            assert_eq!(format_chord(chord), "Ctrl+Shift+Z");
        }
        assert_eq!(
            format_chord(KeyboardShortcut::new(Modifiers::NONE, Key::Slash)),
            "/"
        );
    }
}

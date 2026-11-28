// ============================================================================
// Module:       gui::ui::hint
// Description:  The status bar's hint: what the control under the pointer
//               does, written there by the control itself on hover.
//
// Dependencies: egui
// ============================================================================

//! Hover hints in the status bar.
//!
//! Every control that says what it does says it twice: a tooltip by the
//! pointer, and the same words (with its shortcut) in the status bar's hint
//! section, which stays readable while the pointer moves on and needs no
//! wait. A control writes its hint with [`Hint::hint`] (or through
//! `widgets::tip`, which the shared buttons use); the status bar reads it
//! with [`current`].
//!
//! The status bar is laid out before the page, so it shows what the last
//! frame's hover wrote; [`set`] asks for one more frame when the hint
//! changes, so it never lags behind the pointer once it stops.

use egui::{Context, Id, Response};

/// What the hovered control said, and on which pass.
#[derive(Clone, Debug, Default, PartialEq)]
struct Said {
    text: String,
    keys: String,
    pass: u64,
}

/// Where the hint lives in the context's memory.
fn id() -> Id {
    Id::unique("ave-status-hint")
}

/// Records `text` (and its shortcut `keys`, may be empty) as the hovered
/// control's hint for this frame.
pub fn set(ctx: &Context, text: &str, keys: &str) {
    if text.is_empty() {
        return;
    }
    let pass = ctx.cumulative_pass_nr();
    let said = Said {
        text: text.to_owned(),
        keys: keys.to_owned(),
        pass,
    };
    let changed = ctx.data_mut(|d| {
        let old = d.get_temp::<Said>(id());
        let changed = old
            .as_ref()
            .is_none_or(|o| o.text != said.text || o.keys != said.keys);
        d.insert_temp(id(), said);
        changed
    });
    if changed {
        ctx.request_repaint();
    }
}

/// The hint to show now: what a control wrote this frame or the last one
/// (the status bar draws before the page), as `(text, keys)`.
pub fn current(ctx: &Context) -> Option<(String, String)> {
    let now = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<Said>(id()))
        .filter(|s| s.pass + 1 >= now)
        .map(|s| (s.text, s.keys))
}

/// A response that can say what its control does.
pub trait Hint {
    /// Shows `text` as a tooltip and, while hovered, in the status bar.
    fn hint(self, text: impl AsRef<str>) -> Self;
}

impl Hint for Response {
    fn hint(self, text: impl AsRef<str>) -> Self {
        let text = text.as_ref();
        if text.is_empty() {
            return self;
        }
        if self.hovered() {
            set(&self.ctx, text, "");
        }
        name_for_screen_readers(&self, text);
        // Whether it showed changes nothing: the response goes back either way.
        let _ = egui_material3::MaterialTooltip::new(text).show_for(&self);
        self
    }
}

/// Gives `response`'s widget `text` as its accessible description, and as
/// its name when it has none (an icon-only button, an unlabelled tick box),
/// as egui's own text tooltips do: a Material tooltip names only itself.
pub fn name_for_screen_readers(response: &Response, text: &str) {
    if !response.ctx.has_accesskit_node(response.id) {
        return;
    }
    response.ctx.accesskit_node_builder(response.id, |node| {
        if node.description().is_none() {
            node.set_description(text);
        }
        if node.role() != egui::accesskit::Role::Label
            && node.labelled_by().is_empty()
            && node.label().is_none_or(str::is_empty)
        {
            node.set_label(text);
        }
    });
}

/// Writes the description of a shared control labelled `label` while it is
/// hovered (see [`describe`]). A specific [`Hint::hint`] on the response
/// afterwards replaces it: the last hint written in a frame is the one shown.
pub fn auto(response: &Response, label: &str) {
    if response.hovered()
        && let Some(text) = describe(label)
    {
        set(&response.ctx, text, "");
    }
}

/// What the app's recurring controls do, by their label: the words the
/// status bar shows for a control that says nothing more specific.
#[must_use]
pub fn describe(label: &str) -> Option<&'static str> {
    let label = label.trim_end_matches('\u{2026}').trim_end_matches("...");
    Some(match label {
        // The guide and the Home pipeline.
        "Footage" => "The folder of clips this project is cut from",
        "Analyse" => "Measure every clip (light, focus, motion, shots) and make playback copies",
        "Music" => "The track the cut is timed to, or none (cuts paced every two seconds)",
        "First cut" => "Draft each deliverable from the footage and the rules",
        "Review" => "Play the cut, swap shots, ask the chat for changes",
        "Export" => "Render the videos (and timelines for Resolve) into the output folder",
        "Update" => "Analyse new or changed clips, then make any missing playback copies",
        "Run" => "Start this step",
        "Redraft" => "Draft it again from scratch; edits made by hand are lost",
        "Draft" | "Draft missing" => "Draft the deliverables that have no cut yet",
        "Run everything" => "Run every step that is missing, in order",
        "Open export" | "Export again" => "Open the export panel: render and deliver the cuts",
        "Open editor" => "Open the timeline editor on this project",
        "Re-map" | "Map the music" => "Find the track's tempo, beats, bars and drops",
        "Choose music" => "Pick a track for the cut to follow its beat",
        "Fill gaps" => "Caption the segments that have no caption yet",
        "Set up and analyse" => "Choose footage, music and what to make, then start the analysis",
        "Prepare smooth playback" => "Make light playback copies of 4K and HEVC clips",
        "Draft the first cut" => "Draft every deliverable that has no cut yet",
        "A cut of my own" => "Draft one cut with the length, shape and pacing you choose",
        "Review the cut" => "Open the editor on the first cut",
        "Export the cut" => "Done reviewing? Render the cuts and deliver them",
        "Progress" => "What each step and clip has been through",
        "Details" => "The full story of what was skipped or failed",
        "Dismiss" => "Hide this note",
        // Set-up.
        "Continue" => "On to the next step of set-up",
        "Back" => "Back to the previous step",
        "Skip setup" => "Close set-up without running anything",
        "Start analysis" => "Run the ticked steps, in order, on this machine",
        "Use that folder" => "Use the folder the clips were found in",
        // Workspace tabs and panels.
        "Shots" => "Browse the analysed shots, best first",
        "People" => "Who is in which footage; rename and split them here",
        "Inspector" => "Everything known about what is selected",
        "Facts" => "The numbers behind what is selected",
        "Shot list" => "The open cut as a list, shot by shot",
        "Chat" => "Ask for edits or questions about the footage",
        "Comments" => "Notes pinned to moments of the cut",
        "Whole cut" => "The chat sees the whole cut",
        "Selected shots" => "The chat sees only the shots you selected",
        "Before the drop" => "The chat works on the part before the drop",
        "From the drop" => "The chat works on the drop and after",
        // Export.
        "Render anyway" => "Render even though the check found problems",
        "Only the passing ones" => "Render only the cuts that pass their check",
        // Settings and dialogs.
        "Change" => "Pick a different file or folder",
        "Done" => "Close this",
        "Cancel" => "Stop without changing anything",
        "Save" => "Keep the changes",
        "Discard" => "Throw these changes away",
        "Approve" => "Let the assistant run this",
        "Sign in" => "Sign in to Claude in the browser, for the chat",
        "Back to the template's rules" => "Put every drafting rule back to what the template sets",
        "x264" | "Use x264" => "Encode on the processor: slower, the most compatible",
        "NVENC" | "Use NVENC" => "Encode on the NVIDIA graphics card: several times faster",
        "AMF" => "Encode on an AMD graphics card: faster",
        "x264 (CPU)" => "Encode on the processor: slower, the most compatible",
        "AMF (AMD GPU)" => "Encode on an AMD graphics card: faster",
        "NVENC (NVIDIA GPU)" => "Encode on an NVIDIA graphics card: several times faster",
        "Any shot" => "Any shot may land on the drop",
        "Fire" => "The drop must land on a fire shot",
        "LED" => "The drop must land on an LED shot",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{current, describe, set};

    #[test]
    fn a_hint_shows_for_the_frame_after_and_then_clears() -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let frame = |run: &mut dyn FnMut(&mut egui::Ui)| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| run(ui));
            out.textures_delta.clear();
        };
        frame(&mut |ui| set(ui.ctx(), "Open the export panel", "Ctrl+E"));
        let mut seen = None;
        frame(&mut |ui| seen = current(ui.ctx()));
        assert_eq!(
            seen,
            Some(("Open the export panel".to_owned(), "Ctrl+E".to_owned())),
            "the status bar, laid out first, shows the last frame's hint"
        );
        for _ in 0..2 {
            frame(&mut |ui| seen = current(ui.ctx()));
        }
        assert_eq!(seen, None, "nothing hovered for two frames: no hint");
        Ok(())
    }

    #[test]
    fn recurring_controls_are_described() -> anyhow::Result<()> {
        assert!(
            describe("Change\u{2026}").is_some(),
            "the ellipsis is ignored"
        );
        assert!(describe("Update").is_some(), "a pipeline button");
        assert_eq!(
            describe("Something new"),
            None,
            "unknown labels say nothing"
        );
        Ok(())
    }
}

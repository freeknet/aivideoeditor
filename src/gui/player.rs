// ============================================================================
// Module:       gui::player
// Description:  The shared viewer: one source (a clip range, or a plan) played
//               with transport controls, framed for any aspect ratio.
//
// Dependencies: egui; crate::gui::thumbs
// ============================================================================

//! The viewer every page shares.
//!
//! **Contract (see `docs/GUI_CONTRACTS.md`).** Pages only call the methods
//! below; the decoding behind them is free to change.
//!
//! This first version previews by scrubbing thumbnails at ~12 fps through
//! the thumbnail cache: enough to judge a moment, with no audio. Real-time
//! decoded playback with audio replaces the internals, not the API.

use std::path::{Path, PathBuf};
use std::time::Instant;

use super::thumbs::Thumbs;

/// What the viewer is showing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Source {
    /// Nothing loaded.
    Empty,
    /// A range of one clip.
    Clip {
        /// Source file.
        path: PathBuf,
        /// Range start, seconds.
        t0: f64,
        /// Range end, seconds.
        t1: f64,
        /// Horizontal crop centre for narrow aspects.
        cx: f32,
    },
}

/// The viewer.
pub(crate) struct Player {
    source: Source,
    /// Position within the source range, seconds from `t0`.
    pos: f64,
    playing: bool,
    looping: bool,
    rate: f64,
    last_tick: Option<Instant>,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            source: Source::Empty,
            pos: 0.0,
            playing: false,
            looping: true,
            rate: 1.0,
            last_tick: None,
        }
    }
}

impl Player {
    /// Loads a clip range and rewinds to its start (does not start playing).
    pub(crate) fn open_clip(&mut self, path: &Path, t0: f64, t1: f64, cx: f32) {
        let next = Source::Clip {
            path: path.to_path_buf(),
            t0,
            t1: t1.max(t0 + 0.04),
            cx,
        };
        if self.source != next {
            self.source = next;
            self.pos = 0.0;
        }
    }

    /// What is loaded.
    pub(crate) fn source(&self) -> &Source {
        &self.source
    }

    /// Starts or stops playback.
    pub(crate) fn toggle(&mut self) {
        self.playing = !self.playing;
        self.last_tick = None;
    }

    /// Starts playback.
    pub(crate) fn play(&mut self) {
        self.playing = true;
    }

    /// Stops playback.
    pub(crate) fn pause(&mut self) {
        self.playing = false;
    }

    /// Whether it is playing.
    pub(crate) fn playing(&self) -> bool {
        self.playing
    }

    /// Playback speed (J/K/L style: negative plays backwards).
    pub(crate) fn set_rate(&mut self, rate: f64) {
        self.rate = rate;
    }

    /// Seeks to `t` seconds from the range start.
    pub(crate) fn seek(&mut self, t: f64) {
        self.pos = t.clamp(0.0, self.duration());
    }

    /// Position from the range start, seconds.
    pub(crate) fn position(&self) -> f64 {
        self.pos
    }

    /// Length of the loaded range, seconds.
    pub(crate) fn duration(&self) -> f64 {
        match &self.source {
            Source::Empty => 0.0,
            Source::Clip { t0, t1, .. } => t1 - t0,
        }
    }

    /// Advances the clock; call once per frame before drawing.
    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.playing {
            if let Some(last) = self.last_tick {
                self.pos += now.duration_since(last).as_secs_f64() * self.rate;
                let d = self.duration();
                if self.pos > d || self.pos < 0.0 {
                    if self.looping {
                        self.pos = self.pos.rem_euclid(d.max(0.04));
                    } else {
                        self.pos = self.pos.clamp(0.0, d);
                        self.playing = false;
                    }
                }
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
        self.last_tick = Some(now);
    }

    /// Draws the frame into a box of `size`, framed for `aspect` (w/h).
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        thumbs: &mut Thumbs,
        size: egui::Vec2,
        aspect: f32,
        background: egui::Color32,
    ) -> egui::Response {
        self.tick(ui.ctx());
        let (outer, response) = ui.allocate_exact_size(size, egui::Sense::click());
        ui.painter()
            .rect_filled(outer, egui::CornerRadius::same(6), egui::Color32::BLACK);
        let fit = if size.x / size.y > aspect {
            egui::vec2(size.y * aspect, size.y)
        } else {
            egui::vec2(size.x, size.x / aspect)
        };
        let rect = egui::Rect::from_center_size(outer.center(), fit);
        match &self.source {
            Source::Empty => {
                ui.painter()
                    .rect_filled(rect, egui::CornerRadius::same(4), background);
            }
            Source::Clip { path, t0, cx, .. } => {
                // Snap to ~12 fps so scrubbing reuses cached frames.
                let t = t0 + (self.pos * 12.0).floor() / 12.0;
                let path = path.clone();
                thumbs.paint(ui, rect, &path, t, *cx, background);
            }
        }
        if response.clicked() {
            self.toggle();
        }
        response
    }

    /// Handles the shared transport keys: Space play/pause, J/K/L shuttle
    /// (repeat J or L to go faster), Left/Right one frame-ish (1/12 s).
    /// Call only when the page may read keys (`PageCtx::keys_free`).
    pub(crate) fn handle_keys(&mut self, ui: &egui::Ui) {
        let (space, j, k, l, left, right) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::J),
                i.key_pressed(egui::Key::K),
                i.key_pressed(egui::Key::L),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
            )
        });
        if matches!(self.source, Source::Empty) {
            return;
        }
        if space {
            self.toggle();
        }
        if k {
            self.pause();
            self.set_rate(1.0);
        }
        for (pressed, sign) in [(l, 1.0), (j, -1.0)] {
            if pressed {
                let faster = self.playing() && self.rate.signum() == sign;
                self.set_rate(if faster {
                    (self.rate * 2.0).clamp(-8.0, 8.0)
                } else {
                    sign
                });
                self.play();
            }
        }
        if left {
            self.pause();
            self.seek(self.position() - 1.0 / 12.0);
        }
        if right {
            self.pause();
            self.seek(self.position() + 1.0 / 12.0);
        }
    }

    /// Draws the transport row: play/pause, time, a scrub bar.
    pub(crate) fn transport(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button(if self.playing { "⏸" } else { "▶" }).clicked() {
                self.toggle();
            }
            let d = self.duration();
            let mut pos = self.pos;
            let slider = egui::Slider::new(&mut pos, 0.0..=d.max(0.04)).show_value(false);
            if ui.add(slider).changed() {
                self.seek(pos);
            }
            ui.label(
                egui::RichText::new(format!("{:5.2} / {:5.2}s", self.pos, d))
                    .monospace()
                    .small(),
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::Player;

    #[test]
    fn seeking_is_clamped_to_the_range() {
        let mut p = Player::default();
        p.open_clip(std::path::Path::new("a.mov"), 10.0, 12.0, 0.5);
        p.seek(5.0);
        assert!(
            (p.position() - 2.0).abs() < 1e-9,
            "clamped to the 2 s range"
        );
        p.seek(-1.0);
        assert!(p.position().abs() < 1e-9, "never before the start");
    }
}

// ============================================================================
// Module:       gui::player
// Description:  The shared viewer: one source (a clip range, a cut, or a
//               music range) played with real decoded video and sound.
//
// Dependencies: egui, cpal; crate::gui::thumbs, crate::media
// ============================================================================

//! The viewer every page shares.
//!
//! **Contract (see `docs/GUI_CONTRACTS.md`).** Pages only call the methods
//! below; the decoding behind them is free to change.
//!
//! **How it plays.** Every source is reduced to segments laid back to back
//! ([`clock::Seg`]: a clip is one, a cut is one per shot). The segment under
//! the playhead gets a decoder thread ([`video::VideoStream`]) that streams
//! scaled RGBA frames into a small ring; the UI uploads the frame matching
//! the clock into one reused texture. Near the end of a shot the next
//! shot's decoder starts (pre-roll) so cuts land on the frame. Seeking
//! restarts the decoder; while the playhead is being dragged, or shuttled
//! faster than ×2 or backwards, frames come from the thumbnail cache.
//!
//! **Clock.** While playing forward at ×1 with sound ([`audio::Audio`]),
//! the samples the device has consumed *are* the clock and video follows;
//! otherwise (no device, no audio stream, shuttle speeds) the wall clock.
//! A cut plays its music from the plan's offset with clip sound muted,
//! because beat sync is judged against the music; a clip plays its own
//! sound; a music range plays the track.

mod audio;
pub(crate) mod clock;
mod ring;
pub(crate) mod transport;
pub(crate) mod video;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use self::clock::Seg;
use self::video::{Probes, VideoStream};
use super::thumbs::{Thumbs, cover_uv};

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
    /// A cut: shots back to back, optionally over a music track.
    Sequence {
        /// The shots, in order.
        shots: Vec<SeqShot>,
        /// The music file and the second of it the cut starts at.
        music: Option<(PathBuf, f64)>,
    },
    /// An audio file range (the Music tab).
    Audio {
        /// The file.
        path: PathBuf,
        /// Range start, seconds.
        t0: f64,
        /// Range end, seconds.
        t1: f64,
    },
}

/// One shot of a [`Source::Sequence`].
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SeqShot {
    /// Source file.
    pub(crate) path: PathBuf,
    /// In-point in the source, seconds.
    pub(crate) t_in: f64,
    /// Length on the timeline, seconds.
    pub(crate) dur: f64,
    /// Horizontal crop centre.
    pub(crate) cx: f32,
}

/// How [`Player::show_framed`] frames the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Framing {
    /// Output aspect ratio, width over height.
    pub(crate) aspect: f32,
    /// Show the whole source frame with the area outside the crop dimmed,
    /// instead of only the crop.
    pub(crate) ghost: bool,
}

/// Where [`Player::show_framed`] drew.
pub(crate) struct Shown {
    /// The stage's response (click toggles play).
    pub(crate) response: egui::Response,
    /// The output frame on screen (the crop, at the framing's aspect).
    pub(crate) frame: egui::Rect,
}

/// The frame under the playhead, for overlays and references.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Here {
    /// Source file.
    pub(crate) path: PathBuf,
    /// Time in that file, seconds.
    pub(crate) file_t: f64,
    /// File time the segment ends at, seconds.
    pub(crate) seg_out: f64,
    /// Segment index (the shot, for a cut).
    pub(crate) seg: usize,
    /// Number of segments.
    pub(crate) segs: usize,
}

/// The audio session playing now.
struct Session {
    started: Instant,
    /// Gave up on it (no sound, stalled, or ran out): wall clock instead.
    dead: bool,
}

/// What the texture currently holds.
#[derive(Clone, Debug, PartialEq)]
struct Uploaded {
    path: PathBuf,
    t: f64,
}

/// Default decoded preview width, pixels.
pub(crate) const PREVIEW_WIDTH: u32 = 960;

/// How long the playhead must rest, paused, before a decoder is started
/// for the exact frame (scrubbing shows thumbnails until then).
const SETTLE: Duration = Duration::from_millis(120);

/// How far ahead of the playhead upcoming shots get their decoders,
/// seconds. ffmpeg can take a couple of seconds to deliver a first frame on
/// a loaded machine, so short shots need several decoders warming at once.
const PREROLL: f64 = 2.5;

/// Most decoders warming ahead of the playhead at once.
const MAX_AHEAD: usize = 3;

/// Longest the clock waits for the first frame before playing anyway.
const PRIME_TIMEOUT: Duration = Duration::from_secs(3);

/// The viewer.
pub(crate) struct Player {
    source: Source,
    segs: Vec<Seg>,
    /// Position within the source range, seconds from its start.
    pos: f64,
    playing: bool,
    looping: bool,
    rate: f64,
    last_tick: Option<Instant>,
    mark_in: Option<f64>,
    mark_out: Option<f64>,
    /// Playing the I/O range (`/`): stop or loop at O.
    range_play: bool,
    volume: f32,
    muted: bool,
    preview_width: u32,
    /// The scrub bar is being dragged.
    scrubbing: bool,
    probes: Probes,
    streams: Vec<VideoStream>,
    texture: Option<egui::TextureHandle>,
    uploaded: Option<Uploaded>,
    /// When the paused playhead last moved.
    moved_at: Instant,
    last_pos: f64,
    audio: Option<audio::Audio>,
    session: Option<Session>,
    /// The audio must restart at the playhead (seek, wrap, source change).
    audio_dirty: bool,
    /// The clock may run: the frame under the playhead has been shown since
    /// the last play / seek (or the wait timed out).
    primed: bool,
    /// When the current wait for a first frame began.
    prime_since: Option<Instant>,
    /// The last drawn picture was the decoded frame for the playhead.
    frame_ok: bool,
    /// The playhead jumped: decoders restart rather than catch up.
    jumped: bool,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            source: Source::Empty,
            segs: Vec::new(),
            pos: 0.0,
            playing: false,
            looping: true,
            rate: 1.0,
            last_tick: None,
            mark_in: None,
            mark_out: None,
            range_play: false,
            volume: 0.8,
            muted: false,
            preview_width: PREVIEW_WIDTH,
            scrubbing: false,
            probes: Probes::default(),
            streams: Vec::new(),
            texture: None,
            uploaded: None,
            moved_at: Instant::now(),
            last_pos: 0.0,
            audio: None,
            session: None,
            audio_dirty: true,
            primed: false,
            prime_since: None,
            frame_ok: false,
            jumped: false,
        }
    }
}

impl Player {
    /// Loads a clip range and rewinds to its start (does not start playing).
    pub(crate) fn open_clip(&mut self, path: &Path, t0: f64, t1: f64, cx: f32) {
        self.load(Source::Clip {
            path: path.to_path_buf(),
            t0,
            t1: t1.max(t0 + 0.04),
            cx,
        });
    }

    /// Loads a cut (shots back to back over `music`) at its start.
    pub(crate) fn open_sequence(&mut self, shots: Vec<SeqShot>, music: Option<(PathBuf, f64)>) {
        self.load(Source::Sequence { shots, music });
    }

    /// Loads an audio range (no picture).
    pub(crate) fn open_audio(&mut self, path: &Path, t0: f64, t1: f64) {
        self.load(Source::Audio {
            path: path.to_path_buf(),
            t0,
            t1: t1.max(t0 + 0.04),
        });
    }

    /// Replaces the source unless it is the one already loaded.
    fn load(&mut self, next: Source) {
        if self.source == next {
            return;
        }
        self.segs = segments(&next);
        self.source = next;
        self.pos = 0.0;
        self.mark_in = None;
        self.mark_out = None;
        self.range_play = false;
        self.streams.clear();
        self.uploaded = None;
        self.unprime();
        self.end_audio();
    }

    /// Makes the clock wait for the frame under the playhead.
    fn unprime(&mut self) {
        self.primed = false;
        self.prime_since = None;
        self.frame_ok = false;
    }

    /// For a sequence: the shot under the playhead and the time within it.
    pub(crate) fn sequence_shot(&self) -> Option<(usize, f64)> {
        let Source::Sequence { .. } = &self.source else {
            return None;
        };
        clock::locate(&self.segs, self.pos)
    }

    /// What is loaded.
    pub(crate) fn source(&self) -> &Source {
        &self.source
    }

    /// The frame under the playhead (`None` for audio or nothing).
    pub(crate) fn here(&self) -> Option<Here> {
        self.here_at(self.pos)
    }

    /// The frame at position `pos` (seconds from the source start).
    pub(crate) fn here_at(&self, pos: f64) -> Option<Here> {
        let (seg, within) = clock::locate(&self.segs, pos)?;
        let s = self.segs.get(seg)?;
        Some(Here {
            path: s.path.clone(),
            file_t: s.file_time(within),
            seg_out: s.t_out(),
            seg,
            segs: self.segs.len(),
        })
    }

    /// Where segment `index` starts, seconds from the source start (shot
    /// boundaries for the scrub bar).
    pub(crate) fn seg_starts(&self) -> Vec<f64> {
        (1..self.segs.len())
            .map(|i| clock::seg_start(&self.segs, i))
            .collect()
    }

    /// The frame rate of the picture under the playhead (23.976 when not
    /// known yet), for timecode and frame steps.
    pub(crate) fn fps(&self) -> f64 {
        let path = match (&self.source, self.here()) {
            (Source::Audio { .. } | Source::Empty, _) | (_, None) => None,
            (Source::Clip { .. } | Source::Sequence { .. }, Some(h)) => Some(h.path),
        };
        path.and_then(|p| {
            let map = self.probes.lock().ok()?;
            let fps = map.get(&p)?.fps_f64();
            (fps >= 1.0).then_some(fps)
        })
        .unwrap_or(24000.0 / 1001.0)
    }

    /// Starts or stops playback.
    pub(crate) fn toggle(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Starts playback.
    pub(crate) fn play(&mut self) {
        if !self.playing {
            self.playing = true;
            self.last_tick = None;
            self.audio_dirty = true;
            self.unprime();
            if self.pos >= self.duration() - 1e-3 && self.rate > 0.0 {
                self.pos = 0.0;
            }
        }
    }

    /// Stops playback.
    pub(crate) fn pause(&mut self) {
        self.playing = false;
        self.range_play = false;
        self.end_audio();
    }

    /// Whether it is playing.
    pub(crate) fn playing(&self) -> bool {
        self.playing
    }

    /// Playback speed (J/K/L style: negative plays backwards).
    pub(crate) fn set_rate(&mut self, rate: f64) {
        if (rate - self.rate).abs() > 1e-9 {
            self.rate = rate;
            self.audio_dirty = true;
        }
    }

    /// Playback speed.
    pub(crate) fn rate(&self) -> f64 {
        self.rate
    }

    /// Seeks to `t` seconds from the range start.
    pub(crate) fn seek(&mut self, t: f64) {
        let t = t.clamp(0.0, self.duration());
        if (t - self.pos).abs() > 1e-6 {
            if (t - self.pos).abs() > 0.5 {
                self.jumped = true;
            }
            self.pos = t;
            self.audio_dirty = true;
            self.unprime();
            if self.range_play
                && let (Some(i), Some(o)) = (self.mark_in, self.mark_out)
                && (t < i || t > o)
            {
                self.range_play = false;
            }
        }
    }

    /// Position from the range start, seconds.
    pub(crate) fn position(&self) -> f64 {
        self.pos
    }

    /// Length of the loaded range, seconds.
    pub(crate) fn duration(&self) -> f64 {
        match &self.source {
            Source::Empty => 0.0,
            Source::Clip { t0, t1, .. } | Source::Audio { t0, t1, .. } => t1 - t0,
            Source::Sequence { .. } => clock::total(&self.segs),
        }
    }

    /// Whether playback loops at the end.
    pub(crate) fn looping(&self) -> bool {
        self.looping
    }

    /// Turns looping on or off.
    pub(crate) fn set_looping(&mut self, on: bool) {
        self.looping = on;
    }

    /// The I/O marks, seconds from the range start.
    pub(crate) fn marks(&self) -> (Option<f64>, Option<f64>) {
        (self.mark_in, self.mark_out)
    }

    /// The marked range, ordered, when both marks are set.
    pub(crate) fn marked(&self) -> Option<(f64, f64)> {
        let (i, o) = (self.mark_in?, self.mark_out?);
        Some((i.min(o), i.max(o)))
    }

    /// Sets the in mark at the playhead.
    pub(crate) fn mark_in(&mut self) {
        self.mark_in = Some(self.pos);
        if self.mark_out.is_some_and(|o| o < self.pos) {
            self.mark_out = None;
        }
    }

    /// Sets the out mark at the playhead.
    pub(crate) fn mark_out(&mut self) {
        self.mark_out = Some(self.pos);
        if self.mark_in.is_some_and(|i| i > self.pos) {
            self.mark_in = None;
        }
    }

    /// Clears both marks.
    pub(crate) fn clear_marks(&mut self) {
        self.mark_in = None;
        self.mark_out = None;
        self.range_play = false;
    }

    /// Plays the marked range from I (to the end from I, or from the start
    /// to O, when only one mark is set).
    pub(crate) fn play_marks(&mut self) {
        let lo = self.mark_in.unwrap_or(0.0);
        let hi = self.mark_out.unwrap_or(self.duration());
        if hi - lo < 1e-3 {
            return;
        }
        self.mark_in = Some(lo);
        self.mark_out = Some(hi);
        self.set_rate(1.0);
        self.seek(lo);
        self.audio_dirty = true;
        self.playing = true;
        self.last_tick = None;
        self.range_play = true;
    }

    /// Volume, 0 to 1.
    pub(crate) fn volume(&self) -> f32 {
        self.volume
    }

    /// Whether sound is muted.
    pub(crate) fn muted(&self) -> bool {
        self.muted
    }

    /// Sets volume and mute.
    pub(crate) fn set_volume(&mut self, volume: f32, muted: bool) {
        self.volume = volume.clamp(0.0, 1.0);
        self.muted = muted;
        if let Some(a) = &self.audio {
            a.set_volume(self.volume, self.muted);
        }
    }

    /// Sets the preview width; decoders restart at the new size.
    pub(crate) fn set_preview_width(&mut self, width: u32) {
        let width = width.clamp(256, 3840);
        if width != self.preview_width {
            self.preview_width = width;
            self.streams.clear();
        }
    }

    /// Marks the scrub bar as grabbed or released. While grabbed, frames
    /// come from thumbnails and the clock is held.
    pub(crate) fn set_scrubbing(&mut self, on: bool) {
        if on != self.scrubbing {
            self.scrubbing = on;
            self.audio_dirty = true;
            self.last_tick = None;
        }
    }

    /// Steps `frames` frames (negative: back) and pauses.
    pub(crate) fn step_frames(&mut self, frames: f64) {
        self.pause();
        let fd = 1.0 / self.fps();
        // Land on a frame boundary so repeated steps do not drift.
        let here = (self.pos / fd + 1e-6).floor();
        self.seek((here + frames) * fd + fd * 0.25);
    }

    /// Stops the audio session.
    fn end_audio(&mut self) {
        if self.session.take().is_some()
            && let Some(a) = &mut self.audio
        {
            a.stop();
        }
        self.audio_dirty = true;
    }

    /// The file range the sound for position `pos` comes from:
    /// `(file, file time, length)`.
    fn audio_source(&self, pos: f64) -> Option<(PathBuf, f64, f64)> {
        let rest = (self.duration() - pos).max(0.05) + 0.25;
        match &self.source {
            Source::Empty => None,
            Source::Clip { path, t0, .. } => {
                let known_silent = self
                    .probes
                    .lock()
                    .ok()
                    .and_then(|m| m.get(path).map(|p| !p.has_audio))
                    .unwrap_or(false);
                (!known_silent).then(|| (path.clone(), t0 + pos, rest))
            }
            Source::Audio { path, t0, .. } => Some((path.clone(), t0 + pos, rest)),
            Source::Sequence { music, .. } => music
                .as_ref()
                .map(|(m, offset)| (m.clone(), offset + pos, rest)),
        }
    }

    /// Advances the clock; call once per frame before drawing.
    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let dt = self
            .last_tick
            .map_or(0.0, |last| now.duration_since(last).as_secs_f64())
            .min(0.25);
        self.last_tick = Some(now);
        if !self.playing || self.scrubbing || matches!(self.source, Source::Empty) {
            if self.session.is_some() {
                self.end_audio();
            }
            if (self.pos - self.last_pos).abs() > 1e-9 {
                self.moved_at = now;
                self.last_pos = self.pos;
            }
            return;
        }
        if !self.primed {
            let since = *self.prime_since.get_or_insert(now);
            let decodes = self.rate > 0.0 && self.rate <= 2.0;
            let ready = self.frame_ok
                || self.segs.is_empty()
                || !decodes
                || now.duration_since(since) > PRIME_TIMEOUT;
            if !ready {
                // Hold the clock (and the sound) until the picture is there.
                ctx.request_repaint();
                return;
            }
            self.primed = true;
            self.prime_since = None;
        }
        let wants_sound = (self.rate - 1.0).abs() < 1e-9;
        if !wants_sound && self.session.is_some() {
            self.end_audio();
        }
        if wants_sound && (self.audio_dirty || self.session.is_none()) {
            self.start_audio(now);
        }
        let next = match self.audio_clock(now) {
            Some(Some(t)) => t,
            // Sound is starting: hold the picture until it does.
            Some(None) => self.pos,
            None => self.pos + dt * self.rate,
        };
        let (lo, hi) = match (self.range_play, self.marked()) {
            (true, Some(range)) => range,
            (true, None) | (false, _) => (0.0, self.duration()),
        };
        let (bounded, go) = clock::bound(next, lo, hi, self.looping);
        if (bounded - next).abs() > 1e-6 {
            // Wrapped or clamped: the sound must restart (or stop), and the
            // picture at the new position must be ready first.
            self.audio_dirty = true;
            self.unprime();
        }
        self.pos = bounded;
        self.last_pos = bounded;
        self.moved_at = now;
        if !go {
            self.playing = false;
            self.range_play = false;
            self.end_audio();
        }
        ctx.request_repaint();
    }

    /// Starts sound at the playhead, if the source has any.
    fn start_audio(&mut self, now: Instant) {
        self.audio_dirty = false;
        let Some((file, t, len)) = self.audio_source(self.pos) else {
            self.end_audio();
            self.audio_dirty = false;
            return;
        };
        let audio = self.audio.get_or_insert_with(|| {
            let a = audio::Audio::default();
            a.set_volume(0.8, false);
            a
        });
        audio.set_volume(self.volume, self.muted);
        audio.start(&file, t, len, self.pos);
        self.session = Some(Session {
            started: now,
            dead: false,
        });
    }

    /// The audio clock: `None` when the wall clock rules, `Some(None)`
    /// while the sound is starting, `Some(Some(t))` once it plays.
    fn audio_clock(&mut self, now: Instant) -> Option<Option<f64>> {
        let session = self.session.as_mut()?;
        if session.dead {
            return None;
        }
        let status = self.audio.as_ref()?.status();
        let stalled =
            !status.started && now.duration_since(session.started) > Duration::from_millis(900);
        // Drained before starting: the file has no sound. Either way, or
        // with no device, or a stall, the wall clock takes over.
        if status.failed || stalled || status.drained {
            session.dead = true;
            return None;
        }
        Some(status.started.then_some(status.pos))
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
        let framing = Framing {
            aspect,
            ghost: false,
        };
        self.show_framed(ui, thumbs, size, framing, background)
            .response
    }

    /// Draws the frame into a black stage of `size` and says where the
    /// output frame landed (for safe-area overlays).
    pub(crate) fn show_framed(
        &mut self,
        ui: &mut egui::Ui,
        thumbs: &mut Thumbs,
        size: egui::Vec2,
        framing: Framing,
        background: egui::Color32,
    ) -> Shown {
        self.tick(ui.ctx());
        let (stage, response) = ui.allocate_exact_size(size, egui::Sense::click());
        ui.painter()
            .rect_filled(stage, egui::CornerRadius::same(4), egui::Color32::BLACK);
        let aspect = framing.aspect.max(0.1);
        let mut frame = fit(stage.shrink(2.0), aspect);
        let here = self.here();
        match (&self.source, here) {
            (Source::Empty, _) | (Source::Clip { .. } | Source::Sequence { .. }, None) => {
                ui.painter()
                    .rect_filled(frame, egui::CornerRadius::ZERO, background);
            }
            (Source::Audio { path, .. }, _) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                ui.painter().text(
                    stage.center(),
                    egui::Align2::CENTER_CENTER,
                    format!("Music: {name}"),
                    egui::FontId::proportional(16.0),
                    egui::Color32::from_gray(170),
                );
            }
            (Source::Clip { .. } | Source::Sequence { .. }, Some(here)) => {
                let cx = self.segs.get(here.seg).map_or(0.5, |s| s.cx);
                let picture = self.picture(ui, thumbs, &here);
                match picture {
                    Some((tex, src)) => {
                        frame = paint_picture(ui, stage, &tex, src, framing, cx);
                    }
                    None => {
                        ui.painter()
                            .rect_filled(frame, egui::CornerRadius::ZERO, background);
                    }
                }
            }
        }
        if response.clicked() {
            self.toggle();
        }
        Shown { response, frame }
    }

    /// The texture to show for `here` and its pixel size: the decoded frame
    /// when there is one, else a thumbnail, else the last frame shown.
    fn picture(
        &mut self,
        ui: &egui::Ui,
        thumbs: &mut Thumbs,
        here: &Here,
    ) -> Option<(egui::TextureId, egui::Vec2)> {
        let ctx = ui.ctx().clone();
        let decoding = !self.scrubbing && (!self.playing || (self.rate > 0.0 && self.rate <= 2.0));
        if decoding && let Some(frame) = self.decoded(&ctx, here) {
            let fresh = self
                .uploaded
                .as_ref()
                .is_none_or(|u| u.path != here.path || (u.t - frame.t).abs() > 1e-6);
            if fresh {
                let image = egui::ImageData::from(frame.image);
                let options = egui::TextureOptions::LINEAR;
                match &mut self.texture {
                    Some(tex) => tex.set(image, options),
                    None => self.texture = Some(ctx.load_texture("ave-viewer", image, options)),
                }
                self.uploaded = Some(Uploaded {
                    path: here.path.clone(),
                    t: frame.t,
                });
            }
        }
        // The texture is good enough when it shows this file near here.
        let near = self.uploaded.as_ref().is_some_and(|u| {
            u.path == here.path && (u.t - here.file_t).abs() < 0.5 / self.fps().max(1.0) + 0.02
        });
        self.frame_ok = near;
        if near && let Some(tex) = &self.texture {
            return Some((tex.id(), tex.size_vec2()));
        }
        let snapped = (here.file_t * 12.0).floor() / 12.0;
        let width = (ui.ctx().pixels_per_point() * 640.0).clamp(320.0, 640.0) as u32 / 64 * 64;
        if let Some(tex) = thumbs.get(&ctx, &here.path, snapped, width) {
            return Some((tex.id(), tex.size_vec2()));
        }
        let same_file = self.uploaded.as_ref().is_some_and(|u| u.path == here.path);
        self.texture
            .as_ref()
            .filter(|_| same_file)
            .map(|tex| (tex.id(), tex.size_vec2()))
    }

    /// Runs the decoders for `here` and returns the frame for it, if ready.
    fn decoded(&mut self, ctx: &egui::Context, here: &Here) -> Option<ring::Frame> {
        let seg = self.segs.get(here.seg)?.clone();
        let fd = 1.0 / self.fps();
        let settled = self.playing || self.moved_at.elapsed() >= SETTLE;
        let width = self.preview_width;
        self.streams.retain(|s| s.width == width);
        if self.jumped && settled {
            // A jump: whatever is decoding is in the wrong place.
            self.jumped = false;
            self.streams.clear();
        }
        let mut frame = None;
        let mut start = true;
        if let Some(k) = self
            .streams
            .iter()
            .position(|s| s.seg == here.seg && s.path == seg.path)
        {
            let stream = &self.streams[k];
            let (choice, f) = stream.ring().take(here.file_t, fd);
            match choice {
                ring::Pick::Show(_) => {
                    frame = f;
                    start = false;
                }
                // An empty, ended ring produced nothing (a decode error):
                // retry only once the playhead has moved off its start.
                ring::Pick::Wait => {
                    start = stream.ring().ended() && (stream.start - here.file_t).abs() > fd;
                    if start {
                        self.streams.remove(k);
                    }
                }
                ring::Pick::Restart => {
                    self.streams.remove(k);
                }
            }
        }
        if start && settled {
            // At the very end, decode the last frame rather than nothing.
            let from = here.file_t.min(seg.t_out() - fd).max(seg.t_in);
            self.streams.push(VideoStream::open(
                ctx,
                &self.probes,
                &seg.path,
                here.seg,
                from,
                seg.t_out(),
                width,
            ));
        }
        // Pre-roll the shots coming up so cuts land on time.
        let within = here.file_t - seg.t_in;
        let ahead = if self.playing && self.rate > 0.0 {
            let wrap = self.looping && !self.range_play;
            clock::upcoming(&self.segs, here.seg, within, PREROLL, wrap, MAX_AHEAD)
        } else {
            Vec::new()
        };
        for &n in &ahead {
            if !self.streams.iter().any(|s| s.seg == n)
                && let Some(ns) = self.segs.get(n)
            {
                self.streams.push(VideoStream::open(
                    ctx,
                    &self.probes,
                    &ns.path,
                    n,
                    ns.t_in,
                    ns.t_out(),
                    width,
                ));
            }
        }
        let keep = here.seg;
        self.streams
            .retain(|s| s.seg == keep || ahead.contains(&s.seg));
        frame
    }

    /// Handles the shared transport keys: Space play/pause, J/K/L shuttle
    /// (repeat J or L to go faster), ←/→ one frame, Shift+←/→ one second,
    /// I / O marks, Alt+X clear marks, `/` play the marked range, Ctrl+L
    /// loop. Call only when the page may read keys (`PageCtx::keys_free`).
    pub(crate) fn handle_keys(&mut self, ui: &egui::Ui) {
        if matches!(self.source, Source::Empty) {
            return;
        }
        let keys = ui.input(|i| {
            let m = i.modifiers;
            let plain = !m.command && !m.alt;
            Keys {
                space: plain && i.key_pressed(egui::Key::Space),
                j: plain && !m.shift && i.key_pressed(egui::Key::J),
                k: plain && !m.shift && i.key_pressed(egui::Key::K),
                l: plain && !m.shift && i.key_pressed(egui::Key::L),
                left: plain && i.key_pressed(egui::Key::ArrowLeft),
                right: plain && i.key_pressed(egui::Key::ArrowRight),
                shift: m.shift,
                mark_in: plain && !m.shift && i.key_pressed(egui::Key::I),
                mark_out: plain && !m.shift && i.key_pressed(egui::Key::O),
                clear: m.alt && !m.command && i.key_pressed(egui::Key::X),
                range: plain && i.key_pressed(egui::Key::Slash),
                looping: m.command && !m.shift && i.key_pressed(egui::Key::L),
            }
        });
        if keys.space {
            self.set_rate(1.0);
            self.toggle();
        }
        if keys.k {
            self.pause();
            self.set_rate(1.0);
        }
        for (pressed, dir) in [(keys.l, 1.0), (keys.j, -1.0)] {
            if pressed {
                let next = clock::shuttle(self.rate, self.playing, dir);
                self.set_rate(next);
                self.range_play = false;
                self.play();
            }
        }
        if keys.left || keys.right {
            let dir = if keys.left { -1.0 } else { 1.0 };
            if keys.shift {
                self.pause();
                self.seek(self.pos + dir);
            } else {
                self.step_frames(dir);
            }
        }
        if keys.mark_in {
            self.mark_in();
        }
        if keys.mark_out {
            self.mark_out();
        }
        if keys.clear {
            self.clear_marks();
        }
        if keys.range {
            self.play_marks();
        }
        if keys.looping {
            self.looping = !self.looping;
        }
    }

    /// Draws the transport row in the current egui style.
    pub(crate) fn transport(&mut self, ui: &mut egui::Ui) {
        let style = transport::Style::from_visuals(ui.visuals());
        transport::bar(self, ui, &style);
    }
}

/// The keys [`Player::handle_keys`] reads, in one input pass.
struct Keys {
    space: bool,
    j: bool,
    k: bool,
    l: bool,
    left: bool,
    right: bool,
    shift: bool,
    mark_in: bool,
    mark_out: bool,
    clear: bool,
    range: bool,
    looping: bool,
}

/// The segments a source plays.
fn segments(source: &Source) -> Vec<Seg> {
    match source {
        Source::Empty | Source::Audio { .. } => Vec::new(),
        Source::Clip { path, t0, t1, cx } => vec![Seg {
            path: path.clone(),
            t_in: *t0,
            dur: t1 - t0,
            cx: *cx,
        }],
        Source::Sequence { shots, .. } => shots
            .iter()
            .map(|s| Seg {
                path: s.path.clone(),
                t_in: s.t_in,
                dur: s.dur.max(0.0),
                cx: s.cx,
            })
            .collect(),
    }
}

/// The largest rect of `aspect` (w/h) centred in `outer`.
pub(crate) fn fit(outer: egui::Rect, aspect: f32) -> egui::Rect {
    let size = outer.size();
    let fit = if size.x / size.y.max(1.0) > aspect {
        egui::vec2(size.y * aspect, size.y)
    } else {
        egui::vec2(size.x, size.x / aspect)
    };
    egui::Rect::from_center_size(outer.center(), fit)
}

/// Paints `tex` (of pixel size `src`) into `stage` for `framing`, cropped
/// around `cx`; returns the output frame's rect.
fn paint_picture(
    ui: &egui::Ui,
    stage: egui::Rect,
    tex: &egui::TextureId,
    src: egui::Vec2,
    framing: Framing,
    cx: f32,
) -> egui::Rect {
    let painter = ui.painter();
    let inner = stage.shrink(2.0);
    let src_aspect = if src.y > 0.0 {
        src.x / src.y
    } else {
        16.0 / 9.0
    };
    if !framing.ghost {
        let frame = fit(inner, framing.aspect.max(0.1));
        let uv = cover_uv(src, frame.size(), cx);
        painter.image(*tex, frame, uv, egui::Color32::WHITE);
        return frame;
    }
    let whole = fit(inner, src_aspect);
    let full_uv = egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    painter.image(*tex, whole, full_uv, egui::Color32::WHITE);
    let probe = fit(whole, framing.aspect.max(0.1));
    let uv = cover_uv(src, probe.size(), cx);
    let frame = egui::Rect::from_min_max(
        whole.min + uv.min.to_vec2() * whole.size(),
        whole.min + uv.max.to_vec2() * whole.size(),
    );
    let shade = egui::Color32::from_black_alpha(170);
    for r in [
        egui::Rect::from_min_max(whole.min, egui::pos2(frame.min.x, whole.max.y)),
        egui::Rect::from_min_max(egui::pos2(frame.max.x, whole.min.y), whole.max),
        egui::Rect::from_min_max(
            egui::pos2(frame.min.x, whole.min.y),
            egui::pos2(frame.max.x, frame.min.y),
        ),
        egui::Rect::from_min_max(
            egui::pos2(frame.min.x, frame.max.y),
            egui::pos2(frame.max.x, whole.max.y),
        ),
    ] {
        if r.width() > 0.5 && r.height() > 0.5 {
            painter.rect_filled(r, egui::CornerRadius::ZERO, shade);
        }
    }
    painter.rect_stroke(
        frame,
        egui::CornerRadius::ZERO,
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(140)),
        egui::StrokeKind::Outside,
    );
    frame
}

#[cfg(test)]
mod tests {
    use super::{Player, fit};

    fn shot(dur: f64) -> super::SeqShot {
        super::SeqShot {
            path: "a.mov".into(),
            t_in: 0.0,
            dur,
            cx: 0.5,
        }
    }

    #[test]
    fn a_sequence_finds_the_shot_under_the_playhead() -> anyhow::Result<()> {
        let mut p = Player::default();
        p.open_sequence(vec![shot(1.0), shot(2.0), shot(1.0)], None);
        assert!((p.duration() - 4.0).abs() < 1e-9, "sum of shots");
        p.seek(1.5);
        assert_eq!(p.sequence_shot().map(|(i, _)| i), Some(1), "second shot");
        assert_eq!(p.seg_starts(), vec![1.0, 3.0], "shot boundaries");
        assert_eq!(p.here().map(|h| h.seg), Some(1), "here is the second");
        p.open_audio(std::path::Path::new("m.wav"), 10.0, 20.0);
        assert!((p.duration() - 10.0).abs() < 1e-9, "audio range");
        assert_eq!(p.here(), None, "no picture for audio");
        Ok(())
    }

    #[test]
    fn seeking_is_clamped_to_the_range() -> anyhow::Result<()> {
        let mut p = Player::default();
        p.open_clip(std::path::Path::new("a.mov"), 10.0, 12.0, 0.5);
        p.seek(5.0);
        assert!(
            (p.position() - 2.0).abs() < 1e-9,
            "clamped to the 2 s range"
        );
        p.seek(-1.0);
        assert!(p.position().abs() < 1e-9, "never before the start");
        let here = p.here().map(|h| h.file_t).unwrap_or(-1.0);
        assert!((here - 10.0).abs() < 1e-9, "file time is t0 + pos: {here}");
        Ok(())
    }

    #[test]
    fn marks_order_themselves_and_reset_with_the_source() -> anyhow::Result<()> {
        let mut p = Player::default();
        p.open_clip(std::path::Path::new("a.mov"), 0.0, 10.0, 0.5);
        p.seek(4.0);
        p.mark_in();
        p.seek(6.0);
        p.mark_out();
        assert_eq!(p.marked(), Some((4.0, 6.0)), "in then out");
        p.seek(7.0);
        p.mark_in();
        assert_eq!(p.marks(), (Some(7.0), None), "an in past the out drops it");
        p.play_marks();
        assert_eq!(p.marked(), Some((7.0, 10.0)), "plays to the end from I");
        assert!(p.playing(), "and plays");
        p.open_clip(std::path::Path::new("b.mov"), 0.0, 10.0, 0.5);
        assert_eq!(p.marks(), (None, None), "a new source has no marks");
        Ok(())
    }

    #[test]
    fn frame_steps_land_on_frames() -> anyhow::Result<()> {
        let mut p = Player::default();
        p.open_clip(std::path::Path::new("a.mov"), 0.0, 10.0, 0.5);
        let fd = 1.0 / p.fps();
        for _ in 0..3 {
            p.step_frames(1.0);
        }
        let frame = (p.position() / fd).floor();
        assert!((frame - 3.0).abs() < 1e-9, "three frames in: {frame}");
        p.step_frames(-1.0);
        let frame = (p.position() / fd).floor();
        assert!((frame - 2.0).abs() < 1e-9, "one back: {frame}");
        Ok(())
    }

    #[test]
    fn fitting_keeps_the_aspect() -> anyhow::Result<()> {
        let outer = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0));
        let r = fit(outer, 9.0 / 16.0);
        assert!((r.height() - 900.0).abs() < 1e-3, "full height");
        assert!((r.width() - 506.25).abs() < 1e-3, "narrow width");
        assert!((r.center().x - 800.0).abs() < 1e-3, "centred");
        Ok(())
    }
}

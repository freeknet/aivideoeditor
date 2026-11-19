// ============================================================================
// Module:       gui::thumbs
// Description:  Thumbnails: frames decoded by ffmpeg on worker threads, cached
//               on disk and as GPU textures, requested by any page.
//
// Dependencies: egui (textures), crossbeam-channel; crate::media
// ============================================================================

//! The thumbnail service.
//!
//! Any page asks [`Thumbs::get`] for "this clip at this time, this wide" and
//! gets a texture back once it exists. The first ask queues the decode on a
//! small worker pool (one ffmpeg seek per frame) and returns `None`; the
//! frame lands in a disk cache (`<work>/thumbs/*.rgb`) so the next session
//! is instant, then becomes a texture and the UI repaints.
//!
//! Requests are served newest first, so the cards the user is looking at
//! now load before the ones they scrolled past.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crossbeam_channel::{Receiver, Sender, unbounded};

/// What identifies a thumbnail.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    /// Source file.
    pub(crate) path: PathBuf,
    /// Time in milliseconds.
    pub(crate) ms: u64,
    /// Width in pixels.
    pub(crate) width: u32,
}

impl Key {
    /// A key for `path` at `t` seconds, `width` pixels wide (rounded to
    /// 40 ms so near-identical asks share one decode).
    pub(crate) fn new(path: &Path, t: f64, width: u32) -> Self {
        let ms = ((t.max(0.0) * 1000.0 / 40.0).round() as u64) * 40;
        Self {
            path: path.to_path_buf(),
            ms,
            width,
        }
    }

    /// The disk-cache file name.
    fn file_name(&self) -> String {
        use std::hash::{Hash as _, Hasher as _};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.path.hash(&mut h);
        format!("{:016x}_{}_{}.rgb", h.finish(), self.ms, self.width)
    }
}

/// A decoded frame on its way to becoming a texture.
struct Decoded {
    key: Key,
    size: [usize; 2],
    rgb: Vec<u8>,
}

/// The work queue shared with the workers: newest request last.
type Queue = Arc<Mutex<Vec<Key>>>;

/// The thumbnail cache.
pub(crate) struct Thumbs {
    queue: Queue,
    wake: Sender<()>,
    done: Receiver<Option<Decoded>>,
    pending: HashSet<Key>,
    failed: HashSet<Key>,
    textures: HashMap<Key, egui::TextureHandle>,
    cache_dir: Arc<Mutex<Option<PathBuf>>>,
    started: bool,
    wake_rx: Receiver<()>,
    done_tx: Sender<Option<Decoded>>,
}

impl Default for Thumbs {
    fn default() -> Self {
        let (wake, wake_rx) = unbounded();
        let (done_tx, done) = unbounded();
        Self {
            queue: Arc::new(Mutex::new(Vec::new())),
            wake,
            done,
            pending: HashSet::new(),
            failed: HashSet::new(),
            textures: HashMap::new(),
            cache_dir: Arc::new(Mutex::new(None)),
            started: false,
            wake_rx,
            done_tx,
        }
    }
}

/// Worker threads decoding thumbnails.
const WORKERS: usize = 4;

/// Textures kept before the oldest are dropped.
const MAX_TEXTURES: usize = 1500;

impl Thumbs {
    /// Points the disk cache at `<work>/thumbs` (or turns it off).
    pub(crate) fn set_work(&mut self, work: Option<&Path>) {
        let dir = work.map(|w| w.join("thumbs"));
        if let Some(d) = &dir {
            let _ = std::fs::create_dir_all(d);
        }
        if let Ok(mut slot) = self.cache_dir.lock() {
            *slot = dir;
        }
    }

    /// Starts the workers on first use.
    fn start(&mut self, ctx: &egui::Context) {
        if self.started {
            return;
        }
        self.started = true;
        for _ in 0..WORKERS {
            let queue = Arc::clone(&self.queue);
            let wake = self.wake_rx.clone();
            let done = self.done_tx.clone();
            let cache = Arc::clone(&self.cache_dir);
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                while wake.recv().is_ok() {
                    let Some(key) = queue.lock().ok().and_then(|mut q| q.pop()) else {
                        continue;
                    };
                    let dir = cache.lock().ok().and_then(|d| d.clone());
                    let decoded = load_or_decode(&key, dir.as_deref());
                    let _ = done.send(
                        decoded
                            .map(|(size, rgb)| Decoded {
                                key: key.clone(),
                                size,
                                rgb,
                            })
                            .or_else(|| {
                                // Report the failure so the key is not asked for again.
                                Some(Decoded {
                                    key,
                                    size: [0, 0],
                                    rgb: Vec::new(),
                                })
                            }),
                    );
                    ctx.request_repaint();
                }
            });
        }
    }

    /// Moves finished decodes into textures. Call once per frame.
    pub(crate) fn pump(&mut self, ctx: &egui::Context) {
        while let Ok(Some(d)) = self.done.try_recv() {
            self.pending.remove(&d.key);
            if d.rgb.is_empty() {
                self.failed.insert(d.key);
                continue;
            }
            let image = egui::ColorImage::from_rgb(d.size, &d.rgb);
            let name = format!("thumb-{}-{}", d.key.ms, d.key.width);
            let handle = ctx.load_texture(name, image, egui::TextureOptions::LINEAR);
            if self.textures.len() >= MAX_TEXTURES {
                // Drop an arbitrary half; they reload from the disk cache.
                let keep: Vec<Key> = self
                    .textures
                    .keys()
                    .take(MAX_TEXTURES / 2)
                    .cloned()
                    .collect();
                self.textures.retain(|k, _| keep.contains(k));
            }
            self.textures.insert(d.key, handle);
        }
    }

    /// The texture for `path` at `t`, `width` wide, or `None` while it loads.
    pub(crate) fn get(
        &mut self,
        ctx: &egui::Context,
        path: &Path,
        t: f64,
        width: u32,
    ) -> Option<egui::TextureHandle> {
        let key = Key::new(path, t, width);
        if let Some(tex) = self.textures.get(&key) {
            return Some(tex.clone());
        }
        if self.failed.contains(&key) {
            return None;
        }
        self.start(ctx);
        if self.pending.insert(key.clone()) {
            if let Ok(mut q) = self.queue.lock() {
                q.push(key);
            }
            let _ = self.wake.send(());
        } else if let Ok(mut q) = self.queue.lock() {
            // Already queued: move it to the front (served next).
            if let Some(pos) = q.iter().position(|k| *k == key) {
                let k = q.remove(pos);
                q.push(k);
            }
        }
        None
    }

    /// Paints the thumbnail into `rect` (cover-fit, cropped to `cx` when the
    /// rect is narrower than the frame), or a placeholder while it loads.
    pub(crate) fn paint(
        &mut self,
        ui: &egui::Ui,
        rect: egui::Rect,
        path: &Path,
        t: f64,
        cx: f32,
        placeholder: egui::Color32,
    ) {
        let width = (rect.width() * ui.ctx().pixels_per_point()).clamp(64.0, 640.0) as u32;
        let width = width.div_ceil(64) * 64;
        match self.get(ui.ctx(), path, t, width) {
            Some(tex) => {
                let uv = cover_uv(tex.size_vec2(), rect.size(), cx);
                ui.painter().image(tex.id(), rect, uv, egui::Color32::WHITE);
            }
            None => {
                ui.painter()
                    .rect_filled(rect, egui::CornerRadius::same(4), placeholder);
            }
        }
    }
}

/// The UV rectangle that fills `target` from an image of `image` size
/// without distortion, centred horizontally on `cx`.
pub(crate) fn cover_uv(image: egui::Vec2, target: egui::Vec2, cx: f32) -> egui::Rect {
    if image.x <= 0.0 || image.y <= 0.0 || target.x <= 0.0 || target.y <= 0.0 {
        return egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0));
    }
    let ia = image.x / image.y;
    let ta = target.x / target.y;
    if ta < ia {
        let w = ta / ia;
        let x0 = (cx - w / 2.0).clamp(0.0, 1.0 - w);
        egui::Rect::from_min_max(egui::pos2(x0, 0.0), egui::pos2(x0 + w, 1.0))
    } else {
        let h = ia / ta;
        let y0 = (0.5 - h / 2.0).clamp(0.0, 1.0 - h);
        egui::Rect::from_min_max(egui::pos2(0.0, y0), egui::pos2(1.0, y0 + h))
    }
}

/// Reads a cached frame or decodes and caches it: `(size, rgb)`.
fn load_or_decode(key: &Key, cache: Option<&Path>) -> Option<([usize; 2], Vec<u8>)> {
    let file = cache.map(|d| d.join(key.file_name()));
    if let Some(f) = &file
        && let Ok(bytes) = std::fs::read(f)
        && bytes.len() > 8
    {
        let w = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        let h = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        if w * h * 3 == bytes.len() - 8 {
            return Some(([w, h], bytes[8..].to_vec()));
        }
    }
    let frame = crate::media::frame_rgb(&key.path, key.ms as f64 / 1000.0, key.width).ok()?;
    if let Some(f) = &file {
        let mut bytes = Vec::with_capacity(frame.rgb.len() + 8);
        bytes.extend_from_slice(&(frame.width as u32).to_le_bytes());
        bytes.extend_from_slice(&(frame.height as u32).to_le_bytes());
        bytes.extend_from_slice(&frame.rgb);
        let _ = std::fs::write(f, bytes);
    }
    Some(([frame.width, frame.height], frame.rgb))
}

#[cfg(test)]
mod tests {
    use super::{Key, cover_uv};

    #[test]
    fn a_vertical_crop_of_a_wide_frame_follows_cx() {
        let uv = cover_uv(egui::vec2(1920.0, 1080.0), egui::vec2(90.0, 160.0), 0.8);
        assert!(
            uv.width() < 0.35,
            "a 9:16 crop is about a third of the width: {uv:?}"
        );
        assert!((uv.center().x - 0.8).abs() < 0.01, "centred on cx: {uv:?}");
        let edge = cover_uv(egui::vec2(1920.0, 1080.0), egui::vec2(90.0, 160.0), 0.99);
        assert!(edge.max.x <= 1.0 + 1e-6, "never past the frame: {edge:?}");
    }

    #[test]
    fn keys_round_to_frames() {
        let a = Key::new(std::path::Path::new("a.mov"), 1.001, 160);
        let b = Key::new(std::path::Path::new("a.mov"), 1.010, 160);
        assert_eq!(a, b, "near-identical times share a decode");
    }
}

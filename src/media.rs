// ============================================================================
// Module:       media
// Description:  ffmpeg / ffprobe: locating the tools, probing a file, and
//               streaming decoded frames or audio samples into memory.
//
// Dependencies: anyhow, serde_json, std::process
// ============================================================================

//! Everything that talks to ffmpeg.
//!
//! The rest of the program never builds an ffmpeg command line of its own
//! for reading media: it asks this module for a [`Probe`], a
//! [`FrameReader`] or a buffer of mono samples. Rendering is the one
//! exception, and lives in `render`.
//!
//! The tools are found through `AVE_FFMPEG_DIR` when set, else `PATH`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};

use anyhow::{Context, Result, bail};

/// The folder ffmpeg and ffprobe are in, when not on `PATH`: `AVE_FFMPEG_DIR`,
/// else the one chosen in Settings (read from the config once, then kept
/// here; [`set_tool_dir`] changes it).
fn tool_dir() -> &'static std::sync::RwLock<Option<PathBuf>> {
    static DIR: std::sync::OnceLock<std::sync::RwLock<Option<PathBuf>>> =
        std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::var_os("AVE_FFMPEG_DIR")
            .filter(|d| !d.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                crate::config::load()
                    .ffmpeg_dir
                    .filter(|d| !d.trim().is_empty())
                    .map(PathBuf::from)
            });
        std::sync::RwLock::new(dir)
    })
}

/// Uses `dir` for ffmpeg and ffprobe from now on (`None`: from `PATH`).
pub(crate) fn set_tool_dir(dir: Option<PathBuf>) {
    if let Ok(mut d) = tool_dir().write() {
        *d = dir;
    }
    recheck_ffmpeg();
}

/// Path to an ffmpeg-suite executable (`ffmpeg`, `ffprobe`).
pub(crate) fn tool(name: &str) -> PathBuf {
    let dir = tool_dir().read().ok().and_then(|d| d.clone());
    match dir {
        Some(dir) => dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)),
        None => PathBuf::from(name),
    }
}

/// Whether ffmpeg runs, as its version line (`None`: it does not), checked
/// once and again after [`set_tool_dir`].
pub(crate) fn ffmpeg_status() -> Option<String> {
    if let Some(known) = STATUS.read().ok().and_then(|s| s.clone()) {
        return known;
    }
    let found = ffmpeg_version();
    if let Ok(mut s) = STATUS.write() {
        *s = Some(found.clone());
    }
    found
}

/// The checked ffmpeg status: `None` until checked.
static STATUS: std::sync::RwLock<Option<Option<String>>> = std::sync::RwLock::new(None);

/// Forgets the checked ffmpeg status (the folder changed).
fn recheck_ffmpeg() {
    if let Ok(mut s) = STATUS.write() {
        *s = None;
    }
}

/// The ffmpeg version line, or `None` when ffmpeg does not run.
fn ffmpeg_version() -> Option<String> {
    let out = command("ffmpeg")
        .arg("-version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(str::to_owned)
}

/// Whether ffmpeg can decode on an NVIDIA GPU here (a CUDA device opens),
/// checked once per run.
pub(crate) fn gpu_decode() -> bool {
    static CAN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CAN.get_or_init(|| {
        command("ffmpeg")
            .args(["-v", "error", "-nostdin", "-init_hw_device", "cuda=gpu"])
            .args([
                "-f",
                "lavfi",
                "-i",
                "nullsrc=s=64x64",
                "-frames:v",
                "1",
                "-f",
                "null",
                "-",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Whether ffmpeg can encode with `encoder` on this machine: `None` until
/// the check has run. The first call starts one background check of every
/// encoder (each encodes one small frame, so a build that lists
/// `h264_nvenc` on a PC without an NVIDIA card counts as without it), so a
/// screen can ask every frame.
pub(crate) fn encoder_available(encoder: crate::render::Encoder) -> Option<bool> {
    type Found = std::sync::Mutex<Option<[bool; 3]>>;
    static FOUND: std::sync::OnceLock<std::sync::Arc<Found>> = std::sync::OnceLock::new();
    let slot = FOUND.get_or_init(|| {
        let slot: std::sync::Arc<Found> = std::sync::Arc::new(std::sync::Mutex::new(None));
        let fill = std::sync::Arc::clone(&slot);
        std::thread::spawn(move || {
            let found = [
                encodes("libx264"),
                encodes("h264_amf"),
                encodes("h264_nvenc"),
            ];
            if let Ok(mut s) = fill.lock() {
                *s = Some(found);
            }
        });
        slot
    });
    let found = (*slot.lock().ok()?)?;
    let i = match encoder {
        crate::render::Encoder::X264 => 0,
        crate::render::Encoder::Amf => 1,
        crate::render::Encoder::Nvenc => 2,
    };
    found.get(i).copied()
}

/// Whether ffmpeg encodes one 256 px black frame with the video codec
/// `codec` (hardware encoders refuse anything much smaller).
fn encodes(codec: &str) -> bool {
    command("ffmpeg")
        .args(["-v", "error", "-nostdin", "-f", "lavfi", "-i"])
        .arg("color=c=black:s=256x256:d=0.1")
        .args(["-frames:v", "1", "-c:v", codec, "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A command for an ffmpeg-suite tool, already [`quiet`]: every ffmpeg and
/// ffprobe the program runs is built here, so none can open a window.
pub(crate) fn command(name: &str) -> Command {
    let mut cmd = Command::new(tool(name));
    quiet(&mut cmd);
    cmd
}

/// Keeps a console window from opening for a child process. The app has no
/// console of its own, so without this Windows gives every child process one
/// (a window per ffmpeg run during analysis). Output piped or inherited by
/// the command still arrives: the handles are passed explicitly.
pub(crate) fn quiet(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        // Nothing to configure off Windows.
        let _ = cmd;
    }
}

/// What ffprobe reports about one media file.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Probe {
    /// Duration in seconds.
    pub(crate) duration: f64,
    /// Video width in pixels (0 for audio-only files).
    pub(crate) width: u32,
    /// Video height in pixels.
    pub(crate) height: u32,
    /// Frame rate as a numerator/denominator pair, e.g. `[24000, 1001]`.
    pub(crate) fps: [u32; 2],
    /// Video codec name.
    pub(crate) vcodec: String,
    /// Whether the file has an audio stream.
    pub(crate) has_audio: bool,
    /// When recording started (container tags, else the file time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recorded_at: Option<RecordedAt>,
    /// Camera make and model, from the container tags, when they say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) camera: Option<String>,
}

impl Probe {
    /// Frame rate as a float.
    pub(crate) fn fps_f64(&self) -> f64 {
        if self.fps[1] == 0 {
            0.0
        } else {
            f64::from(self.fps[0]) / f64::from(self.fps[1])
        }
    }
}

/// Runs ffprobe on `path`.
pub(crate) fn probe(path: &Path) -> Result<Probe> {
    let output = command("ffprobe")
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .with_context(|| format!("running ffprobe on {}", path.display()))?;
    if !output.status.success() {
        bail!(
            "ffprobe failed on {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let mtime = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok());
    let streams = json["streams"].as_array().cloned().unwrap_or_default();
    let video = streams.iter().find(|s| s["codec_type"] == "video");
    let duration = json["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(0.0);
    let fps = video
        .and_then(|v| v["r_frame_rate"].as_str())
        .and_then(parse_ratio)
        .unwrap_or([0, 1]);
    let dim = |key: &str| {
        video
            .and_then(|v| v[key].as_u64())
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0)
    };
    Ok(Probe {
        duration,
        width: dim("width"),
        height: dim("height"),
        fps,
        vcodec: video
            .and_then(|v| v["codec_name"].as_str())
            .unwrap_or("")
            .to_owned(),
        has_audio: streams.iter().any(|s| s["codec_type"] == "audio"),
        recorded_at: recorded_at(&json, mtime, duration),
        camera: camera(&json),
    })
}

/// Parses `"24000/1001"`.
fn parse_ratio(text: &str) -> Option<[u32; 2]> {
    let (n, d) = text.split_once('/')?;
    Some([n.trim().parse().ok()?, d.trim().parse().ok()?])
}

/// When a clip was recorded, and how that was found out.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RecordedAt {
    /// Start of recording in UTC, RFC 3339 (`2026-11-12T03:51:11Z`).
    pub(crate) utc: String,
    /// The camera's wall clock with its UTC offset
    /// (`2026-11-11T19:51:11-08:00`), when the offset is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) local: Option<String>,
    /// Where the time came from: `tag:<key>` (the container said so, with
    /// an offset or confirmed as true UTC by the file time),
    /// `tag:<key>+mtime` (a camera clock written as UTC, its offset read
    /// off the file time) or `mtime` (no usable tag: the file's
    /// modification time less the duration).
    pub(crate) source: String,
}

impl RecordedAt {
    /// Start of recording, Unix seconds.
    pub(crate) fn unix(&self) -> Option<i64> {
        parse_time(&self.utc).map(|(t, _)| t)
    }

    /// The camera clock's offset from UTC in minutes, when known.
    pub(crate) fn offset(&self) -> Option<i32> {
        self.local
            .as_deref()
            .and_then(parse_time)
            .and_then(|(_, off)| off)
    }

    /// `19:51:11` on the camera's clock, or `03:51:11Z` when only UTC is
    /// known.
    pub(crate) fn clock(&self) -> String {
        match &self.local {
            Some(l) => l.get(11..19).unwrap_or(l).to_owned(),
            None => format!("{}Z", self.utc.get(11..19).unwrap_or(&self.utc)),
        }
    }
}

/// How a time string named its zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Zone {
    /// An explicit offset from UTC, minutes.
    Offset(i32),
    /// `Z`: claimed UTC, which many cameras write for their local clock.
    Zulu,
    /// No zone at all.
    Bare,
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day count from 1970-01-01 (inverse of
/// [`days_from_civil`]).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parses `2026-11-11T19:51:11.000000Z`, `2026-11-11T19:51:11-0800`,
/// `2026-11-11 19:51:11+01:00` or a bare `2026:11:11 19:51:11`: the wall
/// clock as if it were UTC (Unix seconds) and how the zone was given.
fn parse_wall(text: &str) -> Option<(i64, Zone)> {
    let t = text.trim();
    let num = |a: usize, b: usize| t.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, s) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let wall = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;
    let rest = t
        .get(19..)
        .unwrap_or("")
        .trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let zone = if rest.is_empty() {
        Zone::Bare
    } else if rest.eq_ignore_ascii_case("z") {
        Zone::Zulu
    } else {
        let sign = match rest.chars().next()? {
            '+' => 1,
            '-' => -1,
            _ => return None,
        };
        let digits: String = rest.chars().skip(1).filter(char::is_ascii_digit).collect();
        let oh = digits.get(0..2)?.parse::<i32>().ok()?;
        let om = digits
            .get(2..4)
            .and_then(|m| m.parse::<i32>().ok())
            .unwrap_or(0);
        Zone::Offset(sign * (oh * 60 + om))
    };
    Some((wall, zone))
}

/// Parses an RFC 3339 time to Unix seconds, with its offset in minutes
/// when it gave one (`Z` is 0).
fn parse_time(text: &str) -> Option<(i64, Option<i32>)> {
    let (wall, zone) = parse_wall(text)?;
    Some(match zone {
        Zone::Offset(off) => (wall - i64::from(off) * 60, Some(off)),
        Zone::Zulu => (wall, Some(0)),
        Zone::Bare => (wall, None),
    })
}

/// Formats Unix seconds as RFC 3339: `Z` with no offset, else the wall
/// clock at `offset` minutes.
pub(crate) fn format_time(unix: i64, offset: Option<i32>) -> String {
    let off = offset.unwrap_or(0);
    let wall = unix + i64::from(off) * 60;
    let (y, m, d) = civil_from_days(wall.div_euclid(86_400));
    let secs = wall.rem_euclid(86_400);
    let (h, mi, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let zone = match offset {
        None => "Z".to_owned(),
        Some(o) => format!(
            "{}{:02}:{:02}",
            if o < 0 { '-' } else { '+' },
            o.abs() / 60,
            o.abs() % 60
        ),
    };
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}{zone}")
}

/// The tags ffprobe found, format first, then each stream's: key
/// (lower-cased) and value.
fn tags(json: &serde_json::Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let streams = json["streams"].as_array().cloned().unwrap_or_default();
    for source in std::iter::once(&json["format"]).chain(streams.iter()) {
        if let Some(map) = source["tags"].as_object() {
            for (k, v) in map {
                if let Some(v) = v.as_str() {
                    out.push((k.to_ascii_lowercase(), v.trim().to_owned()));
                }
            }
        }
    }
    out
}

/// When recording started, from the container tags checked against the
/// file's modification time `mtime` (Unix seconds); the file time alone
/// when no tag parses.
///
/// A tag with an offset (`com.apple.quicktime.creationdate`) is taken as
/// it is. A `creation_time` in `Z` is often the camera's local clock
/// mislabelled (Canon EOS, many DJI drones): when the file time sits a
/// whole number of quarter hours (at most 14 h) from the tag plus the
/// duration, that difference is the camera's offset. Otherwise the tag is
/// taken as true UTC.
pub(crate) fn recorded_at(
    json: &serde_json::Value,
    mtime: Option<i64>,
    duration: f64,
) -> Option<RecordedAt> {
    // How far the file time may sit from the end of recording and still
    // confirm a tag: cameras close the file a few seconds after the last
    // frame, and FAT timestamps have 2 s resolution.
    const SLACK: i64 = 120;
    const KEYS: [&str; 3] = ["com.apple.quicktime.creationdate", "creation_time", "date"];
    let tags = tags(json);
    let dur = duration.max(0.0).round() as i64;
    let found = KEYS.iter().find_map(|key| {
        tags.iter()
            .filter(|(k, _)| k == key)
            .find_map(|(_, v)| parse_wall(v).map(|w| (*key, w)))
    });
    let Some((key, (wall, zone))) = found else {
        let end = mtime?;
        return Some(RecordedAt {
            utc: format_time(end - dur, None),
            local: None,
            source: "mtime".into(),
        });
    };
    let utc_only = |unix: i64, source: String| RecordedAt {
        utc: format_time(unix, None),
        local: None,
        source,
    };
    match zone {
        Zone::Offset(off) => {
            let unix = wall - i64::from(off) * 60;
            return Some(RecordedAt {
                utc: format_time(unix, None),
                local: Some(format_time(unix, Some(off))),
                source: format!("tag:{key}"),
            });
        }
        Zone::Zulu | Zone::Bare => {}
    }
    let Some(end) = mtime else {
        return Some(utc_only(wall, format!("tag:{key}")));
    };
    let drift = end - (wall + dur);
    if drift.abs() <= SLACK {
        return Some(utc_only(wall, format!("tag:{key}")));
    }
    let quarter = (drift as f64 / 900.0).round() as i64 * 900;
    if quarter.abs() <= 14 * 3600 && (drift - quarter).abs() <= SLACK {
        // The tag is the wall clock; UTC is `quarter` seconds later.
        let unix = wall + quarter;
        let off = i32::try_from(-quarter / 60).ok()?;
        return Some(RecordedAt {
            utc: format_time(unix, None),
            local: Some(format_time(unix, Some(off))),
            source: format!("tag:{key}+mtime"),
        });
    }
    Some(utc_only(wall, format!("tag:{key}")))
}

/// Camera make and model from the container tags: QuickTime / Android
/// make and model keys, else a maker's fingerprint (a DJI encoder or
/// handler, Canon's `CAEP` brand, Sony's `XAVC`, a GoPro handler).
pub(crate) fn camera(json: &serde_json::Value) -> Option<String> {
    let tags = tags(json);
    let get = |keys: &[&str]| {
        keys.iter().find_map(|key| {
            tags.iter()
                .find(|(k, v)| k == key && !v.is_empty())
                .map(|(_, v)| v.clone())
        })
    };
    let make = get(&[
        "com.apple.quicktime.make",
        "com.android.manufacturer",
        "make",
    ]);
    let model = get(&["com.apple.quicktime.model", "com.android.model", "model"]);
    match (make, model) {
        (Some(make), Some(model)) => {
            return Some(if model.to_lowercase().starts_with(&make.to_lowercase()) {
                model
            } else {
                format!("{make} {model}")
            });
        }
        (Some(one), None) | (None, Some(one)) => return Some(one),
        (None, None) => {}
    }
    let fingerprint = |needle: &str| {
        tags.iter()
            .filter(|(k, _)| {
                matches!(
                    k.as_str(),
                    "encoder" | "handler_name" | "compatible_brands" | "major_brand"
                )
            })
            .find(|(_, v)| v.contains(needle))
            .map(|(_, v)| v.clone())
    };
    if let Some(v) = fingerprint("DJI") {
        // `DJIMavic3` names the model; `DJI.AVC` only the maker.
        return Some(if v.starts_with("DJI") && !v.contains('.') {
            v
        } else {
            "DJI".to_owned()
        });
    }
    if fingerprint("CAEP").is_some() {
        return Some("Canon EOS".to_owned());
    }
    if fingerprint("XAVC").is_some() {
        return Some("Sony".to_owned());
    }
    if fingerprint("GoPro").is_some() {
        return Some("GoPro".to_owned());
    }
    None
}

/// A decoded RGB frame.
pub(crate) struct Frame {
    /// Width in pixels.
    pub(crate) width: usize,
    /// Height in pixels.
    pub(crate) height: usize,
    /// Packed RGB24, row-major.
    pub(crate) rgb: Vec<u8>,
}

/// Streams frames from ffmpeg at a fixed sample rate and size.
pub(crate) struct FrameReader {
    child: Child,
    stdout: ChildStdout,
    width: usize,
    height: usize,
}

impl FrameReader {
    /// Starts decoding `path` at `fps` frames per second, scaled to
    /// `width` pixels wide (height follows the aspect ratio, kept even).
    pub(crate) fn open(path: &Path, probe: &Probe, fps: f64, width: usize) -> Result<Self> {
        let height = if probe.width == 0 {
            width * 9 / 16
        } else {
            let h = width as f64 * f64::from(probe.height) / f64::from(probe.width);
            ((h / 2.0).round() as usize).max(1) * 2
        };
        let filter = format!("fps={fps},scale={width}:{height}:flags=area,format=rgb24");
        let mut cmd = command("ffmpeg");
        cmd.args(["-v", "error", "-nostdin", "-threads", "4"]);
        // Decoded on the graphics card when there is one: 4K HEVC reads six
        // times faster (a codec it cannot decode falls back on its own).
        if gpu_decode() {
            cmd.args(["-hwaccel", "cuda"]);
        }
        let mut child = cmd
            .arg("-i")
            .arg(path)
            .args(["-an", "-sn", "-vf", &filter, "-f", "rawvideo", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("starting ffmpeg on {}", path.display()))?;
        let stdout = child
            .stdout
            .take()
            .context("ffmpeg stdout was not captured")?;
        Ok(Self {
            child,
            stdout,
            width,
            height,
        })
    }

    /// The next frame, or `None` at the end of the stream.
    pub(crate) fn next_frame(&mut self) -> Result<Option<Frame>> {
        let mut rgb = vec![0_u8; self.width * self.height * 3];
        let mut filled = 0;
        while filled < rgb.len() {
            let n = self.stdout.read(&mut rgb[filled..])?;
            if n == 0 {
                return Ok(None);
            }
            filled += n;
        }
        Ok(Some(Frame {
            width: self.width,
            height: self.height,
            rgb,
        }))
    }
}

impl Drop for FrameReader {
    fn drop(&mut self) {
        // The stream may be abandoned early; ffmpeg must not outlive it.
        let _ = self.child.kill();
        // The child may have exited already, and it is being abandoned either way.
        let _ = self.child.wait();
    }
}

/// Decodes the audio of `path` to mono f32 samples at `rate` Hz.
pub(crate) fn audio_mono(path: &Path, rate: u32) -> Result<Vec<f32>> {
    let output = command("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args([
            "-vn",
            "-ac",
            "1",
            "-ar",
            &rate.to_string(),
            "-f",
            "f32le",
            "-",
        ])
        .output()
        .with_context(|| format!("decoding audio of {}", path.display()))?;
    if !output.status.success() {
        bail!(
            "ffmpeg could not decode audio from {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

/// Decodes one frame of `path` at `t` seconds as packed RGB24, scaled to
/// `width` pixels wide (height follows the aspect, kept even). Used for
/// thumbnails: seeking is input-side, so it costs one GOP, not the clip.
pub(crate) fn frame_rgb(path: &Path, t: f64, width: u32) -> Result<Frame> {
    let output = command("ffmpeg")
        .args(["-v", "error", "-nostdin", "-ss", &format!("{t:.3}"), "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-an", "-sn"])
        .args([
            "-vf",
            &format!("scale={width}:-2:flags=bilinear,format=rgb24"),
        ])
        .args(["-f", "rawvideo", "-"])
        .output()
        .with_context(|| format!("decoding a frame of {}", path.display()))?;
    let width = width as usize;
    let row = width * 3;
    if !output.status.success() || row == 0 || output.stdout.len() < row {
        bail!("ffmpeg returned no frame at {t:.2}s of {}", path.display());
    }
    let height = output.stdout.len() / row;
    let mut rgb = output.stdout;
    rgb.truncate(row * height);
    Ok(Frame { width, height, rgb })
}

/// Writes one JPEG frame of `path` at `t` seconds, scaled to `width`.
pub(crate) fn still(path: &Path, t: f64, width: u32, crop: Option<&str>, out: &Path) -> Result<()> {
    let mut filter = String::new();
    if let Some(crop) = crop {
        filter.push_str(crop);
        filter.push(',');
    }
    filter.push_str(&format!("scale={width}:-2:flags=lanczos"));
    let status = command("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-ss",
            &format!("{t:.3}"),
            "-i",
        ])
        .arg(path)
        .args(["-frames:v", "1", "-vf", &filter, "-q:v", "3"])
        .arg(out)
        .status()
        .with_context(|| format!("extracting a still from {}", path.display()))?;
    if !status.success() {
        bail!(
            "ffmpeg could not extract a still at {t:.2}s from {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_ratio;

    #[test]
    fn ratios_parse() -> anyhow::Result<()> {
        assert_eq!(parse_ratio("24000/1001"), Some([24000, 1001]), "NTSC film");
        assert_eq!(parse_ratio("30/1"), Some([30, 1]), "integer rate");
        assert_eq!(parse_ratio("junk"), None, "not a ratio");
        Ok(())
    }

    /// Unix seconds of a UTC wall clock.
    fn unix(text: &str) -> anyhow::Result<i64> {
        super::parse_time(text)
            .map(|(t, _)| t)
            .ok_or_else(|| anyhow::anyhow!("{text} does not parse"))
    }

    #[test]
    fn times_parse_and_format_round_trip() -> anyhow::Result<()> {
        let t = unix("2026-11-12T03:51:11.000000Z")?;
        assert_eq!(
            super::format_time(t, None),
            "2026-11-12T03:51:11Z",
            "UTC round trip"
        );
        assert_eq!(
            super::format_time(t, Some(-480)),
            "2026-11-11T19:51:11-08:00",
            "the wall clock at -08:00 is the evening before"
        );
        assert_eq!(
            unix("2026-11-11T19:51:11-0800")?,
            t,
            "an offset without a colon"
        );
        assert_eq!(unix("2026-11-11 19:51:11-08:00")?, t, "a space for the T");
        assert_eq!(unix("1970-01-01T00:00:00Z")?, 0, "the epoch");
        assert_eq!(
            super::format_time(951_782_400, None),
            "2000-02-29T00:00:00Z",
            "a leap day"
        );
        assert!(super::parse_time("yesterday").is_none(), "junk");
        Ok(())
    }

    #[test]
    fn a_camera_clock_in_z_is_corrected_by_the_file_time() -> anyhow::Result<()> {
        // A Canon EOS MOV: creation_time is the camera's local clock
        // written as UTC; the file closed 13 s later in true UTC, 8 h on.
        let json = serde_json::json!({
            "format": {"tags": {"compatible_brands": "qt  CAEP",
                "creation_time": "2026-11-11T19:51:11.000000Z"}},
            "streams": [],
        });
        let mtime = unix("2026-11-12T03:51:24Z")?;
        let r = super::recorded_at(&json, Some(mtime), 12.2)
            .ok_or_else(|| anyhow::anyhow!("no time"))?;
        assert_eq!(r.utc, "2026-11-12T03:51:11Z", "true UTC: {r:?}");
        assert_eq!(
            r.local.as_deref(),
            Some("2026-11-11T19:51:11-08:00"),
            "the camera's clock and offset: {r:?}"
        );
        assert_eq!(r.source, "tag:creation_time+mtime", "{r:?}");
        assert_eq!(r.clock(), "19:51:11", "printed on the camera clock");
        assert_eq!(
            super::camera(&json).as_deref(),
            Some("Canon EOS"),
            "the CAEP brand"
        );
        // The same tag with the file time agreeing: it is true UTC.
        let agree = unix("2026-11-11T19:51:24Z")?;
        let r = super::recorded_at(&json, Some(agree), 12.2)
            .ok_or_else(|| anyhow::anyhow!("no time"))?;
        assert!(
            r.utc == "2026-11-11T19:51:11Z" && r.local.is_none() && r.source == "tag:creation_time",
            "{r:?}"
        );
        // A copied file (mtime days later): the tag stands as UTC.
        let r = super::recorded_at(&json, Some(mtime + 5 * 86_400), 12.2)
            .ok_or_else(|| anyhow::anyhow!("no time"))?;
        assert_eq!(r.utc, "2026-11-11T19:51:11Z", "{r:?}");
        Ok(())
    }

    #[test]
    fn an_offset_tag_wins_and_no_tag_falls_back_to_the_file_time() -> anyhow::Result<()> {
        let json = serde_json::json!({
            "format": {"tags": {
                "creation_time": "2026-11-12T03:51:11.000000Z",
                "com.apple.quicktime.creationdate": "2026-11-11T19:51:11-0800",
                "com.apple.quicktime.make": "Apple",
                "com.apple.quicktime.model": "iPhone 15 Pro"}},
        });
        let r = super::recorded_at(&json, None, 5.0).ok_or_else(|| anyhow::anyhow!("no time"))?;
        assert_eq!(
            (r.utc.as_str(), r.local.as_deref(), r.source.as_str()),
            (
                "2026-11-12T03:51:11Z",
                Some("2026-11-11T19:51:11-08:00"),
                "tag:com.apple.quicktime.creationdate"
            ),
            "{r:?}"
        );
        assert_eq!(
            super::camera(&json).as_deref(),
            Some("Apple iPhone 15 Pro"),
            "make and model"
        );
        let bare =
            serde_json::json!({"format": {}, "streams": [{"tags": {"handler_name": "DJI.AVC"}}]});
        let end = unix("2026-11-11T20:00:10Z")?;
        let r =
            super::recorded_at(&bare, Some(end), 10.0).ok_or_else(|| anyhow::anyhow!("no time"))?;
        assert_eq!(
            (r.utc.as_str(), r.source.as_str()),
            ("2026-11-11T20:00:00Z", "mtime"),
            "the file time less the duration: {r:?}"
        );
        assert_eq!(super::camera(&bare).as_deref(), Some("DJI"), "DJI handler");
        assert!(
            super::recorded_at(&bare, None, 10.0).is_none(),
            "nothing to go on"
        );
        Ok(())
    }
}

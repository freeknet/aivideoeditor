// ============================================================================
// Module:       render
// Description:  Renders a plan to an mp4 with one ffmpeg filter graph:
//               frame-exact trims on the beat grid, crop, per-shot exposure
//               match, denoise, end card, straight music trim.
//
// Dependencies: rayon; crate::plan, crate::index, crate::beats, crate::media
// ============================================================================

//! Rendering.
//!
//! Every shot is conformed to the source frame rate and trimmed by frame
//! count, with boundaries computed from the cumulative beat times so the
//! cuts never drift off the music. The music is a straight trim from the
//! start bar: no speed change, no fades, no compression, no gain.
//!
//! It runs in two stages: each shot conformed on its own into a part, then
//! the parts joined and finished, so memory stays flat with the shot count.
//! A plan with no music (an empty `music`) keeps each shot's own sound
//! instead, cut to its frames, with silence for a clip that has none.
//!
//! Exposure is matched per shot from the index (the lit region's highlight
//! level and mean luma), which keeps a dark clip next to a bright one from
//! jumping. Colour is matched on the mid-tones only (pixels above black,
//! below the highlights, never an emitter): each shot's mid-tone white
//! balance and level move toward the drop shot's, within ±15 %, so flames
//! never set the white balance and shadows are never pushed far up. The
//! plan's `[finish]` table turns this off (`match_colour = false`) and can
//! cut ahead of the beat (`cut_lead_frames = 1`). No zoom, no speed change:
//! what the camera shot is what plays.
//!
//! The encoder is x264, or NVIDIA NVENC (`h264_nvenc`, `--encoder nvenc`) or AMD
//! AMF (`h264_amf`, `--encoder amf`) on the graphics card.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::analyze::segments::median;
use crate::beats::MusicMap;
use crate::index::Index;
use crate::plan::{Aspect, Plan};

/// Video encoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Encoder {
    /// libx264 on the CPU, CRF 18.
    X264,
    /// AMD AMF on the GPU.
    Amf,
    /// NVIDIA NVENC on the GPU: several times faster than x264 slow.
    Nvenc,
}

/// Samples of `clip` between `t0` and `t1`.
fn window<'a>(index: &'a Index, clip: &str, t0: f32, t1: f32) -> Vec<&'a crate::analyze::Sample> {
    index.clip(clip).map_or_else(Vec::new, |c| {
        c.samples
            .iter()
            .filter(|s| s.t >= t0 && s.t <= t1)
            .collect()
    })
}

/// Per-shot exposure correction: (input white level, gamma).
///
/// Judged on the lit region (what is above the black), so a fire spinner
/// on black is not "brightened" until its flames clip: highlights of the
/// lit region go toward 0.85 (gain at most 1.6), and a dim lit region is
/// lifted gently by gamma (at most 1.2; lifting noise is worse than a
/// mismatch). Indexes from before the lit measures fall back to the frame's
/// p95 and mean.
fn exposure(index: &Index, clip: &str, t0: f32, t1: f32) -> (f32, f32) {
    let w = window(index, clip, t0, t1);
    if w.is_empty() {
        return (1.0, 1.0);
    }
    let lit = w.iter().any(|s| s.lit_frac > 0.0);
    let pick =
        |f: fn(&crate::analyze::Sample) -> f32| median(&w.iter().map(|s| f(s)).collect::<Vec<_>>());
    let (hi, mid, target, max_gamma) = if lit {
        (pick(|s| s.lit_p95), pick(|s| s.lit), 0.25, 1.2)
    } else {
        (pick(|s| s.p95), pick(|s| s.luma), 0.16, 1.3)
    };
    let gain = (0.85 / hi.max(0.05)).clamp(1.0, 1.6);
    let lifted = (mid * gain).max(0.01);
    let gamma = if lifted < target {
        (target.ln() / lifted.ln()).recip().clamp(1.0, max_gamma)
    } else {
        1.0
    };
    (1.0 / gain, gamma)
}

/// Median mid-tone colour (RGB 0..1) of a stretch, from samples whose
/// mid-tones cover at least 1 % of the frame.
fn mid_tones(index: &Index, clip: &str, t0: f32, t1: f32) -> Option<[f32; 3]> {
    let w: Vec<&crate::analyze::Sample> = window(index, clip, t0, t1)
        .into_iter()
        .filter(|s| s.mid_frac >= 0.01)
        .collect();
    if w.is_empty() {
        return None;
    }
    let ch =
        |f: fn(&crate::analyze::Sample) -> f32| median(&w.iter().map(|s| f(s)).collect::<Vec<_>>());
    Some([ch(|s| s.mid_r), ch(|s| s.mid_g), ch(|s| s.mid_b)])
}

/// Rec. 709 luma of an RGB triple.
fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Per-channel gains that move `shot`'s mid-tones toward `reference`'s
/// (both measured after their exposure gain): white balance clamped to
/// ±15 %, brightness to -15 %..+10 % (never pushing shadows far up).
fn colour_gains(shot: [f32; 3], reference: [f32; 3]) -> [f32; 3] {
    let (ls, lr) = (luma(shot).max(1e-3), luma(reference).max(1e-3));
    let mut wb = [1.0_f32; 3];
    for c in 0..3 {
        let want = reference[c] / lr;
        let have = (shot[c] / ls).max(1e-3);
        wb[c] = (want / have).clamp(0.85, 1.18);
    }
    // Keep white balance from changing brightness on its own.
    let after = luma([shot[0] * wb[0], shot[1] * wb[1], shot[2] * wb[2]]).max(1e-3);
    let norm = ls / after;
    let level = (lr / ls).clamp(0.85, 1.1);
    wb.map(|g| (g * norm * level).clamp(0.8, 1.25))
}

/// Every shot's sound in one shape before they are joined, so a stereo clip,
/// a mono one and the silence for a clip without any concatenate cleanly.
const AUDIO_FORMAT: &str = "aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo";

/// One shot of a render: its input arguments (a trimmed source), its video
/// chain (from the input's video to the conformed frames) and its sound.
struct ShotPart {
    /// `-ss … -t … -i <clip>`.
    input: Vec<String>,
    /// The video filters, input label and output label left off.
    video: String,
    /// With no music: the shot's own sound or, for a clip without any, a
    /// silence source. `None` with music.
    audio: Option<Sound>,
}

/// A shot's sound when the cut has no music.
enum Sound {
    /// Filters applied to the input's audio.
    Own(String),
    /// A complete silent source of the shot's length.
    Silence(String),
}

/// What a render is made of: the shots, the end card, the music.
struct Parts {
    /// Every shot, in order.
    shots: Vec<ShotPart>,
    /// The end card: input arguments and filters, when it shows.
    card: Option<(Vec<String>, String)>,
    /// The music's input arguments, when the cut has music.
    music: Option<Vec<String>>,
    /// The cut's length, seconds.
    total: f64,
}

/// Shots conformed at once in a render's first stage: each holds one
/// source decoder (about 400 MB for 4K HEVC), so three stay near 1.5 GB and
/// keep the cores busy.
const PARALLEL_SHOTS: usize = 3;

/// The finishing filters applied to the joined shots.
const FINISH: &str = "unsharp=5:5:0.4:5:5:0,vignette=angle=PI/7";

/// Works out every shot's input and filters, the end card and the music.
fn parts(plan: &Plan, index: &Index, map: &MusicMap, fps: (u32, u32)) -> Result<Parts> {
    let rate = f64::from(fps.0) / f64::from(fps.1);
    let frames = |sec: f64| (sec * rate).round() as i64;
    let timing = plan.timing(map);
    let origin = map.beat_time(plan.start_bar, 0);
    let (card_t, end_t) = plan.card_span(map);
    let (card_t, end_t) = (f64::from(card_t), f64::from(end_t));
    let (ow, oh) = plan.aspect.size();

    // Cut boundaries in output frames. With a cut lead, every cut after the
    // first (the end card's included) comes that many frames early, and
    // the incoming shot starts that much earlier in its source, so what was
    // planned for the beat still plays on the beat.
    let lead = i64::from(plan.finish.cut_lead_frames);
    let mut bounds: Vec<i64> = vec![0];
    for (s, _) in timing.iter().skip(1) {
        bounds.push(frames(f64::from(*s)) - lead);
    }
    bounds.push(frames(card_t) - if plan.end_card.is_empty() { 0 } else { lead });

    // Mid-tone colour reference: the drop shot, after its exposure gain.
    let drop_beat = (plan.drop_bar - plan.start_bar) * 4;
    let mut beat = 0;
    let mut reference = None;
    for (shot, (_, dur)) in plan.shots.iter().zip(&timing) {
        if beat == drop_beat && plan.finish.match_colour {
            let (white, _) = exposure(index, &shot.clip, shot.t_in, shot.t_in + dur);
            reference = mid_tones(index, &shot.clip, shot.t_in, shot.t_in + dur)
                .map(|m| m.map(|c| c / white));
        }
        beat += shot.beats;
    }

    let mut shots = Vec::new();
    for (i, shot) in plan.shots.iter().enumerate() {
        let clip = index
            .clip(&shot.clip)
            .with_context(|| format!("shot {}: clip {} not indexed", i + 1, shot.clip))?;
        let n = bounds[i + 1] - bounds[i];
        if n <= 0 {
            bail!("shot {} has no frames", i + 1);
        }
        let dur = n as f64 / rate;
        // With no music, the shot keeps its own sound, cut to its frames
        // (silence for a clip recorded without any).
        let audio = plan.music.is_empty().then(|| {
            if clip.probe.has_audio {
                Sound::Own(format!(
                    "atrim=end={dur:.6},asetpts=PTS-STARTPTS,{AUDIO_FORMAT}"
                ))
            } else {
                Sound::Silence(format!(
                    "anullsrc=r=48000:cl=stereo,atrim=end={dur:.6},{AUDIO_FORMAT}"
                ))
            }
        });
        let t_in = if i == 0 {
            f64::from(shot.t_in)
        } else {
            (f64::from(shot.t_in) - lead as f64 / rate).max(0.0)
        };
        let input = vec![
            "-ss".into(),
            format!("{t_in:.3}"),
            "-t".into(),
            format!("{:.3}", dur + 0.5),
            "-i".into(),
            clip.path.clone(),
        ];
        let (sw, sh) = (clip.probe.width.max(2), clip.probe.height.max(2));
        let crop = match plan.aspect {
            Aspect::Wide => String::new(),
            Aspect::Vertical | Aspect::Feed => {
                let cw =
                    ((sh as f32 * plan.aspect.crop_frac() * 16.0 / 9.0) / 2.0).round() as u32 * 2;
                let cw = cw.min(sw);
                let x = ((shot.cx * sw as f32) - cw as f32 / 2.0)
                    .round()
                    .clamp(0.0, (sw - cw) as f32) as u32;
                format!("crop={cw}:{sh}:{x}:0,")
            }
        };
        let (white, gamma) = exposure(index, &shot.clip, shot.t_in, shot.t_in + dur as f32);
        let matched = reference
            .zip(mid_tones(
                index,
                &shot.clip,
                shot.t_in,
                shot.t_in + dur as f32,
            ))
            .map(|(r, m)| colour_gains(m.map(|c| c / white), r))
            .filter(|g| g.iter().any(|v| (v - 1.0).abs() > 0.005));
        let mixer = matched.map_or_else(String::new, |g| {
            format!(
                "colorchannelmixer=rr={:.3}:gg={:.3}:bb={:.3},",
                g[0], g[1], g[2]
            )
        });
        let video = format!(
            // Scaled to the output first: denoise and grade at 1080p, not on
            // the 4K source (four times the pixels for the same picture).
            "fps={}/{},trim=end_frame={n},setpts=PTS-STARTPTS,{crop}scale={ow}:{oh}:flags=lanczos,setsar=1,\
             hqdn3d=2:1.5:4:4,colorlevels=rimax={white:.3}:gimax={white:.3}:bimax={white:.3},{mixer}eq=gamma={gamma:.3}:saturation=1.15:contrast=1.04,\
             format=yuv420p",
            fps.0, fps.1
        );
        shots.push(ShotPart {
            input,
            video,
            audio,
        });
    }

    let n_shots = plan.shots.len();
    let card_frames = frames(end_t) - bounds[n_shots];
    let card = (card_frames > 0 && !plan.end_card.is_empty()).then(|| {
        (
            vec![
                "-loop".into(),
                "1".into(),
                "-framerate".into(),
                format!("{}/{}", fps.0, fps.1),
                "-i".into(),
                plan.end_card.clone(),
            ],
            format!(
                "scale={ow}:{oh}:flags=lanczos,trim=end_frame={card_frames},setpts=PTS-STARTPTS,setsar=1,format=yuv420p"
            ),
        )
    });
    let total = frames(if card.is_some() { end_t } else { card_t }) as f64 / rate;
    let music = (!plan.music.is_empty()).then(|| {
        vec![
            "-ss".into(),
            format!("{origin:.3}"),
            "-t".into(),
            format!("{total:.3}"),
            "-i".into(),
            plan.music.clone(),
        ]
    });
    Ok(Parts {
        shots,
        card,
        music,
        total,
    })
}

/// Builds the whole render as one filter graph over every shot as an input;
/// returns (args, graph, seconds). What [`render`] runs in two stages, as
/// one graph to read and test.
#[cfg(test)]
fn build(
    plan: &Plan,
    index: &Index,
    map: &MusicMap,
    fps: (u32, u32),
) -> Result<(Vec<String>, String, f64)> {
    let p = parts(plan, index, map, fps)?;
    let mut args: Vec<String> = Vec::new();
    let mut graph = String::new();
    let mut labels = String::new();
    let mut audio = String::new();
    let mut audio_labels = String::new();
    for (i, shot) in p.shots.iter().enumerate() {
        args.extend(shot.input.iter().cloned());
        let _ = write!(graph, "[{i}:v]{}[v{i}];", shot.video);
        let _ = write!(labels, "[v{i}]");
        match &shot.audio {
            Some(Sound::Own(f)) => {
                let _ = write!(audio, "[{i}:a]{f}[a{i}];");
                let _ = write!(audio_labels, "[a{i}]");
            }
            Some(Sound::Silence(f)) => {
                let _ = write!(audio, "{f}[a{i}];");
                let _ = write!(audio_labels, "[a{i}]");
            }
            None => {}
        }
    }
    let n_shots = p.shots.len();
    let _ = write!(graph, "{labels}concat=n={n_shots}:v=1:a=0,{FINISH}[cut];");
    match &p.card {
        Some((input, chain)) => {
            args.extend(input.iter().cloned());
            let _ = write!(
                graph,
                "[{n_shots}:v]{chain}[card];[cut][card]concat=n=2:v=1:a=0[v]"
            );
        }
        None => graph.push_str("[cut]null[v]"),
    }
    let total = p.total;
    match &p.music {
        None => {
            // The shots' own sound, then silence under the end card.
            let _ = write!(
                graph,
                ";{audio}{audio_labels}concat=n={n_shots}:v=0:a=1,apad,atrim=end={total:.6}[a]"
            );
        }
        Some(input) => args.extend(input.iter().cloned()),
    }
    Ok((args, graph, total))
}

/// Runs one ffmpeg; a failure names `what`.
fn run_ffmpeg(cmd: &mut std::process::Command, what: &str) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("running ffmpeg for {what}"))?;
    if !status.success() {
        bail!("ffmpeg failed on {what}");
    }
    Ok(())
}

/// A path as a line of an ffmpeg concat list.
fn concat_line(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    format!("file '{}'\n", text.replace('\'', "'\\''"))
}

/// Renders `plan` to `out`, in two stages so memory stays flat however many
/// shots there are: each shot is conformed on its own (one source decoder
/// open at a time) into a near-lossless part, then the parts are joined
/// with the concat demuxer (one input) and finished, with the end card and
/// the music or the shots' own sound. One graph over every shot as an input
/// held a 4K HEVC decoder per shot at once: 12 GB for a 69-shot cut.
pub(crate) fn render(
    plan: &Plan,
    index: &Index,
    map: &MusicMap,
    out: &Path,
    encoder: Encoder,
    progress: &crate::commands::Progress,
) -> Result<f64> {
    let first = plan.shots.first().context("the plan has no shots")?;
    let fps = index
        .clip(&first.clip)
        .map_or((24000, 1001), |c| (c.probe.fps[0], c.probe.fps[1].max(1)));
    let p = parts(plan, index, map, fps)?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let work = out.with_extension("parts");
    std::fs::create_dir_all(&work)?;
    let rate = format!("{}/{}", fps.0, fps.1);

    // Stage 1: every shot on its own, a few at a time (each a source
    // decoder; [`PARALLEL_SHOTS`] of them stay well inside memory).
    let total_shots = p.shots.len();
    let gpu_decode = std::sync::atomic::AtomicBool::new(crate::media::gpu_decode());
    let done = std::sync::atomic::AtomicUsize::new(0);
    let began = std::time::Instant::now();
    let one = |(i, shot): (usize, &ShotPart)| -> Result<PathBuf> {
        progress.check()?;
        let part = work.join(format!("{i:04}.mkv"));
        let mut graph = format!("[0:v]{}[v]", shot.video);
        let mut maps = vec!["-map", "[v]"];
        match &shot.audio {
            Some(Sound::Own(f)) => {
                let _ = write!(graph, ";[0:a]{f}[a]");
                maps.extend(["-map", "[a]"]);
            }
            Some(Sound::Silence(f)) => {
                let _ = write!(graph, ";{f}[a]");
                maps.extend(["-map", "[a]"]);
            }
            None => {}
        }
        let conform = |gpu: bool| {
            let mut cmd = crate::media::command("ffmpeg");
            cmd.args(["-v", "error", "-nostdin", "-y"]);
            if gpu {
                cmd.args(["-hwaccel", "cuda"]);
            }
            cmd.args(&shot.input)
                .arg("-filter_complex")
                .arg(&graph)
                .args(&maps)
                .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "10"])
                .args(["-r", &rate, "-pix_fmt", "yuv420p", "-c:a", "pcm_s16le"])
                .arg(&part);
            run_ffmpeg(&mut cmd, &format!("shot {}", i + 1))
        };
        // The source decoded on the graphics card when it can (4K HEVC in
        // about two thirds of the time); a failure there retries on the
        // CPU, and the rest of the render stays on it.
        let gpu = gpu_decode.load(std::sync::atomic::Ordering::Relaxed);
        if !gpu || conform(true).is_err() {
            if gpu {
                gpu_decode.store(false, std::sync::atomic::Ordering::Relaxed);
                log::info!("decoding on the GPU failed; the render goes on decoding on the CPU");
            }
            conform(false)?;
        }
        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let per = began.elapsed().as_secs_f32() / n as f32;
        progress.report(
            &crate::commands::Report::item(
                "render",
                n,
                total_shots,
                format!("shot {}", i + 1),
                crate::commands::ItemState::Done,
            )
            .remaining(Some(per * total_shots.saturating_sub(n) as f32)),
        );
        Ok(part)
    };
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(PARALLEL_SHOTS)
        .build()?;
    let made: Vec<Result<PathBuf>> = pool.install(|| {
        use rayon::prelude::*;
        p.shots.par_iter().enumerate().map(one).collect()
    });
    let mut list = String::new();
    for part in made {
        list.push_str(&concat_line(&part?));
    }
    let list_file = work.join("parts.txt");
    std::fs::write(&list_file, &list)?;

    progress.stage("render", "joining the shots and encoding");
    // Stage 2: join, finish, card, sound.
    let mut args: Vec<String> = vec![
        "-f".into(),
        "concat".into(),
        "-safe".into(),
        "0".into(),
        "-i".into(),
        list_file.display().to_string(),
    ];
    let mut graph = format!("[0:v]{FINISH}[cut];");
    match &p.card {
        Some((input, chain)) => {
            args.extend(input.iter().cloned());
            let _ = write!(graph, "[1:v]{chain}[card];[cut][card]concat=n=2:v=1:a=0[v]");
        }
        None => graph.push_str("[cut]null[v]"),
    }
    let total = p.total;
    let audio_map = match &p.music {
        None => {
            let _ = write!(graph, ";[0:a]apad,atrim=end={total:.6}[a]");
            "[a]".to_owned()
        }
        Some(input) => {
            args.extend(input.iter().cloned());
            format!("{}:a", args.iter().filter(|a| *a == "-i").count() - 1)
        }
    };
    let script = out.with_extension("filter.txt");
    std::fs::write(&script, &graph)?;
    let codec: Vec<&str> = match encoder {
        Encoder::X264 => vec![
            "-c:v", "libx264", "-preset", "slow", "-crf", "18", "-tune", "film",
        ],
        Encoder::Amf => vec![
            "-c:v", "h264_amf", "-quality", "quality", "-rc", "cqp", "-qp_i", "18", "-qp_p", "20",
        ],
        Encoder::Nvenc => vec![
            "-c:v",
            "h264_nvenc",
            "-preset",
            "p6",
            "-tune",
            "hq",
            "-rc",
            "vbr",
            "-cq",
            "19",
            "-b:v",
            "0",
        ],
    };
    let mut cmd = crate::media::command("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-y"])
        .args(&args)
        .arg("-/filter_complex")
        .arg(&script)
        .args(["-map", "[v]", "-map", &audio_map])
        .args(&codec)
        .args(["-r", &rate, "-pix_fmt", "yuv420p"])
        .args([
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
        ])
        .args([
            "-c:a",
            "aac",
            "-b:a",
            "256k",
            "-t",
            &format!("{total:.3}"),
            "-movflags",
            "+faststart",
        ])
        .arg(out);
    run_ffmpeg(
        &mut cmd,
        &format!("{} (filter graph in {})", out.display(), script.display()),
    )?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(total)
}

/// Writes a 720p preview of `video` next to it and returns its path.
pub(crate) fn preview(video: &Path, aspect: Aspect, encoder: Encoder) -> Result<PathBuf> {
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let out = video.with_file_name(format!("{stem}-720p.mp4"));
    let scale = match aspect {
        Aspect::Wide => "scale=-2:720:flags=lanczos",
        Aspect::Vertical | Aspect::Feed => "scale=720:-2:flags=lanczos",
    };
    let status = crate::media::command("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(video)
        .args(["-vf", scale])
        // On the same encoder as the render: a GPU render previews on it too.
        .args(match encoder {
            Encoder::X264 => ["-c:v", "libx264", "-crf", "21", "-preset", "medium"],
            Encoder::Nvenc => ["-c:v", "h264_nvenc", "-cq", "23", "-preset", "p5"],
            Encoder::Amf => ["-c:v", "h264_amf", "-rc", "cqp", "-qp_i", "23"],
        })
        .args(["-c:a", "copy", "-movflags", "+faststart"])
        .arg(&out)
        .status()?;
    if !status.success() {
        bail!("ffmpeg failed making the preview of {}", video.display());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{build, colour_gains, luma};
    use crate::analyze::{ClipIndex, Sample};
    use crate::index::Index;
    use crate::media::Probe;
    use crate::plan::{Aspect, Finish, Plan, Rules, Shot};

    /// A clip whose mid-tones are `mid` and whose lit region is well exposed.
    fn clip(id: &str, mid: [f32; 3]) -> ClipIndex {
        ClipIndex {
            id: id.into(),
            path: format!("{id}.mov"),
            stamp: (0, 0),
            probe: Probe {
                duration: 60.0,
                width: 1920,
                height: 1080,
                fps: [24, 1],
                vcodec: "h264".into(),
                has_audio: false,
                recorded_at: None,
                camera: None,
            },
            samples: (0..360)
                .map(|i| Sample {
                    t: i as f32 / 6.0,
                    lit: 0.4,
                    lit_frac: 0.1,
                    lit_p95: 0.9,
                    mid_r: mid[0],
                    mid_g: mid[1],
                    mid_b: mid[2],
                    mid_frac: 0.2,
                    ..Sample::default()
                })
                .collect(),
            segments: Vec::new(),
            hist: Vec::new(),
            version: crate::analyze::ANALYSIS_VERSION,
        }
    }

    fn plan(finish: Finish) -> Plan {
        let shot = |clip: &str| Shot {
            clip: clip.into(),
            t_in: 10.0,
            beats: 4,
            cx: 0.5,
            who: String::new(),
            seg: String::new(),
            note: String::new(),
        };
        Plan {
            name: "t".into(),
            aspect: Aspect::Wide,
            music: "m.wav".into(),
            start_bar: 0,
            drop_bar: 0,
            card_bar: 2,
            end_bar: 2,
            end_card: String::new(),
            rules: Rules::default(),
            finish,
            shots: vec![shot("A"), shot("B")],
            comments: Vec::new(),
        }
    }

    fn map() -> crate::beats::MusicMap {
        // 120 BPM: a beat is 0.5 s, 12 frames at 24 fps.
        crate::beats::MusicMap {
            path: "m.wav".into(),
            duration: 60.0,
            bpm: 120.0,
            beats: (0..120).map(|i| i as f32 * 0.5).collect(),
            bars: Vec::new(),
            drops: Vec::new(),
            ..Default::default()
        }
    }

    fn index() -> Index {
        Index {
            clips: vec![clip("A", [0.3, 0.2, 0.1]), clip("B", [0.1, 0.2, 0.3])],
            ..Index::default()
        }
    }

    #[test]
    fn colour_matching_moves_a_shot_toward_the_drop_shot() -> anyhow::Result<()> {
        let (_, graph, _) = build(&plan(Finish::default()), &index(), &map(), (24, 1))?;
        let shots: Vec<&str> = graph.split(';').collect();
        assert!(
            !shots[0].contains("colorchannelmixer"),
            "the drop shot is the reference: {}",
            shots[0]
        );
        assert!(
            shots[1].contains("colorchannelmixer=rr=1.250:"),
            "the blue shot is warmed, within the clamp: {}",
            shots[1]
        );
        let (_, plain, _) = build(
            &plan(Finish {
                match_colour: false,
                ..Finish::default()
            }),
            &index(),
            &map(),
            (24, 1),
        )?;
        assert!(!plain.contains("colorchannelmixer"), "matching off");
        Ok(())
    }

    #[test]
    fn colour_gains_are_clamped_and_keep_brightness() -> anyhow::Result<()> {
        let shot = [0.2, 0.2, 0.22];
        let reference = [0.22, 0.2, 0.2];
        let g = colour_gains(shot, reference);
        let after = [shot[0] * g[0], shot[1] * g[1], shot[2] * g[2]];
        assert!(
            (luma(after) - luma(reference)).abs() < 0.005,
            "a small cast is corrected at the reference's brightness: {g:?}"
        );
        assert!(g[0] > 1.0 && g[2] < 1.0, "warmer: {g:?}");
        let darker = colour_gains(shot, [0.02, 0.02, 0.022]);
        assert!(
            darker.iter().all(|v| *v >= 0.85 * 0.99),
            "never darker than -15 %: {darker:?}"
        );
        let brighter = colour_gains(shot, [0.6, 0.6, 0.66]);
        assert!(
            brighter.iter().all(|v| *v <= 1.1 * 1.01),
            "never lifted more than +10 %: {brighter:?}"
        );
        Ok(())
    }

    #[test]
    fn a_cut_lead_moves_every_cut_a_frame_early() -> anyhow::Result<()> {
        let (args, on_beat, _) = build(&plan(Finish::default()), &index(), &map(), (24, 1))?;
        assert!(
            on_beat.contains("[0:v]fps=24/1,trim=end_frame=48,")
                && on_beat.contains("[1:v]fps=24/1,trim=end_frame=48,"),
            "two 2-second shots: {on_beat}"
        );
        assert!(
            args.contains(&"10.000".to_owned()),
            "second in-point on the plan"
        );
        let (args, early, _) = build(
            &plan(Finish {
                cut_lead_frames: 1,
                ..Finish::default()
            }),
            &index(),
            &map(),
            (24, 1),
        )?;
        assert!(
            early.contains("[0:v]fps=24/1,trim=end_frame=47,")
                && early.contains("[1:v]fps=24/1,trim=end_frame=49,"),
            "the cut comes one frame early, total unchanged: {early}"
        );
        assert!(
            args.contains(&"9.958".to_owned()),
            "the incoming shot starts a frame earlier in its source: {args:?}"
        );
        Ok(())
    }

    #[test]
    fn with_no_music_each_shot_keeps_its_own_sound() -> anyhow::Result<()> {
        let mut index = index();
        if let Some(a) = index.clips.first_mut() {
            a.probe.has_audio = true;
        }
        let mut silent = plan(Finish::default());
        silent.music = String::new();
        let (args, graph, total) =
            build(&silent, &index, &crate::beats::MusicMap::paced(), (24, 1))?;
        assert_eq!(
            args.iter().filter(|a| *a == "-i").count(),
            2,
            "the two shots and no music input: {args:?}"
        );
        assert!(
            graph.contains("[0:a]atrim=end=2.000000,"),
            "the clip with sound is cut to its 2 s shot: {graph}"
        );
        assert!(
            graph.contains("anullsrc=r=48000:cl=stereo,atrim=end=2.000000,"),
            "the clip without sound gets 2 s of silence: {graph}"
        );
        assert!(
            graph.ends_with(&format!("concat=n=2:v=0:a=1,apad,atrim=end={total:.6}[a]")),
            "joined and padded to the whole cut: {graph}"
        );
        Ok(())
    }
}

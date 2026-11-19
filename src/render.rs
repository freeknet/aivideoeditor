// ============================================================================
// Module:       render
// Description:  Renders a plan to an mp4 with one ffmpeg filter graph:
//               frame-exact trims on the beat grid, crop, per-shot exposure
//               match, denoise, end card, straight music trim.
//
// Dependencies: crate::plan, crate::index, crate::beats, crate::media
// ============================================================================

//! Rendering.
//!
//! Every shot is conformed to the source frame rate and trimmed by frame
//! count, with boundaries computed from the cumulative beat times so the
//! cuts never drift off the music. The music is a straight trim from the
//! start bar: no speed change, no fades, no compression, no gain.
//!
//! Exposure is matched per shot from the index (the shot's highlight level
//! and mean luma), which keeps a dark clip next to a bright one from
//! jumping. No zoom, no speed change: what the camera shot is what plays.
//!
//! The encoder is AMD AMF (`h264_amf`) when `--encoder amf`, else x264.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::analyze::segments::median;
use crate::beats::MusicMap;
use crate::index::Index;
use crate::media;
use crate::plan::{Aspect, Plan};

/// Video encoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Encoder {
    /// libx264 on the CPU, CRF 18.
    X264,
    /// AMD AMF on the GPU.
    Amf,
}

/// Per-shot exposure correction: (input white level, gamma).
fn exposure(index: &Index, clip: &str, t0: f32, t1: f32) -> (f32, f32) {
    let Some(c) = index.clip(clip) else {
        return (1.0, 1.0);
    };
    let w: Vec<&crate::analyze::Sample> = c
        .samples
        .iter()
        .filter(|s| s.t >= t0 && s.t <= t1)
        .collect();
    if w.is_empty() {
        return (1.0, 1.0);
    }
    let p95 = median(&w.iter().map(|s| s.p95).collect::<Vec<_>>());
    let luma = median(&w.iter().map(|s| s.luma).collect::<Vec<_>>());
    // Bring highlights toward 0.85 (gain at most 1.6), lift dark mids gently.
    let gain = (0.85 / p95.max(0.05)).clamp(1.0, 1.6);
    let lifted = (luma * gain).max(0.01);
    let gamma = if lifted < 0.16 {
        (0.16_f32.ln() / lifted.ln()).recip().clamp(1.0, 1.3)
    } else {
        1.0
    };
    (1.0 / gain, gamma)
}

/// Builds the filter graph and input list; returns (args, graph, seconds).
fn build(
    plan: &Plan,
    index: &Index,
    map: &MusicMap,
    fps: (u32, u32),
) -> Result<(Vec<String>, String, f64)> {
    let rate = f64::from(fps.0) / f64::from(fps.1);
    let frames = |sec: f64| (sec * rate).round() as i64;
    let timing = plan.timing(map);
    let origin = map.beat_time(plan.start_bar, 0);
    let card_t = f64::from(map.beat_time(plan.card_bar, 0) - origin);
    let end_t = f64::from(map.beat_time(plan.end_bar, 0) - origin);
    let (ow, oh) = plan.aspect.size();
    let mut args: Vec<String> = Vec::new();
    let mut graph = String::new();
    let mut labels = String::new();

    for (i, (shot, (start, _))) in plan.shots.iter().zip(&timing).enumerate() {
        let clip = index
            .clip(&shot.clip)
            .with_context(|| format!("shot {}: clip {} not indexed", i + 1, shot.clip))?;
        let next = timing.get(i + 1).map_or(card_t, |(s, _)| f64::from(*s));
        let n = frames(next) - frames(f64::from(*start));
        if n <= 0 {
            bail!("shot {} has no frames", i + 1);
        }
        let dur = n as f64 / rate;
        args.extend([
            "-ss".into(),
            format!("{:.3}", shot.t_in),
            "-t".into(),
            format!("{:.3}", dur + 0.5),
            "-i".into(),
            clip.path.clone(),
        ]);
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
        let _ = write!(
            graph,
            "[{i}:v]fps={}/{},trim=end_frame={n},setpts=PTS-STARTPTS,{crop}hqdn3d=2:1.5:4:4,\
             colorlevels=rimax={white:.3}:gimax={white:.3}:bimax={white:.3},eq=gamma={gamma:.3}:saturation=1.15:contrast=1.04,\
             scale={ow}:{oh}:flags=lanczos,setsar=1,format=yuv420p[v{i}];",
            fps.0, fps.1
        );
        let _ = write!(labels, "[v{i}]");
    }
    let n_shots = plan.shots.len();
    let _ = write!(
        graph,
        "{labels}concat=n={n_shots}:v=1:a=0,unsharp=5:5:0.4:5:5:0,vignette=angle=PI/7[cut];"
    );

    let card_frames = frames(end_t) - frames(card_t);
    let mut parts = 1;
    if card_frames > 0 && !plan.end_card.is_empty() {
        args.extend([
            "-loop".into(),
            "1".into(),
            "-framerate".into(),
            format!("{}/{}", fps.0, fps.1),
            "-i".into(),
            plan.end_card.clone(),
        ]);
        let _ = write!(
            graph,
            "[{n_shots}:v]scale={ow}:{oh}:flags=lanczos,trim=end_frame={card_frames},setpts=PTS-STARTPTS,setsar=1,format=yuv420p[card];\
             [cut][card]concat=n=2:v=1:a=0[v]"
        );
        parts += 1;
    } else {
        graph.push_str("[cut]null[v]");
    }
    let total = frames(if parts == 2 { end_t } else { card_t }) as f64 / rate;
    args.extend([
        "-ss".into(),
        format!("{origin:.3}"),
        "-t".into(),
        format!("{total:.3}"),
        "-i".into(),
        plan.music.clone(),
    ]);
    Ok((args, graph, total))
}

/// Renders `plan` to `out`.
pub(crate) fn render(
    plan: &Plan,
    index: &Index,
    map: &MusicMap,
    out: &Path,
    encoder: Encoder,
) -> Result<f64> {
    let first = plan.shots.first().context("the plan has no shots")?;
    let fps = index
        .clip(&first.clip)
        .map_or((24000, 1001), |c| (c.probe.fps[0], c.probe.fps[1].max(1)));
    let (inputs, graph, total) = build(plan, index, map, fps)?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let script = out.with_extension("filter.txt");
    std::fs::write(&script, &graph)?;
    let audio_input = inputs.iter().filter(|a| *a == "-i").count() - 1;
    let codec: Vec<&str> = match encoder {
        Encoder::X264 => vec![
            "-c:v", "libx264", "-preset", "slow", "-crf", "18", "-tune", "film",
        ],
        Encoder::Amf => vec![
            "-c:v", "h264_amf", "-quality", "quality", "-rc", "cqp", "-qp_i", "18", "-qp_p", "20",
        ],
    };
    let mut cmd = Command::new(media::tool("ffmpeg"));
    cmd.args(["-v", "error", "-nostdin", "-y"])
        .args(&inputs)
        .arg("-/filter_complex")
        .arg(&script)
        .args(["-map", "[v]", "-map", &format!("{audio_input}:a")])
        .args(&codec)
        .args(["-r", &format!("{}/{}", fps.0, fps.1), "-pix_fmt", "yuv420p"])
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
    let status = cmd.status().context("running ffmpeg")?;
    if !status.success() {
        bail!(
            "ffmpeg failed rendering {} (filter graph in {})",
            out.display(),
            script.display()
        );
    }
    Ok(total)
}

/// Writes a 720p preview of `video` next to it and returns its path.
pub(crate) fn preview(video: &Path, aspect: Aspect) -> Result<PathBuf> {
    let stem = video
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let out = video.with_file_name(format!("{stem}-720p.mp4"));
    let scale = match aspect {
        Aspect::Wide => "scale=-2:720:flags=lanczos",
        Aspect::Vertical | Aspect::Feed => "scale=720:-2:flags=lanczos",
    };
    let status = Command::new(media::tool("ffmpeg"))
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(video)
        .args([
            "-vf",
            scale,
            "-c:v",
            "libx264",
            "-crf",
            "21",
            "-preset",
            "medium",
            "-c:a",
            "copy",
            "-movflags",
            "+faststart",
        ])
        .arg(&out)
        .status()?;
    if !status.success() {
        bail!("ffmpeg failed making the preview of {}", video.display());
    }
    Ok(out)
}

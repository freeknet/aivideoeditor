// ============================================================================
// Module:       otio
// Description:  Exports a plan as an OpenTimelineIO (.otio) timeline: one
//               video track of source clips (plus the end card) and one audio
//               track with the music trim, for Resolve, Kdenlive or Premiere.
//
// Dependencies: crate::plan, crate::index, crate::beats, serde_json
// ============================================================================

//! OpenTimelineIO export.
//!
//! The research's layer 2: the plan leaves as a timeline file an editor
//! opens, not as prose. Resolve imports `.otio` directly; Kdenlive and
//! Premiere go through OTIO's adapters. Times are frames at the source
//! rate, so a cut lands on the same frame the renderer uses.
//!
//! Crop centres, performer names and notes ride along in each clip's
//! `metadata.ave`, where an editor's scripting (or a later import) can
//! read them; editors themselves ignore unknown metadata.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::beats::MusicMap;
use crate::index::Index;
use crate::plan::Plan;

/// A `RationalTime.1`.
fn rt(value: f64, rate: f64) -> Value {
    json!({"OTIO_SCHEMA": "RationalTime.1", "rate": rate, "value": value})
}

/// A `TimeRange.1`.
fn range(start: f64, duration: f64, rate: f64) -> Value {
    json!({"OTIO_SCHEMA": "TimeRange.1", "start_time": rt(start, rate), "duration": rt(duration, rate)})
}

/// A `file://` URL for a local path.
fn file_url(path: &str) -> String {
    let p = path.replace('\\', "/");
    if p.starts_with('/') {
        format!("file://{p}")
    } else {
        format!("file:///{p}")
    }
}

/// A `Clip.2` referencing `path`.
fn clip(
    name: &str,
    path: &str,
    start: f64,
    frames: f64,
    rate: f64,
    available: Option<f64>,
    meta: Value,
) -> Value {
    let mut reference = json!({
        "OTIO_SCHEMA": "ExternalReference.1",
        "name": "",
        "target_url": file_url(path),
        "metadata": {},
        "available_range": Value::Null,
    });
    if let Some(total) = available {
        reference["available_range"] = range(0.0, total, rate);
    }
    json!({
        "OTIO_SCHEMA": "Clip.2",
        "name": name,
        "source_range": range(start, frames, rate),
        "media_references": {"DEFAULT_MEDIA": reference},
        "active_media_reference_key": "DEFAULT_MEDIA",
        "effects": [], "markers": [], "enabled": true,
        "metadata": {"ave": meta},
    })
}

/// A `Track.1`.
fn track(name: &str, kind: &str, children: Vec<Value>) -> Value {
    json!({
        "OTIO_SCHEMA": "Track.1", "name": name, "kind": kind, "children": children,
        "source_range": Value::Null, "effects": [], "markers": [], "enabled": true, "metadata": {},
    })
}

/// Builds the timeline JSON for `plan`.
pub(crate) fn timeline(plan: &Plan, index: &Index, map: &MusicMap) -> Result<Value> {
    let first = plan.shots.first().context("the plan has no shots")?;
    let fps = index
        .clip(&first.clip)
        .map_or((24000, 1001), |c| (c.probe.fps[0], c.probe.fps[1].max(1)));
    let rate = f64::from(fps.0) / f64::from(fps.1);
    let frames = |sec: f64| (sec * rate).round();
    let timing = plan.timing(map);
    let origin = map.beat_time(plan.start_bar, 0);
    let card_t = f64::from(map.beat_time(plan.card_bar, 0) - origin);
    let end_t = f64::from(map.beat_time(plan.end_bar, 0) - origin);

    let mut video = Vec::new();
    for (i, (shot, (start, _))) in plan.shots.iter().zip(&timing).enumerate() {
        let c = index
            .clip(&shot.clip)
            .with_context(|| format!("clip {} not indexed", shot.clip))?;
        let next = timing.get(i + 1).map_or(card_t, |(s, _)| f64::from(*s));
        let n = frames(next) - frames(f64::from(*start));
        video.push(clip(
            &format!("{:02} {}", i + 1, shot.clip),
            &c.path,
            frames(f64::from(shot.t_in)),
            n,
            rate,
            Some(frames(c.probe.duration)),
            json!({"cx": shot.cx, "who": shot.who, "seg": shot.seg, "note": shot.note, "aspect": plan.aspect}),
        ));
    }
    if !plan.end_card.is_empty() {
        video.push(clip(
            "end card",
            &plan.end_card,
            0.0,
            frames(end_t) - frames(card_t),
            rate,
            None,
            json!({}),
        ));
    }
    let music = clip(
        "music",
        &plan.music,
        frames(f64::from(origin)),
        frames(end_t),
        rate,
        Some(frames(f64::from(map.duration))),
        json!({"bpm": map.bpm, "start_bar": plan.start_bar, "drop_bar": plan.drop_bar}),
    );
    let drop_at = f64::from(map.beat_time(plan.drop_bar, 0) - origin);
    let mut stack = json!({
        "OTIO_SCHEMA": "Stack.1", "name": "tracks",
        "children": [track("V1", "Video", video), track("A1", "Audio", vec![music])],
        "source_range": Value::Null, "effects": [], "enabled": true, "metadata": {},
        "markers": [{"OTIO_SCHEMA": "Marker.2", "name": "drop", "color": "RED", "comment": "",
            "marked_range": range(frames(drop_at), 0.0, rate), "metadata": {}}],
    });
    stack["name"] = json!("tracks");
    Ok(json!({
        "OTIO_SCHEMA": "Timeline.1",
        "name": plan.name,
        "global_start_time": rt(0.0, rate),
        "tracks": stack,
        "metadata": {"ave": {"aspect": plan.aspect, "size": plan.aspect.size()}},
    }))
}

/// Writes `plan` as `.otio` to `out`.
pub(crate) fn export(plan: &Plan, index: &Index, map: &MusicMap, out: &Path) -> Result<()> {
    let value = timeline(plan, index, map)?;
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(out, serde_json::to_string_pretty(&value)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{file_url, range};

    #[test]
    fn windows_paths_become_file_urls() -> anyhow::Result<()> {
        assert_eq!(file_url("C:\\a\\b.mov"), "file:///C:/a/b.mov", "drive path");
        assert_eq!(file_url("/home/x.mov"), "file:///home/x.mov", "unix path");
        let r = range(10.0, 24.0, 23.976);
        assert_eq!(r["duration"]["value"], 24.0, "duration in frames");
        Ok(())
    }
}

//! Ejecuta `ffprobe` sobre una ruta/URL y devuelve un `Probe`.

use pistreaming_core::error::CoreError;
use pistreaming_core::playback::TrackInfo;
use serde::Deserialize;
use std::process::Stdio;

use crate::decide::Probe;

pub fn ffprobe_available() -> bool {
    std::process::Command::new("ffprobe")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[derive(Deserialize)]
struct FfprobeOut {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    #[serde(default)]
    format: Option<FfprobeFormat>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    codec_type: Option<String>,
    #[serde(default)]
    tags: Option<FfprobeTags>,
    #[serde(default)]
    disposition: Option<FfprobeDisposition>,
}

#[derive(Deserialize)]
struct FfprobeTags {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    title: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeDisposition {
    #[serde(default)]
    default: i64,
    #[serde(default)]
    forced: i64,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    #[serde(default)]
    format_name: Option<String>,
}

fn track_from_stream(s: &FfprobeStream, fallback_index: usize) -> TrackInfo {
    TrackInfo {
        index: s.index.unwrap_or(fallback_index),
        codec: s.codec_name.clone(),
        language: s.tags.as_ref().and_then(|t| t.language.clone()),
        title: s.tags.as_ref().and_then(|t| t.title.clone()),
        default: s
            .disposition
            .as_ref()
            .map(|d| d.default != 0)
            .unwrap_or(false),
        forced: s
            .disposition
            .as_ref()
            .map(|d| d.forced != 0)
            .unwrap_or(false),
    }
}

/// Extrae `Probe` del JSON de ffprobe (pura, testeable).
pub fn probe_from_json(json: &str) -> Result<Probe, CoreError> {
    let out: FfprobeOut = serde_json::from_str(json).map_err(|e| CoreError::Json(e.to_string()))?;
    let video_codec = out
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .and_then(|s| s.codec_name.clone());
    let audio_codec = out
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
        .and_then(|s| s.codec_name.clone());
    let audio_tracks: Vec<TrackInfo> = out
        .streams
        .iter()
        .enumerate()
        .filter(|(_, s)| s.codec_type.as_deref() == Some("audio"))
        .map(|(i, s)| track_from_stream(s, i))
        .collect();
    let subtitle_tracks: Vec<TrackInfo> = out
        .streams
        .iter()
        .enumerate()
        .filter(|(_, s)| s.codec_type.as_deref() == Some("subtitle"))
        .map(|(i, s)| track_from_stream(s, i))
        .collect();
    Ok(Probe {
        container: out.format.and_then(|f| f.format_name).unwrap_or_default(),
        video_codec,
        audio_codec,
        audio_tracks,
        subtitle_tracks,
    })
}

/// Corre ffprobe sobre `input` (ruta local o URL) y parsea el JSON.
pub async fn probe(input: &str) -> Result<Probe, CoreError> {
    let out = tokio::process::Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            input,
        ])
        .output()
        .await
        .map_err(|e| CoreError::Other(format!("ffprobe no ejecutable: {e}")))?;
    if !out.status.success() {
        return Err(CoreError::Other(format!(
            "ffprobe falló: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    probe_from_json(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{
      "format": { "format_name": "matroska,webm" },
      "streams": [
        { "index": 0, "codec_name": "h264", "codec_type": "video" },
        { "index": 1, "codec_name": "ac3",  "codec_type": "audio",
          "tags": { "language": "spa", "title": "Español" },
          "disposition": { "default": 1, "forced": 0 } },
        { "index": 2, "codec_name": "aac",  "codec_type": "audio",
          "tags": { "language": "eng" },
          "disposition": { "default": 0, "forced": 0 } },
        { "index": 3, "codec_name": "subrip", "codec_type": "subtitle",
          "tags": { "language": "spa" },
          "disposition": { "default": 0, "forced": 1 } }
      ]
    }"#;

    #[test]
    fn parsea_video_audio_y_contenedor() {
        let p = probe_from_json(JSON).unwrap();
        assert_eq!(p.container, "matroska,webm");
        assert_eq!(p.video_codec.as_deref(), Some("h264"));
        assert_eq!(p.audio_codec.as_deref(), Some("ac3"));
    }

    #[test]
    fn parsea_pistas_de_audio_y_subtitulos() {
        let p = probe_from_json(JSON).unwrap();
        assert_eq!(p.audio_tracks.len(), 2);
        assert_eq!(p.audio_tracks[0].index, 1);
        assert_eq!(p.audio_tracks[0].language.as_deref(), Some("spa"));
        assert_eq!(p.audio_tracks[0].title.as_deref(), Some("Español"));
        assert!(p.audio_tracks[0].default);
        assert_eq!(p.audio_tracks[1].language.as_deref(), Some("eng"));
        assert!(!p.audio_tracks[1].default);
        assert_eq!(p.subtitle_tracks.len(), 1);
        assert_eq!(p.subtitle_tracks[0].index, 3);
        assert!(p.subtitle_tracks[0].forced);
    }

    #[test]
    fn json_invalido_da_error() {
        assert!(probe_from_json("{").is_err());
    }

    #[tokio::test]
    async fn probe_real_se_salta_sin_ffprobe() {
        if !ffprobe_available() {
            eprintln!("skip: ffprobe no está en PATH");
            return;
        }
        // Genera un archivo de 1s con ffmpeg y lo inspecciona.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t.mp4");
        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "quiet",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x64:rate=5",
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&f)
            .status()
            .unwrap()
            .success();
        if !ok {
            eprintln!("skip: ffmpeg no pudo generar el fixture");
            return;
        }
        let p = probe(f.to_str().unwrap()).await.unwrap();
        assert_eq!(p.video_codec.as_deref(), Some("h264"));
    }
}

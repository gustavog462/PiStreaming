//! Ejecuta `ffprobe` sobre una ruta/URL y devuelve un `Probe`.

use std::process::Stdio;
use pistreaming_core::error::CoreError;
use serde::Deserialize;

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
    codec_name: Option<String>,
    #[serde(default)]
    codec_type: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    #[serde(default)]
    format_name: Option<String>,
}

/// Extrae `Probe` del JSON de ffprobe (pura, testeable).
pub fn probe_from_json(json: &str) -> Result<Probe, CoreError> {
    let out: FfprobeOut =
        serde_json::from_str(json).map_err(|e| CoreError::Json(e.to_string()))?;
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
    Ok(Probe {
        container: out
            .format
            .and_then(|f| f.format_name)
            .unwrap_or_default(),
        video_codec,
        audio_codec,
    })
}

/// Corre ffprobe sobre `input` (ruta local o URL) y parsea el JSON.
pub async fn probe(input: &str) -> Result<Probe, CoreError> {
    let out = tokio::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_format", "-show_streams", input])
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
        { "codec_name": "h264", "codec_type": "video" },
        { "codec_name": "ac3",  "codec_type": "audio" },
        { "codec_name": "subrip", "codec_type": "subtitle" }
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
            .args(["-v", "quiet", "-y", "-f", "lavfi", "-i", "testsrc=size=64x64:rate=5",
                   "-t", "1", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
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

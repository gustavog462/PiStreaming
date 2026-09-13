//! Spawn de ffmpeg para remux (copia) o recode de audio.

use std::path::Path;
use std::process::Stdio;
use pistreaming_core::error::CoreError;

/// Remux MKV→fMP4 sin recodificar (`-c copy`). Escribe en `output` (se crea/crece).
pub fn remux(input: &str, output: &Path) -> Result<tokio::process::Child, CoreError> {
    ffmpeg(input, output, false)
}

/// Remux + recodifica solo el audio a AAC (video `-c copy`).
pub fn recode_audio(input: &str, output: &Path) -> Result<tokio::process::Child, CoreError> {
    ffmpeg(input, output, true)
}

fn ffmpeg(input: &str, output: &Path, recode: bool) -> Result<tokio::process::Child, CoreError> {
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.arg("-v").arg("error").arg("-y").arg("-i").arg(input);
    if recode {
        cmd.args(["-map", "0:v:0", "-map", "0:a:0?", "-c:v", "copy",
                  "-c:a", "aac", "-b:a", "192k", "-ac", "2"]);
    } else {
        cmd.args(["-map", "0:v:0", "-map", "0:a:0?", "-c", "copy"]);
    }
    cmd.args(["-movflags", "frag_keyframe+empty_moov", "-f", "mp4"])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()
        .map_err(|e| CoreError::Other(format!("no se pudo lanzar ffmpeg: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Genera un MKV de 1s con video h264 y audio en `acodec`.
    fn fixture(dir: &Path, acodec: &str) -> std::path::PathBuf {
        let input = dir.join("in.mkv");
        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-v", "quiet", "-y", "-f", "lavfi", "-i",
                "testsrc=size=64x64:rate=5", "-f", "lavfi", "-i",
                "anullsrc=r=48000:cl=stereo", "-t", "1",
                "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", acodec,
            ])
            .arg(&input)
            .status()
            .unwrap()
            .success();
        assert!(ok, "no se pudo generar el fixture con ffmpeg");
        input
    }

    fn assert_fmp4(path: &Path) {
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.len() >= 12, "salida demasiado chica: {} bytes", bytes.len());
        assert_eq!(&bytes[4..8], b"ftyp", "no es MP4 (falta ftyp)");
        assert!(
            bytes.windows(4).any(|w| w == b"moof"),
            "fMP4 debe ser fragmentado (falta moof)"
        );
    }

    #[tokio::test]
    async fn remux_sin_input_falla_rapido() {
        let out = std::env::temp_dir().join("pistreaming_ffmpeg_test.mp4");
        // input inexistente: ffmpeg se lanza, sale con error; verificamos que spawn funciona.
        let child = remux("/no/existe.mkv", &out);
        assert!(child.is_ok(), "spawn debe funcionar si ffmpeg está instalado");
    }

    #[tokio::test]
    async fn remux_copia_video_y_audio_sin_recodificar() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg no está en PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = fixture(dir.path(), "aac");
        let out = dir.path().join("playback.mp4");
        let mut child = remux(input.to_str().unwrap(), &out).unwrap();
        assert!(child.wait().await.unwrap().success(), "ffmpeg remux falló");
        assert_fmp4(&out);
        let p = crate::probe::probe(out.to_str().unwrap()).await.unwrap();
        assert_eq!(p.video_codec.as_deref(), Some("h264"));
        assert_eq!(p.audio_codec.as_deref(), Some("aac"));
    }

    #[tokio::test]
    async fn recode_audio_pasa_el_audio_a_aac() {
        if !ffmpeg_available() {
            eprintln!("skip: ffmpeg no está en PATH");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = fixture(dir.path(), "ac3");
        let out = dir.path().join("playback.mp4");
        let mut child = recode_audio(input.to_str().unwrap(), &out).unwrap();
        assert!(child.wait().await.unwrap().success(), "ffmpeg recode falló");
        assert_fmp4(&out);
        let p = crate::probe::probe(out.to_str().unwrap()).await.unwrap();
        assert_eq!(p.video_codec.as_deref(), Some("h264"));
        assert_eq!(p.audio_codec.as_deref(), Some("aac"), "el audio ac3 debe quedar aac");
    }
}

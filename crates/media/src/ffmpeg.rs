//! Spawn de ffmpeg para remux (copia) o recode de audio.

use std::process::Stdio;
use pistreaming_core::error::CoreError;

/// Remux MKV→fMP4 sin recodificar (`-c copy`). Escribe fMP4 fragmentado en
/// stdout (`pipe:1`): el llamador debe tomar `child.stdout` y consumirlo.
pub fn remux(input: &str) -> Result<tokio::process::Child, CoreError> {
    ffmpeg(input, false)
}

/// Remux + recodifica solo el audio a AAC (video `-c copy`). Igual que `remux`,
/// la salida fMP4 va por stdout (`pipe:1`).
pub fn recode_audio(input: &str) -> Result<tokio::process::Child, CoreError> {
    ffmpeg(input, true)
}

fn ffmpeg(input: &str, recode: bool) -> Result<tokio::process::Child, CoreError> {
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.arg("-v").arg("error").arg("-y").arg("-i").arg(input);
    if recode {
        cmd.args(["-map", "0:v:0", "-map", "0:a:0?", "-c:v", "copy",
                  "-c:a", "aac", "-b:a", "192k", "-ac", "2"]);
    } else {
        cmd.args(["-map", "0:v:0", "-map", "0:a:0?", "-c", "copy"]);
    }
    // fMP4 fragmentado por stdout. `default_base_moof` mejora la compatibilidad
    // del init segment en streaming progresivo; el largo final es desconocido.
    cmd.args([
        "-movflags",
        "frag_keyframe+empty_moov+default_base_moof",
        "-f",
        "mp4",
        "pipe:1",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    // Si el consumidor (body HTTP) se suelta, el child muere.
    .kill_on_drop(true);
    cmd.spawn()
        .map_err(|e| CoreError::Other(format!("no se pudo lanzar ffmpeg: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tokio::io::AsyncReadExt;

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

    /// Drena stdout del child y espera a que termine. Devuelve `(bytes, éxito)`.
    async fn drain(mut child: tokio::process::Child) -> (Vec<u8>, bool) {
        let mut stdout = child.stdout.take().expect("stdout debe venir piped");
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.unwrap();
        let ok = child.wait().await.unwrap().success();
        (bytes, ok)
    }

    fn assert_fmp4(bytes: &[u8]) {
        assert!(bytes.len() >= 12, "salida demasiado chica: {} bytes", bytes.len());
        assert_eq!(&bytes[4..8], b"ftyp", "no es MP4 (falta ftyp)");
        assert!(
            bytes.windows(4).any(|w| w == b"moof"),
            "fMP4 debe ser fragmentado (falta moof)"
        );
    }

    #[tokio::test]
    async fn remux_sin_input_falla_rapido() {
        // input inexistente: ffmpeg se lanza, sale con error; verificamos que spawn funciona.
        let child = remux("/no/existe.mkv");
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
        let child = remux(input.to_str().unwrap()).unwrap();
        let (bytes, ok) = drain(child).await;
        assert!(ok, "ffmpeg remux falló");
        assert_fmp4(&bytes);
        // ffprobe necesita una ruta: persistimos los bytes drenados.
        let out = dir.path().join("playback.mp4");
        std::fs::write(&out, &bytes).unwrap();
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
        let child = recode_audio(input.to_str().unwrap()).unwrap();
        let (bytes, ok) = drain(child).await;
        assert!(ok, "ffmpeg recode falló");
        assert_fmp4(&bytes);
        let out = dir.path().join("playback.mp4");
        std::fs::write(&out, &bytes).unwrap();
        let p = crate::probe::probe(out.to_str().unwrap()).await.unwrap();
        assert_eq!(p.video_codec.as_deref(), Some("h264"));
        assert_eq!(p.audio_codec.as_deref(), Some("aac"), "el audio ac3 debe quedar aac");
    }
}

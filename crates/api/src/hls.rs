//! HLS (HTTP Live Streaming) generado con ffmpeg.
//!
//! Alternativa al MP4 progresivo (`media_cache`): segmentos fMP4 + playlist
//! `index.m3u8` que crece mientras ffmpeg escribe. Pensado para el Pi: el vídeo
//! se copia (`-c:v copy`), nunca se recodifica; solo el audio se pasa a AAC si
//! hace falta. Sin nginx-vod ni servicios externos.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use pistreaming_core::error::CoreError;
use pistreaming_core::playback::PlaybackRoute;

use crate::recovery;
use crate::session::PlaySession;
use crate::SharedState;

const WAIT_FILE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);

fn children() -> &'static Mutex<HashMap<String, tokio::process::Child>> {
    static CHILDREN: OnceLock<Mutex<HashMap<String, tokio::process::Child>>> = OnceLock::new();
    CHILDREN.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn hls_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join("hls")
}

pub fn playlist_path(cache_dir: &Path) -> PathBuf {
    hls_dir(cache_dir).join("index.m3u8")
}

/// Número de ffmpeg HLS activos (para métricas).
pub fn active_count() -> usize {
    children().lock().map(|m| m.len()).unwrap_or(0)
}

/// Mata el ffmpeg HLS de la sesión (si sigue vivo).
pub fn stop(session_id: &str) {
    let child = children()
        .lock()
        .ok()
        .and_then(|mut m| m.remove(session_id));
    if let Some(mut child) = child {
        let _ = child.start_kill();
    }
}

/// Asegura que hay un ffmpeg generando HLS para la sesión. Devuelve la ruta del
/// playlist.
pub async fn ensure_started(
    session: &Arc<parking_lot::RwLock<PlaySession>>,
    raw_url: &str,
) -> Result<PathBuf, CoreError> {
    let (session_id, cache_dir, route, audio_index) = {
        let g = session.read();
        (
            g.id.clone(),
            g.cache_dir.clone(),
            g.plan.route,
            g.audio_index,
        )
    };
    let dir = hls_dir(&cache_dir);
    let playlist = playlist_path(&cache_dir);
    if playlist.exists() {
        return Ok(playlist);
    }
    if route == PlaybackRoute::Direct {
        return Err(CoreError::Unsupported(
            "HLS no aplica a fuentes directas".into(),
        ));
    }
    std::fs::create_dir_all(&dir).ok();

    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-re", "-i", raw_url]);
    match route {
        PlaybackRoute::Remux => {
            cmd.args(["-map", "0:v:0", "-map", "0:a:0?", "-c", "copy"]);
        }
        PlaybackRoute::RecodeAudio => {
            cmd.args(["-map", "0:v:0"]);
            match audio_index {
                Some(idx) => {
                    cmd.args(["-map", &format!("0:{idx}")]);
                }
                None => {
                    cmd.args(["-map", "0:a:0?"]);
                }
            }
            cmd.args(["-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-ac", "2"]);
        }
        PlaybackRoute::Direct => unreachable!(),
    }
    let seg_pattern = dir.join("seg_%05d.m4s");
    cmd.args([
        "-f",
        "hls",
        "-hls_time",
        "4",
        "-hls_playlist_type",
        "event",
        "-hls_segment_type",
        "fmp4",
        "-hls_flags",
        "independent_segments",
        "-hls_segment_filename",
    ])
    .arg(&seg_pattern)
    .arg(&playlist)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);

    let child = cmd
        .spawn()
        .map_err(|e| CoreError::Other(format!("no se pudo lanzar ffmpeg HLS: {e}")))?;
    if let Ok(mut m) = children().lock() {
        m.insert(session_id, child);
    }

    // Esperar a que ffmpeg escriba el playlist.
    let mut waited = Duration::ZERO;
    while !playlist.exists() {
        if waited >= WAIT_FILE {
            return Err(CoreError::Other("timeout esperando el playlist HLS".into()));
        }
        tokio::time::sleep(POLL).await;
        waited += POLL;
    }
    Ok(playlist)
}

fn mime_for(path: &str) -> &'static str {
    if path.ends_with(".m3u8") {
        "application/vnd.apple.mpegurl"
    } else if path.ends_with(".m4s") || path.ends_with(".mp4") {
        "video/mp4"
    } else if path.ends_with(".ts") {
        "video/mp2t"
    } else {
        "application/octet-stream"
    }
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, msg.to_string()).into_response()
}

/// GET /hls/:session/*path — sirve el playlist o un segmento (esperando a que
/// ffmpeg lo escriba si aún no existe).
pub async fn serve(
    State(st): State<SharedState>,
    AxumPath((session, path)): AxumPath<(String, String)>,
) -> Response {
    // Sanitizar: solo nombre de fichero dentro del directorio HLS.
    if path.is_empty() || path.contains("..") || path.contains('/') || path.contains('\\') {
        return err(StatusCode::BAD_REQUEST, "ruta inválida");
    }
    let sess = match recovery::ensure_session(&st, &session).await {
        Ok(s) => s,
        Err(e) => return err(StatusCode::NOT_FOUND, &e),
    };
    let (file_id, cache_dir) = {
        let g = sess.read();
        (g.file_id, g.cache_dir.clone())
    };
    let raw_url = format!("{}/raw/{}?file_id={}", st.public_base, session, file_id);
    if let Err(e) = ensure_started(&sess, &raw_url).await {
        return crate::core_err(e);
    }
    let file = hls_dir(&cache_dir).join(&path);

    // Esperar a que el fichero exista (segmentos se escriben tras el playlist).
    let mut waited = Duration::ZERO;
    while !file.exists() {
        if waited >= WAIT_FILE {
            return err(StatusCode::NOT_FOUND, "segmento no disponible");
        }
        tokio::time::sleep(POLL).await;
        waited += POLL;
    }

    let bytes = match tokio::fs::read(&file).await {
        Ok(b) => b,
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("no se pudo leer: {e}"),
            )
        }
    };
    let mut resp = Response::new(Body::from(bytes));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(mime_for(&path)),
    );
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_de_playlist_y_segmentos() {
        assert_eq!(mime_for("index.m3u8"), "application/vnd.apple.mpegurl");
        assert_eq!(mime_for("seg_00000.m4s"), "video/mp4");
        assert_eq!(mime_for("x.ts"), "video/mp2t");
        assert_eq!(mime_for("x.bin"), "application/octet-stream");
    }

    #[test]
    fn rutas_hls() {
        let dir = Path::new("/tmp/cache/abc");
        assert_eq!(hls_dir(dir), PathBuf::from("/tmp/cache/abc/hls"));
        assert_eq!(
            playlist_path(dir),
            PathBuf::from("/tmp/cache/abc/hls/index.m3u8")
        );
    }
}

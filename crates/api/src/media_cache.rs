//! Caché de media: transcode progresivo a `playback.mp4` dentro de la caché de
//! la sesión. El fichero se sirve mientras crece (`growing`), lo que permite
//! seek dentro de lo ya transcodificado y evita el streaming de un solo
//! consumidor por stdout.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use pistreaming_core::error::CoreError;
use pistreaming_core::playback::PlaybackRoute;

use crate::session::PlaySession;

pub const PLAYBACK_FILE: &str = "playback.mp4";

pub fn playback_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(PLAYBACK_FILE)
}

/// `true` si el transcode de la sesión ya terminó.
pub fn is_done(session: &Arc<parking_lot::RwLock<PlaySession>>) -> bool {
    session.read().transcode_done.load(Ordering::Relaxed)
}

/// Mata el ffmpeg de la sesión (si sigue vivo) y espera a que salga.
pub async fn stop(session: &Arc<parking_lot::RwLock<PlaySession>>) {
    let child = {
        let mut g = session.write();
        g.ffmpeg.take()
    };
    if let Some(mut child) = child {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

/// Borra el `playback.mp4` y resetea el estado de transcode (para cambio de audio).
pub async fn reset(session: &Arc<parking_lot::RwLock<PlaySession>>) {
    stop(session).await;
    let (cache_dir, done) = {
        let g = session.read();
        (g.cache_dir.clone(), g.transcode_done.clone())
    };
    done.store(false, Ordering::Relaxed);
    let _ = tokio::fs::remove_file(playback_path(&cache_dir)).await;
}

/// Asegura que hay un ffmpeg escribiendo `playback.mp4` para la sesión.
///
/// Idempotente: si ya hay un ffmpeg vivo o el fichero está completo, no hace
/// nada. Devuelve la ruta del fichero.
pub async fn ensure_started(
    session: &Arc<parking_lot::RwLock<PlaySession>>,
    raw_url: &str,
    state: Option<crate::SharedState>,
) -> Result<PathBuf, CoreError> {
    let (cache_dir, route, audio_index, done, has_child) = {
        let g = session.read();
        (
            g.cache_dir.clone(),
            g.plan.route,
            g.audio_index,
            g.transcode_done.clone(),
            g.ffmpeg.is_some(),
        )
    };
    let path = playback_path(&cache_dir);
    if route == PlaybackRoute::Direct {
        return Ok(path);
    }
    if has_child || (done.load(Ordering::Relaxed) && path.exists()) {
        return Ok(path);
    }
    std::fs::create_dir_all(&cache_dir).ok();
    let _ = std::fs::remove_file(&path);
    done.store(false, Ordering::Relaxed);

    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.args([
        "-v",
        "error",
        "-y",
        "-re",
        "-probesize",
        "1M",
        "-analyzeduration",
        "0",
        "-rw_timeout",
        "15000000",
        "-i",
        raw_url,
    ]);
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
        PlaybackRoute::Direct => return Ok(path),
    }
    cmd.args([
        "-flush_packets",
        "1",
        "-movflags",
        "frag_keyframe+empty_moov+default_base_moof",
        "-f",
        "mp4",
    ])
    .arg(&path)
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);

    let child = cmd
        .spawn()
        .map_err(|e| CoreError::Other(format!("no se pudo lanzar ffmpeg: {e}")))?;
    {
        let mut g = session.write();
        g.ffmpeg = Some(child);
    }

    // Tarea que espera al ffmpeg y marca `transcode_done`. El child sigue en la
    // sesión para que eviction/switch_audio puedan matarlo.
    let sess = session.clone();
    let done2 = done.clone();
    tokio::spawn(async move {
        let mut success = false;
        loop {
            let finished = {
                let mut g = sess.write();
                match g.ffmpeg.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(status)) => {
                            success = status.success();
                            g.ffmpeg = None;
                            true
                        }
                        Ok(None) => false,
                        Err(_) => {
                            g.ffmpeg = None;
                            true
                        }
                    },
                    None => true,
                }
            };
            if finished {
                done2.store(true, Ordering::Relaxed);
                if success {
                    if let Some(st) = state.as_ref() {
                        let sid = sess.read().id.clone();
                        if let Err(e) = st.store.set_transcode_done(&sid, true) {
                            tracing::warn!(error = %e, "no se pudo persistir transcode_done");
                        }
                    }
                }
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    // Esperar a que ffmpeg cree el fichero (abre el muxer al arrancar).
    let mut waited = Duration::ZERO;
    while !path.exists() {
        if done.load(Ordering::Relaxed) {
            break;
        }
        if waited >= Duration::from_secs(30) {
            return Err(CoreError::Other(
                "timeout esperando a que ffmpeg cree playback.mp4".into(),
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        waited += Duration::from_millis(100);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::PlaySession;
    use pistreaming_core::playback::{PlaybackPlan, PlaybackRoute};
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    fn session(dir: &Path, route: PlaybackRoute) -> Arc<parking_lot::RwLock<PlaySession>> {
        let plan = PlaybackPlan {
            session: "s1".into(),
            route,
            playback_url: "/stream/s1".into(),
            raw_url: Some("/raw/s1".into()),
            browser_may_fail: false,
            needs_recode_audio: false,
            video_codec: "h264".into(),
            audio_codec: Some("aac".into()),
            audio_tracks: vec![],
            subtitle_tracks: vec![],
            active_audio_index: None,
            progress_url: None,
        };
        Arc::new(parking_lot::RwLock::new(PlaySession {
            id: "s1".into(),
            info_hash: "abc".into(),
            file_id: 0,
            plan,
            cache_dir: dir.to_path_buf(),
            created_at: Instant::now(),
            ffmpeg: None,
            progress_key: None,
            kind: None,
            meta_id: None,
            title: None,
            media_path: None,
            magnet: None,
            transcode_done: Arc::new(AtomicBool::new(false)),
            audio_index: None,
        }))
    }

    #[test]
    fn playback_path_es_playback_mp4() {
        assert_eq!(
            playback_path(Path::new("/tmp/x")),
            PathBuf::from("/tmp/x/playback.mp4")
        );
    }

    #[tokio::test]
    async fn direct_no_lanza_ffmpeg() {
        let dir = tempfile::tempdir().unwrap();
        let s = session(dir.path(), PlaybackRoute::Direct);
        let p = ensure_started(&s, "http://127.0.0.1:1/raw/s1", None)
            .await
            .unwrap();
        assert_eq!(p, playback_path(dir.path()));
        assert!(!is_done(&s));
    }

    #[tokio::test]
    async fn reset_borra_fichero_y_marca_no_done() {
        let dir = tempfile::tempdir().unwrap();
        let s = session(dir.path(), PlaybackRoute::Remux);
        let p = playback_path(dir.path());
        std::fs::write(&p, b"x").unwrap();
        s.read().transcode_done.store(true, Ordering::Relaxed);
        reset(&s).await;
        assert!(!p.exists());
        assert!(!is_done(&s));
    }
}

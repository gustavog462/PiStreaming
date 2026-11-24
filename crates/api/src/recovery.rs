//! Recuperación de sesiones de reproducción tras un reinicio.
//!
//! Persistimos cada `PlaySession` en la tabla `play_sessions` (magnet, plan,
//! identidad de ficha). Al arrancar se reabren los torrents y se reconstruyen
//! las sesiones en memoria; si el cliente pide una sesión que no está (p. ej.
//! tras un reinicio a mitad de reproducción), `ensure_session` la recrea.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pistreaming_core::playback::PlaybackPlan;
use pistreaming_store::PlaySessionRow;

use crate::session::PlaySession;
use crate::SharedState;

/// Reabre todas las sesiones persistidas (best effort).
pub async fn load_at_boot(state: &SharedState) {
    let rows = match state.store.list_play_sessions() {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(error = %e, "no se pudieron leer las sesiones persistidas");
            return;
        }
    };
    if rows.is_empty() {
        return;
    }
    let mut restored = 0usize;
    for row in &rows {
        match restore_row(state, row).await {
            Ok(()) => restored += 1,
            Err(e) => {
                tracing::warn!(session = %row.id, error = %e, "no se pudo restaurar la sesión")
            }
        }
    }
    tracing::info!(
        restored,
        total = rows.len(),
        "sesiones de reproducción restauradas"
    );
}

/// Devuelve la sesión, recreándola desde la DB si no está en memoria.
pub async fn ensure_session(
    state: &SharedState,
    id: &str,
) -> Result<Arc<parking_lot::RwLock<PlaySession>>, String> {
    if let Some(s) = state.sessions.get(id) {
        return Ok(s);
    }
    let row = state
        .store
        .get_play_session(id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "sesión desconocida".to_string())?;
    restore_row(state, &row).await?;
    state
        .sessions
        .get(id)
        .ok_or_else(|| "no se pudo recrear la sesión".to_string())
}

/// Asegura que el handle del torrent está en memoria (lo recrea si hace falta).
pub async fn ensure_handle(
    state: &SharedState,
    id: &str,
) -> Result<Arc<librqbit::ManagedTorrent>, String> {
    if let Some(h) = state.handles.read().get(id).cloned() {
        return Ok(h);
    }
    let _ = ensure_session(state, id).await?;
    state
        .handles
        .read()
        .get(id)
        .cloned()
        .ok_or_else(|| "no se pudo recrear el handle del torrent".to_string())
}

async fn restore_row(state: &SharedState, row: &PlaySessionRow) -> Result<(), String> {
    let plan: PlaybackPlan =
        serde_json::from_str(&row.plan_json).map_err(|e| format!("plan inválido: {e}"))?;
    let session = state
        .torrents
        .get()
        .ok_or_else(|| "engine de torrents no inicializado".to_string())?
        .clone();

    let (handle, info_hash) = match row.magnet.as_deref() {
        Some(magnet) if !magnet.is_empty() => {
            let added = tokio::time::timeout(
                Duration::from_secs(30),
                pistreaming_torrent::add_magnet(&session, magnet, &state.cache_dir),
            )
            .await
            .map_err(|_| "timeout reabriendo el magnet".to_string())?
            .map_err(|e| e.to_string())?;
            (added.handle, added.info_hash)
        }
        _ => return Err("la sesión no tiene magnet; no se puede reabrir".to_string()),
    };

    let cache_dir = state.cache_dir.join(&info_hash);
    std::fs::create_dir_all(&cache_dir).ok();
    state.handles.write().insert(row.id.clone(), handle.clone());

    let audio_index = plan
        .active_audio_index
        .and_then(|i| plan.audio_tracks.get(i))
        .map(|t| t.index);
    let media_path = handle
        .with_metadata(|m| {
            m.file_infos
                .get(row.file_id as usize)
                .map(|f| f.relative_filename.clone())
        })
        .ok()
        .flatten()
        .map(|rel| handle.output_folder().join(rel));

    state.sessions.insert(PlaySession {
        id: row.id.clone(),
        info_hash,
        file_id: row.file_id as usize,
        plan,
        cache_dir,
        created_at: Instant::now(),
        ffmpeg: None,
        progress_key: row.progress_key.clone(),
        kind: row.kind.clone(),
        meta_id: row.meta_id.clone(),
        title: row.title.clone(),
        media_path,
        magnet: row.magnet.clone(),
        transcode_done: Arc::new(AtomicBool::new(row.transcode_done)),
        audio_index,
    });
    Ok(())
}

/// Borra de la DB las sesiones más viejas que `max_age_secs`.
pub fn prune(state: &SharedState, max_age_secs: i64) {
    match state.store.prune_play_sessions(max_age_secs) {
        Ok(n) if n > 0 => tracing::info!(pruned = n, "sesiones persistidas expiradas podadas"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "no se pudieron podar sesiones persistidas"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ensure_session_sin_fila_es_error() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_state(dir.path().to_path_buf());
        let err = match ensure_session(&state, "no-existe").await {
            Ok(_) => panic!("debería fallar"),
            Err(e) => e,
        };
        assert!(err.contains("desconocida"), "{err}");
    }

    #[tokio::test]
    async fn prune_borra_viejas() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_state(dir.path().to_path_buf());
        let row = PlaySessionRow {
            id: "s1".into(),
            info_hash: "abc".into(),
            file_id: 0,
            magnet: Some("magnet:?xt=urn:btih:abc".into()),
            plan_json: "{}".into(),
            kind: None,
            meta_id: None,
            title: None,
            progress_key: None,
            transcode_done: false,
            created_at: 0,
        };
        state.store.save_play_session(&row).unwrap();
        prune(&state, 60);
        assert!(state.store.get_play_session("s1").unwrap().is_none());
    }
}

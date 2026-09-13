//! Registro en memoria de sesiones de reproducción activas.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use pistreaming_core::playback::PlaybackPlan;

/// Una sesión de reproducción viva.
pub struct PlaySession {
    pub id: String,
    pub info_hash: String,
    pub file_id: usize,
    pub plan: PlaybackPlan,
    pub cache_dir: std::path::PathBuf,
    pub created_at: Instant,
    /// Child de ffmpeg para remux/recode, si aplica.
    pub ffmpeg: Option<tokio::process::Child>,
    /// Clave compuesta `kind:id` para el progreso, si `POST /api/play` la recibió.
    pub progress_key: Option<String>,
}

#[derive(Default)]
pub struct PlaySessionRegistry {
    inner: RwLock<HashMap<String, Arc<RwLock<PlaySession>>>>,
}

impl PlaySessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, s: PlaySession) -> Arc<RwLock<PlaySession>> {
        let id = s.id.clone();
        let arc = Arc::new(RwLock::new(s));
        self.inner.write().insert(id, Arc::clone(&arc));
        arc
    }

    pub fn get(&self, id: &str) -> Option<Arc<RwLock<PlaySession>>> {
        self.inner.read().get(id).cloned()
    }

    pub fn remove(&self, id: &str) -> Option<Arc<RwLock<PlaySession>>> {
        self.inner.write().remove(id)
    }

    pub fn active_ids(&self) -> Vec<String> {
        self.inner.read().keys().cloned().collect()
    }

    /// `info_hash` de las sesiones vivas. A diferencia de `active_ids` (que usa
    /// el id `hash[..12]-file_id`), este es el nombre real de la carpeta de
    /// caché de la sesión, útil para eviction.
    pub fn active_info_hashes(&self) -> Vec<String> {
        self.inner
            .read()
            .values()
            .map(|s| s.read().info_hash.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// IDs de sesiones más viejas que `max_age`.
    pub fn stale_ids(&self, max_age: Duration) -> Vec<String> {
        let now = Instant::now();
        self.inner
            .read()
            .iter()
            .filter(|(_, s)| now.duration_since(s.read().created_at) > max_age)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pistreaming_core::playback::PlaybackRoute;

    fn plan() -> PlaybackPlan {
        PlaybackPlan {
            session: "s1".into(),
            route: PlaybackRoute::Direct,
            playback_url: "/stream/s1".into(),
            raw_url: Some("/raw/s1".into()),
            browser_may_fail: false,
            needs_recode_audio: false,
            video_codec: "h264".into(),
            audio_codec: Some("aac".into()),
            progress_url: None,
        }
    }

    fn sess(id: &str) -> PlaySession {
        PlaySession {
            id: id.into(),
            info_hash: "abc".into(),
            file_id: 0,
            plan: plan(),
            cache_dir: "/tmp".into(),
            created_at: Instant::now(),
            ffmpeg: None,
            progress_key: None,
        }
    }

    #[test]
    fn insert_get_remove() {
        let r = PlaySessionRegistry::new();
        r.insert(sess("a"));
        assert_eq!(r.len(), 1);
        assert!(r.get("a").is_some());
        assert!(r.remove("a").is_some());
        assert!(r.get("a").is_none());
    }

    #[test]
    fn stale_ids_detecta_viejas() {
        let r = PlaySessionRegistry::new();
        let mut s = sess("old");
        s.created_at = Instant::now() - Duration::from_secs(3600);
        r.insert(s);
        r.insert(sess("new"));
        let stale = r.stale_ids(Duration::from_secs(60));
        assert_eq!(stale, vec!["old".to_string()]);
        assert_eq!(r.active_ids().len(), 2);
    }
}

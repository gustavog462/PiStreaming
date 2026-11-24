use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackRoute {
    /// El browser toma el archivo tal cual (o remux trivial).
    Direct,
    /// Remux MKV→fMP4 (cambio de contenedor, sin recodificar video).
    Remux,
    /// Remux + recodifica SOLO el audio a AAC (video `-c copy`).
    RecodeAudio,
}

/// Pista de audio o subtítulo expuesta al reproductor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackInfo {
    /// Índice del stream en ffprobe (`-map 0:<index>`).
    pub index: usize,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub forced: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackPlan {
    pub session: String,
    pub route: PlaybackRoute,
    /// URL que consume el <video>.
    pub playback_url: String,
    /// URL cruda del .mkv para mpv/VLC cuando el browser no decodifica.
    pub raw_url: Option<String>,
    pub browser_may_fail: bool,
    pub needs_recode_audio: bool,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    /// Pistas de audio del contenedor (para el selector del player).
    #[serde(default)]
    pub audio_tracks: Vec<TrackInfo>,
    /// Pistas de subtítulos embebidas.
    #[serde(default)]
    pub subtitle_tracks: Vec<TrackInfo>,
    /// Índice (en `audio_tracks`) de la pista activa.
    #[serde(default)]
    pub active_audio_index: Option<usize>,
    /// URL absoluta a la que el player hace `PUT` del progreso (`kind`/`id`
    /// recibidos en `POST /api/play`). Ausente si no se conocen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(route: PlaybackRoute) -> PlaybackPlan {
        PlaybackPlan {
            session: "s1".into(),
            route,
            playback_url: "/stream/s1".into(),
            raw_url: Some("/raw/s1.mkv".into()),
            browser_may_fail: true,
            needs_recode_audio: false,
            video_codec: "hevc".into(),
            audio_codec: Some("eac3".into()),
            audio_tracks: vec![],
            subtitle_tracks: vec![],
            active_audio_index: None,
            progress_url: None,
        }
    }

    #[test]
    fn serializes_route_lowercase() {
        let p = plan(PlaybackRoute::Remux);
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains("\"route\":\"remux\""));
        assert!(j.contains("\"video_codec\":\"hevc\""));
    }

    #[test]
    fn progress_url_none_se_omite() {
        let p = plan(PlaybackRoute::Direct);
        let j = serde_json::to_string(&p).unwrap();
        assert!(
            !j.contains("progress_url"),
            "progress_url None no debe serializarse: {j}"
        );
        // Y un JSON sin el campo (compat) deserializa a None.
        let back: PlaybackPlan = serde_json::from_str(
            r#"{"session":"s1","route":"direct","playback_url":"/stream/s1",
                "raw_url":null,"browser_may_fail":false,"needs_recode_audio":false,
                "video_codec":"h264","audio_codec":"aac"}"#,
        )
        .unwrap();
        assert_eq!(back.progress_url, None);
        assert!(back.audio_tracks.is_empty());
        assert_eq!(back.active_audio_index, None);
    }

    #[test]
    fn progress_url_some_se_serializa() {
        let mut p = plan(PlaybackRoute::Direct);
        p.progress_url = Some("/api/progress/movie/tt1".into());
        let j = serde_json::to_string(&p).unwrap();
        assert!(
            j.contains("\"progress_url\":\"/api/progress/movie/tt1\""),
            "progress_url Some debe serializarse: {j}"
        );
    }

    #[test]
    fn recode_audio_serializa_en_minusculas() {
        let j = serde_json::to_string(&PlaybackRoute::RecodeAudio).unwrap();
        assert_eq!(j, "\"recodeaudio\"");
        let back: PlaybackRoute = serde_json::from_str(&j).unwrap();
        assert_eq!(back, PlaybackRoute::RecodeAudio);
    }

    #[test]
    fn tracks_se_serializan_y_deserializan() {
        let mut p = plan(PlaybackRoute::Remux);
        p.audio_tracks = vec![TrackInfo {
            index: 1,
            codec: Some("aac".into()),
            language: Some("spa".into()),
            title: Some("Español".into()),
            default: true,
            forced: false,
        }];
        p.active_audio_index = Some(0);
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains("\"audio_tracks\""));
        assert!(j.contains("\"active_audio_index\":0"));
        let back: PlaybackPlan = serde_json::from_str(&j).unwrap();
        assert_eq!(back.audio_tracks.len(), 1);
        assert_eq!(back.audio_tracks[0].language.as_deref(), Some("spa"));
        assert_eq!(back.active_audio_index, Some(0));
    }
}

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
    /// URL absoluta a la que el player hace `PUT` del progreso (`kind`/`id`
    /// recibidos en `POST /api/play`). Ausente si no se conocen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_route_lowercase() {
        let p = PlaybackPlan {
            session: "s1".into(),
            route: PlaybackRoute::Remux,
            playback_url: "/stream/s1".into(),
            raw_url: Some("/raw/s1.mkv".into()),
            browser_may_fail: true,
            needs_recode_audio: false,
            video_codec: "hevc".into(),
            audio_codec: Some("eac3".into()),
            progress_url: None,
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains("\"route\":\"remux\""));
        assert!(j.contains("\"video_codec\":\"hevc\""));
    }

    #[test]
    fn progress_url_none_se_omite() {
        let p = PlaybackPlan {
            session: "s1".into(),
            route: PlaybackRoute::Direct,
            playback_url: "/stream/s1".into(),
            raw_url: None,
            browser_may_fail: false,
            needs_recode_audio: false,
            video_codec: "h264".into(),
            audio_codec: Some("aac".into()),
            progress_url: None,
        };
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
    }

    #[test]
    fn progress_url_some_se_serializa() {
        let mut p = PlaybackPlan {
            session: "s1".into(),
            route: PlaybackRoute::Direct,
            playback_url: "/stream/s1".into(),
            raw_url: None,
            browser_may_fail: false,
            needs_recode_audio: false,
            video_codec: "h264".into(),
            audio_codec: Some("aac".into()),
            progress_url: None,
        };
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
}

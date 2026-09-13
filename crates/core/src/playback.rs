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
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains("\"route\":\"remux\""));
        assert!(j.contains("\"video_codec\":\"hevc\""));
    }

    #[test]
    fn recode_audio_serializa_en_minusculas() {
        let j = serde_json::to_string(&PlaybackRoute::RecodeAudio).unwrap();
        assert_eq!(j, "\"recodeaudio\"");
        let back: PlaybackRoute = serde_json::from_str(&j).unwrap();
        assert_eq!(back, PlaybackRoute::RecodeAudio);
    }
}

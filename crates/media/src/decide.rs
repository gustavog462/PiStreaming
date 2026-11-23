//! Decisión pura de ruta de reproducción. Sin I/O.

use pistreaming_core::error::CoreError;
use pistreaming_core::playback::{PlaybackPlan, PlaybackRoute, TrackInfo};

/// Subconjunto de ffprobe que necesitamos para decidir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// `format.format_name` normalizado (p. ej. "matroska", "mov,mp4,m4a,3gp,3g2,mj2").
    pub container: String,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_tracks: Vec<TrackInfo>,
    pub subtitle_tracks: Vec<TrackInfo>,
}

const AUDIO_RECODE: [&str; 4] = ["ac3", "eac3", "truehd", "dts"];
const VIDEO_OK: [&str; 5] = ["h264", "hevc", "av1", "vp8", "vp9"];

fn container_is_browser_ok(container: &str) -> bool {
    container
        .split(',')
        .any(|c| matches!(c.trim(), "mp4" | "mov" | "m4v" | "webm"))
}

fn browser_may_fail(video: &str) -> bool {
    matches!(video, "hevc" | "av1")
}

/// Índice (dentro de `audio_tracks`) de la pista activa: la primera marcada
/// `default`, o la primera si ninguna lo está.
fn active_audio_index(tracks: &[TrackInfo]) -> Option<usize> {
    if tracks.is_empty() {
        return None;
    }
    tracks.iter().position(|t| t.default).or(Some(0))
}

pub fn decide(
    session: &str,
    probe: &Probe,
    playback_url: &str,
    raw_url: Option<&str>,
) -> Result<PlaybackPlan, CoreError> {
    let video = probe.video_codec.clone().unwrap_or_default();
    let audio = probe.audio_codec.clone();
    if !VIDEO_OK.contains(&video.as_str()) {
        return Err(CoreError::Unsupported(format!(
            "video codec no soportado: {video}"
        )));
    }
    let audio_needs = audio.as_deref().is_some_and(|a| AUDIO_RECODE.contains(&a));
    let route = if audio_needs {
        PlaybackRoute::RecodeAudio
    } else if container_is_browser_ok(&probe.container) && !browser_may_fail(&video) {
        PlaybackRoute::Direct
    } else {
        PlaybackRoute::Remux
    };
    Ok(PlaybackPlan {
        session: session.to_string(),
        route,
        playback_url: playback_url.to_string(),
        raw_url: raw_url.map(str::to_string),
        browser_may_fail: browser_may_fail(&video),
        needs_recode_audio: audio_needs,
        video_codec: video,
        audio_codec: audio,
        audio_tracks: probe.audio_tracks.clone(),
        subtitle_tracks: probe.subtitle_tracks.clone(),
        active_audio_index: active_audio_index(&probe.audio_tracks),
        progress_url: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(c: &str, v: Option<&str>, a: Option<&str>) -> Probe {
        Probe {
            container: c.into(),
            video_codec: v.map(str::to_string),
            audio_codec: a.map(str::to_string),
            audio_tracks: vec![],
            subtitle_tracks: vec![],
        }
    }

    fn route(c: &str, v: &str, a: Option<&str>) -> PlaybackRoute {
        decide("s", &p(c, Some(v), a), "/stream/s", Some("/raw/s"))
            .unwrap()
            .route
    }

    #[test]
    fn mp4_h264_aac_es_direct() {
        assert_eq!(
            route("mov,mp4,m4a,3gp,3g2,mj2", "h264", Some("aac")),
            PlaybackRoute::Direct
        );
    }

    #[test]
    fn webm_vp9_opus_es_direct() {
        assert_eq!(
            route("matroska,webm", "vp9", Some("opus")),
            PlaybackRoute::Direct
        );
    }

    #[test]
    fn mkv_h264_aac_es_remux() {
        assert_eq!(route("matroska", "h264", Some("aac")), PlaybackRoute::Remux);
    }

    #[test]
    fn mkv_hevc_marca_browser_may_fail() {
        let plan = decide(
            "s",
            &p("matroska", Some("hevc"), Some("aac")),
            "/stream/s",
            None,
        )
        .unwrap();
        assert_eq!(plan.route, PlaybackRoute::Remux);
        assert!(plan.browser_may_fail);
    }

    #[test]
    fn mkv_h264_ac3_recodifica_audio() {
        let plan = decide(
            "s",
            &p("matroska", Some("h264"), Some("ac3")),
            "/stream/s",
            None,
        )
        .unwrap();
        assert_eq!(plan.route, PlaybackRoute::RecodeAudio);
        assert!(plan.needs_recode_audio);
    }

    #[test]
    fn truehd_y_dts_tambien_recodifican() {
        assert_eq!(
            route("matroska", "h264", Some("truehd")),
            PlaybackRoute::RecodeAudio
        );
        assert_eq!(
            route("matroska", "h264", Some("dts")),
            PlaybackRoute::RecodeAudio
        );
    }

    #[test]
    fn video_no_soportado_da_error_422() {
        let e = decide(
            "s",
            &p("mpegts", Some("mpeg2video"), Some("aac")),
            "/stream/s",
            None,
        )
        .unwrap_err();
        assert!(matches!(e, CoreError::Unsupported(_)));
    }

    #[test]
    fn sin_audio_no_rompe() {
        assert_eq!(route("matroska", "h264", None), PlaybackRoute::Remux);
    }

    #[test]
    fn pistas_se_copian_al_plan_y_marca_la_default() {
        let mut probe = p("matroska", Some("h264"), Some("aac"));
        probe.audio_tracks = vec![
            TrackInfo {
                index: 1,
                codec: Some("aac".into()),
                language: Some("eng".into()),
                title: None,
                default: false,
                forced: false,
            },
            TrackInfo {
                index: 2,
                codec: Some("aac".into()),
                language: Some("spa".into()),
                title: None,
                default: true,
                forced: false,
            },
        ];
        probe.subtitle_tracks = vec![TrackInfo {
            index: 3,
            codec: Some("subrip".into()),
            language: Some("spa".into()),
            title: None,
            default: false,
            forced: false,
        }];
        let plan = decide("s", &probe, "/stream/s", None).unwrap();
        assert_eq!(plan.audio_tracks.len(), 2);
        assert_eq!(plan.subtitle_tracks.len(), 1);
        assert_eq!(
            plan.active_audio_index,
            Some(1),
            "debe marcar la pista default"
        );
    }

    #[test]
    fn sin_pistas_active_audio_index_es_none() {
        let plan = decide(
            "s",
            &p("matroska", Some("h264"), Some("aac")),
            "/stream/s",
            None,
        )
        .unwrap();
        assert_eq!(plan.active_audio_index, None);
        assert!(plan.audio_tracks.is_empty());
    }
}

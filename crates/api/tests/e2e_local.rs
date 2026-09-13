//! E2E local: con un seeder local y el engine habilitado, `POST /api/play`
//! devuelve un plan y `GET /stream/:session` responde 206 con el rango pedido.
//!
//! Aísla la red: DHT desactivado. El cliente descubre al seeder mediante un
//! tracker HTTP mínimo servido por el propio test (el magnet declara su `tr`),
//! porque un magnet "pelado" sin DHT/trackers/peers no puede resolver metadatos
//! y `POST /api/play` devolvería 500. El router corre en un listener TCP real
//! para que el fallback del probe (`probe(&raw_url)`) sea alcanzable.
//!
//! Requiere `ffmpeg`/`ffprobe` en PATH; si faltan o no se puede generar un clip
//! soportado, el test hace skip (verde sin asserts).

use std::sync::Arc;
use std::time::Duration;

use librqbit::{CreateTorrentOptions, ListenerOptions, Session, SessionOptions};
use pistreaming_core::playback::{PlaybackPlan, PlaybackRoute};

/// Opciones de sesión aisladas de la red pública: sin DHT y sin trackers.
fn offline_session_opts() -> SessionOptions {
    SessionOptions {
        dht: None,
        disable_trackers: true,
        ..Default::default()
    }
}

/// Opciones del cliente: sin DHT, pero con trackers habilitados para resolver el
/// magnet contra un tracker HTTP local (ver `iniciar_tracker`). No hay trackers
/// públicos: el magnet solo lleva la URL local.
fn client_session_opts() -> SessionOptions {
    SessionOptions {
        dht: None,
        ..Default::default()
    }
}

/// Cuerpo bencode de un tracker HTTP con un único peer compacto (IPv4+puerto).
fn tracker_body(peer: std::net::SocketAddr) -> Vec<u8> {
    let std::net::SocketAddr::V4(v4) = peer else {
        panic!("el tracker de prueba solo soporta IPv4");
    };
    let mut body = b"d8:intervali1800e5:peers6:".to_vec();
    body.extend_from_slice(&v4.ip().octets());
    body.extend_from_slice(&v4.port().to_be_bytes());
    body.push(b'e');
    body
}

/// Genera un clip H.264/MP4 chico (faststart) con ffmpeg.
///
/// Devuelve `false` si falta `ffmpeg`/`ffprobe`, la generación falla o el clip
/// queda demasiado chico para el rango de prueba (skip).
fn generar_clip(path: &std::path::Path) -> bool {
    if !pistreaming_media::probe::ffprobe_available() {
        return false;
    }
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-v",
            "quiet",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=320x240:rate=10",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "18",
            "-movflags",
            "+faststart",
        ])
        .arg(path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok && std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) >= 4096
}

#[tokio::test]
async fn e2e_play_y_stream_range() {
    let dir = tempfile::tempdir().unwrap();
    let seed_dir = dir.path().join("seed");
    let client_dir = dir.path().join("client");
    std::fs::create_dir_all(&seed_dir).unwrap();
    std::fs::create_dir_all(&client_dir).unwrap();

    let clip = seed_dir.join("clip.mp4");
    if !generar_clip(&clip) {
        eprintln!("skip: ffmpeg/ffprobe no disponibles o no se pudo generar un clip soportado");
        return;
    }

    // --- seeder local (sin DHT/trackers), con listener en loopback ---
    let seeder: Arc<Session> = Session::new_with_opts(
        seed_dir.clone(),
        SessionOptions {
            listen: Some(ListenerOptions {
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                ..Default::default()
            }),
            ..offline_session_opts()
        },
    )
    .await
    .unwrap();
    let (_meta, seeder_handle) = seeder
        .create_and_serve_torrent(&clip, CreateTorrentOptions::default())
        .await
        .unwrap();
    seeder_handle.wait_until_initialized().await.unwrap();
    let info_hash = seeder_handle.info_hash().as_string();
    let seed_addr = seeder
        .listen_addr()
        .expect("el seeder debe tener listener local");

    // --- tracker HTTP mínimo: siempre responde con el peer del seeder local ---
    // El engine resuelve los metadatos del magnet ANTES de deduplicar, así que
    // sin DHT/trackers/initial_peers un magnet pelado no resuelve (500).
    let tracker_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tracker_port = tracker_listener.local_addr().unwrap().port();
    let tracker_app = axum::Router::new().route(
        "/announce",
        axum::routing::get(move || async move { tracker_body(seed_addr) }),
    );
    let tracker_server = tokio::spawn(async move {
        axum::serve(tracker_listener, tracker_app).await.unwrap();
    });
    let magnet =
        format!("magnet:?xt=urn:btih:{info_hash}&tr=http://127.0.0.1:{tracker_port}/announce");

    // --- cliente (engine real): sin DHT, descubre al seeder vía el tracker local ---
    let client: Arc<Session> = Session::new_with_opts(client_dir.clone(), client_session_opts())
        .await
        .unwrap();

    // --- router en un listener TCP efímero; public_base apunta a él ---
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut state = pistreaming_api::test_state(client_dir.clone());
    {
        let s = Arc::get_mut(&mut state).expect("state sin clones todavía");
        s.public_base = format!("http://127.0.0.1:{port}");
    }
    state.torrents.set(client.clone()).ok();
    let app = pistreaming_api::router(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let base = format!("http://127.0.0.1:{port}");

    let flujo = async {
        // 1) POST /api/play
        let resp = reqwest::Client::new()
            .post(format!("{base}/api/play"))
            .json(&serde_json::json!({ "magnet": magnet }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "POST /api/play debe dar 200");
        let plan: PlaybackPlan = resp.json().await.unwrap();

        // 2) ruta soportada (nunca Unsupported)
        assert!(
            matches!(
                plan.route,
                PlaybackRoute::Direct | PlaybackRoute::Remux | PlaybackRoute::RecodeAudio
            ),
            "ruta inesperada: {:?}",
            plan.route
        );

        // 3) GET /stream/:session con Range -> 206 coherente
        let r = reqwest::Client::new()
            .get(format!("{base}/stream/{}", plan.session))
            .header("range", "bytes=0-1023")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 206, "GET /stream con Range debe dar 206");
        assert_eq!(
            r.headers()
                .get("content-length")
                .and_then(|v| v.to_str().ok()),
            Some("1024"),
            "Content-Length debe ser 1024"
        );
        let cr = r
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            cr.starts_with("bytes 0-1023/"),
            "Content-Range inesperado: {cr}"
        );
    };

    let result = tokio::time::timeout(Duration::from_secs(120), flujo).await;
    server.abort();
    tracker_server.abort();
    result.expect("el E2E no debe colgarse (timeout)");
}

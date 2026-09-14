//! Levanta una sesión "seeder" que sirve un archivo local como torrent y
//! valida que el engine puede agregarlo y leer bytes del stream.
//!
//! Determinista y sin red pública: se desactivan DHT y trackers en ambas
//! sesiones y el cliente solo conoce al seeder por `initial_peers`.
use std::net::SocketAddr;
use std::sync::Arc;

use librqbit::{
    AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session, SessionOptions,
};
use pistreaming_torrent::{pick_largest_video, torrent_stream};
use tokio::io::AsyncReadExt;

/// Opciones de sesión aisladas de la red pública: sin DHT y sin trackers.
fn offline_session_opts() -> SessionOptions {
    SessionOptions {
        dht: None,
        disable_trackers: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn descarga_de_un_seeder_local() {
    let dir = tempfile::tempdir().unwrap();
    let seed_dir = dir.path().join("seed");
    std::fs::create_dir_all(&seed_dir).unwrap();
    let payload: Vec<u8> = (0..(256 * 1024)).map(|i| (i % 251) as u8).collect();
    let file = seed_dir.join("clip.mkv");
    std::fs::write(&file, &payload).unwrap();

    // Seeder local (sin DHT/trackers) escuchando en loopback: por defecto
    // `SessionOptions.listen` es `None`, así que hay que habilitarlo explícitamente
    // para que exista un `listen_addr` al que el cliente conectarse.
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
    // `AddTorrent::from_local_filename` decodifica un `.torrent`, NO crea uno a
    // partir de un archivo de medios. Para sembrar un archivo local hay que usar
    // `create_and_serve_torrent`, que construye el metainfo y lo agrega a la sesión.
    let (_seed_meta, seeder_handle) = seeder
        .create_and_serve_torrent(&file, CreateTorrentOptions::default())
        .await
        .unwrap();
    seeder_handle.wait_until_initialized().await.unwrap();
    let info_hash = seeder_handle.info_hash().as_string();

    // Cliente sin DHT ni trackers: solo conoce al seeder por `initial_peers`.
    let client_dir = dir.path().join("client");
    std::fs::create_dir_all(&client_dir).unwrap();
    let client: Arc<Session> = Session::new_with_opts(client_dir.clone(), offline_session_opts())
        .await
        .unwrap();

    let addr: SocketAddr = seeder.listen_addr().expect("el seeder debe tener listener local");
    let opts = AddTorrentOptions {
        output_folder: Some(client_dir.to_string_lossy().into_owned()),
        overwrite: true,
        initial_peers: Some(vec![addr]),
        ..Default::default()
    };
    let resp = client
        .add_torrent(
            AddTorrent::from_url(format!("magnet:?xt=urn:btih:{info_hash}")),
            Some(opts),
        )
        .await
        .unwrap();
    let handle = resp.into_handle().unwrap();
    handle.wait_until_initialized().await.unwrap();

    let (file_id, name, len) = pick_largest_video(&handle).unwrap();
    assert!(name.to_string_lossy().ends_with(".mkv"));
    assert_eq!(len, payload.len() as u64);

    let mut stream = torrent_stream(&handle, file_id).await.unwrap();
    let mut head = vec![0u8; 4096];
    stream.read_exact(&mut head).await.unwrap();
    assert_eq!(head, payload[..4096]);
}

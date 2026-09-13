//! Spike: confirma que la API de librqbit 9.x que asumimos existe y compila.
//!
//! HALLAZGO (librqbit 9.0.1): `FileStream` NO es nombrable desde fuera del crate.
//!
//! - `librqbit::FileStream` no está re-exportado en la raíz (E0425).
//! - `librqbit::torrent_state::FileStream` tampoco: `torrent_state` es un módulo
//!   privado (`mod torrent_state;` en lib.rs:75) y `FileStream` (pub en
//!   `torrent_state::streaming`) solo se re-exporta *dentro* de ese módulo privado
//!   (`torrent_state/mod.rs:56`), no hacia afuera (E0603).
//!
//! Por eso el chequeo de traits se hace por *inferencia* sobre el valor que devuelve
//! `ManagedTorrent::stream`, sin nombrar el tipo. El engine (tareas posteriores)
//! deberá exponer el stream como `impl AsyncRead + AsyncSeek + ...` o como
//! `Box<dyn AsyncRead + AsyncSeek + ...>`, nunca como `FileStream` concreto.
use std::sync::Arc;

/// Compila => `ManagedTorrent::stream(Arc<Self>, usize)` existe y su salida
/// implementa `AsyncRead + AsyncSeek + Send + Unpin`. Nunca se ejecuta (solo typecheck).
#[allow(dead_code)]
async fn _managed_torrent_stream_is_seekable(mt: Arc<librqbit::ManagedTorrent>) {
    fn assert_read_seek<T: tokio::io::AsyncRead + tokio::io::AsyncSeek + Send + Unpin>(_: T) {}
    let stream = mt.stream(0).await.unwrap();
    assert_read_seek(stream);
}

#[tokio::test]
async fn session_builds_and_returns_arc() {
    // `Session::new_with_opts(PathBuf, SessionOptions) -> Arc<Session>`.
    let dir = tempfile::tempdir().unwrap();
    let session: Arc<librqbit::Session> =
        librqbit::Session::new_with_opts(dir.path().to_path_buf(), Default::default())
            .await
            .unwrap();
    drop(session);
}

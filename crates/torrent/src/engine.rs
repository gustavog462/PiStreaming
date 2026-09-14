//! Engine de torrents sobre librqbit: sesión única, add_magnet, selección y stream.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use librqbit::{AddTorrent, AddTorrentOptions, ManagedTorrent, Session, SessionOptions};

use pistreaming_core::error::CoreError;

/// Alias local del handle de un torrent.
///
/// `librqbit::ManagedTorrentHandle` NO está re-exportado en la raíz del crate
/// (vive en el módulo privado `torrent_state`), pero su definición es exactamente
/// `Arc<ManagedTorrent>` (`torrent_state/mod.rs:649`) y `librqbit::ManagedTorrent`
/// sí es público, así que este alias es el mismo tipo.
type ManagedTorrentHandle = Arc<ManagedTorrent>;

/// Abre la sesión de librqbit atada (opcionalmente) a una interfaz de salida.
pub async fn open_session(
    cache_dir: PathBuf,
    egress_bind: Option<String>,
) -> Result<Arc<Session>, CoreError> {
    std::fs::create_dir_all(&cache_dir).ok();
    let opts = SessionOptions { bind_device_name: egress_bind, ..Default::default() };
    Session::new_with_opts(cache_dir, opts)
        .await
        .map_err(|e| CoreError::Other(format!("no se pudo abrir la sesión torrent: {e}")))
}

pub struct AddedTorrent {
    pub handle: ManagedTorrentHandle,
    pub info_hash: String,
}

/// Agrega un magnet y espera a que la metadata esté resuelta.
pub async fn add_magnet(
    session: &Arc<Session>,
    magnet: &str,
    output_folder: &Path,
) -> Result<AddedTorrent, CoreError> {
    let opts = AddTorrentOptions {
        // En librqbit 9.0.1 `output_folder` es `Option<String>`, no `PathBuf`.
        output_folder: Some(output_folder.to_string_lossy().into_owned()),
        overwrite: true,
        ..Default::default()
    };
    let resp = session
        .add_torrent(AddTorrent::from_url(magnet.to_string()), Some(opts))
        .await
        .map_err(|e| CoreError::Other(format!("add_torrent falló: {e}")))?;
    let handle = resp
        .into_handle()
        .ok_or_else(|| CoreError::Other("respuesta de add_torrent sin handle".into()))?;
    handle
        .wait_until_initialized()
        .await
        .map_err(|e| CoreError::Other(format!("metadata no resuelta: {e}")))?;
    // `Id20` implementa `as_string()` (hex) pero no `Display`, así que no hay `to_string()`.
    let info_hash = handle.info_hash().as_string();
    Ok(AddedTorrent { handle, info_hash })
}

/// Devuelve `(file_id, nombre relativo, len)` del video más grande del torrent.
pub fn pick_largest_video(handle: &ManagedTorrentHandle) -> Result<(usize, PathBuf, u64), CoreError> {
    let picked = handle
        .with_metadata(|m| {
            let files: Vec<(usize, String, u64)> = m
                .file_infos
                .iter()
                .enumerate()
                .map(|(i, f)| (i, f.relative_filename.to_string_lossy().into_owned(), f.len))
                .collect();
            choose_file_id(&files, None)
                .and_then(|id| files.into_iter().find(|(i, _, _)| *i == id))
                .map(|(id, name, len)| (id, PathBuf::from(name), len))
        })
        .map_err(|e| CoreError::Other(format!("metadata no disponible: {e}")))?;
    picked.ok_or_else(|| CoreError::NotFound("el torrent no tiene archivos de video".into()))
}

/// Devuelve la ruta local del video en el índice `idx` del torrent. `None` si
/// `idx` no corresponde a un video válido (no-video o fuera de rango).
pub fn pick_video_by_index(
    handle: &ManagedTorrentHandle,
    idx: usize,
) -> Result<Option<PathBuf>, CoreError> {
    handle
        .with_metadata(|m| {
            let files: Vec<(usize, String, u64)> = m
                .file_infos
                .iter()
                .enumerate()
                .map(|(i, f)| (i, f.relative_filename.to_string_lossy().into_owned(), f.len))
                .collect();
            choose_file_id(&files, Some(idx))
                .filter(|id| *id == idx)
                .and_then(|id| files.into_iter().find(|(i, _, _)| *i == id))
                .map(|(_, name, _)| PathBuf::from(name))
        })
        .map_err(|e| CoreError::Other(format!("metadata no disponible: {e}")))
}

/// Elige un `file_id` de video. Si `requested` apunta a un video válido, lo usa;
/// si no (ausente, fuera de rango o no-video), cae al video de mayor `len`.
fn choose_file_id(files: &[(usize, String, u64)], requested: Option<usize>) -> Option<usize> {
    if let Some(idx) = requested {
        if let Some((id, _, _)) = files
            .iter()
            .find(|(i, name, _)| *i == idx && is_video_name(name))
        {
            return Some(*id);
        }
    }
    files
        .iter()
        .filter(|(_, name, _)| is_video_name(name))
        .max_by_key(|(_, _, len)| *len)
        .map(|(id, _, _)| *id)
}

fn is_video_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [".mkv", ".mp4", ".m4v", ".avi", ".webm", ".mov", ".ts"]
        .iter()
        .any(|ext| n.ends_with(ext))
}

/// Abre el stream de bytes de un archivo del torrent.
///
/// Devuelve `impl AsyncRead + AsyncSeek + Send + Unpin + 'static` porque el tipo
/// concreto (`librqbit::FileStream`) NO es nombrable fuera del crate
/// (`librqbit::torrent_state` es módulo privado; lo confirmó el spike de la Task 1).
/// Esos bounds SON el contrato: el consumidor (`api`, Tasks 6/9) los toma
/// genéricamente. Si alguna vez hay que unificar ramas, boxear como
/// `Box<dyn AsyncRead + AsyncSeek + Send + Unpin>` (no `Pin<Box<...>>`).
pub async fn torrent_stream(
    handle: &ManagedTorrentHandle,
    file_id: usize,
) -> Result<
    impl tokio::io::AsyncRead + tokio::io::AsyncSeek + Send + Unpin + 'static,
    CoreError,
> {
    handle
        .clone()
        .stream(file_id)
        .await
        .map_err(|e| CoreError::Other(format!("no se pudo abrir el stream: {e}")))
}

#[cfg(test)]
mod tests {
    use super::choose_file_id;

    fn f(id: usize, name: &str, len: u64) -> (usize, String, u64) {
        (id, name.to_string(), len)
    }

    #[test]
    fn elige_el_requested_valido() {
        let files = vec![f(0, "a.mkv", 100), f(1, "b.mp4", 900), f(2, "subs.srt", 1)];
        assert_eq!(choose_file_id(&files, Some(0)), Some(0));
    }

    #[test]
    fn sin_requested_elige_el_mayor() {
        let files = vec![f(0, "a.mkv", 100), f(1, "b.mp4", 900), f(2, "c.avi", 300)];
        assert_eq!(choose_file_id(&files, None), Some(1));
    }

    #[test]
    fn ignora_requested_no_video_o_fuera_de_rango() {
        let files = vec![f(0, "a.mkv", 100), f(1, "b.mp4", 900), f(2, "subs.srt", 50)];
        // No-video: cae al mayor.
        assert_eq!(choose_file_id(&files, Some(2)), Some(1));
        // Fuera de rango: cae al mayor.
        assert_eq!(choose_file_id(&files, Some(99)), Some(1));
    }

    #[test]
    fn none_si_no_hay_videos() {
        let files = vec![f(0, "subs.srt", 10), f(1, "readme.txt", 5)];
        assert_eq!(choose_file_id(&files, None), None);
        assert_eq!(choose_file_id(&files, Some(0)), None);
    }
}

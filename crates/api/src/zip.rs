//! Descarga de un torrent como ZIP al vuelo (streaming, sin fichero temporal).
//!
//! El `zip` writer es síncrono y no necesita `Seek` (`ZipWriter::new_stream`),
//! así que corre en un hilo bloqueante y manda los bytes por un canal acotado
//! al body HTTP. Los datos de cada fichero se leen del stream del torrent.

use std::io::{Read, Write};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use pistreaming_core::error::CoreError;
use tokio::sync::mpsc;

use crate::recovery;
use crate::SharedState;

const CHANNEL: usize = 8;

/// `Write` que manda los bytes al body HTTP por un canal acotado.
struct ChannelWriter {
    tx: mpsc::Sender<Result<Bytes, std::io::Error>>,
    rt: tokio::runtime::Handle,
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let bytes = Bytes::copy_from_slice(buf);
        self.rt
            .block_on(self.tx.send(Ok(bytes)))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "cliente desconectado"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Adapta un `AsyncRead` (stream del torrent) a `std::io::Read` para el ZIP.
struct AsyncReadAdapter {
    stream: Box<dyn tokio::io::AsyncRead + Unpin + Send>,
    rt: tokio::runtime::Handle,
}

impl Read for AsyncReadAdapter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.rt
            .block_on(tokio::io::AsyncReadExt::read(&mut self.stream, buf))
    }
}

/// Normaliza el nombre de una entrada del ZIP (sin `..`, sin rutas absolutas).
fn sanitize_name(name: &str) -> Option<String> {
    let name = name.replace('\\', "/");
    let parts: Vec<&str> = name
        .split('/')
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn sanitize_filename(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | '"' | '\n' | '\r' | '\t' => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "pistreaming".to_string()
    } else {
        cleaned.chars().take(120).collect()
    }
}

/// GET /api/zip/:session — descarga el torrent de la sesión como ZIP.
pub async fn download(State(st): State<SharedState>, Path(session): Path<String>) -> Response {
    let sess = match recovery::ensure_session(&st, &session).await {
        Ok(s) => s,
        Err(e) => return crate::err(StatusCode::NOT_FOUND, &e),
    };
    let handle = match recovery::ensure_handle(&st, &session).await {
        Ok(h) => h,
        Err(e) => return crate::err(StatusCode::NOT_FOUND, &e),
    };
    let title = {
        let g = sess.read();
        g.title.clone().unwrap_or_else(|| g.id.clone())
    };
    let files: Vec<(usize, String)> = match handle.with_metadata(|m| {
        m.file_infos
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                sanitize_name(&f.relative_filename.to_string_lossy()).map(|n| (i, n))
            })
            .collect::<Vec<_>>()
    }) {
        Ok(f) => f,
        Err(e) => {
            return crate::core_err(CoreError::Other(format!("metadata no disponible: {e}")))
        }
    };
    if files.is_empty() {
        return crate::err(StatusCode::NOT_FOUND, "el torrent no tiene archivos");
    }

    let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(CHANNEL);
    let rt = tokio::runtime::Handle::current();
    let handle2 = handle.clone();
    tokio::task::spawn_blocking(move || {
        let writer = ChannelWriter { tx, rt: rt.clone() };
        let mut zip = zip::ZipWriter::new_stream(writer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (file_id, name) in files {
            let stream = match rt.block_on(pistreaming_torrent::torrent_stream(&handle2, file_id)) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, file = %name, "no se pudo abrir el archivo para el ZIP");
                    continue;
                }
            };
            let mut reader = AsyncReadAdapter {
                stream: Box::new(stream),
                rt: rt.clone(),
            };
            if let Err(e) = zip.start_file(name.clone(), options) {
                tracing::warn!(error = %e, file = %name, "no se pudo añadir al ZIP");
                continue;
            }
            if let Err(e) = std::io::copy(&mut reader, &mut zip) {
                tracing::warn!(error = %e, file = %name, "no se pudo copiar al ZIP");
            }
        }
        if let Err(e) = zip.finish() {
            tracing::warn!(error = %e, "no se pudo cerrar el ZIP");
        }
    });

    let stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    let filename = format!("{}.zip", sanitize_filename(&title));
    let mut resp = Response::new(Body::from_stream(stream));
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    resp.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            .unwrap_or(HeaderValue::from_static("attachment")),
    );
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitiza_nombres_de_entrada() {
        assert_eq!(sanitize_name("a/b.mkv"), Some("a/b.mkv".into()));
        assert_eq!(sanitize_name("../etc/passwd"), Some("etc/passwd".into()));
        assert_eq!(sanitize_name("/abs/x"), Some("abs/x".into()));
        assert_eq!(sanitize_name(".."), None);
        assert_eq!(sanitize_name(""), None);
    }

    #[test]
    fn sanitiza_nombre_de_fichero() {
        assert_eq!(sanitize_filename("The Matrix"), "The Matrix");
        assert_eq!(sanitize_filename("a/b\"c"), "a_b_c");
        assert_eq!(sanitize_filename("   "), "pistreaming");
    }
}

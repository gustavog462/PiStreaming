//! Sirve un fichero que aún se está escribiendo (transcode progresivo).
//!
//! A diferencia de `range::ranged_response` (fichero completo y estático), aquí
//! el largo total no se conoce: el fichero crece mientras ffmpeg escribe. Se
//! sirve en streaming y, al llegar al final actual, se espera a que crezca
//! hasta que el transcode termine (`done`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const WAIT_FILE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);
const CHUNK: usize = 64 * 1024;

/// Rango pedido sobre un fichero creciente.
#[derive(Debug, PartialEq, Eq)]
pub enum GrowingRange {
    Full,
    Partial { start: u64, end: Option<u64> },
    Invalid,
}

/// Parsea `Range: bytes=...` sin conocer el largo total.
pub fn parse_range(header_value: Option<&str>) -> GrowingRange {
    let Some(v) = header_value else {
        return GrowingRange::Full;
    };
    let Some(spec) = v.trim().strip_prefix("bytes=") else {
        return GrowingRange::Invalid;
    };
    let Some((s, e)) = spec.split_once('-') else {
        return GrowingRange::Invalid;
    };
    let start = if s.trim().is_empty() {
        // Sufijo (`bytes=-N`) no tiene sentido en un fichero creciente.
        return GrowingRange::Invalid;
    } else {
        match s.trim().parse::<u64>() {
            Ok(n) => n,
            Err(_) => return GrowingRange::Invalid,
        }
    };
    let end = if e.trim().is_empty() {
        None
    } else {
        match e.trim().parse::<u64>() {
            Ok(n) if n >= start => Some(n),
            _ => return GrowingRange::Invalid,
        }
    };
    GrowingRange::Partial { start, end }
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, msg.to_string()).into_response()
}

/// Sirve `path` (creciente) con soporte de Range. `done` indica que el
/// productor ha terminado (no habrá más bytes).
pub async fn serve(
    path: PathBuf,
    done: Arc<AtomicBool>,
    headers: HeaderMap,
    mime: &str,
) -> Response {
    // 1) Esperar a que el productor cree el fichero.
    let mut waited = Duration::ZERO;
    while !path.exists() {
        if done.load(Ordering::Relaxed) {
            return err(StatusCode::NOT_FOUND, "el transcode no generó el fichero");
        }
        if waited >= WAIT_FILE {
            return err(
                StatusCode::GATEWAY_TIMEOUT,
                "timeout esperando el transcode",
            );
        }
        tokio::time::sleep(POLL).await;
        waited += POLL;
    }

    let range = parse_range(headers.get(header::RANGE).and_then(|v| v.to_str().ok()));
    let (start, end) = match range {
        GrowingRange::Invalid => {
            let mut h = HeaderMap::new();
            h.insert(header::CONTENT_RANGE, HeaderValue::from_static("bytes */0"));
            return (StatusCode::RANGE_NOT_SATISFIABLE, h).into_response();
        }
        GrowingRange::Full => (0u64, None),
        GrowingRange::Partial { start, end } => (start, end),
    };

    // 2) Si el rango empieza más allá de lo escrito, esperar a que crezca.
    let mut waited = Duration::ZERO;
    loop {
        let len = match tokio::fs::metadata(&path).await {
            Ok(m) => m.len(),
            Err(e) => {
                return err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("no se pudo medir: {e}"),
                )
            }
        };
        if start < len || done.load(Ordering::Relaxed) {
            if start >= len {
                let mut h = HeaderMap::new();
                h.insert(header::CONTENT_RANGE, HeaderValue::from_static("bytes */0"));
                return (StatusCode::RANGE_NOT_SATISFIABLE, h).into_response();
            }
            break;
        }
        if waited >= WAIT_FILE {
            return err(
                StatusCode::GATEWAY_TIMEOUT,
                "timeout esperando datos del transcode",
            );
        }
        tokio::time::sleep(POLL).await;
        waited += POLL;
    }

    // 3) Abrir y posicionar.
    let mut file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(e) => {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("no se pudo abrir: {e}"),
            )
        }
    };
    if start > 0 {
        if let Err(e) = file.seek(std::io::SeekFrom::Start(start)).await {
            return err(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("seek falló: {e}"),
            );
        }
    }

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    resp_headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));

    let status = if start == 0 && end.is_none() {
        StatusCode::OK
    } else {
        // Largo total desconocido: `*` en Content-Range.
        let cr = match end {
            Some(e) => format!("bytes {start}-{e}/*"),
            None => format!("bytes {start}-/*"),
        };
        resp_headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&cr).unwrap_or(HeaderValue::from_static("bytes 0-/*")),
        );
        StatusCode::PARTIAL_CONTENT
    };

    // 4) Stream que lee hasta EOF y, si no ha terminado, espera a que crezca.
    struct State {
        file: tokio::fs::File,
        buf: Vec<u8>,
        remaining: Option<u64>,
        done: Arc<AtomicBool>,
    }
    let state = State {
        file,
        buf: vec![0u8; CHUNK],
        remaining: end.map(|e| e - start + 1),
        done,
    };
    let stream = futures::stream::unfold(state, |mut st| async move {
        loop {
            if st.remaining == Some(0) {
                return None;
            }
            match st.file.read(&mut st.buf).await {
                Ok(0) => {
                    if st.done.load(Ordering::Relaxed) {
                        return None;
                    }
                    tokio::time::sleep(POLL).await;
                }
                Ok(n) => {
                    let take = match st.remaining {
                        Some(r) => (r.min(n as u64)) as usize,
                        None => n,
                    };
                    let chunk = bytes::Bytes::copy_from_slice(&st.buf[..take]);
                    if let Some(r) = st.remaining.as_mut() {
                        *r -= take as u64;
                    }
                    return Some((Ok::<_, std::io::Error>(chunk), st));
                }
                Err(e) => return Some((Err(e), st)),
            }
        }
    });

    (status, resp_headers, Body::from_stream(stream)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_cabecera_es_full() {
        assert_eq!(parse_range(None), GrowingRange::Full);
    }

    #[test]
    fn rango_abierto_y_cerrado() {
        assert_eq!(
            parse_range(Some("bytes=10-")),
            GrowingRange::Partial {
                start: 10,
                end: None
            }
        );
        assert_eq!(
            parse_range(Some("bytes=0-4")),
            GrowingRange::Partial {
                start: 0,
                end: Some(4)
            }
        );
    }

    #[test]
    fn sufijo_y_basura_son_invalidos() {
        assert_eq!(parse_range(Some("bytes=-20")), GrowingRange::Invalid);
        assert_eq!(parse_range(Some("items=0-4")), GrowingRange::Invalid);
        assert_eq!(parse_range(Some("bytes=abc")), GrowingRange::Invalid);
        assert_eq!(parse_range(Some("bytes=5-1")), GrowingRange::Invalid);
    }

    #[tokio::test]
    async fn sirve_fichero_que_crece() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("growing.bin");
        let done = Arc::new(AtomicBool::new(false));
        std::fs::write(&path, b"hola").unwrap();

        // Productor: añade " mundo" a los 150 ms y marca done.
        let p = path.clone();
        let d = done.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
            f.write_all(b" mundo").unwrap();
            d.store(true, Ordering::Relaxed);
        });

        let resp = serve(path, done, HeaderMap::new(), "application/octet-stream").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"hola mundo");
    }

    #[tokio::test]
    async fn rango_parcial_sobre_fichero_creciente() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("growing.bin");
        let done = Arc::new(AtomicBool::new(true));
        std::fs::write(&path, b"0123456789").unwrap();
        let mut h = HeaderMap::new();
        h.insert(header::RANGE, HeaderValue::from_static("bytes=2-5"));
        let resp = serve(path, done, h, "application/octet-stream").await;
        assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(&body[..], b"2345");
    }
}

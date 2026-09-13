//! Parseo de `Range` y respuesta HTTP con soporte de bytes sobre un reader seekable.

use std::io::SeekFrom;

use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};
use tokio_util::io::ReaderStream;

#[derive(Debug, PartialEq, Eq)]
pub enum RangeOutcome {
    /// Sin cabecera `Range`: servir el archivo completo.
    Full,
    /// Rango válido, `end_excl` exclusivo.
    Partial { start: u64, end_excl: u64 },
    /// Cabecera presente pero no satisfacible → 416.
    Invalid,
}

/// Parsea `Range: bytes=...` contra un largo conocido. Pura.
pub fn parse_range(header_value: Option<&str>, len: u64) -> RangeOutcome {
    let Some(v) = header_value else { return RangeOutcome::Full };
    let Some(spec) = v.trim().strip_prefix("bytes=") else { return RangeOutcome::Invalid };
    let Some((s, e)) = spec.split_once('-') else { return RangeOutcome::Invalid };
    if len == 0 {
        return RangeOutcome::Invalid;
    }
    let (start, end_excl) = if s.is_empty() {
        // sufijo: bytes=-N (últimos N)
        let Ok(n) = e.trim().parse::<u64>() else { return RangeOutcome::Invalid };
        if n == 0 {
            return RangeOutcome::Invalid;
        }
        (len.saturating_sub(n), len)
    } else {
        let Ok(start) = s.trim().parse::<u64>() else { return RangeOutcome::Invalid };
        let end_excl = if e.trim().is_empty() {
            len
        } else {
            match e.trim().parse::<u64>() {
                Ok(last) => last.saturating_add(1),
                Err(_) => return RangeOutcome::Invalid,
            }
        };
        (start, end_excl)
    };
    if start >= len || end_excl > len || start >= end_excl {
        return RangeOutcome::Invalid;
    }
    RangeOutcome::Partial { start, end_excl }
}

/// Construye la respuesta (200/206/416) sirviendo bytes de `reader`.
pub async fn ranged_response<R>(
    mut reader: R,
    len: u64,
    headers: &HeaderMap,
    mime: &str,
) -> Response
where
    R: AsyncRead + AsyncSeek + Send + Unpin + 'static,
{
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    resp_headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream")));

    match parse_range(range, len) {
        RangeOutcome::Invalid => {
            resp_headers.insert(header::CONTENT_RANGE, HeaderValue::from_static("bytes */0"));
            (StatusCode::RANGE_NOT_SATISFIABLE, resp_headers).into_response()
        }
        RangeOutcome::Full => {
            resp_headers.insert(header::CONTENT_LENGTH, num(len));
            let body = Body::from_stream(ReaderStream::with_capacity(reader, 65536));
            (StatusCode::OK, resp_headers, body).into_response()
        }
        RangeOutcome::Partial { start, end_excl } => {
            let to_take = end_excl - start;
            if reader.seek(SeekFrom::Start(start)).await.is_err() {
                return (StatusCode::INTERNAL_SERVER_ERROR, "seek falló").into_response();
            }
            resp_headers.insert(header::CONTENT_LENGTH, num(to_take));
            resp_headers.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {}-{}/{}", start, end_excl - 1, len))
                    .unwrap_or(HeaderValue::from_static("bytes */0")),
            );
            let body = Body::from_stream(ReaderStream::with_capacity(reader.take(to_take), 65536));
            (StatusCode::PARTIAL_CONTENT, resp_headers, body).into_response()
        }
    }
}

fn num(n: u64) -> HeaderValue {
    HeaderValue::from_str(&n.to_string()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_cabecera_full() {
        assert_eq!(parse_range(None, 100), RangeOutcome::Full);
    }

    #[test]
    fn rango_abierto() {
        assert_eq!(parse_range(Some("bytes=10-"), 100), RangeOutcome::Partial { start: 10, end_excl: 100 });
    }

    #[test]
    fn rango_cerrado_es_inclusivo_en_origen() {
        // bytes=0-4 => 5 bytes (end exclusivo 5)
        assert_eq!(parse_range(Some("bytes=0-4"), 100), RangeOutcome::Partial { start: 0, end_excl: 5 });
    }

    #[test]
    fn sufijo() {
        assert_eq!(parse_range(Some("bytes=-20"), 100), RangeOutcome::Partial { start: 80, end_excl: 100 });
    }

    #[test]
    fn fuera_de_rango_es_invalid() {
        assert_eq!(parse_range(Some("bytes=100-"), 100), RangeOutcome::Invalid);
        assert_eq!(parse_range(Some("bytes=0-1000"), 100), RangeOutcome::Invalid);
    }

    #[test]
    fn basura_es_invalid() {
        assert_eq!(parse_range(Some("items=0-4"), 100), RangeOutcome::Invalid);
        assert_eq!(parse_range(Some("bytes=abc"), 100), RangeOutcome::Invalid);
    }
}

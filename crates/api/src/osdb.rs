//! Cliente mínimo de OpenSubtitles para subtítulos externos.
//!
//! Requiere `OPENSUBTITLES_API_KEY` en el entorno. Sin la clave, las rutas de
//! subtítulos externos responden 422 con un mensaje claro (la SPA lo muestra).

use pistreaming_core::error::CoreError;
use serde::Deserialize;

const BASE: &str = "https://api.opensubtitles.com/api/v1";

fn api_key() -> Result<String, CoreError> {
    std::env::var("OPENSUBTITLES_API_KEY")
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            CoreError::Unsupported("OPENSUBTITLES_API_KEY no configurada en el servidor".into())
        })
}

fn client() -> Result<reqwest::Client, CoreError> {
    reqwest::Client::builder()
        .user_agent("PiStreaming/0.1")
        .build()
        .map_err(|e| CoreError::Other(format!("cliente HTTP: {e}")))
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    data: Vec<SearchItem>,
}

#[derive(Deserialize)]
struct SearchItem {
    attributes: SearchAttrs,
}

#[derive(Deserialize)]
struct SearchAttrs {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    release: Option<String>,
    #[serde(default)]
    files: Vec<SubFile>,
}

#[derive(Deserialize)]
struct SubFile {
    file_id: i64,
}

/// Busca subtítulos por IMDb id (`tt…` o numérico) y, opcionalmente, idioma.
pub async fn search(
    imdb_id: &str,
    lang: Option<&str>,
) -> Result<Vec<serde_json::Value>, CoreError> {
    let key = api_key()?;
    let numeric = imdb_id.trim().trim_start_matches("tt");
    let mut url = format!("{BASE}/subtitles?imdb_id={numeric}");
    if let Some(l) = lang {
        url.push_str(&format!("&languages={l}"));
    }
    let resp = client()?
        .get(&url)
        .header("Api-Key", key)
        .send()
        .await
        .map_err(|e| CoreError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(CoreError::Http(format!(
            "OpenSubtitles search: {}",
            resp.status()
        )));
    }
    let body: SearchResponse = resp
        .json()
        .await
        .map_err(|e| CoreError::Json(e.to_string()))?;
    Ok(body
        .data
        .into_iter()
        .filter_map(|item| {
            let file_id = item.attributes.files.first()?.file_id;
            Some(serde_json::json!({
                "file_id": file_id,
                "lang": item.attributes.language,
                "label": item.attributes.release,
            }))
        })
        .collect())
}

/// Idiomas disponibles para un IMDb id (distintos, ordenados).
pub async fn languages(imdb_id: &str) -> Result<Vec<String>, CoreError> {
    let items = search(imdb_id, None).await?;
    let mut langs: Vec<String> = items
        .iter()
        .filter_map(|v| v.get("lang").and_then(|l| l.as_str()).map(str::to_string))
        .collect();
    langs.sort();
    langs.dedup();
    Ok(langs)
}

#[derive(Deserialize)]
struct DownloadResponse {
    link: String,
}

/// Descarga el primer subtítulo del idioma pedido y lo devuelve en VTT.
pub async fn download(imdb_id: &str, lang: &str) -> Result<String, CoreError> {
    let key = api_key()?;
    let items = search(imdb_id, Some(lang)).await?;
    let file_id = items
        .first()
        .and_then(|v| v.get("file_id"))
        .and_then(|v| v.as_i64())
        .ok_or_else(|| CoreError::NotFound("subtítulo no disponible".into()))?;
    let resp = client()?
        .post(format!("{BASE}/download"))
        .header("Api-Key", key)
        .json(&serde_json::json!({ "file_id": file_id }))
        .send()
        .await
        .map_err(|e| CoreError::Http(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(CoreError::Http(format!(
            "OpenSubtitles download: {}",
            resp.status()
        )));
    }
    let dl: DownloadResponse = resp
        .json()
        .await
        .map_err(|e| CoreError::Json(e.to_string()))?;
    let srt = client()?
        .get(&dl.link)
        .send()
        .await
        .map_err(|e| CoreError::Http(e.to_string()))?
        .text()
        .await
        .map_err(|e| CoreError::Http(e.to_string()))?;
    Ok(srt_to_vtt(&srt))
}

/// Descarga el mejor subtítulo: `auto` prueba es → en → cualquiera.
/// Devuelve `(vtt, idioma_real)`.
pub async fn download_auto(imdb_id: &str, lang: &str) -> Result<(String, String), CoreError> {
    if lang != "auto" {
        let vtt = download(imdb_id, lang).await?;
        return Ok((vtt, lang.to_string()));
    }
    for candidate in ["es", "en"] {
        if let Ok(vtt) = download(imdb_id, candidate).await {
            return Ok((vtt, candidate.to_string()));
        }
    }
    // Cualquier idioma disponible.
    let langs = languages(imdb_id).await?;
    for candidate in langs {
        if let Ok(vtt) = download(imdb_id, &candidate).await {
            return Ok((vtt, candidate));
        }
    }
    Err(CoreError::NotFound("subtítulo no disponible".into()))
}

/// Convierte SRT a WebVTT (suficiente para el player).
pub fn srt_to_vtt(srt: &str) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for line in srt.lines() {
        if line.trim().is_empty() {
            out.push('\n');
            continue;
        }
        if line.contains("-->") {
            out.push_str(&line.replace(',', "."));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_a_vtt_convierte_timestamps() {
        let srt =
            "1\n00:00:01,000 --> 00:00:02,500\nHola\n\n2\n00:00:03,000 --> 00:00:04,000\nMundo\n";
        let vtt = srt_to_vtt(srt);
        assert!(vtt.starts_with("WEBVTT\n\n"));
        assert!(vtt.contains("00:00:01.000 --> 00:00:02.500"));
        assert!(vtt.contains("Hola"));
        assert!(vtt.contains("Mundo"));
    }

    #[test]
    fn sin_api_key_es_unsupported() {
        let e =
            CoreError::Unsupported("OPENSUBTITLES_API_KEY no configurada en el servidor".into());
        assert!(e.to_string().contains("OPENSUBTITLES_API_KEY"));
    }
}

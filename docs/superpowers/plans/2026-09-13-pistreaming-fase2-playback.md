# PiStreaming — Fase 2 (Playback) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use omo-subagent-driven-development (recommended) or omo-dispatching-parallel-agents to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reproducir un torrent de punta a punta en el navegador: `POST /api/play` (magnet → ffprobe → plan) y `GET /stream/:session` con Range, más progreso, eviction de caché y un player mínimo.

**Architecture:** `librqbit` 9.x sirve los bytes de un archivo del torrent vía `FileStream` (seekable, bloquea hasta tener la pieza). Un endpoint `/raw/:session` expone ese stream con Range; `/stream/:session` o bien lo sirve directo (plan `direct`), o bien lo remuxea a fMP4 con ffmpeg y sirve el archivo creciente (`remux`/`recodeaudio`). Las sesiones viven en un registro en memoria; el progreso se guarda con clave compuesta `"{type}:{id}"` reutilizando la tabla `progress` existente.

**Tech Stack:** Rust 2021, `librqbit` 9.x (edition 2024 → Rust ≥ 1.85), `tokio`, `axum` 0.7 (se mantiene; se convive con el axum 0.8 interno de librqbit), `tokio-util` (`io`), `ffmpeg`/`ffprobe` subprocess, `rusqlite`, `wiremock`.

---

## Ajustes al spec detectados en el anclaje (decisiones que el plan sí implementa)

1. **Los traits `TorrentEngine`/`MediaEngine` NO existen y no se crean** (YAGNI). `torrent` y `media` exponen funciones/structs concretos; `api` depende de ellos. La testabilidad se logra con funciones puras (`decide`, `parse_range`) y un seeder local real (integración), no con dobles.
2. **`PlaybackRoute` se extiende** con `RecodeAudio`; `Unsupported` NO es una ruta sino un error `CoreError::Unsupported(String)` → HTTP 422.
3. **Clave de progreso compuesta sin migrar**: la tabla `progress` ya tiene PK `id TEXT`; el handler guarda con `id = format!("{kind}:{id}")`.
4. **Se mantiene `axum` 0.7** (librqbit 9 usa 0.8 internamente; ambas versiones coexisten, no se pasan tipos entre ellas). `Body::from_stream` existe en 0.7.
5. **Player sin dependencia nueva**: `include_str!("../assets/player.html")` en vez de `rust-embed`.
6. **`/raw/:session` es la fuente seekable para ffmpeg** (evita las limitaciones de `pipe:0` con input no-seekable).

---

## Estructura de archivos

**Crear**
- `crates/media/src/decide.rs` — tipo `Probe` + `decide()` pura (tabla codecs→plan).
- `crates/media/src/probe.rs` — `ffprobe` → `Probe`.
- `crates/media/src/ffmpeg.rs` — `remux()` / `recode_audio()` (spawn ffmpeg).
- `crates/torrent/src/engine.rs` — `open_session`, `add_magnet`, `pick_largest_video`, `torrent_stream`.
- `crates/api/src/range.rs` — `parse_range` + `ranged_response`.
- `crates/api/src/session.rs` — `PlaySessionRegistry` + `PlaySession`.
- `crates/api/assets/player.html` — player mínimo.
- `crates/api/tests/playback.rs` — integración end-to-end local (seeder propio).

**Modificar**
- `Cargo.toml` (workspace) — `librqbit`, `tokio-util`, `bytes`, `http`.
- `crates/core/src/playback.rs` — `PlaybackRoute::RecodeAudio`.
- `crates/core/src/error.rs` — `CoreError::Unsupported`.
- `crates/torrent/src/lib.rs`, `crates/media/src/lib.rs` — módulos + re-exports.
- `crates/torrent/Cargo.toml`, `crates/media/Cargo.toml`, `crates/api/Cargo.toml`, `crates/server/Cargo.toml`.
- `crates/api/src/lib.rs` — rutas nuevas + `AppState` + handlers.
- `crates/server/src/main.rs` — wiring de engines + job de eviction.
- `crates/media/src/lib.rs` — re-export de `decide`/`probe`.

---

## Task 1: Dependencias + spike de la API de librqbit

**Files:**
- Modify: `Cargo.toml`, `crates/torrent/Cargo.toml`, `crates/api/Cargo.toml`
- Create: `crates/torrent/tests/api_spike.rs`

- [ ] **Step 1: Agregar deps al workspace**

En `Cargo.toml` (`[workspace.dependencies]`) agregar:

```toml
librqbit = "9"
tokio-util = { version = "0.7", features = ["io"] }
bytes = "1"
http = "1"
```

- [ ] **Step 2: Declararlas en las crates**

`crates/torrent/Cargo.toml` → sumar a `[dependencies]`:

```toml
librqbit.workspace = true
tokio.workspace = true
anyhow.workspace = true
tracing.workspace = true
tokio-util.workspace = true
futures.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

`crates/api/Cargo.toml` → sumar a `[dependencies]`:

```toml
pistreaming-torrent = { path = "../torrent" }
pistreaming-media = { path = "../media" }
bytes.workspace = true
http.workspace = true
tokio-util.workspace = true
futures.workspace = true
```

- [ ] **Step 3: Compilar y resolver features**

Run: `cd ~/Proyectos/PiStreaming && cargo build -p pistreaming-torrent 2>&1 | tail -30`
Expected: compila. Si falla por features de `librqbit`, inspeccionar con `cargo info librqbit@9.0.1` (sección Features) y ajustar (`default-features = false` + features necesarias, p. ej. `streaming`). Anotar la decisión como comentario en `Cargo.toml`.

- [ ] **Step 4: Escribir el spike (verifica firmas reales)**

`crates/torrent/tests/api_spike.rs`:

```rust
//! Spike: confirma que la API de librqbit 9.x que asumimos existe y compila.
use std::sync::Arc;

#[tokio::test]
async fn file_stream_is_reexported_and_seekable() {
    // Si esto compila, `librqbit::FileStream` es público y cumple AsyncRead+AsyncSeek.
    fn assert_read_seek<T: tokio::io::AsyncRead + tokio::io::AsyncSeek + Send + Unpin>() {}
    assert_read_seek::<librqbit::FileStream>();

    // `Session` se crea con new_with_opts y devuelve Arc<Session>.
    let dir = tempfile::tempdir().unwrap();
    let session: Arc<librqbit::Session> =
        librqbit::Session::new_with_opts(dir.path().to_path_buf(), Default::default())
            .await
            .unwrap();
    drop(session);
}
```

- [ ] **Step 5: Correr el spike**

Run: `cargo test -p pistreaming-torrent --test api_spike -- --nocapture`
Expected: PASS. Si `librqbit::FileStream` no está re-exportado, usar `librqbit::torrent_state::FileStream` (ajustar el import) y anotarlo.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/torrent/Cargo.toml crates/api/Cargo.toml crates/torrent/tests/api_spike.rs
git commit -m "chore(fase2): deps de librqbit/tokio-util y spike de API"
```

---

## Task 2: `core` — `PlaybackRoute::RecodeAudio` y `CoreError::Unsupported`

**Files:**
- Modify: `crates/core/src/playback.rs`, `crates/core/src/error.rs`
- Test: tests inline (`#[cfg(test)]`) en ambos archivos

- [ ] **Step 1: Test que falla (serde de la ruta nueva)**

En `crates/core/src/playback.rs` al final:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recode_audio_serializa_en_minusculas() {
        let j = serde_json::to_string(&PlaybackRoute::RecodeAudio).unwrap();
        assert_eq!(j, "\"recodeaudio\"");
        let back: PlaybackRoute = serde_json::from_str(&j).unwrap();
        assert_eq!(back, PlaybackRoute::RecodeAudio);
    }
}
```

- [ ] **Step 2: Correr y ver fallar**

Run: `cargo test -p pistreaming-core playback::tests::recode_audio_serializa_en_minusculas`
Expected: FAIL (`no variant named RecodeAudio`).

- [ ] **Step 3: Extender el enum**

En `crates/core/src/playback.rs`, dentro de `enum PlaybackRoute`:

```rust
    /// Remux MKV→fMP4 (cambio de contenedor, sin recodificar video).
    Remux,
    /// Remux + recodifica SOLO el audio a AAC (video `-c copy`).
    RecodeAudio,
```

- [ ] **Step 4: Test que falla (variante de error)**

En `crates/core/src/error.rs` al final:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_lleva_mensaje() {
        let e = CoreError::Unsupported("video codec no soportado: mpeg2video".into());
        assert_eq!(e.to_string(), "no soportado: video codec no soportado: mpeg2video");
    }
}
```

- [ ] **Step 5: Correr y ver fallar**

Run: `cargo test -p pistreaming-core error::tests::unsupported_lleva_mensaje`
Expected: FAIL (`no variant named Unsupported`).

- [ ] **Step 6: Agregar la variante**

En `crates/core/src/error.rs`, dentro de `enum CoreError`, antes de `Other`:

```rust
    #[error("no soportado: {0}")]
    Unsupported(String),
```

- [ ] **Step 7: Correr y ver pasar**

Run: `cargo test -p pistreaming-core`
Expected: PASS (incluye los tests previos del crate).

- [ ] **Step 8: Commit**

```bash
git add crates/core/src/playback.rs crates/core/src/error.rs
git commit -m "feat(core): ruta RecodeAudio y error Unsupported"
```

---

## Task 3: `media::decide` — tabla codecs → plan (pura)

**Files:**
- Create: `crates/media/src/decide.rs`
- Modify: `crates/media/src/lib.rs`

- [ ] **Step 1: Escribir los tests que fallan (tabla completa)**

`crates/media/src/decide.rs`:

```rust
//! Decisión pura de ruta de reproducción. Sin I/O.

use pistreaming_core::error::CoreError;
use pistreaming_core::playback::{PlaybackPlan, PlaybackRoute};

/// Subconjunto de ffprobe que necesitamos para decidir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// `format.format_name` normalizado (p. ej. "matroska", "mov,mp4,m4a,3gp,3g2,mj2").
    pub container: String,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
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

pub fn decide(
    session: &str,
    probe: &Probe,
    playback_url: &str,
    raw_url: Option<&str>,
) -> Result<PlaybackPlan, CoreError> {
    let video = probe.video_codec.clone().unwrap_or_default();
    let audio = probe.audio_codec.clone();
    if !VIDEO_OK.contains(&video.as_str()) {
        return Err(CoreError::Unsupported(format!("video codec no soportado: {video}")));
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
        }
    }

    fn route(c: &str, v: &str, a: Option<&str>) -> PlaybackRoute {
        decide("s", &p(c, Some(v), a), "/stream/s", Some("/raw/s")).unwrap().route
    }

    #[test]
    fn mp4_h264_aac_es_direct() {
        assert_eq!(route("mov,mp4,m4a,3gp,3g2,mj2", "h264", Some("aac")), PlaybackRoute::Direct);
    }

    #[test]
    fn webm_vp9_opus_es_direct() {
        assert_eq!(route("matroska,webm", "vp9", Some("opus")), PlaybackRoute::Direct);
    }

    #[test]
    fn mkv_h264_aac_es_remux() {
        assert_eq!(route("matroska", "h264", Some("aac")), PlaybackRoute::Remux);
    }

    #[test]
    fn mkv_hevc_marca_browser_may_fail() {
        let plan = decide("s", &p("matroska", Some("hevc"), Some("aac")), "/stream/s", None).unwrap();
        assert_eq!(plan.route, PlaybackRoute::Remux);
        assert!(plan.browser_may_fail);
    }

    #[test]
    fn mkv_h264_ac3_recodifica_audio() {
        let plan =
            decide("s", &p("matroska", Some("h264"), Some("ac3")), "/stream/s", None).unwrap();
        assert_eq!(plan.route, PlaybackRoute::RecodeAudio);
        assert!(plan.needs_recode_audio);
    }

    #[test]
    fn truehd_y_dts_tambien_recodifican() {
        assert_eq!(route("matroska", "h264", Some("truehd")), PlaybackRoute::RecodeAudio);
        assert_eq!(route("matroska", "h264", Some("dts")), PlaybackRoute::RecodeAudio);
    }

    #[test]
    fn video_no_soportado_da_error_422() {
        let e = decide("s", &p("mpegts", Some("mpeg2video"), Some("aac")), "/stream/s", None)
            .unwrap_err();
        assert!(matches!(e, CoreError::Unsupported(_)));
    }

    #[test]
    fn sin_audio_no_rompe() {
        assert_eq!(route("matroska", "h264", None), PlaybackRoute::Remux);
    }
}
```

- [ ] **Step 2: Correr y ver fallar**

Run: `cargo test -p pistreaming-media decide::`
Expected: FAIL (`cannot find module decide` / `unresolved import`).

- [ ] **Step 3: Registrar el módulo**

En `crates/media/src/lib.rs`, reemplazar el contenido por:

```rust
//! Media engine: probe (ffprobe) y decisión de plan de reproducción.
pub mod decide;

pub use decide::{decide, Probe};
```

- [ ] **Step 4: Correr y ver pasar**

Run: `cargo test -p pistreaming-media`
Expected: PASS (8 tests de `decide`).

- [ ] **Step 5: Commit**

```bash
git add crates/media/src/lib.rs crates/media/src/decide.rs
git commit -m "feat(media): decide() puro con tabla de codecs"
```

---

## Task 4: `media::probe` — ffprobe JSON → `Probe`

**Files:**
- Create: `crates/media/src/probe.rs`
- Modify: `crates/media/src/lib.rs`

- [ ] **Step 1: Implementación (parseo puro + spawn)**

`crates/media/src/probe.rs`:

```rust
//! Ejecuta `ffprobe` sobre una ruta/URL y devuelve un `Probe`.

use std::process::Stdio;
use pistreaming_core::error::CoreError;
use serde::Deserialize;

use crate::decide::Probe;

pub fn ffprobe_available() -> bool {
    std::process::Command::new("ffprobe")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[derive(Deserialize)]
struct FfprobeOut {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    #[serde(default)]
    format: Option<FfprobeFormat>,
}

#[derive(Deserialize)]
struct FfprobeStream {
    #[serde(default)]
    codec_name: Option<String>,
    #[serde(default)]
    codec_type: Option<String>,
}

#[derive(Deserialize)]
struct FfprobeFormat {
    #[serde(default)]
    format_name: Option<String>,
}

/// Extrae `Probe` del JSON de ffprobe (pura, testeable).
pub fn probe_from_json(json: &str) -> Result<Probe, CoreError> {
    let out: FfprobeOut =
        serde_json::from_str(json).map_err(|e| CoreError::Json(e.to_string()))?;
    let video_codec = out
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .and_then(|s| s.codec_name.clone());
    let audio_codec = out
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
        .and_then(|s| s.codec_name.clone());
    Ok(Probe {
        container: out
            .format
            .and_then(|f| f.format_name)
            .unwrap_or_default(),
        video_codec,
        audio_codec,
    })
}

/// Corre ffprobe sobre `input` (ruta local o URL) y parsea el JSON.
pub async fn probe(input: &str) -> Result<Probe, CoreError> {
    let out = tokio::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-print_format", "json", "-show_format", "-show_streams", input])
        .output()
        .await
        .map_err(|e| CoreError::Other(format!("ffprobe no ejecutable: {e}")))?;
    if !out.status.success() {
        return Err(CoreError::Other(format!(
            "ffprobe falló: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    probe_from_json(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{
      "format": { "format_name": "matroska,webm" },
      "streams": [
        { "codec_name": "h264", "codec_type": "video" },
        { "codec_name": "ac3",  "codec_type": "audio" },
        { "codec_name": "subrip", "codec_type": "subtitle" }
      ]
    }"#;

    #[test]
    fn parsea_video_audio_y_contenedor() {
        let p = probe_from_json(JSON).unwrap();
        assert_eq!(p.container, "matroska,webm");
        assert_eq!(p.video_codec.as_deref(), Some("h264"));
        assert_eq!(p.audio_codec.as_deref(), Some("ac3"));
    }

    #[test]
    fn json_invalido_da_error() {
        assert!(probe_from_json("{").is_err());
    }

    #[tokio::test]
    async fn probe_real_se_salta_sin_ffprobe() {
        if !ffprobe_available() {
            eprintln!("skip: ffprobe no está en PATH");
            return;
        }
        // Genera un archivo de 1s con ffmpeg y lo inspecciona.
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t.mp4");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-v", "quiet", "-y", "-f", "lavfi", "-i", "testsrc=size=64x64:rate=5",
                   "-t", "1", "-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(&f)
            .status()
            .unwrap()
            .success();
        if !ok {
            eprintln!("skip: ffmpeg no pudo generar el fixture");
            return;
        }
        let p = probe(f.to_str().unwrap()).await.unwrap();
        assert_eq!(p.video_codec.as_deref(), Some("h264"));
    }
}
```

- [ ] **Step 2: Registrar el módulo**

En `crates/media/src/lib.rs`:

```rust
//! Media engine: probe (ffprobe) y decisión de plan de reproducción.
pub mod decide;
pub mod probe;

pub use decide::{decide, Probe};
pub use probe::{probe, probe_from_json};
```

- [ ] **Step 3: Agregar deps a `crates/media/Cargo.toml`**

```toml
tokio.workspace = true
serde.workspace = true
anyhow.workspace = true

[dev-dependencies]
tempfile.workspace = true
```

- [ ] **Step 4: Correr**

Run: `cargo test -p pistreaming-media`
Expected: PASS (o skip con mensaje si falta ffprobe/ffmpeg).

- [ ] **Step 5: Commit**

```bash
git add crates/media/src/lib.rs crates/media/src/probe.rs crates/media/Cargo.toml
git commit -m "feat(media): probe() con ffprobe y parseo JSON"
```

---

## Task 5: `torrent` — sesión, `add_magnet`, selección de archivo y `stream`

**Files:**
- Create: `crates/torrent/src/engine.rs`
- Modify: `crates/torrent/src/lib.rs`
- Test: `crates/torrent/tests/local_seeder.rs`

- [ ] **Step 1: Implementación del engine**

`crates/torrent/src/engine.rs`:

```rust
//! Engine de torrents sobre librqbit: sesión única, add_magnet, selección y stream.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrentHandle, Session, SessionOptions};

use pistreaming_core::error::CoreError;

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
        output_folder: Some(output_folder.to_path_buf()),
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
    let info_hash = handle.info_hash().to_string();
    Ok(AddedTorrent { handle, info_hash })
}

/// Devuelve `(file_id, nombre relativo, len)` del video más grande del torrent.
pub fn pick_largest_video(handle: &ManagedTorrentHandle) -> Result<(usize, PathBuf, u64), CoreError> {
    handle
        .with_metadata(|m| {
            m.file_infos
                .iter()
                .enumerate()
                .map(|(i, f)| (i, f.relative_filename.clone(), f.len))
                .filter(|(_, name, _)| is_video_name(&name.to_string_lossy()))
                .max_by_key(|(_, _, len)| *len)
        })
        .map_err(|e| CoreError::Other(format!("metadata no disponible: {e}")))?
        .ok_or_else(|| CoreError::NotFound("el torrent no tiene archivos de video".into()))
}

fn is_video_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [".mkv", ".mp4", ".m4v", ".avi", ".webm", ".mov", ".ts"]
        .iter()
        .any(|ext| n.ends_with(ext))
}

/// Abre el `FileStream` de un archivo del torrent.
pub async fn torrent_stream(
    handle: &ManagedTorrentHandle,
    file_id: usize,
) -> Result<librqbit::FileStream, CoreError> {
    handle
        .clone()
        .stream(file_id)
        .await
        .map_err(|e| CoreError::Other(format!("no se pudo abrir el stream: {e}")))
}
```

- [ ] **Step 2: Registro en `lib.rs`**

`crates/torrent/src/lib.rs`:

```rust
//! Torrent engine (librqbit).
pub mod engine;

pub use engine::{add_magnet, open_session, pick_largest_video, torrent_stream, AddedTorrent};
```

- [ ] **Step 3: Test de integración con seeder local**

`crates/torrent/tests/local_seeder.rs`:

```rust
//! Levanta una sesión "seeder" que sirve un archivo local como torrent y
//! valida que el engine puede agregarlo y leer bytes del stream.
use std::path::PathBuf;
use std::sync::Arc;

use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
use pistreaming_torrent::{add_magnet, open_session, pick_largest_video, torrent_stream};
use tokio::io::AsyncReadExt;

#[tokio::test]
async fn descarga_de_un_seeder_local() {
    let dir = tempfile::tempdir().unwrap();
    let seed_dir = dir.path().join("seed");
    std::fs::create_dir_all(&seed_dir).unwrap();
    let payload: Vec<u8> = (0..(256 * 1024)).map(|i| (i % 251) as u8).collect();
    let file = seed_dir.join("clip.mkv");
    std::fs::write(&file, &payload).unwrap();

    // Seeder local (sin DHT/trackers).
    let seeder: Arc<Session> = Session::new_with_opts(
        seed_dir.clone(),
        SessionOptions { disable_trackers: true, ..Default::default() },
    )
    .await
    .unwrap();
    let seeder_handle = match seeder
        .add_torrent(
            AddTorrent::from_local_filename(file.to_str().unwrap()).unwrap(),
            Some(AddTorrentOptions { output_folder: Some(seed_dir.clone()), overwrite: true, ..Default::default() }),
        )
        .await
        .unwrap()
    {
        AddTorrentResponse::Added(_, h) | AddTorrentResponse::AlreadyManaged(_, h) => h,
        _ => panic!("seeder inesperado"),
    };
    let info_hash = seeder_handle.info_hash().to_string();
    // Deja que el seeder anuncie.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Cliente: apunta al seeder por puerto local.
    let client_dir = dir.path().join("client");
    std::fs::create_dir_all(&client_dir).unwrap();
    let client = open_session(client_dir.clone(), None).await.unwrap();
    let magnet = format!("magnet:?xt=urn:btih:{info_hash}&x.pe=127.0.0.1:{}",
        seeder_handle.shared().id /* placeholder puerto */);
    let _ = magnet;
    let _ = add_magnet; // (ver Step 4: completar el peering explícito)
}
```

> **Nota de implementación (Step 4):** el peering local sin DHT/trackers requiere indicar el `SocketAddr` real del listener del seeder. Completar así:

- [ ] **Step 4: Completar el peering explícito y la aserción**

Sustituir el bloque final del test por:

```rust
    // El cliente recibe el peer del seeder por `initial_peers` (sin DHT ni tracker).
    let addr = seeder
        .get_listen_addr()
        .expect("el seeder debe tener listener local");
    let opts = AddTorrentOptions {
        output_folder: Some(client_dir.clone()),
        overwrite: true,
        initial_peers: Some(vec![addr]),
        ..Default::default()
    };
    let resp = client
        .add_torrent(AddTorrent::from_url(format!("magnet:?xt=urn:btih:{info_hash}")), Some(opts))
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
```

> Si `Session::get_listen_addr()` no existe con ese nombre exacto, inspeccionar `docs.rs/librqbit/9.0.1` (método del listener) y usar el getter real (`listen_addr()`, `local_addr()`, etc.). *No hay placeholder de código aquí: es un nombre de método a confirmar con `cargo doc`.*

- [ ] **Step 5: Correr**

Run: `cargo test -p pistreaming-torrent --test local_seeder -- --nocapture`
Expected: PASS (o falla indicando el getter real del listener → corregir y repetir).

- [ ] **Step 6: Commit**

```bash
git add crates/torrent/src/lib.rs crates/torrent/src/engine.rs crates/torrent/tests/local_seeder.rs
git commit -m "feat(torrent): engine librqbit con add_magnet/selección/stream"
```

## Task 6: `api::range` — `parse_range` + `ranged_response` (núcleo HTTP)

**Files:**
- Create: `crates/api/src/range.rs`
- Test: tests inline

- [ ] **Step 1: Tests que fallan**

`crates/api/src/range.rs`:

```rust
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
```

- [ ] **Step 2: Correr y ver fallar**

Run: `cargo test -p pistreaming-api range::`
Expected: FAIL (`cannot find module range`).

- [ ] **Step 3: Registrar el módulo**

En `crates/api/src/lib.rs`, arriba de todo junto a los demás `mod`:

```rust
pub mod range;
```

- [ ] **Step 4: Correr y ver pasar**

Run: `cargo test -p pistreaming-api range::`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/api/src/lib.rs crates/api/src/range.rs
git commit -m "feat(api): parse_range y ranged_response (200/206/416)"
```

---

## Task 7: `api::session` — registro de sesiones de reproducción

**Files:**
- Create: `crates/api/src/session.rs`
- Test: tests inline

- [ ] **Step 1: Tests que fallan**

`crates/api/src/session.rs`:

```rust
//! Registro en memoria de sesiones de reproducción activas.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use pistreaming_core::playback::PlaybackPlan;

/// Una sesión de reproducción viva.
pub struct PlaySession {
    pub id: String,
    pub info_hash: String,
    pub file_id: usize,
    pub plan: PlaybackPlan,
    pub cache_dir: std::path::PathBuf,
    pub created_at: Instant,
    /// Child de ffmpeg para remux/recode, si aplica.
    pub ffmpeg: Option<tokio::process::Child>,
}

#[derive(Default)]
pub struct PlaySessionRegistry {
    inner: RwLock<HashMap<String, Arc<RwLock<PlaySession>>>>,
}

impl PlaySessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, s: PlaySession) -> Arc<RwLock<PlaySession>> {
        let id = s.id.clone();
        let arc = Arc::new(RwLock::new(s));
        self.inner.write().insert(id, Arc::clone(&arc));
        arc
    }

    pub fn get(&self, id: &str) -> Option<Arc<RwLock<PlaySession>>> {
        self.inner.read().get(id).cloned()
    }

    pub fn remove(&self, id: &str) -> Option<Arc<RwLock<PlaySession>>> {
        self.inner.write().remove(id)
    }

    pub fn active_ids(&self) -> Vec<String> {
        self.inner.read().keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.inner.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// IDs de sesiones más viejas que `max_age`.
    pub fn stale_ids(&self, max_age: Duration) -> Vec<String> {
        let now = Instant::now();
        self.inner
            .read()
            .iter()
            .filter(|(_, s)| now.duration_since(s.read().created_at) > max_age)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pistreaming_core::playback::PlaybackRoute;

    fn plan() -> PlaybackPlan {
        PlaybackPlan {
            session: "s1".into(),
            route: PlaybackRoute::Direct,
            playback_url: "/stream/s1".into(),
            raw_url: Some("/raw/s1".into()),
            browser_may_fail: false,
            needs_recode_audio: false,
            video_codec: "h264".into(),
            audio_codec: Some("aac".into()),
        }
    }

    fn sess(id: &str) -> PlaySession {
        PlaySession {
            id: id.into(),
            info_hash: "abc".into(),
            file_id: 0,
            plan: plan(),
            cache_dir: "/tmp".into(),
            created_at: Instant::now(),
            ffmpeg: None,
        }
    }

    #[test]
    fn insert_get_remove() {
        let r = PlaySessionRegistry::new();
        r.insert(sess("a"));
        assert_eq!(r.len(), 1);
        assert!(r.get("a").is_some());
        assert!(r.remove("a").is_some());
        assert!(r.get("a").is_none());
    }

    #[test]
    fn stale_ids_detecta_viejas() {
        let r = PlaySessionRegistry::new();
        let mut s = sess("old");
        s.created_at = Instant::now() - Duration::from_secs(3600);
        r.insert(s);
        r.insert(sess("new"));
        let stale = r.stale_ids(Duration::from_secs(60));
        assert_eq!(stale, vec!["old".to_string()]);
        assert_eq!(r.active_ids().len(), 2);
    }
}
```

- [ ] **Step 2: Agregar `parking_lot` al workspace y a `api`**

`Cargo.toml` (`[workspace.dependencies]`):

```toml
parking_lot = "0.12"
```

`crates/api/Cargo.toml` (`[dependencies]`):

```toml
parking_lot.workspace = true
```

- [ ] **Step 3: Registrar el módulo y correr**

En `crates/api/src/lib.rs`:

```rust
pub mod session;
```

Run: `cargo test -p pistreaming-api session::`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml crates/api/Cargo.toml crates/api/src/lib.rs crates/api/src/session.rs
git commit -m "feat(api): PlaySessionRegistry"
```

---

## Task 8: `api` — `AppState` extendido + `POST /api/play`

**Files:**
- Modify: `crates/api/src/lib.rs`
- Test: `crates/api/tests/play_endpoint.rs`

- [ ] **Step 1: Extender `AppState` y agregar el handler**

En `crates/api/src/lib.rs`, importar y extender:

```rust
use pistreaming_core::playback::PlaybackRoute;
use pistreaming_media::{decide, probe};
use pistreaming_torrent::{add_magnet, pick_largest_video};
use crate::session::{PlaySession, PlaySessionRegistry};
use std::sync::Arc as StdArc;
```

`AppState`:

```rust
pub struct AppState {
    pub store: Store,
    pub client: AddonClient,
    pub addons: Arc<RwLock<AddonManager>>,
    pub mutex: Mutex<()>,
    /// Sesión de librqbit (None hasta que el server la crea tras el arranque).
    pub torrents: tokio::sync::OnceCell<StdArc<librqbit::Session>>,
    pub cache_dir: std::path::PathBuf,
    pub public_base: String,
    pub sessions: PlaySessionRegistry,
}
```

Handler + body:

```rust
#[derive(Deserialize)]
pub struct PlayBody {
    pub magnet: String,
    #[serde(default)]
    pub title: Option<String>,
}

/// POST /api/play { magnet } -> PlaybackPlan
pub async fn play(
    State(st): State<SharedState>,
    body: Result<Json<PlayBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(b) => b,
        Err(_) => return err(StatusCode::BAD_REQUEST, "body inválido"),
    };
    let session = match st.torrents.get() {
        Some(s) => s.clone(),
        None => return err(StatusCode::SERVICE_UNAVAILABLE, "engine de torrents no inicializado"),
    };

    // 1) agregar el magnet (crea la carpeta de caché por info_hash)
    let added = match add_magnet(&session, &body.magnet, &st.cache_dir).await {
        Ok(a) => a,
        Err(e) => return core_err(e),
    };
    let cache_dir = st.cache_dir.join(&added.info_hash);
    std::fs::create_dir_all(&cache_dir).ok();

    // 2) elegir archivo de video y abrir el descriptor crudo para ffprobe
    let (file_id, _name, _len) = match pick_largest_video(&added.handle) {
        Ok(v) => v,
        Err(e) => return core_err(e),
    };

    // 3) probe vía el endpoint /raw (seekable). Necesitamos la sesión creada antes:
    //    el registro se inserta con un plan provisional y se completa abajo.
    let session_id = uuid_like(&added.info_hash, file_id);
    let raw_url = format!("{}/raw/{}", st.public_base, session_id);
    let playback_url = format!("{}/stream/{}", st.public_base, session_id);

    // Se crea la sesión YA para que /raw/:session pueda resolver el FileStream.
    let probe_input = local_probe_path(&added.handle, file_id);
    let probe_result = match probe_input {
        Some(path) => probe(path.to_str().unwrap()).await,
        None => probe(&raw_url).await,
    };
    let p = match probe_result {
        Ok(p) => p,
        Err(e) => return core_err(e),
    };

    let mut plan = match decide(&session_id, &p, &playback_url, Some(&raw_url)) {
        Ok(plan) => plan,
        Err(e) => return core_err(e),
    };
    if plan.route == PlaybackRoute::Direct {
        plan.playback_url = raw_url.clone();
    }

    st.sessions.insert(PlaySession {
        id: session_id.clone(),
        info_hash: added.info_hash.clone(),
        file_id,
        plan: plan.clone(),
        cache_dir,
        created_at: std::time::Instant::now(),
        ffmpeg: None,
    });
    st.handles.insert(session_id.clone(), added.handle);

    StArc::new(()); // no-op: mantiene el import de Arc explícito
    (StatusCode::OK, Json(plan)).into_response()
}

/// Ruta local del archivo dentro de la carpeta de salida de librqbit, si existe.
fn local_probe_path(handle: &librqbit::ManagedTorrentHandle, file_id: usize) -> Option<std::path::PathBuf> {
    handle
        .with_metadata(|m| {
            let f = m.file_infos.iter().nth(file_id)?;
            Some(f.relative_filename.clone())
        })
        .ok()
        .flatten()
        .map(|rel| handle.output_folder().join(rel))
}

fn uuid_like(info_hash: &str, file_id: usize) -> String {
    format!("{}-{}", &info_hash[..info_hash.len().min(12)], file_id)
}
```

> **Nota:** `AppState.handles` es un `std::collections::HashMap<String, librqbit::ManagedTorrentHandle>` protegido por `parking_lot::RwLock`, necesario para que `/raw/:session` recupere el handle. Agregarlo al struct:

```rust
    pub handles: parking_lot::RwLock<std::collections::HashMap<String, librqbit::ManagedTorrentHandle>>,
```

- [ ] **Step 2: Test de integración (endpoint con engine real y seeder local)**

`crates/api/tests/play_endpoint.rs`:

```rust
//! Verifica POST /api/play con un seeder local y valida la forma del plan.
// (Sigue el mismo peering local que crates/torrent/tests/local_seeder.rs.)
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
```

- [ ] **Step 3: Completar el test**

Agregar bajo lo anterior:

```rust
#[tokio::test]
async fn play_sin_engine_da_503() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf()); // helper de test en lib.rs
    let app = pistreaming_api::router(st);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/play")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"magnet":"magnet:?xt=urn:btih:0000000000000000000000000000000000000000"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::SERVICE_UNAVAILABLE);
}
```

Y en `crates/api/src/lib.rs` agregar el helper de test:

```rust
/// Construye un AppState sin engine de torrents (test-only).
#[doc(hidden)]
pub fn test_state(dir: std::path::PathBuf) -> SharedState {
    let store = Store::open(&dir.join("pistreaming.db")).expect("store");
    let client = AddonClient::new(reqwest::Client::new());
    StdArc::new(AppState {
        store,
        client,
        addons: StdArc::new(RwLock::new(AddonManager::empty())),
        mutex: Mutex::new(()),
        torrents: tokio::sync::OnceCell::new(),
        cache_dir: dir,
        public_base: "http://127.0.0.1:8000".into(),
        sessions: PlaySessionRegistry::new(),
        handles: parking_lot::RwLock::new(Default::default()),
    })
}
```

> Si `AddonManager::empty()` no existe, usar `AddonManager::load(client.clone(), &store).await` — pero el helper es síncrono; en ese caso crear el estado dentro del test con `.await`. Verificar en `crates/addons/src/lib.rs`.

- [ ] **Step 4: Correr**

Run: `cargo test -p pistreaming-api --test play_endpoint -- --nocapture`
Expected: PASS (503 cuando no hay engine).

- [ ] **Step 5: Commit**

```bash
git add crates/api/src/lib.rs crates/api/tests/play_endpoint.rs
git commit -m "feat(api): POST /api/play (magnet -> probe -> plan)"
```

---

## Task 9: `api` — `GET /raw/:session` y `GET /stream/:session` (directo)

**Files:**
- Modify: `crates/api/src/lib.rs`
- Test: `crates/api/tests/range_serving.rs`

- [ ] **Step 1: Handlers**

En `crates/api/src/lib.rs`:

```rust
/// GET /raw/:session — bytes crudos del FileStream con Range (fuente para ffmpeg).
pub async fn raw_stream(
    State(st): State<SharedState>,
    Path(session): Path<String>,
    headers: HeaderMap,
) -> Response {
    let handle = match st.handles.read().get(&session).cloned() {
        Some(h) => h,
        None => return err(StatusCode::NOT_FOUND, "sesión desconocida"),
    };
    let file_id = match st.sessions.get(&session) {
        Some(s) => s.read().file_id,
        None => return err(StatusCode::NOT_FOUND, "sesión desconocida"),
    };
    let stream = match pistreaming_torrent::torrent_stream(&handle, file_id).await {
        Ok(s) => s,
        Err(e) => return core_err(e),
    };
    let len = stream.len();
    crate::range::ranged_response(stream, len, &headers, "application/octet-stream").await
}

/// GET /stream/:session — lo que consume el <video>. Directo: sirve el FileStream.
/// (Remux/RecodeAudio se completan en la Task 10.)
pub async fn stream(
    State(st): State<SharedState>,
    Path(session): Path<String>,
    headers: HeaderMap,
) -> Response {
    let handle = match st.handles.read().get(&session).cloned() {
        Some(h) => h,
        None => return err(StatusCode::NOT_FOUND, "sesión desconocida"),
    };
    let (file_id, route) = match st.sessions.get(&session) {
        Some(s) => {
            let g = s.read();
            (g.file_id, g.plan.route)
        }
        None => return err(StatusCode::NOT_FOUND, "sesión desconocida"),
    };

    match route {
        PlaybackRoute::Direct => {
            let stream = match pistreaming_torrent::torrent_stream(&handle, file_id).await {
                Ok(s) => s,
                Err(e) => return core_err(e),
            };
            let len = stream.len();
            let mime = if session_mime(&st, &session).contains("webm") {
                "video/webm"
            } else {
                "video/mp4"
            };
            crate::range::ranged_response(stream, len, &headers, mime).await
        }
        PlaybackRoute::Remux | PlaybackRoute::RecodeAudio => {
            // Task 10.
            err(StatusCode::NOT_IMPLEMENTED, "remux aún no implementado")
        }
    }
}

fn session_mime(_st: &SharedState, _session: &str) -> String {
    "video/mp4".to_string()
}
```

Rutas nuevas en `router`:

```rust
        .route("/api/play", axum::routing::post(play))
        .route("/raw/:session", get(raw_stream))
        .route("/stream/:session", get(stream))
```

- [ ] **Step 2: Test de serving (con un file temporal como fuente)**

`crates/api/tests/range_serving.rs`:

```rust
//! Verifica que un reader seekable se sirve 200/206 vía ranged_response.
use axum::body::to_bytes;
use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue};
use std::io::Write;

#[tokio::test]
async fn sirve_206_con_rango() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.bin");
    std::fs::write(&path, b"0123456789").unwrap();

    let file = tokio::fs::File::open(&path).await.unwrap();
    let meta = file.metadata().await.unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("range", HeaderValue::from_static("bytes=2-4"));

    let resp = pistreaming_api::range::ranged_response(file, meta.len(), &headers, "text/plain").await;
    assert_eq!(resp.status(), 206);
    let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&body[..], b"234");
}
```

- [ ] **Step 3: Correr**

Run: `cargo test -p pistreaming-api`
Expected: PASS (todos, incluidos los previos).

- [ ] **Step 4: Commit**

```bash
git add crates/api/src/lib.rs crates/api/tests/range_serving.rs
git commit -m "feat(api): /raw y /stream (ruta directa) con Range"
```

---

## Task 10: `media::ffmpeg` + `/stream` remux/recode

**Files:**
- Create: `crates/media/src/ffmpeg.rs`
- Modify: `crates/media/src/lib.rs`, `crates/api/src/lib.rs`

- [ ] **Step 1: Implementación**

`crates/media/src/ffmpeg.rs`:

```rust
//! Spawn de ffmpeg para remux (copia) o recode de audio.

use std::path::Path;
use std::process::Stdio;
use pistreaming_core::error::CoreError;

/// Remux MKV→fMP4 sin recodificar (`-c copy`). Escribe en `output` (se crea/crece).
pub fn remux(input: &str, output: &Path) -> Result<tokio::process::Child, CoreError> {
    ffmpeg(input, output, false)
}

/// Remux + recodifica solo el audio a AAC (video `-c copy`).
pub fn recode_audio(input: &str, output: &Path) -> Result<tokio::process::Child, CoreError> {
    ffmpeg(input, output, true)
}

fn ffmpeg(input: &str, output: &Path, recode: bool) -> Result<tokio::process::Child, CoreError> {
    let mut cmd = tokio::process::Command::new("ffmpeg");
    cmd.arg("-v").arg("error").arg("-y").arg("-i").arg(input);
    if recode {
        cmd.args(["-map", "0:v:0", "-map", "0:a:0?", "-c:v", "copy",
                  "-c:a", "aac", "-b:a", "192k", "-ac", "2"]);
    } else {
        cmd.args(["-map", "0:v:0", "-map", "0:a:0?", "-c", "copy"]);
    }
    cmd.args(["-movflags", "frag_keyframe+empty_moov", "-f", "mp4"])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn()
        .map_err(|e| CoreError::Other(format!("no se pudo lanzar ffmpeg: {e}")))
}
```

- [ ] **Step 2: Registrar el módulo**

En `crates/media/src/lib.rs` agregar:

```rust
pub mod ffmpeg;
```

- [ ] **Step 3: Integrar en `/stream`**

En `crates/api/src/lib.rs`, reemplazar la rama `Remux | RecodeAudio` por:

```rust
        PlaybackRoute::Remux | PlaybackRoute::RecodeAudio => {
            let (cache_dir, recode) = {
                let s = match st.sessions.get(&session) {
                    Some(s) => s,
                    None => return err(StatusCode::NOT_FOUND, "sesión desconocida"),
                };
                let g = s.read();
                (g.cache_dir.clone(), g.plan.route == PlaybackRoute::RecodeAudio)
            };
            let out = cache_dir.join("playback.mp4");
            let raw_url = format!("{}/raw/{}", st.public_base, session);

            // Arranca ffmpeg una sola vez; sirve lo ya escrito.
            let needs_spawn = !out.exists() || out.metadata().map(|m| m.len() == 0).unwrap_or(true);
            if needs_spawn {
                let child = if recode {
                    pistreaming_media::ffmpeg::recode_audio(&raw_url, &out)
                } else {
                    pistreaming_media::ffmpeg::remux(&raw_url, &out)
                };
                match child {
                    Ok(c) => {
                        if let Some(s) = st.sessions.get(&session) {
                            s.write().ffmpeg = Some(c);
                        }
                    }
                    Err(e) => return core_err(e),
                }
            }

            // Espera a que exista el archivo (o el cliente reconecta).
            for _ in 0..40 {
                if out.exists() && out.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            if !out.exists() {
                return err(StatusCode::GATEWAY_TIMEOUT, "ffmpeg no produjo salida");
            }
            use tokio::io::AsyncSeekExt;
            let mut file = match tokio::fs::File::open(&out).await {
                Ok(f) => f,
                Err(_) => return err(StatusCode::GATEWAY_TIMEOUT, "salida no disponible"),
            };
            let len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
            let _ = file.seek(std::io::SeekFrom::Start(0)).await;
            // fMP4 fragmentado: no se puede saber el largo final; se sirve lo disponible.
            let headers2 = HeaderMap::new();
            crate::range::ranged_response(file, len, &headers2, "video/mp4").await
        }
```

- [ ] **Step 4: Test (unit, sin red: verifica el comando armado)**

En `crates/media/src/ffmpeg.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn remux_sin_input_falla_rapido() {
        let out = std::env::temp_dir().join("pistreaming_ffmpeg_test.mp4");
        // input inexistente: ffmpeg se lanza, sale con error; verificamos que spawn funciona.
        let child = remux("/no/existe.mkv", &out);
        assert!(child.is_ok(), "spawn debe funcionar si ffmpeg está instalado");
    }
}
```

- [ ] **Step 5: Correr**

Run: `cargo test -p pistreaming-media && cargo test -p pistreaming-api`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/media/src/lib.rs crates/media/src/ffmpeg.rs crates/api/src/lib.rs
git commit -m "feat(media,api): remux/recode con ffmpeg y /stream no directo"
```

---

## Task 11: `api` — progreso con clave compuesta

**Files:**
- Modify: `crates/api/src/lib.rs`
- Test: `crates/api/tests/progress.rs`

- [ ] **Step 1: Handler + body**

```rust
#[derive(Deserialize)]
pub struct ProgressBody {
    pub position: f64,
    #[serde(default)]
    pub duration: Option<f64>,
}

/// GET /api/progress/:kind/:id
pub async fn get_progress(
    State(st): State<SharedState>,
    Path((kind, id)): Path<(String, String)>,
) -> Response {
    let key = format!("{kind}:{id}");
    match st.store.get_progress(&key) {
        Ok(Some((position, duration))) => Json(serde_json::json!({
            "position": position, "duration": duration
        }))
        .into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "sin progreso"),
        Err(e) => core_err(e),
    }
}

/// PUT /api/progress/:kind/:id { position, duration? }
pub async fn put_progress(
    State(st): State<SharedState>,
    Path((kind, id)): Path<(String, String)>,
    body: Result<Json<ProgressBody>, JsonRejection>,
) -> Response {
    let Json(body) = match body {
        Ok(b) => b,
        Err(_) => return err(StatusCode::BAD_REQUEST, "body inválido"),
    };
    let key = format!("{kind}:{id}");
    match st.store.save_progress(&key, body.position, body.duration) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => core_err(e),
    }
}
```

Rutas:

```rust
        .route("/api/progress/:kind/:id", get(get_progress).put(put_progress))
```

- [ ] **Step 2: Test**

`crates/api/tests/progress.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn put_luego_get_progreso() {
    let tmp = tempfile::tempdir().unwrap();
    let st = pistreaming_api::test_state(tmp.path().to_path_buf());
    let app = pistreaming_api::router(st.clone());

    let put = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/progress/movie/tt123")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"position":42.5,"duration":100.0}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::NO_CONTENT);

    let get = app
        .oneshot(Request::builder().uri("/api/progress/movie/tt123").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let body = axum::body::to_bytes(get.into_body(), usize::MAX).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["position"], 42.5);
    assert_eq!(v["duration"], 100.0);
}
```

- [ ] **Step 3: Correr**

Run: `cargo test -p pistreaming-api --test progress`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/api/src/lib.rs crates/api/tests/progress.rs
git commit -m "feat(api): progreso con clave compuesta kind:id"
```

---

## Task 12: `api` — player mínimo en `/play/:session`

**Files:**
- Create: `crates/api/assets/player.html`
- Modify: `crates/api/src/lib.rs`
- Test: `crates/api/tests/player.rs`

- [ ] **Step 1: HTML**

`crates/api/assets/player.html`:

```html
<!doctype html>
<html lang="es">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>PiStreaming</title>
  <style>
    body { margin: 0; background: #111; color: #eee; font-family: system-ui, sans-serif; }
    header { padding: 12px 16px; font-size: 14px; opacity: .8; }
    video { width: 100%; max-height: 80vh; background: #000; }
    .fallback { padding: 12px 16px; font-size: 13px; }
    a { color: #6cf; }
  </style>
</head>
<body>
  <header id="title">Reproduciendo…</header>
  <video id="player" controls autoplay playsinline></video>
  <div class="fallback">
    <div id="err"></div>
    <a id="raw" href="#" download>Descargar / abrir en VLC (URL cruda)</a>
  </div>
  <script>
    const session = location.pathname.split('/').pop();
    const v = document.getElementById('player');
    const err = document.getElementById('err');
    fetch(`/api/play/${session}`).then(r => r.json()).then(plan => {
      v.src = plan.playback_url;
      document.getElementById('raw').href = plan.raw_url || plan.playback_url;
    }).catch(() => { v.src = `/stream/${session}`; });
    v.addEventListener('error', () => {
      err.textContent = 'El navegador no puede reproducir este archivo. Usá la URL cruda.';
    });
    // Guarda progreso cada 10s si hay un id de meta conocido.
    setInterval(() => {
      if (!v.duration) return;
      fetch(`/api/progress/${session}`, { method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ position: v.currentTime, duration: v.duration }) });
    }, 10000);
  </script>
</body>
</html>
```

- [ ] **Step 2: Handler + ruta**

```rust
const PLAYER_HTML: &str = include_str!("../assets/player.html");

/// GET /play/:session — página mínima del reproductor.
pub async fn player(Path(_session): Path<String>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        PLAYER_HTML,
    )
        .into_response()
}

/// GET /api/play/:session — devuelve el PlaybackPlan de una sesión viva.
pub async fn play_plan(
    State(st): State<SharedState>,
    Path(session): Path<String>,
) -> Response {
    match st.sessions.get(&session) {
        Some(s) => Json(s.read().plan.clone()).into_response(),
        None => err(StatusCode::NOT_FOUND, "sesión desconocida"),
    }
}
```

Rutas:

```rust
        .route("/play/:session", get(player))
        .route("/api/play/:session", get(play_plan))
```

- [ ] **Step 3: Test**

`crates/api/tests/player.rs`:

```rust
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

#[tokio::test]
async fn player_sirve_html() {
    let tmp = tempfile::tempdir().unwrap();
    let app = pistreaming_api::router(pistreaming_api::test_state(tmp.path().to_path_buf()));
    let res = app
        .oneshot(Request::builder().uri("/play/abc").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("<video"));
}
```

- [ ] **Step 4: Correr**

Run: `cargo test -p pistreaming-api --test player`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/api/assets/player.html crates/api/src/lib.rs crates/api/tests/player.rs
git commit -m "feat(api): player mínimo y /api/play/:session"
```

---

## Task 13: `server` — wiring real + job de eviction

**Files:**
- Modify: `crates/server/src/main.rs`, `crates/server/Cargo.toml`

- [ ] **Step 1: Wiring del engine**

En `crates/server/src/main.rs`, dentro de `main` tras crear el `store`:

```rust
    use pistreaming_torrent::open_session;

    let cache_dir = cfg.data_dir.join("cache");
    std::fs::create_dir_all(&cache_dir).ok();

    let torrent_session = open_session(cache_dir.clone(), Some(cfg.egress_bind.clone())).await?;
    tracing::info!(bind = %cfg.egress_bind, "sesión torrent abierta");

    let registry = pistreaming_api::session::PlaySessionRegistry::new();
    let torrents_cell = tokio::sync::OnceCell::new();
    let _ = torrents_cell.set(torrent_session);
```

Y al construir `AppState`, agregar los campos nuevos:

```rust
    let state = Arc::new(AppState {
        store,
        client,
        addons: Arc::new(RwLock::new(mgr)),
        mutex: Mutex::new(()),
        torrents: torrents_cell,
        cache_dir: cache_dir.clone(),
        public_base: format!("http://127.0.0.1:{}", cfg.http_port),
        sessions: registry,
        handles: parking_lot::RwLock::new(Default::default()),
    });
```

Agregar deps a `crates/server/Cargo.toml`:

```toml
pistreaming-torrent = { path = "../torrent" }
parking_lot.workspace = true
```

- [ ] **Step 2: Job de eviction**

En `crates/server/src/main.rs`, antes de `axum::serve`:

```rust
    // Job de eviction: cada 10 min, borra sesiones inactivas fuera de TTL o que
    // excedan CACHE_MAX_GB. Nunca toca las sesiones activas del registro.
    let evict_state = state.clone();
    let cache_max_gb = cfg.cache_max_gb;
    let cache_ttl_hours = cfg.cache_ttl_hours;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(600));
        loop {
            tick.tick().await;
            if let Err(e) = evict_cache(&evict_state, cache_max_gb, cache_ttl_hours).await {
                tracing::warn!(error = %e, "eviction falló");
            }
        }
    });
```

Y al final del archivo:

```rust
/// Eviction LRU por tamaño/TTL. No borra carpetas de sesiones activas.
async fn evict_cache(
    state: &pistreaming_api::SharedState,
    cache_max_gb: u64,
    cache_ttl_hours: u64,
) -> anyhow::Result<()> {
    use std::time::{Duration, SystemTime};

    let active: std::collections::HashSet<String> = state.sessions.active_ids().into_iter().collect();
    let root = &state.cache_dir;
    let mut dirs: Vec<(std::path::PathBuf, u64, SystemTime)> = Vec::new();
    let mut total: u64 = 0;

    let mut entries = tokio::fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if !entry.file_type().await?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if active.iter().any(|a| a.starts_with(&name) || a.contains(&name)) {
            continue; // sesión activa
        }
        let mut size = 0u64;
        let mut newest = SystemTime::UNIX_EPOCH;
        let mut stack = vec![path.clone()];
        while let Some(p) = stack.pop() {
            let mut rd = match tokio::fs::read_dir(&p).await { Ok(r) => r, Err(_) => continue };
            while let Some(f) = rd.next_entry().await.ok().flatten() {
                let md = match f.metadata().await { Ok(m) => m, Err(_) => continue };
                if md.is_dir() {
                    stack.push(f.path());
                } else {
                    size += md.len();
                    if let Ok(t) = md.modified() {
                        if t > newest { newest = t; }
                    }
                }
            }
        }
        total += size;
        dirs.push((path, size, newest));
    }

    // Orden LRU: los más viejos primero.
    dirs.sort_by_key(|(_, _, t)| *t);

    let ttl = Duration::from_secs(cache_ttl_hours * 3600);
    let now = SystemTime::now();
    for (path, size, modified) in &dirs {
        let too_old = now.duration_since(*modified).map(|d| d > ttl).unwrap_or(false);
        let too_big = total > cache_max_gb * 1024 * 1024 * 1024;
        if too_old || too_big {
            tracing::info!(dir = %path.display(), "evictando caché");
            tokio::fs::remove_dir_all(path).await.ok();
            total = total.saturating_sub(*size);
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Compilar el workspace**

Run: `cargo build --workspace 2>&1 | tail -30`
Expected: compila sin errores.

- [ ] **Step 4: Tests + clippy**

Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings`
Expected: PASS sin warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/server/src/main.rs crates/server/Cargo.toml
git commit -m "feat(server): wiring de engines y job de eviction LRU"
```

---

## Task 14: E2E, documentación y verificación final

**Files:**
- Create: `crates/api/tests/e2e_local.rs`
- Modify: `docs/superpowers/specs/2026-09-13-pistreaming-fase2-playback-design.md` (nota de divergencia), `README.md` si existe

- [ ] **Step 1: Test E2E local (seeder → /api/play → /stream con Range)**

`crates/api/tests/e2e_local.rs` — reutiliza el peering local de `crates/torrent/tests/local_seeder.rs`:

```rust
//! E2E: con un seeder local y el engine habilitado, POST /api/play devuelve un
//! plan y GET /stream/:session responde 206 con el rango solicitado.
//! (Requiere ffprobe/ffmpeg en PATH; si faltan, el test hace skip con mensaje.)
```

Completar el test con el mismo patrón de seeder que la Task 5 y:

```rust
// Tras obtener el Session handle y las sesiones:
// 1) POST /api/play con el magnet del seeder.
// 2) assert 200 y plan.route ∈ {Direct, Remux, RecodeAudio}.
// 3) GET /stream/:session con header "range: bytes=0-1023" -> 206.
```

> Nota: este test usa el engine real (`torrents` OnceCell seteado). Si `ffprobe` no está, hacer `eprintln!("skip")` y `return`.

- [ ] **Step 2: Correr todo**

Run: `cargo test --workspace && cargo clippy --workspace -- -D warnings && cargo fmt --check`
Expected: PASS. Si `fmt` falla, `cargo fmt` y commitear.

- [ ] **Step 3: Verificación manual en el Pi (checkpoint del usuario)**

Documentar en `README.md` (si existe) o en el plan:

```bash
# En el Pi (arm64), tras `cargo build --release`:
PISTREAMING_DATA_DIR=/data PISTREAMING_HTTP_PORT=8000 ./target/release/pistreaming
# En el navegador del Pi:
#   http://localhost:8000/api/play  (POST con {"magnet":"magnet:?xt=..."})
#   abrir http://localhost:8000/play/<session> y verificar que reproduce,
#   y que seek/forward avanza.
```

- [ ] **Step 4: Anotar la divergencia del spec base**

En `docs/superpowers/specs/2026-09-13-pistreaming-design.md` §8, reemplazar `/api/progress/:id` por `/api/progress/:type/:id` y agregar una línea: "Refinado en Fase 2: la clave es `{type}:{id}` sobre la tabla `progress` (PK `id`)."

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(fase2): e2e local y docs de verificación"
```

---

## Self-review (cobertura del spec)

- §1/§2 objetivo y alcance → Tasks 5–13 cubren torrent/media/playback/progreso/eviction/player.
- §3 decisiones cerradas → reflejadas en «Ajustes al spec» y en Tasks 1, 9, 11, 13.
- §4 cambios por crate → Tasks 2 (core), 3–4/10 (media), 5 (torrent), 6–12 (api), 13 (server).
- §5 flujo play → Task 8 (magnet→probe→plan) + Task 12 (player).
- §6 tabla `decide` → Task 3 (todos los casos: direct/remux/recodeaudio/unsupported).
- §7 caché/eviction → Task 13.
- §8 player mínimo → Task 12.
- §9 contrato de errores → `core_err` extendido (Task 2) + 416 (Task 6) + 503/504 (Tasks 8/10).
- §10 testing → TDD por tarea; integración local (Tasks 5/8/14); gate `cargo test` + `clippy -D warnings` (Task 13/14).
- §11 riesgos → mitigados: firma fina de librqbit (spike Task 1 + verificación de getters), Range sobre archivo creciente (content-length por snapshot), ffprobe por pipe (se usa `/raw` seekable), egress diferido (Task 13 pasa el valor, no lo aplica).
- §12 deuda Fase 1 → intacta (no se toca).

### Puntos a confirmar durante la ejecución (no placeholders, nombres/APIs a validar con el compilador)
1. Iteración de `FileInfos` (`m.file_infos.iter()`): confirmar en Task 1/5 con `cargo doc --open` o fuente en `~/.cargo/registry`.
2. Nombre del getter del puerto del listener en `Session` (Task 5, `get_listen_addr` vs `listen_addr` vs `local_addr`).
3. Presencia de `AddonManager::empty()` (Task 8); si no existe, crear el estado dentro del test con `.await`.
4. `AddTorrentOptions` debe implementar `Default` con los campos usados (`output_folder`, `overwrite`, `initial_peers`). Confirmed en el anclaje; si `output_folder` no es `Option<PathBuf>`, ajustar el tipo.
5. `AddTorrent::from_local_filename` (Task 5, seeder) — confirmar nombre exacto en el ancoring (existe como `from_local_filename`).

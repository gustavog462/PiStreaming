# Despliegue

## Servicio systemd

`config/pistreaming.service`:

```ini
[Unit]
Description=PiStreaming (personal streaming server)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=<usuario>
Group=<usuario>
WorkingDirectory=/home/<usuario>
Environment=DATA_DIR=/home/<usuario>/pistream-data
ExecStart=/home/<usuario>/pistreaming
Restart=always
RestartSec=3
StandardOutput=append:/home/<usuario>/pistream.log
StandardError=append:/home/<usuario>/pistream.log

[Install]
WantedBy=multi-user.target
```

Instalación:

```bash
sudo install -m 0644 config/pistreaming.service /etc/systemd/system/pistreaming.service
sudo systemctl daemon-reload
sudo systemctl enable --now pistreaming
```

## Logrotate

`config/logrotate-pistreaming`:

```
/home/<usuario>/pistream.log {
    weekly
    rotate 4
    size 10M
    missingok
    notifempty
    copytruncate
    compress
    delaycompress
}
```

```bash
sudo install -m 0644 config/logrotate-pistreaming /etc/logrotate.d/pistreaming
```

## Variables de entorno

Ver `config/env.example`. Las principales:

| Variable | Default | Descripción |
|---|---|---|
| `DATA_DIR` | `/data` | datos (DB, caché, biblioteca) |
| `HTTP_PORT` | `8000` | puerto HTTP |
| `EGRESS_BIND` | `eth0` | interfaz de salida torrent |
| `CACHE_MAX_GB` | `40` | límite de caché |
| `CACHE_TTL_HOURS` | `48` | TTL de caché (la DB puede sobrescribirlo) |
| `LIBRARY_DIR` | `<DATA_DIR>/library` | biblioteca |
| `OPENSUBTITLES_API_KEY` | — | subtítulos online |
| `PISTREAMING_CONFIG` | — | config alternativa |
| `RUST_LOG` | — | nivel de log |

## Datos

```
<DATA_DIR>/
├── pistreaming.db        # SQLite (WAL)
├── pistreaming.db-wal
├── pistreaming.db-shm
├── library/              # ficheros guardados
└── cache/
    ├── librqbit-state/   # persistencia de torrents (session.json, .torrent, .bitv)
    ├── <hash>/playback.mp4
    └── <hash>.torrent
```

## Build (reconstrucción)

El binario desplegado es aarch64, compilado con **rustc 1.98.1** y
**librqbit 9.0.1**. El repo `fase2-playback` tiene el workspace y el
`rust-toolchain.toml`; los assets recuperados se pueden reincorporar a
`crates/api/assets/`.

```bash
# en un host aarch64 (o con cross)
cargo build --release
# binario: target/release/pistreaming
```

Dependencias del sistema: `ffmpeg`/`ffprobe` (6.1.1 en el servidor).

## Operación

```bash
systemctl status pistreaming
systemctl restart pistreaming
journalctl -u pistreaming -n 50 --no-pager
tail -f /home/<usuario>/pistream.log
curl -s http://127.0.0.1:8000/api/health
```

## Diagnóstico rápido

- **No arranca**: revisa `DATA_DIR` (debe existir y ser escribible) y el log.
- **No reproduce**: prueba `/api/health`, `/api/search`, `/api/streams`; si el
  `play` da 504, la fuente está fría (usa `/api/warm` o reintenta).
- **Transcode lento**: el Pi no tiene decodificación HW para H.264; el
  `recode` de vídeo es costoso. Prefiere fuentes x264 (el ranking ya lo hace).
- **Disco lleno**: `/api/play` responde 507; baja `cache_max_gb` o limpia
  `cache/`.
- **Subtítulos online**: sin `OPENSUBTITLES_API_KEY` la función está
  deshabilitada.

## Notas de seguridad

- El servicio escucha en `0.0.0.0:8000` **sin autenticación**. Ver
  `docs/seguridad.md`.
- El binario contiene rutas de compilación con el usuario; no lo publiques en
  un repo público.

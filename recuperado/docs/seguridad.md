# Seguridad — hallazgos de la revisión (2026-11-22)

> Revisión de la instancia desplegada. Prioridades: alto / medio / bajo.

## 1. Sin autenticación y expuesto en `0.0.0.0:8000` — ALTO

No hay login, token ni API key. El servicio escucha en todas las interfaces y
el firewall del host estaba inactivo. Verificado: responde desde la LAN y desde
la red Tailscale.

Cualquiera con acceso a la red puede buscar, lanzar torrents, descargar
contenido, cambiar addons/ajustes y consumir CPU/disco.

**Recomendación:** auth por token (cabecera/cookie), o bind a `127.0.0.1` +
`tailscale serve`, o firewall limitando el puerto a la LAN/tailnet de confianza.

## 2. ffmpeg huérfano tras desconectar el reproductor — MEDIO-ALTO

El transcode progresivo a `playback.mp4` usa `ffmpeg -re` (ritmo real). Al
cerrar el reproductor, ffmpeg sigue corriendo y sigue descargando y
transcodificando el vídeo completo. Observado: más de 5 minutos después de
cortar el cliente, ffmpeg seguía al 10 % de CPU y `playback.mp4` creciendo.

El binario tiene funciones para matar ffmpeg (`no se pudo matar ffmpeg`,
`no se pudo esperar ffmpeg`), pero el apagado parece ocurrir al cerrar/expirar
la sesión, no al desconectar el cliente. No hay endpoint de cancelación.

**Recomendación:** matar el ffmpeg de la sesión al desconectar el cliente (o al
expirar la sesión), o separar "pre-transcode" de "streaming" y no usar `-re` en
el pre-transcode.

## 3. Probe de 8 s demasiado corto en fuentes frías — MEDIO

En el primer play de un torrent sin peers, `ffprobe` expira a los 8 s y el plan
cae a fallback (`route=recodeaudio`, `browser_may_fail=true`), recodificando
audio innecesariamente.

**Recomendación:** subir el timeout, reintentar el probe tras el `warm`, o
hacer el fallback a `remux`.

## 4. Las URLs directas no funcionan — MEDIO

El frontend soporta fuentes no-torrent (`stream.url`) y las manda como `magnet`
al `POST /api/play`. El servidor las trata siempre como torrent: un mp4
`http(s)` devuelve 500 `error decoding torrent` o 504.

**Recomendación:** detectar `content-type: video/*` (o extensión) y crear un
plan `direct` sin pasar por librqbit.

## 5. Solo binario, sin código fuente — MEDIO

No hay repo git ni `Cargo.toml` en el servidor; el directorio de compilación
`pistreaming-src` ya no existe. El repo de GitHub se quedó en la versión del
2026-09-14 (el binario es del 2026-09-21).

**Recomendación:** recuperar/reconstruir el fuente y subirlo; mantener el repo
al día.

## 6. Sin hardening de systemd — MEDIO

```
ProtectSystem=no  ProtectHome=no  NoNewPrivileges=no
MemoryMax=infinity  CPUQuota=infinity  PrivateTmp=no
```

Servicio de red (y cliente torrent) corriendo como usuario normal con acceso
total al home y sin límites.

**Recomendación:** `NoNewPrivileges=yes`, `ProtectSystem=strict`,
`ProtectHome=read-only` (con `ReadWritePaths=<DATA_DIR>`), `PrivateTmp=yes`,
`MemoryMax=512M`, `CPUQuota=200%`.

## 7. SSRF ciego en `/api/play` — MEDIO-BAJO

El endpoint acepta URLs `http(s)` y las descarga server-side (para bajar
`.torrent`). Sin auth, un usuario de la red puede hacer que el servidor consulte
URLs internas y deducir servicios por tiempos/errores.

**Recomendación:** validar/limitar URLs (solo addons conocidos) o exigir auth.

## 8. `OPENSUBTITLES_API_KEY` no configurada — BAJO

La app soporta subtítulos online, pero la variable no está en el entorno. El
propio binario lo registra: `OPENSUBTITLES_API_KEY no configurada en el
servidor`.

**Recomendación:** `EnvironmentFile=/etc/pistreaming.env` (permisos 600).

## 9. Sin backup de la base de datos — BAJO

Addons, ajustes, biblioteca y progreso viven en `pistreaming.db`. No hay cron
de backup de esa DB.

**Recomendación:** backup diario con `sqlite3 .backup` o copiando DB+WAL.

## 10. Detalles menores

- DB con freelist alto (1217/1296 páginas libres); un `VACUUM` la reduciría.
- El log de arranque muestra `cache_ttl_hours: 48` (default) aunque la DB tenga
  otro valor; confunde al diagnosticar.
- Sin cabeceras de seguridad (CSP, X-Frame-Options…).
- Binario no stripped (24,6 MB con símbolos).
- `pistreaming --help` no existe: arranca con defaults y falla sin `DATA_DIR`.

## Lo que está bien

- Servicio estable (5 días sin reinicios, consumo bajo).
- Logrotate configurado.
- Validación de entrada y protección path traversal (rechaza
  `movie/../../etc/passwd` y `../cache`).
- Sin CORS permisivo.
- Frontend sin `innerHTML` (sin XSS evidente).
- DB íntegra, WAL y recuperación de sesiones.
- Límites de caché y TTL configurables.

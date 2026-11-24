# Fase 3 — autenticación e i18n

## Autenticación

La app queda **abierta por defecto** (como hasta ahora). Se activa con
`AUTH_PASSWORD` en el entorno del servicio.

### Activar

1. Crea `/etc/pistreaming.env` (permisos 600):

   ```bash
   sudo install -m 600 /dev/null /etc/pistreaming.env
   sudo tee /etc/pistreaming.env >/dev/null <<'EOF'
   AUTH_PASSWORD=<TU_CONTRASEÑA>
   # Opcional: secreto para firmar cookie/tokens (si falta se deriva de la contraseña)
   # AUTH_SECRET=<CADENA_ALEATORIA_LARGA>
   # Opcional: API key para clientes externos (scripts, addons)
   # PISTREAMING_API_KEY=<TU_API_KEY>
   EOF
   ```

2. El unit ya incluye `EnvironmentFile=-/etc/pistreaming.env` (opcional).
3. `sudo systemctl restart pistreaming`.

### Cómo funciona

- **Login**: `GET /login` (formulario) → `POST /api/login` valida la contraseña
  y pone una cookie `pistreaming_session` firmada con HMAC-SHA256
  (`HttpOnly`, `SameSite=Lax`, 30 días). `POST /api/logout` la borra.
- **Middleware**: protege todo salvo `/login`, `/api/login`, `/api/health`,
  `/assets/*`, `/icon.svg`, `/embed.js` y `/embed/:session`.
  - API y media (`/api/*`, `/stream/*`, `/raw/*`, `/hls/*`, `/thumb/*`) →
    **401** si no hay credencial.
  - Páginas → redirect a `/login?next=...`.
- **API key** (opcional): `Authorization: Bearer <key>` o `?api_key=<key>`.
- **Token de embed**: `/embed/:session` es público; el `<video>` de la página
  lleva `?token=<HMAC(sesión)>`, así el iframe cross-site reproduce sin cookie.
  El botón «Compartir» de la SPA genera el snippet con `/embed.js`.
- La SPA redirige a `/login` al recibir un 401 y vuelve a la vista tras entrar.
- En Ajustes aparece «Salir» cuando la auth está activa.

### Notas

- La contraseña se compara en tiempo constante; la cookie y los tokens se
  firman con HMAC-SHA256.
- Si cambias `AUTH_PASSWORD` sin `AUTH_SECRET`, las cookies/tokens viejos dejan
  de valer (el secreto se deriva de la contraseña).
- Para HTTPS detrás de un proxy, añade `Secure` a la cookie si corresponde.

## i18n (ES/EN)

- Selector de idioma en **Ajustes** (Español / English); se guarda en
  `localStorage.lang` y se detecta `navigator.language` la primera vez.
- Traducidos: navegación, buscador, botones principales (Reproducir, Volver,
  Guardar, Compartir, Descargar ZIP, Reintentar), ajustes y mensajes de estado.
- Los títulos/sinopsis de las fuentes son contenido y no se traducen.
- El idioma del documento (`<html lang>`) se actualiza al cambiar.

## Pendiente

- Traducción de subtítulos con IA (descartada por el usuario).
- Discover/recomendaciones.
- Tests end-to-end de auth en CI.

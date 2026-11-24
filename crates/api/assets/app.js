// PiStreaming SPA — router por hash + vistas. Sin dependencias ni build.
const app = document.getElementById("app");
const nav = document.getElementById("nav");
const banner = document.getElementById("banner");

// --- Cachés en memoria para navegación instantánea ------------------------
// planCache: el plan ya llegó con el POST de «Reproducir»; viewPlayer lo
// consume sin el GET extra a /api/play/:session. searchCache: resultados por
// (kind, query) para repintar la grilla al instante al volver a Buscar.
// posterCache/preDetail: lo que ya sabemos de una tarjeta clickeada, para
// pintar la ficha antes del fetch. streamsPrefetch: /api/streams en vuelo
// (disparado en el hover/touch de la tarjeta, antes de entrar a la ficha).
const planCache = new Map();
const searchCache = new Map();
const posterCache = new Map();
const streamsPrefetch = new Map();
let preDetail = null;

// Generación de ruta: cada `route()` la incrementa; las vistas async comprueban
// que siguen siendo la ruta viva antes de pintar (evita que una vista lenta pise
// la actual al navegar rápido). `activeAbort` cancela los fetch en vuelo al
// navegar.
let routeSeq = 0;
let activeAbort = null;
const stale = (seq) => seq !== routeSeq;
function abortActive() {
  if (activeAbort) {
    activeAbort.abort();
    activeAbort = null;
  }
}

// Limpieza del reproductor montado (interval de progreso + listeners), para que
// reintentar/remontar no deje PUTs huérfanos ni timers vivos.
let activePlayerDispose = null;
function disposeActivePlayer() {
  if (activePlayerDispose) {
    try { activePlayerDispose(); } catch (_) {}
    activePlayerDispose = null;
  }
}

function detailKey(kind, id) {
  return `${kind}:${id}`;
}

// Dispara (una sola vez) la búsqueda de fuentes de un título. Se usa en el
// hover/touch de una tarjeta (prefetch) y al entrar a la ficha (reuso).
function prefetchStreams(kind, id) {
  const key = detailKey(kind, id);
  if (!streamsPrefetch.has(key)) {
    streamsPrefetch.set(
      key,
      api(`/api/streams/${kind}/${encodeURIComponent(id)}`, { navigateAbort: false }).catch(() => {
        // No cachear el fallo (ni un abort por navegación): si no, volver a la
        // ficha mostraría "Sin fuentes" hasta recargar la página entera.
        streamsPrefetch.delete(key);
        return { streams: [] };
      }),
    );
  }
  return streamsPrefetch.get(key);
}

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k === "class") node.className = v;
    else if (k.startsWith("on") && typeof v === "function") node.addEventListener(k.slice(2), v);
    else if (v === true) node.setAttribute(k, "");
    else node.setAttribute(k, v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    node.append(c.nodeType ? c : document.createTextNode(String(c)));
  }
  return node;
}

// Icono SVG inline (sin dependencias). `paths` son valores del atributo `d`.
function svgIcon(paths, size = 20) {
  const ns = "http://www.w3.org/2000/svg";
  const svg = document.createElementNS(ns, "svg");
  for (const [k, v] of Object.entries({
    viewBox: "0 0 24 24", width: size, height: size, fill: "none",
    stroke: "currentColor", "stroke-width": "1.9",
    "stroke-linecap": "round", "stroke-linejoin": "round",
    "aria-hidden": "true", focusable: "false",
  })) svg.setAttribute(k, v);
  for (const d of paths) {
    const p = document.createElementNS(ns, "path");
    p.setAttribute("d", d);
    svg.append(p);
  }
  return svg;
}

// --- Pistas de audio / subtítulos ----------------------------------------
// `GET /api/play/:session` puede traer `audio_tracks`/`subtitle_tracks`; los
// planes viejos cacheados no las incluyen, así que normalizamos a arrays.
function normalizeTracks(plan) {
  const p = plan || {};
  return {
    audio: Array.isArray(p.audio_tracks) ? p.audio_tracks : [],
    subs: Array.isArray(p.subtitle_tracks) ? p.subtitle_tracks : [],
    activeAudio: Number.isInteger(p.active_audio_index) ? p.active_audio_index : null,
    route: p.route || null,
  };
}

// Título si existe; si no, idioma en mayúsculas; si no, «Pista N».
function trackLabel(t, i) {
  const title = t && typeof t.title === "string" ? t.title.trim() : "";
  if (title) return title;
  const lang = t && typeof t.language === "string" ? t.language.trim() : "";
  if (lang) return lang.toUpperCase();
  return `Pista ${i + 1}`;
}

const CODEC_LABEL = {
  subrip: "SRT", srt: "SRT", ass: "ASS", ssa: "ASS", webvtt: "VTT", vtt: "VTT",
  mov_text: "MOV_TEXT", ttml: "TTML",
  aac: "AAC", ac3: "AC3", eac3: "EAC3", dts: "DTS", opus: "Opus", vorbis: "Vorbis",
  mp3: "MP3", flac: "FLAC", truehd: "TrueHD", pcm_s16le: "PCM",
};

// Metadato secundario (codec/forzado). No es imprescindible: null si no hay.
function trackMeta(t) {
  if (!t) return null;
  const bits = [];
  if (t.codec) {
    const c = String(t.codec).toLowerCase();
    bits.push(CODEC_LABEL[c] || String(t.codec).toUpperCase());
  }
  if (t.forced) bits.push("Forzado");
  return bits.length ? bits.join(" · ") : null;
}

function showBanner(msg) {
  banner.textContent = String(msg || "Error");
  banner.hidden = false;
  clearTimeout(showBanner._t);
  showBanner._t = setTimeout(() => { banner.hidden = true; }, 6000);
}

const DEFAULT_FETCH_TIMEOUT_MS = 30000;

async function api(path, opts = {}) {
  // `timeoutMs` es opcional (default: 30 s). Con `navigateAbort` (default true)
  // la petición se cancela al cambiar de vista; los fetch de fondo (prefetch de
  // fuentes, warm) lo desactivan para sobrevivir a la navegación.
  const { timeoutMs = DEFAULT_FETCH_TIMEOUT_MS, signal, navigateAbort = true, ...fetchOpts } = opts;
  const ctrl = new AbortController();
  let timedOut = false;
  const timer = timeoutMs
    ? setTimeout(() => { timedOut = true; ctrl.abort(); }, timeoutMs)
    : undefined;
  const signals = [ctrl.signal];
  if (signal) signals.push(signal);
  if (navigateAbort && activeAbort) signals.push(activeAbort.signal);
  fetchOpts.signal = signals.length > 1 && AbortSignal.any
    ? AbortSignal.any(signals)
    : ctrl.signal;
  let res;
  try {
    res = await fetch(path, fetchOpts);
  } catch (e) {
    if (e && e.name === "AbortError") {
      if (timedOut) {
        const te = new Error("Tiempo de espera agotado");
        te.timeout = true;
        throw te;
      }
      // Cancelado por navegación: no es un error que deba pintarse.
      const ae = new Error("cancelado");
      ae.aborted = true;
      throw ae;
    }
    throw e;
  } finally {
    clearTimeout(timer);
  }
  if (!res.ok) {
    if (res.status === 401) {
      // Sesión caducada o auth activada: vuelve al login y regresa aquí.
      const next = encodeURIComponent(location.pathname + location.hash);
      location.href = `/login?next=${next}`;
      const err = new Error("no autorizado");
      err.status = 401;
      err.aborted = true;
      throw err;
    }
    let msg = `Error ${res.status}`;
    try {
      const j = await res.json();
      if (j.error) msg = j.error;
    } catch (_) {}
    // Conservamos el status en el Error para que los llamadores distingan casos
    // específicos (p. ej. 507 `no_space`).
    const err = new Error(msg);
    err.status = res.status;
    throw err;
  }
  if (res.status === 204) return null;
  const ct = res.headers.get("content-type") || "";
  return ct.includes("application/json") ? res.json() : res.text();
}

function setTab(tab) {
  if (!nav) return;
  for (const a of nav.querySelectorAll("a")) a.classList.toggle("active", a.dataset.tab === tab);
}

const loading = () => el("div", { class: "state" }, el("div", { class: "spinner" }), t("loading"));
const empty = (msg) => el("div", { class: "state" }, msg);
function errorBox(msg, retry) {
  return el("div", { class: "state error" }, el("p", {}, msg),
    retry ? el("button", { class: "btn", onclick: retry }, t("retry")) : null);
}

async function route() {
  const seq = ++routeSeq;
  abortActive();
  activeAbort = new AbortController();
  const raw = location.hash.replace(/^#\/?/, "");
  const parts = raw.split("/");
  const head = parts[0] || "search";
  app.replaceChildren(loading());
  try {
    if (head === "search") return await viewSearch(parts[1]);
    if (head === "detail") return await viewDetail(parts[1], parts.slice(2).join("/"), seq);
    if (head === "player") return await viewPlayer(parts[1], seq);
    if (head === "library") {
      if (parts[1]) return await viewLibraryPlayer(decodeURIComponent(parts.slice(1).join("/")), seq);
      return await viewLibrary(seq);
    }
    if (head === "addons") return await viewAddons(seq);
    if (head === "settings") return await viewSettings(seq);
    app.replaceChildren(empty("Ruta desconocida"));
  } catch (e) {
    if (e && e.aborted) return; // navegación: no pises la vista nueva
    showBanner(e.message);
    app.replaceChildren(errorBox(e.message, route));
  }
}

// --- Buscar ---------------------------------------------------------------
const KINDS = [
  { id: "movie", labelKey: "movies" },
  { id: "series", labelKey: "series" },
];
const KIND_LABEL = { movie: "películas", series: "series" };

// --- i18n mínimo (ES/EN) ---------------------------------------------------
const I18N = {
  es: {
    navSearch: "Buscar", navLibrary: "Biblioteca", navAddons: "Addons", navSettings: "Ajustes",
    movies: "Películas", series: "Series",
    searchMovie: "Buscar películas…", searchSeries: "Buscar series…",
    search: "Buscar", loading: "Cargando…", retry: "Reintentar", back: "Volver",
    backToDetail: "Volver a la ficha", play: "Reproducir", save: "Guardar",
    share: "Compartir", downloadZip: "Descargar ZIP", noSources: "Sin fuentes disponibles",
    settings: "Ajustes", language: "Idioma", logout: "Salir", saveSettings: "Guardar",
    requiresRestart: "requiere reinicio",
    label_cache_max_gb: "Caché máxima (GB)", label_cache_ttl_hours: "TTL de caché (horas)",
    label_egress_bind: "Interfaz de salida", label_http_port: "Puerto HTTP",
    label_data_dir: "Directorio de datos",
  },
  en: {
    navSearch: "Search", navLibrary: "Library", navAddons: "Addons", navSettings: "Settings",
    movies: "Movies", series: "Series",
    searchMovie: "Search movies…", searchSeries: "Search series…",
    search: "Search", loading: "Loading…", retry: "Retry", back: "Back",
    backToDetail: "Back to details", play: "Play", save: "Save",
    share: "Share", downloadZip: "Download ZIP", noSources: "No sources available",
    settings: "Settings", language: "Language", logout: "Sign out", saveSettings: "Save",
    requiresRestart: "requires restart",
    label_cache_max_gb: "Max cache (GB)", label_cache_ttl_hours: "Cache TTL (hours)",
    label_egress_bind: "Egress interface", label_http_port: "HTTP port",
    label_data_dir: "Data directory",
  },
};
let LANG = "es";
try {
  const saved = localStorage.getItem("lang");
  if (saved === "es" || saved === "en") LANG = saved;
  else if ((navigator.language || "").toLowerCase().startsWith("en")) LANG = "en";
} catch (_) {}

function t(key) {
  const d = I18N[LANG] || I18N.es;
  return d[key] || I18N.es[key] || key;
}

function applyLang() {
  document.documentElement.lang = LANG;
  const nav = document.getElementById("nav");
  if (!nav) return;
  const labels = {
    search: t("navSearch"), library: t("navLibrary"),
    addons: t("navAddons"), settings: t("navSettings"),
  };
  for (const a of nav.querySelectorAll("a")) {
    const k = a.dataset.tab;
    if (labels[k]) a.textContent = labels[k];
  }
}

function normalizeKind(k) {
  return k === "series" ? "series" : "movie";
}

// El tipo se refleja en el hash; «movie» se mantiene como #/search pelado.
function searchHash(kind) {
  return kind === "movie" ? "#/search" : `#/search/${kind}`;
}

async function viewSearch(rawKind) {
  setTab("search");
  let kind = normalizeKind(rawKind);
  const input = el("input", { name: "q", placeholder: "Buscar…" });
  const buttons = {};
  const applyKind = () => {
    for (const k of KINDS) {
      const on = k.id === kind;
      buttons[k.id].classList.toggle("active", on);
      buttons[k.id].setAttribute("aria-pressed", on ? "true" : "false");
    }
    input.placeholder = t(kind === "series" ? "searchSeries" : "searchMovie");
    // Reflejar el tipo en el hash sin recargar la vista (sobrevive reload/back).
    const hash = searchHash(kind);
    if (location.hash !== hash) history.replaceState(null, "", hash);
  };
  const chooseKind = (next) => {
    kind = normalizeKind(next);
    applyKind();
    const q = input.value.trim();
    if (q) runSearch(q, kind);
  };
  const segmented = el("div",
    { class: "segmented", role: "group", "aria-label": "Tipo de búsqueda" },
    ...KINDS.map((k) => {
      const b = el("button", {
        type: "button",
        class: "segmented-btn",
        "aria-pressed": "false",
        onclick: () => chooseKind(k.id),
      }, t(k.labelKey));
      buttons[k.id] = b;
      return b;
    }),
  );
  const form = el("form", { class: "searchbar", onsubmit: async (e) => {
    e.preventDefault();
    clearTimeout(debounceT);
    const q = input.value.trim();
    if (q) await runSearch(q, kind);
  }},
    segmented,
    input,
    el("button", { class: "btn primary", type: "submit" }, "Buscar"),
  );
  app.replaceChildren(el("h1", {}, "Buscar"), form,
    el("div", { id: "results", class: "grid" }));
  // Búsqueda al escribir (debounce): resultados mientras se tipea, sin esperar
  // al botón. El submit (Enter) cancela el debounce pendiente.
  let debounceT;
  input.addEventListener("input", () => {
    clearTimeout(debounceT);
    const q = input.value.trim();
    if (q.length < 2) return;
    debounceT = setTimeout(() => runSearch(q, kind), 350);
  });
  const last = sessionStorage.getItem("lastQuery");
  if (last) input.value = last;
  applyKind();
  if (last) await runSearch(last, kind);
}

let searchSeq = 0;
async function runSearch(q, kind = "movie") {
  kind = normalizeKind(kind);
  sessionStorage.setItem("lastQuery", q);
  const results = document.getElementById("results");
  if (!results) return;
  const seq = ++searchSeq;
  const key = `${kind}:${q.toLowerCase()}`;
  // Resultados cacheados del cliente: la grilla aparece ya (posters del caché
  // del browser) y el refetch en background refresca por si cambió algo.
  const cached = searchCache.get(key);
  if (cached) {
    results.replaceChildren(...cached.map(metaCard));
  } else {
    results.replaceChildren(loading());
  }
  let data;
  try {
    data = await api(`/api/search?query=${encodeURIComponent(q)}&kind=${kind}`);
  } catch (e) {
    if (e && e.aborted) return;
    if (seq !== searchSeq) return;
    // Antes esto dejaba "Cargando…" para siempre y un UnhandledPromiseRejection.
    results.replaceChildren(errorBox(e.message, () => runSearch(q, kind)));
    return;
  }
  if (seq !== searchSeq) return; // descartá respuestas obsoletas
  const metas = data.metas || [];
  searchCache.set(key, metas);
  if (!metas.length) return results.replaceChildren(empty(`Sin resultados de ${KIND_LABEL[kind]}`));
  results.replaceChildren(...metas.map(metaCard));
}

// Devuelve una URL http/https normalizada, o null si el poster no es válido.
// Evita inyectar declaraciones CSS por concatenación de strings.
function safePosterUrl(poster) {
  if (!poster) return null;
  try {
    const url = new URL(poster);
    if (url.protocol === "http:" || url.protocol === "https:") return url.href;
  } catch (_) {}
  return null;
}

function metaCard(m) {
  const kind = m.type || "movie";
  const id = m.id;
  const key = detailKey(kind, id);
  const poster = safePosterUrl(m.poster || m.background || "");
  if (m.name || poster) posterCache.set(key, { name: m.name || id, poster });
  // <img> con lazy/async: el browser prioriza lo visible y no bloquea el parseo.
  const img = poster ? el("img", {
    class: "poster-img",
    src: poster,
    alt: "",
    loading: "lazy",
    decoding: "async",
  }) : null;
  const cover = el("div", { class: "poster" },
    img || el("span", { class: "poster-fallback" }, (m.name || "?").slice(0, 1)));
  // Prefetch de fuentes al asomar el puntero / primer toque: al entrar a la
  // ficha los streams ya están (o vienen en camino), sin espera extra.
  const warm = () => prefetchStreams(kind, id);
  return el("a", {
    class: "card",
    href: `#/detail/${kind}/${encodeURIComponent(id)}`,
    onmouseenter: warm,
    ontouchstart: warm,
    onclick: () => { preDetail = { key, name: m.name || id, poster }; },
  },
    cover,
    el("div", { class: "card-title" }, m.name || m.id),
  );
}

// --- Detalle --------------------------------------------------------------
// Encabezado de la ficha (poster + título + sinopsis).
function detailHead(m, id) {
  const poster = safePosterUrl(m.poster || "");
  const cover = el("div", { class: "poster large" },
    poster
      ? el("img", { class: "poster-img", src: poster, alt: "", decoding: "async" })
      : el("span", { class: "poster-fallback" }, (m.name || id || "?").slice(0, 1)));
  return el("div", { class: "detail-head" },
    cover,
    el("div", { class: "detail-meta" },
      el("h1", {}, m.name || id),
      m.releaseInfo ? el("p", { class: "muted" }, m.releaseInfo) : null,
      m.description ? el("p", { class: "desc" }, m.description) : null,
    ),
  );
}

async function viewDetail(kind, id, seq) {
  setTab("search");
  if (!kind || !id) { app.replaceChildren(empty("Ficha inválida")); return; }
  id = decodeURIComponent(id);
  const key = detailKey(kind, id);
  // Pintado instantáneo con lo aprendido del clic (poster + nombre): la
  // navegación lista → ficha no muestra nunca un spinner vacío.
  const pre = (preDetail && preDetail.key === key)
    ? preDetail
    : (posterCache.get(key) || null);
  preDetail = null;
  app.replaceChildren(
    detailHead(pre ? { name: pre.name, poster: pre.poster } : { name: id }, id),
    el("h2", { class: "section" }, "Fuentes"),
    loading(),
  );
  // `streams` via prefetch (ya en vuelo desde la tarjeta) o fetch normal.
  const [meta, streamsData] = await Promise.all([
    api(`/api/meta/${kind}/${encodeURIComponent(id)}`).catch(() => null),
    prefetchStreams(kind, id),
  ]);
  if (stale(seq)) return; // el usuario ya navegó a otra vista
  const m = meta || { name: id };
  // Sólo actualizar la caché con meta real: si el fetch falló, no pises lo
  // aprendido del clic (nombre/póster de la tarjeta).
  if (meta && (m.name || m.poster)) {
    posterCache.set(key, { name: m.name || id, poster: safePosterUrl(m.poster || "") });
  }

  // --- Series: selector de temporada/episodio -----------------------------
  // Los addons Stremio traen los episodios en `meta.videos`. El id de fuentes
  // pasa a ser `tt…:temporada:episodio`, que es lo que consumen los streams.
  const videos = (kind === "series" && Array.isArray(m.videos))
    ? m.videos.filter((v) => v && v.season != null && v.episode != null)
    : [];
  if (!videos.length) {
    // Película o serie sin lista de episodios: flujo simple de siempre.
    mountSources(m, streamsData.streams || [], kind, id);
    return;
  }

  // Episodio actual: el del hash (`tt…:T:E`), o el último emitido, o el último
  // de la lista. La fecha de emisión evita preseleccionar capítulos futuros.
  const byId = new Map(videos.map((v) => [v.id, v]));
  let current = byId.get(id)
    || (id.includes(":")
      ? { id, season: parseInt(id.split(":")[1], 10), episode: parseInt(id.split(":")[2], 10), name: null }
      : null);
  if (!current) {
    const now = Date.now();
    const aired = videos
      .filter((v) => v.released && Date.parse(v.released) > 0 && Date.parse(v.released) < now)
      .sort((a, b) => (a.season - b.season) || (a.episode - b.episode));
    current = aired[aired.length - 1]
      || videos.slice().sort((a, b) => (a.season - b.season) || (a.episode - b.episode))[videos.length - 1];
  }

  const seasons = [...new Set(videos.map((v) => v.season))].sort((a, b) => a - b);
  let curSeason = seasons.includes(current.season) ? current.season : seasons[0];
  const epsOf = (s) => videos
    .filter((v) => v.season === s)
    .sort((a, b) => (a.episode || 0) - (b.episode || 0));

  const seasonSel = el("select", {
    class: "sel", "aria-label": "Temporada",
    onchange: (e) => { curSeason = Number(e.target.value); renderEps(); },
  }, seasons.map((s) => el("option", { value: String(s), selected: s === curSeason }, `Temporada ${s}`)));
  const epSel = el("select", {
    class: "sel", "aria-label": "Episodio",
    onchange: (e) => pickEpisode(Number(e.target.value)),
  });
  const renderEps = () => {
    epSel.replaceChildren(...epsOf(curSeason).map((v) => el("option", {
      value: String(v.episode),
      selected: v.season === current.season && v.episode === current.episode,
    }, v.name ? `E${v.episode} · ${v.name}` : `Episodio ${v.episode}`)));
  };
  renderEps();

  const selector = el("div", { class: "episodes" },
    el("div", { class: "ep-selects" },
      el("label", { class: "ep-field" }, el("span", {}, "Temporada"), seasonSel),
      el("label", { class: "ep-field" }, el("span", {}, "Episodio"), epSel),
    ),
    el("p", { class: "muted ep-now", "aria-live": "polite" }),
  );
  const epNow = selector.querySelector(".ep-now");
  const describeEp = (v) => {
    const bits = [`T${v.season} · E${v.episode}`];
    if (v.name) bits.push(v.name);
    if (v.released && Date.parse(v.released) > 0) {
      const d = new Date(v.released);
      if (!Number.isNaN(d.getTime())) bits.push(d.toLocaleDateString("es", { day: "numeric", month: "short", year: "numeric" }));
    }
    return bits.join(" · ");
  };
  epNow.textContent = describeEp(current);

  function pickEpisode(ep) {
    const v = epsOf(curSeason).find((x) => x.episode === ep);
    if (!v || (v.season === current.season && v.episode === current.episode)) return;
    current = v;
    epNow.textContent = describeEp(v);
    // El episodio queda en el hash (compartible y sobrevive un reload) sin
    // re-lanzar la ruta: sólo se recarga la sección de fuentes.
    history.replaceState(null, "", `#/detail/${kind}/${encodeURIComponent(v.id)}`);
    loadSources(v.id);
  }

  const sourcesBox = el("div");
  const loadSources = (sourcesId) => {
    sourcesBox.replaceChildren(loading());
    prefetchStreams(kind, sourcesId).then((data) => {
      if (stale(seq)) return;
      mountSources(m, data.streams || [], kind, sourcesId, sourcesBox);
    }).catch(() => {
      if (!stale(seq)) {
        sourcesBox.replaceChildren(errorBox("No se pudieron cargar las fuentes", () => loadSources(sourcesId)));
      }
    });
  };

  app.replaceChildren(
    detailHead(m, id),
    selector,
    el("h2", { class: "section" }, "Fuentes"),
    sourcesBox,
  );
  loadSources(current.id);
}

/// Pinta el panel de fuentes para `sourcesId` (id de película o de episodio)
/// dentro de `box` (o reemplazando la vista entera si no viene box).
function mountSources(m, streams, kind, sourcesId, box) {
  const sources = streams.length
    ? renderSources(streams, kind, sourcesId, m.name)
    : empty(t("noSources"));
  if (box) {
    box.replaceChildren(sources);
  } else {
    // Sin `box` (película): reemplaza la vista entera, sustituyendo el
    // spinner inicial por la ficha + la lista de fuentes.
    app.replaceChildren(
      detailHead(m, sourcesId),
      el("h2", { class: "section" }, "Fuentes"),
      sources,
    );
  }
  // Pre-warm: calienta el torrent de la mejor fuente mientras el usuario
  // revisa la lista. Fire-and-forget: si falla (o tarda), play lo resuelve.
  warmBestSource(streams);
}

// Hashes ya calentados en esta sesión del navegador (evita re-POST inútil;
// el server es idempotente, pero igualmente sobra).
const warmedHashes = new Set();

/// POST /api/warm para una fuente concreta (idempotente por hash en esta
/// sesión del navegador). Se usa al entrar a la ficha (mejor fuente) y al
/// asomar el puntero/tocar una fila, para que al dar Play el torrent ya esté
/// conectado y el probe no expire.
function warmStream(it) {
  if (!it || !it.playable || !it.raw.infoHash) return;
  const hash = it.raw.infoHash.toLowerCase();
  if (warmedHashes.has(hash)) return;
  warmedHashes.add(hash);
  api("/api/warm", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      info_hash: hash,
      trackers: trackersOf(it.raw),
      fileIdx: it.raw.fileIdx ?? null,
    }),
    navigateAbort: false,
  }).catch(() => warmedHashes.delete(hash)); // falló: permitir reintento
}

// Título que huele a pack/colección (varios títulos en un mismo torrent): no
// conviene calentarlo — la metadata es enorme (el warm expira) y el archivo
// elegido puede no ser la película buscada.
const PACK_RE = /collection|trilog|complete|pack\b|saga|filmograf|antholog|bonanza|redux|\bmovies\b|\bfilms\b|\bvol\.?\s*\d|\bmega\b|\b\d{4}\s*-\s*\d{4}\b|\b\d+\s*-\s*\d+\b/i;

function isPack(it) {
  return PACK_RE.test(`${it.raw.name || ""} ${it.raw.title || ""}`);
}

// Mejor candidata para recomendar/calentar. `items` viene en el orden del
// servidor, que ya rankea por reproducibilidad en navegador (x264 antes que
// HEVC) y luego calidad/seeds: se respeta ese orden en vez de re-ordenar por
// seeds (un 4K x265 con muchos seeds arrancaría y no se vería).
function pickBest(items) {
  const playables = items.filter((it) => it.playable);
  const compat = playables.filter((it) => !it.incompat);
  return compat.find((it) => !isPack(it)) || compat[0]
    || playables.find((it) => !isPack(it)) || playables[0] || null;
}

/// Calienta la misma fuente que la UI marca «Recomendada» (ver `pickBest`), que
/// es la que el usuario tiene más probabilidad de pulsar.
function warmBestSource(streams) {
  try {
    warmStream(pickBest(streams.map(parseStream)));
  } catch (_) {}
}

// --- Preferencia: reproducir el original sin convertir ---------------------
// Por defecto los HEVC/AV1 se convierten a H.264 en el servidor (el navegador
// suele no decodificarlos). Este toggle apaga esa conversión para quien tiene
// decodificador por hardware (o usa un player externo). Persistente.
function forceDirectEnabled() {
  try { return localStorage.getItem("forceDirect") === "1"; } catch (_) { return false; }
}
function setForceDirect(on) {
  try {
    if (on) localStorage.setItem("forceDirect", "1");
    else localStorage.removeItem("forceDirect");
  } catch (_) {}
}

// --- Fuentes: parseo defensivo y panel de filtros -------------------------
// Los addons (p. ej. Torrentio) no exponen campos estructurados: calidad,
// seeds, tamaño e idioma vienen embebidos en `name` + `title`. Parseamos el
// texto con expresiones tolerantes y caemos en "—" cuando no hay dato.
const QUALITY_RANK = { "4K": 4, "1080p": 3, "720p": 2, "480p": 1 };
const QUALITY_VALUES = ["4K", "1080p", "720p", "480p"];
const QUALITY_CLASS = { "4K": "q-4k", "1080p": "q-1080", "720p": "q-720", "480p": "q-480" };
const qualityRank = (q) => QUALITY_RANK[q] || 0;
const seedKey = (it) => (it.seeds == null ? -1 : it.seeds);

function parseSeeds(text) {
  let m = text.match(/👤\s*(\d+)/) || text.match(/(?:👥|🌱)\s*(\d+)/);
  if (m) return Number(m[1]);
  m = text.match(/(\d+)\s*(?:seeders?|seeds?)\b/)
    || text.match(/(?:seeders?|seeds?)\s*[:\-]?\s*(\d+)/);
  return m ? Number(m[1]) : null;
}

function parseQuality(text) {
  if (/\b2160p\b|\b4k\b/.test(text)) return "4K";
  if (/\b1080p\b/.test(text)) return "1080p";
  if (/\b720p\b/.test(text)) return "720p";
  if (/\b480p\b/.test(text)) return "480p";
  return "—";
}

function parseSize(text) {
  const m = text.match(/💾\s*([\d.,]+)\s*(gb|mb|gib|mib)\b/);
  if (!m) return "—";
  const unit = { gb: "GB", mb: "MB", gib: "GiB", mib: "MiB" }[m[2]];
  return `${m[1]} ${unit}`;
}

function sizeToMB(size) {
  const m = String(size).match(/([\d.,]+)\s*(gb|mb|gib|mib)\b/i);
  if (!m) return -1;
  const n = parseFloat(m[1].replace(",", "."));
  if (!isFinite(n)) return -1;
  return /^g/i.test(m[2]) ? n * 1024 : n;
}

// Banderas de país → idioma. Torrentio/Comet marcan el idioma con banderas.
const FLAG_LANG = {
  "🇬🇧": "Inglés", "🇺🇸": "Inglés", "🇨🇦": "Inglés", "🇦🇺": "Inglés",
  "🇪🇸": "Castellano",
  "🇲🇽": "Latino", "🇦🇷": "Latino", "🇨🇱": "Latino", "🇨🇴": "Latino", "🇵🇪": "Latino",
  "🇮🇳": "Hindi", "🇯🇵": "Japonés", "🇰🇷": "Coreano", "🇫🇷": "Francés",
  "🇩🇪": "Alemán", "🇮🇹": "Italiano", "🇵🇹": "Portugués", "🇧🇷": "Portugués",
  "🇷🇺": "Ruso", "🇨🇳": "Chino", "🇸🇦": "Árabe", "🇺🇦": "Ucraniano",
  "🇳🇱": "Neerlandés", "🇭🇺": "Húngaro", "🇩🇰": "Danés", "🇸🇪": "Sueco",
  "🇳🇴": "Noruego", "🇹🇷": "Turco", "🇮🇩": "Indonesio", "🇵🇱": "Polaco",
  "🇨🇿": "Checo", "🇭🇷": "Croata", "🇹🇭": "Tailandés", "🇫🇮": "Finés",
};

function parseLang(text) {
  // El español primero: es lo que más importa para filtrar.
  if (/castellano|español|\besp\b|\bcast\b|🇪🇸/.test(text)) return "Castellano";
  if (/latino|\blat\b|\blatam\b|🇲🇽|🇦🇷|🇨🇱|🇨🇴|🇵🇪/.test(text)) return "Latino";
  if (/vose|\bvos\b|subtitulad/.test(text)) return "VOSE";
  if (/\bmulti\b/.test(text)) return "MULTI";
  if (/\bdual\b/.test(text)) return "Dual";
  // Primera bandera reconocida.
  const flag = text.match(/[\u{1F1E6}-\u{1F1FF}]{2}/u);
  if (flag && FLAG_LANG[flag[0]]) return FLAG_LANG[flag[0]];
  // Palabras / siglas.
  if (/ingl[eé]s|\benglish\b|\beng\b|\bing\b/.test(text)) return "Inglés";
  if (/hindi/.test(text)) return "Hindi";
  if (/japanese|japon[eé]s|\bjap\b/.test(text)) return "Japonés";
  if (/korean|coreano|\bkor\b/.test(text)) return "Coreano";
  if (/french|franc[eé]s|\bfre\b|\bvff\b/.test(text)) return "Francés";
  if (/german|alem[aá]n|\bger\b/.test(text)) return "Alemán";
  if (/italian|italiano|\bita\b/.test(text)) return "Italiano";
  if (/portuguese|portugu[eé]s|\bpor\b/.test(text)) return "Portugués";
  if (/russian|ruso|\brus\b/.test(text)) return "Ruso";
  if (/chinese|chino|\bchi\b/.test(text)) return "Chino";
  return "—";
}

function parseProvider(s) {
  if (s.source_addon) return String(s.source_addon);
  const first = String(s.name || "").split(/\r?\n/).map((x) => x.trim()).filter(Boolean)[0];
  if (!first) return "—";
  // Si la primera línea es en realidad la fila de datos (emoji, seeds) no es
  // un proveedor; y si no hay ninguna palabra real, tampoco.
  if (/[💾👤👥🌱]/.test(first) || /\bseed(?:er)?s?\b/i.test(first)) return "—";
  if (!/[A-Za-zÀ-ÿ]{3,}/.test(first)) return "—";
  return first;
}

function parseStream(s, index) {
  const hay = `${s.name || ""} ${s.title || ""}`.toLowerCase();
  const seeds = parseSeeds(hay);
  const size = parseSize(hay);
  return {
    raw: s,
    index,
    seeds,
    size,
    sizeMB: sizeToMB(size),
    quality: parseQuality(hay),
    lang: parseLang(hay),
    provider: parseProvider(s),
    // Conservamos `sources` (trackers/dht del addon): `startPlay` lo necesita
    // para armar el magnet con `&tr=`. Sin esto quedaba descartado.
    sources: Array.isArray(s.sources) ? s.sources : [],
    // Codec incompatible con el navegador (HEVC/10bit/DV) o null.
    incompat: codecWarn(s),
    // Reproducible: fuente presente y (seeds > 0 o url directa).
    playable: !!(s.infoHash || s.url) && ((seeds != null && seeds > 0) || !!s.url),
  };
}

const badge = (text, cls) => el("span", { class: cls }, text);

// Marcadores de codecs que el navegador NO decodifica (y el Pi no puede
// transcodificar video): HEVC/x265/H.265, 10-bit y Dolby Vision. Devuelve un
// resumen (p. ej. "x265 · 10bit") o null si la fuente es compatible.
function codecWarn(s) {
  const hay = `${s.name || ""} ${s.title || ""}`.toLowerCase();
  const hits = [];
  if (/\bhevc\b|\bx265\b|\bh\.?265\b/.test(hay)) hits.push("x265/HEVC");
  if (/\b10.?bit\b/.test(hay)) hits.push("10bit");
  if (/\bdolby ?vision\b|\bdv\b/.test(hay)) hits.push("Dolby Vision");
  // 4K/UHD/HDR/AV1/VP9: el Pi no puede transcodificar 4K en tiempo real y
  // muchos navegadores móviles no decodifican HEVC/AV1. Se ocultan por defecto
  // con «Solo compatibles con el navegador».
  if (/\b2160p\b|\b4k\b|\buhd\b|\bhdr\b|\bav1\b|\bvp9\b/.test(hay)) hits.push("4K/HEVC/AV1");
  return hits.length ? hits.slice(0, 2).join(" · ") : null;
}

function sourceRow(it, isBest, kind, id, title) {
  const s = it.raw;
  const badges = el("div", { class: "badges" },
    badge(it.quality, `badge quality ${QUALITY_CLASS[it.quality] || ""}`),
    badge(`👤 ${it.seeds == null ? "—" : it.seeds}`, `badge seeds${it.seeds > 0 ? " ok" : ""}`),
    badge(`💾 ${it.size}`, "badge size"),
    badge(it.lang, "badge lang"),
    badge(it.provider, "badge provider"),
  );
  if (!it.playable) badges.append(badge("sin seeds", "badge warn"));
  if (it.incompat) {
    badges.append(badge(
      forceDirectEnabled()
        ? `⚠️ ${it.incompat} (original, sin convertir)`
        : `⚠️ ${it.incompat} → se convierte a H264`,
      "badge badcodec",
    ));
  }
  // Pre-warm por fila: al asomar el puntero/tocar, el torrent de ESA fuente se
  // calienta antes de que el usuario llegue a pulsar «Reproducir».
  const warm = () => warmStream(it);
  return el("div", {
    class: `row-item source-row${isBest ? " is-best" : ""}`,
    onmouseenter: warm,
    ontouchstart: warm,
  },
    el("div", { class: "source-main" },
      el("div", { class: "source-name" },
        isBest ? el("span", { class: "pill-best" }, "Recomendada") : null,
        el("strong", {}, s.name || "Fuente"),
      ),
      el("div", { class: "muted source-sub" }, s.title || s.infoHash || s.url || ""),
      badges,
    ),
    el("div", { class: "source-action" },
      el("button", { class: "btn primary", onclick: (e) => startPlay(e.currentTarget, s, kind, id, title) }, t("play")),
    ),
  );
}

function renderSources(streams, kind, id, title) {
  const items = streams.map(parseStream);
  const uniq = (arr) => arr.filter((v, i) => arr.indexOf(v) === i);

  const qualities = QUALITY_VALUES.filter((q) => items.some((it) => it.quality === q));
  const langs = uniq(items.map((it) => it.lang)).filter((l) => l !== "—");
  const providers = uniq(items.map((it) => it.provider)).filter((p) => p !== "—");

  // Estado del panel. El orden por defecto SIEMPRE es seeds desc.
  let sortBy = "seeds";
  let quality = null;
  let lang = null;
  let provider = null;
  let onlySeeds = false;
  // Ocultar fuentes que el navegador no puede decodificar (x265/10bit/DV).
  // Por defecto ON sólo si EXISTEN fuentes compatibles: si todas son x265,
  // se muestran todas con su badge de advertencia (mejor que una lista vacía).
  let hideIncompat = items.some((it) => !it.incompat);

  const passes = (it, exclude) => {
    if (exclude !== "quality" && quality && it.quality !== quality) return false;
    if (exclude !== "lang" && lang && it.lang !== lang) return false;
    if (exclude !== "provider" && provider && it.provider !== provider) return false;
    if (exclude !== "seeds" && onlySeeds && !(it.seeds > 0)) return false;
    if (exclude !== "incompat" && hideIncompat && it.incompat) return false;
    return true;
  };

  const order = (arr) => arr.slice().sort((a, b) => {
    let d = 0;
    if (sortBy === "quality") d = qualityRank(b.quality) - qualityRank(a.quality);
    else if (sortBy === "size") d = b.sizeMB - a.sizeMB;
    if (d) return d;
    d = seedKey(b) - seedKey(a);                        // más seeds primero
    if (d) return d;
    d = qualityRank(b.quality) - qualityRank(a.quality); // empates: calidad desc
    if (d) return d;
    return a.index - b.index;                           // y luego orden original
  });

  const makeChip = (label, onSelect) => {
    const count = el("span", { class: "count" });
    const btn = el("button", {
      type: "button", class: "chip", "aria-pressed": "false", onclick: onSelect,
    }, label, count);
    return {
      el: btn,
      count,
      setActive: (on) => {
        btn.classList.toggle("active", on);
        btn.setAttribute("aria-pressed", on ? "true" : "false");
      },
    };
  };

  const listEl = el("div", { class: "list" });
  const countEl = el("p", { class: "sources-count muted", role: "status" });

  // Orden (por defecto: Seeds ↓).
  const sortDefs = [
    { id: "seeds", label: "Seeds ↓" },
    { id: "quality", label: "Calidad" },
    { id: "size", label: "Tamaño" },
  ];
  const sortBtns = sortDefs.map((d) => el("button", {
    type: "button", class: "segmented-btn", "aria-pressed": "false",
    onclick: () => { sortBy = d.id; refresh(); },
  }, d.label));
  const sortRow = el("div", { class: "filter-row" },
    el("span", { class: "filter-label" }, "Ordenar"),
    el("div", { class: "segmented", role: "group", "aria-label": "Ordenar fuentes" }, ...sortBtns),
  );

  // Calidad.
  const qAll = makeChip("Todas", () => { quality = null; refresh(); });
  const qChips = qualities.map((q) => ({
    value: q,
    chip: makeChip(q, () => { quality = q; refresh(); }),
  }));
  const qualityRow = qualities.length
    ? el("div", { class: "filter-row" },
        el("span", { class: "filter-label" }, "Calidad"),
        el("div", { class: "chips", role: "group", "aria-label": "Filtrar por calidad" },
          qAll.el, ...qChips.map((c) => c.chip.el)),
      )
    : null;

  // Idioma.
  const lAll = makeChip("Todos", () => { lang = null; refresh(); });
  const lChips = langs.map((l) => ({
    value: l,
    chip: makeChip(l, () => { lang = l; refresh(); }),
  }));
  const langRow = langs.length
    ? el("div", { class: "filter-row" },
        el("span", { class: "filter-label" }, "Idioma"),
        el("div", { class: "chips", role: "group", "aria-label": "Filtrar por idioma" },
          lAll.el, ...lChips.map((c) => c.chip.el)),
      )
    : null;

  // Extras: «Solo con seeds», «Solo compatibles», conversión de HEVC/AV1 y
  // proveedor (si hay más de uno).
  const seedsToggle = el("input", { type: "checkbox",
    onchange: (e) => { onlySeeds = e.target.checked; refresh(); } });
  const compatToggle = el("input", { type: "checkbox", checked: hideIncompat,
    onchange: (e) => { hideIncompat = e.target.checked; refresh(); } });
  const extras = [
    el("label", { class: "switch" }, seedsToggle, "Solo con seeds"),
    el("label", { class: "switch" }, compatToggle, "Solo compatibles con el navegador"),
  ];
  if (items.some((it) => it.incompat)) {
    const fd = el("input", { type: "checkbox", checked: forceDirectEnabled(),
      onchange: (e) => { setForceDirect(e.target.checked); refresh(); } });
    extras.push(el("label", { class: "switch" }, fd,
      "Reproducir original (HEVC/AV1) sin convertir"));
  }
  let pAll = null;
  let pChips = [];
  if (providers.length > 1) {
    pAll = makeChip("Todos", () => { provider = null; refresh(); });
    pChips = providers.map((p) => ({
      value: p,
      chip: makeChip(p, () => { provider = p; refresh(); }),
    }));
    extras.push(el("div", { class: "chips", role: "group", "aria-label": "Filtrar por proveedor" },
      pAll.el, ...pChips.map((c) => c.chip.el)));
  }
  const extrasRow = el("div", { class: "filter-row extras" }, ...extras);

  const filters = el("div", { class: "filters", role: "group", "aria-label": "Filtros de fuentes" },
    sortRow, qualityRow, langRow, extrasRow);

  function clearFilters() {
    quality = null;
    lang = null;
    provider = null;
    onlySeeds = false;
    seedsToggle.checked = false;
    hideIncompat = items.some((it) => !it.incompat);
    compatToggle.checked = hideIncompat;
    refresh();
  }

  function refresh() {
    // Contadores por faceta: cada uno cuenta con los demás filtros aplicados.
    if (qualityRow) {
      const base = items.filter((it) => passes(it, "quality"));
      qAll.count.textContent = String(base.length);
      qAll.setActive(quality === null);
      qChips.forEach(({ value, chip }) => {
        chip.count.textContent = String(base.filter((it) => it.quality === value).length);
        chip.setActive(quality === value);
      });
    }
    if (langRow) {
      const base = items.filter((it) => passes(it, "lang"));
      lAll.count.textContent = String(base.length);
      lAll.setActive(lang === null);
      lChips.forEach(({ value, chip }) => {
        chip.count.textContent = String(base.filter((it) => it.lang === value).length);
        chip.setActive(lang === value);
      });
    }
    if (pAll) {
      const base = items.filter((it) => passes(it, "provider"));
      pAll.count.textContent = String(base.length);
      pAll.setActive(provider === null);
      pChips.forEach(({ value, chip }) => {
        chip.count.textContent = String(base.filter((it) => it.provider === value).length);
        chip.setActive(provider === value);
      });
    }
    sortBtns.forEach((b, i) => {
      const on = sortDefs[i].id === sortBy;
      b.classList.toggle("active", on);
      b.setAttribute("aria-pressed", on ? "true" : "false");
    });

    const filtered = items.filter((it) => passes(it, null));
    const ordered = order(filtered);
    countEl.textContent = filtered.length === items.length
      ? `${items.length} ${items.length === 1 ? "fuente" : "fuentes"}`
      : `${filtered.length} de ${items.length} fuentes`;

    if (!ordered.length) {
      listEl.replaceChildren(
        el("div", { class: "state" }, "Ninguna fuente con estos filtros"),
        el("div", { class: "filter-reset" },
          el("button", { type: "button", class: "btn", onclick: clearFilters }, "Limpiar filtros")),
      );
      return;
    }
    // La recomendada es `pickBest` (orden del servidor: compatible/x264, sin
    // packs). Si los filtros del usuario la ocultaron, se cae a la primera
    // reproducible del listado visible.
    const chosen = pickBest(items);
    const playables = ordered.filter((it) => it.playable);
    const best = (chosen && ordered.includes(chosen) ? chosen : null)
      || playables.find((it) => !it.incompat && !isPack(it))
      || playables[0] || ordered[0];
    listEl.replaceChildren(...ordered.map((it, i) => {
      const row = sourceRow(it, it === best, kind, id, title);
      if (i < 9) row.style.animationDelay = `${i * 28}ms`;
      return row;
    }));
  }

  refresh();
  return el("div", { class: "source-panel" }, filters, countEl, listEl);
}

const PLAY_TIMEOUT_MS = 60000;

// Trackers públicos de respaldo. Muchos addons (p. ej. Torrentio) devuelven
// `sources: []`: sin trackers ni DHT, librqbit no descubre peers y ffmpeg se
// queda en «have 0». Estos trackers mantienen viva la discovery en ese caso.
const DEFAULT_TRACKERS = [
  "udp://tracker.opentrackr.org:1337/announce",
  "udp://open.demonii.com:1337/announce",
  "udp://open.stealth.si:80/announce",
  "udp://tracker.torrent.eu.org:451/announce",
  "udp://tracker.tiny-vps.com:6969/announce",
  "udp://exodus.desync.com:6969/announce",
];

// Trackers de un stream para el magnet (`&tr=`): los `tracker:` del addon y,
// si no trae ninguno, los públicos por defecto (discovery no muerta).
function trackersOf(stream) {
  const trackers = (stream.sources || [])
    .filter((src) => typeof src === "string" && src.startsWith("tracker:"))
    .map((src) => src.slice("tracker:".length))
    .filter(Boolean);
  return trackers.length ? trackers : DEFAULT_TRACKERS;
}

async function startPlay(btn, stream, kind, id, title) {
  const seq = routeSeq;
  let magnet = stream.url;
  // Fuentes no-Torrentio (con `url`) se reproducen tal cual; el magnet solo se
  // arma cuando falta la url y hay `infoHash` (caso Torrentio).
  if (!magnet && stream.infoHash) {
    const params = new URLSearchParams();
    params.append("xt", `urn:btih:${stream.infoHash}`);
    for (const tr of trackersOf(stream)) {
      params.append("tr", tr);
    }
    magnet = `magnet:?${params.toString()}`;
  }
  if (!magnet) { showBanner("La fuente no trae magnet ni url"); return; }
  // Feedback inmediato y bloqueo de doble click; se revierte si falla.
  if (btn) {
    btn.disabled = true;
    btn.replaceChildren(el("div", { class: "spinner" }), "Preparando…");
  }
  // Shell del reproductor YA: el usuario ve la pantalla de video (con su
  // «Cargando video…») mientras el server resuelve magnet + probe. La pantalla
  // de reproducción aparece al instante, no después del POST.
  app.replaceChildren(playerShell(title, () => { history.back(); }));
  const playBody = () => JSON.stringify({
    magnet,
    title,
    kind,
    id,
    fileIdx: stream.fileIdx ?? null,
    forceDirect: forceDirectEnabled(),
  });
  try {
    // Un torrent recién agregado puede no haber conectado a ningún par
    // todavía: un 504 con "sin seeds" muchas veces se resuelve al reintento
    // (el warm ya conectó peers mientras tanto). Un reintento automático.
    let plan;
    try {
      plan = await api("/api/play", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: playBody(),
        timeoutMs: PLAY_TIMEOUT_MS,
      });
    } catch (first) {
      if (first && (first.aborted || first.status !== 504)) throw first;
      const st = document.querySelector(".video-status span");
      if (st) st.textContent = "La fuente está fría; reintentando…";
      plan = await api("/api/play", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: playBody(),
        timeoutMs: PLAY_TIMEOUT_MS,
      });
    }
    // Si el usuario navegó mientras se resolvía el plan, no lo arrastres al
    // player (antes esto lo teletransportaba de vuelta desde donde estuviera).
    if (stale(seq)) return;
    // El plan ya está: viewPlayer no necesita el GET a /api/play/:session.
    planCache.set(plan.session, plan);
    // Guardamos cómo lanzar esta sesión para poder re-crearla si el server
    // pierde su registro (`GET /api/play/:session` → 404). El `session_id` es
    // determinista, así que re-POSTear el mismo payload reproduce la sesión.
    rememberReplay(plan.session, {
      magnet,
      url: stream.url || null, // fuentes no-torrent (Direct) reproducen esta url
      title,
      kind,
      id,
      fileIdx: stream.fileIdx ?? null,
      forceDirect: forceDirectEnabled(),
    });
    const hash = `#/player/${encodeURIComponent(plan.session)}`;
    if (location.hash === hash) {
      // Mismo hash: no hay hashchange, montamos el player a mano.
      app.replaceChildren(playerView(plan.playback_url, plan.progress_url, plan.session, title, plan));
    } else {
      location.hash = hash;
    }
  } catch (e) {
    if (e && e.aborted) return; // el usuario navegó: no lo traigas de vuelta
    // `no_space` (507 o el `error` del body): disco lleno en el server.
    const noSpace = e.status === 507 || (e.message || "").includes("no_space");
    showBanner(noSpace
      ? "No hay espacio en disco en el servidor. Borra algo del caché e intenta de nuevo."
      : e.timeout
        ? "No se pudo obtener el enlace de reproducción (¿la fuente no tiene seeds?). Probá otra."
        : e.message);
    if (btn) { btn.disabled = false; btn.textContent = "Reproducir"; }
    // De vuelta a la ficha, donde está el botón que falló. Si el hash ya es ése
    // (no habría `hashchange`), se remonta la vista a mano.
    if (kind && id) {
      const target = `#/detail/${encodeURIComponent(kind)}/${encodeURIComponent(id)}`;
      if (location.hash === target) route();
      else location.hash = target;
    } else {
      route();
    }
  }
}

// --- Reproductor (torrent) ------------------------------------------------
// Shell que se muestra mientras llega el plan (POST /api/play en vuelo):
// pantalla de video con spinner, como la real, para que la transición sea
// percibida como instantánea.
function playerShell(title, onBack) {
  const video = el("video", { class: "video", playsinline: true });
  const status = el("div", { class: "video-status" },
    el("div", { class: "spinner" }), el("span", {}, "Cargando video…"));
  return el("div", { class: "player-wrap" },
    title ? el("h1", { class: "player-title" }, title) : null,
    el("div", { class: "video-wrap" }, video, status),
    el("div", { class: "player-actions" },
      el("button", { class: "btn ghost", onclick: onBack }, t("back"))),
  );
}

async function viewPlayer(session, seq) {
  setTab(null);
  session = decodeURIComponent(session || "");
  // Camino rápido: el plan llegó con el POST de «Reproducir» (startPlay lo
  // cachea) → el player se monta sin ningún fetch adicional.
  const cached = planCache.get(session);
  if (cached) {
    planCache.delete(session);
    app.replaceChildren(playerView(cached.playback_url, cached.progress_url, session, null, cached));
    return;
  }
  let plan;
  try {
    plan = await api(`/api/play/${encodeURIComponent(session)}`);
  } catch (e) {
    if (e && e.aborted) return;
    // 404: la fila de la sesión ya no está en el server (venció o se perdió su
    // registro). Si guardamos cómo lanzarla, la re-creamos en vez de mostrar
    // una pantalla muerta.
    if (e && e.status === 404) return recoverSession(session, seq);
    throw e; // otros errores: `route()` los muestra con reintento
  }
  if (stale(seq)) return;
  app.replaceChildren(playerView(plan.playback_url, plan.progress_url, session, null, plan));
}

// Re-crea una sesión cuyo `GET /api/play/:session` dio 404, usando el payload
// que `startPlay` dejó en `sessionStorage`. Si no hay payload, cae a una
// pantalla accionable (sin inventar la causa de la caída).
async function recoverSession(session, seq) {
  const known = knownSessionIdentity(session);
  const kind = known ? known.kind : null;
  const id = known ? known.id : null;
  const replay = knownReplay(session);
  if (!replay) { renderSessionGone(app, kind, id); return; }
  app.replaceChildren(el("div", { class: "state" },
    el("div", { class: "spinner" }), "Reanudando la reproducción…"));
  try {
    const plan = await api("/api/play", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(replayBody(replay)),
      timeoutMs: PLAY_TIMEOUT_MS,
    });
    if (stale(seq)) return;
    const hash = `#/player/${encodeURIComponent(plan.session)}`;
    // El id es determinista: si ya estamos en ese hash no hay `hashchange`,
    // así que remontamos el player a mano.
    if (location.hash === hash) {
      app.replaceChildren(playerView(plan.playback_url, plan.progress_url, plan.session, null, plan));
    } else {
      location.hash = hash;
    }
  } catch (e) {
    if (e && e.aborted) return;
    showBanner(e.message);
    renderSessionGone(app, kind, id);
  }
}

// --- Reproductor (biblioteca) ---------------------------------------------
async function viewLibraryPlayer(id, seq) {
  setTab("library");
  const data = await api("/api/library");
  if (stale(seq)) return;
  const item = (data.items || []).find((i) => i.id === id);
  if (!item) { app.replaceChildren(empty("No encontrado en la biblioteca")); return; }
  const [kind, ...rest] = id.split(":");
  const progressUrl = `/api/progress/${kind}/${rest.join(":")}`;
  app.replaceChildren(
    playerView(`/library/${encodeURIComponent(id)}/stream`, progressUrl, null, item.title),
  );
}

// Aviso de arranque lento. Generoso: un torrent frío puede tardar en resolver
// metadata + probe + primer fragmento de ffmpeg; avisar a los 12 s daba falsos
// positivos aunque el video arrancara a los pocos segundos.
const VIDEO_STALL_MS = 20000;

// Identidad de meta (`kind`/`id`) para volver a la ficha si la sesión muere.
// Se deriva de la URL de progreso (`/api/progress/<kind>/<id>`, donde `id`
// puede contener `:`) y se recuerda por sesión para el caso de recarga.
function identityFromProgressUrl(progressUrl) {
  if (!progressUrl) return null;
  const marker = "/api/progress/";
  const at = progressUrl.indexOf(marker);
  if (at < 0) return null;
  const rest = progressUrl.slice(at + marker.length);
  const slash = rest.indexOf("/");
  if (slash <= 0 || slash === rest.length - 1) return null;
  return { kind: rest.slice(0, slash), id: rest.slice(slash + 1) };
}

function rememberSessionIdentity(session, kind, id) {
  if (!session || !kind || !id) return;
  try { sessionStorage.setItem(`play:${session}`, JSON.stringify({ kind, id })); } catch (_) {}
}

function knownSessionIdentity(session) {
  if (!session) return null;
  try { return JSON.parse(sessionStorage.getItem(`play:${session}`) || "null"); } catch (_) { return null; }
}

// Payload para re-lanzar la sesión (magnet/url + metadata). `startPlay` lo
// guarda tras resolver el POST; `recoverSession`/`retry` lo re-postean.
function rememberReplay(session, payload) {
  if (!session || !payload) return;
  try { sessionStorage.setItem(`replay:${session}`, JSON.stringify(payload)); } catch (_) {}
}

function knownReplay(session) {
  if (!session) return null;
  try { return JSON.parse(sessionStorage.getItem(`replay:${session}`) || "null"); } catch (_) { return null; }
}

// Normaliza el payload guardado al body que espera `POST /api/play`.
function replayBody(p) {
  return {
    magnet: p.magnet || p.url,
    title: p.title ?? null,
    kind: p.kind ?? null,
    id: p.id ?? null,
    fileIdx: p.fileIdx ?? null,
    forceDirect: p.forceDirect ?? forceDirectEnabled(),
  };
}

// Vuelve a la ficha si se conoce la identidad; si no, al historial.
function backToDetail(kind, id) {
  if (kind && id) location.hash = `#/detail/${encodeURIComponent(kind)}/${encodeURIComponent(id)}`;
  else history.back();
}

// La sesión ya no está en el server y no tenemos cómo re-crearla. Pantalla
// accionable que reusa `.state`/`.player-expired` en vez de dejar un <video>
// roto. No afirmamos la causa (pudo vencer o perderse su registro).
function renderSessionGone(container, kind, id) {
  container.replaceChildren(
    el("div", { class: "state player-expired" },
      el("p", { class: "expired-title" }, "Sesión expirada"),
      el("p", {}, "Esta reproducción ya no está disponible. Volvé a la ficha y tocá Play otra vez."),
      el("button", { class: "btn primary", onclick: () => backToDetail(kind, id) }, t("backToDetail")),
    ),
  );
}

// La sesión existe, pero el stream no arrancó (sin pares, datos ya evictados
// del caché o falló el remux). Distinto de «Sesión expirada»: acá se puede
// reintentar. Reusa la tarjeta de `.player-expired` con acento propio.
function renderStreamError(container, onRetry, onBack, hint) {
  container.replaceChildren(
    el("div", { class: "state player-expired player-stream-error" },
      el("p", { class: "expired-title" }, "No pudimos iniciar la reproducción"),
      hint ? el("p", { class: "expired-hint" }, hint) : null,
      el("p", {}, "La sesión sigue activa, pero el video no arrancó. Puede que la fuente no tenga pares, que los datos ya no estén en el caché o que el servidor no pueda preparar el video. Reintentá o probá otra fuente."),
      el("div", { class: "player-actions" },
        el("button", { class: "btn primary", onclick: (e) => onRetry(e.currentTarget) }, t("retry")),
        el("button", { class: "btn", onclick: onBack }, t("backToDetail")),
      ),
    ),
  );
}

function playerView(src, progressUrl, session, title, plan) {
  // Remontar el player (p. ej. al reintentar) debe limpiar el anterior: si no,
  // queda vivo su interval de progreso haciendo PUT sobre una sesión vieja.
  disposeActivePlayer();
  const identity = identityFromProgressUrl(progressUrl);
  if (identity) rememberSessionIdentity(session, identity.kind, identity.id);

  // HLS para rutas remux/recodeaudio (el backend ya genera playlist/segmentos);
  // directo sigue usando el MP4 con Range (seekable). Fallback al MP4 si hls.js
  // no está o falla.
  const isHls = !!(session && plan && plan.route && plan.route !== "direct");
  let hls = null;
  const video = el("video", { class: "video", controls: true, autoplay: true, playsinline: true, src: isHls ? null : src, poster: session ? `/thumb/${encodeURIComponent(session)}` : null });

  // Capa de estado sobre el video: nunca dejar un rectángulo negro sin explicación.
  const status = el("div", { class: "video-status" },
    el("div", { class: "spinner" }), el("span", {}, "Cargando video…"));
  const wrap = el("div", { class: "video-wrap" }, video, status);

  // Barra de buffer: cuánto video hay descargado por delante. En fuentes
  // torrent es la señal honesta de «va rápido / va justo».
  const bufFill = el("div", { class: "buffer-fill" });
  const bufBar = el("div", { class: "buffer-bar", "aria-hidden": "true" }, bufFill);
  wrap.append(bufBar);
  const syncBuffer = () => {
    try {
      if (video.buffered.length && video.duration) {
        const end = video.buffered.end(video.buffered.length - 1);
        bufFill.style.width = `${Math.min(100, (end / video.duration) * 100)}%`;
      }
    } catch (_) {}
  };
  video.addEventListener("progress", syncBuffer);
  video.addEventListener("timeupdate", syncBuffer);

  let started = false;
  let switchingAudio = false;    // hay un POST de cambio de audio en vuelo
  let teardownTracks = () => {}; // cierra y limpia el selector de pistas
  let onlineVttTrack = null;     // <track> dinámico de subtítulos online
  let stallTimer;
  const reveal = () => {
    started = true;
    clearTimeout(stallTimer);
    status.remove();
  };
  const fail = (msg) => {
    if (started) return; // ya arrancó: no pintes el aviso encima del video
    clearTimeout(stallTimer);
    status.replaceChildren(el("span", { class: "video-error" }, msg));
  };
  // Reintento del stream: re-lanza la sesión con el payload guardado (si lo
  // hay) y, si no, remonta el player recargando el mismo `src` vía `route()`.
  const retry = async (btn) => {
    const replay = session ? knownReplay(session) : null;
    if (!replay) { route(); return; }
    if (btn) { btn.disabled = true; btn.replaceChildren(el("div", { class: "spinner" }), "Reintentando…"); }
    try {
      const plan = await api("/api/play", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(replayBody(replay)),
        timeoutMs: PLAY_TIMEOUT_MS,
      });
      app.replaceChildren(playerView(plan.playback_url, plan.progress_url, plan.session, title, plan));
    } catch (e) {
      showBanner(e.message);
      if (btn) { btn.disabled = false; btn.textContent = "Reintentar"; }
    }
  };
  video.addEventListener("canplay", reveal);
  video.addEventListener("playing", reveal);
  video.addEventListener("error", () => {
    // Con HLS, los errores fatales los gestiona hls.js (fallback al MP4).
    if (hls) return;
    // Código 4 = el navegador no pudo cargar/decodificar la fuente. La sesión
    // sigue viva: falló el media/torrent (sin pares, datos fuera del caché o
    // remux caído). No es «sesión expirada» y hay que poder reintentar.
    if (video.error && video.error.code === 4) {
      clearTimeout(stallTimer);
      clearInterval(tick);
      teardownTracks();
      // Pista específica: si el plan venía de una fuente HEVC/10-bit/DV, el
      // navegador casi seguro no decodifica ese video (el Pi no transcodifica).
      const vc = plan && typeof plan.video_codec === "string" ? plan.video_codec.toLowerCase() : "";
      const hint = (vc === "hevc" || vc === "h265" || (plan && plan.browser_may_fail))
        ? "Esta fuente es x265/HEVC, AV1 o 4K: tu navegador no la decodifica y el Pi no puede convertirla en tiempo real. Elegí una fuente x264/AVC 1080p."
        : null;
      renderStreamError(wrap, retry,
        () => backToDetail(identity ? identity.kind : null, identity ? identity.id : null), hint);
      return;
    }
    teardownTracks();
    const code = video.error ? video.error.code : "?";
    const msg = `No se pudo reproducir el video (código ${code}). Probá otra fuente.`;
    showBanner(msg);
    fail(msg);
  });
  stallTimer = setTimeout(() => fail(
    "El video está tardando en arrancar; puede que la fuente no tenga seeds. Probá otra."), VIDEO_STALL_MS);

  if (isHls) {
    const hlsUrl = `/hls/${encodeURIComponent(session)}/index.m3u8`;
    const resetStall = () => {
      clearTimeout(stallTimer);
      stallTimer = setTimeout(() => fail(
        "El video está tardando en arrancar; puede que la fuente no tenga seeds. Probá otra."), VIDEO_STALL_MS);
    };
    if (window.Hls && window.Hls.isSupported()) {
      hls = new window.Hls({ maxBufferLength: 30, maxMaxBufferLength: 60 });
      hls.loadSource(hlsUrl);
      hls.attachMedia(video);
      hls.on(window.Hls.Events.MANIFEST_PARSED, resetStall);
      hls.on(window.Hls.Events.FRAG_LOADED, resetStall);
      hls.on(window.Hls.Events.ERROR, (_, data) => {
        if (data && data.fatal) {
          try { hls.destroy(); } catch (_) {}
          hls = null;
          video.src = src; // fallback al MP4 progresivo
        }
      });
    } else if (video.canPlayType("application/vnd.apple.mpegurl")) {
      video.src = hlsUrl;
    } else {
      video.src = src;
    }
  }

  // --- Selector de pistas (audio y subtítulos) ----------------------------
  // Cambio en caliente sin recargar la vista: subtítulos vía `textTracks` y
  // audio vía `POST /api/play/:session/audio` + el nuevo `playback_url`. Si el
  // plan no trae pistas (o no hay sesión), no se monta nada.
  let tracksState = normalizeTracks(plan);
  let activeSub = -1;          // índice de subtítulo activo (-1 = apagado)
  let pendingAudio = null;     // índice de audio destino mientras cambia
  let subtitleLoading = null;  // subtítulo esperando a que cargue su VTT
  let panelOpen = false;
  let ctlEl = null;
  let panelEl = null;
  const audioOpts = [];
  const subOpts = [];
  let audioNote = null;
  let busyNote = null;

  // El panel se monta si hay pistas embebidas (exige sesión) o identidad de
  // meta (subtítulos online por IMDb, que también sirven en Direct/biblioteca).
  if (session || identity) {
    const hasSubs = tracksState.subs.length > 0;
    const subPos = {};
    tracksState.subs.forEach((t, i) => { subPos[t.index] = i; });
    const trackEls = hasSubs ? tracksState.subs.map((t, i) => el("track", {
      kind: "subtitles",
      srclang: t.language ? String(t.language) : "und",
      label: trackLabel(t, i),
      src: `/subtitles/${encodeURIComponent(session)}/${encodeURIComponent(t.index)}`,
    })) : [];
    if (trackEls.length) video.append(...trackEls);

    // Subtítulos: todos `disabled` = apagado; el elegido `showing`. Si su VTT
    // aún no cargó, esperamos `load`/`cuechange` sin bloquear la interfaz.
    const setSubtitle = (idx) => {
      // Elegir un subtítulo embebido apaga el online (son excluyentes).
      if (onlineVttTrack) { onlineVttTrack.remove(); onlineVttTrack = null; }
      activeSub = idx;
      const pos = subPos[idx];
      trackEls.forEach((te, j) => {
        const tt = te.track;
        if (tt) tt.mode = j === pos ? "showing" : "disabled";
      });
      subtitleLoading = null;
      if (pos != null) {
        const te = trackEls[pos];
        const tt = te.track;
        if (tt && !(tt.cues && tt.cues.length)) {
          subtitleLoading = idx;
          const done = () => {
            te.removeEventListener("load", done);
            if (tt) tt.removeEventListener("cuechange", done);
            if (subtitleLoading === idx) { subtitleLoading = null; syncPanel(); }
          };
          te.addEventListener("load", done);
          if (tt) tt.addEventListener("cuechange", done);
        }
      }
      syncPanel();
    };

    // Audio: pide el cambio y reengancha el <video> al nuevo `playback_url`.
    // El servidor ya dejó el seek en `position`; el 0 tras `loadedmetadata` es
    // lo correcto (la URL nueva arranca en la posición pedida).
    const changeAudio = async (index) => {
      if (switchingAudio || index === tracksState.activeAudio) return;
      switchingAudio = true;
      pendingAudio = index;
      syncPanel();
      let timer;
      const done = () => {
        if (!switchingAudio) return;
        switchingAudio = false;
        pendingAudio = null;
        clearTimeout(timer);
        syncPanel();
      };
      try {
        const res = await api(`/api/play/${encodeURIComponent(session)}/audio`, {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ index, position: video.currentTime || 0 }),
        });
        tracksState = normalizeTracks(res);
        const onMeta = () => {
          video.removeEventListener("loadedmetadata", onMeta);
          try { video.currentTime = 0; } catch (_) {}
          const pr = video.play();
          if (pr && pr.catch) pr.catch(() => {});
          // Recargar el <video> puede volver a desactivar los subtítulos.
          if (activeSub >= 0) setSubtitle(activeSub);
          done();
        };
        video.addEventListener("loadedmetadata", onMeta);
        timer = setTimeout(done, 15000);
        video.src = res.playback_url;
        video.load();
      } catch (e) {
        showBanner(e.message);
        done();
      }
    };

    const audioGroup = el("div", { class: "track-options", role: "radiogroup", "aria-label": "Pista de audio" });
    tracksState.audio.forEach((t, i) => {
      const meta = trackMeta(t);
      const check = el("span", { class: "track-opt-mark", "aria-hidden": "true" });
      const busy = el("span", { class: "spinner", hidden: true, "aria-hidden": "true" });
      const btn = el("button", {
        type: "button", class: "track-opt", role: "radio", "aria-checked": "false",
        onclick: () => changeAudio(t.index),
      },
        el("span", { class: "track-opt-main" },
          el("span", { class: "track-opt-label" }, trackLabel(t, i)),
          meta ? el("span", { class: "track-opt-meta" }, meta) : null),
        check, busy);
      audioGroup.append(btn);
      audioOpts.push({ btn, check, busy, index: t.index });
    });
    audioNote = el("p", { class: "track-note", hidden: true }, "El audio no se puede cambiar en esta fuente");
    busyNote = el("p", { class: "track-busy", hidden: true }, el("span", { class: "spinner" }), "Cambiando pista…");

    const subGroup = el("div", { class: "track-options", role: "radiogroup", "aria-label": "Subtítulos" });
    const subDefs = [{ index: -1, label: "Apagado", meta: null }].concat(
      tracksState.subs.map((t, i) => ({ index: t.index, label: trackLabel(t, i), meta: trackMeta(t) })),
    );
    subDefs.forEach((d) => {
      const check = el("span", { class: "track-opt-mark", "aria-hidden": "true" });
      const busy = el("span", { class: "spinner", hidden: true, "aria-hidden": "true" });
      const btn = el("button", {
        type: "button", class: "track-opt", role: "radio", "aria-checked": "false",
        onclick: () => setSubtitle(d.index),
      },
        el("span", { class: "track-opt-main" },
          el("span", { class: "track-opt-label" }, d.label),
          d.meta ? el("span", { class: "track-opt-meta" }, d.meta) : null),
        check, busy);
      subGroup.append(btn);
      subOpts.push({ btn, check, busy, index: d.index });
    });

    function syncPanel() {
      const locked = tracksState.route === "direct";
      audioOpts.forEach((o) => {
        const on = o.index === tracksState.activeAudio;
        const busy = switchingAudio && o.index === pendingAudio;
        o.btn.classList.toggle("active", on);
        o.btn.setAttribute("aria-checked", on ? "true" : "false");
        o.btn.disabled = locked;
        o.btn.tabIndex = on ? 0 : -1;
        o.check.hidden = busy;
        o.busy.hidden = !busy;
      });
      if (!locked && audioOpts.length && !audioOpts.some((o) => o.btn.tabIndex === 0)) {
        audioOpts[0].btn.tabIndex = 0;
      }
      audioNote.hidden = !locked;
      subOpts.forEach((o) => {
        const on = o.index === activeSub;
        const busy = subtitleLoading === o.index;
        o.btn.classList.toggle("active", on);
        o.btn.setAttribute("aria-checked", on ? "true" : "false");
        o.btn.tabIndex = on ? 0 : -1;
        o.check.hidden = busy;
        o.busy.hidden = !busy;
      });
      if (subOpts.length && !subOpts.some((o) => o.btn.tabIndex === 0)) subOpts[0].btn.tabIndex = 0;
      busyNote.hidden = !switchingAudio;
    }

    const audioSection = tracksState.audio.length
      ? el("div", { class: "track-section" },
          el("h3", { class: "track-heading" }, "Audio"), audioGroup, audioNote, busyNote)
      : null;
    const subSection = hasSubs
      ? el("div", { class: "track-section" },
          el("h3", { class: "track-heading" }, "Subtítulos"), subGroup)
      : null;

    const btnEl = el("button", {
      type: "button", class: "track-btn",
      "aria-label": "Pistas de audio y subtítulos",
      "aria-haspopup": "dialog", "aria-expanded": "false", "aria-controls": "track-panel",
      onclick: () => (panelOpen ? closePanel(true) : openPanel()),
    },
      svgIcon([
        "M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z",
        "M7 12.5h3.2", "M13.8 12.5H17",
      ]),
      el("span", { class: "track-btn-text" }, "Pistas"));
    panelEl = el("div", {
      id: "track-panel", class: "video-track-panel", role: "dialog",
      "aria-label": "Pistas de audio y subtítulos", hidden: true,
    }, audioSection, subSection);
    ctlEl = el("div", { class: "video-track-ctl" }, btnEl);
    wrap.append(ctlEl, panelEl);

    function onDocClick(e) {
      if (!panelEl.contains(e.target) && !btnEl.contains(e.target)) closePanel(false);
    }
    function onDocKey(e) {
      if (e.key === "Escape") { e.preventDefault(); closePanel(true); }
    }
    function openPanel() {
      panelOpen = true;
      panelEl.hidden = false;
      btnEl.setAttribute("aria-expanded", "true");
      syncPanel();
      document.addEventListener("click", onDocClick, true);
      document.addEventListener("keydown", onDocKey);
      const first = panelEl.querySelector('[aria-checked="true"]:not([disabled])')
        || panelEl.querySelector("button:not([disabled])");
      if (first) first.focus();
    }
    function closePanel(refocus) {
      if (!panelOpen) return;
      panelOpen = false;
      panelEl.hidden = true;
      btnEl.setAttribute("aria-expanded", "false");
      document.removeEventListener("click", onDocClick, true);
      document.removeEventListener("keydown", onDocKey);
      if (refocus) btnEl.focus();
    }
    // Navegación con flechas dentro de cada grupo (radiogroup) + selección.
    function roving(groupEl, buttons) {
      groupEl.addEventListener("keydown", (e) => {
        if (!["ArrowDown", "ArrowRight", "ArrowUp", "ArrowLeft", "Home", "End"].includes(e.key)) return;
        const btns = buttons().filter((b) => !b.disabled);
        if (!btns.length) return;
        const cur = btns.indexOf(document.activeElement);
        if (cur < 0) { btns[0].focus(); return; }
        let next = cur;
        if (e.key === "ArrowDown" || e.key === "ArrowRight") next = (cur + 1) % btns.length;
        else if (e.key === "ArrowUp" || e.key === "ArrowLeft") next = (cur - 1 + btns.length) % btns.length;
        else if (e.key === "Home") next = 0;
        else if (e.key === "End") next = btns.length - 1;
        e.preventDefault();
        btns[next].focus();
        btns[next].click();
      });
    }
    if (tracksState.audio.length) roving(audioGroup, () => audioOpts.map((o) => o.btn));
    if (hasSubs) roving(subGroup, () => subOpts.map((o) => o.btn));

    // --- Subtítulos online (OpenSubtitles) --------------------------------
    // Se buscan solos por IMDb (kind/id de la ficha): al abrir el panel ya
    // están. Español primero; si hay y no hay embebido activo, se activa solo.
    const onlineLangLabel = { es: "Español", en: "Inglés", la: "Latino" };
    const onlineGroup = el("div", { class: "track-options", role: "radiogroup", "aria-label": "Subtítulos online" });
    const onlineSection = el("div", { class: "track-section", hidden: true },
      el("h3", { class: "track-heading" }, "Subtítulos online"),
      onlineGroup,
      el("p", { class: "track-note" }, "Descargados automáticamente desde OpenSubtitles."));
    panelEl.append(onlineSection);
    const onlineOpts = [];
    const attachVtt = (lang) => {
      if (onlineVttTrack) { onlineVttTrack.remove(); onlineVttTrack = null; }
      const te = el("track", {
        kind: "subtitles",
        srclang: lang,
        label: onlineLangLabel[lang] || lang.toUpperCase(),
        src: `/api/subtitles/external/${encodeURIComponent(identity.kind)}/${encodeURIComponent(identity.id)}/file/${encodeURIComponent(lang)}`,
      });
      video.append(te);
      onlineVttTrack = te;
      te.addEventListener("load", () => { if (te.track) te.track.mode = "showing"; }, { once: true });
    };
    const setOnlineSub = (lang, btn) => {
      activeSub = -1;
      trackEls.forEach((te) => { if (te.track) te.track.mode = "disabled"; });
      onlineOpts.forEach((o) => o.btn.classList.toggle("active", o.btn === btn));
      attachVtt(lang);
      syncPanel();
    };
    let onlineLoaded = false;
    const ensureOnlineSubs = async () => {
      if (!identity || onlineLoaded) return;
      onlineLoaded = true;
      const base = `/api/subtitles/external/${encodeURIComponent(identity.kind)}/${encodeURIComponent(identity.id)}`;
      let langs = [];
      try {
        const list = await api(base, { navigateAbort: false });
        langs = (list && list.languages) || [];
      } catch (_) {}
      if (!langs.length) {
        // Auto: el server busca el mejor por idioma (es → en → la) y lo baja.
        try {
          const got = await api(base, {
            method: "PUT",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ lang: "auto" }),
            navigateAbort: false,
            timeoutMs: 45000,
          });
          if (got && got.language) langs = [got.language];
        } catch (_) {}
      }
      if (!langs.length) return;
      for (const lang of langs) {
        const btn = el("button", {
          type: "button", class: "track-opt", role: "radio", "aria-checked": "false",
          onclick: () => setOnlineSub(lang, btn),
        },
          el("span", { class: "track-opt-main" },
            el("span", { class: "track-opt-label" }, onlineLangLabel[lang] || lang.toUpperCase()),
            el("span", { class: "track-opt-meta" }, "Online")),
          el("span", { class: "track-opt-mark", "aria-hidden": "true" }));
        onlineGroup.append(btn);
        onlineOpts.push({ btn, lang });
      }
      onlineSection.hidden = false;
      // Español por defecto si no hay ningún subtítulo activo ya.
      if (activeSub < 0 && !onlineOpts.some((o) => o.btn.classList.contains("active"))) {
        const pref = langs.includes("es") ? "es" : langs[0];
        const target = onlineOpts.find((o) => o.lang === pref);
        if (target) setOnlineSub(pref, target.btn);
      }
    };
    ensureOnlineSubs().catch(() => {});

    teardownTracks = () => {
      closePanel(false);
      if (ctlEl) ctlEl.hidden = true;
      if (onlineVttTrack) { onlineVttTrack.remove(); onlineVttTrack = null; }
    };
    syncPanel();
  }

  if (progressUrl) {
    fetch(progressUrl)
      .then((r) => (r.ok ? r.json() : null))
      .then((p) => {
        // No saltes al usuario hacia atrás si el video ya arrancó o ya se movió
        // (el GET puede resolver tarde en redes lentas).
        if (started || video.currentTime > 0) return;
        if (p && p.position > 1 && (!p.duration || p.position < p.duration - 15)) {
          video.currentTime = p.position;
        }
      })
      .catch(() => {});
  }

  const saveProgress = () => {
    if (switchingAudio || !progressUrl || !video.duration) return;
    fetch(progressUrl, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ position: video.currentTime, duration: video.duration }),
    }).catch(() => {});
  };
  const tick = setInterval(saveProgress, 10000);
  video.addEventListener("pause", saveProgress);
  video.addEventListener("ended", saveProgress);
  // Un único punto de limpieza: libera el interval y los listeners. `dispose`
  // se llama al desmontar (remontaje vía playerView) y al cambiar de hash.
  const dispose = () => {
    clearInterval(tick);
    if (hls) { try { hls.destroy(); } catch (_) {} hls = null; }
    teardownTracks();
    window.removeEventListener("hashchange", onHash);
    if (activePlayerDispose === dispose) activePlayerDispose = null;
  };
  const onHash = () => { saveProgress(); dispose(); };
  window.addEventListener("hashchange", onHash, { once: true });
  activePlayerDispose = dispose;

  const actions = [el("button", { class: "btn ghost", onclick: () => history.back() }, t("back"))];
  if (session) {
    // Compartir: copia el snippet de embed (iframe vía /embed.js).
    actions.push(el("button", { class: "btn", onclick: () => {
      const code = `<div data-pistreaming-session="${session}"></div>\n<script src="${location.origin}/embed.js" async></script>`;
      const done = () => showBanner("Código de embed copiado");
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(code).then(done).catch(() => prompt("Copia el código de embed:", code));
      } else {
        prompt("Copia el código de embed:", code);
      }
    } }, t("share")));
    // Descargar el torrent completo como ZIP al vuelo.
    actions.push(el("a", { class: "btn", href: `/api/zip/${encodeURIComponent(session)}`, download: "" }, t("downloadZip")));
    actions.push(el("button", { class: "btn", onclick: async (e) => {
      e.target.disabled = true;
      try {
        await api("/api/library", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ session }),
        });
        showBanner("Guardado en la biblioteca");
      } catch (err) { showBanner(err.message); }
      finally { e.target.disabled = false; }
    } }, t("save")));
  }
  // Aviso de ruta: el usuario ve POR QUÉ arranca/suena distinto (transcode de
  // audio o video que el navegador puede no decodificar).
  const notes = [];
  if (plan && plan.route === "recodeaudio") {
    notes.push("Audio transcodificado a AAC (la pista original no se reproduce en el navegador).");
  }
  if (plan && plan.browser_may_fail) {
    notes.push("Video HEVC/AV1: si no se ve, probá una fuente x264 o abrí la URL cruda en mpv/VLC.");
  }
  const noteEl = notes.length ? el("p", { class: "muted player-note" }, notes.join(" ")) : null;

  return el("div", { class: "player-wrap" },
    title ? el("h1", { class: "player-title" }, title) : null,
    wrap,
    noteEl,
    el("div", { class: "player-actions" }, ...actions),
  );
}

// --- Biblioteca -----------------------------------------------------------
async function viewLibrary(seq) {
  setTab("library");
  const data = await api("/api/library");
  if (stale(seq)) return;
  const items = data.items || [];
  const body = items.length
    ? el("div", { class: "grid" }, ...items.map(libCard))
    : empty("Todavía no guardaste nada");
  app.replaceChildren(el("h1", {}, "Biblioteca"), body);
}

function libCard(it) {
  const mb = Math.round((it.size_bytes || 0) / (1024 * 1024));
  return el("div", { class: "card" },
    el("div", { class: "card-title" }, it.title || it.id),
    el("div", { class: "muted" }, `${it.kind} · ${mb} MB`),
    el("div", { class: "card-actions" },
      el("a", { class: "btn primary", href: `#/library/${encodeURIComponent(it.id)}` }, t("play")),
      el("button", { class: "btn ghost", onclick: async () => {
        try {
          await api(`/api/library/${encodeURIComponent(it.id)}`, { method: "DELETE" });
          route();
        } catch (e) { showBanner(e.message); }
      } }, "Quitar"),
    ),
  );
}

// --- Addons ---------------------------------------------------------------
async function viewAddons(seq) {
  setTab("addons");
  const rows = await api("/api/addons");
  if (stale(seq)) return;
  const form = el("form", { class: "searchbar", onsubmit: async (e) => {
    e.preventDefault();
    const url = e.target.url.value.trim();
    if (!url) return;
    try {
      await api("/api/addons", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ url }),
      });
      route();
    } catch (err) { showBanner(err.message); }
  }},
    el("input", { name: "url", placeholder: "https://addon.example/manifest.json" }),
    el("button", { class: "btn primary", type: "submit" }, "Agregar"),
  );
  const list = el("div", { class: "list" });
  if (!rows.length) list.append(empty("Sin addons"));
  rows.forEach((a) => {
    list.append(el("div", { class: "row-item" },
      el("div", {}, el("strong", {}, a.name || a.url), el("div", { class: "muted" }, a.url)),
      el("label", { class: "switch" },
        el("input", { type: "checkbox", checked: !!a.enabled, onchange: async (e) => {
          try {
            await api(`/api/addons/${encodeURIComponent(a.url)}`, {
              method: "PATCH",
              headers: { "content-type": "application/json" },
              body: JSON.stringify({ enabled: e.target.checked }),
            });
          } catch (err) {
            showBanner(err.message);
            e.target.checked = !e.target.checked;
          }
        } }),
        "activo"),
      el("button", { class: "btn ghost", onclick: async () => {
        try {
          await api(`/api/addons/${encodeURIComponent(a.url)}`, { method: "DELETE" });
          route();
        } catch (err) { showBanner(err.message); }
      } }, "Borrar"),
    ));
  });
  app.replaceChildren(el("h1", {}, "Addons"), form, list);
}

// --- Ajustes --------------------------------------------------------------
const HOT_FIELDS = ["cache_max_gb", "cache_ttl_hours"];
const STATIC_FIELDS = ["egress_bind", "http_port", "data_dir"];
const NUMERIC = new Set(["cache_max_gb", "cache_ttl_hours", "http_port"]);

async function viewSettings(seq) {
  setTab("settings");
  const s = await api("/api/settings");
  if (stale(seq)) return;
  const inputs = {};
  const form = el("form", { class: "form", onsubmit: async (e) => {
    e.preventDefault();
    const body = {};
    for (const f of [...HOT_FIELDS, ...STATIC_FIELDS]) {
      const raw = inputs[f].value.trim();
      if (raw === "") continue;
      body[f] = NUMERIC.has(f) ? Number(raw) : raw;
    }
    try {
      const r = await api("/api/settings", {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      const applied = (r.applied || []).length ? `Aplicado: ${r.applied.join(", ")}. ` : "";
      const restart = (r.requires_restart || []).length
        ? `${t("requiresRestart")}: ${r.requires_restart.map((k) => t("label_" + k) || k).join(", ")}.`
        : "";
      showBanner(applied + restart || "Guardado");
      route();
    } catch (err) { showBanner(err.message); }
  }});
  for (const f of [...HOT_FIELDS, ...STATIC_FIELDS]) {
    const input = el("input", { value: s[f] ?? "", type: NUMERIC.has(f) ? "number" : "text" });
    inputs[f] = input;
    const label = el("label", { class: "field" }, el("span", {}, t("label_" + f) || f), input);
    if (STATIC_FIELDS.includes(f)) label.append(el("small", { class: "muted" }, t("requiresRestart")));
    form.append(label);
  }
  form.append(el("button", { class: "btn primary", type: "submit" }, t("saveSettings")));
  const langSel = el("select", { class: "sel", "aria-label": t("language"), onchange: (e) => {
    LANG = e.target.value === "en" ? "en" : "es";
    try { localStorage.setItem("lang", LANG); } catch (_) {}
    applyLang();
    route();
  } },
    el("option", { value: "es", selected: LANG === "es" }, "Español"),
    el("option", { value: "en", selected: LANG === "en" }, "English"),
  );
  const langField = el("label", { class: "field" }, el("span", {}, t("language")), langSel);
  const children = [el("h1", {}, t("settings")), langField, form];
  if (s.auth_enabled) {
    children.push(el("form", { method: "post", action: "/api/logout", class: "form" },
      el("button", { class: "btn", type: "submit" }, t("logout"))));
  }
  app.replaceChildren(...children);
}

window.addEventListener("hashchange", route);
applyLang();
route();

// PiStreaming SPA — router por hash + vistas. Sin dependencias ni build.
const app = document.getElementById("app");
const nav = document.getElementById("nav");
const banner = document.getElementById("banner");

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

function showBanner(msg) {
  banner.textContent = msg;
  banner.hidden = false;
  clearTimeout(showBanner._t);
  showBanner._t = setTimeout(() => { banner.hidden = true; }, 6000);
}

async function api(path, opts = {}) {
  const res = await fetch(path, opts);
  if (!res.ok) {
    let msg = `Error ${res.status}`;
    try {
      const j = await res.json();
      if (j.error) msg = j.error;
    } catch (_) {}
    throw new Error(msg);
  }
  if (res.status === 204) return null;
  const ct = res.headers.get("content-type") || "";
  return ct.includes("application/json") ? res.json() : res.text();
}

function setTab(tab) {
  if (!nav) return;
  for (const a of nav.querySelectorAll("a")) a.classList.toggle("active", a.dataset.tab === tab);
}

const loading = () => el("div", { class: "state" }, el("div", { class: "spinner" }), "Cargando…");
const empty = (msg) => el("div", { class: "state" }, msg);
function errorBox(msg, retry) {
  return el("div", { class: "state error" }, el("p", {}, msg),
    retry ? el("button", { class: "btn", onclick: retry }, "Reintentar") : null);
}

async function route() {
  const raw = location.hash.replace(/^#\/?/, "");
  const parts = raw.split("/");
  const head = parts[0] || "search";
  app.replaceChildren(loading());
  try {
    if (head === "search") return await viewSearch(parts[1]);
    if (head === "detail") return await viewDetail(parts[1], parts.slice(2).join("/"));
    if (head === "player") return await viewPlayer(parts[1]);
    if (head === "library") {
      if (parts[1]) return await viewLibraryPlayer(decodeURIComponent(parts.slice(1).join("/")));
      return await viewLibrary();
    }
    if (head === "addons") return await viewAddons();
    if (head === "settings") return await viewSettings();
    app.replaceChildren(empty("Ruta desconocida"));
  } catch (e) {
    showBanner(e.message);
    app.replaceChildren(errorBox(e.message, route));
  }
}

// --- Buscar ---------------------------------------------------------------
const KINDS = [
  { id: "movie", label: "Películas" },
  { id: "series", label: "Series" },
];
const KIND_LABEL = { movie: "películas", series: "series" };

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
    input.placeholder = `Buscar ${KIND_LABEL[kind]}…`;
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
      }, k.label);
      buttons[k.id] = b;
      return b;
    }),
  );
  const form = el("form", { class: "searchbar", onsubmit: async (e) => {
    e.preventDefault();
    const q = input.value.trim();
    if (q) await runSearch(q, kind);
  }},
    segmented,
    input,
    el("button", { class: "btn primary", type: "submit" }, "Buscar"),
  );
  app.replaceChildren(el("h1", {}, "Buscar"), form,
    el("div", { id: "results", class: "grid" }));
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
  results.replaceChildren(loading());
  const data = await api(`/api/search?query=${encodeURIComponent(q)}&kind=${kind}`);
  if (seq !== searchSeq) return; // descartá respuestas obsoletas
  const metas = data.metas || [];
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
  const poster = safePosterUrl(m.poster || m.background || "");
  const cover = el("div", { class: "poster" },
    poster ? null : el("span", { class: "poster-fallback" }, (m.name || "?").slice(0, 1)));
  if (poster) cover.style.backgroundImage = `url("${poster}")`;
  return el("a", { class: "card", href: `#/detail/${m.type}/${encodeURIComponent(m.id)}` },
    cover,
    el("div", { class: "card-title" }, m.name || m.id),
  );
}

// --- Detalle --------------------------------------------------------------
async function viewDetail(kind, id) {
  setTab("search");
  if (!kind || !id) { app.replaceChildren(empty("Ficha inválida")); return; }
  id = decodeURIComponent(id);
  const [meta, streamsData] = await Promise.all([
    api(`/api/meta/${kind}/${encodeURIComponent(id)}`).catch(() => null),
    api(`/api/streams/${kind}/${encodeURIComponent(id)}`).catch(() => ({ streams: [] })),
  ]);
  const m = meta || { name: id };
  const streams = streamsData.streams || [];
  const poster = safePosterUrl(m.poster || "");
  const cover = el("div", { class: "poster large" },
    poster ? null : el("span", { class: "poster-fallback" }, (m.name || "?").slice(0, 1)));
  if (poster) cover.style.backgroundImage = `url("${poster}")`;
  const head = el("div", { class: "detail-head" },
    cover,
    el("div", { class: "detail-meta" },
      el("h1", {}, m.name || id),
      m.releaseInfo ? el("p", { class: "muted" }, m.releaseInfo) : null,
      m.description ? el("p", { class: "desc" }, m.description) : null,
    ),
  );
  const list = el("div", { class: "list" });
  if (!streams.length) list.append(empty("Sin fuentes disponibles"));
  streams.forEach((s) => {
    list.append(el("div", { class: "row-item" },
      el("div", {}, el("strong", {}, s.name || "Fuente"),
        el("div", { class: "muted" }, s.title || s.infoHash || s.url || "")),
      el("button", { class: "btn primary", onclick: () => startPlay(s, kind, id, m.name) }, "Reproducir"),
    ));
  });
  app.replaceChildren(head, el("h2", { class: "section" }, "Fuentes"), list);
}

async function startPlay(stream, kind, id, title) {
  const magnet = stream.infoHash ? `magnet:?xt=urn:btih:${stream.infoHash}` : stream.url;
  if (!magnet) { showBanner("La fuente no trae magnet ni url"); return; }
  try {
    const plan = await api("/api/play", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ magnet, title, kind, id }),
    });
    location.hash = `#/player/${encodeURIComponent(plan.session)}`;
  } catch (e) { showBanner(e.message); }
}

// --- Reproductor (torrent) ------------------------------------------------
async function viewPlayer(session) {
  setTab(null);
  session = decodeURIComponent(session || "");
  const plan = await api(`/api/play/${encodeURIComponent(session)}`);
  app.replaceChildren(playerView(plan.playback_url, plan.progress_url, session, null));
}

// --- Reproductor (biblioteca) ---------------------------------------------
async function viewLibraryPlayer(id) {
  setTab("library");
  const data = await api("/api/library");
  const item = (data.items || []).find((i) => i.id === id);
  const [kind, ...rest] = id.split(":");
  const progressUrl = `/api/progress/${kind}/${rest.join(":")}`;
  app.replaceChildren(
    playerView(`/library/${encodeURIComponent(id)}/stream`, progressUrl, null, item ? item.title : id),
  );
}

function playerView(src, progressUrl, session, title) {
  const video = el("video", { class: "video", controls: true, autoplay: true, playsinline: true, src });
  video.addEventListener("error", () => showBanner("El navegador no puede reproducir este archivo."));

  if (progressUrl) {
    fetch(progressUrl)
      .then((r) => (r.ok ? r.json() : null))
      .then((p) => {
        if (p && p.position > 1 && (!p.duration || p.position < p.duration - 15)) {
          video.currentTime = p.position;
        }
      })
      .catch(() => {});
  }

  const saveProgress = () => {
    if (!progressUrl || !video.duration) return;
    fetch(progressUrl, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ position: video.currentTime, duration: video.duration }),
    }).catch(() => {});
  };
  const tick = setInterval(saveProgress, 10000);
  video.addEventListener("pause", saveProgress);
  video.addEventListener("ended", saveProgress);
  window.addEventListener("hashchange", () => {
    saveProgress();
    clearInterval(tick);
  }, { once: true });

  const actions = [el("button", { class: "btn ghost", onclick: () => history.back() }, "Volver")];
  if (session) {
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
    } }, "Guardar"));
  }
  return el("div", { class: "player-wrap" },
    title ? el("h1", { class: "player-title" }, title) : null,
    video,
    el("div", { class: "player-actions" }, ...actions),
  );
}

// --- Biblioteca -----------------------------------------------------------
async function viewLibrary() {
  setTab("library");
  const data = await api("/api/library");
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
      el("a", { class: "btn primary", href: `#/library/${encodeURIComponent(it.id)}` }, "Reproducir"),
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
async function viewAddons() {
  setTab("addons");
  const rows = await api("/api/addons");
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
const LABELS = {
  cache_max_gb: "Caché máxima (GB)",
  cache_ttl_hours: "TTL de caché (horas)",
  egress_bind: "Interfaz de salida",
  http_port: "Puerto HTTP",
  data_dir: "Directorio de datos",
};
const NUMERIC = new Set(["cache_max_gb", "cache_ttl_hours", "http_port"]);

async function viewSettings() {
  setTab("settings");
  const s = await api("/api/settings");
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
        ? `Requiere reinicio: ${r.requires_restart.map((k) => LABELS[k] || k).join(", ")}.`
        : "";
      showBanner(applied + restart || "Guardado");
      route();
    } catch (err) { showBanner(err.message); }
  }});
  for (const f of [...HOT_FIELDS, ...STATIC_FIELDS]) {
    const input = el("input", { value: s[f] ?? "", type: NUMERIC.has(f) ? "number" : "text" });
    inputs[f] = input;
    const label = el("label", { class: "field" }, el("span", {}, LABELS[f] || f), input);
    if (STATIC_FIELDS.includes(f)) label.append(el("small", { class: "muted" }, "requiere reinicio"));
    form.append(label);
  }
  form.append(el("button", { class: "btn primary", type: "submit" }, "Guardar"));
  app.replaceChildren(el("h1", {}, "Ajustes"), form);
}

window.addEventListener("hashchange", route);
route();

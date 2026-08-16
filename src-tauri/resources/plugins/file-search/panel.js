/**
 * File Search — island panel (Alt+Space).
 * Empty: folders | history. Enter with query: type rail + results.
 */
(function blockNativeDialogs() {
  const ban = (name, fallback) => {
    try {
      window[name] = function banned(...args) {
        console.error(`[file-search/panel] forbidden window.${name}()`, args);
        return fallback;
      };
    } catch {
      /* ignore */
    }
  };
  ban("alert", undefined);
  ban("confirm", false);
  ban("prompt", null);
})();

const hub = () => {
  if (!window.hub) throw new Error("window.hub missing");
  return window.hub;
};

const STORE_KEY = "store";
const HISTORY_MAX = 12;
const COLORS = ["#ffd60a", "#1c1c1e", "#30d158", "#0a84ff", "#ff9f0a", "#bf5af2"];

/** Left rail filters — Everything syntax appended to the user keyword. */
const TYPE_FILTERS = [
  { id: "all", label: "全部", clause: "" },
  { id: "folder", label: "文件夹", clause: "folder:" },
  { id: "excel", label: "EXCEL", clause: "ext:xls;xlsx;xlsm;xlsb;csv" },
  { id: "word", label: "WORD", clause: "ext:doc;docx;docm;rtf" },
  { id: "ppt", label: "PPT", clause: "ext:ppt;pptx;pptm" },
  { id: "pdf", label: "PDF", clause: "ext:pdf" },
  {
    id: "image",
    label: "图片",
    clause: "ext:jpg;jpeg;png;gif;webp;bmp;ico;svg;heic;tif;tiff",
  },
  {
    id: "video",
    label: "视频",
    clause: "ext:mp4;mkv;avi;mov;wmv;flv;webm;m4v;ts",
  },
  {
    id: "audio",
    label: "音频",
    clause: "ext:mp3;wav;flac;aac;m4a;ogg;wma;ape",
  },
  {
    id: "archive",
    label: "压缩文件",
    clause: "ext:zip;rar;7z;tar;gz;bz2;xz;iso;cab",
  },
];

const state = {
  store: { version: 1, categories: [], history: [] },
  view: "home", // home | results
  query: "",
  results: [],
  total: 0,
  loading: false,
  error: null,
  activeCategoryId: null,
  typeFilterId: "all",
  status: null,
};

function uid() {
  return crypto.randomUUID
    ? crypto.randomUUID()
    : `c-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function colorFor(name) {
  let h = 0;
  const s = String(name || "");
  for (let i = 0; i < s.length; i += 1) h = (h * 31 + s.charCodeAt(i)) >>> 0;
  return COLORS[h % COLORS.length];
}

function escapeHtml(s) {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function formatSize(n) {
  if (n == null || !Number.isFinite(n) || n < 0) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

function normalize(raw) {
  const cats = Array.isArray(raw?.categories)
    ? raw.categories.map((c, i) => ({
        id: c.id || uid(),
        name: String(c.name || "未命名").trim() || "未命名",
        path: String(c.path || "").trim(),
        color: c.color || colorFor(c.name),
        order: typeof c.order === "number" ? c.order : i,
      }))
    : [];
  cats.sort((a, b) => a.order - b.order);
  const history = Array.isArray(raw?.history)
    ? raw.history
        .map((h) => String(h || "").trim())
        .filter(Boolean)
        .slice(0, HISTORY_MAX)
    : [];
  return { version: 1, categories: cats, history };
}

function activeTypeFilter() {
  return (
    TYPE_FILTERS.find((f) => f.id === state.typeFilterId) || TYPE_FILTERS[0]
  );
}

/** Combine keyword + type clause for Everything IPC. */
function buildEverythingQuery(userQuery) {
  const q = String(userQuery ?? "").trim();
  const clause = (activeTypeFilter().clause || "").trim();
  if (!clause) return q;
  if (!q) return clause;
  return `${clause} ${q}`;
}

async function loadStore() {
  const raw = await hub().storage.get(STORE_KEY);
  state.store = normalize(raw);
}

async function saveStore() {
  await hub().storage.set(STORE_KEY, state.store);
}

async function pushHistory(q) {
  const text = String(q || "").trim();
  if (!text) return;
  const next = [text, ...state.store.history.filter((h) => h !== text)].slice(
    0,
    HISTORY_MAX,
  );
  state.store.history = next;
  await saveStore();
}

function activeCategory() {
  if (!state.activeCategoryId) return null;
  return state.store.categories.find((c) => c.id === state.activeCategoryId) || null;
}

async function refreshStatus() {
  try {
    state.status = await hub().everything.status();
  } catch (e) {
    state.status = { running: false, error: String(e?.message || e) };
  }
}

async function runSearch(query, opts = {}) {
  const q = String(query ?? "").trim();
  const keepFilter = opts.keepFilter === true;
  state.query = q;
  if (!keepFilter) state.typeFilterId = "all";
  if (!q) {
    state.view = "home";
    state.results = [];
    state.total = 0;
    state.error = null;
    state.loading = false;
    state.typeFilterId = "all";
    render();
    return;
  }
  state.view = "results";
  state.loading = true;
  state.error = null;
  render();
  try {
    const cat = activeCategory();
    const searchOpts = { max: 60 };
    if (cat?.path) searchOpts.pathPrefix = cat.path;
    const res = await hub().everything.search(buildEverythingQuery(q), searchOpts);
    state.results = Array.isArray(res?.results) ? res.results : [];
    state.total = Number(res?.total) || state.results.length;
    await pushHistory(q);
  } catch (e) {
    state.results = [];
    state.total = 0;
    state.error = String(e?.message || e);
  } finally {
    state.loading = false;
    render();
  }
}

async function setTypeFilter(id) {
  if (!TYPE_FILTERS.some((f) => f.id === id)) return;
  if (state.typeFilterId === id && state.view === "results") return;
  state.typeFilterId = id;
  if (state.query.trim()) {
    await runSearch(state.query, { keepFilter: true });
  } else {
    render();
  }
}

async function openHit(hit, reveal) {
  const path = hit?.fullPath || hit?.full_path;
  if (!path) return;
  try {
    if (reveal) await hub().everything.reveal(path);
    else await hub().everything.open(path);
  } catch (e) {
    state.error = String(e?.message || e);
    render();
  }
}

async function openFolder(cat) {
  if (!cat?.path) return;
  try {
    await hub().everything.open(cat.path);
  } catch (e) {
    state.error = String(e?.message || e);
    render();
  }
}

function renderHome() {
  const folders = state.store.categories
    .slice(0, 8)
    .map(
      (c) => `
      <button type="button" class="folder-tile" data-id="${escapeHtml(c.id)}" title="${escapeHtml(c.path)}">
        <span class="folder-icon" style="background:${escapeHtml(c.color)}">${escapeHtml((c.name || "?").slice(0, 1))}</span>
        <span class="folder-name">${escapeHtml(c.name)}</span>
      </button>`,
    )
    .join("");

  const history = state.store.history
    .slice(0, 8)
    .map(
      (h, i) => `
      <button type="button" class="hist-row" data-i="${i}">
        <span class="hist-dot" aria-hidden></span>
        <span class="hist-text">${escapeHtml(h)}</span>
      </button>`,
    )
    .join("");

  const status = !state.status?.running
    ? `<div class="banner warn">${escapeHtml(state.status?.error || "Everything 未运行")}</div>`
    : "";

  return `
    ${status}
    <div class="cards">
      <section class="card">
        <div class="card-title">常用文件夹</div>
        <div class="folder-row">
          ${
            folders ||
            `<div class="empty-mini">在快捷区弹窗「分类」里添加文件夹</div>`
          }
        </div>
      </section>
      <section class="card">
        <div class="card-title">历史搜索</div>
        <div class="hist-list">
          ${history || `<div class="empty-mini">回车搜索后会出现在这里</div>`}
        </div>
      </section>
    </div>
  `;
}

function renderTypeRail() {
  return `
    <nav class="type-rail" aria-label="文件类型">
      ${TYPE_FILTERS.map(
        (f) => `
        <button
          type="button"
          class="type-item${f.id === state.typeFilterId ? " is-active" : ""}"
          data-type="${escapeHtml(f.id)}"
        >${escapeHtml(f.label)}</button>`,
      ).join("")}
    </nav>
  `;
}

function renderResults() {
  const hits = state.results
    .map((r, i) => {
      const kind = r.isFolder ? "夹" : "文";
      const size = r.isFolder ? "" : formatSize(r.size);
      return `
        <button type="button" class="hit" data-i="${i}" title="${escapeHtml(r.fullPath)}">
          <span class="hit-kind">${kind}</span>
          <span class="hit-body">
            <span class="hit-name">${escapeHtml(r.name)}</span>
            <span class="hit-path">${escapeHtml(r.path)}</span>
          </span>
          <span class="hit-size">${escapeHtml(size)}</span>
        </button>`;
    })
    .join("");

  const filterLabel = activeTypeFilter().label;
  const metaLabel = state.loading
    ? "搜索中…"
    : state.error
      ? escapeHtml(state.error)
      : `「${escapeHtml(state.query)}」· ${filterLabel} · ${state.total} 条`;

  return `
    <div class="results-layout">
      ${renderTypeRail()}
      <div class="results-main">
        <div class="results-meta">
          <span class="results-meta-text">${metaLabel}</span>
          <button type="button" class="link" id="backHome">返回</button>
        </div>
        <div class="results-list">
          ${hits || (state.loading ? "" : `<div class="empty-mini">无结果</div>`)}
        </div>
      </div>
    </div>
  `;
}

function render() {
  const root = document.getElementById("root");
  if (!root) return;
  root.innerHTML =
    state.view === "results" ? renderResults() : renderHome();
  bind();
}

function bind() {
  document.querySelectorAll(".folder-tile").forEach((el) => {
    el.addEventListener("click", () => {
      const id = el.getAttribute("data-id");
      const cat = state.store.categories.find((c) => c.id === id);
      if (cat) void openFolder(cat);
    });
  });
  document.querySelectorAll(".hist-row").forEach((el) => {
    el.addEventListener("click", () => {
      const i = Number(el.getAttribute("data-i"));
      const q = state.store.history[i];
      if (q) void runSearch(q);
    });
  });
  document.querySelectorAll(".type-item").forEach((el) => {
    el.addEventListener("click", () => {
      const id = el.getAttribute("data-type");
      if (id) void setTypeFilter(id);
    });
  });
  document.querySelectorAll(".hit").forEach((el) => {
    el.addEventListener("click", (e) => {
      const i = Number(el.getAttribute("data-i"));
      const hit = state.results[i];
      if (hit) void openHit(hit, e.altKey);
    });
    el.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const i = Number(el.getAttribute("data-i"));
      const hit = state.results[i];
      if (hit) void openHit(hit, true);
    });
  });
  document.getElementById("backHome")?.addEventListener("click", () => {
    state.view = "home";
    state.query = "";
    state.results = [];
    state.typeFilterId = "all";
    render();
  });
}

function onHostSearch(ev) {
  const d = ev?.detail;
  if (!d || d.action !== "submit") return;
  void runSearch(d.query || "");
}

async function boot() {
  await loadStore();
  await refreshStatus();
  render();
  window.addEventListener("wh-island-search", onHostSearch);
  hub().panel?.onLeave?.(() => {
    state.view = "home";
    state.query = "";
    state.results = [];
    state.error = null;
    state.typeFilterId = "all";
  });
}

boot().catch((e) => {
  console.error("[file-search/panel]", e);
  const root = document.getElementById("root");
  if (root) {
    root.innerHTML = `<div class="empty-mini">启动失败：${escapeHtml(e?.message || e)}</div>`;
  }
});

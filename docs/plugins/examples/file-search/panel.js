/**
 * File Search — island panel (Alt+Space).
 * Home: category folders → card grid (React + @hello-pangea/dnd via board.js).
 * Enter with query: type rail + results.
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
const GRID_COLS = 2;
const MAX_SPAN = 2;
const MAX_ROWS = 2;
const COLORS = ["#ffd60a", "#1c1c1e", "#30d158", "#0a84ff", "#ff9f0a", "#bf5af2"];

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
  store: emptyStore(),
  view: "home",
  query: "",
  results: [],
  total: 0,
  loading: false,
  error: null,
  activeFolderId: null,
  typeFilterId: "all",
  status: null,
  editing: false,
  form: null,
  /** @type {null | number} open “更多” menu on results row index */
  hitMenuIndex: null,
  /** brief status under results meta */
  flash: null,
};

/** @type {null | { update: Function; unmount: Function }} */
let boardApi = null;

function emptyStore() {
  return { version: 2, folders: [], cards: [], history: [] };
}

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

function clampSpan(n) {
  const v = Math.round(Number(n));
  if (!Number.isFinite(v)) return 1;
  return Math.max(1, Math.min(MAX_SPAN, v));
}

function pinCols(w) {
  // 1× → 4 cols, 2× → 8 cols (icon size stays fixed; more slots when wider)
  return 4 * clampSpan(w);
}

function pinRows(h) {
  // 1× → 2 rows, 2× → 4 rows
  return 2 * clampSpan(h);
}

/** 1×1→8, 2×1→16, 1×2→16, 2×2→32 — icons keep fixed size */
function pinCapacity(w, h) {
  return pinCols(w) * pinRows(h);
}

function rectsOverlap(a, b) {
  return !(
    a.x + a.w <= b.x ||
    b.x + b.w <= a.x ||
    a.y + a.h <= b.y ||
    b.y + b.h <= a.y
  );
}

function cardRect(c) {
  return { x: c.x, y: c.y, w: c.w, h: c.h };
}

function normalizePin(p) {
  const name = String(p?.name || "未命名").trim() || "未命名";
  return {
    id: p?.id || uid(),
    name,
    path: String(p?.path || "").trim(),
    color: p?.color || colorFor(name),
  };
}

function normalizeCard(c, folderId, i) {
  const kind = c?.kind === "history" ? "history" : "pins";
  const w = clampSpan(c?.w ?? (kind === "history" ? 1 : 2));
  const h = clampSpan(c?.h ?? 1);
  const x = Math.max(0, Math.min(GRID_COLS - w, Math.round(Number(c?.x) || 0)));
  const y = Math.max(0, Math.round(Number(c?.y) || 0));
  const pins =
    kind === "history"
      ? []
      : (Array.isArray(c?.pins) ? c.pins : []).map(normalizePin);
  return {
    id: c?.id || uid(),
    folderId: c?.folderId || folderId,
    kind,
    title:
      String(c?.title || (kind === "history" ? "历史搜索" : "卡片组")).trim() ||
      (kind === "history" ? "历史搜索" : "卡片组"),
    x,
    y,
    w,
    h,
    pins,
    order: typeof c?.order === "number" ? c.order : i,
  };
}

function defaultBoard(folderId, pinsFromCategories) {
  const pins = (pinsFromCategories || []).map(normalizePin);
  const pinsCard = normalizeCard(
    {
      kind: "pins",
      title: "常用文件夹",
      x: 0,
      y: 0,
      w: 2,
      h: 1,
      pins: pins.slice(0, pinCapacity(2, 1)),
    },
    folderId,
    0,
  );
  const histCard = normalizeCard(
    {
      kind: "history",
      title: "历史搜索",
      x: 0,
      y: 1,
      w: 2,
      h: 1,
      pins: [],
    },
    folderId,
    1,
  );
  return [pinsCard, histCard];
}

function migrateV1(raw) {
  const folderId = uid();
  const folders = [{ id: folderId, name: "常用", order: 0 }];
  const cats = Array.isArray(raw?.categories) ? raw.categories : [];
  const pins = cats.map((c) =>
    normalizePin({
      id: c.id || uid(),
      name: c.name,
      path: c.path,
      color: c.color,
    }),
  );
  const history = Array.isArray(raw?.history)
    ? raw.history
        .map((h) => String(h || "").trim())
        .filter(Boolean)
        .slice(0, HISTORY_MAX)
    : [];
  return {
    version: 2,
    folders,
    cards: defaultBoard(folderId, pins),
    history,
  };
}

function ensureHistoryCard(store, folderId) {
  const has = store.cards.some(
    (c) => c.folderId === folderId && c.kind === "history",
  );
  if (has) return store;
  const others = store.cards.filter((c) => c.folderId === folderId);
  let y = 0;
  for (const c of others) y = Math.max(y, c.y + c.h);
  store.cards.push(
    normalizeCard(
      { kind: "history", title: "历史搜索", x: 0, y, w: 2, h: 1 },
      folderId,
      store.cards.length,
    ),
  );
  return store;
}

function resolveCollisions(store) {
  for (const folder of store.folders) {
    const list = store.cards
      .filter((c) => c.folderId === folder.id)
      .sort((a, b) => a.y - b.y || a.x - b.x || a.order - b.order);
    const pack = window.FileSearchBoard?.packCards;
    const packed = pack
      ? pack(list, MAX_ROWS)
      : list.map((c, i) => ({
          ...c,
          x: i % GRID_COLS,
          y: Math.floor(i / GRID_COLS),
          w: 1,
          h: 1,
        }));
    const byId = new Map(packed.map((c) => [c.id, c]));
    for (const card of list) {
      const p = byId.get(card.id);
      if (!p) continue;
      card.x = p.x;
      card.y = p.y;
      card.w = p.w;
      card.h = p.h;
    }
  }
}

function normalize(raw) {
  if (!raw || typeof raw !== "object") {
    const folderId = uid();
    return {
      version: 2,
      folders: [{ id: folderId, name: "常用", order: 0 }],
      cards: defaultBoard(folderId, []),
      history: [],
    };
  }

  if (raw.version !== 2 && Array.isArray(raw.categories)) {
    return migrateV1(raw);
  }

  const history = Array.isArray(raw.history)
    ? raw.history
        .map((h) => String(h || "").trim())
        .filter(Boolean)
        .slice(0, HISTORY_MAX)
    : [];

  let folders = Array.isArray(raw.folders)
    ? raw.folders.map((f, i) => ({
        id: f.id || uid(),
        name: String(f.name || "未命名").trim() || "未命名",
        order: typeof f.order === "number" ? f.order : i,
      }))
    : [];
  folders.sort((a, b) => a.order - b.order);

  if (!folders.length) {
    const folderId = uid();
    folders = [{ id: folderId, name: "常用", order: 0 }];
    return {
      version: 2,
      folders,
      cards: defaultBoard(folderId, []),
      history,
    };
  }

  let cards = Array.isArray(raw.cards)
    ? raw.cards.map((c, i) =>
        normalizeCard(c, c.folderId || folders[0].id, i),
      )
    : [];

  const folderIds = new Set(folders.map((f) => f.id));
  cards = cards.filter((c) => folderIds.has(c.folderId));

  const store = { version: 2, folders, cards, history };
  for (const f of folders) ensureHistoryCard(store, f.id);
  resolveCollisions(store);
  return store;
}

function cardsInFolder(folderId) {
  return state.store.cards.filter((c) => c.folderId === folderId);
}

function findFreeSlot(folderId, w, h, ignoreId) {
  const ww = clampSpan(w);
  let hh = clampSpan(h);
  if (hh > MAX_ROWS) hh = MAX_ROWS;
  const others = cardsInFolder(folderId).filter((c) => c.id !== ignoreId);
  const trySizes = [
    { w: ww, h: hh },
    { w: ww, h: 1 },
    { w: 1, h: hh },
    { w: 1, h: 1 },
  ];
  for (const size of trySizes) {
    for (let y = 0; y <= MAX_ROWS - size.h; y += 1) {
      for (let x = 0; x <= GRID_COLS - size.w; x += 1) {
        const probe = { x, y, w: size.w, h: size.h };
        if (!others.some((c) => rectsOverlap(probe, cardRect(c)))) {
          return probe;
        }
      }
    }
  }
  return null;
}

function canPlace(folderId, rect, ignoreId) {
  if (rect.x < 0 || rect.y < 0) return false;
  if (rect.w < 1 || rect.h < 1 || rect.w > MAX_SPAN || rect.h > MAX_SPAN) {
    return false;
  }
  if (rect.x + rect.w > GRID_COLS) return false;
  if (rect.y + rect.h > MAX_ROWS) return false;
  return !cardsInFolder(folderId).some(
    (c) => c.id !== ignoreId && rectsOverlap(rect, cardRect(c)),
  );
}

function activeFolder() {
  const id = state.activeFolderId;
  const folders = state.store.folders;
  if (id && folders.some((f) => f.id === id)) {
    return folders.find((f) => f.id === id);
  }
  return folders[0] || null;
}

function activeTypeFilter() {
  return (
    TYPE_FILTERS.find((f) => f.id === state.typeFilterId) || TYPE_FILTERS[0]
  );
}

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
  state.activeFolderId = state.store.folders[0]?.id || null;
  if (raw && raw.version !== 2) await saveStore();
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
  state.editing = false;
  state.form = null;
  state.hitMenuIndex = null;
  unmountBoard();
  state.loading = true;
  state.error = null;
  render();
  const searchGen = (state._searchGen = (state._searchGen || 0) + 1);
  try {
    const hubApi = hub();
    if (!hubApi?.everything?.search) {
      throw new Error("hub.everything 不可用（插件未就绪）");
    }
    const searchPromise = hubApi.everything.search(buildEverythingQuery(q), {
      max: 60,
    });
    const timeoutPromise = new Promise((_, reject) => {
      window.setTimeout(
        () => reject(new Error("搜索超时，请确认 Everything 正在运行后重试")),
        6000,
      );
    });
    const res = await Promise.race([searchPromise, timeoutPromise]);
    if (searchGen !== state._searchGen) return;
    state.results = Array.isArray(res?.results) ? res.results : [];
    state.total = Number(res?.total) || state.results.length;
    await pushHistory(q);
  } catch (e) {
    if (searchGen !== state._searchGen) return;
    state.results = [];
    state.total = 0;
    state.error = String(e?.message || e);
  } finally {
    if (searchGen === state._searchGen) {
      state.loading = false;
      render();
    }
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

async function openPath(path) {
  if (!path) return;
  try {
    await hub().everything.open(path);
  } catch (e) {
    state.error = String(e?.message || e);
    render();
  }
}

function basenameFromPath(path) {
  const s = String(path || "").replace(/[\\/]+$/, "");
  const parts = s.split(/[\\/]/);
  return parts[parts.length - 1] || s || "未命名";
}

/* ---------- mutations ---------- */

async function addFolder(name) {
  const n = String(name || "").trim() || "新分类";
  const folder = {
    id: uid(),
    name: n,
    order: state.store.folders.length,
  };
  state.store.folders.push(folder);
  state.store.cards.push(
    ...defaultBoard(folder.id, []).map((c, i) => ({ ...c, order: i })),
  );
  state.activeFolderId = folder.id;
  await saveStore();
  render();
}

async function renameFolder(id, name) {
  const f = state.store.folders.find((x) => x.id === id);
  if (!f) return;
  f.name = String(name || "").trim() || f.name;
  await saveStore();
  render();
}

async function deleteFolder(id) {
  if (state.store.folders.length <= 1) return;
  state.store.folders = state.store.folders.filter((f) => f.id !== id);
  state.store.cards = state.store.cards.filter((c) => c.folderId !== id);
  if (state.activeFolderId === id) {
    state.activeFolderId = state.store.folders[0]?.id || null;
  }
  await saveStore();
  render();
}

async function addCardGroup() {
  const folder = activeFolder();
  if (!folder) return;
  const slot = findFreeSlot(folder.id, 2, 1, null);
  if (!slot) return;
  const card = normalizeCard(
    { kind: "pins", title: "卡片组", ...slot, pins: [] },
    folder.id,
    state.store.cards.length,
  );
  state.store.cards.push(card);
  await saveStore();
  render();
}

async function deleteCard(cardId) {
  const card = state.store.cards.find((c) => c.id === cardId);
  if (!card || card.kind === "history") return;
  state.store.cards = state.store.cards.filter((c) => c.id !== cardId);
  await saveStore();
  render();
}

async function renameCard(cardId, title) {
  const card = state.store.cards.find((c) => c.id === cardId);
  if (!card) return;
  card.title = String(title || "").trim() || card.title;
  await saveStore();
  render();
}

/**
 * @returns {"ok"|"full"|"dup"|"missing"}
 */
async function addPin(cardId, path, name, opts = {}) {
  const card = state.store.cards.find((c) => c.id === cardId);
  if (!card || card.kind !== "pins") return "missing";
  const p = String(path || "").trim();
  if (!p) return "missing";
  const cap = pinCapacity(card.w, card.h);
  if (card.pins.length >= cap) return "full";
  const key = String(p || "").trim();
  if (card.pins.some((pin) => pathsEqual(pin.path, key))) {
    return "dup";
  }
  card.pins.push(
    normalizePin({
      name: String(name || "").trim() || basenameFromPath(p),
      path: p,
    }),
  );
  await saveStore();
  if (opts.render !== false) render();
  return "ok";
}

function flashMsg(text) {
  state.flash = String(text || "");
  render();
  window.setTimeout(() => {
    if (state.flash === text) {
      state.flash = null;
      if (state.view === "results") render();
    }
  }, 1600);
}

async function addHitToCard(hitIndex, cardId) {
  const hit = state.results[hitIndex];
  const path = hit?.fullPath || hit?.full_path;
  if (!path) return;
  const card = state.store.cards.find((c) => c.id === cardId);
  const folder = state.store.folders.find((f) => f.id === card?.folderId);
  const label = folder
    ? `${folder.name} · ${card?.title || "卡片组"}`
    : card?.title || "卡片组";
  const status = await addPin(cardId, path, hit?.name || "", { render: false });
  state.hitMenuIndex = null;
  if (status === "ok") flashMsg(`已添加到「${label}」`);
  else if (status === "dup") flashMsg("该路径已在卡片组中（不是已满）");
  else if (status === "full") flashMsg("卡片组已满，请先扩容或移除");
  else flashMsg("无法添加");
}

async function removePin(cardId, pinId) {
  const card = state.store.cards.find((c) => c.id === cardId);
  if (!card || card.kind !== "pins") return;
  card.pins = card.pins.filter((p) => p.id !== pinId);
  await saveStore();
  render();
}

async function applyLayout(items) {
  const folder = activeFolder();
  if (!folder) return;
  const map = new Map(cardsInFolder(folder.id).map((c) => [c.id, c]));
  let changed = false;
  for (const it of items || []) {
    const card = map.get(it.id);
    if (!card) continue;
    const x = Math.max(0, Number(it.x) || 0);
    const y = Math.max(0, Number(it.y) || 0);
    const w = clampSpan(it.w);
    const h = Math.min(MAX_ROWS, clampSpan(it.h));
    if (card.x !== x || card.y !== y || card.w !== w || card.h !== h) {
      changed = true;
    }
    card.x = x;
    card.y = y;
    card.w = w;
    card.h = h;
  }
  if (!changed) return;
  await saveStore();
  // Soft-refresh board props without wiping the React tree mid-interaction
  if (boardApi?.update && state.view === "home") {
    boardApi.update(boardProps());
  } else {
    render();
  }
}

async function reorderCards(orderedIds) {
  const folder = activeFolder();
  if (!folder) return;
  const map = new Map(cardsInFolder(folder.id).map((c) => [c.id, c]));
  const ordered = orderedIds.map((id) => map.get(id)).filter(Boolean);
  const pack = window.FileSearchBoard?.packCards;
  const packed = pack
    ? pack(ordered, MAX_ROWS)
    : ordered.map((c, i) => ({
        ...c,
        x: i % GRID_COLS,
        y: Math.min(MAX_ROWS - 1, Math.floor(i / GRID_COLS)),
        w: 1,
        h: 1,
      }));
  await applyLayout(
    packed.map((p) => ({ id: p.id, x: p.x, y: p.y, w: p.w, h: p.h })),
  );
}

async function resizeCard(cardId, w, h) {
  const card = state.store.cards.find((c) => c.id === cardId);
  if (!card) return;
  const next = {
    id: cardId,
    x: card.x,
    y: card.y,
    w: clampSpan(w),
    h: Math.min(MAX_ROWS, clampSpan(h)),
  };
  if (next.x + next.w > GRID_COLS) {
    next.x = Math.max(0, GRID_COLS - next.w);
  }
  if (next.y + next.h > MAX_ROWS) {
    next.y = Math.max(0, MAX_ROWS - next.h);
  }
  if (!canPlace(card.folderId, next, cardId)) {
    const slot = findFreeSlot(card.folderId, next.w, next.h, cardId);
    if (!slot) return;
    next.x = slot.x;
    next.y = slot.y;
    next.w = slot.w;
    next.h = slot.h;
  }
  await applyLayout([next]);
}

/* ---------- board (hello-pangea) ---------- */

function boardProps() {
  const folder = activeFolder();
  const cards = folder ? cardsInFolder(folder.id) : [];
  const renaming =
    state.form?.type === "card" ? state.form : null;
  return {
    cards: cards.map((c) => ({
      id: c.id,
      kind: c.kind,
      title: c.title,
      x: c.x,
      y: c.y,
      w: c.w,
      h: c.h,
      pins: c.pins,
    })),
    history: state.store.history,
    editing: state.editing,
    renameCardId: renaming?.cardId || null,
    renameValue: renaming?.value || "",
    onLayoutChange: (items) => {
      void applyLayout(items);
    },
    onOpenPin: (path) => {
      void openPath(path);
    },
    onHistoryClick: (q) => {
      void runSearch(q);
    },
    onAddPin: (cardId) => {
      state.form = { type: "pin", cardId };
      render();
    },
    onRemovePin: (cardId, pinId) => {
      void removePin(cardId, pinId);
    },
    onRenameCard: (cardId) => {
      const card = state.store.cards.find((c) => c.id === cardId);
      state.form = { type: "card", cardId, value: card?.title || "" };
      if (boardApi?.update) boardApi.update(boardProps());
      else render();
    },
    onRenameCommit: (cardId, title) => {
      state.form = null;
      void renameCard(cardId, title);
    },
    onRenameCancel: () => {
      state.form = null;
      if (boardApi?.update) boardApi.update(boardProps());
      else render();
    },
    onDeleteCard: (cardId) => {
      void deleteCard(cardId);
    },
  };
}

function unmountBoard() {
  try {
    boardApi?.unmount?.();
  } catch {
    /* ignore */
  }
  boardApi = null;
}

function fitBoardHeight() {
  // react-grid-layout measures rowHeight itself via ResizeObserver in board.js
}

function mountOrUpdateBoard() {
  const el = document.getElementById("boardMount");
  const api = window.FileSearchBoard;
  if (!el || !api?.mount) {
    const detail =
      (typeof window !== "undefined" && window.__whBoardErr) ||
      (!window.FileSearchBoard ? "FileSearchBoard 未定义" : "mount 缺失");
    el &&
      (el.innerHTML = `<div class="empty-mini" style="color:rgba(255,255,255,0.5);padding:12px">看板组件未加载<br/><span style="font-size:11px;opacity:.7">${detail}</span></div>`);
    return;
  }
  const props = boardProps();
  if (boardApi?.update) {
    boardApi.update(props);
  } else {
    boardApi = api.mount(el, props);
  }
  requestAnimationFrame(() => {
    fitBoardHeight();
    requestAnimationFrame(fitBoardHeight);
  });
}

/* ---------- render ---------- */

function renderChipEdit({ inputId, placeholder, value }) {
  const text = String(value || "");
  return `
    <div class="chip-edit" id="chipEdit">
      <button type="button" class="chip-btn cancel" id="formCancel" title="取消" aria-label="取消">
        <svg class="chip-ico" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M4.2 4.2l7.6 7.6M11.8 4.2L4.2 11.8" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>
        </svg>
      </button>
      <input
        class="chip-input"
        id="${escapeHtml(inputId)}"
        type="text"
        placeholder="${escapeHtml(placeholder)}"
        value="${escapeHtml(text)}"
        autocomplete="off"
        spellcheck="false"
      />
      <button type="button" class="chip-btn ok" id="formOk" title="确认" aria-label="确认">
        <svg class="chip-ico" viewBox="0 0 16 16" aria-hidden="true">
          <path d="M3.2 8.2l3.2 3.2 6.4-6.8" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>
        </svg>
      </button>
    </div>
  `;
}

function renderHome() {
  const folder = activeFolder();
  const folderId = folder?.id;
  const f = state.form;
  const renamingId =
    f?.type === "folder" && f.mode === "rename" ? f.folderId : null;

  const tabs = state.store.folders
    .map((tab) => {
      if (renamingId && tab.id === renamingId) {
        return renderChipEdit({
          inputId: "folderName",
          placeholder: "分类名",
          value: f.value || tab.name || "",
        });
      }
      return `
      <button
        type="button"
        class="folder-tab${tab.id === folderId ? " is-active" : ""}"
        data-folder="${escapeHtml(tab.id)}"
      >${escapeHtml(tab.name)}</button>`;
    })
    .join("");

  const status = !state.status?.running
    ? `<div class="banner warn">${escapeHtml(state.status?.error || "Everything 未运行")}</div>`
    : "";

  let tabsExtra = "";
  if (state.editing && f?.type === "folder" && f.mode === "add") {
    tabsExtra = renderChipEdit({
      inputId: "folderName",
      placeholder: "新分类名",
      value: "",
    });
  } else if (state.editing && !renamingId) {
    tabsExtra = `<button type="button" class="folder-tab is-add" id="addFolder" title="新建分类">+</button>`;
  }

  let actions = "";
  if (!state.editing) {
    actions = `<button type="button" class="tool-btn" id="toggleEdit">编辑</button>`;
  } else if (f?.type === "pin") {
    actions = renderChipEdit({
      inputId: "pinPath",
      placeholder: "粘贴路径后打勾",
      value: "",
    });
  } else if (f?.type === "folder" || f?.type === "card") {
    // Folder rename/add lives in tab row; card rename lives inside the card.
    actions = `<button type="button" class="tool-btn is-primary" id="toggleEdit">完成</button>`;
  } else {
    actions = `
      <button type="button" class="tool-btn" id="addCard">+ 卡片组</button>
      ${
        state.store.folders.length > 1
          ? `<button type="button" class="tool-btn danger" id="delFolder">删分类</button>`
          : ""
      }
      <button type="button" class="tool-btn" id="renameFolder">改名</button>
      <button type="button" class="tool-btn is-primary" id="toggleEdit">完成</button>
    `;
  }

  return `
    ${status}
    <div class="home-toolbar">
      <div class="folder-tabs" role="tablist">
        ${tabs}
        ${tabsExtra}
      </div>
      <div class="toolbar-actions">
        ${actions}
      </div>
    </div>
    <div id="boardMount" class="board-mount"></div>
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

function pathsEqual(a, b) {
  const norm = (p) =>
    String(p || "")
      .trim()
      .replace(/[\\/]+$/, "")
      .toLowerCase();
  const x = norm(a);
  const y = norm(b);
  return !!x && !!y && x === y;
}

function renderHitMenu(hitIndex) {
  const hit = state.results[hitIndex];
  const hitPath = hit?.fullPath || hit?.full_path || "";
  const folders = state.store.folders || [];
  const sections = folders
    .map((folder) => {
      const pinCards = cardsInFolder(folder.id).filter((c) => c.kind === "pins");
      const items =
        pinCards.length === 0
          ? `<div class="hit-menu-empty">暂无卡片组</div>`
          : pinCards
              .map((card) => {
                const cap = pinCapacity(card.w, card.h);
                const n = card.pins.length;
                const remain = Math.max(0, cap - n);
                const full = remain <= 0;
                const dup = card.pins.some((p) => pathsEqual(p.path, hitPath));
                const disabled = full || dup;
                let hint;
                if (full && dup) hint = "已满";
                else if (full) hint = "已满";
                else if (dup) hint = `已在组内·剩${remain}`;
                else hint = `${n}/${cap}`;
                return `
                  <button
                    type="button"
                    class="hit-menu-item${disabled ? " is-disabled" : ""}"
                    data-add-card="${escapeHtml(card.id)}"
                    data-hit="${hitIndex}"
                    title="${escapeHtml(
                      full
                        ? "卡片组已满，请先扩容或移除快捷"
                        : dup
                          ? "该路径已在此卡片组中（仍可添加其它项）"
                          : `还可添加 ${remain} 个`,
                    )}"
                    ${disabled ? "disabled" : ""}
                  >
                    <span class="hit-menu-item-title">${escapeHtml(card.title)}</span>
                    <span class="hit-menu-item-cap">${escapeHtml(hint)}</span>
                  </button>`;
              })
              .join("");
      return `
        <div class="hit-menu-section">
          <div class="hit-menu-section-label">${escapeHtml(folder.name)}</div>
          ${items}
        </div>`;
    })
    .join("");

  return `
    <div class="hit-menu" role="menu" aria-label="添加到卡片组">
      <div class="hit-menu-head">添加到卡片组</div>
      <div class="hit-menu-note">已在组内 ≠ 已满；有空位仍可加其它路径</div>
      <div class="hit-menu-body">
        ${sections || `<div class="hit-menu-empty">请先在主页创建分类与卡片组</div>`}
      </div>
    </div>`;
}

function renderResults() {
  const hits = state.results
    .map((r, i) => {
      const kind = r.isFolder ? "夹" : "文";
      const size = r.isFolder ? "" : formatSize(r.size);
      const menuOpen = state.hitMenuIndex === i;
      return `
        <div class="hit${menuOpen ? " is-menu-open" : ""}" data-i="${i}">
          <button type="button" class="hit-main" data-i="${i}" title="${escapeHtml(r.fullPath)}">
            <span class="hit-kind">${kind}</span>
            <span class="hit-body">
              <span class="hit-name">${escapeHtml(r.name)}</span>
              <span class="hit-path">${escapeHtml(r.path)}</span>
            </span>
            <span class="hit-size">${escapeHtml(size)}</span>
          </button>
          <button
            type="button"
            class="hit-more${menuOpen ? " is-open" : ""}"
            data-more="${i}"
            title="更多"
            aria-label="更多"
            aria-expanded="${menuOpen ? "true" : "false"}"
          >
            <svg class="hit-more-ico" viewBox="0 0 16 16" aria-hidden="true">
              <circle cx="8" cy="3.5" r="1.35" fill="currentColor"/>
              <circle cx="8" cy="8" r="1.35" fill="currentColor"/>
              <circle cx="8" cy="12.5" r="1.35" fill="currentColor"/>
            </svg>
          </button>
          ${menuOpen ? renderHitMenu(i) : ""}
        </div>`;
    })
    .join("");

  const filterLabel = activeTypeFilter().label;
  const metaLabel = state.loading
    ? "搜索中…"
    : state.flash
      ? escapeHtml(state.flash)
      : state.error
        ? escapeHtml(state.error)
        : `「${escapeHtml(state.query)}」· ${filterLabel} · ${state.total} 条`;

  return `
    <div class="results-layout">
      ${renderTypeRail()}
      <div class="results-main">
        <div class="results-meta">
          <span class="results-meta-text${state.flash ? " is-flash" : ""}">${metaLabel}</span>
          <button type="button" class="link" id="backHome">返回</button>
        </div>
        <div class="results-list">
          ${
            state.loading
              ? `<div class="empty-mini">搜索中…</div>`
              : hits || `<div class="empty-mini">${state.error ? escapeHtml(state.error) : "无结果"}</div>`
          }
        </div>
      </div>
    </div>
  `;
}

function render() {
  const root = document.getElementById("root");
  if (!root) return;
  const keepBoard = state.view === "home" && boardApi;
  // Unmount React board before wiping DOM when leaving home or full remount needed
  if (state.view !== "home") {
    unmountBoard();
  } else if (keepBoard) {
    // Soft-update path: only remount shell if structure markers missing
  }

  root.classList.toggle("is-editing", state.editing && state.view === "home");
  root.innerHTML =
    state.view === "results" ? renderResults() : renderHome();
  // innerHTML cleared React root — always remount board on home
  if (state.view === "home") {
    boardApi = null;
    mountOrUpdateBoard();
  }
  bind();
}

function bindForm() {
  const submit = () => {
    const f = state.form;
    if (!f) return;
    if (f.type === "pin") {
      const path = document.getElementById("pinPath")?.value || "";
      state.form = null;
      void addPin(f.cardId, path, "");
      return;
    }
    if (f.type === "card") {
      const title = document.getElementById("cardTitle")?.value || "";
      state.form = null;
      void renameCard(f.cardId, title);
      return;
    }
    if (f.type === "folder") {
      const name = document.getElementById("folderName")?.value || "";
      state.form = null;
      if (f.mode === "rename" && f.folderId) void renameFolder(f.folderId, name);
      else void addFolder(name);
    }
  };

  document.getElementById("formCancel")?.addEventListener("click", () => {
    state.form = null;
    render();
  });
  document.getElementById("formOk")?.addEventListener("click", submit);

  const focusEl =
    document.getElementById("pinPath") ||
    document.getElementById("cardTitle") ||
    document.getElementById("folderName");

  let measureCanvas = null;
  const fitChipInput = (el) => {
    if (!el) return;
    const cs = getComputedStyle(el);
    if (!measureCanvas) measureCanvas = document.createElement("canvas");
    const ctx = measureCanvas.getContext("2d");
    if (!ctx) return;
    ctx.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
    const sample = el.value || "字字"; // empty → ~2 Chinese chars
    const raw = Math.ceil(ctx.measureText(sample).width) + 4;
    const minW = Math.ceil(ctx.measureText("字字").width) + 4;
    const maxW = 120;
    const w = Math.max(minW, Math.min(maxW, raw));
    el.style.width = `${w}px`;
  };

  fitChipInput(focusEl);
  focusEl?.focus?.();
  focusEl?.select?.();
  focusEl?.addEventListener("input", () => fitChipInput(focusEl));
  focusEl?.addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      submit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      state.form = null;
      render();
    }
  });
}

function bind() {
  document.querySelectorAll(".folder-tab[data-folder]").forEach((el) => {
    el.addEventListener("click", () => {
      state.activeFolderId = el.getAttribute("data-folder");
      render();
    });
  });

  document.getElementById("toggleEdit")?.addEventListener("click", () => {
    state.editing = !state.editing;
    state.form = null;
    render();
  });

  document.getElementById("addCard")?.addEventListener("click", () => {
    void addCardGroup();
  });

  document.getElementById("addFolder")?.addEventListener("click", () => {
    state.form = { type: "folder", mode: "add", value: "" };
    render();
  });

  document.getElementById("renameFolder")?.addEventListener("click", () => {
    const f = activeFolder();
    if (!f) return;
    state.form = {
      type: "folder",
      mode: "rename",
      folderId: f.id,
      value: f.name,
    };
    render();
  });

  document.getElementById("delFolder")?.addEventListener("click", () => {
    const f = activeFolder();
    if (f) void deleteFolder(f.id);
  });

  document.querySelectorAll(".type-item").forEach((el) => {
    el.addEventListener("click", () => {
      const id = el.getAttribute("data-type");
      if (id) void setTypeFilter(id);
    });
  });

  document.querySelectorAll(".hit-main").forEach((el) => {
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

  document.querySelectorAll(".hit-more").forEach((el) => {
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      const i = Number(el.getAttribute("data-more"));
      if (!Number.isFinite(i)) return;
      state.hitMenuIndex = state.hitMenuIndex === i ? null : i;
      render();
    });
  });

  document.querySelectorAll("[data-add-card]").forEach((el) => {
    el.addEventListener("click", (e) => {
      e.stopPropagation();
      const cardId = el.getAttribute("data-add-card");
      const i = Number(el.getAttribute("data-hit"));
      if (cardId && Number.isFinite(i)) void addHitToCard(i, cardId);
    });
  });

  if (state.hitMenuIndex != null) {
    const menu = document.querySelector(".hit-menu");
    const btn = document.querySelector(".hit-more.is-open");
    if (menu && btn) {
      const r = btn.getBoundingClientRect();
      const mw = menu.offsetWidth || 220;
      const mh = menu.offsetHeight || 160;
      let top = r.bottom + 4;
      let left = r.right - mw;
      if (top + mh > window.innerHeight - 8) {
        top = Math.max(8, r.top - mh - 4);
      }
      if (left < 8) left = 8;
      if (left + mw > window.innerWidth - 8) {
        left = Math.max(8, window.innerWidth - mw - 8);
      }
      menu.style.top = `${top}px`;
      menu.style.left = `${left}px`;
    }
    const closeMenu = (e) => {
      const t = e.target;
      if (t?.closest?.(".hit-menu") || t?.closest?.(".hit-more")) return;
      state.hitMenuIndex = null;
      render();
    };
    // next tick so the opening click does not instantly close
    window.setTimeout(() => {
      document.addEventListener("click", closeMenu, { once: true });
    }, 0);
  }

  document.getElementById("backHome")?.addEventListener("click", () => {
    state.view = "home";
    state.query = "";
    state.results = [];
    state.typeFilterId = "all";
    state.hitMenuIndex = null;
    state.flash = null;
    render();
  });

  bindForm();
  window.addEventListener("resize", fitBoardHeight);
}

function onHostSearch(ev) {
  const d = ev?.detail;
  if (!d) return;
  if (d.action === "openFavorites") {
    void openFavoritesHome();
    return;
  }
  if (d.action !== "submit") return;
  void runSearch(d.query || "");
}

/** Host hotkey: expand to card-group home on the 「常用」 folder. */
async function openFavoritesHome() {
  const fav =
    state.store.folders.find((f) => f.name === "常用") ||
    state.store.folders[0] ||
    null;
  if (fav) state.activeFolderId = fav.id;
  state.query = "";
  state.view = "home";
  state.results = [];
  state.total = 0;
  state.error = null;
  state.loading = false;
  state.typeFilterId = "all";
  state.editing = false;
  state.form = null;
  state.hitMenuIndex = null;
  render();
}

function withTimeout(promise, ms, label) {
  return Promise.race([
    promise,
    new Promise((_, reject) => {
      window.setTimeout(
        () => reject(new Error(`${label}超时（${Math.round(ms / 1000)}s）`)),
        ms,
      );
    }),
  ]);
}

async function boot() {
  await withTimeout(loadStore(), 5000, "读取配置");
  await withTimeout(refreshStatus(), 4000, "检测 Everything");
  render();
  window.addEventListener("wh-island-search", onHostSearch);

  // Host posts leave while expand animation is still running (panelActive=false).
  // Bootstrap also sync-fires onLeave when registering during phase===leave —
  // that used to unmountBoard() right after the first render and leave a blank panel.
  let panelSessionLive = false;
  hub().panel?.onEnter?.(() => {
    panelSessionLive = true;
    if (state.view === "home") render();
  });
  hub().panel?.onLeave?.(() => {
    if (!panelSessionLive) return;
    panelSessionLive = false;
    state.view = "home";
    state.query = "";
    state.results = [];
    state.error = null;
    state.typeFilterId = "all";
    state.editing = false;
    state.form = null;
    state.hitMenuIndex = null;
    state.flash = null;
    unmountBoard();
  });
}

boot().catch((e) => {
  console.error("[file-search/panel]", e);
  const root = document.getElementById("root");
  if (root) {
    root.innerHTML = `<div class="empty-mini">启动失败：${escapeHtml(e?.message || e)}</div>`;
  }
});

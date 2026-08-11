/**
 * App Library — hosted popup UI.
 * Uses injected window.hub. Never use alert / confirm / prompt.
 */
const PLUGIN_ID =
  window.__WH_PLUGIN_ID__ ||
  (window.hub && window.hub.pluginId) ||
  "com.window-hub.app-library";

(function blockNativeDialogs() {
  const ban = (name, fallback) => {
    try {
      window[name] = function bannedNativeDialog(...args) {
        console.error(
          `[app-library] forbidden window.${name}() — use in-app UI`,
          args,
        );
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
  if (!window.hub) throw new Error("window.hub missing — open via host popup");
  return window.hub;
};

const TILE_COLORS = [
  "#3dd6c6",
  "#6ea8fe",
  "#f0b429",
  "#f07178",
  "#7bd88f",
  "#c792ea",
  "#82aaff",
  "#ffcb6b",
];

const state = {
  store: { version: 1, apps: [] },
  windows: [],
  mode: "library", // library | add-pick | add-manual | edit
  query: "",
  editingId: null,
  draftName: "",
  draftExe: "",
  toast: null,
  confirm: null,
};

function getAppEl() {
  return document.getElementById("app");
}

function uid() {
  return crypto.randomUUID
    ? crypto.randomUUID()
    : `a-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function colorFor(name) {
  const s = String(name || "");
  let h = 0;
  for (let i = 0; i < s.length; i += 1) h = (h * 31 + s.charCodeAt(i)) >>> 0;
  return TILE_COLORS[h % TILE_COLORS.length];
}

function initialOf(name) {
  const t = String(name || "").trim();
  if (!t) return "?";
  return t.slice(0, 1).toUpperCase();
}

function normalize(raw) {
  const parsed = raw || {};
  const apps = Array.isArray(parsed.apps)
    ? parsed.apps.map((a) => ({
        id: a.id || uid(),
        name: String(a.name || "未命名").trim() || "未命名",
        color: a.color || colorFor(a.name),
        bind: {
          exe: a.bind?.exe ? String(a.bind.exe) : undefined,
          titleIncludes: a.bind?.titleIncludes
            ? String(a.bind.titleIncludes)
            : undefined,
        },
        pinned: !!a.pinned,
        lastHwnd: a.lastHwnd ?? undefined,
        order: typeof a.order === "number" ? a.order : 0,
      }))
    : [];
  return { version: 1, apps };
}

function escapeHtml(s) {
  return String(s ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function escapeAttr(s) {
  return escapeHtml(s).replaceAll("'", "&#39;");
}

async function save() {
  try {
    await hub().storage.set("store", state.store);
  } catch (err) {
    console.error("[app-library] save failed", err);
  }
  render();
}

function patch(mutator) {
  const next = mutator(state.store);
  if (next !== state.store) {
    state.store = next;
    void save();
  } else {
    render();
  }
}

function showToast(msg) {
  state.toast = msg;
  render();
  window.setTimeout(() => {
    if (state.toast === msg) {
      state.toast = null;
      render();
    }
  }, 1600);
}

function matchWindow(app, win, used) {
  if (used.has(win.hwnd)) return false;
  const bind = app.bind || {};
  if (bind.exe) {
    const exe = (win.exe_name ?? win.exe ?? "").toLowerCase();
    if (!exe.includes(String(bind.exe).toLowerCase())) return false;
  }
  if (
    bind.titleIncludes &&
    !win.title.toLowerCase().includes(String(bind.titleIncludes).toLowerCase())
  ) {
    return false;
  }
  return !!(bind.exe || bind.titleIncludes);
}

function rebindApps(apps) {
  const used = new Set();
  return (apps || []).map((app) => {
    if (app.lastHwnd != null) {
      const live = state.windows.find((w) => w.hwnd === app.lastHwnd);
      if (live) {
        used.add(live.hwnd);
        return { ...app, lastHwnd: live.hwnd };
      }
    }
    const candidate = state.windows.find((w) => matchWindow(app, w, used));
    if (candidate) {
      used.add(candidate.hwnd);
      return { ...app, lastHwnd: candidate.hwnd };
    }
    return { ...app, lastHwnd: undefined };
  });
}

function boundHwnds() {
  const set = new Set();
  for (const a of state.store.apps) {
    if (a.lastHwnd != null) set.add(a.lastHwnd);
  }
  return set;
}

function filteredApps() {
  const q = state.query.trim().toLowerCase();
  const list = [...state.store.apps].sort((a, b) => {
    if (!!b.pinned !== !!a.pinned) return b.pinned ? 1 : -1;
    return (a.order || 0) - (b.order || 0) || a.name.localeCompare(b.name, "zh");
  });
  if (!q) return list;
  return list.filter((a) => {
    const hay = `${a.name} ${a.bind?.exe || ""} ${a.bind?.titleIncludes || ""}`.toLowerCase();
    return hay.includes(q);
  });
}

function addFromWindow(win) {
  const name = (win.title || "").trim() || win.exe_name || "应用";
  const exe = (win.exe_name || "").replace(/\.exe$/i, "");
  const app = {
    id: uid(),
    name: name.slice(0, 48),
    color: colorFor(name),
    bind: {
      exe: exe || undefined,
      titleIncludes: name.slice(0, 32) || undefined,
    },
    pinned: false,
    lastHwnd: win.hwnd,
    order: state.store.apps.length,
  };
  return { ...state.store, apps: [...state.store.apps, app] };
}

function addManual(name, exe) {
  const cleanName = (name || "").trim() || "未命名";
  const cleanExe = (exe || "").trim().replace(/\.exe$/i, "");
  if (!cleanExe && !cleanName) return state.store;
  const app = {
    id: uid(),
    name: cleanName,
    color: colorFor(cleanName),
    bind: {
      exe: cleanExe || undefined,
      titleIncludes: cleanExe ? undefined : cleanName.slice(0, 32),
    },
    pinned: false,
    lastHwnd: undefined,
    order: state.store.apps.length,
  };
  return { ...state.store, apps: [...state.store.apps, app] };
}

function updateApp(id, patchFields) {
  return {
    ...state.store,
    apps: state.store.apps.map((a) =>
      a.id === id ? { ...a, ...patchFields } : a,
    ),
  };
}

function removeApp(id) {
  return {
    ...state.store,
    apps: state.store.apps.filter((a) => a.id !== id),
  };
}

function togglePin(id) {
  return {
    ...state.store,
    apps: state.store.apps.map((a) =>
      a.id === id ? { ...a, pinned: !a.pinned } : a,
    ),
  };
}

async function focusApp(app) {
  if (app.lastHwnd == null) {
    showToast("应用未在运行");
    return;
  }
  try {
    await hub().windows.focus(`hwnd:${app.lastHwnd}`);
    await hub().popup.close();
  } catch (err) {
    console.error(err);
    showToast("切换失败");
  }
}

function onWindows(list) {
  state.windows = list || [];
  const nextApps = rebindApps(state.store.apps);
  const next = { ...state.store, apps: nextApps };
  const changed = JSON.stringify(next) !== JSON.stringify(state.store);
  state.store = next;
  if (changed) {
    void hub()
      .storage.set("store", state.store)
      .catch(() => undefined);
  }
  render();
}

function renderLibraryBody() {
  const apps = filteredApps();
  if (!state.store.apps.length) {
    return `
      <div class="al-empty">
        <p>还没有应用</p>
        <p class="al-muted">从正在运行的窗口添加，或手填名称与 exe</p>
        <button type="button" class="al-primary-btn" id="to-add">添加应用</button>
      </div>
    `;
  }
  if (!apps.length) {
    return `<div class="al-empty"><p>没有匹配「${escapeHtml(state.query)}」的应用</p></div>`;
  }
  return `
    <div class="al-grid">
      ${apps
        .map((app) => {
          const live = app.lastHwnd != null;
          return `
            <button type="button" class="al-tile${live ? " is-live" : " is-stale"}" data-open="${escapeAttr(app.id)}" title="${escapeAttr(app.name)}${live ? "" : "（未运行）"}">
              <span class="al-avatar" style="background:${escapeAttr(app.color)}">${escapeHtml(initialOf(app.name))}</span>
              <span class="al-tile-name">${escapeHtml(app.name)}</span>
              <span class="al-tile-meta">${live ? "运行中" : "未运行"}${app.pinned ? " · 已固定" : ""}</span>
            </button>
          `;
        })
        .join("")}
    </div>
  `;
}

function renderAddPick() {
  const bound = boundHwnds();
  const rows = state.windows.length
    ? state.windows
        .map((w) => {
          const already = bound.has(w.hwnd);
          return `
            <button type="button" class="al-row" data-add-hwnd="${w.hwnd}" ${already ? "disabled" : ""} title="${escapeAttr(w.title)}">
              <span class="al-row-title">${escapeHtml(w.title || "(无标题)")}</span>
              <span class="al-row-meta">${already ? "已在库中" : escapeHtml(w.exe_name || `pid ${w.pid}`)}</span>
            </button>
          `;
        })
        .join("")
    : `<div class="al-empty"><p>没有可用窗口</p></div>`;
  return `
    <div class="al-section">
      <div class="al-section-label">选择正在运行的窗口加入应用库</div>
      <div class="al-list">${rows}</div>
      <button type="button" class="al-text-btn" id="to-manual">改为手填…</button>
    </div>
  `;
}

function renderAddManual() {
  return `
    <div class="al-section">
      <div class="al-section-label">手填应用（exe 关键字用于匹配窗口）</div>
      <div class="al-form-col">
        <input class="al-input" id="manual-name" placeholder="显示名称，如 Cursor" value="${escapeAttr(state.draftName)}" />
        <input class="al-input" id="manual-exe" placeholder="exe 关键字，如 Cursor" value="${escapeAttr(state.draftExe)}" />
        <div class="al-form-row">
          <button type="button" class="al-text-btn" id="to-pick">从窗口选</button>
          <button type="button" class="al-primary-btn" id="manual-save">添加</button>
        </div>
      </div>
    </div>
  `;
}

function renderEdit() {
  const app = state.store.apps.find((a) => a.id === state.editingId);
  if (!app) {
    state.mode = "library";
    return renderLibraryBody();
  }
  return `
    <div class="al-section">
      <div class="al-section-label">编辑「${escapeHtml(app.name)}」</div>
      <div class="al-form-col">
        <input class="al-input" id="edit-name" value="${escapeAttr(state.draftName)}" placeholder="显示名称" />
        <input class="al-input" id="edit-exe" value="${escapeAttr(state.draftExe)}" placeholder="exe 关键字" />
        <div class="al-form-row">
          <button type="button" class="al-text-btn" id="edit-pin">${app.pinned ? "取消固定" : "固定到快捷区"}</button>
          <button type="button" class="al-text-btn is-danger" id="edit-remove">删除</button>
          <button type="button" class="al-primary-btn" id="edit-save">保存</button>
        </div>
      </div>
    </div>
  `;
}

function render() {
  const app = getAppEl();
  if (!app) return;

  let body = "";
  if (state.mode === "add-pick") body = renderAddPick();
  else if (state.mode === "add-manual") body = renderAddManual();
  else if (state.mode === "edit") body = renderEdit();
  else body = renderLibraryBody();

  const confirm = state.confirm;
  const showSearch = state.mode === "library" && state.store.apps.length > 0;

  app.innerHTML = `
    <header class="al-header">
      <div class="al-title-wrap">
        <div class="al-title">应用库</div>
        <div class="al-sub">${state.store.apps.length} 个应用</div>
      </div>
      <div class="al-header-actions">
        ${
          state.mode === "library"
            ? `<button type="button" class="al-icon-btn" id="to-add" title="添加">+</button>`
            : `<button type="button" class="al-text-btn" id="back-library">返回</button>`
        }
        <button type="button" class="al-icon-btn" id="close" title="关闭">×</button>
      </div>
    </header>
    ${
      showSearch
        ? `<div class="al-search-row">
            <input class="al-input" id="search" placeholder="搜索应用…" value="${escapeAttr(state.query)}" />
          </div>`
        : ""
    }
    ${body}
    ${
      state.mode === "library" && state.store.apps.length
        ? `<p class="al-hint">点击打开 · 右键或长按编辑</p>`
        : ""
    }
    ${
      state.toast
        ? `<div class="al-toast" role="status">${escapeHtml(state.toast)}</div>`
        : ""
    }
    ${
      confirm
        ? `
      <div class="al-confirm" role="dialog" aria-modal="true">
        <div class="al-confirm-card">
          <p class="al-confirm-msg">${escapeHtml(confirm.message)}</p>
          <div class="al-confirm-actions">
            <button type="button" class="al-text-btn" id="confirm-cancel">取消</button>
            <button type="button" class="al-primary-btn is-danger" id="confirm-ok">${escapeHtml(confirm.okLabel || "删除")}</button>
          </div>
        </div>
      </div>
    `
        : ""
    }
  `;

  bindEvents();
  document.getElementById("search")?.focus();
  document.getElementById("manual-name")?.focus();
  document.getElementById("edit-name")?.focus();
  if (confirm) document.getElementById("confirm-ok")?.focus();
}

function bindEvents() {
  const q = (sel) => document.querySelector(sel);

  q("#close")?.addEventListener("click", () => void hub().popup.close());
  q("#back-library")?.addEventListener("click", () => {
    state.mode = "library";
    state.editingId = null;
    render();
  });
  q("#to-add")?.addEventListener("click", () => {
    state.mode = "add-pick";
    state.draftName = "";
    state.draftExe = "";
    render();
  });
  q("#to-manual")?.addEventListener("click", () => {
    state.mode = "add-manual";
    render();
  });
  q("#to-pick")?.addEventListener("click", () => {
    state.mode = "add-pick";
    render();
  });

  q("#search")?.addEventListener("input", (ev) => {
    state.query = ev.target.value || "";
    // Keep caret: only re-render grid section would be nicer; full render ok for small lists
    const pos = ev.target.selectionStart;
    render();
    const input = document.getElementById("search");
    if (input) {
      input.focus();
      try {
        input.setSelectionRange(pos, pos);
      } catch {
        /* ignore */
      }
    }
  });

  document.querySelectorAll("[data-open]").forEach((el) => {
    const id = el.getAttribute("data-open");
    let pressTimer = null;
    let longPressed = false;
    const openEdit = () => {
      const appItem = state.store.apps.find((a) => a.id === id);
      if (!appItem) return;
      state.mode = "edit";
      state.editingId = id;
      state.draftName = appItem.name;
      state.draftExe = appItem.bind?.exe || "";
      render();
    };
    el.addEventListener("click", () => {
      if (longPressed) {
        longPressed = false;
        return;
      }
      const appItem = state.store.apps.find((a) => a.id === id);
      if (appItem) void focusApp(appItem);
    });
    el.addEventListener("contextmenu", (ev) => {
      ev.preventDefault();
      openEdit();
    });
    el.addEventListener("pointerdown", () => {
      longPressed = false;
      pressTimer = window.setTimeout(() => {
        pressTimer = null;
        longPressed = true;
        openEdit();
      }, 550);
    });
    el.addEventListener("pointerup", () => {
      if (pressTimer) clearTimeout(pressTimer);
    });
    el.addEventListener("pointerleave", () => {
      if (pressTimer) clearTimeout(pressTimer);
    });
  });

  document.querySelectorAll("[data-add-hwnd]").forEach((el) => {
    el.addEventListener("click", () => {
      const hwnd = Number(el.getAttribute("data-add-hwnd"));
      const win = state.windows.find((w) => w.hwnd === hwnd);
      state.mode = "library";
      if (win) patch(() => addFromWindow(win));
      else render();
    });
  });

  q("#manual-save")?.addEventListener("click", () => {
    const name = q("#manual-name")?.value || "";
    const exe = q("#manual-exe")?.value || "";
    if (!(name || "").trim() && !(exe || "").trim()) {
      showToast("请填写名称或 exe");
      return;
    }
    state.mode = "library";
    state.draftName = "";
    state.draftExe = "";
    patch(() => addManual(name, exe));
  });

  q("#edit-save")?.addEventListener("click", () => {
    const name = (q("#edit-name")?.value || "").trim();
    const exe = (q("#edit-exe")?.value || "").trim().replace(/\.exe$/i, "");
    const id = state.editingId;
    state.mode = "library";
    state.editingId = null;
    if (!id) {
      render();
      return;
    }
    patch((s) =>
      updateApp(id, {
        name: name || "未命名",
        color: colorFor(name || "未命名"),
        bind: {
          exe: exe || undefined,
          titleIncludes: exe
            ? state.store.apps.find((a) => a.id === id)?.bind?.titleIncludes
            : (name || "").slice(0, 32) || undefined,
        },
      }),
    );
  });

  q("#edit-pin")?.addEventListener("click", () => {
    if (!state.editingId) return;
    patch(() => togglePin(state.editingId));
  });

  q("#edit-remove")?.addEventListener("click", () => {
    const id = state.editingId;
    const appItem = state.store.apps.find((a) => a.id === id);
    if (!id || !appItem) return;
    state.confirm = {
      message: `从应用库删除「${appItem.name}」？`,
      okLabel: "删除",
      onYes: () => {
        state.confirm = null;
        state.mode = "library";
        state.editingId = null;
        patch(() => removeApp(id));
      },
    };
    render();
  });

  q("#confirm-cancel")?.addEventListener("click", () => {
    state.confirm = null;
    render();
  });
  q("#confirm-ok")?.addEventListener("click", () => {
    const fn = state.confirm?.onYes;
    state.confirm = null;
    if (typeof fn === "function") fn();
    else render();
  });
}

void (async () => {
  const mount = () => {
    if (!document.getElementById("app")) {
      window.setTimeout(mount, 16);
      return;
    }
    void boot();
  };

  async function boot() {
    try {
      const raw = await hub().storage.get("store");
      state.store = normalize(raw);
    } catch (err) {
      console.error(err);
    }
    render();
    try {
      hub().windows.subscribe(onWindows);
    } catch (err) {
      console.error(err);
      const list = await hub().windows.list().catch(() => []);
      onWindows(list);
    }
  }

  mount();
})();

void PLUGIN_ID;

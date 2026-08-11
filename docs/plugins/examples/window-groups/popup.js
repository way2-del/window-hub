/**
 * Window Groups — independent plugin UI.
 * Uses injected window.hub (no host React / iframe bridge).
 *
 * NEVER use window.alert / confirm / prompt — WebView 原生对话框常抢焦点、点不到。
 * 确认删除等一律用应用内 `.wg-confirm` 遮罩。
 */
const PLUGIN_ID =
  window.__WH_PLUGIN_ID__ ||
  (window.hub && window.hub.pluginId) ||
  "com.window-hub.window-groups";

/** Block browser-native dialogs (sync APIs cannot show our in-app UI). */
(function blockNativeDialogs() {
  const ban = (name, fallback) => {
    try {
      window[name] = function bannedNativeDialog(...args) {
        console.error(
          `[window-groups] forbidden window.${name}() — use in-app UI`,
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

const state = {
  store: { version: 2, activeGroupId: null, groups: [], pins: [] },
  windows: [],
  mode: "list",
  draftName: "",
  editingAliasId: null,
  aliasDraft: "",
  /** In-app confirm; never use window.confirm / alert (WebView 原生弹窗常点不到). */
  confirm: null,
};

function getAppEl() {
  return document.getElementById("app");
}

const PIN_DEFAULT_W = 96;

function uid() {
  return crypto.randomUUID
    ? crypto.randomUUID()
    : `g-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function normalize(raw) {
  const parsed = raw || {};
  return {
    version: 2,
    activeGroupId: parsed.activeGroupId ?? null,
    groups: Array.isArray(parsed.groups) ? parsed.groups : [],
    pins: Array.isArray(parsed.pins) ? parsed.pins : [],
  };
}

function getActiveGroup() {
  const { groups, activeGroupId } = state.store;
  return groups.find((g) => g.id === activeGroupId) ?? groups[0] ?? null;
}

function findWindowItem(itemId) {
  for (const g of state.store.groups) {
    const it = (g.items || []).find((x) => x.id === itemId);
    if (it) return { group: g, item: it };
  }
  return null;
}

function isPinned(kind, refId) {
  return state.store.pins.some((p) => p.kind === kind && p.refId === refId);
}

function boundHwnds() {
  const set = new Set();
  for (const g of state.store.groups) {
    for (const it of g.items || []) {
      if (it.lastHwnd != null) set.add(it.lastHwnd);
    }
  }
  return set;
}

async function save() {
  try {
    await hub().storage.set("store", state.store);
  } catch (err) {
    console.error("[window-groups] save failed", err);
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

function createGroup(name) {
  const clean = (name || "").trim() || `组 ${state.store.groups.length + 1}`;
  const group = { id: uid(), name: clean, items: [] };
  return {
    ...state.store,
    activeGroupId: group.id,
    groups: [...state.store.groups, group],
  };
}

function renameGroup(id, name) {
  return {
    ...state.store,
    groups: state.store.groups.map((g) =>
      g.id === id ? { ...g, name: (name || "").trim() || g.name } : g,
    ),
  };
}

function deleteGroup(id) {
  const groups = state.store.groups.filter((g) => g.id !== id);
  const removed = state.store.groups.find((g) => g.id === id);
  const removedItemIds = new Set((removed?.items || []).map((it) => it.id));
  const pins = state.store.pins.filter(
    (p) =>
      !(p.kind === "group" && p.refId === id) &&
      !(p.kind === "window" && removedItemIds.has(p.refId)),
  );
  return {
    version: 2,
    activeGroupId:
      state.store.activeGroupId === id ? groups[0]?.id ?? null : state.store.activeGroupId,
    groups,
    pins,
  };
}

function setActiveGroup(id) {
  if (!state.store.groups.some((g) => g.id === id)) return state.store;
  return { ...state.store, activeGroupId: id };
}

function addWindow(groupId, win) {
  return {
    ...state.store,
    groups: state.store.groups.map((g) => {
      if (g.id !== groupId) return g;
      const items = g.items || [];
      if (items.some((it) => it.lastHwnd === win.hwnd)) return g;
      return {
        ...g,
        items: [
          ...items,
          {
            id: uid(),
            alias: win.title.trim() || win.title,
            lastHwnd: win.hwnd,
            bind: {
              exe: win.exe_name ?? undefined,
              titleIncludes: win.title.slice(0, 48) || undefined,
            },
          },
        ],
      };
    }),
  };
}

function updateAlias(groupId, itemId, alias) {
  return {
    ...state.store,
    groups: state.store.groups.map((g) =>
      g.id === groupId
        ? {
            ...g,
            items: (g.items || []).map((it) =>
              it.id === itemId ? { ...it, alias: (alias || "").trim() || it.alias } : it,
            ),
          }
        : g,
    ),
  };
}

function removeItem(groupId, itemId) {
  return {
    ...state.store,
    groups: state.store.groups.map((g) =>
      g.id === groupId
        ? { ...g, items: (g.items || []).filter((it) => it.id !== itemId) }
        : g,
    ),
    pins: state.store.pins.filter((p) => !(p.kind === "window" && p.refId === itemId)),
  };
}

function clearStale(groupId) {
  const live = new Set(state.windows.map((w) => w.hwnd));
  return {
    ...state.store,
    groups: state.store.groups.map((g) => {
      if (g.id !== groupId) return g;
      return {
        ...g,
        items: (g.items || []).filter((it) => it.lastHwnd != null && live.has(it.lastHwnd)),
      };
    }),
  };
}

function pinGroup(groupId) {
  if (isPinned("group", groupId)) return state.store;
  return {
    ...state.store,
    pins: [
      ...state.store.pins,
      { id: uid(), kind: "group", refId: groupId, width: PIN_DEFAULT_W },
    ],
  };
}

function pinWindow(itemId) {
  if (isPinned("window", itemId)) return state.store;
  return {
    ...state.store,
    pins: [
      ...state.store.pins,
      { id: uid(), kind: "window", refId: itemId, width: PIN_DEFAULT_W },
    ],
  };
}

function unpin(kind, refId) {
  return {
    ...state.store,
    pins: state.store.pins.filter((p) => !(p.kind === kind && p.refId === refId)),
  };
}

function rebind(items) {
  const live = new Map(state.windows.map((w) => [w.hwnd, w]));
  const used = new Set();
  return (items || []).map((it) => {
    if (it.lastHwnd != null && live.has(it.lastHwnd)) {
      used.add(it.lastHwnd);
      return { ...it, lastHwnd: it.lastHwnd };
    }
    const candidate = state.windows.find((w) => {
      if (used.has(w.hwnd)) return false;
      const bind = it.bind || {};
      if (bind.exe) {
        const exe = (w.exe_name ?? w.exe ?? "").toLowerCase();
        if (!exe.includes(String(bind.exe).toLowerCase())) return false;
      }
      if (
        bind.titleIncludes &&
        !w.title.toLowerCase().includes(String(bind.titleIncludes).toLowerCase())
      ) {
        return false;
      }
      if (bind.titleRegex) {
        try {
          if (!new RegExp(bind.titleRegex, "i").test(w.title)) return false;
        } catch {
          return false;
        }
      }
      return !!(bind.exe || bind.titleIncludes || bind.titleRegex);
    });
    if (candidate) {
      used.add(candidate.hwnd);
      return { ...it, lastHwnd: candidate.hwnd };
    }
    return { ...it, lastHwnd: undefined };
  });
}

function onWindows(list) {
  state.windows = list || [];
  if (state.store.groups.length) {
    const next = {
      ...state.store,
      groups: state.store.groups.map((g) => ({ ...g, items: rebind(g.items) })),
    };
    const changed = JSON.stringify(next) !== JSON.stringify(state.store);
    state.store = next;
    if (changed) {
      void hub()
        .storage.set("store", state.store)
        .catch(() => undefined);
    }
  }
  render();
}

function render() {
  const app = getAppEl();
  if (!app) {
    console.error("[window-groups] #app missing");
    return;
  }
  const active = getActiveGroup();
  const mode = state.mode;
  const staleCount = active
    ? (active.items || []).filter((it) => it.lastHwnd == null).length
    : 0;

  const tabs = state.store.groups.length
    ? state.store.groups
        .map(
          (g) => `
            <button type="button" class="wg-tab${g.id === active?.id ? " is-active" : ""}" data-group="${g.id}">
              ${escapeHtml(g.name)}
              <span class="wg-tab-count">${(g.items || []).length}</span>
            </button>
          `,
        )
        .join("")
    : '<span class="wg-hint">还没有组</span>';

  let body = "";
  if (mode === "rename" && active) {
    body = `
      <div class="wg-form">
        <input class="wg-input" id="rename-input" value="${escapeAttr(state.draftName)}" placeholder="组名称" />
        <button type="button" class="wg-primary-btn" id="rename-save">保存</button>
      </div>
    `;
  } else if (mode === "add" && active) {
    const rows = state.windows.length
      ? state.windows
          .map((w) => {
            const already = boundHwnds().has(w.hwnd);
            return `
              <button type="button" class="wg-row" data-add="${w.hwnd}" ${already ? "disabled" : ""} title="${escapeAttr(w.title)}">
                <span class="wg-row-title">${escapeHtml(w.title)}</span>
                <span class="wg-row-meta">${already ? "已在组内" : escapeHtml(w.exe_name || `pid ${w.pid}`)}</span>
              </button>
            `;
          })
          .join("")
      : '<div class="wg-empty">没有可用窗口</div>';
    body = `
      <div class="wg-section">
        <div class="wg-section-label">选择要加入「${escapeHtml(active.name)}」的窗口</div>
        <div class="wg-list">${rows}</div>
      </div>
    `;
  } else {
    let list = "";
    if (!active) {
      list = `
        <div class="wg-empty">
          创建窗口组后，可为多账号窗口起别名并快速切换
          <div class="wg-form" style="margin-top:12px">
            <input class="wg-input" id="create-input" placeholder="例如：开发者工具 · 账号A" />
            <button type="button" class="wg-primary-btn" id="create-save">创建</button>
          </div>
        </div>
      `;
    } else if (!(active.items || []).length) {
      list = `
        <div class="wg-empty">
          组内还没有窗口
          <button type="button" class="wg-primary-btn" data-to-add="1" style="margin-top:12px">添加窗口</button>
        </div>
      `;
    } else {
      list = `
        <div class="wg-list">
          ${(active.items || [])
            .map((it) => {
              const stale = it.lastHwnd == null;
              const editing = state.editingAliasId === it.id;
              if (editing) {
                return `
                  <div class="wg-form">
                    <input class="wg-input" id="alias-input" value="${escapeAttr(state.aliasDraft)}" />
                    <button type="button" class="wg-primary-btn" id="alias-save">确定</button>
                  </div>
                `;
              }
              const pinned = isPinned("window", it.id);
              return `
                <div class="wg-item${stale ? " is-stale" : ""}">
                  <button type="button" class="wg-item-main" data-focus="${it.lastHwnd ?? ""}" ${stale ? "disabled" : ""} title="${stale ? "窗口已关闭" : `切换到 ${escapeAttr(it.alias)}`}">
                    <span class="wg-row-title">${escapeHtml(it.alias)}</span>
                    <span class="wg-row-meta">${stale ? "已失效" : escapeHtml(it.bind?.exe || it.bind?.titleIncludes || "")}</span>
                  </button>
                  <div class="wg-item-actions">
                    <button type="button" class="wg-text-btn" data-edit-alias="${it.id}">别名</button>
                    <button type="button" class="wg-text-btn" data-pin-window="${it.id}">${pinned ? "取消固定" : "固定到快捷区"}</button>
                    <button type="button" class="wg-text-btn is-danger" data-remove-item="${it.id}">移除</button>
                  </div>
                </div>
              `;
            })
            .join("")}
        </div>
      `;
    }
    body = `<div class="wg-section">${list}</div>`;
  }

  const groupPinned = active ? isPinned("group", active.id) : false;
  const confirm = state.confirm;

  app.innerHTML = `
    <header class="wg-header">
      <div class="wg-title">窗口组</div>
      <button type="button" class="wg-icon-btn" id="close" title="关闭">×</button>
    </header>
    <div class="wg-groups-row">
      <div class="wg-group-tabs" role="tablist">${tabs}</div>
      <button type="button" class="wg-icon-btn" id="new-group" title="新建组">+</button>
    </div>
    ${
      active
        ? `
      <div class="wg-toolbar">
        <button type="button" class="wg-text-btn" id="pin-group">${groupPinned ? "取消固定" : "固定到快捷区"}</button>
        <button type="button" class="wg-text-btn" id="rename-group">重命名</button>
        <button type="button" class="wg-text-btn" data-to-add="1">${mode === "add" ? "取消添加" : "添加窗口"}</button>
        <button type="button" class="wg-text-btn is-danger" id="delete-group">删除组</button>
        ${staleCount > 0 ? `<button type="button" class="wg-text-btn" id="clear-stale">清理失效 (${staleCount})</button>` : ""}
      </div>
    `
        : ""
    }
    ${body}
    ${
      confirm
        ? `
      <div class="wg-confirm" role="dialog" aria-modal="true" aria-labelledby="wg-confirm-msg">
        <div class="wg-confirm-card">
          <p class="wg-confirm-msg" id="wg-confirm-msg">${escapeHtml(confirm.message)}</p>
          <div class="wg-confirm-actions">
            <button type="button" class="wg-text-btn" id="confirm-cancel">取消</button>
            <button type="button" class="wg-primary-btn is-danger" id="confirm-ok">${escapeHtml(confirm.okLabel || "删除")}</button>
          </div>
        </div>
      </div>
    `
        : ""
    }
  `;

  bindEvents();

  document.getElementById("rename-input")?.focus();
  document.getElementById("create-input")?.focus();
  const aliasInput = document.getElementById("alias-input");
  if (aliasInput) {
    aliasInput.focus();
    aliasInput.select();
  }
  if (confirm) {
    document.getElementById("confirm-ok")?.focus();
  }
}

function bindEvents() {
  const q = (sel) => document.querySelector(sel);
  const active = getActiveGroup();

  q("#close")?.addEventListener("click", () => void hub().popup.close());
  q("#new-group")?.addEventListener("click", () => {
    state.mode = "list";
    patch((s) => createGroup(`组 ${s.groups.length + 1}`));
  });
  q("#pin-group")?.addEventListener("click", () => {
    if (!active) return;
    patch(() =>
      isPinned("group", active.id) ? unpin("group", active.id) : pinGroup(active.id),
    );
  });
  q("#rename-group")?.addEventListener("click", () => {
    state.mode = "rename";
    state.draftName = active?.name ?? "";
    render();
  });
  q("#delete-group")?.addEventListener("click", () => {
    if (!active) return;
    state.confirm = {
      message: `删除组「${active.name}」？`,
      okLabel: "删除",
      onYes: () => {
        state.confirm = null;
        state.mode = "list";
        patch(() => deleteGroup(active.id));
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
  if (state.confirm) {
    const onKey = (ev) => {
      if (ev.key === "Escape") {
        state.confirm = null;
        render();
      }
    };
    document.addEventListener("keydown", onKey, { once: true });
  }
  q("#clear-stale")?.addEventListener("click", () => {
    if (active) patch(() => clearStale(active.id));
  });
  document.querySelectorAll("[data-to-add]").forEach((el) => {
    el.addEventListener("click", () => {
      state.mode = state.mode === "add" ? "list" : "add";
      render();
    });
  });

  document.querySelectorAll("[data-group]").forEach((el) => {
    el.addEventListener("click", () => patch(() => setActiveGroup(el.dataset.group)));
  });
  document.querySelectorAll("[data-add]").forEach((el) => {
    el.addEventListener("click", () => {
      const win = state.windows.find((w) => String(w.hwnd) === el.dataset.add);
      state.mode = "list";
      if (active && win) patch(() => addWindow(active.id, win));
    });
  });
  document.querySelectorAll("[data-focus]").forEach((el) => {
    el.addEventListener("click", () => {
      if (!el.dataset.focus) return;
      void (async () => {
        await hub().windows.focus(`hwnd:${el.dataset.focus}`);
        await hub().popup.close();
      })().catch((err) => console.error(err));
    });
  });
  document.querySelectorAll("[data-edit-alias]").forEach((el) => {
    el.addEventListener("click", () => {
      const it = active?.items?.find((x) => x.id === el.dataset.editAlias);
      state.editingAliasId = el.dataset.editAlias;
      state.aliasDraft = it?.alias ?? "";
      render();
    });
  });
  document.querySelectorAll("[data-pin-window]").forEach((el) => {
    el.addEventListener("click", () => {
      const id = el.dataset.pinWindow;
      patch(() => (isPinned("window", id) ? unpin("window", id) : pinWindow(id)));
    });
  });
  document.querySelectorAll("[data-remove-item]").forEach((el) => {
    el.addEventListener("click", () => {
      if (active) patch(() => removeItem(active.id, el.dataset.removeItem));
    });
  });

  q("#rename-save")?.addEventListener("click", () => {
    const input = q("#rename-input");
    state.mode = "list";
    if (active && input) patch(() => renameGroup(active.id, input.value));
  });
  q("#create-save")?.addEventListener("click", () => {
    const input = q("#create-input");
    state.mode = "list";
    if (input) patch(() => createGroup(input.value));
  });
  q("#alias-save")?.addEventListener("click", () => {
    const input = q("#alias-input");
    if (active && input && state.editingAliasId) {
      patch(() => updateAlias(active.id, state.editingAliasId, input.value));
    }
    state.editingAliasId = null;
    state.aliasDraft = "";
  });
}

function applyPreferGroup(groupId) {
  const id = typeof groupId === "string" ? groupId.trim() : "";
  if (!id || !state.store.groups.some((g) => g.id === id)) return false;
  state.mode = "list";
  state.editingAliasId = null;
  if (state.store.activeGroupId === id) {
    render();
    return true;
  }
  state.store = { ...state.store, activeGroupId: id };
  void hub()
    .storage.set("store", state.store)
    .catch((err) => console.error(err));
  render();
  return true;
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

void (async () => {
  const mount = () => {
    if (!document.getElementById("app")) {
      window.setTimeout(mount, 16);
      return;
    }
    void boot();
  };

  async function boot() {
    // Host already applies Mica Alt (settings sidebar). Do not re-apply mica here —
    // a second DWM backdrop looks like an extra frosted overlay.
    const prefer =
      new URLSearchParams(window.location.search).get("preferGroup") ||
      new URLSearchParams(window.location.search).get("preferGroupId");

    // 存储与窗口列表并行，先出壳再填数据
    const storeP = hub()
      .storage.get("store")
      .catch((err) => {
        console.error(err);
        return null;
      });
    try {
      hub().windows.subscribe(onWindows);
    } catch (err) {
      console.error(err);
      void hub()
        .windows.list()
        .then(onWindows)
        .catch(() => onWindows([]));
    }

    try {
      const raw = await storeP;
      state.store = normalize(raw);
    } catch (err) {
      console.error(err);
    }
    if (prefer) applyPreferGroup(prefer);
    render();

    try {
      const listen = window.__TAURI__?.event?.listen;
      if (typeof listen === "function") {
        await listen("plugin-popup-prefer-group", (ev) => {
          const id = typeof ev?.payload === "string" ? ev.payload : null;
          if (id) applyPreferGroup(id);
        });
      }
    } catch (err) {
      console.error(err);
    }
  }

  mount();
})();

void PLUGIN_ID;

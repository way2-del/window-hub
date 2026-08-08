/**
 * Window Groups — shortcuts strip (Host iframe shell).
 * Draws manage chip + store.pins; no hub.shortcuts.setPins.
 */
const hub = () => {
  if (!window.hub) throw new Error("window.hub missing");
  return window.hub;
};

const HOVER_OPEN_MS = 180;

const state = {
  store: { version: 2, activeGroupId: null, groups: [], pins: [] },
  windows: [],
  foregroundId: null,
  popupOpen: false,
  hoverTimer: null,
  hoverGen: 0,
  /** Last preferGroup sent while popup open — avoid repeat opens from DOM rebuild. */
  lastPreferSent: null,
};

function normalize(raw) {
  const parsed = raw || {};
  return {
    version: 2,
    activeGroupId: parsed.activeGroupId ?? null,
    groups: Array.isArray(parsed.groups) ? parsed.groups : [],
    pins: Array.isArray(parsed.pins) ? parsed.pins : [],
  };
}

function findWindowItem(itemId) {
  for (const g of state.store.groups) {
    const it = (g.items || []).find((x) => x.id === itemId);
    if (it) return { group: g, item: it };
  }
  return null;
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
      return !!(bind.exe || bind.titleIncludes || bind.titleRegex);
    });
    if (candidate) {
      used.add(candidate.hwnd);
      return { ...it, lastHwnd: candidate.hwnd };
    }
    return { ...it, lastHwnd: undefined };
  });
}

function escapeHtml(s) {
  return String(s ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function manageIcon() {
  return `<svg class="wg-chip-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true" focusable="false">
    <rect x="3" y="3" width="7" height="7" rx="1" />
    <rect x="14" y="3" width="7" height="7" rx="1" />
    <rect x="3" y="14" width="7" height="7" rx="1" />
    <rect x="14" y="14" width="7" height="7" rx="1" />
  </svg>`;
}

function pinViews() {
  return state.store.pins.map((p) => {
    if (p.kind === "window") {
      const hit = findWindowItem(p.refId);
      const hwnd = hit?.item?.lastHwnd;
      const windowId = hwnd != null ? `hwnd:${hwnd}` : null;
      return {
        id: p.id,
        kind: "window",
        label: hit?.item?.alias || "窗口",
        badge: null,
        /** Window pins only focus — never open manage popup. */
        action: "focus.window",
        windowId,
        stale: windowId == null,
      };
    }
    const g = state.store.groups.find((x) => x.id === p.refId);
    return {
      id: p.id,
      kind: "group",
      refId: p.refId,
      label: g?.name || "组",
      badge: g ? (g.items || []).length || null : null,
      action: "popup.open",
      windowId: null,
      stale: false,
    };
  });
}

function clearHover() {
  if (state.hoverTimer) {
    clearTimeout(state.hoverTimer);
    state.hoverTimer = null;
  }
  state.hoverGen += 1;
}

/** Open or switch group; never toggle-close (used by hover + group click). */
function requestPopupOpen(preferGroupId) {
  const gid =
    typeof preferGroupId === "string" && preferGroupId.trim()
      ? preferGroupId.trim()
      : null;

  void (async () => {
    try {
      if (gid && state.store.groups.some((g) => g.id === gid)) {
        if (state.store.activeGroupId !== gid) {
          state.store = { ...state.store, activeGroupId: gid };
          await hub().storage.set("store", state.store);
        }
      }
      if (state.popupOpen) {
        // Already open: ignore bare manage hover/re-enter; only switch group once.
        if (!gid) return;
        if (state.lastPreferSent === gid) return;
        state.lastPreferSent = gid;
        hub().popup.open({ preferGroupId: gid });
        return;
      }
      state.lastPreferSent = gid;
      hub().popup.open(gid ? { preferGroupId: gid } : {});
    } catch (err) {
      console.error(err);
    }
  })();
}

function schedulePopup(preferGroupId) {
  clearHover();
  const gid =
    typeof preferGroupId === "string" && preferGroupId.trim()
      ? preferGroupId.trim()
      : null;
  const gen = state.hoverGen;
  state.hoverTimer = setTimeout(() => {
    state.hoverTimer = null;
    if (gen !== state.hoverGen) return;
    requestPopupOpen(gid);
  }, HOVER_OPEN_MS);
}

/** Manage click: toggle. Group click: open/switch. */
function onManageClick() {
  clearHover();
  if (state.popupOpen) {
    void hub()
      .popup.close()
      .catch(() => undefined);
    return;
  }
  requestPopupOpen(null);
}

function reportSize() {
  const bar = document.getElementById("bar");
  if (!bar) return;
  const rect = bar.getBoundingClientRect();
  const min = Number.parseFloat(
    getComputedStyle(document.documentElement).getPropertyValue("--wh-bar-h"),
  );
  const floor = Number.isFinite(min) && min > 0 ? min : 28;
  const w = Math.ceil(Math.max(bar.scrollWidth, rect.width, floor));
  try {
    hub().shortcuts.requestSize({ width: w });
  } catch {
    /* noop */
  }
}

/** Sync status-bar height → CSS var; choose vertical align (window-groups = center). */
async function applyBarGeometry() {
  try {
    const b = await hub().shortcuts.getBounds();
    const h = Number(b?.height ?? b?.barHeight) || 28;
    document.documentElement.style.setProperty("--wh-bar-h", `${h}px`);
    document.documentElement.dataset.vAlign = "center";
  } catch {
    document.documentElement.style.setProperty("--wh-bar-h", "28px");
    document.documentElement.dataset.vAlign = "center";
  }
}

function render() {
  const bar = document.getElementById("bar");
  if (!bar) return;
  const pins = pinViews();

  let html = `
    <button type="button" class="wg-chip is-manage${state.popupOpen ? " is-active" : ""}" data-manage="1" aria-label="窗口组" title="窗口组">
      ${manageIcon()}
    </button>
  `;

  pins.forEach((pin, index) => {
    const isFg = !!(state.foregroundId && pin.windowId === state.foregroundId);
    const title =
      pin.kind === "window" && pin.stale
        ? `${pin.label}（已失效）`
        : pin.label;
    html += `<span class="wg-divider" aria-hidden></span>`;
    html += `
      <button type="button" class="wg-chip is-pin${isFg ? " is-fg" : ""}${pin.stale ? " is-stale" : ""}" data-pin="${escapeHtml(pin.id)}" data-kind="${escapeHtml(pin.kind)}" data-ref-id="${escapeHtml(pin.refId || "")}" data-action="${escapeHtml(pin.action)}" data-window-id="${escapeHtml(pin.windowId || "")}" title="${escapeHtml(title)}" ${pin.stale ? "disabled" : ""}>
        ${isFg ? `<span class="wg-active-dot" aria-hidden></span>` : ""}
        <span class="wg-chip-label">${escapeHtml(pin.label)}</span>
        ${pin.badge != null ? `<span class="wg-badge">${escapeHtml(pin.badge)}</span>` : ""}
      </button>
    `;
    void index;
  });

  bar.innerHTML = html;
  bind();
  requestAnimationFrame(() => {
    reportSize();
    requestAnimationFrame(reportSize);
  });
}

function getActiveGroup() {
  const { groups, activeGroupId } = state.store;
  return groups.find((g) => g.id === activeGroupId) ?? groups[0] ?? null;
}

function bind() {
  const manage = document.querySelector("[data-manage]");
  manage?.addEventListener("pointerenter", () => schedulePopup(null));
  manage?.addEventListener("pointerleave", clearHover);
  manage?.addEventListener("click", onManageClick);

  document.querySelectorAll("[data-pin]").forEach((el) => {
    const kind = el.getAttribute("data-kind");
    const windowId = el.getAttribute("data-window-id");
    const refId = el.getAttribute("data-ref-id");
    if (kind === "group") {
      el.addEventListener("pointerenter", () => schedulePopup(refId));
      el.addEventListener("pointerleave", clearHover);
    }
    el.addEventListener("click", () => {
      clearHover();
      if (kind === "window") {
        if (!windowId) return;
        void hub()
          .windows.focus(windowId)
          .catch((err) => console.error(err));
        return;
      }
      if (kind === "group") {
        requestPopupOpen(refId);
      }
    });
  });
}

async function loadStore() {
  try {
    const raw = await hub().storage.get("store");
    state.store = normalize(raw);
  } catch (err) {
    console.error(err);
  }
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

async function boot() {
  await applyBarGeometry();
  await loadStore();
  render();
  try {
    hub().windows.subscribe(onWindows);
  } catch (err) {
    console.error(err);
    const list = await hub().windows.list().catch(() => []);
    onWindows(list);
  }
  try {
    hub().foreground.subscribe((fg) => {
      if (fg && fg.isSelf) return;
      const id = fg?.windowId ?? null;
      if (id === state.foregroundId) return;
      state.foregroundId = id;
      // Update green dots without wiping DOM (avoids pointerenter → popup spam).
      document.querySelectorAll("[data-pin][data-kind=window]").forEach((el) => {
        const wid = el.getAttribute("data-window-id");
        const on = !!(id && wid && wid === id);
        el.classList.toggle("is-fg", on);
        const dot = el.querySelector(".wg-active-dot");
        if (on && !dot) {
          el.insertAdjacentHTML(
            "afterbegin",
            '<span class="wg-active-dot" aria-hidden></span>',
          );
        } else if (!on && dot) {
          dot.remove();
        }
      });
    });
  } catch {
    /* noop */
  }
  window.addEventListener("wh-shortcuts-evt", (ev) => {
    const d = ev && ev.detail;
    if (!d) return;
    if (d.type === "popup-opened") {
      state.popupOpen = true;
      document.querySelector("[data-manage]")?.classList.add("is-active");
      return;
    }
    if (d.type === "popup-closed") {
      state.popupOpen = false;
      state.lastPreferSent = null;
      document.querySelector("[data-manage]")?.classList.remove("is-active");
    }
  });
  window.addEventListener("wh-shortcuts-refresh", () => {
    void loadStore().then(render);
  });
  window.addEventListener("resize", reportSize);
  const bar = document.getElementById("bar");
  if (bar && typeof ResizeObserver !== "undefined") {
    const ro = new ResizeObserver(() => reportSize());
    ro.observe(bar);
  }
}

void boot();

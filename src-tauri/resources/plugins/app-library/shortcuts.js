/**
 * App Library — shortcuts strip (Host iframe shell).
 * manage=custom：条内自画 2×2 管理钮 + pinned apps。
 */
const hub = () => {
  if (!window.hub) throw new Error("window.hub missing");
  return window.hub;
};

const HOVER_OPEN_MS = 180;

const state = {
  store: { version: 1, apps: [] },
  windows: [],
  foregroundId: null,
  popupOpen: false,
  hoverTimer: null,
  hoverGen: 0,
};

function uid() {
  return crypto.randomUUID
    ? crypto.randomUUID()
    : `a-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function colorFor(name) {
  const colors = [
    "#3dd6c6",
    "#6ea8fe",
    "#f0b429",
    "#f07178",
    "#7bd88f",
    "#c792ea",
  ];
  const s = String(name || "");
  let h = 0;
  for (let i = 0; i < s.length; i += 1) h = (h * 31 + s.charCodeAt(i)) >>> 0;
  return colors[h % colors.length];
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

function escapeHtml(s) {
  return String(s ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function manageIcon() {
  return `<svg class="al-chip-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true" focusable="false">
    <rect x="3" y="3" width="7" height="7" rx="1.5" />
    <rect x="14" y="3" width="7" height="7" rx="1.5" />
    <rect x="3" y="14" width="7" height="7" rx="1.5" />
    <rect x="14" y="14" width="7" height="7" rx="1.5" />
  </svg>`;
}

function pinnedApps() {
  return state.store.apps
    .filter((a) => a.pinned)
    .sort((a, b) => (a.order || 0) - (b.order || 0) || a.name.localeCompare(b.name, "zh"));
}

function clearHover() {
  if (state.hoverTimer) {
    clearTimeout(state.hoverTimer);
    state.hoverTimer = null;
  }
  state.hoverGen += 1;
}

function requestPopupOpen() {
  void (async () => {
    try {
      if (state.popupOpen) return;
      hub().popup.open({});
    } catch (err) {
      console.error(err);
    }
  })();
}

function schedulePopup() {
  clearHover();
  const gen = state.hoverGen;
  state.hoverTimer = setTimeout(() => {
    state.hoverTimer = null;
    if (gen !== state.hoverGen) return;
    requestPopupOpen();
  }, HOVER_OPEN_MS);
}

function onManageClick() {
  clearHover();
  if (state.popupOpen) {
    void hub()
      .popup.close()
      .catch(() => undefined);
    return;
  }
  requestPopupOpen();
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
  const pins = pinnedApps();

  let html = `
    <button type="button" class="al-chip is-manage${state.popupOpen ? " is-active" : ""}" data-manage="1" aria-label="应用库" title="应用库">
      ${manageIcon()}
    </button>
  `;

  pins.forEach((app) => {
    const windowId = app.lastHwnd != null ? `hwnd:${app.lastHwnd}` : null;
    const isFg = !!(state.foregroundId && windowId === state.foregroundId);
    const stale = windowId == null;
    html += `<span class="al-divider" aria-hidden></span>`;
    html += `
      <button type="button" class="al-chip is-pin${isFg ? " is-fg" : ""}${stale ? " is-stale" : ""}" data-app="${escapeHtml(app.id)}" data-window-id="${escapeHtml(windowId || "")}" title="${escapeHtml(app.name)}${stale ? "（未运行）" : ""}" ${stale ? "disabled" : ""}>
        ${isFg ? `<span class="al-active-dot" aria-hidden></span>` : ""}
        <span class="al-chip-dot" style="background:${escapeHtml(app.color)}" aria-hidden></span>
        <span class="al-chip-label">${escapeHtml(app.name)}</span>
      </button>
    `;
  });

  bar.innerHTML = html;
  bind();
  requestAnimationFrame(() => {
    reportSize();
    requestAnimationFrame(reportSize);
  });
}

function bind() {
  const manage = document.querySelector("[data-manage]");
  manage?.addEventListener("pointerenter", schedulePopup);
  manage?.addEventListener("pointerleave", clearHover);
  manage?.addEventListener("click", onManageClick);

  document.querySelectorAll("[data-app]").forEach((el) => {
    el.addEventListener("click", () => {
      clearHover();
      const windowId = el.getAttribute("data-window-id");
      if (!windowId) return;
      void hub()
        .windows.focus(windowId)
        .catch((err) => console.error(err));
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
      document.querySelectorAll("[data-app]").forEach((el) => {
        const wid = el.getAttribute("data-window-id");
        const on = !!(id && wid && wid === id);
        el.classList.toggle("is-fg", on);
        const dot = el.querySelector(".al-active-dot");
        if (on && !dot) {
          el.insertAdjacentHTML(
            "afterbegin",
            '<span class="al-active-dot" aria-hidden></span>',
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

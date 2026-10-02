/**
 * Page Watch — shortcuts strip: chrome-ink icon + popup.open + watch re-register.
 * manage=custom：必须自画入口并调用 hub.popup.open（Host 不会代开）。
 * 通知 onAction / hub.notify 只挂在快捷区（常驻）；弹窗不得再发岛通知。
 */
(function () {
  const STORE_KEY = "watchItems";
  const PREFS_KEY = "prefs";
  const DEFAULT_PREFS = {
    notifyTitle: "{title}",
    notifyBody: "{title}更新了",
  };
  /** Shortcuts strip short label (full name stays on title / plugin.json). */
  const SHORT_NAME = "监测";

  const state = {
    popupOpen: false,
  };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function chipIcon() {
    /* Square glyph centered in 24 viewBox (like world-clock circle), not a wide browser frame. */
    return (
      '<svg class="pw-chip-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true" focusable="false">' +
      '<rect x="5" y="5" width="14" height="14" rx="3.5" stroke="currentColor" stroke-width="1.8"/>' +
      '<path d="M5 9.25h14" stroke="currentColor" stroke-width="1.8"/>' +
      '<path d="M9 14.2c.85-1.55 1.9-2.35 3-2.35s2.15.8 3 2.35" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"/>' +
      '<circle class="pw-fill" cx="12" cy="14.2" r="1.15"/>' +
      "</svg>"
    );
  }

  /** Cache last reported width — Host iframe resize must not re-measure client box. */
  let lastWidth = 0;

  function reportSize() {
    const bar = document.getElementById("bar");
    if (!bar || !window.hub || !window.hub.shortcuts || !window.hub.shortcuts.requestSize) {
      return;
    }
    // Intrinsic content only. Math.max(..., getBoundingClientRect()) tracks the Host
    // iframe after requestSize and can oscillate 1px with a window.resize listener.
    const width = Math.ceil(Math.max(bar.scrollWidth, 28));
    if (width <= 0 || width === lastWidth) return;
    try {
      window.hub.shortcuts.requestSize({ width: width });
      lastWidth = width;
    } catch (_) {}
  }

  function paint() {
    const bar = document.getElementById("bar");
    if (!bar) return;
    bar.innerHTML =
      '<button type="button" class="pw-chip is-manage" id="pw-open" title="网页监测" aria-label="网页监测">' +
      chipIcon() +
      '<span class="pw-chip-label">' +
      SHORT_NAME +
      "</span>" +
      "</button>";
    const btn = document.getElementById("pw-open");
    if (!btn) return;

    btn.addEventListener("click", (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      void togglePopup();
    });
    reportSize();
  }

  function openPopup() {
    const h = hub();
    if (!h.popup || !h.popup.open) return;
    try {
      h.popup.open({});
    } catch (_) {}
  }

  function togglePopup() {
    const h = hub();
    if (!h.popup) return;
    if (state.popupOpen) {
      void h.popup.close().catch(() => undefined);
      return;
    }
    openPopup();
  }

  async function loadPrefs() {
    try {
      const raw = await hub().storage.get(PREFS_KEY);
      if (raw && typeof raw === "object") {
        return {
          notifyTitle:
            typeof raw.notifyTitle === "string" && raw.notifyTitle.trim()
              ? raw.notifyTitle
              : DEFAULT_PREFS.notifyTitle,
          notifyBody:
            typeof raw.notifyBody === "string" && raw.notifyBody.trim()
              ? raw.notifyBody
              : DEFAULT_PREFS.notifyBody,
        };
      }
    } catch (_) {}
    return Object.assign({}, DEFAULT_PREFS);
  }

  function applyNotifyTemplate(tpl, vars) {
    return String(tpl == null ? "" : tpl).replace(
      /\{(title|name|text|url|selector)\}/g,
      function (_, key) {
        if (key === "name") return vars.title != null ? String(vars.title) : "";
        return vars[key] != null ? String(vars[key]) : "";
      },
    );
  }

  function formatNotify(prefs, vars) {
    const title =
      applyNotifyTemplate(prefs.notifyTitle, vars).trim() || vars.title || "网页变化";
    const body =
      applyNotifyTemplate(prefs.notifyBody, vars).trim() ||
      String(vars.text || "").slice(0, 160) ||
      "监测内容已更新";
    return { title: title.slice(0, 80), body: body.slice(0, 200) };
  }

  async function syncWatches() {
    const h = hub();
    if (!h.storage || !h.webview || !h.webview.watch) return;
    const raw = await h.storage.get(STORE_KEY);
    const items = Array.isArray(raw) ? raw : [];
    for (const it of items) {
      if (!it || !it.enabled) continue;
      try {
        await h.webview.watch.start({
          id: it.id,
          url: it.url,
          selector: it.selector,
          intervalMs: Math.max(30000, Number(it.intervalMs) || 60000),
          title: it.title || "网页监测",
        });
      } catch (_) {}
    }
  }

  /**
   * Dedup by Host change stamp (atMs), not by mute window.
   * Same watchId+atMs = same Host event (re-delivered) → skip.
   * New atMs = real new scan → notify immediately.
   * changeId is also stamped into hub.notify data for debugging.
   */
  const seenChangeIds = new Set();
  const SEEN_MAX = 40;

  function changeIdOf(ev, watchId) {
    const atMs = Number(ev && ev.atMs);
    if (Number.isFinite(atMs) && atMs > 0) {
      return String(watchId || "") + "@" + atMs;
    }
    // Fallback if Host omitted atMs: content fingerprint (not a time mute).
    const textKey = String((ev && ev.text) || "")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 400);
    return String(watchId || "") + "#txt:" + textKey;
  }

  async function claimChangeId(changeId) {
    if (!changeId) return true;
    if (seenChangeIds.has(changeId)) return false;
    const key = "__notifyChangeId:" + changeId;
    const token =
      String(Date.now()) + ":" + Math.random().toString(36).slice(2, 9);
    try {
      const prev = await hub().storage.get(key);
      if (prev) return false;
      await hub().storage.set(key, { token: token, at: Date.now() });
      await new Promise((r) => setTimeout(r, 30));
      const cur = await hub().storage.get(key);
      if (!cur || typeof cur !== "object" || cur.token !== token) return false;
      seenChangeIds.add(changeId);
      if (seenChangeIds.size > SEEN_MAX) {
        const first = seenChangeIds.values().next().value;
        seenChangeIds.delete(first);
      }
      return true;
    } catch (_) {
      seenChangeIds.add(changeId);
      return true;
    }
  }

  async function notifyOnce(ev) {
    const watchId = ev && (ev.watchId || ev.id);
    const sampleText = ((ev && ev.text) || "").slice(0, 800);
    const atMs = Number(ev && ev.atMs) || Date.now();
    const changeId = changeIdOf(ev, watchId);
    // WebView2 XSLT deprecation chrome is not site content.
    if (/This site uses XSLT/i.test(sampleText) && sampleText.length < 400) {
      return;
    }
    if (!(await claimChangeId(changeId))) {
      try {
        console.info("[page-watch] skip duplicate changeId", changeId);
      } catch (_) {}
      return;
    }

    let itemTitle = (ev && ev.title) || "网页变化";
    let url = (ev && ev.url) || "";
    let selector = "";
    try {
      const raw = await hub().storage.get(STORE_KEY);
      const items = Array.isArray(raw) ? raw : [];
      const it = items.find((x) => x && x.id === watchId);
      if (it) {
        itemTitle = it.title || itemTitle;
        url = it.url || url;
        selector = it.selector || "";
      }
    } catch (_) {}

    const prefs = await loadPrefs();
    const formatted = formatNotify(prefs, {
      title: itemTitle,
      text: sampleText.slice(0, 200),
      url: url,
      selector: selector,
    });

    const notifyData = {
      watchId: watchId,
      url: url,
      title: itemTitle,
      changeId: changeId,
      atMs: atMs,
    };

    try {
      console.info("[page-watch] notify", changeId);
    } catch (_) {}

    try {
      // ttlMs: auto-clear so unread banners do not stack forever (Host default is sticky).
      await hub().notify({
        title: formatted.title,
        body: formatted.body,
        urgency: "active",
        ttlMs: 8000,
        data: notifyData,
        defaultActionId: url ? "open" : undefined,
        actions: url
          ? [
              {
                id: "open",
                slot: "end",
                label: "打开",
                background: "rgba(255,255,255,0.2)",
                data: notifyData,
              },
            ]
          : undefined,
      });
    } catch (_) {}
  }

  async function handleNotifyAction(ev) {
    if (!ev || ev.actionId !== "open") return;
    const data = ev.data || {};
    let url = data.url ? String(data.url) : "";
    let title = data.title ? String(data.title) : "网页监测";
    if (!url && data.watchId) {
      try {
        const raw = await hub().storage.get(STORE_KEY);
        const items = Array.isArray(raw) ? raw : [];
        const it = items.find((x) => x && x.id === data.watchId);
        if (it) {
          url = it.url;
          title = it.title || title;
        }
      } catch (_) {}
    }
    if (!url || !hub().webview || !hub().webview.open) return;
    try {
      await hub().webview.open({ url: url, title: title });
    } catch (_) {}
  }

  let notifyBound = false;

  function bindNotify() {
    if (notifyBound) return;
    const h = hub();
    if (h.webview && h.webview.onChanged) {
      h.webview.onChanged((ev) => {
        void notifyOnce(ev);
      });
      notifyBound = true;
    }
    if (h.notify && typeof h.notify.onAction === "function") {
      h.notify.onAction((ev) => {
        void handleNotifyAction(ev);
      });
    }
  }

  async function mergePickIntoDraft(ev) {
    if (!ev || ev.cancelled) return;
    const selector = String(ev.selector || "").trim();
    if (!selector) return;
    const h = hub();
    let draft = {};
    try {
      const raw = await h.storage.get("formDraft");
      if (raw && typeof raw === "object") draft = raw;
    } catch (_) {}
    draft.version = 1;
    draft.formOpen = true;
    draft.selector = selector;
    draft.preview = ev.textPreview || draft.preview || "";
    if (ev.sessionId) draft.sessionId = ev.sessionId;
    draft.hint = "已选定元素，可保存监测任务";
    draft.hintError = false;
    draft.updatedAt = Date.now();
    try {
      await h.storage.set("formDraft", draft);
    } catch (_) {}
  }

  function bindPick() {
    const h = hub();
    if (!h.webview || !h.webview.onPick) return;
    h.webview.onPick((ev) => {
      void mergePickIntoDraft(ev);
    });
  }

  async function appendScanLog(ev) {
    if (!ev || !ev.watchId) return;
    const SCAN_LOG_KEY = "scanLogs";
    const SCAN_LOG_MAX = 200;
    let logs = [];
    try {
      const raw = await hub().storage.get(SCAN_LOG_KEY);
      logs = Array.isArray(raw) ? raw : [];
    } catch (_) {}
    logs.unshift({
      id: "s" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6),
      watchId: String(ev.watchId),
      title: ev.title || "",
      atMs: Number(ev.atMs) || Date.now(),
      ok: ev.ok !== false,
      error: ev.error ? String(ev.error) : "",
      text: String(ev.text || ""),
      prevText: String(ev.prevText || ""),
      changed: !!ev.changed,
      baseline: !!ev.baseline,
    });
    if (logs.length > SCAN_LOG_MAX) logs.length = SCAN_LOG_MAX;
    try {
      await hub().storage.set(SCAN_LOG_KEY, logs);
    } catch (_) {}
  }

  function bindScanned() {
    const h = hub();
    if (!h.webview || !h.webview.onScanned) return;
    h.webview.onScanned((ev) => {
      void appendScanLog(ev);
    });
  }

  function bindBarH() {
    try {
      const b = hub().shortcuts && hub().shortcuts.getBounds && hub().shortcuts.getBounds();
      if (b && typeof b.then === "function") {
        b.then((bounds) => {
          const h = (bounds && (bounds.height || bounds.barHeight)) || 28;
          document.documentElement.style.setProperty("--wh-bar-h", h + "px");
          reportSize();
        }).catch(() => {});
      }
    } catch (_) {}
  }

  function setPopupOpen(open) {
    state.popupOpen = !!open;
    const btn = document.getElementById("pw-open");
    if (btn) btn.classList.toggle("is-active", state.popupOpen);
  }

  function boot() {
    paint();
    bindBarH();
    void syncWatches();
    bindNotify();
    bindPick();
    bindScanned();
    // Re-measure after paint / font inject — never on window.resize (Host width feedback).
    requestAnimationFrame(function () {
      reportSize();
    });
    if (document.fonts && document.fonts.ready) {
      document.fonts.ready.then(function () {
        reportSize();
      }).catch(function () {});
    }
    window.addEventListener("wh-shortcuts-evt", (ev) => {
      const d = ev && ev.detail;
      if (!d) return;
      if (d.type === "popup-opened") {
        setPopupOpen(true);
        return;
      }
      if (d.type === "popup-closed") {
        setPopupOpen(false);
      }
    });
    window.addEventListener("wh-shortcuts-refresh", () => {
      void syncWatches();
      reportSize();
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }
})();

/**
 * 成语 — 快捷区自画 manage + chip
 * manage → 历史弹窗；chip 文案=word，悬停=拼音+释义；点击/定时切换
 * 拼音：同目录 pinyin-pro.min.js（Host 注入；默认带声调符号）
 */
(function () {
  const API = "http://43.139.23.203:8765/api/random";
  const CACHE_KEY = "cache";
  const HISTORY_KEY = "history";
  const HISTORY_MAX = 200;
  const HOVER_OPEN_MS = 180;

  const state = {
    popupOpen: false,
    hoverTimer: null,
    hoverGen: 0,
    /** Chip is under pointer — refresh tip after paint/click. */
    chipHover: false,
  };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  /** 带声调：chéng yǔ；优先 pinyin-pro，回退 pinyinlite（无调） */
  function toPinyin(word) {
    const text = String(word || "").trim();
    if (!text) return "";
    try {
      if (
        typeof pinyinPro !== "undefined" &&
        pinyinPro &&
        typeof pinyinPro.pinyin === "function"
      ) {
        return String(
          pinyinPro.pinyin(text, {
            toneType: "symbol",
            type: "string",
            separator: " ",
          }) || "",
        ).trim();
      }
      if (typeof pinyinlite === "function") {
        const rows = pinyinlite(text);
        if (!Array.isArray(rows)) return "";
        return rows
          .map(function (py) {
            return Array.isArray(py) && py[0] ? String(py[0]) : "";
          })
          .filter(Boolean)
          .join(" ");
      }
    } catch (_) {}
    return "";
  }

  function emptyItem() {
    return {
      id: 0,
      word: "…",
      meaning: "正在加载成语",
      pinyin: "",
      placeholder: true,
    };
  }

  function parseItem(data) {
    if (!data || typeof data !== "object") return emptyItem();
    const word = String(data.word || "").trim();
    const meaning = String(data.meaning || "").trim();
    if (!word) return emptyItem();
    const computed = toPinyin(word);
    const pinyin = computed || String(data.pinyin || "").trim();
    return {
      id: Number(data.id) || 0,
      word: word,
      meaning: meaning || word,
      pinyin: pinyin,
      placeholder: false,
    };
  }

  function manageIcon() {
    return `<svg class="idiom-chip-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true" focusable="false">
    <rect x="3" y="3" width="7" height="7" rx="1" />
    <rect x="14" y="3" width="7" height="7" rx="1" />
    <rect x="3" y="14" width="7" height="7" rx="1" />
    <rect x="14" y="14" width="7" height="7" rx="1" />
  </svg>`;
  }

  async function loadSettings() {
    const h = hub();
    const all = (await h.settings.getAll().catch(function () {
      return {};
    })) || {};
    var sec = Number(all.refreshSeconds);
    if (!Number.isFinite(sec) || sec < 0) sec = 60;
    return { refreshSeconds: Math.floor(sec) };
  }

  async function loadHistory() {
    const raw = await hub()
      .storage.get(HISTORY_KEY)
      .catch(function () {
        return null;
      });
    const items = Array.isArray(raw && raw.items) ? raw.items : [];
    return { version: 1, items: items };
  }

  async function pushHistory(item) {
    if (!item || item.placeholder || !item.word) return;
    const store = await loadHistory();
    const key = item.id ? "id:" + item.id : "w:" + item.word;
    const next = store.items.filter(function (x) {
      const k = x.id ? "id:" + x.id : "w:" + x.word;
      return k !== key;
    });
    next.unshift({
      id: item.id || 0,
      word: item.word,
      meaning: item.meaning || "",
      pinyin: toPinyin(item.word) || item.pinyin || "",
      seenAt: Date.now(),
    });
    while (next.length > HISTORY_MAX) next.pop();
    await hub()
      .storage.set(HISTORY_KEY, { version: 1, items: next })
      .catch(function () {});
  }

  function chipEl() {
    return document.getElementById("chip");
  }

  function paint(item) {
    const el = chipEl();
    if (!el) return;
    const payload = item || emptyItem();
    el.textContent = payload.word;
    el.removeAttribute("title");
    const lines = [payload.pinyin, payload.meaning].filter(Boolean);
    el.dataset.tipPinyin = payload.pinyin || "";
    el.dataset.tipMeaning = payload.meaning || "";
    el.setAttribute(
      "aria-label",
      payload.word +
        (payload.pinyin ? " " + payload.pinyin : "") +
        (payload.meaning ? "：" + payload.meaning : ""),
    );
    el._idiomTipLines = lines;
    requestSize();
    if (state.chipHover) showChipTip(el);
  }

  function showChipTip(el) {
    const h = hub();
    if (!h.shortcuts || !h.shortcuts.showTip) return;
    const lines =
      (el && el._idiomTipLines) ||
      [el && el.dataset.tipPinyin, el && el.dataset.tipMeaning].filter(Boolean);
    if (!lines || !lines.length) return;
    const r = el.getBoundingClientRect();
    h.shortcuts.showTip({
      lines: lines,
      x: r.left + r.width / 2,
      y: r.bottom,
    });
  }

  function hideChipTip() {
    try {
      hub().shortcuts.hideTip();
    } catch (_) {}
  }

  function requestSize() {
    const h = hub();
    const bar = document.getElementById("bar");
    if (!bar || !h.shortcuts || !h.shortcuts.requestSize) return;
    const w = Math.ceil(
      bar.getBoundingClientRect().width || bar.scrollWidth || 48,
    );
    try {
      h.shortcuts.requestSize({ width: Math.max(56, Math.min(220, w + 2)) });
    } catch (_) {}
  }

  function clearHover() {
    if (state.hoverTimer) {
      clearTimeout(state.hoverTimer);
      state.hoverTimer = null;
    }
    state.hoverGen += 1;
  }

  function requestPopupOpen() {
    void (async function () {
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
    state.hoverTimer = setTimeout(function () {
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
        .catch(function () {});
      return;
    }
    requestPopupOpen();
  }

  async function refresh() {
    const h = hub();
    const el = chipEl();
    if (el) el.classList.add("is-busy");
    try {
      const res = await h.fetch(API, { method: "GET", timeoutMs: 12000 });
      if (!res || !res.ok)
        throw new Error("成语请求失败 HTTP " + (res && res.status));
      var data;
      try {
        data = JSON.parse(res.body || "{}");
      } catch (_) {
        throw new Error("成语接口返回非 JSON");
      }
      const item = parseItem(data);
      if (item.placeholder) throw new Error("成语数据为空");
      await h.storage.set(CACHE_KEY, { item: item, savedAt: Date.now() });
      await pushHistory(item);
      paint(item);
      return item;
    } finally {
      if (el) el.classList.remove("is-busy");
    }
  }

  async function bootFromCache() {
    const raw = await hub()
      .storage.get(CACHE_KEY)
      .catch(function () {
        return null;
      });
    if (raw && raw.item && raw.item.word) {
      const item = parseItem(raw.item);
      paint(item);
      return item;
    }
    paint(emptyItem());
    return null;
  }

  var inflight = null;
  async function tick() {
    if (inflight) return inflight;
    inflight = (async function () {
      try {
        await refresh();
      } catch (err) {
        console.warn("[idiom] refresh", err);
        const cached = await bootFromCache();
        if (!cached) paint(emptyItem());
      } finally {
        inflight = null;
      }
    })();
    return inflight;
  }

  function bindManage() {
    const manage = document.querySelector("[data-manage]");
    manage?.addEventListener("pointerenter", schedulePopup);
    manage?.addEventListener("pointerleave", clearHover);
    manage?.addEventListener("click", onManageClick);
  }

  async function boot() {
    const h = hub();
    const bar = document.getElementById("bar");
    if (bar && !document.getElementById("manage")) {
      bar.insertAdjacentHTML(
        "afterbegin",
        `<button type="button" id="manage" class="idiom-chip is-manage" data-manage="1" aria-label="成语历史" title="成语历史">${manageIcon()}</button>`,
      );
    }
    bindManage();

    paint(emptyItem());
    await bootFromCache();
    await tick();

    var timer = null;
    async function schedule() {
      if (timer) {
        clearInterval(timer);
        timer = null;
      }
      const s = await loadSettings();
      if (s.refreshSeconds <= 0) return;
      timer = setInterval(function () {
        void tick();
      }, s.refreshSeconds * 1000);
    }
    await schedule();

    if (h.settings && h.settings.subscribe) {
      h.settings.subscribe(function () {
        void schedule();
      });
    }

    const el = chipEl();
    if (el) {
      el.addEventListener("click", function (e) {
        e.preventDefault();
        e.stopPropagation();
        // Keep tip visible while hovered; paint() refreshes content after fetch.
        void tick();
      });
      el.addEventListener("pointerenter", function () {
        state.chipHover = true;
        showChipTip(el);
      });
      el.addEventListener("pointerleave", function () {
        state.chipHover = false;
        hideChipTip();
      });
    }

    window.addEventListener("wh-shortcuts-evt", function (ev) {
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

    window.addEventListener("wh-shortcuts-refresh", function () {
      requestSize();
    });
    window.setTimeout(requestSize, 50);
    window.setTimeout(requestSize, 200);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

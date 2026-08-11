/**
 * 背成语 — 快捷区：缓存/拉词 → 左侧 chip 显示当前词；点击在芯片下方开弹窗。
 * 订阅 storage：弹窗「换一条」后 chip 立刻对齐。
 */
(function () {
  const API = "http://43.139.23.203:8765/api/random";
  const CACHE_KEY = "cache";
  let timer = 0;

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function normalize(raw) {
    if (!raw || typeof raw !== "object") return null;
    const word = String(raw.word || "").trim();
    const meaning = String(raw.meaning || "").trim();
    if (!word) return null;
    const id = raw.id != null ? Number(raw.id) : null;
    return {
      id: Number.isFinite(id) ? id : null,
      word: word,
      meaning: meaning || "（暂无释义）",
      fetchedAt: typeof raw.fetchedAt === "number" ? raw.fetchedAt : Date.now(),
    };
  }

  function paintChip(idiom) {
    const bar = document.getElementById("bar");
    const label = bar && bar.querySelector(".idiom-chip-label");
    if (!label) return;
    const word = idiom && idiom.word ? idiom.word : "成语";
    label.textContent = word;
    if (bar) {
      bar.title = idiom && idiom.meaning ? idiom.meaning : "下拉背成语";
    }
    reportWidth();
  }

  function reportWidth() {
    const h = hub();
    const bar = document.getElementById("bar");
    if (!bar || !h.shortcuts || !h.shortcuts.requestSize) return;
    const w = Math.ceil(
      Math.max(bar.scrollWidth, bar.getBoundingClientRect().width, 28),
    );
    void h.shortcuts.requestSize({ width: w });
  }

  function openPopup() {
    const h = hub();
    if (!h.popup || !h.popup.open) return;
    h.popup.open({});
  }

  async function fetchAndCache() {
    const h = hub();
    const res = await h.fetch(API, { method: "GET", timeoutMs: 12000 });
    if (!res || !res.ok) throw new Error("fetch failed");
    const data = JSON.parse(res.body);
    const idiom = normalize({ ...data, fetchedAt: Date.now() });
    if (!idiom) throw new Error("empty idiom");
    await h.storage.set(CACHE_KEY, idiom);
    paintChip(idiom);
    return idiom;
  }

  async function ensureChip() {
    const h = hub();
    try {
      const cached = normalize(await h.storage.get(CACHE_KEY));
      if (cached) {
        paintChip(cached);
        return cached;
      }
    } catch (_) {}
    return fetchAndCache();
  }

  /** @returns {Promise<number>} seconds; 0 = manual only */
  async function readRefreshSeconds() {
    try {
      const s = await hub().settings.getAll();
      if (s && s.autoRefreshSeconds != null) {
        const n = Number(s.autoRefreshSeconds);
        return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
      }
      const mins = Number(s && s.autoRefreshMinutes);
      return Number.isFinite(mins) && mins > 0 ? Math.floor(mins * 60) : 0;
    } catch (_) {
      return 0;
    }
  }

  function armTimer(seconds) {
    window.clearInterval(timer);
    timer = 0;
    if (seconds <= 0) return;
    timer = window.setInterval(function () {
      void fetchAndCache().catch(function () {});
    }, seconds * 1000);
  }

  function onStorageChanged(ev) {
    if (!ev || ev.key !== CACHE_KEY) return;
    if (ev.removed) {
      paintChip(null);
      return;
    }
    const idiom = normalize(ev.value);
    paintChip(idiom);
  }

  function bindChip() {
    const bar = document.getElementById("bar");
    if (!bar) return;
    bar.addEventListener("click", function (e) {
      e.preventDefault();
      e.stopPropagation();
      openPopup();
    });
    bar.addEventListener("keydown", function (e) {
      if (e.key !== "Enter" && e.key !== " ") return;
      e.preventDefault();
      openPopup();
    });
  }

  async function boot() {
    const h = hub();
    try {
      const b = await h.shortcuts.getBounds();
      if (b && b.height) {
        document.documentElement.style.setProperty("--wh-bar-h", b.height + "px");
      }
    } catch (_) {}

    bindChip();
    paintChip(null);

    try {
      await ensureChip();
    } catch (_) {
      paintChip(null);
    }

    armTimer(await readRefreshSeconds());

    if (h.storage && h.storage.subscribe) {
      h.storage.subscribe(onStorageChanged);
    }
    if (h.settings && h.settings.subscribe) {
      h.settings.subscribe(function () {
        void readRefreshSeconds().then(armTimer);
      });
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

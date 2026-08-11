/**
 * 背成语 — 弹窗：读缓存 + 揭晓释义 + 换一条（挂 #app.wg-shell）
 */
(function () {
  const API = "http://43.139.23.203:8765/api/random";
  const CACHE_KEY = "cache";

  const state = {
    idiom: null,
    revealed: false,
    showMeaningAlways: false,
    loading: false,
    error: "",
  };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function escapeHtml(s) {
    return String(s ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;");
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

  async function readShowMeaningAlways() {
    try {
      const s = await hub().settings.getAll();
      return !!(s && s.showMeaningAlways);
    } catch (_) {
      return false;
    }
  }

  async function applyRevealPref() {
    state.showMeaningAlways = await readShowMeaningAlways();
    if (state.showMeaningAlways) state.revealed = true;
  }

  async function loadCache() {
    try {
      const raw = await hub().storage.get(CACHE_KEY);
      state.idiom = normalize(raw);
    } catch (_) {
      state.idiom = null;
    }
  }

  async function saveCache(idiom) {
    await hub().storage.set(CACHE_KEY, idiom);
  }

  function applyIdiom(idiom, resetReveal) {
    state.idiom = idiom;
    state.error = "";
    if (resetReveal) {
      state.revealed = !!state.showMeaningAlways;
    } else if (state.showMeaningAlways) {
      state.revealed = true;
    }
    render();
  }

  async function fetchIdiom() {
    const h = hub();
    const res = await h.fetch(API, { method: "GET", timeoutMs: 12000 });
    if (!res || !res.ok) {
      throw new Error("接口请求失败" + (res ? "（" + res.status + "）" : ""));
    }
    let data;
    try {
      data = JSON.parse(res.body);
    } catch (_) {
      throw new Error("接口返回不是 JSON");
    }
    const idiom = normalize({ ...data, fetchedAt: Date.now() });
    if (!idiom) throw new Error("接口未返回成语");
    return idiom;
  }

  async function refresh() {
    if (state.loading) return;
    state.loading = true;
    state.error = "";
    render();
    try {
      const idiom = await fetchIdiom();
      state.idiom = idiom;
      state.revealed = !!state.showMeaningAlways;
      await saveCache(idiom);
    } catch (e) {
      state.error = String(e && e.message ? e.message : e);
    } finally {
      state.loading = false;
      render();
    }
  }

  function render() {
    const root = document.getElementById("app");
    if (!root) return;
    root.className = "wg-shell";
    const idiom = state.idiom;
    const meaningText = !idiom
      ? "打开后会拉取一条成语"
      : state.revealed
        ? idiom.meaning
        : "先想一想释义，再点「揭晓」";

    root.innerHTML =
      '<header class="idiom-header">' +
      "<div>" +
      '<div class="idiom-kicker">Idiom</div>' +
      '<div class="idiom-title">背成语</div>' +
      '<div class="idiom-meta">' +
      (idiom && idiom.id != null ? "#" + escapeHtml(String(idiom.id)) : "随机词条") +
      (state.loading ? " · 加载中…" : "") +
      "</div></div>" +
      '<button type="button" class="idiom-close" data-act="close" aria-label="关闭">✕</button>' +
      "</header>" +
      '<section class="idiom-card">' +
      '<p class="idiom-word">' +
      escapeHtml(idiom ? idiom.word : "——") +
      "</p>" +
      (state.error
        ? '<p class="idiom-error">' + escapeHtml(state.error) + "</p>"
        : '<p class="idiom-meaning' +
          (idiom && !state.revealed ? " is-hidden" : "") +
          '">' +
          escapeHtml(meaningText) +
          "</p>") +
      "</section>" +
      '<div class="idiom-actions">' +
      (state.showMeaningAlways
        ? ""
        : '<button type="button" class="idiom-btn" data-act="reveal" ' +
          (!idiom || state.revealed || state.loading ? "disabled" : "") +
          ">揭晓</button>") +
      '<button type="button" class="idiom-btn is-primary" data-act="next" ' +
      (state.loading ? "disabled" : "") +
      ">换一条</button>" +
      "</div>";

    root.querySelector('[data-act="close"]')?.addEventListener("click", function () {
      hub().popup.close().catch(console.error);
    });
    root.querySelectorAll("[data-act]").forEach(function (btn) {
      btn.addEventListener("click", function () {
        const act = btn.getAttribute("data-act");
        if (act === "close") return;
        if (act === "reveal") {
          state.revealed = true;
          render();
        } else if (act === "next") {
          void refresh();
        }
      });
    });
  }

  async function syncFromCache() {
    await applyRevealPref();
    await loadCache();
    applyIdiom(state.idiom, true);
    if (!state.idiom && !state.loading) void refresh();
  }

  async function boot() {
    await syncFromCache();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

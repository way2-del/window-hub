/**
 * 成语历史弹窗 — 最近 200 条，分页，点进详情（含拼音）
 * 拼音库：同目录 pinyin-pro.min.js（Host 先注入；默认声调符号）
 */
(function () {
  const HISTORY_KEY = "history";
  const PAGE_SIZE = 8;

  const state = {
    items: [],
    page: 0,
    view: "list", // list | detail
    detail: null,
  };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing — open via host popup");
    return window.hub;
  }

  function escapeHtml(s) {
    return String(s ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;");
  }

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

  function ensurePinyin(item) {
    if (!item) return item;
    const py = toPinyin(item.word);
    if (py) return Object.assign({}, item, { pinyin: py });
    return item;
  }

  function formatTime(ts) {
    const n = Number(ts);
    if (!Number.isFinite(n) || n <= 0) return "";
    const d = new Date(n);
    const pad = function (x) {
      return String(x).padStart(2, "0");
    };
    return (
      d.getFullYear() +
      "-" +
      pad(d.getMonth() + 1) +
      "-" +
      pad(d.getDate()) +
      " " +
      pad(d.getHours()) +
      ":" +
      pad(d.getMinutes())
    );
  }

  async function load() {
    const raw = await hub()
      .storage.get(HISTORY_KEY)
      .catch(function () {
        return null;
      });
    const items = Array.isArray(raw && raw.items) ? raw.items : [];
    state.items = items.map(ensurePinyin);
    const maxPage = Math.max(0, Math.ceil(state.items.length / PAGE_SIZE) - 1);
    if (state.page > maxPage) state.page = maxPage;
  }

  function pageSlice() {
    const start = state.page * PAGE_SIZE;
    return state.items.slice(start, start + PAGE_SIZE);
  }

  function pageCount() {
    return Math.max(1, Math.ceil(state.items.length / PAGE_SIZE) || 1);
  }

  function render() {
    const app = document.getElementById("app");
    if (!app) return;
    app.classList.add("wg-shell");

    if (state.view === "detail" && state.detail) {
      const it = ensurePinyin(state.detail);
      app.innerHTML = `
        <header class="id-header">
          <div class="id-title-wrap">
            <div class="id-title">成语详情</div>
            <div class="id-sub">历史记录</div>
          </div>
          <button type="button" class="id-icon-btn" data-back="1" aria-label="返回" title="返回">←</button>
        </header>
        <div class="id-detail">
          <div class="id-detail-word">${escapeHtml(it.word)}</div>
          ${it.pinyin ? `<div class="id-detail-py">${escapeHtml(it.pinyin)}</div>` : ""}
          <div class="id-detail-meaning">${escapeHtml(it.meaning || "暂无释义")}</div>
          ${it.seenAt ? `<div class="id-detail-meta">出现于 ${escapeHtml(formatTime(it.seenAt))}</div>` : ""}
        </div>
      `;
      app.querySelector("[data-back]")?.addEventListener("click", function () {
        state.view = "list";
        state.detail = null;
        render();
      });
      return;
    }

    const slice = pageSlice();
    const total = state.items.length;
    const pages = pageCount();
    const pageLabel = total
      ? `第 ${state.page + 1} / ${pages} 页 · 共 ${total} 条`
      : "暂无历史";

    let rows = "";
    if (!slice.length) {
      rows = `<div class="id-empty">还没有出现过的成语<br/>切换快捷区成语后会记在这里</div>`;
    } else {
      slice.forEach(function (it, idx) {
        const globalIdx = state.page * PAGE_SIZE + idx;
        const py = it.pinyin || toPinyin(it.word);
        rows += `
          <button type="button" class="id-row" data-idx="${globalIdx}">
            <span class="id-row-word">${escapeHtml(it.word)}</span>
            ${py ? `<span class="id-row-py">${escapeHtml(py)}</span>` : ""}
            <span class="id-row-meaning">${escapeHtml(it.meaning || "")}</span>
          </button>
        `;
      });
    }

    app.innerHTML = `
      <header class="id-header">
        <div class="id-title-wrap">
          <div class="id-title">成语历史</div>
          <div class="id-sub">最近 ${Math.min(200, total)} 条 · 点击查看详情</div>
        </div>
      </header>
      <div class="id-list">${rows}</div>
      <div class="id-pager">
        <button type="button" class="id-page-btn" data-prev="1" ${state.page <= 0 ? "disabled" : ""}>上一页</button>
        <span class="id-page-meta">${escapeHtml(pageLabel)}</span>
        <button type="button" class="id-page-btn" data-next="1" ${state.page >= pages - 1 || !total ? "disabled" : ""}>下一页</button>
      </div>
    `;

    app.querySelectorAll("[data-idx]").forEach(function (el) {
      el.addEventListener("click", function () {
        const i = Number(el.getAttribute("data-idx"));
        const hit = state.items[i];
        if (!hit) return;
        state.detail = ensurePinyin(hit);
        state.view = "detail";
        render();
      });
    });
    app.querySelector("[data-prev]")?.addEventListener("click", function () {
      if (state.page <= 0) return;
      state.page -= 1;
      render();
    });
    app.querySelector("[data-next]")?.addEventListener("click", function () {
      if (state.page >= pageCount() - 1) return;
      state.page += 1;
      render();
    });
  }

  async function boot() {
    await load();
    render();
    window.addEventListener("wh-plugin-popup-refresh", function () {
      void load().then(render);
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

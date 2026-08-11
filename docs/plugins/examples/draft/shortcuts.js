/**
 * 随心记 — 快捷区：图标入口；点击在芯片下方打开弹窗。
 */
(function () {
  const STORE_KEY = "store";

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function normalize(raw) {
    const parsed = raw && typeof raw === "object" ? raw : {};
    const drafts = Array.isArray(parsed.drafts) ? parsed.drafts : [];
    const activeId = parsed.activeId != null ? String(parsed.activeId) : null;
    const list = drafts
      .filter(function (it) {
        return it && it.id;
      })
      .map(function (it) {
        return {
          id: String(it.id),
          title: String(it.title || "").trim(),
          body: String(it.body || ""),
          updatedAt: typeof it.updatedAt === "number" ? it.updatedAt : Date.now(),
          createdAt: typeof it.createdAt === "number" ? it.createdAt : Date.now(),
        };
      })
      .sort(function (a, b) {
        return b.updatedAt - a.updatedAt;
      });
    return { drafts: list, activeId: activeId };
  }

  function firstLine(text) {
    const t = String(text || "").replace(/\r\n/g, "\n").trim();
    if (!t) return "";
    return t.split("\n")[0].trim();
  }

  function trunc(s, n) {
    const t = String(s || "");
    return t.length <= n ? t : t.slice(0, n - 1) + "…";
  }

  function currentDraft(store) {
    if (!store.drafts.length) return null;
    if (store.activeId) {
      const hit = store.drafts.find(function (d) {
        return d.id === store.activeId;
      });
      if (hit) return hit;
    }
    return store.drafts[0];
  }

  function draftIcon() {
    return (
      '<svg class="draft-chip-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true" focusable="false">' +
      '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8l-5-5Z"/>' +
      '<path d="M14 3v5h5"/>' +
      '<path d="M9 13h6M9 17h4"/>' +
      "</svg>"
    );
  }

  function paint(store) {
    const bar = document.getElementById("bar");
    if (!bar) return;
    const cur = currentDraft(store);
    const line = cur ? firstLine(cur.title || cur.body) : "";
    bar.title = line ? trunc(line, 80) : "打开随心记";
    bar.innerHTML = draftIcon();
    bar.classList.remove("is-loading");
    bar.classList.add("is-ready");
    bar.removeAttribute("aria-hidden");
    reportWidth();
  }

  function reportWidth() {
    const h = hub();
    const bar = document.getElementById("bar");
    if (!bar || !h.shortcuts || !h.shortcuts.requestSize) return;
    const w = Math.ceil(
      Math.max(bar.scrollWidth, bar.getBoundingClientRect().width, 22),
    );
    void h.shortcuts.requestSize({ width: w });
  }

  function openPopup() {
    const h = hub();
    if (!h.popup || !h.popup.open) return;
    h.popup.open({});
  }

  async function loadAndPaint() {
    try {
      paint(normalize(await hub().storage.get(STORE_KEY)));
    } catch (_) {
      paint({ drafts: [], activeId: null });
    }
  }

  function onStorageChanged(ev) {
    if (!ev || ev.key !== STORE_KEY) return;
    if (ev.removed) {
      paint({ drafts: [], activeId: null });
      return;
    }
    paint(normalize(ev.value));
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
    await loadAndPaint();

    if (h.storage && h.storage.subscribe) {
      h.storage.subscribe(onStorageChanged);
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

/**
 * 待办 — 快捷区隐形 worker：冷启动 / storage 变更时同步岛栏「待办 N」。
 */
(function () {
  const STORE_KEY = "store";

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function openCount(raw) {
    const parsed = raw && typeof raw === "object" ? raw : {};
    const items = Array.isArray(parsed.items) ? parsed.items : [];
    let n = 0;
    for (let i = 0; i < items.length; i++) {
      const it = items[i];
      if (it && it.id && it.title && !it.done) n += 1;
    }
    return n;
  }

  async function syncBar() {
    const h = hub();
    if (!h.island) return;
    try {
      const raw = await h.storage.get(STORE_KEY);
      const n = openCount(raw);
      if (!n) {
        await h.island.clearBar();
        return;
      }
      await h.island.setBar({ text: "待办 " + n, title: n + " 项未完成" });
    } catch (err) {
      console.warn("[todo worker] syncBar", err);
    }
  }

  async function boot() {
    const h = hub();
    try {
      if (h.shortcuts && h.shortcuts.requestSize) {
        await h.shortcuts.requestSize({ width: 1 });
      }
    } catch (_) {}
    await syncBar();
    if (h.storage && h.storage.subscribe) {
      h.storage.subscribe(function (ev) {
        if (ev && ev.key && ev.key !== STORE_KEY) return;
        void syncBar();
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

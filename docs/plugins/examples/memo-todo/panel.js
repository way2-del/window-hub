/**
 * 备忘 Todo — 岛下拉：详情列表（不占用岛栏摘要）
 */
(function () {
  const STORE_KEY = "store";

  const state = { items: [] };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function normalize(raw) {
    const parsed = raw && typeof raw === "object" ? raw : {};
    const items = Array.isArray(parsed.items) ? parsed.items : [];
    return items
      .filter((it) => it && it.id && it.title)
      .map((it) => ({
        id: String(it.id),
        title: String(it.title),
        note: String(it.note || ""),
        dueAt: typeof it.dueAt === "number" ? it.dueAt : null,
        done: !!it.done,
        notifiedAt: typeof it.notifiedAt === "number" ? it.notifiedAt : null,
        createdAt: typeof it.createdAt === "number" ? it.createdAt : Date.now(),
      }));
  }

  function escapeHtml(s) {
    return String(s ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;");
  }

  function pad(n) {
    return String(n).padStart(2, "0");
  }

  function formatDue(ms) {
    if (ms == null) return "";
    const d = new Date(ms);
    return (
      pad(d.getMonth() + 1) +
      "/" +
      pad(d.getDate()) +
      " " +
      pad(d.getHours()) +
      ":" +
      pad(d.getMinutes())
    );
  }

  function openItems() {
    return state.items.filter((it) => !it.done);
  }

  function sortItems(items) {
    return items.slice().sort((a, b) => {
      if (a.done !== b.done) return a.done ? 1 : -1;
      if (a.dueAt != null && b.dueAt != null) return a.dueAt - b.dueAt;
      if (a.dueAt != null) return -1;
      if (b.dueAt != null) return 1;
      return b.createdAt - a.createdAt;
    });
  }

  async function clearLegacyIslandBar() {
    const h = hub();
    if (!h.island || !h.island.clearBar) return;
    try {
      await h.island.clearBar();
    } catch (_) {}
  }

  async function load() {
    const raw = await hub().storage.get(STORE_KEY);
    state.items = normalize(raw);
  }

  async function save() {
    await hub().storage.set(STORE_KEY, { version: 1, items: state.items });
  }

  function render() {
    const root = document.getElementById("app");
    if (!root) return;
    root.className = "memo-root";
    const items = sortItems(state.items);
    const openCount = openItems().length;
    const now = Date.now();

    root.innerHTML =
      '<header class="memo-head">' +
      '<span class="memo-title">备忘</span>' +
      '<span class="memo-sub">' +
      openCount +
      " 未完成</span></header>" +
      '<div class="memo-rail" role="list">' +
      (items.length
        ? items
            .map(function (it) {
              const overdue = !it.done && it.dueAt != null && it.dueAt <= now;
              return (
                '<article class="memo-card' +
                (it.done ? " is-done" : "") +
                (overdue ? " is-overdue" : "") +
                '" data-id="' +
                escapeHtml(it.id) +
                '" role="listitem">' +
                '<div class="memo-card-body">' +
                '<div class="memo-card-title">' +
                escapeHtml(it.title) +
                "</div>" +
                (it.note
                  ? '<div class="memo-card-note">' + escapeHtml(it.note) + "</div>"
                  : "") +
                (it.dueAt != null
                  ? '<div class="memo-card-due' +
                    (overdue ? " is-overdue" : "") +
                    '">提醒 ' +
                    escapeHtml(formatDue(it.dueAt)) +
                    (overdue ? " · 已到期" : "") +
                    "</div>"
                  : "") +
                "</div>" +
                '<button type="button" class="memo-card-act" data-act="toggle">' +
                (it.done ? "撤销" : "完成") +
                "</button></article>"
              );
            })
            .join("")
        : '<p class="memo-hint">暂无备忘<br />在快捷区打开弹窗添加</p>') +
      "</div>";

    root.querySelectorAll(".memo-card").forEach(function (el) {
      const id = el.getAttribute("data-id");
      el.querySelector('[data-act="toggle"]')?.addEventListener("click", function () {
        const it = state.items.find((x) => x.id === id);
        if (!it) return;
        it.done = !it.done;
        void save().then(render).catch(console.error);
      });
    });
  }

  async function boot() {
    await load();
    render();
    await clearLegacyIslandBar();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

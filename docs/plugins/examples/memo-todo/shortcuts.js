/**
 * 备忘 Todo — 快捷区：当前任务摘要 + 轮询发通知（不占岛栏）
 */
(function () {
  const STORE_KEY = "store";
  const POLL_MS = 10_000;

  const state = {
    items: [],
    width: 72,
  };

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

  function trunc(s, n) {
    const t = String(s || "");
    return t.length <= n ? t : t.slice(0, n - 1) + "…";
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

  function formatTime(ms) {
    const d = new Date(ms);
    return pad(d.getHours()) + ":" + pad(d.getMinutes());
  }

  function openItems() {
    return state.items.filter((it) => !it.done);
  }

  function currentTask() {
    const open = openItems();
    if (!open.length) return null;
    const soon = open
      .filter((it) => it.dueAt != null)
      .sort((a, b) => a.dueAt - b.dueAt)[0];
    return soon || open[0];
  }

  function hasOverdue() {
    const now = Date.now();
    return openItems().some((it) => it.dueAt != null && it.dueAt <= now);
  }

  async function load() {
    const raw = await hub().storage.get(STORE_KEY);
    state.items = normalize(raw);
  }

  async function save() {
    await hub().storage.set(STORE_KEY, { version: 1, items: state.items });
  }

  async function fireDueReminders() {
    const h = hub();
    if (!h.notify) return;
    const now = Date.now();
    let changed = false;
    for (const it of state.items) {
      if (it.done || it.dueAt == null || it.dueAt > now) continue;
      if (it.notifiedAt != null && it.notifiedAt >= it.dueAt) continue;
      try {
        await h.notify({
          title: "备忘提醒",
          body: it.title,
          urgency: "active",
          ttlMs: 12_000,
          data: { todoId: it.id },
          actions: [
            {
              id: "done",
              slot: "start",
              label: "完成",
              background: "#34c759",
              data: { todoId: it.id },
            },
            {
              id: "later",
              slot: "end",
              label: "稍后",
              background: "rgba(255,255,255,0.22)",
              data: { todoId: it.id },
            },
          ],
        });
        it.notifiedAt = now;
        changed = true;
      } catch (err) {
        console.warn("[memo-todo] notify", err);
      }
    }
    if (changed) await save();
  }

  async function handleNotifyAction(ev) {
    const todoId = ev && ev.data && ev.data.todoId;
    if (!todoId) return;
    await load();
    const it = state.items.find((x) => x.id === todoId);
    if (!it) return;
    if (ev.actionId === "done") {
      it.done = true;
      await save();
      render();
      return;
    }
    if (ev.actionId === "later") {
      it.dueAt = Date.now() + 10 * 60 * 1000;
      it.notifiedAt = null;
      await save();
      render();
    }
  }

  async function fitWidth() {
    const h = hub();
    if (!h.shortcuts || !h.shortcuts.requestSize) return;
    const el = document.getElementById("bar");
    const w = Math.ceil((el && el.scrollWidth) || state.width);
    state.width = Math.max(56, Math.min(240, w + 4));
    try {
      await h.shortcuts.requestSize({ width: state.width });
    } catch (_) {}
  }

  function openPopup() {
    const h = hub();
    if (!h.popup || !h.popup.open) return;
    h.popup.open({}).catch(console.error);
  }

  function render() {
    const root = document.getElementById("bar");
    if (!root) return;
    const n = openItems().length;
    const task = currentTask();
    const due = hasOverdue();

    let label = "备忘";
    let tip = "打开备忘列表";
    if (task) {
      const time = task.dueAt != null ? formatTime(task.dueAt) + " " : "";
      label = trunc(time + task.title, 12);
      tip =
        n +
        " 项进行中 · " +
        task.title +
        (task.dueAt != null ? " · 提醒 " + formatTime(task.dueAt) : "");
    }

    root.innerHTML =
      '<button type="button" class="memo-chip' +
      (task ? " has-task" : "") +
      '" id="memo-chip" title="' +
      escapeHtml(tip) +
      '">' +
      '<span class="memo-chip-dot' +
      (due ? " is-due" : task ? " is-active" : "") +
      '" aria-hidden="true"></span>' +
      '<span class="memo-chip-label">' +
      escapeHtml(label) +
      "</span>" +
      (n > 1
        ? '<span class="memo-chip-count">' + n + "</span>"
        : "") +
      "</button>";
    document.getElementById("memo-chip")?.addEventListener("click", openPopup);
    void fitWidth();
  }

  async function tick() {
    await load();
    await fireDueReminders();
    render();
  }

  async function clearLegacyIslandBar() {
    const h = hub();
    if (!h.island || !h.island.clearBar) return;
    try {
      await h.island.clearBar();
    } catch (_) {}
  }

  async function boot() {
    const h = hub();
    try {
      if (h.shortcuts && h.shortcuts.getBounds) {
        const b = await h.shortcuts.getBounds();
        const height = b && (b.height || b.barHeight);
        if (height) {
          document.documentElement.style.setProperty("--wh-bar-h", height + "px");
        }
      }
    } catch (_) {}
    if (h.notify && typeof h.notify.onAction === "function") {
      h.notify.onAction(function (ev) {
        void handleNotifyAction(ev).catch(console.error);
      });
    }
    await clearLegacyIslandBar();
    await tick();
    setInterval(function () {
      void tick().catch(console.error);
    }, POLL_MS);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

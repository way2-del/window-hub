/**
 * 备忘 Todo — 弹窗：完整列表 + 提醒编辑 + hub.storage + 岛栏同步
 */
(function () {
  const STORE_KEY = "store";
  const POLL_MS = 12_000;

  const state = {
    items: [],
    editingDueId: null,
    itemsSig: "",
    /** Deferred list refresh while user is typing in compose / due editor */
    pendingRender: false,
  };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function uid() {
    return "t-" + Date.now().toString(36) + "-" + Math.random().toString(36).slice(2, 7);
  }

  function normalize(raw) {
    const parsed = raw && typeof raw === "object" ? raw : {};
    const items = Array.isArray(parsed.items) ? parsed.items : [];
    return {
      version: 1,
      items: items
        .filter((it) => it && typeof it === "object" && it.id && it.title)
        .map((it) => ({
          id: String(it.id),
          title: String(it.title).trim(),
          note: String(it.note || ""),
          dueAt: typeof it.dueAt === "number" ? it.dueAt : null,
          done: !!it.done,
          notifiedAt: typeof it.notifiedAt === "number" ? it.notifiedAt : null,
          createdAt: typeof it.createdAt === "number" ? it.createdAt : Date.now(),
        })),
    };
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

  function toLocalInputValue(ms) {
    if (ms == null) return "";
    const d = new Date(ms);
    return (
      d.getFullYear() +
      "-" +
      pad(d.getMonth() + 1) +
      "-" +
      pad(d.getDate()) +
      "T" +
      pad(d.getHours()) +
      ":" +
      pad(d.getMinutes())
    );
  }

  function fromLocalInputValue(v) {
    if (!v) return null;
    const t = new Date(v).getTime();
    return Number.isFinite(t) ? t : null;
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

  /** Clear legacy island.bar text if any (this plugin no longer owns the bar). */
  async function clearLegacyIslandBar() {
    const h = hub();
    if (!h.island || !h.island.clearBar) return;
    try {
      await h.island.clearBar();
    } catch (_) {}
  }

  function itemsSig(items) {
    return JSON.stringify(
      (items || []).map((it) => [
        it.id,
        it.title,
        it.note,
        it.dueAt,
        it.done,
        it.notifiedAt,
      ]),
    );
  }

  function isEditingUi() {
    const ae = document.activeElement;
    if (!ae || ae === document.body) return false;
    const tag = (ae.tagName || "").toUpperCase();
    if (tag === "TEXTAREA") return true;
    if (tag === "INPUT") return true;
    if (ae.isContentEditable) return true;
    if (state.editingDueId) return true;
    return false;
  }

  function captureComposeDraft() {
    const form = document.getElementById("compose");
    if (!form) return null;
    const title = form.querySelector('[name="title"]');
    const note = form.querySelector('[name="note"]');
    const due = form.querySelector('[name="due"]');
    const ae = document.activeElement;
    let focus = null;
    if (ae && form.contains(ae) && ae.getAttribute("name")) {
      focus = {
        name: ae.getAttribute("name"),
        start: typeof ae.selectionStart === "number" ? ae.selectionStart : null,
        end: typeof ae.selectionEnd === "number" ? ae.selectionEnd : null,
      };
    }
    return {
      title: title ? title.value : "",
      note: note ? note.value : "",
      due: due ? due.value : "",
      focus,
    };
  }

  function restoreComposeDraft(draft) {
    if (!draft) return;
    const form = document.getElementById("compose");
    if (!form) return;
    const title = form.querySelector('[name="title"]');
    const note = form.querySelector('[name="note"]');
    const due = form.querySelector('[name="due"]');
    if (title) title.value = draft.title || "";
    if (note) note.value = draft.note || "";
    if (due) due.value = draft.due || "";
    if (draft.focus && draft.focus.name) {
      const el = form.querySelector('[name="' + draft.focus.name + '"]');
      if (el && typeof el.focus === "function") {
        el.focus();
        if (
          draft.focus.start != null &&
          draft.focus.end != null &&
          typeof el.setSelectionRange === "function"
        ) {
          try {
            el.setSelectionRange(draft.focus.start, draft.focus.end);
          } catch (_) {}
        }
      }
    }
  }

  async function load() {
    const raw = await hub().storage.get(STORE_KEY);
    state.items = normalize(raw).items;
    state.itemsSig = itemsSig(state.items);
  }

  async function save() {
    await hub().storage.set(STORE_KEY, { version: 1, items: state.items });
    state.itemsSig = itemsSig(state.items);
  }

  function renderSafe() {
    const draft = captureComposeDraft();
    render();
    restoreComposeDraft(draft);
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

  function render() {
    const root = document.getElementById("app");
    if (!root) return;
    /* Host mounts #app.wg-shell — keep both so popup.css tokens apply */
    root.className = "wg-shell memo-shell";
    const items = sortItems(state.items);
    const openCount = openItems().length;

    root.innerHTML =
      '<header class="memo-header">' +
      '<div><div class="memo-title">备忘 Todo</div>' +
      '<div class="memo-sub">' +
      openCount +
      " 项未完成 · 共 " +
      items.length +
      " 项</div></div>" +
      '<button type="button" class="memo-close" data-act="close" aria-label="关闭">✕</button>' +
      "</header>" +
      '<form class="memo-compose" id="compose">' +
      '<div class="memo-compose-fields">' +
      '<input type="text" name="title" placeholder="新备忘标题" maxlength="80" required autocomplete="off" />' +
      '<textarea name="note" placeholder="详细说明（可选）" maxlength="400"></textarea>' +
      '<div class="memo-compose-meta">' +
      "<label>提醒 <input type=\"datetime-local\" name=\"due\" /></label>" +
      "</div></div>" +
      '<button type="submit" class="memo-add">添加</button>' +
      "</form>" +
      '<div class="memo-list" role="list">' +
      (items.length
        ? items
            .map((it) => {
              const overdue = !it.done && it.dueAt != null && it.dueAt <= Date.now();
              const editing = state.editingDueId === it.id;
              return (
                '<article class="memo-item' +
                (it.done ? " is-done" : "") +
                '" data-id="' +
                escapeHtml(it.id) +
                '" role="listitem">' +
                '<button type="button" class="memo-check" data-act="toggle" aria-label="完成">' +
                (it.done ? "✓" : "") +
                "</button>" +
                '<div class="memo-item-body">' +
                '<div class="memo-item-title">' +
                escapeHtml(it.title) +
                "</div>" +
                (it.note
                  ? '<div class="memo-item-note">' + escapeHtml(it.note) + "</div>"
                  : "") +
                (it.dueAt != null
                  ? '<div class="memo-item-due' +
                    (overdue ? " is-overdue" : "") +
                    '">提醒 ' +
                    escapeHtml(formatDue(it.dueAt)) +
                    (overdue ? " · 已到期" : "") +
                    "</div>"
                  : '<div class="memo-item-due">未设置提醒</div>') +
                (editing
                  ? '<div class="memo-edit-due">' +
                    '<input type="datetime-local" data-act="due-input" value="' +
                    escapeHtml(toLocalInputValue(it.dueAt)) +
                    '" />' +
                    '<button type="button" data-act="due-save">保存</button>' +
                    '<button type="button" data-act="due-clear">清除</button>' +
                    '<button type="button" data-act="due-cancel">取消</button>' +
                    "</div>"
                  : "") +
                "</div>" +
                '<div class="memo-item-actions">' +
                '<button type="button" data-act="edit-due">提醒</button>' +
                '<button type="button" class="is-danger" data-act="remove">删除</button>' +
                "</div></article>"
              );
            })
            .join("")
        : '<div class="memo-empty">还没有备忘，先在上方添加一条</div>') +
      "</div>" +
      '<footer class="memo-footer">数据保存在本机插件存储 · 提醒由前端轮询触发</footer>';

    const form = document.getElementById("compose");
    if (form) {
      form.addEventListener("submit", function (e) {
        e.preventDefault();
        const fd = new FormData(form);
        const title = String(fd.get("title") || "").trim();
        if (!title) return;
        const note = String(fd.get("note") || "").trim();
        const dueAt = fromLocalInputValue(String(fd.get("due") || ""));
        state.items.unshift({
          id: uid(),
          title,
          note,
          dueAt,
          done: false,
          notifiedAt: null,
          createdAt: Date.now(),
        });
        void save().then(render).catch(console.error);
      });
    }

    root.querySelector('[data-act="close"]')?.addEventListener("click", function () {
      hub().popup.close().catch(console.error);
    });

    root.querySelectorAll(".memo-item").forEach(function (el) {
      const id = el.getAttribute("data-id");
      el.querySelector('[data-act="toggle"]')?.addEventListener("click", function () {
        const it = state.items.find((x) => x.id === id);
        if (!it) return;
        it.done = !it.done;
        if (it.done) it.notifiedAt = it.notifiedAt || Date.now();
        void save().then(render).catch(console.error);
      });
      el.querySelector('[data-act="remove"]')?.addEventListener("click", function () {
        state.items = state.items.filter((x) => x.id !== id);
        if (state.editingDueId === id) state.editingDueId = null;
        void save().then(render).catch(console.error);
      });
      el.querySelector('[data-act="edit-due"]')?.addEventListener("click", function () {
        state.editingDueId = id;
        render();
      });
      el.querySelector('[data-act="due-cancel"]')?.addEventListener("click", function () {
        state.editingDueId = null;
        render();
      });
      el.querySelector('[data-act="due-clear"]')?.addEventListener("click", function () {
        const it = state.items.find((x) => x.id === id);
        if (!it) return;
        it.dueAt = null;
        it.notifiedAt = null;
        state.editingDueId = null;
        void save().then(render).catch(console.error);
      });
      el.querySelector('[data-act="due-save"]')?.addEventListener("click", function () {
        const it = state.items.find((x) => x.id === id);
        const input = el.querySelector('[data-act="due-input"]');
        if (!it || !input) return;
        it.dueAt = fromLocalInputValue(input.value);
        it.notifiedAt = null;
        state.editingDueId = null;
        void save().then(render).catch(console.error);
      });
    });
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
      if (isEditingUi()) state.pendingRender = true;
      else renderSafe();
      return;
    }
    if (ev.actionId === "later") {
      it.dueAt = Date.now() + 10 * 60 * 1000;
      it.notifiedAt = null;
      await save();
      if (isEditingUi()) state.pendingRender = true;
      else renderSafe();
    }
  }

  /** Poll: fire reminders; never wipe compose while typing. */
  async function tick() {
    const before = state.itemsSig;
    await load();
    await fireDueReminders();
    const changed = state.itemsSig !== before;
    if (!changed && !state.pendingRender) return;
    if (isEditingUi()) {
      state.pendingRender = true;
      return;
    }
    state.pendingRender = false;
    renderSafe();
  }

  function onComposeBlurFlush() {
    window.setTimeout(function () {
      if (!state.pendingRender || isEditingUi()) return;
      state.pendingRender = false;
      renderSafe();
    }, 0);
  }

  async function boot() {
    const h = hub();
    if (h.notify && typeof h.notify.onAction === "function") {
      h.notify.onAction(function (ev) {
        void handleNotifyAction(ev).catch(console.error);
      });
    }
    // Host inject 后立刻检查 #app；先同步画壳，再 await storage。
    render();
    await load();
    render();
    await clearLegacyIslandBar();
    await fireDueReminders();
    document.addEventListener(
      "focusout",
      function (ev) {
        const t = ev.target;
        if (!t || !t.closest) return;
        if (!t.closest("#compose") && !t.closest(".memo-edit-due")) return;
        onComposeBlurFlush();
      },
      true,
    );
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

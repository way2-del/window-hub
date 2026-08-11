/**
 * 待办 — 灵动岛面板：新建记录创建时间，岛栏显示未完成数。
 */
(function () {
  const STORE_KEY = "store";
  const state = { items: [], busy: false, error: "" };

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
    return items
      .filter(function (it) {
        return it && it.id && it.title;
      })
      .map(function (it) {
        return {
          id: String(it.id),
          title: String(it.title).trim(),
          done: !!it.done,
          createdAt: typeof it.createdAt === "number" ? it.createdAt : Date.now(),
        };
      });
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

  function formatCreated(ms) {
    const d = new Date(ms);
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

  function openItems() {
    return state.items.filter(function (it) {
      return !it.done;
    });
  }

  function sortItems(items) {
    return items.slice().sort(function (a, b) {
      if (a.done !== b.done) return a.done ? 1 : -1;
      return b.createdAt - a.createdAt;
    });
  }

  async function load() {
    const raw = await hub().storage.get(STORE_KEY);
    state.items = normalize(raw);
  }

  async function save() {
    await hub().storage.set(STORE_KEY, { version: 1, items: state.items });
  }

  function syncBar() {
    const h = hub();
    if (!h.island) return Promise.resolve();
    const n = openItems().length;
    if (!n) return h.island.clearBar().catch(function () {});
    return h.island
      .setBar({ text: "待办 " + n, title: n + " 项未完成" })
      .catch(function () {});
  }

  function persist() {
    return save()
      .then(function () {
        state.error = "";
        // 岛栏常驻是天气时 setBar 会被 Host 忽略，不影响保存
        return syncBar().catch(function () {});
      })
      .catch(function (err) {
        state.error = String(err && err.message ? err.message : err);
        console.error("[todo] save", err);
      });
  }

  let clearInputOnce = false;

  function addTodo(title) {
    const t = String(title || "").trim();
    if (!t || state.busy) return false;
    state.busy = true;
    state.error = "";
    state.items.unshift({
      id: uid(),
      title: t,
      done: false,
      createdAt: Date.now(),
    });
    clearInputOnce = true;
    render();
    void persist().finally(function () {
      state.busy = false;
      render();
    });
    return true;
  }

  function render() {
    const root = document.getElementById("app");
    if (!root) return;
    const keepFocus =
      clearInputOnce ||
      (document.activeElement && document.activeElement.id === "todo-input");
    const keepVal = clearInputOnce
      ? ""
      : keepFocus && document.activeElement
        ? document.activeElement.value
        : "";
    clearInputOnce = false;
    root.className = "todo-root";
    const items = sortItems(state.items);
    const openCount = openItems().length;

    root.innerHTML =
      '<header class="todo-head">' +
      '<span class="todo-title">待办</span>' +
      '<span class="todo-sub">' +
      openCount +
      " 未完成</span></header>" +
      '<div class="todo-compose" id="compose">' +
      '<input id="todo-input" type="text" maxlength="120" placeholder="新建待办…" autocomplete="off" />' +
      '<button type="button" id="todo-add"' +
      (state.busy ? " disabled" : "") +
      ">添加</button></div>" +
      (state.error
        ? '<p class="todo-error">' + escapeHtml(state.error) + "</p>"
        : "") +
      '<div class="todo-rail" role="list">' +
      (items.length
        ? items
            .map(function (it) {
              return (
                '<article class="todo-card' +
                (it.done ? " is-done" : "") +
                '" data-id="' +
                escapeHtml(it.id) +
                '" role="listitem">' +
                '<div class="todo-card-body">' +
                '<div class="todo-card-title">' +
                escapeHtml(it.title) +
                "</div>" +
                '<div class="todo-card-meta">创建 ' +
                escapeHtml(formatCreated(it.createdAt)) +
                "</div></div>" +
                '<div class="todo-card-acts">' +
                '<button type="button" data-act="toggle">' +
                (it.done ? "撤销" : "完成") +
                "</button>" +
                '<button type="button" class="is-danger" data-act="remove">删除</button>' +
                "</div></article>"
              );
            })
            .join("")
        : '<p class="todo-hint">暂无待办<br/>在上方输入后添加</p>') +
      "</div>";

    const input = document.getElementById("todo-input");
    const addBtn = document.getElementById("todo-add");

    function doAdd() {
      if (!input) return;
      if (addTodo(input.value)) {
        /* render() 已刷新；新输入框保持空 */
      }
    }

    if (addBtn) {
      addBtn.addEventListener("click", function (e) {
        e.preventDefault();
        e.stopPropagation();
        doAdd();
      });
    }
    if (input) {
      if (keepFocus) {
        input.value = keepVal;
        input.focus();
        try {
          const n = input.value.length;
          input.setSelectionRange(n, n);
        } catch (_) {}
      }
      input.addEventListener("keydown", function (e) {
        if (e.key !== "Enter") return;
        e.preventDefault();
        e.stopPropagation();
        doAdd();
      });
    }

    root.querySelectorAll(".todo-card").forEach(function (el) {
      const id = el.getAttribute("data-id");
      el.querySelector('[data-act="toggle"]').addEventListener("click", function (e) {
        e.stopPropagation();
        const it = state.items.find(function (x) {
          return x.id === id;
        });
        if (!it) return;
        it.done = !it.done;
        render();
        void persist();
      });
      el.querySelector('[data-act="remove"]').addEventListener("click", function (e) {
        e.stopPropagation();
        state.items = state.items.filter(function (x) {
          return x.id !== id;
        });
        render();
        void persist();
      });
    });
  }

  async function boot() {
    try {
      await load();
    } catch (err) {
      state.error = String(err && err.message ? err.message : err);
      console.error("[todo] load", err);
    }
    render();
    void syncBar();
    const h = hub();
    if (h.storage && h.storage.subscribe) {
      h.storage.subscribe(function (ev) {
        if (!ev || ev.key !== STORE_KEY) return;
        if (state.busy) return;
        state.items = normalize(ev.removed ? null : ev.value);
        render();
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

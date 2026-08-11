/**
 * 随心记 — 弹窗：多篇历史 + 防抖自动保存（挂 #app.wg-shell）
 */
(function () {
  const STORE_KEY = "store";
  const AUTOSAVE_MS = 600;
  const MAX_DRAFTS = 40;

  const state = {
    drafts: [],
    activeId: null,
    dirty: false,
    saving: false,
    saveHint: "",
  };

  let autosaveTimer = null;

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function uid() {
    return "d-" + Date.now().toString(36) + "-" + Math.random().toString(36).slice(2, 7);
  }

  function normalize(raw) {
    const parsed = raw && typeof raw === "object" ? raw : {};
    const drafts = Array.isArray(parsed.drafts) ? parsed.drafts : [];
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
    let activeId = parsed.activeId != null ? String(parsed.activeId) : null;
    if (activeId && !list.some(function (d) {
      return d.id === activeId;
    })) {
      activeId = list[0] ? list[0].id : null;
    }
    if (!activeId && list[0]) activeId = list[0].id;
    return { drafts: list, activeId: activeId };
  }

  function escapeHtml(s) {
    return String(s ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;");
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

  function isBlankDraft(d) {
    return !d || (!String(d.title || "").trim() && !String(d.body || "").trim());
  }

  function formatTime(ts) {
    if (!ts) return "";
    const d = new Date(ts);
    if (Number.isNaN(d.getTime())) return "";
    const now = Date.now();
    const diff = now - ts;
    if (diff < 60 * 1000) return "刚刚";
    if (diff < 60 * 60 * 1000) return Math.floor(diff / 60000) + " 分钟前";
    if (diff < 24 * 60 * 60 * 1000) return Math.floor(diff / 3600000) + " 小时前";
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    const hh = String(d.getHours()).padStart(2, "0");
    const mm = String(d.getMinutes()).padStart(2, "0");
    const thisYear = new Date().getFullYear();
    return (y === thisYear ? "" : y + "/") + m + "/" + day + " " + hh + ":" + mm;
  }

  function activeDraft() {
    return (
      state.drafts.find(function (d) {
        return d.id === state.activeId;
      }) || null
    );
  }

  async function persist() {
    // 裁剪过多历史（保留最近）
    if (state.drafts.length > MAX_DRAFTS) {
      state.drafts = state.drafts
        .slice()
        .sort(function (a, b) {
          return b.updatedAt - a.updatedAt;
        })
        .slice(0, MAX_DRAFTS);
      if (!state.drafts.some(function (d) {
        return d.id === state.activeId;
      })) {
        state.activeId = state.drafts[0] ? state.drafts[0].id : null;
      }
    }
    await hub().storage.set(STORE_KEY, {
      version: 1,
      drafts: state.drafts,
      activeId: state.activeId,
    });
  }

  async function load() {
    const store = normalize(await hub().storage.get(STORE_KEY));
    state.drafts = store.drafts;
    state.activeId = store.activeId;
    state.dirty = false;
    state.saveHint = "";
  }

  function readEditor() {
    const titleEl = document.getElementById("draft-title");
    const bodyEl = document.getElementById("draft-body");
    return {
      title: titleEl ? String(titleEl.value || "").trim() : "",
      body: bodyEl ? String(bodyEl.value || "") : "",
    };
  }

  function ensureActive() {
    if (state.activeId && activeDraft()) return;
    const now = Date.now();
    const blank = {
      id: uid(),
      title: "",
      body: "",
      updatedAt: now,
      createdAt: now,
    };
    state.drafts.unshift(blank);
    state.activeId = blank.id;
    state.dirty = false;
  }

  function clearAutosave() {
    if (autosaveTimer) {
      window.clearTimeout(autosaveTimer);
      autosaveTimer = null;
    }
  }

  function scheduleAutosave() {
    clearAutosave();
    autosaveTimer = window.setTimeout(function () {
      autosaveTimer = null;
      void saveCurrent({ quiet: true }).catch(console.error);
    }, AUTOSAVE_MS);
  }

  async function saveCurrent(opts) {
    const quiet = !!(opts && opts.quiet);
    ensureActive();
    const cur = activeDraft();
    if (!cur) return;
    const ed = readEditor();
    const same =
      String(cur.title || "") === ed.title && String(cur.body || "") === ed.body;
    if (same && !state.dirty) {
      if (!quiet) updateStatusOnly("已保存");
      return;
    }
    state.saving = true;
    if (!quiet) updateStatusOnly("保存中…");
    cur.title = ed.title;
    cur.body = ed.body;
    cur.updatedAt = Date.now();
    state.dirty = false;
    state.drafts.sort(function (a, b) {
      return b.updatedAt - a.updatedAt;
    });
    try {
      await persist();
      state.saveHint = "已自动保存 · " + formatTime(cur.updatedAt);
      if (!quiet) state.saveHint = "已保存 · " + formatTime(cur.updatedAt);
    } finally {
      state.saving = false;
      render(true);
    }
  }

  function updateStatusOnly(text) {
    const el = document.querySelector(".draft-status");
    if (el) el.textContent = text;
  }

  function findReusableBlank(excludeId) {
    return state.drafts.find(function (d) {
      return d.id !== excludeId && isBlankDraft(d);
    });
  }

  async function newDraft() {
    clearAutosave();
    if (state.dirty || hasEditorContent()) {
      await saveCurrent({ quiet: true });
    }
    const cur = activeDraft();
    // 当前已是空白 → 无需再建
    if (cur && isBlankDraft(cur) && !hasEditorContent()) {
      focusBody();
      return;
    }
    // 复用其它空白篇，避免堆一堆空篇
    const reusable = findReusableBlank(cur ? cur.id : null);
    if (reusable) {
      state.activeId = reusable.id;
      state.dirty = false;
      await persist();
      render(false);
      focusBody();
      return;
    }
    const now = Date.now();
    const blank = {
      id: uid(),
      title: "",
      body: "",
      updatedAt: now,
      createdAt: now,
    };
    state.drafts.unshift(blank);
    state.activeId = blank.id;
    state.dirty = false;
    state.saveHint = "新笔记";
    await persist();
    render(false);
    focusBody();
  }

  function hasEditorContent() {
    const ed = readEditor();
    return !!(ed.title || String(ed.body || "").trim());
  }

  async function selectDraft(id) {
    if (id === state.activeId) return;
    clearAutosave();
    if (state.dirty || hasEditorContent()) {
      await saveCurrent({ quiet: true });
    }
    state.activeId = id;
    state.dirty = false;
    state.saveHint = "";
    await persist();
    render(false);
    focusBody();
  }

  async function removeDraft(id) {
    clearAutosave();
    state.drafts = state.drafts.filter(function (d) {
      return d.id !== id;
    });
    if (state.activeId === id) {
      state.activeId = state.drafts[0] ? state.drafts[0].id : null;
    }
    ensureActive();
    state.dirty = false;
    state.saveHint = "已删除";
    await persist();
    render(false);
  }

  async function closePopup() {
    clearAutosave();
    try {
      if (state.dirty || hasEditorContent()) {
        await saveCurrent({ quiet: true });
      }
    } catch (_) {}
    await hub().popup.close();
  }

  function focusBody() {
    window.setTimeout(function () {
      const el = document.getElementById("draft-body");
      if (el && typeof el.focus === "function") el.focus();
    }, 0);
  }

  function draftLabel(d) {
    return trunc(firstLine(d.title || d.body) || "空白笔记", 26) || "空白笔记";
  }

  function render(keepFocus) {
    const root = document.getElementById("app");
    if (!root) return;
    root.className = "wg-shell";
    ensureActive();
    const cur = activeDraft();
    const title = cur ? cur.title : "";
    const body = cur ? cur.body : "";
    const ae = document.activeElement;
    const focusName = keepFocus && ae && ae.id ? ae.id : null;
    const selStart =
      keepFocus && ae && typeof ae.selectionStart === "number" ? ae.selectionStart : null;
    const selEnd =
      keepFocus && ae && typeof ae.selectionEnd === "number" ? ae.selectionEnd : null;

    const statusText = state.saving
      ? "保存中…"
      : state.dirty
        ? "编辑中 · 即将自动保存"
        : state.saveHint ||
          (cur && cur.updatedAt ? "上次保存 · " + formatTime(cur.updatedAt) : "本地笔记");

    const historyHtml = state.drafts.length
      ? state.drafts
          .map(function (d) {
            return (
              '<div class="draft-item' +
              (d.id === state.activeId ? " is-active" : "") +
              '" data-id="' +
              escapeHtml(d.id) +
              '" role="listitem">' +
              '<button type="button" class="draft-item-main" data-act="select">' +
              '<span class="draft-item-text">' +
              escapeHtml(draftLabel(d)) +
              "</span>" +
              '<span class="draft-item-meta">' +
              escapeHtml(formatTime(d.updatedAt)) +
              (isBlankDraft(d) ? " · 空" : "") +
              "</span></button>" +
              '<button type="button" class="draft-item-del" data-act="item-del" aria-label="删除">✕</button>' +
              "</div>"
            );
          })
          .join("")
      : '<div class="draft-empty">还没有历史，写点什么再点「+」新建下一篇</div>';

    root.innerHTML =
      '<header class="draft-header">' +
      "<div>" +
      '<div class="draft-kicker">Notes</div>' +
      '<div class="draft-title">随心记</div>' +
      '<div class="draft-sub">可保存多篇历史，自动写入本地</div>' +
      "</div>" +
      '<div class="draft-header-actions">' +
      '<button type="button" class="draft-icon-btn is-accent" data-act="new" title="新建" aria-label="新建">+</button>' +
      '<button type="button" class="draft-icon-btn" data-act="close" aria-label="关闭">✕</button>' +
      "</div></header>" +
      '<div class="draft-editor">' +
      '<input type="text" id="draft-title" placeholder="标题（可选）" maxlength="80" autocomplete="off" value="' +
      escapeHtml(title) +
      '" />' +
      '<textarea id="draft-body" placeholder="在这里随手写… 切换或关闭会自动保存" maxlength="20000">' +
      escapeHtml(body) +
      "</textarea>" +
      '<div class="draft-toolbar">' +
      '<span class="draft-status">' +
      escapeHtml(statusText) +
      "</span>" +
      '<button type="button" class="is-primary" data-act="save">保存</button>' +
      '<button type="button" class="is-danger" data-act="delete">删除</button>' +
      "</div></div>" +
      '<section class="draft-history">' +
      '<div class="draft-history-head">' +
      '<span class="draft-list-label">历史</span>' +
      '<span class="draft-list-count">' +
      state.drafts.length +
      " 篇</span></div>" +
      '<div class="draft-list" role="list">' +
      historyHtml +
      "</div></section>";

    root.querySelector('[data-act="close"]')?.addEventListener("click", function () {
      void closePopup().catch(console.error);
    });
    root.querySelector('[data-act="save"]')?.addEventListener("click", function () {
      clearAutosave();
      void saveCurrent({ quiet: false }).catch(console.error);
    });
    root.querySelector('[data-act="new"]')?.addEventListener("click", function () {
      void newDraft().catch(console.error);
    });
    root.querySelector('[data-act="delete"]')?.addEventListener("click", function () {
      if (!state.activeId) return;
      void removeDraft(state.activeId).catch(console.error);
    });

    const titleEl = document.getElementById("draft-title");
    const bodyEl = document.getElementById("draft-body");
    function onEdit() {
      state.dirty = true;
      updateStatusOnly("编辑中 · 即将自动保存");
      scheduleAutosave();
    }
    titleEl?.addEventListener("input", onEdit);
    bodyEl?.addEventListener("input", onEdit);

    root.querySelectorAll(".draft-item").forEach(function (el) {
      const id = el.getAttribute("data-id");
      el.querySelector('[data-act="select"]')?.addEventListener("click", function () {
        void selectDraft(id).catch(console.error);
      });
      el.querySelector('[data-act="item-del"]')?.addEventListener("click", function (e) {
        e.stopPropagation();
        void removeDraft(id).catch(console.error);
      });
    });

    if (focusName) {
      const el = document.getElementById(focusName);
      if (el && typeof el.focus === "function") {
        el.focus();
        if (
          selStart != null &&
          selEnd != null &&
          typeof el.setSelectionRange === "function"
        ) {
          try {
            el.setSelectionRange(selStart, selEnd);
          } catch (_) {}
        }
      }
    }
  }

  async function boot() {
    await load();
    ensureActive();
    if (!state.drafts.length) {
      await persist();
    }
    render(false);
    focusBody();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

/**
 * Page Watch — popup: list / add (open WebView + pick) / toggle / notify on change.
 * Draft form fields persist in hub.storage so closing the popup does not wipe input.
 */
(function () {
  const STORE_KEY = "watchItems";
  const DRAFT_KEY = "formDraft";
  const SCAN_LOG_KEY = "scanLogs";
  const PREFS_KEY = "prefs";
  const SCAN_LOG_MAX = 200;
  const DEFAULT_PREFS = {
    notifyTitle: "{title}",
    notifyBody: "{title}更新了",
  };

  /** @type {{ id: string, title: string, url: string, selector: string, intervalMs: number, enabled: boolean, textPreview?: string }[]} */
  let items = [];
  /** @type {{ id: string, watchId: string, title?: string, atMs: number, ok: boolean, error?: string, text: string, prevText: string, changed: boolean, baseline: boolean }[]} */
  let scanLogs = [];
  let logFilterId = null;
  /** @type {"main" | "logs" | "settings"} */
  let currentPage = "main";
  /** @type {{ notifyTitle: string, notifyBody: string }} */
  let prefs = Object.assign({}, DEFAULT_PREFS);
  let confirmResolver = null;
  let draftSessionId = null;
  /** When set, save updates this item instead of creating a new one. */
  let editingId = null;
  let busy = false;
  let draftTimer = null;

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function uid() {
    return "w" + Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
  }

  function root() {
    const el = document.getElementById("app");
    if (!el) throw new Error("#app missing");
    el.classList.add("wg-shell", "pw-shell");
    return el;
  }

  function val(id) {
    const el = document.getElementById(id);
    return el && typeof el.value === "string" ? el.value : "";
  }

  function setVal(id, v) {
    const el = document.getElementById(id);
    if (el) el.value = v == null ? "" : String(v);
  }

  function isFormOpen() {
    const form = document.getElementById("pw-form");
    return !!(form && !form.classList.contains("hidden"));
  }

  function readDraftFromDom() {
    return {
      version: 1,
      formOpen: isFormOpen(),
      title: val("pw-title-input"),
      url: val("pw-url"),
      selector: val("pw-selector"),
      preview: val("pw-preview"),
      intervalSec: Number(val("pw-interval")) || 60,
      sessionId: draftSessionId || null,
      editingId: editingId || null,
      hint: (document.getElementById("pw-hint") && document.getElementById("pw-hint").textContent) || "",
      hintError: !!(document.getElementById("pw-hint") && document.getElementById("pw-hint").classList.contains("error")),
      updatedAt: Date.now(),
    };
  }

  function draftHasContent(d) {
    if (!d || typeof d !== "object") return false;
    return !!(
      d.formOpen ||
      String(d.title || "").trim() ||
      String(d.url || "").trim() ||
      String(d.selector || "").trim() ||
      String(d.preview || "").trim() ||
      d.sessionId
    );
  }

  async function persistDraftNow() {
    try {
      const d = readDraftFromDom();
      if (!draftHasContent(d)) {
        await hub().storage.set(DRAFT_KEY, null);
        return;
      }
      await hub().storage.set(DRAFT_KEY, d);
    } catch (_) {}
  }

  function schedulePersistDraft() {
    if (draftTimer) clearTimeout(draftTimer);
    draftTimer = setTimeout(() => {
      draftTimer = null;
      void persistDraftNow();
    }, 200);
  }

  async function clearDraft() {
    draftSessionId = null;
    editingId = null;
    if (draftTimer) {
      clearTimeout(draftTimer);
      draftTimer = null;
    }
    try {
      await hub().storage.set(DRAFT_KEY, null);
    } catch (_) {}
    updateFormChrome();
  }

  function updateFormChrome() {
    const save = document.getElementById("pw-save");
    const heading = document.getElementById("pw-form-heading");
    if (save) save.textContent = editingId ? "保存修改" : "保存监测";
    if (heading) heading.textContent = editingId ? "编辑监测项" : "新建监测项";
  }

  function applyDraft(d) {
    if (!d || typeof d !== "object") return false;
    if (d.title != null) setVal("pw-title-input", d.title);
    if (d.url != null) setVal("pw-url", d.url);
    if (d.selector != null) setVal("pw-selector", d.selector);
    if (d.preview != null) setVal("pw-preview", d.preview);
    if (d.intervalSec != null && Number.isFinite(Number(d.intervalSec))) {
      setVal("pw-interval", Math.max(30, Math.round(Number(d.intervalSec))));
    }
    draftSessionId = d.sessionId ? String(d.sessionId) : null;
    editingId = d.editingId ? String(d.editingId) : null;
    if (editingId && !items.some((x) => x.id === editingId)) editingId = null;
    const pickBtn = document.getElementById("pw-pick");
    if (pickBtn) pickBtn.disabled = !draftSessionId;
    if (d.hint) setHint(String(d.hint), !!d.hintError);
    const open = !!d.formOpen || draftHasContent(d);
    showForm(open);
    updateFormChrome();
    return open;
  }

  function resetFormFields(defaultSec) {
    setVal("pw-title-input", "");
    setVal("pw-url", "");
    setVal("pw-selector", "");
    setVal("pw-preview", "");
    setVal("pw-interval", String(defaultSec || 60));
    draftSessionId = null;
    editingId = null;
    const pickBtn = document.getElementById("pw-pick");
    if (pickBtn) pickBtn.disabled = true;
    updateFormChrome();
  }

  async function loadItems() {
    const raw = await hub().storage.get(STORE_KEY);
    items = Array.isArray(raw) ? raw : [];
  }

  async function loadScanLogs() {
    const raw = await hub().storage.get(SCAN_LOG_KEY);
    scanLogs = Array.isArray(raw) ? raw : [];
  }

  async function saveScanLogs() {
    await hub().storage.set(SCAN_LOG_KEY, scanLogs.slice(0, SCAN_LOG_MAX));
  }

  async function appendScanLog(ev) {
    if (!ev || !ev.watchId) return;
    const entry = {
      id: uid(),
      watchId: String(ev.watchId),
      title: ev.title || "",
      atMs: Number(ev.atMs) || Date.now(),
      ok: ev.ok !== false,
      error: ev.error ? String(ev.error) : "",
      text: String(ev.text || ""),
      prevText: String(ev.prevText || ""),
      changed: !!ev.changed,
      baseline: !!ev.baseline,
    };
    scanLogs.unshift(entry);
    if (scanLogs.length > SCAN_LOG_MAX) scanLogs.length = SCAN_LOG_MAX;
    await saveScanLogs();
    if (currentPage === "logs") renderLogs();
  }

  function formatTime(ms) {
    const d = new Date(ms || Date.now());
    const p = (n) => String(n).padStart(2, "0");
    return (
      p(d.getMonth() + 1) +
      "-" +
      p(d.getDate()) +
      " " +
      p(d.getHours()) +
      ":" +
      p(d.getMinutes()) +
      ":" +
      p(d.getSeconds())
    );
  }

  function shortText(s, n) {
    const t = String(s || "").replace(/\s+/g, " ").trim();
    if (t.length <= (n || 120)) return t;
    return t.slice(0, n || 120) + "…";
  }

  async function loadPrefs() {
    try {
      const raw = await hub().storage.get(PREFS_KEY);
      if (raw && typeof raw === "object") {
        prefs = {
          notifyTitle:
            typeof raw.notifyTitle === "string" && raw.notifyTitle.trim()
              ? raw.notifyTitle
              : DEFAULT_PREFS.notifyTitle,
          notifyBody:
            typeof raw.notifyBody === "string" && raw.notifyBody.trim()
              ? raw.notifyBody
              : DEFAULT_PREFS.notifyBody,
        };
        return;
      }
    } catch (_) {}
    prefs = Object.assign({}, DEFAULT_PREFS);
  }

  async function savePrefs() {
    await hub().storage.set(PREFS_KEY, {
      notifyTitle: prefs.notifyTitle,
      notifyBody: prefs.notifyBody,
    });
  }

  function applyNotifyTemplate(tpl, vars) {
    return String(tpl == null ? "" : tpl).replace(
      /\{(title|name|text|url|selector)\}/g,
      function (_, key) {
        if (key === "name") return vars.title != null ? String(vars.title) : "";
        return vars[key] != null ? String(vars[key]) : "";
      },
    );
  }

  function formatNotify(vars) {
    const title = applyNotifyTemplate(prefs.notifyTitle, vars).trim() || vars.title || "网页变化";
    const body =
      applyNotifyTemplate(prefs.notifyBody, vars).trim() ||
      String(vars.text || "").slice(0, 160) ||
      "监测内容已更新";
    return {
      title: title.slice(0, 80),
      body: body.slice(0, 200),
    };
  }

  function askConfirm(message, okLabel) {
    return new Promise((resolve) => {
      const overlay = document.getElementById("pw-confirm");
      const msg = document.getElementById("pw-confirm-msg");
      const okBtn = document.getElementById("pw-confirm-ok");
      if (!overlay || !msg) {
        resolve(false);
        return;
      }
      if (confirmResolver) {
        confirmResolver(false);
        confirmResolver = null;
      }
      confirmResolver = resolve;
      msg.textContent = message || "确定吗？";
      if (okBtn) okBtn.textContent = okLabel || "确定";
      overlay.classList.remove("hidden");
    });
  }

  function closeConfirm(result) {
    const overlay = document.getElementById("pw-confirm");
    if (overlay) overlay.classList.add("hidden");
    const fn = confirmResolver;
    confirmResolver = null;
    if (fn) fn(!!result);
  }

  function showPage(page) {
    currentPage = page === "logs" ? "logs" : page === "settings" ? "settings" : "main";
    const main = document.getElementById("pw-page-main");
    const logs = document.getElementById("pw-page-logs");
    const settings = document.getElementById("pw-page-settings");
    if (main) main.classList.toggle("hidden", currentPage !== "main");
    if (logs) logs.classList.toggle("hidden", currentPage !== "logs");
    if (settings) settings.classList.toggle("hidden", currentPage !== "settings");
    if (currentPage === "logs") renderLogs();
    if (currentPage === "settings") fillSettingsForm();
  }

  function openLogsPage(watchId) {
    logFilterId = watchId || null;
    showPage("logs");
  }

  function openSettingsPage() {
    showPage("settings");
  }

  function fillSettingsForm() {
    setVal("pw-set-title", prefs.notifyTitle || DEFAULT_PREFS.notifyTitle);
    setVal("pw-set-body", prefs.notifyBody || DEFAULT_PREFS.notifyBody);
    updateSettingsPreview();
  }

  function updateSettingsPreview() {
    const titleTpl = val("pw-set-title") || DEFAULT_PREFS.notifyTitle;
    const bodyTpl = val("pw-set-body") || DEFAULT_PREFS.notifyBody;
    const vars = {
      title: "快递",
      text: "运单状态：运输中",
      url: "https://example.com",
      selector: "div.waybill",
    };
    const title = applyNotifyTemplate(titleTpl, vars).trim() || "网页变化";
    const body = applyNotifyTemplate(bodyTpl, vars).trim() || "监测内容已更新";
    const el = document.getElementById("pw-set-preview");
    if (el) el.textContent = "预览：" + title + " · " + body;
  }

  async function onSaveSettings() {
    const titleTpl = val("pw-set-title").trim() || DEFAULT_PREFS.notifyTitle;
    const bodyTpl = val("pw-set-body").trim() || DEFAULT_PREFS.notifyBody;
    prefs = { notifyTitle: titleTpl, notifyBody: bodyTpl };
    await savePrefs();
    setHint("通知格式已保存");
    showPage("main");
  }

  function renderLogs() {
    const box = document.getElementById("pw-logs");
    const label = document.getElementById("pw-logs-title");
    if (!box) return;
    const filtered = logFilterId
      ? scanLogs.filter((x) => x.watchId === logFilterId)
      : scanLogs;
    if (label) {
      const it = logFilterId ? items.find((x) => x.id === logFilterId) : null;
      label.textContent = it
        ? (it.title || it.url || "扫描日志")
        : "全部扫描日志";
    }
    if (!filtered.length) {
      box.innerHTML =
        '<div class="pw-empty">暂无扫描记录。启用监测后，每次轮询都会记一条并对比上次文本。</div>';
      return;
    }
    box.innerHTML = filtered
      .map((log) => {
        const it = items.find((x) => x.id === log.watchId);
        const name = escapeHtml((it && (it.title || it.url)) || log.title || log.watchId);
        let badge = "same";
        let badgeText = "无变化";
        if (!log.ok) {
          badge = "err";
          badgeText = "失败";
        } else if (log.baseline) {
          badge = "base";
          badgeText = "基线";
        } else if (log.changed) {
          badge = "chg";
          badgeText = "有变化";
        }
        let body = "";
        if (!log.ok) {
          body =
            '<div class="pw-log-err">' +
            escapeHtml(log.error || "扫描失败") +
            "</div>";
        } else if (log.changed) {
          body =
            '<div class="pw-log-cmp">' +
            '<div class="pw-log-col"><div class="pw-log-col-h">上次</div><div class="pw-log-col-t">' +
            escapeHtml(shortText(log.prevText, 220) || "（空）") +
            "</div></div>" +
            '<div class="pw-log-col"><div class="pw-log-col-h">本次</div><div class="pw-log-col-t">' +
            escapeHtml(shortText(log.text, 220) || "（空）") +
            "</div></div>" +
            "</div>";
        } else {
          body =
            '<div class="pw-log-text">' +
            escapeHtml(
              shortText(log.text, 220) || (log.baseline ? "（首次采样）" : "（空）"),
            ) +
            "</div>";
        }
        return (
          '<div class="pw-log" data-watch="' +
          escapeAttr(log.watchId) +
          '">' +
          '<div class="pw-log-top">' +
          '<span class="pw-log-time">' +
          escapeHtml(formatTime(log.atMs)) +
          "</span>" +
          '<span class="pw-log-badge ' +
          badge +
          '">' +
          badgeText +
          "</span>" +
          (logFilterId
            ? ""
            : '<span class="pw-log-name">' + name + "</span>") +
          "</div>" +
          body +
          "</div>"
        );
      })
      .join("");
  }

  async function saveItems() {
    await hub().storage.set(STORE_KEY, items);
    await syncWatches();
    updateBadge();
  }

  async function syncWatches() {
    const h = hub();
    if (!h.webview || !h.webview.watch) return;
    for (const it of items) {
      if (it.enabled) {
        await h.webview.watch.start({
          id: it.id,
          url: it.url,
          selector: it.selector,
          intervalMs: Math.max(30000, Number(it.intervalMs) || 60000),
          title: it.title || "网页监测",
        });
      } else {
        try {
          await h.webview.watch.stop({ id: it.id });
        } catch (_) {}
      }
    }
  }

  function updateBadge() {
    const n = items.filter((x) => x.enabled).length;
    try {
      hub().shortcuts.setBadge(n > 0 ? { text: String(n) } : null);
    } catch (_) {}
  }

  function setBusy(v) {
    busy = !!v;
    const btns = document.querySelectorAll("[data-busy]");
    btns.forEach((b) => {
      b.disabled = busy;
    });
    const pickBtn = document.getElementById("pw-pick");
    if (pickBtn && !busy) pickBtn.disabled = !draftSessionId;
  }

  function setHint(msg, isError) {
    const el = document.getElementById("pw-hint");
    if (!el) return;
    el.textContent = msg || "";
    el.classList.toggle("error", !!isError);
  }

  function renderList() {
    const list = document.getElementById("pw-list");
    if (!list) return;
    if (!items.length) {
      list.innerHTML = '<div class="pw-empty">还没有监测项。添加网址并划定元素开始监测。</div>';
      return;
    }
    list.innerHTML = items
      .map((it) => {
        const title = escapeHtml(it.title || it.url);
        const meta = escapeHtml((it.selector || "") + " · " + Math.round((it.intervalMs || 60000) / 1000) + "s");
        const on = it.enabled ? "on" : "";
        return (
          '<div class="pw-item" data-id="' +
          escapeAttr(it.id) +
          '">' +
          '<div class="pw-item-title">' +
          title +
          "</div>" +
          '<div class="pw-item-ops">' +
          '<button type="button" class="pw-toggle ' +
          on +
          '" data-act="toggle" title="开/关监测"><span class="pw-toggle-knob"></span></button>' +
          '<button type="button" class="pw-mini" data-act="open">打开</button>' +
          '<button type="button" class="pw-mini" data-act="edit">编辑</button>' +
          '<button type="button" class="pw-mini" data-act="logs">日志</button>' +
          '<button type="button" class="pw-mini" data-act="del">删除</button>' +
          "</div>" +
          '<div class="pw-item-meta">' +
          meta +
          "</div>" +
          "</div>"
        );
      })
      .join("");
  }

  function escapeHtml(s) {
    return String(s || "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function escapeAttr(s) {
    return escapeHtml(s).replace(/'/g, "&#39;");
  }

  function showForm(show) {
    if (show) showPage("main");
    const form = document.getElementById("pw-form");
    if (!form) return;
    form.classList.toggle("hidden", !show);
  }

  async function defaultIntervalMs() {
    try {
      const v = await hub().settings.get("defaultIntervalSec");
      const sec = Number(v);
      if (Number.isFinite(sec) && sec >= 30) return Math.round(sec * 1000);
    } catch (_) {}
    return 60000;
  }

  async function onAddOpen() {
    const url = val("pw-url").trim();
    if (!url) {
      setHint("请输入 https 网址", true);
      schedulePersistDraft();
      return;
    }
    setBusy(true);
    setHint("正在打开网页…");
    void persistDraftNow();
    try {
      const title = val("pw-title-input").trim() || "网页监测";
      const res = await hub().webview.open({ url: url, title: title });
      draftSessionId = res && res.sessionId ? res.sessionId : null;
      setHint("在网页中点击「划定元素」，然后点选目标区域");
      const pickBtn = document.getElementById("pw-pick");
      if (pickBtn) pickBtn.disabled = !draftSessionId;
      await persistDraftNow();
    } catch (e) {
      setHint(String(e && e.message ? e.message : e), true);
      await persistDraftNow();
    } finally {
      setBusy(false);
    }
  }

  function applyPickToForm(pick) {
    if (!pick || pick.cancelled) return false;
    const selector = String(pick.selector || "").trim();
    if (!selector) return false;
    setVal("pw-selector", selector);
    setVal("pw-preview", pick.textPreview || pick.preview || "");
    if (pick.sessionId) draftSessionId = String(pick.sessionId);
    const pickBtn = document.getElementById("pw-pick");
    if (pickBtn) pickBtn.disabled = !draftSessionId;
    showForm(true);
    setHint("已选定元素，可保存监测任务");
    schedulePersistDraft();
    return true;
  }

  async function ensureBrowseSession() {
    const url = val("pw-url").trim();
    if (draftSessionId) {
      try {
        if (hub().webview && hub().webview.snapshot) {
          await hub().webview.snapshot({
            sessionId: draftSessionId,
            selector: "body",
          });
          return draftSessionId;
        }
        return draftSessionId;
      } catch (_) {
        // Stale id after browse window closed / host restart
        draftSessionId = null;
      }
    }
    if (!url) throw new Error("请先填写网址并打开网页");
    const title = val("pw-title-input").trim() || "网页监测";
    const res = await hub().webview.open({ url: url, title: title });
    draftSessionId = res && res.sessionId ? res.sessionId : null;
    if (!draftSessionId) throw new Error("打开网页失败，未获得会话");
    const pickBtn = document.getElementById("pw-pick");
    if (pickBtn) pickBtn.disabled = false;
    await persistDraftNow();
    return draftSessionId;
  }

  async function onPick() {
    setBusy(true);
    setHint("准备划定…若浏览窗已关会自动重新打开");
    void persistDraftNow();

    let settled = false;
    const applyOnce = (pick) => {
      if (settled) return false;
      if (!applyPickToForm(Object.assign({ sessionId: draftSessionId }, pick || {}))) return false;
      settled = true;
      void persistDraftNow();
      return true;
    };

    const pollTimer = setInterval(() => {
      void (async () => {
        if (settled) return;
        try {
          const last = await hub().webview.takeLastPick();
          if (last) applyOnce(last);
        } catch (_) {}
      })();
    }, 400);

    try {
      const sessionId = await ensureBrowseSession();
      setHint(
        "请在网页中点击元素（Esc 取消）。出现蓝色高亮框后再点选；点选后弹窗会回填选择器。",
      );
      const pick = await hub().webview.startPick({ sessionId: sessionId });
      applyOnce(pick);
    } catch (e) {
      try {
        const last = await hub().webview.takeLastPick();
        if (last && applyOnce(last)) {
          /* ok */
        } else if (!settled) {
          setHint(String(e && e.message ? e.message : e), true);
          await persistDraftNow();
        }
      } catch (_) {
        if (!settled) {
          setHint(String(e && e.message ? e.message : e), true);
          await persistDraftNow();
        }
      }
    } finally {
      clearInterval(pollTimer);
      if (!settled) {
        try {
          const last = await hub().webview.takeLastPick();
          applyOnce(last);
        } catch (_) {}
      }
      setBusy(false);
    }
  }

  async function onWholePage() {
    setVal("pw-selector", "body");
    showForm(true);
    setHint("已设为整页监测（选择器 body）");
    if (draftSessionId && hub().webview && hub().webview.snapshot) {
      setBusy(true);
      try {
        const snap = await hub().webview.snapshot({
          sessionId: draftSessionId,
          selector: "body",
        });
        setVal("pw-preview", ((snap && snap.text) || "").slice(0, 240));
        setHint("已设为整页监测，可直接保存");
      } catch (e) {
        setHint("整页已选定；预览拉取失败可仍保存：" + String(e && e.message ? e.message : e), true);
      } finally {
        setBusy(false);
      }
    }
    await persistDraftNow();
  }

  function beginEdit(it) {
    if (!it) return;
    editingId = it.id;
    setVal("pw-title-input", it.title || "");
    setVal("pw-url", it.url || "");
    setVal("pw-selector", it.selector || "");
    setVal("pw-preview", it.textPreview || "");
    setVal("pw-interval", String(Math.max(30, Math.round((it.intervalMs || 60000) / 1000))));
    draftSessionId = null;
    const pickBtn = document.getElementById("pw-pick");
    if (pickBtn) pickBtn.disabled = true;
    showForm(true);
    updateFormChrome();
    setHint("正在编辑「" + (it.title || it.url || "") + "」。可改选择器或重新划定后保存。");
    schedulePersistDraft();
  }

  async function onSave() {
    const url = val("pw-url").trim();
    let selector = val("pw-selector").trim();
    if (!selector) selector = "body";
    if (!url) {
      setHint("需要网址", true);
      schedulePersistDraft();
      return;
    }
    let intervalMs = Number(val("pw-interval")) * 1000;
    if (!Number.isFinite(intervalMs) || intervalMs < 30000) intervalMs = await defaultIntervalMs();
    const title = val("pw-title-input").trim() || url;
    const textPreview = val("pw-preview").slice(0, 240);
    const wholePage = selector === "body" || selector === "html";

    if (editingId) {
      const idx = items.findIndex((x) => x.id === editingId);
      if (idx < 0) {
        setHint("原监测项已不存在，将另存为新项", true);
        editingId = null;
      } else {
        const prev = items[idx];
        const wasEnabled = prev.enabled !== false;
        setBusy(true);
        setHint("正在更新基线…");
        try {
          // stop clears Host last_text so the next scan becomes a fresh baseline
          // instead of comparing against the pre-edit snapshot.
          try {
            await hub().webview.watch.stop({ id: prev.id });
          } catch (_) {}

          let nextPreview = textPreview;
          try {
            if (hub().webview && hub().webview.snapshot) {
              const snap = await hub().webview.snapshot({
                url: url,
                selector: selector,
              });
              const t = ((snap && snap.text) || "").replace(/\s+/g, " ").trim();
              if (t) nextPreview = t.slice(0, 240);
            }
          } catch (e) {
            console.warn("[page-watch] baseline snapshot", e);
          }

          items[idx] = {
            id: prev.id,
            title: title,
            url: url,
            selector: selector,
            intervalMs: intervalMs,
            enabled: wasEnabled,
            textPreview: nextPreview,
            wholePage: wholePage,
          };
          setVal("pw-preview", nextPreview);
          await saveItems();
          const sec = Math.round((await defaultIntervalMs()) / 1000);
          resetFormFields(sec);
          showForm(false);
          setHint(
            wasEnabled
              ? "已更新监测项，已丢弃旧基线；下次成功扫描将记为新基线"
              : "已更新监测项（当前未启用监测）",
          );
          await clearDraft();
          renderList();
        } finally {
          setBusy(false);
        }
        return;
      }
    }

    const item = {
      id: uid(),
      title: title,
      url: url,
      selector: selector,
      intervalMs: intervalMs,
      enabled: true,
      textPreview: textPreview,
      wholePage: wholePage,
    };
    items.unshift(item);
    await saveItems();
    const sec = Math.round((await defaultIntervalMs()) / 1000);
    resetFormFields(sec);
    showForm(false);
    setHint("已添加并开始监测");
    await clearDraft();
    renderList();
  }

  async function onCancel() {
    const sec = Math.round((await defaultIntervalMs()) / 1000);
    resetFormFields(sec);
    showForm(false);
    setHint("");
    await clearDraft();
  }

  async function onListClick(ev) {
    const t = ev.target;
    const btn = t && t.closest ? t.closest("[data-act]") : null;
    const row = t && t.closest ? t.closest(".pw-item") : null;
    if (!btn || !row) return;
    const id = row.getAttribute("data-id");
    const it = items.find((x) => x.id === id);
    if (!it) return;
    const act = btn.getAttribute("data-act");
    if (act === "toggle") {
      it.enabled = !it.enabled;
      await saveItems();
      renderList();
      return;
    }
    if (act === "open") {
      try {
        await hub().webview.open({ url: it.url, title: it.title });
      } catch (e) {
        setHint(String(e && e.message ? e.message : e), true);
      }
      return;
    }
    if (act === "edit") {
      beginEdit(it);
      return;
    }
    if (act === "logs") {
      openLogsPage(it.id);
      return;
    }
    if (act === "del") {
      const ok = await askConfirm(
        "确定删除「" + (it.title || it.url || "该项") + "」？删除后不可恢复。",
        "删除",
      );
      if (!ok) return;
      it.enabled = false;
      try {
        await hub().webview.watch.stop({ id: it.id });
      } catch (_) {}
      items = items.filter((x) => x.id !== id);
      await saveItems();
      renderList();
      setHint("已删除监测项");
    }
  }

  function bindActions() {
    document.getElementById("pw-add")?.addEventListener("click", () => {
      void (async () => {
        const sec = Math.round((await defaultIntervalMs()) / 1000);
        resetFormFields(sec);
        showForm(true);
        setHint("");
        await persistDraftNow();
      })();
    });
    document.getElementById("pw-cancel")?.addEventListener("click", () => void onCancel());
    document.getElementById("pw-open")?.addEventListener("click", () => void onAddOpen());
    document.getElementById("pw-pick")?.addEventListener("click", () => void onPick());
    document.getElementById("pw-whole")?.addEventListener("click", () => void onWholePage());
    document.getElementById("pw-save")?.addEventListener("click", () => void onSave());
    document.getElementById("pw-list")?.addEventListener("click", (ev) => void onListClick(ev));
    document.getElementById("pw-logs-back")?.addEventListener("click", () => {
      showPage("main");
    });
    document.getElementById("pw-logs-clear")?.addEventListener("click", () => {
      void (async () => {
        const scope = logFilterId
          ? "该任务的扫描日志"
          : "全部扫描日志";
        const ok = await askConfirm("确定清空" + scope + "？清空后不可恢复。", "清空");
        if (!ok) return;
        if (logFilterId) {
          scanLogs = scanLogs.filter((x) => x.watchId !== logFilterId);
        } else {
          scanLogs = [];
        }
        await saveScanLogs();
        renderLogs();
      })();
    });
    document.getElementById("pw-logs-all")?.addEventListener("click", () => {
      openLogsPage(null);
    });
    document.getElementById("pw-settings")?.addEventListener("click", () => {
      openSettingsPage();
    });
    document.getElementById("pw-settings-back")?.addEventListener("click", () => {
      showPage("main");
    });
    document.getElementById("pw-settings-save")?.addEventListener("click", () => {
      void onSaveSettings();
    });
    document.getElementById("pw-settings-reset")?.addEventListener("click", () => {
      setVal("pw-set-title", DEFAULT_PREFS.notifyTitle);
      setVal("pw-set-body", DEFAULT_PREFS.notifyBody);
      updateSettingsPreview();
    });
    ["pw-set-title", "pw-set-body"].forEach((id) => {
      const el = document.getElementById(id);
      if (!el) return;
      el.addEventListener("input", () => updateSettingsPreview());
    });
    document.getElementById("pw-confirm-ok")?.addEventListener("click", () => {
      closeConfirm(true);
    });
    document.getElementById("pw-confirm-cancel")?.addEventListener("click", () => {
      closeConfirm(false);
    });
    document.getElementById("pw-confirm")?.addEventListener("click", (ev) => {
      if (ev.target && ev.target.id === "pw-confirm") closeConfirm(false);
    });

    ["pw-title-input", "pw-url", "pw-selector", "pw-interval"].forEach((id) => {
      const el = document.getElementById(id);
      if (!el) return;
      el.addEventListener("input", () => schedulePersistDraft());
      el.addEventListener("change", () => schedulePersistDraft());
    });

    // Flush draft before popup teardown.
    window.addEventListener("pagehide", () => {
      void persistDraftNow();
    });
    window.addEventListener("beforeunload", () => {
      void persistDraftNow();
    });
  }

  function mountShell() {
    const el = root();
    el.innerHTML =
      '<div class="pw-page" id="pw-page-main">' +
      '<div class="pw-head">' +
      '<h1 class="pw-title">网页监测</h1>' +
      '<div class="pw-actions">' +
      '<button type="button" class="pw-btn" id="pw-settings" data-busy title="通知与其它设置">设置</button>' +
      '<button type="button" class="pw-btn" id="pw-logs-all" data-busy title="查看全部扫描日志">日志</button>' +
      '<button type="button" class="pw-btn primary" id="pw-add" data-busy>添加</button>' +
      "</div></div>" +
      '<div class="pw-form hidden" id="pw-form">' +
      '<div class="pw-form-heading" id="pw-form-heading">新建监测项</div>' +
      '<label class="pw-label">标题</label>' +
      '<input class="pw-input" id="pw-title-input" placeholder="例如：公告栏" />' +
      '<label class="pw-label">网址</label>' +
      '<div class="pw-row">' +
      '<input class="pw-input" id="pw-url" placeholder="https://…" />' +
      '<button type="button" class="pw-btn" id="pw-open" data-busy>打开</button>' +
      "</div>" +
      '<label class="pw-label">选择器（可手改；空则整页 body）</label>' +
      '<div class="pw-row">' +
      '<input class="pw-input" id="pw-selector" placeholder="CSS 选择器，或点整页" />' +
      '<button type="button" class="pw-btn" id="pw-pick" data-busy disabled>划定</button>' +
      '<button type="button" class="pw-btn" id="pw-whole" data-busy>整页</button>' +
      "</div>" +
      '<label class="pw-label">预览文本</label>' +
      '<input class="pw-input" id="pw-preview" readonly />' +
      '<label class="pw-label">间隔（秒，最少 30）</label>' +
      '<input class="pw-input" id="pw-interval" type="number" min="30" step="10" value="60" />' +
      '<div class="pw-row">' +
      '<button type="button" class="pw-btn primary" id="pw-save" data-busy>保存监测</button>' +
      '<button type="button" class="pw-btn" id="pw-cancel">取消</button>' +
      "</div></div>" +
      '<p class="pw-hint" id="pw-hint"></p>' +
      '<div class="pw-list" id="pw-list"></div>' +
      "</div>" +
      '<div class="pw-page pw-page-logs hidden" id="pw-page-logs">' +
      '<div class="pw-head">' +
      '<div class="pw-head-nav">' +
      '<button type="button" class="pw-btn" id="pw-logs-back" aria-label="返回">← 返回</button>' +
      '<h1 class="pw-title" id="pw-logs-title">扫描日志</h1>' +
      "</div>" +
      '<div class="pw-actions">' +
      '<button type="button" class="pw-btn danger" id="pw-logs-clear">清空</button>' +
      "</div></div>" +
      '<div class="pw-logs" id="pw-logs"></div>' +
      "</div>" +
      '<div class="pw-page pw-page-settings hidden" id="pw-page-settings">' +
      '<div class="pw-head">' +
      '<div class="pw-head-nav">' +
      '<button type="button" class="pw-btn" id="pw-settings-back" aria-label="返回">← 返回</button>' +
      '<h1 class="pw-title">设置</h1>' +
      "</div></div>" +
      '<div class="pw-settings-body">' +
      '<div class="pw-form-heading">通知文案</div>' +
      '<p class="pw-settings-help">可用变量：{title}（名称）、{text}（变化文本）、{url}、{selector}。常量直接写在模板里，例如 <code>{title}更新了</code>。</p>' +
      '<label class="pw-label">标题模板</label>' +
      '<input class="pw-input" id="pw-set-title" placeholder="{title}" />' +
      '<label class="pw-label">正文模板</label>' +
      '<input class="pw-input" id="pw-set-body" placeholder="{title}更新了" />' +
      '<p class="pw-set-preview" id="pw-set-preview"></p>' +
      '<div class="pw-row">' +
      '<button type="button" class="pw-btn primary" id="pw-settings-save">保存</button>' +
      '<button type="button" class="pw-btn" id="pw-settings-reset">恢复默认</button>' +
      "</div>" +
      '<p class="pw-settings-help">变化通知：点横幅中部或「打开」都会打开对应网页（需 Host 支持 defaultActionId）。</p>' +
      "</div></div>" +
      '<div class="pw-confirm hidden" id="pw-confirm" role="dialog" aria-modal="true">' +
      '<div class="pw-confirm-card">' +
      '<p class="pw-confirm-msg" id="pw-confirm-msg"></p>' +
      '<div class="pw-row pw-confirm-actions">' +
      '<button type="button" class="pw-btn danger" id="pw-confirm-ok">确定</button>' +
      '<button type="button" class="pw-btn" id="pw-confirm-cancel">取消</button>' +
      "</div></div></div>";
    bindActions();
  }

  async function boot() {
    mountShell();
    await loadItems();
    await loadScanLogs();
    await loadPrefs();
    const sec = Math.round((await defaultIntervalMs()) / 1000);
    setVal("pw-interval", String(sec));

    let restored = false;
    try {
      const draft = await hub().storage.get(DRAFT_KEY);
      if (draftHasContent(draft)) {
        restored = applyDraft(draft);
      }
    } catch (_) {}

    if (!restored) {
      setVal("pw-interval", String(sec));
    }

    renderList();
    showPage("main");
    await syncWatches();
    updateBadge();

    // Apply pick that finished while popup was closed / recreating.
    try {
      const last = await hub().webview.takeLastPick();
      if (last) applyPickToForm(last);
    } catch (_) {}

    if (hub().webview && hub().webview.onPick) {
      hub().webview.onPick((ev) => {
        if (!ev || ev.cancelled) return;
        applyPickToForm(ev);
        void persistDraftNow();
      });
    }

    if (hub().webview && hub().webview.onScanned) {
      hub().webview.onScanned((ev) => {
        void appendScanLog(ev);
      });
    }

    // Island notify is owned by shortcuts.js (always-on). Popup must not also
    // hub.notify on onChanged — that raced the strip and stacked N identical banners.
    if (hub().notify && typeof hub().notify.onAction === "function") {
      hub().notify.onAction((ev) => {
        void handleNotifyAction(ev);
      });
    }
  }

  async function handleNotifyAction(ev) {
    if (!ev || ev.actionId !== "open") return;
    const data = ev.data || {};
    let url = data.url ? String(data.url) : "";
    let title = data.title ? String(data.title) : "网页监测";
    if (!url && data.watchId) {
      const it = items.find((x) => x.id === data.watchId);
      if (it) {
        url = it.url;
        title = it.title || title;
      }
    }
    if (!url) return;
    try {
      await hub().webview.open({ url: url, title: title });
    } catch (_) {}
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void boot());
  } else {
    void boot();
  }
})();

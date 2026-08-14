/**
 * 中转站 — 弹窗（挂 #app.wg-shell）
 * 添加文件 / 拖入 / 多选批量操作 / 右键菜单
 *
 * 文件拖入：WebView2 上 HTML5 File.path 常为空，路径入库由 Host PluginPopupHost
 * 的 onDragDropEvent 负责；本页只做视觉反馈，靠 staging.subscribe 刷新，避免双通道重复入库卡顿。
 * HTML5 drop 仅可靠处理 text/plain 与无 path 的图片 bytes。
 */
(function () {
  const KIND_FALLBACK = { file: "文件", text: "文字", image: "图片", folder: "文件夹" };
  const DRAG_THRESHOLD = 6;
  const THUMB_CONCURRENCY = 3;
  const thumbCache = Object.create(null);

  let rail = null;
  let emptyEl = null;
  let dropZone = null;
  let copyAllBtn = null;
  let addBtn = null;
  let footerEl = null;
  let selBar = null;
  let selCountEl = null;
  let ctxMenu = null;
  let ctxIds = [];
  let disposed = false;
  let itemsCache = [];
  let refreshTimer = null;
  let refreshSeq = 0;
  /** @type {Set<string>} */
  let selected = new Set();
  let lastAnchorId = null;
  const disposers = [];
  const thumbQueue = [];
  let thumbActive = 0;

  function onDispose() {
    if (disposed) return;
    disposed = true;
    if (refreshTimer) {
      window.clearTimeout(refreshTimer);
      refreshTimer = null;
    }
    while (disposers.length) {
      const fn = disposers.pop();
      try {
        fn?.();
      } catch (_) {
        /* ignore */
      }
    }
  }

  window.addEventListener("wh-plugin-popup-dispose", onDispose, { once: true });

  function hub() {
    if (disposed) return null;
    return window.hub;
  }

  function truncName(name) {
    const s = String(name || "");
    if (s.length <= 18) return s;
    return s.slice(0, 16) + "…";
  }

  function fallbackEl(kind) {
    const el = document.createElement("span");
    el.className = "ts-thumb-fallback" + (kind === "text" ? " is-text" : "");
    el.textContent = KIND_FALLBACK[kind] || "文件";
    return el;
  }

  function hideCtx() {
    if (!ctxMenu) return;
    ctxMenu.hidden = true;
    ctxIds = [];
  }

  function showCtx(x, y, ids) {
    if (!ctxMenu) return;
    ctxIds = ids.slice();
    ctxMenu.hidden = false;
    const pad = 8;
    const rect = ctxMenu.getBoundingClientRect();
    const maxX = window.innerWidth - rect.width - pad;
    const maxY = window.innerHeight - rect.height - pad;
    ctxMenu.style.left = Math.max(pad, Math.min(x, maxX)) + "px";
    ctxMenu.style.top = Math.max(pad, Math.min(y, maxY)) + "px";
  }

  function selectedIds() {
    return Array.from(selected);
  }

  function syncSelectionUi() {
    if (!rail) return;
    rail.querySelectorAll(".ts-card").forEach(function (card) {
      const id = card.dataset.id;
      card.classList.toggle("is-selected", !!(id && selected.has(id)));
    });
    const n = selected.size;
    if (selBar) {
      selBar.hidden = n === 0;
      if (selCountEl) selCountEl.textContent = "已选 " + n + " 项";
    }
    if (footerEl && n === 0) {
      /* footer text set by refresh */
    }
  }

  function clearSelection() {
    selected.clear();
    lastAnchorId = null;
    syncSelectionUi();
  }

  function selectOnly(id) {
    selected.clear();
    selected.add(id);
    lastAnchorId = id;
    syncSelectionUi();
  }

  function toggleSelect(id) {
    if (selected.has(id)) selected.delete(id);
    else selected.add(id);
    lastAnchorId = id;
    syncSelectionUi();
  }

  function selectRange(toId) {
    const ids = itemsCache.map(function (it) {
      return it.id;
    });
    const anchor = lastAnchorId && ids.indexOf(lastAnchorId) >= 0 ? lastAnchorId : toId;
    const a = ids.indexOf(anchor);
    const b = ids.indexOf(toId);
    if (a < 0 || b < 0) {
      selectOnly(toId);
      return;
    }
    const lo = Math.min(a, b);
    const hi = Math.max(a, b);
    selected.clear();
    for (let i = lo; i <= hi; i++) selected.add(ids[i]);
    lastAnchorId = anchor;
    syncSelectionUi();
  }

  function pumpThumbs() {
    while (thumbActive < THUMB_CONCURRENCY && thumbQueue.length) {
      const job = thumbQueue.shift();
      if (!job) break;
      thumbActive += 1;
      Promise.resolve()
        .then(job)
        .catch(function () {
          /* ignore */
        })
        .finally(function () {
          thumbActive -= 1;
          pumpThumbs();
        });
    }
  }

  function enqueueThumb(fn) {
    thumbQueue.push(fn);
    pumpThumbs();
  }

  function bindOsDrag(card, h, it) {
    if (!h.staging.startDrag || !it.path) return;
    card.addEventListener("pointerdown", function (e) {
      if (e.button !== 0) return;
      const startX = e.clientX;
      const startY = e.clientY;
      let started = false;

      function onMove(ev) {
        if (started) return;
        const dx = ev.clientX - startX;
        const dy = ev.clientY - startY;
        if (Math.hypot(dx, dy) < DRAG_THRESHOLD) return;
        started = true;
        cleanup();
        card.dataset.didDrag = "1";
        card.classList.add("is-dragging");
        let ids = [it.id];
        if (selected.has(it.id) && selected.size > 1) {
          ids = selectedIds().filter(function (id) {
            const item = itemsCache.find(function (x) {
              return x.id === id;
            });
            return item && item.path;
          });
          if (!ids.length) ids = [it.id];
        }
        h.staging
          .startDrag(ids)
          .catch(console.error)
          .finally(function () {
            card.classList.remove("is-dragging");
          });
      }

      function cleanup() {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", cleanup);
        window.removeEventListener("pointercancel", cleanup);
      }

      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", cleanup);
      window.addEventListener("pointercancel", cleanup);
    });
  }

  function makeCard(h, it) {
    const card = document.createElement("button");
    card.type = "button";
    card.className = "ts-card";
    card.dataset.id = it.id;
    if (selected.has(it.id)) card.classList.add("is-selected");
    card.title =
      (it.path || it.label || "") +
      "\n单击选择 · Ctrl 多选 · Shift 连选\n右键批量操作 · 拖出到文件夹";

    const name = document.createElement("span");
    name.className = "ts-name";
    name.textContent = truncName(it.label || it.id);

    const useThumb =
      it.kind === "image" ||
      ((it.kind === "file" || it.kind === "folder") && it.path && h.staging.thumb);

    if (useThumb) {
      const img = document.createElement("img");
      img.className = "ts-thumb" + (it.kind === "image" ? "" : " is-icon");
      img.alt = "";
      img.draggable = false;
      const cached = thumbCache[it.id];
      if (cached) {
        img.src = cached;
      } else if (h.staging.thumb) {
        const itemId = it.id;
        const kind = it.kind;
        enqueueThumb(function () {
          if (disposed) return;
          return h.staging
            .thumb(itemId)
            .then(function (url) {
              if (!url) {
                if (img.isConnected) img.replaceWith(fallbackEl(kind));
                return;
              }
              thumbCache[itemId] = url;
              if (img.isConnected) img.src = url;
            })
            .catch(function () {
              if (img.isConnected) img.replaceWith(fallbackEl(kind));
            });
        });
      } else {
        img.replaceWith(fallbackEl(it.kind));
      }
      card.appendChild(img);
    } else {
      card.appendChild(fallbackEl(it.kind));
    }

    card.appendChild(name);
    bindOsDrag(card, h, it);

    card.addEventListener("click", function (e) {
      if (card.dataset.didDrag) {
        delete card.dataset.didDrag;
        e.preventDefault();
        return;
      }
      if (e.shiftKey) {
        selectRange(it.id);
        return;
      }
      if (e.ctrlKey || e.metaKey) {
        toggleSelect(it.id);
        return;
      }
      selectOnly(it.id);
    });

    card.addEventListener("contextmenu", function (e) {
      e.preventDefault();
      e.stopPropagation();
      if (!selected.has(it.id)) {
        selectOnly(it.id);
      }
      showCtx(e.clientX, e.clientY, selectedIds());
    });

    card.addEventListener("dblclick", function (e) {
      e.preventDefault();
      e.stopPropagation();
      if (!it.path) return;
      if (it.kind === "file" || it.kind === "image" || it.kind === "folder") {
        if (h.staging.open) {
          h.staging.open(it.id).catch(console.error);
        } else {
          h.staging.reveal(it.id).catch(console.error);
        }
      }
    });

    return card;
  }

  function scheduleRefresh() {
    if (disposed) return;
    if (refreshTimer) window.clearTimeout(refreshTimer);
    refreshTimer = window.setTimeout(function () {
      refreshTimer = null;
      void refresh();
    }, 40);
  }

  async function refresh() {
    const h = hub();
    if (!h || !h.staging || !rail) return;
    const seq = ++refreshSeq;
    const items = await h.staging.list();
    if (disposed || seq !== refreshSeq) return;

    itemsCache = items || [];
    const alive = new Set(
      itemsCache.map(function (it) {
        return it.id;
      }),
    );
    Array.from(selected).forEach(function (id) {
      if (!alive.has(id)) selected.delete(id);
    });

    // Clear pending thumb jobs for removed cards; keep in-flight ones.
    thumbQueue.length = 0;

    rail.querySelectorAll(".ts-card").forEach(function (n) {
      n.remove();
    });
    if (!itemsCache.length) {
      emptyEl.hidden = false;
      copyAllBtn.disabled = true;
      clearSelection();
      if (footerEl) footerEl.textContent = "暂无内容";
      return;
    }
    emptyEl.hidden = true;
    copyAllBtn.disabled = !itemsCache.some(function (it) {
      return it.kind === "file" || it.kind === "image" || it.kind === "folder";
    });
    const frag = document.createDocumentFragment();
    for (const it of itemsCache) {
      frag.appendChild(makeCard(h, it));
    }
    rail.appendChild(frag);
    syncSelectionUi();
    if (footerEl) {
      const files = itemsCache.filter(function (it) {
        return it.kind === "file";
      }).length;
      const folders = itemsCache.filter(function (it) {
        return it.kind === "folder";
      }).length;
      const texts = itemsCache.filter(function (it) {
        return it.kind === "text";
      }).length;
      const images = itemsCache.filter(function (it) {
        return it.kind === "image";
      }).length;
      const parts = [];
      if (files) parts.push("文件 " + files);
      if (folders) parts.push("文件夹 " + folders);
      if (texts) parts.push("文字 " + texts);
      if (images) parts.push("图片 " + images);
      footerEl.textContent = parts.join(" · ") || itemsCache.length + " 项";
    }
  }

  async function ingestDataTransfer(dt) {
    const h = hub();
    if (!h || !h.staging || !dt) return;
    const text = dt.getData("text/plain");
    if (text && text.trim() && !dt.files?.length) {
      await h.staging.addText(text).catch(console.error);
    }
    const files = dt.files;
    if (!files || !files.length) return;
    const paths = [];
    for (let i = 0; i < files.length; i++) {
      const f = files.item(i);
      if (!f) continue;
      const path = f.path;
      if (typeof path === "string" && path.trim()) {
        paths.push(path);
        continue;
      }
      if (f.type && f.type.startsWith("image/")) {
        try {
          const buf = new Uint8Array(await f.arrayBuffer());
          const ext = (f.name.split(".").pop() || "png").toLowerCase();
          await h.staging.addImageBytes(f.name || "图片", Array.from(buf), ext);
        } catch (err) {
          console.error(err);
        }
      }
    }
    // 有 path 时交给 Host Tauri 通道；此处仅在 Host 未接到时兜底（极少）
    if (paths.length && h.staging.addPaths) {
      await h.staging.addPaths(paths).catch(console.error);
    }
  }

  function bindDrop() {
    if (!dropZone) return;
    dropZone.addEventListener("dragenter", function (e) {
      e.preventDefault();
      dropZone.classList.add("is-over");
    });
    dropZone.addEventListener("dragover", function (e) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      dropZone.classList.add("is-over");
    });
    dropZone.addEventListener("dragleave", function (e) {
      if (e.relatedTarget && dropZone.contains(e.relatedTarget)) return;
      dropZone.classList.remove("is-over");
    });
    dropZone.addEventListener("drop", function (e) {
      e.preventDefault();
      dropZone.classList.remove("is-over");
      const dt = e.dataTransfer;
      // 系统文件（含空 path 的 File 列表）一律交给 Host Tauri，避免双写/读大图卡顿
      if (dt && dt.files && dt.files.length) return;
      void ingestDataTransfer(dt).then(scheduleRefresh).catch(console.error);
    });
  }

  /** 仅视觉反馈；入库由 Host 负责，避免与 PluginPopupHost 双写卡顿 */
  function bindTauriFileDrop() {
    const api =
      window.__TAURI__ &&
      (window.__TAURI__.webview || window.__TAURI__.window);
    if (!api) return;
    const getCurrent =
      (window.__TAURI__.webview && window.__TAURI__.webview.getCurrentWebview) ||
      (window.__TAURI__.window && window.__TAURI__.window.getCurrentWindow);
    if (typeof getCurrent !== "function") return;
    let target;
    try {
      target = getCurrent();
    } catch (_) {
      return;
    }
    if (!target || typeof target.onDragDropEvent !== "function") return;
    void target
      .onDragDropEvent(function (ev) {
        if (disposed) return;
        const p = ev && ev.payload;
        if (!p) return;
        if (p.type === "enter" || p.type === "over") {
          if (dropZone) dropZone.classList.add("is-over");
          return;
        }
        if (p.type === "leave" || p.type === "cancel") {
          if (dropZone) dropZone.classList.remove("is-over");
          return;
        }
        if (p.type !== "drop") return;
        if (dropZone) dropZone.classList.remove("is-over");
        if (footerEl) footerEl.textContent = "正在导入…";
        // Host 已 addPaths；subscribe 会 refresh。此处不重复调用。
      })
      .then(function (un) {
        if (typeof un === "function") {
          if (disposed) un();
          else disposers.push(un);
        }
      })
      .catch(console.error);
  }

  function actCopyPaths(ids) {
    const h = hub();
    if (!h || !ids.length) return;
    if (ids.length === 1) {
      h.staging.copy(ids[0]).catch(console.error);
      return;
    }
    if (h.staging.copyPaths) {
      h.staging.copyPaths(ids).catch(console.error);
    } else {
      ids.forEach(function (id) {
        h.staging.copy(id).catch(console.error);
      });
    }
  }

  function actCopyFiles(ids) {
    const h = hub();
    if (!h || !ids.length) return;
    if (h.staging.copyFiles) {
      h.staging.copyFiles(ids).catch(console.error);
    } else {
      actCopyPaths(ids);
    }
  }

  function actRemove(ids) {
    const h = hub();
    if (!h || !ids.length) return;
    h.staging
      .remove(ids)
      .then(function () {
        ids.forEach(function (id) {
          selected.delete(id);
          delete thumbCache[id];
        });
        scheduleRefresh();
      })
      .catch(console.error);
  }

  function mount() {
    const app = document.getElementById("app");
    if (!app) return;
    app.innerHTML =
      '<header class="ts-header">' +
      '<div><div class="ts-kicker">STAGING</div><div class="ts-title">中转站</div></div>' +
      '<div class="ts-head-actions">' +
      '<button type="button" class="ts-btn ts-btn-primary" id="btn-add">添加</button>' +
      '<button type="button" class="ts-btn" id="btn-copy-all" disabled>复制路径</button>' +
      '<button type="button" class="ts-more" id="btn-more" aria-label="更多" aria-expanded="false">···</button>' +
      '<div class="ts-menu" id="menu" hidden>' +
      '<button type="button" id="menu-add-folder">添加文件夹</button>' +
      '<button type="button" id="menu-clear">清空全部</button></div>' +
      "</div></header>" +
      '<div class="ts-selbar" id="selbar" hidden>' +
      '<span class="ts-selcount" id="selcount">已选 0 项</span>' +
      '<div class="ts-sel-actions">' +
      '<button type="button" class="ts-btn" id="btn-sel-copy-path">复制路径</button>' +
      '<button type="button" class="ts-btn" id="btn-sel-copy-file">复制文件</button>' +
      '<button type="button" class="ts-btn ts-btn-danger" id="btn-sel-remove">移除</button>' +
      '<button type="button" class="ts-btn" id="btn-sel-clear">取消选择</button>' +
      "</div></div>" +
      '<div class="ts-drop" id="drop">' +
      '<p class="ts-hint" id="empty">拖入文件、文件夹、文字或图片<br/>或点「添加」</p>' +
      '<div class="ts-rail" id="rail" role="list"></div>' +
      "</div>" +
      '<footer class="ts-footer" id="footer">暂无内容</footer>' +
      '<div class="ts-ctx" id="ctx" hidden>' +
      '<button type="button" data-act="open">打开</button>' +
      '<button type="button" data-act="copy-path">复制路径</button>' +
      '<button type="button" data-act="copy-file">复制到剪贴板</button>' +
      '<button type="button" data-act="reveal">打开位置</button>' +
      '<button type="button" data-act="remove" class="is-danger">从列表删除</button>' +
      "</div>";

    rail = document.getElementById("rail");
    emptyEl = document.getElementById("empty");
    dropZone = document.getElementById("drop");
    copyAllBtn = document.getElementById("btn-copy-all");
    addBtn = document.getElementById("btn-add");
    footerEl = document.getElementById("footer");
    selBar = document.getElementById("selbar");
    selCountEl = document.getElementById("selcount");
    ctxMenu = document.getElementById("ctx");
    const moreBtn = document.getElementById("btn-more");
    const menu = document.getElementById("menu");
    const clearBtn = document.getElementById("menu-clear");
    const addFolderBtn = document.getElementById("menu-add-folder");

    function closeMenu() {
      menu.hidden = true;
      moreBtn.setAttribute("aria-expanded", "false");
    }

    moreBtn.addEventListener("click", function (e) {
      e.stopPropagation();
      hideCtx();
      const open = menu.hidden;
      menu.hidden = !open;
      moreBtn.setAttribute("aria-expanded", open ? "true" : "false");
    });
    document.addEventListener("click", function (e) {
      closeMenu();
      hideCtx();
      const t = e.target;
      if (t && rail && rail.contains(t)) return;
      if (t && selBar && selBar.contains(t)) return;
      if (t && ctxMenu && ctxMenu.contains(t)) return;
      // 点空白处取消选择
      if (selected.size) clearSelection();
    });
    menu.addEventListener("click", function (e) {
      e.stopPropagation();
    });
    ctxMenu.addEventListener("click", function (e) {
      e.stopPropagation();
    });
    if (addFolderBtn) {
      addFolderBtn.addEventListener("click", function () {
        closeMenu();
        const h = hub();
        if (!h || !h.staging.pickFolders) return;
        h.staging
          .pickFolders()
          .then(function () {
            scheduleRefresh();
          })
          .catch(console.error);
      });
    }
    clearBtn.addEventListener("click", function () {
      closeMenu();
      const h = hub();
      if (!h) return;
      h.staging
        .clear()
        .then(function () {
          selected.clear();
          Object.keys(thumbCache).forEach(function (k) {
            delete thumbCache[k];
          });
          scheduleRefresh();
        })
        .catch(console.error);
    });
    copyAllBtn.addEventListener("click", function () {
      const h = hub();
      if (!h || !h.staging.copyAllPaths) return;
      h.staging.copyAllPaths().catch(console.error);
    });
    addBtn.addEventListener("click", function () {
      const h = hub();
      if (!h || !h.staging.pickFiles) {
        console.error("[transfer] pickFiles unavailable");
        return;
      }
      h.staging
        .pickFiles()
        .then(function () {
          scheduleRefresh();
        })
        .catch(console.error);
    });

    document.getElementById("btn-sel-copy-path").addEventListener("click", function (e) {
      e.stopPropagation();
      actCopyPaths(selectedIds());
    });
    document.getElementById("btn-sel-copy-file").addEventListener("click", function (e) {
      e.stopPropagation();
      actCopyFiles(selectedIds());
    });
    document.getElementById("btn-sel-remove").addEventListener("click", function (e) {
      e.stopPropagation();
      actRemove(selectedIds());
    });
    document.getElementById("btn-sel-clear").addEventListener("click", function (e) {
      e.stopPropagation();
      clearSelection();
    });

    ctxMenu.querySelectorAll("button[data-act]").forEach(function (btn) {
      btn.addEventListener("click", function () {
        const act = btn.getAttribute("data-act");
        const ids = ctxIds.slice();
        hideCtx();
        const h = hub();
        if (!h || !ids.length) return;
        if (act === "open") {
          const id = ids[0];
          if (h.staging.open) {
            h.staging.open(id).catch(console.error);
          } else {
            h.staging.reveal(id).catch(console.error);
          }
        } else if (act === "copy-path") {
          actCopyPaths(ids);
        } else if (act === "copy-file") {
          actCopyFiles(ids);
        } else if (act === "reveal") {
          h.staging.reveal(ids[0]).catch(console.error);
        } else if (act === "remove") {
          actRemove(ids);
        }
      });
    });

    document.addEventListener("keydown", function (e) {
      if (disposed) return;
      const tag = (e.target && e.target.tagName) || "";
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      if ((e.key === "a" || e.key === "A") && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        selected.clear();
        itemsCache.forEach(function (it) {
          selected.add(it.id);
        });
        if (itemsCache.length) lastAnchorId = itemsCache[0].id;
        syncSelectionUi();
        return;
      }
      if (e.key === "Escape") {
        if (selected.size) {
          e.preventDefault();
          clearSelection();
        }
        return;
      }
      if ((e.key === "Delete" || e.key === "Backspace") && selected.size) {
        e.preventDefault();
        actRemove(selectedIds());
      }
    });

    bindDrop();
    bindTauriFileDrop();
  }

  function boot(attempt) {
    if (disposed) return;
    const h = hub();
    if (!h || !h.staging) {
      if ((attempt || 0) < 30) {
        requestAnimationFrame(function () {
          boot((attempt || 0) + 1);
        });
      }
      return;
    }
    mount();
    if (h.staging.subscribe) {
      const un = h.staging.subscribe(function () {
        if (!disposed) scheduleRefresh();
      });
      if (typeof un === "function") disposers.push(un);
    } else {
      scheduleRefresh();
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }
})();

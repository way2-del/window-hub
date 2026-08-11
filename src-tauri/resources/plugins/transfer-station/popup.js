/**
 * 中转站 — 弹窗（挂 #app.wg-shell）
 * 添加文件 / 拖入 / 右键菜单 / 复制路径·文件
 */
(function () {
  const KIND_FALLBACK = { file: "文件", text: "文字", image: "图片", folder: "文件夹" };
  const DRAG_THRESHOLD = 6;
  const thumbCache = Object.create(null);

  let rail = null;
  let emptyEl = null;
  let dropZone = null;
  let copyAllBtn = null;
  let addBtn = null;
  let footerEl = null;
  let ctxMenu = null;
  let ctxItemId = null;

  function hub() {
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
    ctxItemId = null;
  }

  function showCtx(x, y, itemId) {
    if (!ctxMenu) return;
    ctxItemId = itemId;
    ctxMenu.hidden = false;
    const pad = 8;
    const rect = ctxMenu.getBoundingClientRect();
    const maxX = window.innerWidth - rect.width - pad;
    const maxY = window.innerHeight - rect.height - pad;
    ctxMenu.style.left = Math.max(pad, Math.min(x, maxX)) + "px";
    ctxMenu.style.top = Math.max(pad, Math.min(y, maxY)) + "px";
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
        h.staging
          .startDrag([it.id])
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
    card.title = (it.path || it.label || "") + "\n右键更多操作 · 拖出到文件夹";

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
        h.staging
          .thumb(it.id)
          .then(function (url) {
            if (!url) {
              img.replaceWith(fallbackEl(it.kind));
              return;
            }
            thumbCache[it.id] = url;
            img.src = url;
          })
          .catch(function () {
            img.replaceWith(fallbackEl(it.kind));
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
      h.staging.copy(it.id).catch(console.error);
      card.classList.add("is-selected");
      window.setTimeout(function () {
        card.classList.remove("is-selected");
      }, 450);
    });

    card.addEventListener("contextmenu", function (e) {
      e.preventDefault();
      e.stopPropagation();
      showCtx(e.clientX, e.clientY, it.id);
    });

    card.addEventListener("dblclick", function (e) {
      e.preventDefault();
      if (it.kind === "file" || it.kind === "image" || it.kind === "folder") {
        h.staging.reveal(it.id).catch(console.error);
      }
    });

    card.addEventListener("keydown", function (e) {
      if (e.key === "Delete" || e.key === "Backspace") {
        e.preventDefault();
        h.staging.remove(it.id).then(refresh).catch(console.error);
      }
    });

    return card;
  }

  async function refresh() {
    const h = hub();
    if (!h || !h.staging || !rail) return;
    const items = await h.staging.list();
    rail.querySelectorAll(".ts-card").forEach(function (n) {
      n.remove();
    });
    if (!items.length) {
      emptyEl.hidden = false;
      copyAllBtn.disabled = true;
      if (footerEl) footerEl.textContent = "暂无内容";
      return;
    }
    emptyEl.hidden = true;
    copyAllBtn.disabled = !items.some(function (it) {
      return it.kind === "file" || it.kind === "image" || it.kind === "folder";
    });
    for (const it of items) {
      rail.appendChild(makeCard(h, it));
    }
    if (footerEl) {
      const files = items.filter(function (it) {
        return it.kind === "file";
      }).length;
      const folders = items.filter(function (it) {
        return it.kind === "folder";
      }).length;
      const texts = items.filter(function (it) {
        return it.kind === "text";
      }).length;
      const images = items.filter(function (it) {
        return it.kind === "image";
      }).length;
      const parts = [];
      if (files) parts.push("文件 " + files);
      if (folders) parts.push("文件夹 " + folders);
      if (texts) parts.push("文字 " + texts);
      if (images) parts.push("图片 " + images);
      footerEl.textContent = parts.join(" · ") || items.length + " 项";
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
      void ingestDataTransfer(e.dataTransfer).then(refresh).catch(console.error);
    });
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
      '<div class="ts-drop" id="drop">' +
      '<p class="ts-hint" id="empty">拖入文件、文件夹、文字或图片<br/>或点「添加」</p>' +
      '<div class="ts-rail" id="rail" role="list"></div>' +
      "</div>" +
      '<footer class="ts-footer" id="footer">暂无内容</footer>' +
      '<div class="ts-ctx" id="ctx" hidden>' +
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
    document.addEventListener("click", function () {
      closeMenu();
      hideCtx();
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
            return refresh();
          })
          .catch(console.error);
      });
    }
    clearBtn.addEventListener("click", function () {
      closeMenu();
      const h = hub();
      if (!h) return;
      h.staging.clear().then(refresh).catch(console.error);
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
          return refresh();
        })
        .catch(console.error);
    });

    ctxMenu.querySelectorAll("button[data-act]").forEach(function (btn) {
      btn.addEventListener("click", function () {
        const act = btn.getAttribute("data-act");
        const id = ctxItemId;
        hideCtx();
        const h = hub();
        if (!h || !id) return;
        if (act === "copy-path") {
          h.staging.copy(id).catch(console.error);
        } else if (act === "copy-file") {
          if (h.staging.copyFiles) {
            h.staging.copyFiles(id).catch(console.error);
          } else {
            h.staging.copy(id).catch(console.error);
          }
        } else if (act === "reveal") {
          h.staging.reveal(id).catch(console.error);
        } else if (act === "remove") {
          h.staging.remove(id).then(refresh).catch(console.error);
        }
      });
    });

    bindDrop();
  }

  function boot(attempt) {
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
      h.staging.subscribe(function () {
        refresh();
      });
    } else {
      refresh();
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", boot);
  } else {
    boot();
  }
})();

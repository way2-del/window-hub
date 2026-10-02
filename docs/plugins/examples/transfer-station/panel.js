(function () {
  const rail = document.getElementById("rail");
  const emptyEl = document.getElementById("empty");
  const copyAllBtn = document.getElementById("btn-copy-all");
  const moreBtn = document.getElementById("btn-more");
  const menu = document.getElementById("menu");
  const clearBtn = document.getElementById("menu-clear");

  const KIND_FALLBACK = { file: "文件", text: "文字", image: "图片", link: "链接" };
  const DRAG_THRESHOLD = 6;
  const thumbCache = Object.create(null);

  function hub() {
    return window.hub;
  }

  function closeMenu() {
    menu.hidden = true;
    moreBtn.setAttribute("aria-expanded", "false");
  }

  moreBtn.addEventListener("click", function (e) {
    e.stopPropagation();
    const open = menu.hidden;
    menu.hidden = !open;
    moreBtn.setAttribute("aria-expanded", open ? "true" : "false");
  });

  document.addEventListener("click", function () {
    closeMenu();
  });
  menu.addEventListener("click", function (e) {
    e.stopPropagation();
  });

  clearBtn.addEventListener("click", function () {
    closeMenu();
    const h = hub();
    if (!h) return;
    h.staging.clear().then(refresh).catch(console.error);
  });

  copyAllBtn.addEventListener("click", function () {
    const h = hub();
    if (!h || !h.staging.copyAllPaths) return;
    h.staging.copyAllPaths().catch(function (err) {
      console.error(err);
    });
  });

  function truncName(name) {
    const s = String(name || "");
    if (s.length <= 18) return s;
    return s.slice(0, 16) + "…";
  }

  function isLink(it) {
    return it && it.kind === "link";
  }

  function cardTitle(it) {
    if (isLink(it)) {
      return (it.label || "链接") + "\n单击复制 · 双击打开链接 · Delete 删除";
    }
    if (it.kind === "text") {
      return (it.label || "文字") + "\n单击复制 · 双击打开 · Delete 删除";
    }
    return (
      (it.path || it.label || "") +
      "\n拖出到文件夹 / 网页上传 · 单击复制 · 双击打开 · Delete 删除"
    );
  }

  function bindOsDrag(card, h, it) {
    if (!h.staging.startDrag || !it.path) return;
    if (it.kind === "text" || it.kind === "link") return;
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
    card.className = "ts-card" + (isLink(it) ? " is-link" : "");
    card.setAttribute("role", "listitem");
    card.title = cardTitle(it);

    const name = document.createElement("span");
    name.className = "ts-name";
    name.textContent = truncName(it.label || it.id);

    if (it.kind === "image") {
      const img = document.createElement("img");
      img.className = "ts-thumb";
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

    card.addEventListener("dblclick", function (e) {
      e.preventDefault();
      if (h.staging.open) {
        h.staging.open(it.id).catch(console.error);
        return;
      }
      if (it.kind === "file" || it.kind === "image") {
        h.staging.reveal(it.id).catch(console.error);
      } else if (isLink(it) && h.staging.reveal) {
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

  function fallbackEl(kind) {
    const el = document.createElement("span");
    let cls = "ts-thumb-fallback";
    if (kind === "text") cls += " is-text";
    if (kind === "link") cls += " is-link";
    el.className = cls;
    el.textContent = KIND_FALLBACK[kind] || "文件";
    return el;
  }

  async function refresh() {
    const h = hub();
    if (!h || !h.staging) return;
    const items = await h.staging.list();
    rail.querySelectorAll(".ts-card").forEach(function (n) {
      n.remove();
    });
    if (!items.length) {
      emptyEl.hidden = false;
      copyAllBtn.disabled = true;
      return;
    }
    emptyEl.hidden = true;
    copyAllBtn.disabled = !items.some(function (it) {
      return it.kind === "file" || it.kind === "image";
    });
    for (const it of items) {
      rail.appendChild(makeCard(h, it));
    }
  }

  function syncBar(summary) {
    const h = hub();
    if (!h || !h.island) return;
    if (!summary || !summary.total) {
      h.island.clearBar().catch(function () {});
      return;
    }
    const parts = ["中转站"];
    if (summary.files > 0) parts.push("文件 " + summary.files);
    if (summary.texts > 0) parts.push("文字片段 " + summary.texts);
    if (summary.links > 0) parts.push("链接 " + summary.links);
    if (summary.images > 0) parts.push("图片 " + summary.images);
    h.island.setBar({ text: parts.join(" | "), title: "中转站" }).catch(function () {});
  }

  function boot() {
    const h = hub();
    if (!h || !h.staging) {
      setTimeout(boot, 40);
      return;
    }
    if (h.staging.subscribe) {
      h.staging.subscribe(function (summary) {
        refresh();
        syncBar(summary);
      });
    } else {
      refresh();
    }
  }
  boot();
})();

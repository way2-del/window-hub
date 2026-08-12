/**
 * 中转站 — 快捷区图标：点击打开；拖入在图标上松开入库（Host Tauri 为主）。
 * 禁止 dragenter/over 时开弹窗——新 HWND 插入拖拽会话会出现禁止光标。
 */
(function () {
  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function tsIcon() {
    return (
      '<svg class="ts-chip-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true" focusable="false">' +
      '<path d="M4 7h16v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7z"/>' +
      '<path d="M8 7V5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>' +
      '<path d="M12 11v5"/>' +
      '<path d="M9.5 13.5 12 16l2.5-2.5"/>' +
      "</svg>"
    );
  }

  function paint() {
    const bar = document.getElementById("bar");
    if (!bar) return;
    bar.title = "打开中转站（拖到图标上松开即可入库）";
    bar.innerHTML = tsIcon();
    bar.classList.remove("is-loading");
    bar.classList.add("is-ready");
    bar.removeAttribute("aria-hidden");
    reportWidth();
  }

  function reportWidth() {
    const h = hub();
    const bar = document.getElementById("bar");
    if (!bar || !h.shortcuts || !h.shortcuts.requestSize) return;
    const w = Math.ceil(
      Math.max(bar.scrollWidth, bar.getBoundingClientRect().width, 22),
    );
    void h.shortcuts.requestSize({ width: w });
  }

  function openPopup() {
    const h = hub();
    if (!h.popup || !h.popup.open) return;
    h.popup.open({});
  }

  function bindChip() {
    const bar = document.getElementById("bar");
    if (!bar) return;
    bar.addEventListener("click", function (e) {
      e.preventDefault();
      e.stopPropagation();
      openPopup();
    });
    bar.addEventListener("keydown", function (e) {
      if (e.key !== "Enter" && e.key !== " ") return;
      e.preventDefault();
      openPopup();
    });
    bar.addEventListener("dragenter", function (e) {
      e.preventDefault();
    });
    bar.addEventListener("dragover", function (e) {
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
    });
  }

  async function boot() {
    const h = hub();
    try {
      const b = await h.shortcuts.getBounds();
      if (b && b.height) {
        document.documentElement.style.setProperty("--wh-bar-h", b.height + "px");
      }
    } catch (_) {}
    bindChip();
    paint();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

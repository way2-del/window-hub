/**
 * Excalidraw popup:
 * - Frameless popup: canvas + small enlarge icon → native OS window (closes popup).
 * - Native window: OS title bar + settings-frame Mica (same as 灵动岛设置).
 */
(function () {
  const EX_VER = "0.17.6";
  const REACT_VER = "18.2.0";
  const CDN = "https://cdn.jsdelivr.net/npm";
  const SCENE_KEY = "scene";
  const SAVE_DEBOUNCE_MS = 900;

  const isNative =
    typeof location !== "undefined" &&
    (/(?:\?|&)window=plugin-window(?:&|$)/.test(location.search || "") ||
      /(?:\?|&)nativeFrame=1(?:&|$)/.test(location.search || ""));

  let saveTimer = null;
  let statusEl = null;
  /** Right-edge drag → beyond max width opens maximized native window. */
  const POPUP_W_MIN = 360;
  const POPUP_MAX_W_DEFAULT = 720;

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function setStatus(text) {
    if (statusEl) statusEl.textContent = text || "";
  }

  function detachToWindow(opts) {
    const fullscreen = !!(opts && opts.windowedFullscreen);
    setStatus(fullscreen ? "正在全屏打开…" : "正在打开窗口…");
    const api = hub().popup;
    const run =
      api && typeof api.openAsWindow === "function"
        ? () =>
            api.openAsWindow({
              width: 1120,
              height: 720,
              windowedFullscreen: fullscreen,
            })
        : () => {
            const core = window.__TAURI__ && window.__TAURI__.core;
            if (!core || !core.invoke) return Promise.reject(new Error("no invoke"));
            return core.invoke("schedule_plugin_popup_as_window", {
              pluginId: window.__WH_PLUGIN_ID__ || hub().pluginId,
              width: 1120,
              height: 720,
              windowedFullscreen: fullscreen,
            });
          };
    return run();
  }

  function persistPopupSize(width, height) {
    const w = Math.round(width);
    const h = Math.round(height);
    if (!Number.isFinite(w) || !Number.isFinite(h) || w < 1 || h < 1) return;
    const s = hub().settings;
    if (!s || typeof s.set !== "function") return;
    Promise.all([s.set("popupWidth", w), s.set("popupHeight", h)]).catch((e) =>
      console.warn("[excalidraw] persist size", e),
    );
  }

  function bindRightResize(root) {
    const handle = document.createElement("div");
    handle.className = "ex-resize-e";
    handle.title = "向右拖拽加宽；超过上限将全屏打开";
    root.appendChild(handle);

    const drag = {
      active: false,
      startX: 0,
      startW: 0,
      maxW: POPUP_MAX_W_DEFAULT,
      height: 640,
      raf: 0,
      pendingW: 0,
      detaching: false,
      dirty: false,
    };

    function applyWidth(w) {
      const api = hub().popup;
      if (!api || typeof api.resize !== "function") return;
      api.resize({ width: w, height: drag.height }).catch(() => undefined);
      drag.dirty = true;
    }

    handle.addEventListener("pointerdown", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      drag.active = true;
      drag.detaching = false;
      drag.dirty = false;
      drag.startX = e.screenX;
      drag.startW = Math.round(window.innerWidth);
      drag.height = Math.round(window.innerHeight);
      drag.maxW = POPUP_MAX_W_DEFAULT;
      drag.pendingW = drag.startW;
      handle.classList.add("is-dragging");
      try {
        handle.setPointerCapture(e.pointerId);
      } catch (_) {}
      hub()
        .settings.getAll()
        .then((s) => {
          const n = s && Number(s.popupMaxWidth);
          if (Number.isFinite(n) && n >= POPUP_W_MIN) drag.maxW = Math.round(n);
        })
        .catch(() => undefined);
    });

    handle.addEventListener("pointermove", (e) => {
      if (!drag.active || drag.detaching) return;
      const next = Math.round(drag.startW + (e.screenX - drag.startX));
      if (next > drag.maxW) {
        drag.detaching = true;
        drag.active = false;
        handle.classList.remove("is-dragging");
        try {
          handle.releasePointerCapture(e.pointerId);
        } catch (_) {}
        // Persist last in-range size before jumping to fullscreen window.
        persistPopupSize(drag.maxW, drag.height);
        detachToWindow({ windowedFullscreen: true }).catch((err) => {
          console.error("[excalidraw] resize→fullscreen", err);
          setStatus("全屏失败");
          drag.detaching = false;
        });
        return;
      }
      drag.pendingW = Math.max(POPUP_W_MIN, Math.min(drag.maxW, next));
      if (drag.raf) return;
      drag.raf = window.requestAnimationFrame(() => {
        drag.raf = 0;
        if (!drag.active) return;
        applyWidth(drag.pendingW);
      });
    });

    function endDrag(e) {
      if (!drag.active) return;
      drag.active = false;
      handle.classList.remove("is-dragging");
      try {
        if (e && e.pointerId != null) handle.releasePointerCapture(e.pointerId);
      } catch (_) {}
      if (drag.raf) {
        window.cancelAnimationFrame(drag.raf);
        drag.raf = 0;
      }
      if (!drag.detaching && drag.pendingW > 0) {
        applyWidth(drag.pendingW);
        if (drag.dirty || drag.pendingW !== drag.startW) {
          persistPopupSize(drag.pendingW, drag.height);
          setStatus("已记住大小");
          window.setTimeout(() => setStatus(""), 1200);
        }
      }
    }

    handle.addEventListener("pointerup", endDrag);
    handle.addEventListener("pointercancel", endDrag);
  }

  function mountShell(root) {
    if (isNative) {
      root.className = "wg-shell ex-native";
      root.innerHTML =
        '<div class="ex-canvas" id="ex-canvas">' +
        '<div class="ex-loading">正在加载 Excalidraw…</div></div>' +
        '<div class="ex-toast" id="ex-status" aria-live="polite"></div>';
    } else {
      root.className = "wg-shell ex-popup";
      root.innerHTML =
        '<div class="ex-canvas" id="ex-canvas">' +
        '<div class="ex-loading">正在加载 Excalidraw…</div></div>' +
        '<button type="button" class="ex-detach" id="ex-detach" title="窗口化 · 独立系统窗口">' +
        '<svg viewBox="0 0 20 20" aria-hidden="true">' +
        '<rect x="3" y="5" width="11" height="9" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.5"/>' +
        '<path d="M9 3h8v8" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>' +
        '<path d="M17 3l-5.5 5.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>' +
        "</svg></button>" +
        '<span class="ex-status" id="ex-status" aria-live="polite"></span>';
      bindRightResize(root);
    }
    statusEl = root.querySelector("#ex-status");

    const detach = root.querySelector("#ex-detach");
    if (detach) {
      detach.addEventListener("click", () => {
        detach.disabled = true;
        detach.classList.add("is-busy");
        detachToWindow({ windowedFullscreen: false }).catch((e) => {
          console.error("[excalidraw] openAsWindow", e);
          detach.disabled = false;
          detach.classList.remove("is-busy");
          setStatus("打开失败");
        });
      });
    }
  }

  function loadScript(src) {
    return new Promise((resolve, reject) => {
      const s = document.createElement("script");
      s.src = src;
      s.async = false;
      s.onload = () => resolve();
      s.onerror = () => reject(new Error("script: " + src));
      document.head.appendChild(s);
    });
  }

  function loadCss(href) {
    return new Promise((resolve, reject) => {
      const link = document.createElement("link");
      link.rel = "stylesheet";
      link.href = href;
      link.onload = () => resolve();
      link.onerror = () => reject(new Error("css: " + href));
      document.head.appendChild(link);
    });
  }

  function scheduleSave(payload) {
    if (saveTimer) clearTimeout(saveTimer);
    setStatus("保存中…");
    saveTimer = setTimeout(() => {
      hub()
        .storage.set(SCENE_KEY, payload)
        .then(() => setStatus("已保存"))
        .catch((e) => {
          console.error(e);
          setStatus("保存失败");
        });
    }, SAVE_DEBOUNCE_MS);
  }

  function mountIframeFallback(canvas, reason) {
    console.warn("[excalidraw] fallback iframe:", reason);
    canvas.innerHTML = "";
    const frame = document.createElement("iframe");
    frame.title = "Excalidraw";
    frame.src = "https://excalidraw.com";
    frame.allow = "clipboard-read; clipboard-write";
    frame.style.cssText =
      "width:100%;height:100%;border:0;display:block;background:#121212;";
    canvas.appendChild(frame);
    setStatus("在线模式");
  }

  async function bootUmd(canvas) {
    setStatus("加载组件…");
    await loadCss(CDN + "/@excalidraw/excalidraw@" + EX_VER + "/dist/excalidraw.min.css");
    await loadScript(CDN + "/react@" + REACT_VER + "/umd/react.production.min.js");
    await loadScript(CDN + "/react-dom@" + REACT_VER + "/umd/react-dom.production.min.js");
    await loadScript(
      CDN + "/@excalidraw/excalidraw@" + EX_VER + "/dist/excalidraw.production.min.js",
    );

    const React = window.React;
    const ReactDOM = window.ReactDOM;
    const Lib = window.ExcalidrawLib;
    if (!React || !ReactDOM || !Lib || !Lib.Excalidraw) {
      throw new Error("ExcalidrawLib missing after UMD load");
    }

    let initialData = null;
    try {
      const saved = await hub().storage.get(SCENE_KEY);
      if (saved && typeof saved === "object") {
        initialData = {
          elements: Array.isArray(saved.elements) ? saved.elements : [],
          appState:
            saved.appState && typeof saved.appState === "object" ? saved.appState : {},
          files: saved.files && typeof saved.files === "object" ? saved.files : {},
        };
      }
    } catch (e) {
      console.warn("[excalidraw] scene", e);
    }

    canvas.innerHTML = "";
    const mount = document.createElement("div");
    mount.style.cssText = "width:100%;height:100%;";
    canvas.appendChild(mount);

    const props = {
      theme: "dark",
      UIOptions: {
        canvasActions: {
          loadScene: true,
          saveToActiveFile: false,
          export: true,
        },
      },
      onChange: function (elements, appState, files) {
        scheduleSave({
          elements: elements,
          appState: {
            viewBackgroundColor: appState && appState.viewBackgroundColor,
            currentItemFontFamily: appState && appState.currentItemFontFamily,
            gridSize: appState && appState.gridSize,
          },
          files: files || {},
          savedAt: Date.now(),
        });
      },
    };
    if (initialData) props.initialData = initialData;

    const el = React.createElement(Lib.Excalidraw, props);
    if (typeof ReactDOM.createRoot === "function") {
      ReactDOM.createRoot(mount).render(el);
    } else {
      ReactDOM.render(el, mount);
    }
  }

  async function main() {
    const root = document.getElementById("app");
    if (!root) throw new Error("#app missing");
    mountShell(root);
    const canvas = document.getElementById("ex-canvas");
    try {
      await bootUmd(canvas);
      if (!isNative) setStatus("");
      else {
        setStatus("就绪");
        window.setTimeout(() => setStatus(""), 1200);
      }
    } catch (e) {
      console.error("[excalidraw] umd failed", e);
      try {
        mountIframeFallback(canvas, e && e.message ? e.message : e);
      } catch (e2) {
        canvas.innerHTML =
          '<div class="ex-error"><div>无法加载 Excalidraw</div><div>请确认已联网后重开。</div><code>' +
          String(e && e.message ? e.message : e) +
          "</code></div>";
        setStatus("加载失败");
      }
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => {
      main().catch(console.error);
    });
  } else {
    main().catch(console.error);
  }
})();

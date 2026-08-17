/**
 * Excalidraw — shortcuts strip.
 * Opens a larger frameless popup (fixed under bar). Detach via in-popup button.
 */
(function () {
  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  const ICON =
    '<svg viewBox="0 0 24 24" aria-hidden="true" focusable="false"><path fill="currentColor" d="M4.5 17.5 9 7l3.2 5.6L14.5 9 19.5 17.5h-15Zm4.4-2.2h6.2l-1.4-2.5-1.6 2.2-1.5-2.6-1.7 2.9Z"/></svg>';

  async function openBoard() {
    const h = hub();
    let width = 480;
    let height = 640;
    try {
      const settings = await h.settings.getAll();
      if (settings && Number(settings.popupWidth) > 0) width = Number(settings.popupWidth);
      if (settings && Number(settings.popupHeight) > 0) height = Number(settings.popupHeight);
    } catch (_) {}
    await h.popup.open({
      width: width,
      height: height,
      resizable: false,
      nativeFrame: false,
    });
  }

  function mount() {
    const root = document.getElementById("bar") || document.body;
    root.innerHTML =
      '<button type="button" class="ex-chip" title="打开 Excalidraw">' +
      ICON +
      "<span>画板</span></button>";
    const btn = root.querySelector(".ex-chip");
    if (btn) {
      btn.addEventListener("click", () => {
        openBoard().catch((e) => console.error("[excalidraw]", e));
      });
    }
    try {
      hub().shortcuts.requestSize({ width: 72 });
    } catch (_) {}
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", mount);
  } else {
    mount();
  }
})();

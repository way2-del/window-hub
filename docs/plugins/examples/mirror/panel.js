/**
 * 镜子 — 仅在 hub.panel.onEnter（岛完全展开）后开摄像头；onLeave / 收起时关闭。
 * Host iframe 须 allow=camera。
 * WebView2 拒绝后会持久 DENY；经 hub.media.resetCameraPermission 可再授权。
 */
(function () {
  const video = document.getElementById("video");
  const status = document.getElementById("status");
  const root = document.querySelector(".mirror-root");
  let stream = null;
  let starting = false;

  function setStatus(html, isError) {
    if (!status) return;
    status.classList.toggle("is-error", !!isError);
    status.innerHTML = html;
  }

  function isDeniedError(e) {
    const name = String(e && e.name ? e.name : "");
    const msg = String(e && e.message ? e.message : e || "");
    return (
      name === "NotAllowedError" ||
      /permission|denied|not allowed/i.test(msg)
    );
  }

  function bindActions() {
    const retry = document.getElementById("camRetry");
    const privacy = document.getElementById("camPrivacy");
    if (retry) {
      retry.addEventListener("click", function (ev) {
        ev.preventDefault();
        ev.stopPropagation();
        void reauthorize();
      });
    }
    if (privacy) {
      privacy.addEventListener("click", function (ev) {
        ev.preventDefault();
        ev.stopPropagation();
        const h = window.hub;
        if (h && h.media && h.media.openCameraPrivacySettings) {
          void h.media.openCameraPrivacySettings().catch(function () {});
        }
      });
    }
  }

  function showDenied(detail) {
    if (root) root.classList.remove("is-ready");
    setStatus(
      "摄像头权限被拒绝" +
        "<span>" +
        (detail || "Permission denied") +
        "</span>" +
        '<div class="mirror-actions">' +
        '<button type="button" class="mirror-btn" id="camRetry">重新授权</button>' +
        '<button type="button" class="mirror-btn is-ghost" id="camPrivacy">系统设置</button>' +
        "</div>" +
        "<span class=\"mirror-hint\">若仍失败：Windows 设置 → 隐私 → 相机，允许桌面应用使用相机</span>",
      true,
    );
    bindActions();
  }

  function showGenericError(e) {
    if (root) root.classList.remove("is-ready");
    setStatus(
      "摄像头不可用" +
        "<span>" +
        (e && e.message ? String(e.message) : "无法打开摄像头") +
        "</span>" +
        '<div class="mirror-actions">' +
        '<button type="button" class="mirror-btn" id="camRetry">重试</button>' +
        '<button type="button" class="mirror-btn is-ghost" id="camPrivacy">系统设置</button>' +
        "</div>",
      true,
    );
    bindActions();
  }

  async function reauthorize() {
    setStatus("正在重置权限…", false);
    try {
      const h = window.hub;
      if (h && h.media && h.media.resetCameraPermission) {
        await h.media.resetCameraPermission();
      }
    } catch (e) {
      showDenied(e && e.message ? String(e.message) : String(e));
      return;
    }
    stream = null;
    starting = false;
    await start();
  }

  async function start() {
    if (stream || starting) return;
    starting = true;
    setStatus("正在打开摄像头…", false);
    try {
      stream = await navigator.mediaDevices.getUserMedia({
        audio: false,
        video: {
          facingMode: "user",
          width: { ideal: 1280 },
          height: { ideal: 720 },
        },
      });
      if (video) {
        video.srcObject = stream;
        await video.play().catch(function () {});
      }
      if (root) root.classList.add("is-ready");
      setStatus("", false);
    } catch (e) {
      if (isDeniedError(e)) showDenied(e && e.message ? String(e.message) : "");
      else showGenericError(e);
    } finally {
      starting = false;
    }
  }

  function stop() {
    starting = false;
    if (video) video.srcObject = null;
    if (stream) {
      stream.getTracks().forEach(function (t) {
        t.stop();
      });
    }
    stream = null;
    if (root) root.classList.remove("is-ready");
    setStatus("展开后加载摄像头", false);
  }

  function bind() {
    const h = window.hub;
    if (!h || !h.panel || !h.panel.onEnter || !h.panel.onLeave) {
      setStatus("面板生命周期不可用", true);
      return;
    }
    setStatus("展开后加载摄像头", false);
    h.panel.onEnter(function () {
      void start();
    });
    h.panel.onLeave(function () {
      stop();
    });
  }

  window.addEventListener("pagehide", stop);
  window.addEventListener("beforeunload", stop);

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", bind);
  } else {
    bind();
  }
})();

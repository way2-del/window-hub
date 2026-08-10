/**
 * 镜子 — 仅在 hub.panel.onEnter（岛完全展开）后开摄像头；onLeave / 收起时关闭。
 * Host iframe 须 allow=camera。
 */
(function () {
  const video = document.getElementById("video");
  const status = document.getElementById("status");
  const root = document.querySelector(".mirror-root");
  let stream = null;
  let starting = false;

  function setStatus(text, isError) {
    if (!status) return;
    status.classList.toggle("is-error", !!isError);
    status.innerHTML = text;
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
      if (root) root.classList.remove("is-ready");
      setStatus(
        "摄像头不可用<span>" +
          (e && e.message ? String(e.message) : "无法打开摄像头") +
          "</span>",
        true,
      );
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

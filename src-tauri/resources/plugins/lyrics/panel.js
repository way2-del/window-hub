(function () {
  let busy = false;
  let lastImg = "";

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function paint(now) {
    const songEl = document.getElementById("song");
    const lineEl = document.getElementById("line");
    const metaEl = document.getElementById("meta");
    const wrap = document.getElementById("mirror-wrap");
    const img = document.getElementById("mirror");
    if (!songEl || !lineEl || !metaEl || !wrap || !img) return;

    if (!now || !now.active) {
      songEl.textContent = "未检测到网易云";
      lineEl.textContent = "请打开网易云音乐并开启桌面歌词";
      lineEl.hidden = false;
      wrap.hidden = true;
      img.removeAttribute("src");
      lastImg = "";
      metaEl.textContent = "";
      return;
    }

    const desk = now.desktopLyrics === true;
    const title = String(now.title || "").trim();
    const artist = String(now.artist || "").trim();
    songEl.textContent = [title, artist].filter(Boolean).join(" · ") || "网易云 · 播放中";

    if (!desk) {
      lineEl.hidden = false;
      lineEl.textContent = "未开启桌面歌词（请在网易云打开）";
      wrap.hidden = true;
      img.removeAttribute("src");
      lastImg = "";
      metaEl.textContent = "桌面歌词：关 · 关后岛栏不显示歌词";
      return;
    }

    const image = String(now.lyricImage || "").trim();
    // DWM 由 Host 画在面板上方专属槽；这里不要再写提示/叠字
    if (now.mirrorLive) {
      wrap.hidden = true;
      img.removeAttribute("src");
      lastImg = "";
      lineEl.hidden = true;
      lineEl.textContent = "";
      metaEl.textContent = "";
      return;
    }
    if (image) {
      if (image !== lastImg) {
        img.src = image;
        lastImg = image;
      }
      wrap.hidden = false;
      lineEl.hidden = true;
      metaEl.textContent = "";
      return;
    }

    wrap.hidden = true;
    img.removeAttribute("src");
    lastImg = "";
    lineEl.hidden = false;
    lineEl.textContent = "正在捕捉桌面歌词（建议横向单行）";
    const bits = ["桌面歌词：开"];
    if (now.source) bits.unshift("来源：" + now.source);
    metaEl.textContent = bits.join(" · ");
  }

  async function transport(action) {
    if (busy) return;
    const h = hub();
    if (!h.media || !h.media.transport) {
      console.warn("[lyrics panel] hub.media.transport missing");
      return;
    }
    busy = true;
    try {
      await h.media.transport(action);
    } catch (err) {
      console.warn("[lyrics panel] transport", action, err);
    } finally {
      setTimeout(function () {
        busy = false;
      }, 180);
    }
  }

  async function openNetease() {
    if (busy) return;
    const h = hub();
    if (!h.media || !h.media.openNetease) {
      console.warn("[lyrics panel] hub.media.openNetease missing");
      return;
    }
    busy = true;
    try {
      await h.media.openNetease();
    } catch (err) {
      console.warn("[lyrics panel] openNetease", err);
      const metaEl = document.getElementById("meta");
      if (metaEl) {
        metaEl.textContent = String(err && err.message ? err.message : err);
      }
    } finally {
      setTimeout(function () {
        busy = false;
      }, 280);
    }
  }

  function bindTransport() {
    const prev = document.getElementById("btn-prev");
    const toggle = document.getElementById("btn-toggle");
    const next = document.getElementById("btn-next");
    const openBtn = document.getElementById("btn-open");
    if (prev) {
      prev.addEventListener("click", function (e) {
        e.preventDefault();
        e.stopPropagation();
        void transport("prev");
      });
    }
    if (toggle) {
      toggle.addEventListener("click", function (e) {
        e.preventDefault();
        e.stopPropagation();
        void transport("play-pause");
      });
    }
    if (next) {
      next.addEventListener("click", function (e) {
        e.preventDefault();
        e.stopPropagation();
        void transport("next");
      });
    }
    if (openBtn) {
      openBtn.addEventListener("click", function (e) {
        e.preventDefault();
        e.stopPropagation();
        void openNetease();
      });
    }
  }

  async function tick() {
    const h = hub();
    try {
      const now =
        h.media && h.media.neteaseNowPlaying
          ? await h.media.neteaseNowPlaying()
          : null;
      paint(now);
    } catch (err) {
      console.warn("[lyrics panel]", err);
    }
  }

  async function boot() {
    bindTransport();
    await tick();
    let timer = 0;
    function arm() {
      if (timer) clearTimeout(timer);
      const ms = document.hidden ? 2000 : 280;
      timer = setTimeout(function () {
        void tick().finally(arm);
      }, ms);
    }
    arm();
    document.addEventListener("visibilitychange", function () {
      arm();
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

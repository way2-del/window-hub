(function () {
  const HOLD_MS = 900;
  let held = { songKey: "", lyric: "", at: 0 };
  let busy = false;

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function paint(now) {
    const songEl = document.getElementById("song");
    const lineEl = document.getElementById("line");
    const metaEl = document.getElementById("meta");
    if (!songEl || !lineEl || !metaEl) return;
    if (!now || !now.active) {
      held = { songKey: "", lyric: "", at: 0 };
      songEl.textContent = "未检测到网易云";
      lineEl.textContent = "请打开网易云音乐并开启桌面歌词";
      metaEl.textContent = "";
      return;
    }
    const desk = now.desktopLyrics === true;
    const title = String(now.title || "").trim();
    const artist = String(now.artist || "").trim();
    const songKey = title + "\0" + artist;
    if (held.songKey && held.songKey !== songKey) {
      held = { songKey: "", lyric: "", at: 0 };
    }
    songEl.textContent = [title, artist].filter(Boolean).join(" · ") || "网易云 · 播放中";

    if (!desk) {
      held = { songKey: "", lyric: "", at: 0 };
      lineEl.textContent = "未开启桌面歌词（请在网易云打开）";
      metaEl.textContent = "桌面歌词：关 · 关后岛栏不显示歌词";
      return;
    }

    let lyric = String(now.lyric || "").trim();
    if (lyric) {
      held = { songKey: songKey, lyric: lyric, at: Date.now() };
    } else if (
      held.lyric &&
      held.songKey === songKey &&
      Date.now() - held.at < HOLD_MS
    ) {
      lyric = held.lyric;
    }

    lineEl.textContent = lyric ? lyric : "桌面歌词已开 · 正在同步";
    const bits = [];
    if (now.source) bits.push("来源：" + now.source);
    bits.push("桌面歌词：开");
    if (lyric && !String(now.lyric || "").trim()) bits.push("保持上一句");
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

  function bindTransport() {
    const prev = document.getElementById("btn-prev");
    const toggle = document.getElementById("btn-toggle");
    const next = document.getElementById("btn-next");
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
      const ms = document.hidden ? 2500 : 800;
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

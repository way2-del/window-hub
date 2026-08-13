(function () {
  const HOLD_MS = 6000;
  let held = { songKey: "", lyric: "", at: 0 };

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
    const title = String(now.title || "").trim();
    const artist = String(now.artist || "").trim();
    const songKey = title + "\0" + artist;
    if (held.songKey && held.songKey !== songKey) {
      held = { songKey: "", lyric: "", at: 0 };
    }
    songEl.textContent = [title, artist].filter(Boolean).join(" · ") || "网易云 · 播放中";

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

    const desk =
      now.desktopLyrics === true ||
      now.source === "desktop-lyrics" ||
      now.source === "api-lrc" ||
      now.source === "memory";
    lineEl.textContent = lyric
      ? lyric
      : desk
        ? "桌面歌词已开 · 正在同步"
        : "未检测到桌面歌词窗口（请在网易云开启）";
    const bits = [];
    if (now.source) bits.push("来源：" + now.source);
    bits.push(desk ? "桌面歌词：开" : "桌面歌词：关");
    if (lyric && !String(now.lyric || "").trim()) bits.push("保持上一句");
    metaEl.textContent = bits.join(" · ");
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

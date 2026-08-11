(function () {
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
      songEl.textContent = "未检测到网易云";
      lineEl.textContent = "请打开网易云音乐并开启桌面歌词";
      metaEl.textContent = "";
      return;
    }
    const title = String(now.title || "").trim();
    const artist = String(now.artist || "").trim();
    songEl.textContent = [title, artist].filter(Boolean).join(" · ") || "网易云 · 播放中";
    const lyric = String(now.lyric || "").trim();
    lineEl.textContent = lyric || "暂无歌词行（可开启桌面歌词）";
    metaEl.textContent = now.source ? "来源：" + now.source : "";
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
    setInterval(function () {
      void tick();
    }, 600);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

/**
 * 歌词 — 快捷区隐形 worker：捕捉桌面歌词窗 → hub.island.setBar({ image })
 * 对齐 MyDockFinder「彩色映射」：不 OCR、不本地选句，避免错字/超前。
 */
(function () {
  const CACHE_KEY = "cache";
  const POLL_MS = 220;
  const POLL_HIDDEN_MS = 1500;

  let settingsCache = null;
  let lastBarKey = "";

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function truncate(s, n) {
    const t = String(s || "").trim();
    if (!t) return "";
    const chars = [...t];
    if (chars.length <= n) return t;
    return chars.slice(0, n - 1).join("") + "…";
  }

  function imageKey(img) {
    const s = String(img || "");
    if (!s) return "";
    // 完整 dataURL 太长；用长度 + 首尾做脏检查即可
    return s.length + ":" + s.slice(32, 56) + ":" + s.slice(-40);
  }

  async function loadSettings(force) {
    if (!force && settingsCache) return settingsCache;
    const h = hub();
    const all = (await h.settings.getAll().catch(function () { return {}; })) || {};
    settingsCache = {
      showWhenIdle: !!all.showWhenIdle,
      preferLyric: all.preferLyric !== false,
      requireDesktopLyrics: all.requireDesktopLyrics !== false,
    };
    return settingsCache;
  }

  function desktopLyricsOn(now) {
    return !!(now && now.desktopLyrics === true);
  }

  function barFrom(now, settings) {
    if (settings.requireDesktopLyrics && !desktopLyricsOn(now)) {
      if (settings.showWhenIdle && now && now.active) {
        return { text: "开桌面歌词", title: "请在网易云开启「桌面歌词」以显示在灵动岛" };
      }
      return null;
    }
    if (!desktopLyricsOn(now)) {
      return null;
    }
    if (!now || !now.active) {
      if (settings.showWhenIdle) {
        return { text: "网易云 · 未播放", title: "打开网易云音乐并开启桌面歌词" };
      }
      return null;
    }

    const title = String(now.title || "").trim();
    const artist = String(now.artist || "").trim();
    const song = [title, artist].filter(Boolean).join(" · ");
    const image = String(now.lyricImage || "").trim();

    // 有桌面歌词镜像：缩小贴上岛栏（彩色映射）
    if (image) {
      return {
        text: "♪",
        title: song || "网易云 · 桌面歌词",
        image: image,
      };
    }

    // 截图尚未就绪：先占位歌名
    if (song) {
      return {
        text: truncate(song, 28),
        title: song + " · 捕捉桌面歌词中",
      };
    }
    if (settings.showWhenIdle || settings.preferLyric) {
      return { text: "捕捉桌面歌词…", title: "网易云 · 桌面歌词" };
    }
    return null;
  }

  async function applyBar(payload) {
    const key = payload
      ? payload.text +
        "\0" +
        (payload.title || "") +
        "\0" +
        imageKey(payload.image)
      : "";
    if (key === lastBarKey) return;
    lastBarKey = key;
    const h = hub();
    if (!h.island) return;
    try {
      if (!payload) {
        if (h.island.clearBar) await h.island.clearBar();
        return;
      }
      await h.island.setBar({
        text: payload.text,
        title: payload.title,
        image: payload.image || "",
      });
    } catch (err) {
      console.warn("[lyrics] setBar", err);
    }
  }

  async function tick() {
    const h = hub();
    const settings = await loadSettings(false);
    let now = null;
    try {
      if (h.media && h.media.neteaseNowPlaying) {
        now = await h.media.neteaseNowPlaying();
      }
    } catch (err) {
      console.warn("[lyrics] poll", err);
    }
    const payload = barFrom(now, settings);
    await applyBar(payload);
    // 不把巨大 base64 写入 storage
    if (payload && !payload.image) {
      await h.storage
        .set(CACHE_KEY, {
          title: now && now.title,
          artist: now && now.artist,
          savedAt: Date.now(),
        })
        .catch(function () {});
    }
  }

  async function boot() {
    const h = hub();
    try {
      if (h.shortcuts && h.shortcuts.requestSize) {
        await h.shortcuts.requestSize({ width: 1 });
      }
    } catch (_) {}
    await tick();
    let timer = 0;
    function arm() {
      if (timer) clearTimeout(timer);
      const ms = document.hidden ? POLL_HIDDEN_MS : POLL_MS;
      timer = setTimeout(function () {
        void tick().finally(arm);
      }, ms);
    }
    arm();
    document.addEventListener("visibilitychange", function () {
      arm();
    });
    if (h.settings && h.settings.subscribe) {
      h.settings.subscribe(function () {
        settingsCache = null;
        void tick();
      });
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

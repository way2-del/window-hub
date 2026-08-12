/**
 * 歌词 — 快捷区隐形 worker：轮询网易云 → hub.island.setBar
 * Host：hub.media.neteaseNowPlaying（桌面歌词 / 窗口标题 / 内存兜底）
 */
(function () {
  const CACHE_KEY = "cache";
  const POLL_MS = 1600;

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

  async function loadSettings(force) {
    if (!force && settingsCache) return settingsCache;
    const h = hub();
    const all = (await h.settings.getAll().catch(function () { return {}; })) || {};
    settingsCache = {
      showWhenIdle: !!all.showWhenIdle,
      preferLyric: all.preferLyric !== false,
    };
    return settingsCache;
  }

  function barFrom(now, settings) {
    if (!now || !now.active) {
      if (settings.showWhenIdle) {
        return { text: "网易云 · 未播放", title: "打开网易云音乐并开启桌面歌词" };
      }
      return null;
    }
    const lyric = String(now.lyric || "").trim();
    const title = String(now.title || "").trim();
    const artist = String(now.artist || "").trim();
    const song = [title, artist].filter(Boolean).join(" · ");
    if (settings.preferLyric && lyric) {
      return {
        text: truncate(lyric, 28),
        title: song || lyric,
      };
    }
    if (song) {
      return { text: truncate(song, 28), title: song };
    }
    if (lyric) {
      return { text: truncate(lyric, 28), title: lyric };
    }
    if (settings.showWhenIdle) {
      return { text: "网易云 · 播放中", title: "网易云音乐" };
    }
    return null;
  }

  async function applyBar(payload) {
    const key = payload ? payload.text + "\0" + (payload.title || "") : "";
    if (key === lastBarKey) return;
    lastBarKey = key;
    const h = hub();
    if (!h.island) return;
    try {
      if (!payload) {
        if (h.island.clearBar) await h.island.clearBar();
        return;
      }
      await h.island.setBar({ text: payload.text, title: payload.title });
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
    // Avoid hammering SQLite every tick — only when lyric/title changes.
    const cacheKey = payload ? payload.text : "";
    if (cacheKey !== lastBarKey) {
      await h.storage.set(CACHE_KEY, { now: now, savedAt: Date.now() }).catch(function () {});
    }
    await applyBar(payload);
  }

  async function boot() {
    const h = hub();
    try {
      if (h.shortcuts && h.shortcuts.requestSize) {
        await h.shortcuts.requestSize({ width: 1 });
      }
    } catch (_) {}
    await tick();
    setInterval(function () {
      void tick();
    }, POLL_MS);
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

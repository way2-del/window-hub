/**
 * 歌词 — 快捷区隐形 worker：轮询网易云 → hub.island.setBar
 * Host：hub.media.neteaseNowPlaying（桌面歌词 / api-lrc / 内存）
 */
(function () {
  const CACHE_KEY = "cache";
  const POLL_MS = 400;
  const POLL_HIDDEN_MS = 2000;
  /** 同曲短暂读空时保留上一句；切歌必须清掉，否则会串上一首 */
  const HOLD_LYRIC_MS = 900;

  let settingsCache = null;
  let lastBarKey = "";
  let held = { songKey: "", text: "", at: 0 };

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
      requireDesktopLyrics: all.requireDesktopLyrics !== false,
    };
    return settingsCache;
  }

  function desktopLyricsOn(now) {
    // 只认 Host 检测到的「可见桌面歌词窗」，不要把 api-lrc 来源当成已开
    return !!(now && now.desktopLyrics === true);
  }

  function resolveLyric(now, title, artist) {
    if (!desktopLyricsOn(now)) {
      held = { songKey: "", text: "", at: 0 };
      return "";
    }
    let lyric = String(now && now.lyric || "").trim();
    const songKey = title + "\0" + artist;
    if (held.songKey && held.songKey !== songKey) {
      held = { songKey: "", text: "", at: 0 };
    }
    if (lyric) {
      held = { songKey: songKey, text: lyric, at: Date.now() };
      return lyric;
    }
    if (
      held.text &&
      held.songKey === songKey &&
      Date.now() - held.at < HOLD_LYRIC_MS
    ) {
      return held.text;
    }
    return "";
  }

  function barFrom(now, settings) {
    if (settings.requireDesktopLyrics && !desktopLyricsOn(now)) {
      held = { songKey: "", text: "", at: 0 };
      if (settings.showWhenIdle && now && now.active) {
        return { text: "开桌面歌词", title: "请在网易云开启「桌面歌词」以显示在灵动岛" };
      }
      return null;
    }
    if (!desktopLyricsOn(now)) {
      held = { songKey: "", text: "", at: 0 };
      return null;
    }
    if (!now || !now.active) {
      held = { songKey: "", text: "", at: 0 };
      if (settings.showWhenIdle) {
        return { text: "网易云 · 未播放", title: "打开网易云音乐并开启桌面歌词" };
      }
      return null;
    }
    const title = String(now.title || "").trim();
    const artist = String(now.artist || "").trim();
    const song = [title, artist].filter(Boolean).join(" · ");
    const lyric = resolveLyric(now, title, artist);

    // preferLyric：桌面歌词开着时优先歌词行（短暂读空用 hold /「同步中」）
    if (settings.preferLyric) {
      if (lyric) {
        return { text: truncate(lyric, 28), title: song || lyric };
      }
      return {
        text: "歌词同步中…",
        title: song || "网易云 · 桌面歌词",
      };
    }
    if (lyric) {
      return { text: truncate(lyric, 28), title: song || lyric };
    }
    if (song) {
      return { text: truncate(song, 28), title: song };
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

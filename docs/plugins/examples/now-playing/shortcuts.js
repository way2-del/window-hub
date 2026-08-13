/**
 * Now Playing — shortcuts invisible worker.
 * Polls local Now Playing HTTP API → island.setBar (lyrics / title).
 * Width 0 so it never paints in the shortcuts strip.
 *
 * Offline: exponential backoff (服务未开时勿每秒打 hub.fetch 拖垮 Host).
 */
(function () {
  const CACHE_KEY = "np-cache";
  const PLUGIN_ID =
    (typeof window.__WH_PLUGIN_ID__ === "string" && window.__WH_PLUGIN_ID__) ||
    "com.window-hub.now-playing";
  const OFFLINE_MAX_MS = 60000;
  let timer = null;
  let lastBar = "";
  let settings = {
    apiBase: "http://127.0.0.1:9863",
    pollMs: 1200,
    barMode: "lyric",
  };
  let lyricCache = { key: "", lines: [] };
  let lastStoreKey = "";
  let tickInFlight = false;
  /** consecutive failed /api/query (service down) */
  let failStreak = 0;
  /** only push setBar when Host selected us as 岛栏常驻 */
  let isBarResident = false;
  let prefsKnown = false;

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function joinUrl(base, path) {
    const b = String(base || "").replace(/\/+$/, "");
    const p = path.startsWith("/") ? path : "/" + path;
    return b + p;
  }

  function nextDelayMs() {
    if (failStreak <= 0) return settings.pollMs;
    // 1→2.5s, 2→5s, 3→10s, 4→20s, 5+→40–60s
    const ms = Math.round(settings.pollMs * Math.pow(2, failStreak));
    return Math.min(OFFLINE_MAX_MS, Math.max(2500, ms));
  }

  async function refreshSettings() {
    const all =
      (await hub()
        .settings.getAll()
        .catch(function () {
          return {};
        })) || {};
    settings = {
      apiBase:
        String(all.apiBase || "http://127.0.0.1:9863").trim() ||
        "http://127.0.0.1:9863",
      pollMs: Math.max(500, Number(all.pollMs) || 1200),
      barMode: String(all.barMode || "lyric"),
    };
    return settings;
  }

  async function apiGet(base, path) {
    // 离线探测：短超时，避免服务未开时每次卡满 2.5s+
    const res = await hub().fetch(joinUrl(base, path), {
      method: "GET",
      timeoutMs: failStreak > 0 ? 800 : 1500,
    });
    if (!res || !res.ok) throw new Error("HTTP " + (res && res.status));
    return JSON.parse(res.body || "{}");
  }

  function parseLyricLines(lrc) {
    const raw = String(lrc || "");
    const out = [];
    const lines = raw.split(/\r?\n/);
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i].trim();
      if (!line) continue;
      if (line.charAt(0) === "{") {
        try {
          const obj = JSON.parse(line);
          const t = typeof obj.t === "number" ? obj.t : 0;
          const parts = Array.isArray(obj.c) ? obj.c : [];
          const text = parts
            .map(function (c) {
              return c && c.tx != null ? String(c.tx) : "";
            })
            .join("")
            .trim();
          if (text) out.push({ t: t, text: text });
        } catch (_) {
          /* skip */
        }
        continue;
      }
      const m = line.match(/^\[(\d{1,2}):(\d{1,2})(?:\.(\d{1,3}))?\](.*)$/);
      if (m) {
        const min = Number(m[1]) || 0;
        const sec = Number(m[2]) || 0;
        let frac = m[3] || "0";
        if (frac.length === 1) frac += "00";
        else if (frac.length === 2) frac += "0";
        const ms = min * 60000 + sec * 1000 + (Number(frac.slice(0, 3)) || 0);
        const text = String(m[4] || "").trim();
        if (text) out.push({ t: ms, text: text });
      }
    }
    out.sort(function (a, b) {
      return a.t - b.t;
    });
    return out;
  }

  function lyricAt(lines, progressMs) {
    if (!lines.length) return "";
    let cur = "";
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].t <= progressMs) cur = lines[i].text;
      else break;
    }
    return cur;
  }

  function trackKey(track) {
    if (!track) return "";
    return (
      String(track.id || "") +
      "\0" +
      String(track.title || "") +
      "\0" +
      String(track.author || "")
    );
  }

  function barPayload(mode, track, lyricLine, connected) {
    if (!connected) {
      return { text: "", title: "" };
    }
    const title = (track && track.title) || "";
    const author = (track && track.author) || "";
    const song = [title, author].filter(Boolean).join(" · ") || "暂无歌曲";
    const lyric = (lyricLine || "").trim();
    if (mode === "title") {
      return { text: song, title: song };
    }
    if (mode === "both") {
      const text = lyric ? "🎶 " + lyric : song;
      return { text: text, title: lyric ? song + " · " + lyric : song };
    }
    // lyric（默认）：有歌词加音符前缀；无歌词回落歌名不加
    return {
      text: lyric ? "🎶 " + lyric : song,
      title: lyric ? song + " · " + lyric : song,
    };
  }

  async function applyBar(text, title) {
    const h = hub();
    if (!h.island || !h.island.setBar) return;
    const t = String(text || "").trim();
    if (!t) {
      if (!lastBar) return;
      try {
        await h.island.clearBar();
      } catch (_) {}
      lastBar = "";
      return;
    }
    const key = t + "\0" + (title || "");
    if (key === lastBar) return;
    lastBar = key;
    try {
      await h.island.setBar({ text: t, title: title || t });
    } catch (err) {
      console.warn("[now-playing] setBar", err);
    }
  }

  async function ensureLyrics(base, track) {
    const key = trackKey(track);
    if (!key) {
      lyricCache = { key: "", lines: [] };
      return [];
    }
    if (lyricCache.key === key) return lyricCache.lines;
    try {
      const lyric = await apiGet(base, "/api/lyric");
      const lines = parseLyricLines(lyric.lrc);
      lyricCache = { key: key, lines: lines };
      return lines;
    } catch (_) {
      lyricCache = { key: key, lines: [] };
      return [];
    }
  }

  function applyIslandPrefs(prefs) {
    prefsKnown = true;
    const bar =
      prefs && typeof prefs.barResident === "string"
        ? prefs.barResident
        : prefs && typeof prefs.bar_resident === "string"
          ? prefs.bar_resident
          : "";
    const next = bar === PLUGIN_ID;
    if (isBarResident && !next) {
      void applyBar("", "");
    }
    isBarResident = next;
  }

  async function tick() {
    if (tickInFlight) return;
    tickInFlight = true;
    try {
      // 未当选岛栏常驻：极少探测（仅保活），避免无意义 IPC
      if (prefsKnown && !isBarResident) {
        failStreak = Math.max(failStreak, 3);
        return;
      }

      const s = settings;
      let connected = false;
      let track = null;
      let progressMs = 0;
      let lyricLine = "";
      try {
        const q = await apiGet(s.apiBase, "/api/query");
        connected = true;
        failStreak = 0;
        track = q.track || null;
        const player = q.player || {};
        if (!player.hasSong) {
          track = null;
        } else {
          progressMs =
            typeof player.seekbarCurrentPosition === "number"
              ? Math.round(player.seekbarCurrentPosition * 1000)
              : 0;
          if (s.barMode !== "title") {
            const lines = await ensureLyrics(s.apiBase, track);
            lyricLine = lyricAt(lines, progressMs);
          }
        }
      } catch (err) {
        connected = false;
        failStreak = Math.min(8, failStreak + 1);
        if (failStreak <= 2) {
          console.warn("[now-playing] poll offline", failStreak, err);
        }
      }

      const active = !!(connected && track);
      if (!isBarResident && prefsKnown) {
        return;
      }

      const payload = barPayload(s.barMode, track, lyricLine, connected);
      const storeKey =
        String(connected) +
        "\0" +
        ((track && (track.title || "")) || "") +
        "\0" +
        ((track && (track.author || "")) || "") +
        "\0" +
        lyricLine;
      if (storeKey !== lastStoreKey) {
        lastStoreKey = storeKey;
        await hub()
          .storage.set(CACHE_KEY, {
            connected: connected,
            track: track,
            progressMs: progressMs,
            lyricLine: lyricLine,
            savedAt: Date.now(),
          })
          .catch(function () {});
      }
      if (!active) {
        await applyBar("", "");
        return;
      }
      await applyBar(payload.text, payload.title);
    } finally {
      tickInFlight = false;
    }
  }

  async function boot() {
    const h = hub();
    try {
      if (h.shortcuts && h.shortcuts.requestSize) {
        await h.shortcuts.requestSize({ width: 0 });
      }
    } catch (_) {}

    await refreshSettings();

    const loop = async function () {
      try {
        await tick();
      } catch (err) {
        console.warn("[now-playing]", err);
        failStreak = Math.min(8, failStreak + 1);
      }
      timer = window.setTimeout(loop, nextDelayMs());
    };
    void loop();

    if (h.settings && h.settings.subscribe) {
      h.settings.subscribe(function () {
        void refreshSettings().then(function () {
          failStreak = 0;
          if (timer) {
            clearTimeout(timer);
            timer = null;
          }
          void loop();
        });
      });
    }
    window.addEventListener("wh-shortcuts-evt", function (ev) {
      var d = ev && ev.detail;
      if (!d) return;
      if (d.type === "island-prefs") {
        applyIslandPrefs(d.prefs || {});
        failStreak = 0;
        if (timer) {
          clearTimeout(timer);
          timer = null;
        }
        void loop();
        return;
      }
    });

    // Host 可能晚于 boot 才广播 prefs；先拉一次（若 bridge 无此命令则忽略）
    try {
      if (h.invoke) {
        /* no-op */
      }
    } catch (_) {}
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot();
    });
  } else {
    void boot();
  }
})();

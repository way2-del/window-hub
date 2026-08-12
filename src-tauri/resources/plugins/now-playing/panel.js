/**
 * Now Playing — island panel mini player (minimal).
 * Layout: cover + title/artist + eq | progress | prev / pause / next only.
 */
(function () {
  const CACHE_KEY = "np-cache";
  let timer = null;
  let settings = {
    apiBase: "http://127.0.0.1:9863",
    pollMs: 1200,
  };
  let lyricCache = { key: "", lines: [] };
  let lastStoreKey = "";
  let state = {
    connected: false,
    track: null,
    player: null,
    progressMs: 0,
    lyricLine: "",
    error: "",
  };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function escapeHtml(s) {
    return String(s ?? "")
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;")
      .replaceAll('"', "&quot;");
  }

  function joinUrl(base, path) {
    const b = String(base || "").replace(/\/+$/, "");
    const p = path.startsWith("/") ? path : "/" + path;
    return b + p;
  }

  function fmtMs(ms) {
    const n = Math.max(0, Math.floor(Number(ms) / 1000) || 0);
    const m = Math.floor(n / 60);
    const s = n % 60;
    return m + ":" + String(s).padStart(2, "0");
  }

  function fmtRemain(progressMs, durationMs) {
    if (!durationMs) return "--:--";
    const left = Math.max(0, durationMs - progressMs);
    return "-" + fmtMs(left);
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
    };
    return settings;
  }

  async function loadSettingsCached() {
    return settings;
  }

  async function apiGet(base, path) {
    const res = await hub().fetch(joinUrl(base, path), {
      method: "GET",
      timeoutMs: 2500,
    });
    if (!res || !res.ok) throw new Error("HTTP " + (res && res.status));
    return JSON.parse(res.body || "{}");
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
        } catch (_) {}
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
    let cur = "";
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].t <= progressMs) cur = lines[i].text;
      else break;
    }
    return cur;
  }

  function iconPrev() {
    return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M11.5 12 20 6.2v11.6L11.5 12zm-7.5 5.8V6.2h2.2v11.6H4z"/></svg>';
  }
  function iconNext() {
    return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12.5 12 4 17.8V6.2L12.5 12zm5.3-5.8h2.2v11.6h-2.2V6.2z"/></svg>';
  }
  function iconPlay() {
    return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8.2 5.2v13.6L19.2 12 8.2 5.2z"/></svg>';
  }
  function iconPause() {
    return '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6.4 5h3.4v14H6.4V5zm7.8 0h3.4v14h-3.4V5z"/></svg>';
  }

  function render() {
    const app = document.getElementById("app");
    if (!app) return;
    const track = state.track || {};
    const player = state.player || {};
    const hasSong = !!(state.connected && player.hasSong && track.title);
    const paused = !!player.isPaused;
    const playing = hasSong && !paused;
    const durationMs = (Number(track.duration) || 0) * 1000;
    const pct = durationMs > 0 ? Math.min(100, (state.progressMs / durationMs) * 100) : 0;
    const cover = track.cover || "";
    const subtitle = !state.connected
      ? state.error || "未连接 Now Playing"
      : hasSong
        ? track.author || track.album || "—"
        : "暂无歌曲";

    app.innerHTML =
      '<div class="np-head">' +
      (cover
        ? '<img class="np-cover" alt="" src="' + escapeHtml(cover) + '" />'
        : '<div class="np-cover is-empty" aria-hidden="true">♪</div>') +
      '<div class="np-meta">' +
      '<div class="np-title">' +
      escapeHtml(hasSong ? track.title : "正在播放") +
      "</div>" +
      '<div class="np-artist">' +
      escapeHtml(subtitle) +
      "</div>" +
      "</div>" +
      '<div class="np-eq' +
      (playing ? " is-playing" : "") +
      '" aria-hidden="true"><span></span><span></span><span></span><span></span></div>' +
      "</div>" +
      '<div class="np-progress">' +
      '<span class="np-time">' +
      escapeHtml(fmtMs(state.progressMs)) +
      "</span>" +
      '<div class="np-bar" aria-hidden="true"><i style="width:' +
      pct.toFixed(2) +
      '%"></i></div>' +
      '<span class="np-time is-end">' +
      escapeHtml(fmtRemain(state.progressMs, durationMs)) +
      "</span></div>" +
      '<div class="np-controls">' +
      '<button type="button" class="np-btn" data-act="previous" title="上一曲">' +
      iconPrev() +
      "</button>" +
      '<button type="button" class="np-btn is-main" data-act="play_pause" title="播放 / 暂停">' +
      (paused || !hasSong ? iconPlay() : iconPause()) +
      "</button>" +
      '<button type="button" class="np-btn" data-act="next" title="下一曲">' +
      iconNext() +
      "</button>" +
      "</div>";

    app.querySelectorAll("[data-act]").forEach(function (el) {
      el.addEventListener("click", function () {
        void sendMedia(el.getAttribute("data-act"));
      });
    });
  }

  async function sendMedia(action) {
    try {
      if (!hub().media || !hub().media.sendKey) {
        throw new Error("hub.media.sendKey unavailable");
      }
      await hub().media.sendKey(action);
      window.setTimeout(function () {
        void refresh();
      }, 250);
    } catch (err) {
      console.warn("[now-playing] media", err);
    }
  }

  async function refresh() {
    const s = await loadSettingsCached();
    try {
      const q = await apiGet(s.apiBase, "/api/query");
      state.connected = true;
      state.error = "";
      state.player = q.player || null;
      state.track = q.player && q.player.hasSong ? q.track || null : null;
      const seek =
        state.player && typeof state.player.seekbarCurrentPosition === "number"
          ? state.player.seekbarCurrentPosition
          : 0;
      state.progressMs = Math.round(seek * 1000);
      if (state.track) {
        const lines = await ensureLyrics(s.apiBase, state.track);
        state.lyricLine = lyricAt(lines, state.progressMs);
      } else {
        state.lyricLine = "";
      }
      const storeKey =
        String(state.connected) +
        "\0" +
        ((state.track && state.track.title) || "") +
        "\0" +
        state.lyricLine +
        "\0" +
        Math.floor(state.progressMs / 1000);
      if (storeKey !== lastStoreKey) {
        lastStoreKey = storeKey;
        await hub()
          .storage.set(CACHE_KEY, {
            connected: state.connected,
            track: state.track,
            progressMs: state.progressMs,
            lyricLine: state.lyricLine,
            savedAt: Date.now(),
          })
          .catch(function () {});
      }
    } catch (err) {
      state.connected = false;
      state.error = String((err && err.message) || err || "连接失败");
    }
    render();
  }

  async function loop() {
    await refresh();
    timer = window.setTimeout(function () {
      void loop();
    }, settings.pollMs);
  }

  function schedule() {
    if (timer) clearTimeout(timer);
    timer = null;
    void loop();
  }

  async function boot() {
    await refreshSettings();
    const cached = await hub()
      .storage.get(CACHE_KEY)
      .catch(function () {
        return null;
      });
    if (cached) {
      state.connected = !!cached.connected;
      state.track = cached.track || null;
      state.player = { hasSong: !!cached.track, isPaused: false };
      state.progressMs = Number(cached.progressMs) || 0;
      state.lyricLine = cached.lyricLine || "";
      render();
    } else {
      render();
    }

    const h = hub();
    if (h.panel && h.panel.onEnter) {
      h.panel.onEnter(function () {
        schedule();
      });
    }
    if (h.panel && h.panel.onLeave) {
      h.panel.onLeave(function () {
        if (timer) {
          clearTimeout(timer);
          timer = null;
        }
      });
    }
    // 勿在 boot 就轮询：面板未展开时不应打 HTTP
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot();
    });
  } else {
    void boot();
  }
})();

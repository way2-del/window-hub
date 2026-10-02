/**
 * Now Playing — full lyrics companion (Host home right card).
 * Polls /api/query + /api/lyric via hub.fetch (same as shortcuts worker).
 */
(function () {
  let settings = {
    apiBase: "http://127.0.0.1:9863",
    pollMs: 1200,
    lyricOffsetMs: 0,
  };
  let timer = null;
  let failStreak = 0;
  let lyricCache = { key: "", lines: [] };
  let lastRenderKey = "";
  let active = false;

  const dom = {
    meta: document.getElementById("meta"),
    title: document.getElementById("title"),
    artist: document.getElementById("artist"),
    list: document.getElementById("list"),
    empty: document.getElementById("empty"),
  };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function joinUrl(base, path) {
    const b = String(base || "").replace(/\/+$/, "");
    const p = path.startsWith("/") ? path : "/" + path;
    return b + p;
  }

  async function refreshSettings() {
    const all =
      (await hub()
        .settings.getAll()
        .catch(function () {
          return {};
        })) || {};
    const offsetRaw = Number(all.lyricOffsetMs);
    settings = {
      apiBase:
        String(all.apiBase || "http://127.0.0.1:9863").trim() ||
        "http://127.0.0.1:9863",
      pollMs: Math.max(500, Number(all.pollMs) || 1200),
      lyricOffsetMs: Number.isFinite(offsetRaw)
        ? Math.max(-10000, Math.min(10000, Math.round(offsetRaw)))
        : 0,
    };
  }

  async function apiGet(path) {
    const res = await hub().fetch(joinUrl(settings.apiBase, path), {
      method: "GET",
      timeoutMs: failStreak > 0 ? 400 : 1200,
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

  async function ensureLyrics(track) {
    const key = trackKey(track);
    if (!key) {
      lyricCache = { key: "", lines: [] };
      return [];
    }
    if (lyricCache.key === key) return lyricCache.lines;
    try {
      const lyric = await apiGet("/api/lyric");
      const lines = parseLyricLines(lyric.lrc);
      lyricCache = { key: key, lines: lines };
      return lines;
    } catch (_) {
      lyricCache = { key: key, lines: [] };
      return [];
    }
  }

  function activeIndex(lines, progressMs) {
    let idx = -1;
    for (let i = 0; i < lines.length; i++) {
      if (lines[i].t <= progressMs) idx = i;
      else break;
    }
    return idx;
  }

  function render(track, lines, progressMs) {
    const title = track && track.title ? String(track.title) : "";
    const artist = track && track.author ? String(track.author) : "";
    const idx = activeIndex(lines, progressMs);
    const key =
      trackKey(track) +
      "\0" +
      lines.length +
      "\0" +
      idx +
      "\0" +
      (lines[0] && lines[0].text);

    if (!track || !lines.length) {
      if (dom.meta) dom.meta.hidden = true;
      if (dom.list) {
        dom.list.hidden = true;
        dom.list.innerHTML = "";
      }
      if (dom.empty) {
        dom.empty.hidden = false;
        dom.empty.textContent = track ? "暂无歌词" : "Nothing Playing";
      }
      lastRenderKey = key;
      return;
    }

    if (dom.empty) dom.empty.hidden = true;
    if (dom.meta) {
      dom.meta.hidden = false;
      if (dom.title) dom.title.textContent = title;
      if (dom.artist) dom.artist.textContent = artist;
    }
    if (!dom.list) return;
    dom.list.hidden = false;

    const structureKey = trackKey(track) + "\0" + lines.length;
    if (!lastRenderKey.startsWith(structureKey)) {
      dom.list.innerHTML = "";
      for (let i = 0; i < lines.length; i++) {
        const li = document.createElement("li");
        li.textContent = lines[i].text;
        li.dataset.i = String(i);
        dom.list.appendChild(li);
      }
    }

    const items = dom.list.querySelectorAll("li");
    for (let i = 0; i < items.length; i++) {
      items[i].classList.toggle("is-on", i === idx);
    }
    const on = idx >= 0 ? items[idx] : null;
    if (on && typeof on.scrollIntoView === "function") {
      on.scrollIntoView({ block: "center", behavior: "smooth" });
    }
    lastRenderKey = key;
  }

  async function tick() {
    if (!active) return;
    try {
      await refreshSettings();
      const q = await apiGet("/api/query");
      failStreak = 0;
      const player = q.player || {};
      const track = player.hasSong ? q.track || null : null;
      const progressMs =
        typeof player.seekbarCurrentPosition === "number"
          ? Math.round(player.seekbarCurrentPosition * 1000)
          : 0;
      const lines = track ? await ensureLyrics(track) : [];
      render(track, lines, progressMs + (settings.lyricOffsetMs || 0));
    } catch (_) {
      failStreak = Math.min(12, failStreak + 1);
      if (!lyricCache.lines.length) render(null, [], 0);
    }
  }

  function clearTimer() {
    if (timer) {
      clearTimeout(timer);
      timer = null;
    }
  }

  function schedule() {
    clearTimer();
    if (!active) return;
    const delay =
      failStreak <= 0
        ? settings.pollMs
        : Math.min(60000, Math.round(settings.pollMs * Math.pow(2, failStreak)));
    timer = setTimeout(async function () {
      await tick();
      schedule();
    }, delay);
  }

  function onEnter() {
    active = true;
    void (async function () {
      await refreshSettings();
      await tick();
      schedule();
    })();
  }

  function onLeave() {
    active = false;
    clearTimer();
  }

  if (window.hub && window.hub.panel) {
    window.hub.panel.onEnter(onEnter);
    window.hub.panel.onLeave(onLeave);
  } else {
    window.addEventListener("message", function (ev) {
      const d = ev.data;
      if (!d || d.channel !== "island-panel-lifecycle-fwd") return;
      if (d.phase === "enter") onEnter();
      if (d.phase === "leave") onLeave();
    });
  }
})();

/**
 * Now Playing island panel — visual port of
 * https://github.com/Widdit/now-playing-service/tree/master/Assets/PublicExample
 * (iOS 歌曲组件). Data via hub.fetch / hub.media.
 */
(function () {
  const DEFAULTS = {
    TITLE: "Nothing Playing",
    ARTIST: "Get the music started",
  };
  const NUM_BARS = 6;

  let settings = {
    apiBase: "http://127.0.0.1:9863",
    pollMs: 1200,
  };
  let timer = null;
  let failStreak = 0;
  let currentCoverUrl = "";
  let currentTitleStr = "";
  let currentArtistStr = "";
  let isPausedGlobal = true;
  let pauseIconHoldUntil = 0;
  let pauseIconAnimTimer = null;
  let coverImageObj = null;
  let lastProgressSec = 0;
  let lastDurationSec = 0;
  let waveTimer = null;
  let lastDraw = 0;

  const dom = {
    titleContainer: document.getElementById("track-title"),
    titleScroller: document.getElementById("title-scroller"),
    artist: document.getElementById("track-artist"),
    cover: document.getElementById("album-cover"),
    timeCurrent: document.getElementById("time-current"),
    timeRemaining: document.getElementById("time-remaining"),
    progressFill: document.getElementById("progress-fill"),
    playPauseBtn: document.getElementById("play-pause-btn"),
    waveformCanvas: document.getElementById("waveform-canvas"),
    prevBtn: document.getElementById("prev-btn"),
    nextBtn: document.getElementById("next-btn"),
    openAppBtn: document.getElementById("open-app-btn"),
  };
  const ctx = dom.waveformCanvas ? dom.waveformCanvas.getContext("2d") : null;

  const springs = [];
  for (let i = 0; i < NUM_BARS; i++) {
    springs.push({
      pos: 0.2 + Math.random() * 0.3,
      vel: 0,
      target: 0.4,
    });
  }

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
    settings = {
      apiBase:
        String(all.apiBase || "http://127.0.0.1:9863").trim() ||
        "http://127.0.0.1:9863",
      pollMs: Math.max(500, Number(all.pollMs) || 1200),
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

  async function apiPost(path, body) {
    const res = await hub().fetch(joinUrl(settings.apiBase, path), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body || {}),
      timeoutMs: 4000,
    });
    if (!res || !res.ok) throw new Error("HTTP " + (res && res.status));
    try {
      return JSON.parse(res.body || "null");
    } catch (_) {
      return res.body;
    }
  }

  function formatTime(seconds) {
    const n = Math.max(0, Math.floor(Number(seconds) || 0));
    const m = Math.floor(n / 60);
    const s = n % 60;
    return m + ":" + String(s).padStart(2, "0");
  }

  function loadCoverImage(src) {
    return new Promise(function (resolve) {
      if (!src) {
        resolve(null);
        return;
      }
      const img = new Image();
      img.crossOrigin = "anonymous";
      img.onload = function () {
        resolve(img);
      };
      img.onerror = function () {
        resolve(null);
      };
      img.src = src;
    });
  }

  async function resolveCoverSrc(coverUrl) {
    if (!coverUrl) return "";
    if (String(coverUrl).startsWith("data:")) return coverUrl;
    try {
      const b64 = await apiPost("/api/cover/convert", { cover_url: coverUrl });
      if (typeof b64 === "string" && b64) {
        return b64.indexOf("data:") === 0 ? b64 : "data:image/jpeg;base64," + b64;
      }
    } catch (_) {}
    // fallback: absolute API cover if relative
    if (/^https?:\/\//i.test(coverUrl)) return coverUrl;
    return joinUrl(settings.apiBase, coverUrl);
  }

  function updateSprings(dt) {
    const playing = !isPausedGlobal;
    for (let i = 0; i < springs.length; i++) {
      const s = springs[i];
      if (playing) {
        if (Math.random() < 0.08) {
          s.target = 0.25 + Math.random() * 0.75;
        }
      } else {
        s.target = 0.12 + (i % 3) * 0.04;
      }
      const stiffness = playing ? 180 : 90;
      const damping = playing ? 12 : 18;
      const force = (s.target - s.pos) * stiffness - s.vel * damping;
      s.vel += force * dt;
      s.pos += s.vel * dt;
      s.pos = Math.max(0.05, Math.min(1, s.pos));
    }
  }

  function drawWaveform() {
    if (!ctx || !dom.waveformCanvas) return;
    const canvas = dom.waveformCanvas;
    const rect = canvas.getBoundingClientRect();
    const w = Math.max(1, Math.floor(rect.width * (window.devicePixelRatio || 1)));
    const h = Math.max(1, Math.floor(rect.height * (window.devicePixelRatio || 1)));
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    ctx.clearRect(0, 0, w, h);

    const gap = Math.max(2, w * 0.08);
    const barW = (w - gap * (NUM_BARS - 1)) / NUM_BARS;
    const radius = Math.min(barW / 2, h * 0.12);

    for (let i = 0; i < NUM_BARS; i++) {
      const bh = Math.max(h * 0.12, springs[i].pos * h);
      const x = i * (barW + gap);
      const y = h - bh;
      ctx.save();
      // rounded bar path
      const r = Math.min(radius, barW / 2, bh / 2);
      ctx.beginPath();
      ctx.moveTo(x + r, y);
      ctx.arcTo(x + barW, y, x + barW, y + bh, r);
      ctx.arcTo(x + barW, y + bh, x, y + bh, r);
      ctx.arcTo(x, y + bh, x, y, r);
      ctx.arcTo(x, y, x + barW, y, r);
      ctx.closePath();
      ctx.clip();

      if (coverImageObj) {
        const imgAspect = coverImageObj.width / coverImageObj.height;
        const canvasAspect = w / h;
        let sx = 0;
        let sy = 0;
        let sw = coverImageObj.width;
        let sh = coverImageObj.height;
        if (imgAspect > canvasAspect) {
          sh = coverImageObj.height;
          sw = sh * canvasAspect;
          sx = (coverImageObj.width - sw) / 2;
        } else {
          sw = coverImageObj.width;
          sh = sw / canvasAspect;
          sy = (coverImageObj.height - sh) / 2;
        }
        ctx.drawImage(coverImageObj, sx, sy, sw, sh, 0, 0, w, h);
        ctx.fillStyle = "rgba(0,0,0,0.18)";
        ctx.fillRect(x, y, barW, bh);
      } else {
        ctx.fillStyle = "rgba(136,133,139,0.85)";
        ctx.fillRect(x, y, barW, bh);
      }
      ctx.restore();
    }
  }

  function waveLoop(ts) {
    if (!lastDraw) lastDraw = ts;
    const dt = Math.min(0.05, (ts - lastDraw) / 1000);
    lastDraw = ts;
    updateSprings(dt);
    if (ts - (waveLoop._lastPaint || 0) > 32) {
      waveLoop._lastPaint = ts;
      drawWaveform();
    }
    waveTimer = requestAnimationFrame(waveLoop);
  }

  function startWave() {
    if (waveTimer) return;
    lastDraw = 0;
    waveTimer = requestAnimationFrame(waveLoop);
  }

  function stopWave() {
    if (waveTimer) cancelAnimationFrame(waveTimer);
    waveTimer = null;
  }

  function coercePaused(v) {
    if (v === false || v === 0 || v === "false" || v === "0") return false;
    if (v === true || v === 1 || v === "true" || v === "1") return true;
    return true;
  }

  function applyPlayPauseVisual(playing) {
    const btn = dom.playPauseBtn;
    if (!btn) return;
    const playIcon = btn.querySelector(".icon-play");
    const pauseIcon = btn.querySelector(".icon-pause");
    btn.classList.toggle("is-playing", playing);
    btn.classList.toggle("is-play", !playing);
    if (playIcon) playIcon.toggleAttribute("hidden", playing);
    if (pauseIcon) pauseIcon.toggleAttribute("hidden", !playing);
  }

  /**
   * 缩放切换：先缩着旧图标 → 最小时换新图标 → 再弹回（对齐 PublicExample）
   * @param {boolean} isPaused
   * @param {{ silent?: boolean }} [opts] silent=true 时不动画（首屏）
   */
  function updatePlayPauseIcon(isPaused, opts) {
    const btn = dom.playPauseBtn;
    if (!btn) return;
    const playing = !coercePaused(isPaused);
    const already = btn.classList.contains("is-playing") === playing;
    if (already) {
      applyPlayPauseVisual(playing);
      return;
    }
    if (opts && opts.silent) {
      if (pauseIconAnimTimer) {
        window.clearTimeout(pauseIconAnimTimer);
        pauseIconAnimTimer = null;
      }
      btn.classList.remove("animating");
      applyPlayPauseVisual(playing);
      return;
    }
    if (pauseIconAnimTimer) {
      window.clearTimeout(pauseIconAnimTimer);
      pauseIconAnimTimer = null;
    }
    btn.classList.add("animating");
    pauseIconAnimTimer = window.setTimeout(function () {
      pauseIconAnimTimer = null;
      applyPlayPauseVisual(playing);
      // 下一帧再去掉 animating，确保已换成新图标再放大
      requestAnimationFrame(function () {
        btn.classList.remove("animating");
      });
    }, 200);
  }

  function checkTitleScroll() {
    const scroller = dom.titleScroller;
    const container = dom.titleContainer;
    if (!scroller || !container) return;
    const span = scroller.querySelector("span");
    if (!span) return;
    // duplicate for seamless scroll
    const text = span.textContent || "";
    scroller.innerHTML = "";
    const a = document.createElement("span");
    a.textContent = text;
    scroller.appendChild(a);
    const need = a.scrollWidth > container.clientWidth + 2;
    container.classList.toggle("is-scrolling", need);
    scroller.classList.toggle("animate", need);
    if (need) {
      const b = document.createElement("span");
      b.textContent = text;
      scroller.appendChild(b);
      const distance = a.scrollWidth + 32;
      scroller.style.animationDuration = Math.max(distance / 32, 5) + "s";
    } else {
      scroller.style.animationDuration = "";
    }
  }

  function setProgress(currentSec, durationSec) {
    lastProgressSec = currentSec;
    lastDurationSec = durationSec;
    if (dom.timeCurrent) dom.timeCurrent.textContent = formatTime(currentSec);
    if (dom.timeRemaining) {
      dom.timeRemaining.textContent = durationSec
        ? "-" + formatTime(Math.max(0, durationSec - currentSec))
        : "-0:00";
    }
    const pct = durationSec > 0 ? Math.min(100, (currentSec / durationSec) * 100) : 0;
    if (dom.progressFill) dom.progressFill.style.width = pct + "%";
  }

  async function updateUI(data, opts) {
    const hasData = data && data.player && data.player.hasSong && data.track && data.track.title;
    const player = hasData ? data.player : { isPaused: true, seekbarCurrentPosition: 0 };
    const track = hasData ? data.track : {};
    const displayTitle = hasData ? track.title : DEFAULTS.TITLE;
    const displayArtist = hasData ? track.author || "" : DEFAULTS.ARTIST;
    const displayCoverUrl = hasData ? track.cover || "" : "";

    // 点击后短时锁定本地态，避免 API 尚未跟上时又闪回双竖线再切三角形
    if (Date.now() >= pauseIconHoldUntil) {
      isPausedGlobal = coercePaused(player.isPaused);
    }
    updatePlayPauseIcon(isPausedGlobal, {
      silent: Boolean(opts && opts.silentIcon),
    });

    if (displayTitle !== currentTitleStr) {
      currentTitleStr = displayTitle;
      if (dom.titleContainer) dom.titleContainer.title = displayTitle;
      if (dom.titleScroller) {
        dom.titleScroller.innerHTML = "";
        const span = document.createElement("span");
        span.textContent = displayTitle;
        dom.titleScroller.appendChild(span);
      }
      window.setTimeout(checkTitleScroll, 0);
    }
    if (displayArtist !== currentArtistStr) {
      currentArtistStr = displayArtist;
      if (dom.artist) {
        dom.artist.textContent = displayArtist;
        dom.artist.title = displayArtist;
      }
    }

    if (displayCoverUrl !== currentCoverUrl) {
      currentCoverUrl = displayCoverUrl;
      if (dom.cover) dom.cover.classList.remove("loaded");
      const src = await resolveCoverSrc(displayCoverUrl);
      if (dom.cover) {
        if (src) {
          dom.cover.src = src;
          coverImageObj = await loadCoverImage(src);
          if (coverImageObj) dom.cover.classList.add("loaded");
        } else {
          dom.cover.removeAttribute("src");
          coverImageObj = null;
        }
      }
    }

    const current = Number(player.seekbarCurrentPosition) || 0;
    const duration =
      Number(track.duration) ||
      Number(player.seekbarCurrentPositionMax) ||
      Number(player.duration) ||
      0;
    setProgress(current, duration);
  }

  async function tick() {
    try {
      const q = await apiGet("/api/query");
      failStreak = 0;
      await updateUI(q);
    } catch (err) {
      failStreak = Math.min(8, failStreak + 1);
      await updateUI(null, { silentIcon: true });
    }
  }

  function nextDelay() {
    if (failStreak <= 0) return settings.pollMs;
    // Park after sustained offline so opening the panel cannot spam Host.
    if (failStreak >= 5) return 60000;
    return Math.min(30000, Math.max(2500, settings.pollMs * Math.pow(2, failStreak)));
  }

  function schedule() {
    if (timer) clearTimeout(timer);
    timer = null;
    const loop = async function () {
      await tick();
      timer = window.setTimeout(loop, nextDelay());
    };
    void loop();
  }

  function stopPoll() {
    if (timer) clearTimeout(timer);
    timer = null;
  }

  async function sendMedia(action) {
    try {
      if (hub().media && hub().media.sendKey) {
        await hub().media.sendKey(action);
        return;
      }
    } catch (_) {}
    // fallback Now Playing HTTP if available
    try {
      await apiPost("/api/media/" + action, {});
    } catch (_) {}
  }

  function bindControls() {
    function tap(el, action) {
      if (!el) return;
      const go = function (e) {
        e.preventDefault();
        e.stopPropagation();
        if (action === "play_pause") {
          isPausedGlobal = !isPausedGlobal;
          pauseIconHoldUntil = Date.now() + 900;
          updatePlayPauseIcon(isPausedGlobal);
        } else {
          el.classList.add("animating");
          window.setTimeout(function () {
            el.classList.remove("animating");
          }, 280);
        }
        void sendMedia(action).then(function () {
          window.setTimeout(function () {
            void tick();
          }, 280);
        });
      };
      el.addEventListener("click", go);
      el.addEventListener("keydown", function (e) {
        if (e.key === "Enter" || e.key === " ") go(e);
      });
    }
    tap(dom.prevBtn, "previous");
    tap(dom.nextBtn, "next");
    tap(dom.playPauseBtn, "play_pause");

    if (dom.openAppBtn) {
      const openApp = function (e) {
        e.preventDefault();
        e.stopPropagation();
        if (dom.openAppBtn.classList.contains("is-unbound")) return;
        dom.openAppBtn.classList.add("animating");
        window.setTimeout(function () {
          dom.openAppBtn.classList.remove("animating");
        }, 280);
        void openBoundApp();
      };
      dom.openAppBtn.addEventListener("click", openApp);
      dom.openAppBtn.addEventListener("keydown", function (e) {
        if (e.key === "Enter" || e.key === " ") openApp(e);
      });
    }
  }

  async function refreshOpenAppBtn() {
    if (!dom.openAppBtn) return;
    let key = null;
    try {
      const h = hub();
      if (h.island && h.island.getBoundTray) {
        key = await h.island.getBoundTray();
      }
    } catch (err) {
      console.warn("[now-playing] getBoundTray", err);
      key = null;
    }
    const bound = typeof key === "string" && key.trim().length > 0;
    // Always show the control; muted when unbound so users notice the affordance.
    dom.openAppBtn.hidden = false;
    dom.openAppBtn.removeAttribute("hidden");
    dom.openAppBtn.classList.toggle("is-unbound", !bound);
    dom.openAppBtn.title = bound ? "打开应用" : "请先在插件详情中绑定打开应用托盘";
    dom.openAppBtn.setAttribute("aria-disabled", bound ? "false" : "true");
  }

  async function openBoundApp() {
    try {
      const h = hub();
      if (!h.island || !h.island.openBoundTray) return;
      if (dom.openAppBtn && dom.openAppBtn.classList.contains("is-unbound")) {
        await refreshOpenAppBtn();
        if (dom.openAppBtn.classList.contains("is-unbound")) return;
      }
      await h.island.openBoundTray();
      if (h.panel && h.panel.close) {
        try {
          h.panel.close();
        } catch (_) {}
      }
    } catch (err) {
      console.warn("[now-playing] openBoundTray", err);
      void refreshOpenAppBtn();
    }
  }

  async function boot() {
    await refreshSettings();
    bindControls();
    startWave();
    await updateUI(null, { silentIcon: true });
    await refreshOpenAppBtn();

    const h = hub();
    if (h.panel && h.panel.onEnter) {
      h.panel.onEnter(function () {
        failStreak = 0;
        schedule();
        startWave();
        window.setTimeout(checkTitleScroll, 50);
        void refreshOpenAppBtn();
      });
    }
    if (h.panel && h.panel.onLeave) {
      h.panel.onLeave(function () {
        stopPoll();
      });
    }
    if (h.settings && h.settings.subscribe) {
      h.settings.subscribe(function () {
        void refreshSettings();
        void refreshOpenAppBtn();
      });
    }
    window.addEventListener("message", function (ev) {
      var d = ev && ev.data;
      if (!d || d.channel !== "island-prefs-fwd") return;
      void refreshOpenAppBtn();
    });
    window.addEventListener("resize", function () {
      checkTitleScroll();
      drawWaveform();
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot();
    });
  } else {
    void boot();
  }
})();

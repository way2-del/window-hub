/**
 * 系统监控 — 快捷区风扇
 * 转速随 max(CPU,GPU)；颜色跟状态栏 chrome。
 * 用 rAF 累加角度调速，避免定时重绘 DOM / 改 animation-duration 导致卡顿。
 */
(function () {
  const POLL_MS = 2500;

  const state = {
    tempC: null,
    snap: null,
    timer: null,
    mounted: false,
    /** degrees per second */
    degPerSec: 360 / 3.2,
    angle: 0,
    lastTs: 0,
    raf: 0,
  };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function fanSvg() {
    return `<svg class="sm-fan" viewBox="0 0 1024 1024" xmlns="http://www.w3.org/2000/svg" aria-hidden="true" focusable="false">
      <path fill="currentColor" d="M970.8544 601.68192C954.44992 497.4592 856.6784 434.82112 734.49472 448.33792c-51.0464 5.632-141.28128 48.10752-152.4224 21.3504-19.43552-91.21792 94.6176-117.7088 177.8176-121.9584 108.4416-5.56032 144.55808-67.4304 91.904-161.0752C795.4944 86.528 654.26432 26.0608 562.13504 62.63808c-94.79168 37.632-134.06208 141.06624-105.09312 279.20384 7.424 35.1744 41.6768 81.8688-13.59872 96.6656-41.18528 11.0592-68.52608-29.9008-78.81728-71.2192-8.192-33.024-16.7424-66.47808-19.83488-100.2496-10.73152-117.47328-74.752-147.52768-174.2336-80.128-98.7136 66.9184-148.14208 195.6352-101.8368 283.648 59.15648 112.4352 160.43008 121.4976 271.0528 97.04448 35.70688-7.90528 83.88608-41.5232 97.35168 15.33952 9.0112 38.144-32.768 63.6928-72.6528 79.0016-33.3824 12.84096-66.01728 16.7936-100.98688 18.52416-110.41792 5.43744-142.25408 64.64512-88.24832 160.2048C231.936 941.056 375.68512 999.59808 466.30912 959.232c102.16448-45.49632 160.0512-164.52608 107.2128-304.14848-4.608-25.3952-35.2768-57.0368-5.80608-70.5024 28.18048-12.8512 53.1968 8.82688 67.09248 28.7744 32.5632 46.7968 46.4896 97.6896 40.0896 158.68928-7.95648 75.8272 34.5088 112.45568 113.3568 98.0992 102.84032-18.75968 199.54688-160.74752 182.59968-268.46208z m-462.56128-48.9984c-22.1696 0-40.1408-17.9712-40.1408-40.1408 0-22.1696 17.9712-40.1408 40.1408-40.1408 22.1696 0 40.1408 17.9712 40.1408 40.1408 0 22.1696-17.96096 40.1408-40.1408 40.1408z m0 0"/>
    </svg>`;
  }

  /** Map effective °C → seconds per revolution (lower = faster). */
  function spinDurSec(tempC) {
    if (tempC == null || !Number.isFinite(tempC)) return 3.2;
    const t = Math.max(30, Math.min(95, tempC));
    return 3.4 - ((t - 30) / 65) * 3.05;
  }

  function tipText(snap) {
    const parts = [];
    if (snap && snap.effectiveTempC != null) {
      parts.push(`温度 ${Math.round(snap.effectiveTempC)}°`);
    } else {
      parts.push("温度暂无");
    }
    if (snap && snap.cpu) {
      parts.push(`CPU ${Math.round(snap.cpu.usagePct)}%`);
    }
    if (snap && snap.memory) {
      parts.push(`内存 ${Math.round(snap.memory.usagePct)}%`);
    }
    return parts.join(" · ");
  }

  function setSpeedFromTemp(tempC) {
    const dur = spinDurSec(tempC);
    state.degPerSec = 360 / Math.max(0.2, dur);
  }

  function tick(ts) {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      state.raf = window.requestAnimationFrame(tick);
      return;
    }
    if (!state.lastTs) state.lastTs = ts;
    const dt = Math.min(0.064, (ts - state.lastTs) / 1000);
    state.lastTs = ts;
    state.angle = (state.angle + state.degPerSec * dt) % 360;
    const fan = document.querySelector(".sm-fan");
    if (fan) {
      fan.style.transform = `rotate(${state.angle.toFixed(2)}deg)`;
    }
    state.raf = window.requestAnimationFrame(tick);
  }

  function mount() {
    if (state.mounted) return;
    const root = document.getElementById("root") || document.body;
    root.innerHTML = `<div class="sm-strip">
      <button type="button" class="sm-fan-btn" aria-label="系统监控">${fanSvg()}</button>
    </div>`;
    const btn = root.querySelector(".sm-fan-btn");
    if (btn) {
      btn.addEventListener("click", () => {
        try {
          hub().popup.open({});
        } catch (e) {
          console.error(e);
        }
      });
    }
    state.mounted = true;
    setSpeedFromTemp(state.tempC);
    state.lastTs = 0;
    state.raf = window.requestAnimationFrame(tick);
    try {
      hub().shortcuts.requestSize({ width: 34 });
    } catch (_) {}
  }

  function applyLive() {
    const btn = document.querySelector(".sm-fan-btn");
    if (btn) btn.title = tipText(state.snap);
    setSpeedFromTemp(state.tempC);
  }

  async function refresh() {
    try {
      const snap = await hub().sysmon.snapshot();
      state.snap = snap;
      state.tempC =
        snap && snap.effectiveTempC != null && Number.isFinite(snap.effectiveTempC)
          ? snap.effectiveTempC
          : null;
    } catch (e) {
      console.error("[sysmon]", e);
      state.tempC = null;
      state.snap = null;
    }
    applyLive();
  }

  async function boot() {
    try {
      const b = await hub().shortcuts.getBounds();
      const h = b && typeof b.height === "number" ? b.height : 28;
      document.documentElement.style.setProperty("--wh-bar-h", `${h}px`);
    } catch (_) {
      document.documentElement.style.setProperty("--wh-bar-h", "28px");
    }
    if (!document.getElementById("root")) {
      const root = document.createElement("div");
      root.id = "root";
      document.body.appendChild(root);
    }
    mount();
    await refresh();
    state.timer = window.setInterval(() => {
      void refresh();
    }, POLL_MS);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void boot());
  } else {
    void boot();
  }
})();

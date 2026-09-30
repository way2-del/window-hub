/**
 * 世界时钟 — 快捷区
 * 文案显示「非本机」时区的时间，避免与顶栏本机时钟重复；点击打开双时区弹窗。
 */
(function () {
  const ZONE_SHORT = {
    "__local__": "本机",
    "Asia/Shanghai": "中国",
    "Asia/Hong_Kong": "香港",
    "Asia/Taipei": "台北",
    "Asia/Tokyo": "东京",
    "Asia/Seoul": "首尔",
    "Asia/Singapore": "新加坡",
    "Asia/Bangkok": "曼谷",
    "Asia/Dubai": "迪拜",
    "Asia/Kolkata": "新德里",
    "Europe/London": "伦敦",
    "Europe/Paris": "巴黎",
    "Europe/Berlin": "柏林",
    "Europe/Moscow": "莫斯科",
    "America/New_York": "纽约",
    "America/Chicago": "芝加哥",
    "America/Denver": "丹佛",
    "America/Los_Angeles": "洛杉矶",
    "America/Sao_Paulo": "圣保罗",
    "Australia/Sydney": "悉尼",
    "Pacific/Auckland": "奥克兰",
  };

  const state = {
    zoneA: "__local__",
    zoneB: "Europe/London",
    displayParts: "time",
    hourFormat: "24",
    timer: null,
  };

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function localIana() {
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "Asia/Shanghai";
    } catch (_) {
      return "Asia/Shanghai";
    }
  }

  function resolveZone(raw) {
    const v = String(raw || "").trim();
    if (!v || v === "__local__") return localIana();
    return v;
  }

  function shortOf(raw) {
    const key = String(raw || "").trim() || "__local__";
    if (ZONE_SHORT[key]) return ZONE_SHORT[key];
    const parts = key.split("/");
    return parts[parts.length - 1].replace(/_/g, " ");
  }

  function formatContent(iana) {
    const now = new Date();
    const parts = state.displayParts.split(",");
    const values = [];
    if (parts.includes("date")) values.push(new Intl.DateTimeFormat("zh-CN", {
      timeZone: iana, month: "numeric", day: "numeric",
    }).format(now));
    if (parts.includes("weekday")) values.push(new Intl.DateTimeFormat("zh-CN", {
      timeZone: iana, weekday: "short",
    }).format(now));
    if (parts.includes("time")) values.push(new Intl.DateTimeFormat("en-US", {
      timeZone: iana, hour: "numeric", minute: "2-digit",
      hourCycle: state.hourFormat === "12" ? "h12" : "h23",
    }).format(now));
    return values.join(" ");
  }

  /** Prefer a zone that is not the machine-local clock. */
  function stripTarget() {
    const local = localIana();
    const a = resolveZone(state.zoneA);
    const b = resolveZone(state.zoneB);
    if (b !== local) return { iana: b, raw: state.zoneB };
    if (a !== local) return { iana: a, raw: state.zoneA };
    return { iana: b, raw: state.zoneB };
  }

  let lastWidth = 0;
  function reportWidth() {
    const bar = document.querySelector(".wc-strip");
    if (!bar) return;
    // Intrinsic only — do not mix getBoundingClientRect (Host iframe feedback).
    const width = Math.ceil(Math.max(bar.scrollWidth, 28));
    if (width <= 0 || width === lastWidth) return;
    try {
      hub().shortcuts.requestSize({ width });
      lastWidth = width;
    } catch (_) {}
  }

  function paint() {
    const btn = document.querySelector(".wc-chip");
    if (!btn) return;
    const t = stripTarget();
    const label = shortOf(t.raw);
    const time = formatContent(t.iana);
    btn.innerHTML = `<svg class="wc-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true"><circle cx="12" cy="12" r="9" stroke="currentColor" stroke-width="1.8"/><path d="M12 6.5V12l3.5 2" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>${time ? `<span class="wc-time">${time}</span>` : ""}`;
    btn.title = `${label} ${time} · 点击查看双时区`;
    btn.setAttribute("aria-label", btn.title);
    reportWidth();
  }

  function bind() {
    document.querySelector(".wc-chip")?.addEventListener("click", () => {
      try {
        hub().popup.open({});
      } catch (e) {
        console.error(e);
      }
    });
  }

  async function loadSettings() {
    try {
      const all = await hub().settings.getAll();
      if (all && typeof all === "object") {
        if (all.zoneA != null) state.zoneA = String(all.zoneA);
        if (all.zoneB != null) state.zoneB = String(all.zoneB);
        if (all.displayParts != null) state.displayParts = String(all.displayParts);
        if (all.hourFormat != null) state.hourFormat = String(all.hourFormat);
      }
    } catch (e) {
      console.error("[world-clock]", e);
    }
  }

  async function boot() {
    try {
      const b = await hub().shortcuts.getBounds();
      const h = b && typeof b.height === "number" ? b.height : 28;
      document.documentElement.style.setProperty("--wh-bar-h", `${h}px`);
    } catch (_) {
      document.documentElement.style.setProperty("--wh-bar-h", "28px");
    }
    bind();
    await loadSettings();
    paint();
    const bar = document.querySelector(".wc-strip");
    if (bar) new ResizeObserver(reportWidth).observe(bar);
    document.fonts?.ready.then(reportWidth);
    try {
      hub().settings.subscribe((all) => {
        if (all && typeof all === "object") {
          if (all.zoneA != null) state.zoneA = String(all.zoneA);
          if (all.zoneB != null) state.zoneB = String(all.zoneB);
        if (all.displayParts != null) state.displayParts = String(all.displayParts);
        if (all.hourFormat != null) state.hourFormat = String(all.hourFormat);
          paint();
        }
      });
    } catch (_) {}
    state.timer = window.setInterval(paint, 15_000);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void boot());
  } else {
    void boot();
  }
})();

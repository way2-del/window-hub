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

  function formatHm(iana) {
    const fmt = new Intl.DateTimeFormat("en-US", {
      timeZone: iana,
      hour: "numeric",
      minute: "2-digit",
      hour12: false,
      hourCycle: "h23",
    });
    const bag = {};
    for (const p of fmt.formatToParts(new Date())) {
      if (p.type !== "literal") bag[p.type] = p.value;
    }
    let hour = Number(bag.hour);
    if (hour === 24) hour = 0;
    return `${hour}:${String(Number(bag.minute)).padStart(2, "0")}`;
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

  function paint() {
    const btn = document.querySelector(".wc-chip");
    if (!btn) return;
    const t = stripTarget();
    const label = shortOf(t.raw);
    const time = formatHm(t.iana);
    btn.innerHTML = `<span class="wc-label">${label}</span><span class="wc-time">${time}</span>`;
    btn.title = `${label} ${time} · 点击查看双时区`;
    try {
      const w = Math.min(160, Math.max(72, label.length * 12 + 48));
      hub().shortcuts.requestSize({ width: w });
    } catch (_) {}
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
    try {
      hub().settings.subscribe((all) => {
        if (all && typeof all === "object") {
          if (all.zoneA != null) state.zoneA = String(all.zoneA);
          if (all.zoneB != null) state.zoneB = String(all.zoneB);
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

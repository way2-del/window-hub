/**
 * 世界时钟 — 弹窗：表盘 + 双栏数字时间 + 齿轮设置
 */
(function () {
  const WEEKDAYS = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

  const ZONE_OPTIONS = [
    { value: "__local__", label: "本机时区（自动）" },
    { value: "Asia/Shanghai", label: "中国标准时间" },
    { value: "Asia/Hong_Kong", label: "香港" },
    { value: "Asia/Taipei", label: "台北" },
    { value: "Asia/Tokyo", label: "东京" },
    { value: "Asia/Seoul", label: "首尔" },
    { value: "Asia/Singapore", label: "新加坡" },
    { value: "Asia/Bangkok", label: "曼谷" },
    { value: "Asia/Dubai", label: "迪拜" },
    { value: "Asia/Kolkata", label: "新德里" },
    { value: "Europe/London", label: "伦敦" },
    { value: "Europe/Paris", label: "巴黎" },
    { value: "Europe/Berlin", label: "柏林" },
    { value: "Europe/Moscow", label: "莫斯科" },
    { value: "America/New_York", label: "纽约" },
    { value: "America/Chicago", label: "芝加哥" },
    { value: "America/Denver", label: "丹佛" },
    { value: "America/Los_Angeles", label: "洛杉矶" },
    { value: "America/Sao_Paulo", label: "圣保罗" },
    { value: "Australia/Sydney", label: "悉尼" },
    { value: "Pacific/Auckland", label: "奥克兰" },
  ];

  const ZONE_LABELS = {
    "__local__": "本机时区",
    "Asia/Shanghai": "中国标准时间",
    "Asia/Hong_Kong": "香港时间",
    "Asia/Taipei": "台北时间",
    "Asia/Tokyo": "东京时间",
    "Asia/Seoul": "首尔时间",
    "Asia/Singapore": "新加坡时间",
    "Asia/Bangkok": "曼谷时间",
    "Asia/Dubai": "迪拜时间",
    "Asia/Kolkata": "新德里时间",
    "Europe/London": "伦敦时间",
    "Europe/Paris": "巴黎时间",
    "Europe/Berlin": "柏林时间",
    "Europe/Moscow": "莫斯科时间",
    "America/New_York": "纽约时间",
    "America/Chicago": "芝加哥时间",
    "America/Denver": "丹佛时间",
    "America/Los_Angeles": "洛杉矶时间",
    "America/Sao_Paulo": "圣保罗时间",
    "Australia/Sydney": "悉尼时间",
    "Pacific/Auckland": "奥克兰时间",
  };

  const state = {
    zoneA: "__local__",
    zoneB: "Europe/London",
    settingsOpen: false,
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

  function labelOf(raw) {
    const key = String(raw || "").trim() || "__local__";
    if (key === "__local__") {
      const iana = localIana();
      return ZONE_LABELS[iana] || "本机时区";
    }
    if (ZONE_LABELS[key]) return ZONE_LABELS[key];
    const parts = key.split("/");
    return parts[parts.length - 1].replace(/_/g, " ") + "时间";
  }

  function formatClock(iana) {
    const d = new Date();
    const fmt = new Intl.DateTimeFormat("en-US", {
      timeZone: iana,
      hour: "numeric",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
      month: "numeric",
      day: "numeric",
      hourCycle: "h23",
    });
    const bag = {};
    for (const p of fmt.formatToParts(d)) {
      if (p.type !== "literal") bag[p.type] = p.value;
    }
    let hour = Number(bag.hour);
    if (hour === 24) hour = 0;
    const minute = Number(bag.minute);
    const second = Number(bag.second);
    const month = Number(bag.month);
    const day = Number(bag.day);
    const wdEn = new Intl.DateTimeFormat("en-US", {
      timeZone: iana,
      weekday: "short",
    }).format(d);
    const wdMap = { Sun: 0, Mon: 1, Tue: 2, Wed: 3, Thu: 4, Fri: 5, Sat: 6 };
    const wd = wdMap[wdEn] != null ? wdMap[wdEn] : 0;
    const period = hour < 12 ? "上午" : "下午";
    /** 白天 6:00–17:59 白底；夜间黑底（按该时区本地时） */
    const isDay = hour >= 6 && hour < 18;
    return {
      hour,
      minute,
      second,
      isDay,
      time: `${state.hourFormat === "12" ? hour % 12 || 12 : hour}:${String(minute).padStart(2, "0")}`,
      period: state.hourFormat === "12" ? period : "",
      dateLine: `${month}月${day}日 ${WEEKDAYS[wd]}`,
    };
  }

  function gearSvg() {
    return `<svg viewBox="0 0 24 24" fill="none" aria-hidden="true">
      <path d="M12 15.2a3.2 3.2 0 1 0 0-6.4 3.2 3.2 0 0 0 0 6.4Z" stroke="currentColor" stroke-width="1.6"/>
      <path d="M19.4 13.2v-2.4l-1.7-.3a6.6 6.6 0 0 0-.5-1.2l1-1.4-1.7-1.7-1.4 1a6.6 6.6 0 0 0-1.2-.5L13.2 4.6h-2.4l-.3 1.7c-.4.1-.8.3-1.2.5l-1.4-1-1.7 1.7 1 1.4c-.2.4-.4.8-.5 1.2l-1.7.3v2.4l1.7.3c.1.4.3.8.5 1.2l-1 1.4 1.7 1.7 1.4-1c.4.2.8.4 1.2.5l.3 1.7h2.4l.3-1.7c.4-.1.8-.3 1.2-.5l1.4 1 1.7-1.7-1-1.4c.2-.4.4-.8.5-1.2l1.7-.3Z" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>
    </svg>`;
  }

  /** 简约表盘：12 点刻度 + 小时数字；指针用 transform 驱动 */
  function dialSvg(clock) {
    const theme = clock.isDay ? "day" : "night";
    const cx = 50;
    const cy = 50;
    const ticks = [];
    for (let i = 0; i < 60; i++) {
      const a = (i / 60) * Math.PI * 2;
      const major = i % 5 === 0;
      const r1 = major ? 41.5 : 43.2;
      const r2 = 46.2;
      const x1 = cx + Math.sin(a) * r1;
      const y1 = cy - Math.cos(a) * r1;
      const x2 = cx + Math.sin(a) * r2;
      const y2 = cy - Math.cos(a) * r2;
      ticks.push(
        `<line class="wc-tick${major ? " is-hour" : ""}" x1="${x1.toFixed(2)}" y1="${y1.toFixed(2)}" x2="${x2.toFixed(2)}" y2="${y2.toFixed(2)}" />`,
      );
    }
    const nums = [];
    for (let n = 1; n <= 12; n++) {
      const a = (n / 12) * Math.PI * 2;
      const r = 34.5;
      const x = cx + Math.sin(a) * r;
      const y = cy - Math.cos(a) * r;
      nums.push(
        `<text class="wc-num" x="${x.toFixed(2)}" y="${y.toFixed(2)}" text-anchor="middle" dominant-baseline="central">${n}</text>`,
      );
    }
    const hAngle =
      ((clock.hour % 12) + clock.minute / 60 + clock.second / 3600) * 30;
    const mAngle = (clock.minute + clock.second / 60) * 6;
    const sAngle = clock.second * 6;
    return `<div class="wc-dial is-${theme}" aria-hidden="true">
      <svg class="wc-dial-svg" viewBox="0 0 100 100">
        <circle class="wc-face" cx="50" cy="50" r="48" />
        <g class="wc-ticks">${ticks.join("")}</g>
        <g class="wc-nums">${nums.join("")}</g>
        <g class="wc-hand wc-hand-h" transform="rotate(${hAngle.toFixed(3)} 50 50)">
          <line x1="50" y1="50" x2="50" y2="28" />
        </g>
        <g class="wc-hand wc-hand-m" transform="rotate(${mAngle.toFixed(3)} 50 50)">
          <line x1="50" y1="50" x2="50" y2="18" />
        </g>
        <g class="wc-hand wc-hand-s" transform="rotate(${sAngle.toFixed(3)} 50 50)">
          <line x1="50" y1="54" x2="50" y2="14" />
        </g>
        <circle class="wc-hub-ring" cx="50" cy="50" r="2.4" />
        <circle class="wc-hub" cx="50" cy="50" r="1.35" />
      </svg>
    </div>`;
  }

  function optionsHtml(selected) {
    return ZONE_OPTIONS.map((o) => {
      const sel = o.value === selected ? " selected" : "";
      return `<option value="${o.value}"${sel}>${o.label}</option>`;
    }).join("");
  }

  function colHtml(raw) {
    const iana = resolveZone(raw);
    const clock = formatClock(iana);
    return `<div class="wc-col">
      <div class="wc-name">${labelOf(raw)}</div>
      ${dialSvg(clock)}
      <div class="wc-big">${clock.time}<span class="wc-period">${clock.period}</span></div>
      <div class="wc-date">${clock.dateLine}</div>
    </div>`;
  }

  function render() {
    const app = document.getElementById("app");
    if (!app) return;
    app.innerHTML = `
      <div class="wc-toolbar">
        <button type="button" class="wc-gear${state.settingsOpen ? " is-on" : ""}" id="wc-gear" aria-label="设置" title="设置">${gearSvg()}</button>
      </div>
      <div class="wc-dual${state.settingsOpen ? " is-hidden" : ""}" id="wc-dual">
        ${colHtml(state.zoneA)}
        ${colHtml(state.zoneB)}
      </div>
      <div class="wc-settings${state.settingsOpen ? " is-open" : ""}" id="wc-settings">
        <div class="wc-settings-title">世界时钟设置</div>
        <div class="wc-field">
          <label for="wc-zone-a">时区一</label>
          <select id="wc-zone-a">${optionsHtml(state.zoneA)}</select>
        </div>
        <div class="wc-field">
          <label for="wc-zone-b">时区二（快捷区优先显示）</label>
          <select id="wc-zone-b">${optionsHtml(state.zoneB)}</select>
        </div>
        <div class="wc-field">
          <label for="wc-display-parts">快捷区显示内容</label>
          <select id="wc-display-parts">${[{"value":"time","label":"时间"},{"value":"date","label":"日期"},{"value":"weekday","label":"星期"},{"value":"date,time","label":"日期 + 时间"},{"value":"weekday,time","label":"星期 + 时间"},{"value":"date,weekday","label":"日期 + 星期"},{"value":"date,weekday,time","label":"日期 + 星期 + 时间"},{"value":"icon","label":"仅时钟图标"}].map(o => `<option value="${o.value}"${o.value === state.displayParts ? " selected" : ""}>${o.label}</option>`).join("")}</select>
        </div>
        <div class="wc-field">
          <label for="wc-hour-format">时间格式（快捷区和弹窗）</label>
          <select id="wc-hour-format"><option value="24"${state.hourFormat === "24" ? " selected" : ""}>24 小时制</option><option value="12"${state.hourFormat === "12" ? " selected" : ""}>12 小时制</option></select>
        </div>
        <p class="wc-hint">快捷区不显示本机时钟（顶栏已有）。默认时区一跟随系统，时区二为伦敦。表盘按该时区昼夜切换白/黑底。</p>
      </div>
    `;

    document.getElementById("wc-gear")?.addEventListener("click", () => {
      state.settingsOpen = !state.settingsOpen;
      render();
    });

    const bind = (id, key) => {
      const el = document.getElementById(id);
      if (!el) return;
      el.addEventListener("change", async () => {
        state[key] = el.value;
        try {
          await hub().settings.set(key, el.value);
        } catch (e) {
          console.error(e);
        }
        if (!state.settingsOpen) render();
      });
    };
    bind("wc-zone-a", "zoneA");
    bind("wc-zone-b", "zoneB");
    bind("wc-display-parts", "displayParts");
    bind("wc-hour-format", "hourFormat");
  }

  function tickClocks() {
    if (state.settingsOpen) return;
    const dual = document.getElementById("wc-dual");
    if (!dual) return;
    dual.innerHTML = `${colHtml(state.zoneA)}${colHtml(state.zoneB)}`;
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
      console.error("[world-clock popup]", e);
    }
  }

  async function boot() {
    await loadSettings();
    render();
    try {
      hub().settings.subscribe((all) => {
        if (!all || typeof all !== "object") return;
        if (all.zoneA != null) state.zoneA = String(all.zoneA);
        if (all.zoneB != null) state.zoneB = String(all.zoneB);
        if (all.displayParts != null) state.displayParts = String(all.displayParts);
        if (all.hourFormat != null) state.hourFormat = String(all.hourFormat);
        render();
      });
    } catch (_) {}
    state.timer = window.setInterval(tickClocks, 1000);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void boot());
  } else {
    void boot();
  }
})();

/**
 * 课表 — 快捷区：今日下一节（MA Design 日程）
 */
(function () {
  const PREFS_KEY = "prefs";
  const POLL_MS = 60_000;

  const RAW_EVENTS = [
    { d: "2026-09-28", t: "welcome", en: "Welcome and Intro Project Launch", cn: "欢迎仪式与导入项目启动", l: "IGLT", u: "11:00-12:00", c: "18:00-19:00" },
    { d: "2026-09-29", t: "intro", en: "Intro Project LAUNCH PART A", cn: "导入项目启动 (A部分)" },
    { d: "2026-09-30", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-10-01", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-10-02", t: "lecture", en: "FRIDAY LECTURE 'Design is unknown'", cn: "周五讲座：设计是未知的", l: "IGLT", u: "11:00-12:00", c: "18:00-19:00" },
    { d: "2026-10-05", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-10-06", t: "intro", en: "Intro Project Tutorials", cn: "导入项目辅导", l: "MADEP Building", u: "10:00-17:00", c: "17:00-24:00" },
    { d: "2026-10-07", t: "lang", en: "Language Session", cn: "语言课程", l: "Online", u: "11:15-13:00", c: "18:15-20:00" },
    { d: "2026-10-08", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-10-09", t: "lecture", en: "FRIDAY LECTURE 'Design is unknown'", cn: "周五讲座：设计是未知的", l: "IGLT", u: "11:00-12:00", c: "18:00-19:00" },
    { d: "2026-10-12", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-10-13", t: "intro", en: "Intro Project Tutorials", cn: "导入项目辅导", l: "MADEP Building", u: "10:00-17:00", c: "17:00-24:00" },
    { d: "2026-10-14", t: "lang", en: "Language Session", cn: "语言课程", l: "Online", u: "11:15-13:00", c: "18:15-20:00" },
    { d: "2026-10-15", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-10-16", t: "lecture", en: "FRIDAY LECTURE 'Design is participatory'", cn: "周五讲座：设计是参与式的", l: "IGLT", u: "11:00-12:00", c: "18:00-19:00" },
    { d: "2026-10-19", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-10-20", t: "intro", en: "Intro Project Tutorials", cn: "导入项目辅导", l: "MADEP Building", u: "10:00-17:00", c: "17:00-24:00" },
    { d: "2026-10-21", t: "lang", en: "Language Session", cn: "语言课程", l: "Online", u: "11:15-13:00", c: "18:15-20:00" },
    { d: "2026-10-22", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-10-23", t: "lecture", en: "FRIDAY LECTURE 'Design is Experimental'", cn: "周五讲座：设计是实验性的", l: "IGLT", u: "11:00-12:00", c: "18:00-19:00" },
    { d: "2026-10-26", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-10-27", t: "intro", en: "Intro Project Tutorials", cn: "导入项目辅导", l: "MADEP Building", u: "10:00-17:00", c: "18:00-01:00(+1)" },
    { d: "2026-10-28", t: "lang", en: "Language Session", cn: "语言课程", l: "Online", u: "11:15-13:00", c: "19:15-21:00" },
    { d: "2026-10-29", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-10-30", t: "lecture", en: "FRIDAY LECTURE 'Design is Ecological'", cn: "周五讲座：设计是生态的", l: "IGLT", u: "11:00-12:00", c: "19:00-20:00" },
    { d: "2026-11-02", t: "reading", en: "Reading week", cn: "阅读周" },
    { d: "2026-11-03", t: "reading", en: "Reading week", cn: "阅读周" },
    { d: "2026-11-04", t: "reading", en: "Reading week", cn: "阅读周" },
    { d: "2026-11-05", t: "reading", en: "Reading week", cn: "阅读周" },
    { d: "2026-11-06", t: "reading", en: "Reading week", cn: "阅读周" },
    { d: "2026-11-09", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-11-10", t: "intro", en: "Intro Project Tutorials", cn: "导入项目辅导", l: "MADEP Building", u: "10:00-17:00", c: "18:00-01:00(+1)" },
    { d: "2026-11-11", t: "lang", en: "Language Session", cn: "语言课程", l: "Online", u: "11:15-13:00", c: "19:15-21:00" },
    { d: "2026-11-12", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-11-13", t: "lecture", en: "FRIDAY LECTURE 'Design is Speculative'", cn: "周五讲座：设计是推测性的", l: "IGLT", u: "11:00-12:00", c: "19:00-20:00" },
    { d: "2026-11-16", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-11-17", t: "intro", en: "Intro Project Tutorials", cn: "导入项目辅导", l: "MADEP Building", u: "10:00-17:00", c: "18:00-01:00(+1)" },
    { d: "2026-11-18", t: "transloc", en: "Translocality 1 Launch", cn: "跨地域项目1 启动", l: "PSH LG02", u: "14:00-15:00", c: "22:00-23:00" },
    { d: "2026-11-19", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-11-20", t: "lecture", en: "FRIDAY LECTURE 'Design is Embodied'", cn: "周五讲座：设计是具身的", l: "IGLT", u: "11:00-12:00", c: "19:00-20:00" },
    { d: "2026-11-23", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-11-24", t: "intro", en: "Intro Project Tutorials", cn: "导入项目辅导", l: "MADEP Building", u: "10:00-17:00", c: "18:00-01:00(+1)" },
    { d: "2026-11-25", t: "lang", en: "Language Session", cn: "语言课程", l: "Online", u: "11:15-13:00", c: "19:15-21:00" },
    { d: "2026-11-26", t: "self", en: "Self-directed project work", cn: "自主项目推进" },
    { d: "2026-11-27", t: "lecture", en: "FRIDAY LECTURE 'Design is Translation'", cn: "周五讲座：设计是转译的", l: "IGLT", u: "11:00-12:00", c: "19:00-20:00" },
    { d: "2026-11-30", t: "assist", en: "Degree Show Assist", cn: "毕业展协助" },
    { d: "2026-12-01", t: "assist", en: "Degree Show Assist", cn: "毕业展协助" },
    { d: "2026-12-02", t: "assist", en: "Degree Show Assist", cn: "毕业展协助" },
    { d: "2026-12-03", t: "assist", en: "Degree Show Assist", cn: "毕业展协助" },
    { d: "2026-12-04", t: "assist", en: "Degree Show Assist", cn: "毕业展协助" },
    { d: "2026-12-07", t: "install", en: "Degree Show install", cn: "毕业展布展" },
    { d: "2026-12-08", t: "install", en: "Degree Show install", cn: "毕业展布展" },
    { d: "2026-12-09", t: "install", en: "Degree Show install", cn: "毕业展布展" },
    { d: "2026-12-10", t: "madep", en: "MADEP private view", cn: "MADEP 专业展预展" },
    { d: "2026-12-11", t: "open", en: "Degree Show Open", cn: "毕业展开放日" },
    { d: "2026-12-14", t: "takedown", en: "TAKE DOWN SHOW", cn: "撤展" },
    { d: "2026-12-15", t: "takedown", en: "TAKE DOWN SHOW", cn: "撤展" },
    { d: "2026-12-18", t: "takedown", en: "LAST DAY OF TERM", cn: "学期最后一天" },
  ];

  const state = { tz: "CN" };
  let lastWidth = 0;

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

  function trunc(s, n) {
    const t = String(s || "");
    return t.length <= n ? t : t.slice(0, n - 1) + "…";
  }

  function pad(n) {
    return String(n).padStart(2, "0");
  }

  function formatYmd(d) {
    return d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate());
  }

  function parseStartMin(time) {
    if (!time) return -1;
    const m = String(time).match(/^(\d{1,2}):(\d{2})/);
    if (!m) return -1;
    return +m[1] * 60 + +m[2];
  }

  function todayEvents() {
    const ymd = formatYmd(new Date());
    return RAW_EVENTS.filter((e) => e.d === ymd);
  }

  function nextEvent() {
    const list = todayEvents();
    if (!list.length) return null;
    const now = new Date().getHours() * 60 + new Date().getMinutes();
    const timed = list
      .map((e) => ({ e, start: parseStartMin(state.tz === "CN" ? e.c : e.u) }))
      .filter((x) => x.start >= 0);
    if (!timed.length) return list[0];
    const upcoming = timed.find((x) => x.start >= now - 30);
    return (upcoming || timed[timed.length - 1]).e;
  }

  function reportWidth() {
    const bar = document.getElementById("bar");
    if (!bar) return;
    const width = Math.ceil(Math.max(bar.scrollWidth, 28));
    if (width <= 0 || width === lastWidth) return;
    try {
      hub().shortcuts.requestSize({ width });
      lastWidth = width;
    } catch (_) {}
  }

  function openPopup() {
    const h = hub();
    if (!h.popup || !h.popup.open) return;
    h.popup.open({}).catch(console.error);
  }

  async function loadPrefs() {
    try {
      const raw = await hub().storage.get(PREFS_KEY);
      if (raw && (raw.tz === "UK" || raw.tz === "CN")) state.tz = raw.tz;
    } catch (_) {}
  }

  function render() {
    const root = document.getElementById("bar");
    if (!root) return;
    const ev = nextEvent();
    const cn = state.tz === "CN";
    let label = cn ? "课表" : "Schedule";
    let time = "";
    let tip = cn ? "打开 MA Design 周课表" : "Open MA Design week schedule";

    if (ev) {
      label = trunc(cn ? ev.cn : ev.en, 8);
      time = (cn ? ev.c : ev.u) || "";
      if (time) time = time.split("-")[0];
      tip =
        (cn ? ev.cn : ev.en) +
        (ev.l ? " · " + ev.l : "") +
        ((cn ? ev.c : ev.u) ? " · " + (cn ? ev.c : ev.u) : "");
    }

    root.innerHTML =
      '<button type="button" class="cs-chip" id="cs-chip" title="' +
      escapeHtml(tip) +
      '">' +
      '<svg class="cs-icon" viewBox="0 0 24 24" fill="none" aria-hidden="true">' +
      '<rect x="4" y="5" width="16" height="15" rx="2.5" stroke="currentColor" stroke-width="1.7"/>' +
      '<path d="M4 10h16M9 3v3M15 3v3" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/>' +
      '<rect x="7.5" y="13" width="3.2" height="3.2" rx="0.6" fill="currentColor" opacity="0.85"/>' +
      "</svg>" +
      '<span class="cs-label">' +
      escapeHtml(label) +
      "</span>" +
      (time ? '<span class="cs-time">' + escapeHtml(time) + "</span>" : "") +
      "</button>";

    document.getElementById("cs-chip")?.addEventListener("click", openPopup);
    reportWidth();
  }

  async function boot() {
    try {
      const b = await hub().shortcuts.getBounds();
      const h = b && typeof b.height === "number" ? b.height : 28;
      document.documentElement.style.setProperty("--wh-bar-h", h + "px");
    } catch (_) {
      document.documentElement.style.setProperty("--wh-bar-h", "28px");
    }
    await loadPrefs();
    render();
    document.fonts?.ready.then(reportWidth);
    setInterval(function () {
      void loadPrefs().then(render).catch(console.error);
    }, POLL_MS);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

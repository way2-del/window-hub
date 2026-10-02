/**
 * 课表 — 岛面板：本周整表一览（可下滑）
 */
(function () {
  const PREFS_KEY = "prefs";

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

  const WD = {
    CN: ["日", "一", "二", "三", "四", "五", "六"],
    UK: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
  };

  const state = { tz: "CN" };

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

  function pad(n) {
    return String(n).padStart(2, "0");
  }

  function formatYmd(d) {
    return d.getFullYear() + "-" + pad(d.getMonth() + 1) + "-" + pad(d.getDate());
  }

  function startOfDay(d) {
    return new Date(d.getFullYear(), d.getMonth(), d.getDate());
  }

  function addDays(d, n) {
    const x = new Date(d);
    x.setDate(x.getDate() + n);
    return x;
  }

  function startOfWeek(d) {
    const date = startOfDay(d);
    const day = date.getDay();
    const diff = day === 0 ? -6 : 1 - day;
    return addDays(date, diff);
  }

  function eventsFor(ymd) {
    return RAW_EVENTS.filter((e) => e.d === ymd);
  }

  async function loadPrefs() {
    try {
      const raw = await hub().storage.get(PREFS_KEY);
      if (raw && (raw.tz === "UK" || raw.tz === "CN")) state.tz = raw.tz;
    } catch (_) {}
  }

  function render() {
    const root = document.getElementById("root");
    if (!root) return;
    const cn = state.tz === "CN";
    const labels = WD[state.tz];
    const weekStart = startOfWeek(new Date());
    const todayYmd = formatYmd(startOfDay(new Date()));
    const end = addDays(weekStart, 6);
    const range =
      pad(weekStart.getMonth() + 1) +
      "/" +
      pad(weekStart.getDate()) +
      " – " +
      pad(end.getMonth() + 1) +
      "/" +
      pad(end.getDate());

    let body = "";
    let count = 0;
    for (let i = 0; i < 7; i++) {
      const day = addDays(weekStart, i);
      const ymd = formatYmd(day);
      const list = eventsFor(ymd);
      if (!list.length) continue;
      count += list.length;
      const isToday = ymd === todayYmd;
      const dayLabel =
        (cn ? "周" + labels[day.getDay()] : labels[day.getDay()]) +
        " · " +
        pad(day.getMonth() + 1) +
        "/" +
        pad(day.getDate()) +
        (isToday ? (cn ? " · 今天" : " · Today") : "");
      body += list
        .map(function (e) {
          const title = cn ? e.cn : e.en;
          const time = cn ? e.c : e.u;
          return (
            '<article class="cs-panel-card' +
            (isToday ? " is-today" : "") +
            '">' +
            '<div class="cs-panel-card-day">' +
            escapeHtml(dayLabel) +
            "</div>" +
            '<div class="cs-panel-time">' +
            escapeHtml(time || (cn ? "全天" : "All day")) +
            "</div>" +
            '<div class="cs-panel-name">' +
            escapeHtml(title) +
            "</div>" +
            (e.l
              ? '<div class="cs-panel-loc">' + escapeHtml(e.l) + "</div>"
              : "") +
            "</article>"
          );
        })
        .join("");
    }

    root.innerHTML =
      '<header class="cs-panel-head">' +
      '<div class="cs-panel-title">' +
      (cn ? "本周课表" : "This week") +
      "</div>" +
      '<div class="cs-panel-sub">' +
      escapeHtml(range) +
      " · " +
      count +
      "</div></header>" +
      '<div class="cs-panel-scroll">' +
      (body ||
        '<div class="cs-panel-empty">' +
        (cn ? "本周暂无课程" : "No events this week") +
        "</div>") +
      "</div>";
  }

  async function boot() {
    await loadPrefs();
    render();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

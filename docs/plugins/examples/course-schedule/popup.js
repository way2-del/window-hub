/**
 * 课表 — MA Design 整学期竖向列表（玻璃壳）
 * UK/中文切换；顶栏日期随滚动为「10月28」；无关闭按钮。
 */
(function () {
  const STORE_KEY = "prefs";
  const TERM_START = "2026-09-28";
  const TERM_END = "2026-12-18";

  const LEGEND_KEYS = [
    "welcome",
    "intro",
    "lecture",
    "lang",
    "self",
    "reading",
    "transloc",
    "assist",
    "install",
    "open",
  ];

  const i18n = {
    UK: {
      today: "Today",
      title: "MA Design",
      allDay: "All day",
      daysShort: ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"],
      monthsShort: [
        "Jan",
        "Feb",
        "Mar",
        "Apr",
        "May",
        "Jun",
        "Jul",
        "Aug",
        "Sep",
        "Oct",
        "Nov",
        "Dec",
      ],
      legend: {
        welcome: "Launch",
        intro: "Tutorials",
        lecture: "Lectures",
        lang: "Language",
        self: "Self-directed",
        reading: "Reading Week",
        transloc: "Translocality",
        assist: "Degree Show",
        install: "Install",
        open: "Show Open",
      },
      hint: "Scroll the full term · times = UK",
    },
    CN: {
      today: "今天",
      title: "MA Design 课表",
      allDay: "全天",
      daysShort: ["周日", "周一", "周二", "周三", "周四", "周五", "周六"],
      monthsShort: null,
      legend: {
        welcome: "项目启动",
        intro: "项目辅导",
        lecture: "讲座",
        lang: "语言课",
        self: "自主项目",
        reading: "阅读周",
        transloc: "跨地域项目",
        assist: "毕展协助",
        install: "布展",
        open: "展览开放",
      },
      hint: "下滑浏览整学期 · 时间 = 中国时区",
    },
  };

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

  const state = {
    tz: "CN",
    focusYmd: TERM_START,
    visibleYmd: TERM_START,
    scrollYmd: null,
  };

  let scrollEl = null;

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

  function parseYmd(s) {
    const p = String(s).split("-").map(Number);
    return new Date(p[0], p[1] - 1, p[2]);
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

  function dict() {
    return i18n[state.tz] || i18n.CN;
  }

  /** "10月28日" / "28 Oct" */
  function formatMd(d) {
    const date = typeof d === "string" ? parseYmd(d) : d;
    const m = date.getMonth() + 1;
    const day = date.getDate();
    if (state.tz === "CN") return m + "月" + day + "日";
    const short = dict().monthsShort[date.getMonth()];
    return day + " " + short;
  }

  /** First day of month relative to ymd, deltaMonths = -1 | +1 */
  function monthFirstYmd(ymd, deltaMonths) {
    const d = parseYmd(ymd);
    const first = new Date(d.getFullYear(), d.getMonth() + deltaMonths, 1);
    return formatYmd(first);
  }

  function eventsFor(ymd) {
    return RAW_EVENTS.filter((e) => e.d === ymd);
  }

  function termDays() {
    const days = [];
    let cur = parseYmd(TERM_START);
    const end = parseYmd(TERM_END);
    while (cur <= end) {
      days.push(new Date(cur));
      cur = addDays(cur, 1);
    }
    return days;
  }

  function clampYmd(ymd) {
    if (ymd < TERM_START) return TERM_START;
    if (ymd > TERM_END) return TERM_END;
    return ymd;
  }

  function setTopDate(ymd) {
    const next = clampYmd(ymd);
    if (state.visibleYmd === next) return;
    state.visibleYmd = next;
    const el = document.getElementById("cs-date");
    if (el) el.textContent = formatMd(next);
  }

  function eventCard(ev, dayCtx) {
    const title = state.tz === "CN" ? ev.cn : ev.en;
    const time = state.tz === "CN" ? ev.c : ev.u;
    const d = dict();
    let meta = "";
    if (time) {
      meta +=
        '<div class="cs-ev-row"><span class="cs-ev-time">' +
        escapeHtml(time) +
        "</span></div>";
    } else {
      meta +=
        '<div class="cs-ev-row"><span class="cs-ev-time">' +
        escapeHtml(d.allDay) +
        "</span></div>";
    }
    if (ev.l) {
      meta += '<div class="cs-ev-row">⌖ ' + escapeHtml(ev.l) + "</div>";
    }
    return (
      '<article class="cs-ev bg-' +
      escapeHtml(ev.t) +
      (dayCtx.isToday ? " is-today" : "") +
      '">' +
      '<div class="cs-ev-when">' +
      '<span class="cs-ev-wd">' +
      escapeHtml(dayCtx.wd) +
      "</span>" +
      '<span class="cs-ev-date">' +
      escapeHtml(dayCtx.md) +
      "</span>" +
      (dayCtx.isToday
        ? '<span class="cs-ev-today">' + escapeHtml(d.today) + "</span>"
        : "") +
      "</div>" +
      '<div class="cs-ev-title">' +
      escapeHtml(title) +
      "</div>" +
      '<div class="cs-ev-meta">' +
      meta +
      "</div></article>"
    );
  }

  function emptyDayCard(dayCtx) {
    return (
      '<article class="cs-ev cs-ev-empty' +
      (dayCtx.isToday ? " is-today" : "") +
      '">' +
      '<div class="cs-ev-when">' +
      '<span class="cs-ev-wd">' +
      escapeHtml(dayCtx.wd) +
      "</span>" +
      '<span class="cs-ev-date">' +
      escapeHtml(dayCtx.md) +
      "</span></div>" +
      '<div class="cs-ev-title cs-ev-muted">—</div></article>'
    );
  }

  function syncDateFromScroll() {
    if (!scrollEl) return;
    const sections = scrollEl.querySelectorAll(".cs-day[data-ymd]");
    if (!sections.length) return;
    const top = scrollEl.scrollTop + 12;
    let current = sections[0].getAttribute("data-ymd");
    for (let i = 0; i < sections.length; i++) {
      const sec = sections[i];
      if (sec.offsetTop <= top) current = sec.getAttribute("data-ymd");
      else break;
    }
    if (current) setTopDate(current);
  }

  function scrollToYmd(ymd, smooth) {
    const target = clampYmd(ymd);
    state.focusYmd = target;
    const el = scrollEl && scrollEl.querySelector('.cs-day[data-ymd="' + target + '"]');
    if (!el || !scrollEl) {
      setTopDate(target);
      return;
    }
    state.scrollYmd = target;
    el.scrollIntoView({ behavior: smooth ? "smooth" : "auto", block: "start" });
    setTopDate(target);
    window.setTimeout(function () {
      state.scrollYmd = null;
      syncDateFromScroll();
    }, smooth ? 420 : 40);
  }

  function render() {
    const root = document.getElementById("app");
    if (!root) return;
    root.className = "wg-shell cs-shell";

    const d = dict();
    const todayYmd = formatYmd(startOfDay(new Date()));
    const topYmd = clampYmd(state.visibleYmd || state.focusYmd || TERM_START);

    let legend = "";
    LEGEND_KEYS.forEach(function (key) {
      legend +=
        '<span class="cs-legend-item"><span class="cs-legend-dot dot-' +
        key +
        '"></span>' +
        escapeHtml(d.legend[key] || key) +
        "</span>";
    });

    const days = termDays();
    let daysHtml = "";
    days.forEach(function (day) {
      const ymd = formatYmd(day);
      const evs = eventsFor(ymd);
      const dayCtx = {
        wd: d.daysShort[day.getDay()],
        md: formatMd(day),
        isToday: ymd === todayYmd,
      };
      daysHtml +=
        '<section class="cs-day" data-ymd="' +
        escapeHtml(ymd) +
        '" id="day-' +
        escapeHtml(ymd) +
        '">' +
        (evs.length
          ? evs.map(function (ev) {
              return eventCard(ev, dayCtx);
            }).join("")
          : emptyDayCard(dayCtx)) +
        "</section>";
    });

    root.innerHTML =
      '<header class="cs-header">' +
      '<div class="cs-brand">' +
      '<div class="cs-logo" aria-hidden="true">✎</div>' +
      '<div class="cs-title">' +
      escapeHtml(d.title) +
      "</div></div>" +
      '<div class="cs-lang" role="group" aria-label="Language">' +
      '<button type="button" data-act="tz" data-tz="UK"' +
      (state.tz === "UK" ? ' class="is-on"' : "") +
      ">UK</button>" +
      '<button type="button" data-act="tz" data-tz="CN"' +
      (state.tz === "CN" ? ' class="is-on"' : "") +
      ">中文</button>" +
      "</div></header>" +
      '<div class="cs-nav">' +
      '<button type="button" class="cs-today-btn" data-act="today">' +
      escapeHtml(d.today) +
      "</button>" +
      '<div class="cs-date-shell">' +
      '<button type="button" class="cs-nav-btn" data-act="prev-month" aria-label="Previous month">‹</button>' +
      '<div class="cs-date-label" id="cs-date">' +
      escapeHtml(formatMd(topYmd)) +
      "</div>" +
      '<button type="button" class="cs-nav-btn" data-act="next-month" aria-label="Next month">›</button>' +
      "</div></div>" +
      '<div class="cs-legend">' +
      legend +
      "</div>" +
      '<div class="cs-scroll" id="cs-scroll">' +
      '<div class="cs-week">' +
      daysHtml +
      "</div></div>" +
      '<footer class="cs-footer">' +
      escapeHtml(d.hint) +
      "</footer>";

    bind(root);

    const jump = clampYmd(state.focusYmd || TERM_START);
    requestAnimationFrame(function () {
      scrollToYmd(jump, false);
    });
  }

  function bind(root) {
    scrollEl = document.getElementById("cs-scroll");

    root.querySelectorAll('[data-act="tz"]').forEach(function (btn) {
      btn.addEventListener("click", function () {
        state.tz = btn.getAttribute("data-tz") === "UK" ? "UK" : "CN";
        state.focusYmd = state.visibleYmd || state.focusYmd;
        void savePrefs().then(render).catch(function () {
          render();
        });
      });
    });

    root.querySelector('[data-act="prev-month"]')?.addEventListener("click", function () {
      const base = state.visibleYmd || state.focusYmd || TERM_START;
      scrollToYmd(monthFirstYmd(base, -1), true);
    });

    root.querySelector('[data-act="next-month"]')?.addEventListener("click", function () {
      const base = state.visibleYmd || state.focusYmd || TERM_START;
      scrollToYmd(monthFirstYmd(base, 1), true);
    });

    root.querySelector('[data-act="today"]')?.addEventListener("click", function () {
      const real = formatYmd(startOfDay(new Date()));
      const target =
        real >= TERM_START && real <= TERM_END ? real : TERM_START;
      scrollToYmd(target, true);
    });

    if (scrollEl) {
      scrollEl.addEventListener(
        "scroll",
        function () {
          if (state.scrollYmd) return;
          syncDateFromScroll();
        },
        { passive: true },
      );
    }
  }

  async function loadPrefs() {
    try {
      const raw = await hub().storage.get(STORE_KEY);
      if (raw && typeof raw === "object" && (raw.tz === "UK" || raw.tz === "CN")) {
        state.tz = raw.tz;
      }
    } catch (_) {}
  }

  async function savePrefs() {
    try {
      await hub().storage.set(STORE_KEY, { tz: state.tz });
    } catch (_) {}
  }

  async function boot() {
    await loadPrefs();
    const real = formatYmd(startOfDay(new Date()));
    if (real >= TERM_START && real <= TERM_END) {
      state.focusYmd = real;
      state.visibleYmd = real;
    } else {
      state.focusYmd = TERM_START;
      state.visibleYmd = TERM_START;
    }
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

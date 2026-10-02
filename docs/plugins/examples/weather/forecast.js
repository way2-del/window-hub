/**
 * 天气 — 7 天预报（Host 左卡展开 companion）
 */
(function () {
  const FORECAST_API = "https://cn.apihz.cn/api/tianqi/tqyb.php";
  const CACHE_KEY = "cache";
  const FORECAST_KEY = "forecast";

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

  function num(v, fallback) {
    if (typeof v === "number" && Number.isFinite(v)) return v;
    if (typeof v === "string") {
      const n = parseFloat(v.replace(/[^\d.-]/g, ""));
      if (Number.isFinite(n)) return n;
    }
    return fallback == null ? 0 : fallback;
  }

  function pad(n) {
    return String(n).padStart(2, "0");
  }

  function todayLabel() {
    const d = new Date();
    return pad(d.getMonth() + 1) + "/" + pad(d.getDate());
  }

  function parseDay(raw, fallbackWd) {
    if (!raw || typeof raw !== "object") return null;
    const high = Math.round(num(raw.wd1, NaN));
    const low = Math.round(num(raw.wd2, NaN));
    const w1 = String(raw.weather1 || "").trim();
    const w2 = String(raw.weather2 || "").trim();
    const cond =
      w1 && w2 && w1 !== w2 ? w1 + "转" + w2 : w1 || w2 || "-";
    const icon = String(raw.weather1img || raw.weather2img || "").trim();
    const wd = String(raw.weekday_short_cn || raw.weekday || fallbackWd || "").trim();
    const md = String(raw.monthDay || "").trim();
    return {
      weekday: wd || "—",
      monthDay: md,
      condition: cond,
      iconUrl: icon,
      high: Number.isFinite(high) ? high : "-",
      low: Number.isFinite(low) ? low : "-",
    };
  }

  function parseForecast(data) {
    const days = [];
    const today = parseDay(
      {
        weekday: "今天",
        monthDay: todayLabel(),
        weather1: data.weather1,
        weather2: data.weather2,
        wd1: data.wd1,
        wd2: data.wd2,
        weather1img: data.weather1img,
        weather2img: data.weather2img,
      },
      "今天",
    );
    if (today) {
      today.isToday = true;
      days.push(today);
    }
    for (let i = 2; i <= 7; i++) {
      const block = data["weatherday" + i];
      const day = parseDay(block, "");
      if (day) {
        day.isToday = false;
        days.push(day);
      }
    }
    return {
      city: String(data.shi || data.name || data.place || "").trim() || "-",
      days: days,
      savedAt: Date.now(),
    };
  }

  function render(payload, opts) {
    const root = document.getElementById("app");
    if (!root) return;
    const err = opts && opts.error;
    const busy = opts && opts.busy;
    const days = (payload && payload.days) || [];
    const city = (payload && payload.city) || "";

    let body = "";
    if (!days.length) {
      body =
        '<div class="' +
        (err ? "fc-error" : "fc-empty") +
        '">' +
        escapeHtml(err || (busy ? "加载中…" : "暂无预报")) +
        "</div>";
    } else {
      body =
        '<div class="fc-list" role="list">' +
        days
          .map(function (d) {
            return (
              '<div class="fc-row' +
              (d.isToday ? " is-today" : "") +
              '" role="listitem">' +
              '<span class="fc-wd">' +
              escapeHtml(d.isToday ? "今天" : d.weekday.replace(/^星期/, "周")) +
              "</span>" +
              (d.iconUrl
                ? '<img class="fc-icon" src="' +
                  escapeHtml(d.iconUrl) +
                  '" alt="" />'
                : '<span class="fc-icon" aria-hidden="true"></span>') +
              '<span class="fc-cond">' +
              escapeHtml(d.condition) +
              "</span>" +
              '<span class="fc-temp">' +
              escapeHtml(d.low) +
              "° <span>/</span> " +
              escapeHtml(d.high) +
              "°</span></div>"
            );
          })
          .join("") +
        "</div>";
    }

    root.innerHTML =
      '<header class="fc-head">' +
      '<div class="fc-title">7 天预报</div>' +
      '<div class="fc-sub">' +
      escapeHtml(city || (busy ? "…" : "")) +
      "</div></header>" +
      (err && days.length
        ? '<div class="fc-error" style="margin:0;padding:0 2px 4px;text-align:left">' +
          escapeHtml(err) +
          "</div>"
        : "") +
      body;
  }

  async function loadSettings() {
    const all = (await hub().settings.getAll().catch(function () {
      return {};
    })) || {};
    return {
      apiId: String(all.apiId || "88888888").trim() || "88888888",
      apiKey: String(all.apiKey || "88888888").trim() || "88888888",
      cityOrIp: String(all.cityOrIp || "").trim(),
    };
  }

  async function resolvePlace() {
    const s = await loadSettings();
    if (s.cityOrIp && !/^\d{1,3}(\.\d{1,3}){3}$/.test(s.cityOrIp)) {
      return s.cityOrIp;
    }
    const cache = await hub().storage.get(CACHE_KEY).catch(function () {
      return null;
    });
    const city = cache && cache.info && cache.info.city;
    if (city && city !== "-") return String(city);
    return "北京";
  }

  async function loadCached() {
    const raw = await hub().storage.get(FORECAST_KEY).catch(function () {
      return null;
    });
    if (raw && Array.isArray(raw.days) && raw.days.length) return raw;
    return null;
  }

  async function refresh() {
    const cached = await loadCached();
    render(cached, { busy: true });
    try {
      const h = hub();
      const s = await loadSettings();
      const place = await resolvePlace();
      const qs = new URLSearchParams({
        id: s.apiId,
        key: s.apiKey,
        place: place,
        day: "7",
      });
      const res = await h.fetch(FORECAST_API + "?" + qs.toString(), {
        method: "GET",
        timeoutMs: 15000,
      });
      if (!res || !res.ok) throw new Error("请求失败 HTTP " + (res && res.status));
      const data = JSON.parse(res.body || "{}");
      if (num(data.code, 0) !== 200) throw new Error(data.msg || "接口错误");
      const payload = parseForecast(data);
      await h.storage.set(FORECAST_KEY, payload);
      render(payload, {});
    } catch (e) {
      render(cached, {
        error: e && e.message ? String(e.message) : "预报加载失败",
        busy: false,
      });
    }
  }

  async function boot() {
    const cached = await loadCached();
    if (cached) render(cached, {});
    else render(null, { busy: true });
    await refresh();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

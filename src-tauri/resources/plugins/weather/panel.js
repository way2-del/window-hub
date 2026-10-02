/**
 * 天气 — 岛下拉详情：读缓存 + 手动刷新（hub.fetch → storage → 重绘）
 */
(function () {
  const API = "https://cn.apihz.cn/api/tianqi/tqybip.php";
  const CACHE_KEY = "cache";
  let refreshing = false;

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

  function emptyInfo() {
    return {
      city: "-",
      temp: "-",
      humidity: "-",
      wind: "-",
      condition: "-",
      iconUrl: "",
      feelsLike: "-",
      high: "-",
      low: "-",
      uptime: "",
    };
  }

  function pickCity(data) {
    if (data.shi) return data.shi;
    if (data.name) return data.name;
    if (data.place) {
      const parts = String(data.place)
        .split(/[,，]/)
        .map(function (s) { return s.trim(); })
        .filter(Boolean);
      return parts[parts.length - 1] || data.place;
    }
    if (data.sheng) return data.sheng;
    return "-";
  }

  function dayPart() {
    const h = new Date().getHours();
    return h >= 6 && h < 18 ? "day" : "night";
  }

  function parseWeather(data) {
    const now = data.nowinfo || {};
    const temp = num(now.temperature != null ? now.temperature : data.temperature);
    const humidity = Math.round(num(now.humidity != null ? now.humidity : data.humidity));
    const feelsLike = num(now.feelst != null ? now.feelst : data.feelst, temp);
    const windScale = String(
      now.windScale || data.windScale || data.windleve1 || "",
    ).trim();
    const windDir = String(
      now.windDirection || data.windDirection || data.winddirection1 || "",
    ).trim();
    const wind = [windDir, windScale].filter(Boolean).join(" ") || "-";
    const part = dayPart();
    const condition =
      (part === "day" ? data.weather1 : data.weather2) ||
      data.weather1 ||
      data.weather2 ||
      "-";
    const iconUrl =
      (part === "day" ? data.weather1img : data.weather2img) ||
      data.weather1img ||
      data.weather2img ||
      "";
    const high = Math.max(num(data.wd1, temp), num(data.wd2, temp), temp);
    const low = Math.min(num(data.wd1, temp) || temp, num(data.wd2, temp) || temp, temp);
    return {
      city: pickCity(data),
      temp: Math.round(temp),
      humidity: humidity,
      wind: wind,
      condition: condition,
      iconUrl: iconUrl,
      feelsLike: Math.round(feelsLike),
      high: Math.round(high),
      low: Math.round(low),
      uptime: String(now.uptime || data.uptime || ""),
    };
  }

  function dash(v) {
    if (v == null || v === "") return "-";
    return String(v);
  }

  function render(info, opts) {
    const root = document.getElementById("app");
    if (!root) return;
    const data = info || emptyInfo();
    const busy = opts && opts.busy;
    const err = opts && opts.error;
    root.innerHTML =
      '<div class="wx-head">' +
      '<div class="wx-top">' +
      '<div class="wx-loc"><span>' +
      escapeHtml(dash(data.city)) +
      "</span></div>" +
      '<div class="wx-temp">' +
      escapeHtml(dash(data.temp)) +
      "°</div></div>" +
      '<button type="button" class="wx-refresh' +
      (busy ? " is-busy" : "") +
      '" id="wx-refresh"' +
      (busy ? " disabled" : "") +
      ">" +
      (busy ? "刷新中…" : "刷新") +
      "</button></div>" +
      '<div class="wx-body">' +
      '<div class="wx-condition">' +
      (data.iconUrl
        ? '<img src="' + escapeHtml(data.iconUrl) + '" alt="" />'
        : "") +
      "<span>" +
      escapeHtml(dash(data.condition)) +
      "</span></div>" +
      (data.uptime
        ? '<div class="wx-uptime">更新 ' + escapeHtml(data.uptime) + "</div>"
        : '<div class="wx-uptime">暂无更新时间</div>') +
      (err ? '<div class="wx-error">' + escapeHtml(err) + "</div>" : "") +
      '<div class="wx-metrics" role="list">' +
      '<div class="wx-metric" role="listitem"><span>湿度</span><strong>' +
      escapeHtml(dash(data.humidity)) +
      "%</strong></div>" +
      '<div class="wx-metric" role="listitem"><span>风力</span><strong>' +
      escapeHtml(dash(data.wind)) +
      "</strong></div>" +
      '<div class="wx-metric" role="listitem"><span>体感</span><strong>' +
      escapeHtml(dash(data.feelsLike)) +
      "°</strong></div>" +
      '<div class="wx-metric" role="listitem"><span>高低</span><strong>' +
      escapeHtml(dash(data.low)) +
      "°/" +
      escapeHtml(dash(data.high)) +
      "°</strong></div>" +
      "</div></div>";

    const btn = document.getElementById("wx-refresh");
    if (btn) {
      btn.addEventListener("click", function () {
        void refreshNow();
      });
    }
  }

  async function loadSettings() {
    const h = hub();
    const all = (await h.settings.getAll().catch(function () { return {}; })) || {};
    return {
      apiId: String(all.apiId || "88888888").trim() || "88888888",
      apiKey: String(all.apiKey || "88888888").trim() || "88888888",
      cityOrIp: String(all.cityOrIp || "").trim(),
    };
  }

  async function loadCache() {
    const raw = await hub().storage.get(CACHE_KEY).catch(function () {
      return null;
    });
    return raw && raw.info ? raw.info : null;
  }

  async function load(opts) {
    const info = await loadCache();
    render(info, opts || {});
  }

  async function refreshNow() {
    if (refreshing) return;
    refreshing = true;
    const prev = await loadCache();
    render(prev, { busy: true });
    try {
      const h = hub();
      const s = await loadSettings();
      const qs = new URLSearchParams({ id: s.apiId, key: s.apiKey });
      if (s.cityOrIp) qs.set("ip", s.cityOrIp);
      const url = API + "?" + qs.toString();
      const res = await h.fetch(url, { method: "GET", timeoutMs: 15000 });
      if (!res || !res.ok) throw new Error("请求失败 HTTP " + (res && res.status));
      const data = JSON.parse(res.body || "{}");
      if (num(data.code, 0) !== 200) throw new Error(data.msg || "接口错误");
      const info = parseWeather(data);
      await h.storage.set(CACHE_KEY, { info: info, savedAt: Date.now() });
      if (h.island && h.island.setBar) {
        const text =
          info.temp +
          "° " +
          info.city +
          " · " +
          info.condition +
          " 湿度" +
          info.humidity +
          "%";
        await h.island.setBar({
          text: text,
          title:
            info.city +
            " " +
            info.temp +
            "° · " +
            info.condition +
            " · " +
            info.wind,
        }).catch(function () {});
      }
      render(info, {});
    } catch (e) {
      const info = await loadCache();
      render(info, {
        error: e && e.message ? String(e.message) : "刷新失败",
      });
    } finally {
      refreshing = false;
    }
  }

  async function boot() {
    await load();
    const h = hub();
    if (h.settings && h.settings.subscribe) {
      h.settings.subscribe(function () {
        void load();
      });
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", function () {
      void boot().catch(console.error);
    });
  } else {
    void boot().catch(console.error);
  }
})();

/**
 * 天气 — 快捷区隐形 worker：轮询 API → hub.storage 缓存 → hub.island.setBar
 * 无数据时岛栏占位「-」；Host 仅在「岛栏常驻」选中本插件时展示。
 */
(function () {
  const API = "https://cn.apihz.cn/api/tianqi/tqybip.php";
  const CACHE_KEY = "cache";

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function num(v, fallback) {
    if (typeof v === "number" && Number.isFinite(v)) return v;
    if (typeof v === "string") {
      const n = parseFloat(v.replace(/[^\d.-]/g, ""));
      if (Number.isFinite(n)) return n;
    }
    return fallback == null ? 0 : fallback;
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
      pressure: "-",
      uptime: "",
      placeholder: true,
    };
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
      pressure: String(now.pressure != null ? now.pressure : data.pressure || "-"),
      uptime: String(now.uptime || data.uptime || ""),
      placeholder: false,
    };
  }

  function barText(info) {
    const t = info && info.temp != null ? info.temp : "-";
    const city = (info && info.city) || "-";
    const cond = (info && info.condition) || "-";
    const hum = info && info.humidity != null ? info.humidity : "-";
    return t + "° " + city + " · " + cond + " 湿度" + hum + "%";
  }

  function barTitle(info) {
    const city = (info && info.city) || "-";
    const t = info && info.temp != null ? info.temp : "-";
    const cond = (info && info.condition) || "-";
    const wind = (info && info.wind) || "-";
    return city + " " + t + "° · " + cond + " · " + wind;
  }

  async function loadSettings() {
    const h = hub();
    const all = (await h.settings.getAll().catch(function () { return {}; })) || {};
    return {
      apiId: String(all.apiId || "88888888").trim() || "88888888",
      apiKey: String(all.apiKey || "88888888").trim() || "88888888",
      cityOrIp: String(all.cityOrIp || "").trim(),
      refreshMinutes: Math.max(15, Number(all.refreshMinutes) || 30),
    };
  }

  async function applyBar(info) {
    const h = hub();
    if (!h.island || !h.island.setBar) return;
    const payload = info || emptyInfo();
    try {
      await h.island.setBar({
        text: barText(payload),
        title: barTitle(payload),
      });
    } catch (err) {
      console.warn("[weather] setBar", err);
    }
  }

  async function refresh() {
    const h = hub();
    const s = await loadSettings();
    const qs = new URLSearchParams({ id: s.apiId, key: s.apiKey });
    if (s.cityOrIp) qs.set("ip", s.cityOrIp);
    const url = API + "?" + qs.toString();
    const res = await h.fetch(url, { method: "GET", timeoutMs: 15000 });
    if (!res || !res.ok) throw new Error("天气请求失败 HTTP " + (res && res.status));
    const data = JSON.parse(res.body || "{}");
    if (num(data.code, 0) !== 200) throw new Error(data.msg || "天气接口错误");
    const info = parseWeather(data);
    await h.storage.set(CACHE_KEY, { info: info, savedAt: Date.now() });
    await applyBar(info);
    return info;
  }

  async function bootFromCache() {
    const raw = await hub().storage.get(CACHE_KEY).catch(function () { return null; });
    if (raw && raw.info) {
      await applyBar(raw.info);
      return raw.info;
    }
    await applyBar(emptyInfo());
    return null;
  }

  async function tick() {
    try {
      await refresh();
    } catch (err) {
      console.warn("[weather] refresh", err);
      const cached = await bootFromCache();
      if (!cached) await applyBar(emptyInfo());
    }
  }

  async function boot() {
    const h = hub();
    try {
      if (h.shortcuts && h.shortcuts.requestSize) {
        await h.shortcuts.requestSize({ width: 1 });
      }
    } catch (_) {}
    await applyBar(emptyInfo());
    await bootFromCache();
    await tick();
    let timer = null;
    async function schedule() {
      if (timer) clearInterval(timer);
      const s = await loadSettings();
      timer = setInterval(function () {
        void tick();
      }, s.refreshMinutes * 60 * 1000);
    }
    await schedule();
    if (h.settings && h.settings.subscribe) {
      h.settings.subscribe(function () {
        void tick().then(schedule);
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

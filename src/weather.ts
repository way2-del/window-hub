/** 接口盒子 IP 天气：https://www.free-api.com/doc/670 */

import { invoke } from "@tauri-apps/api/core";

export type WeatherInfo = {
  city: string;
  temp: number;
  humidity: number;
  wind: string;
  condition: string;
  iconUrl: string;
  feelsLike: number;
  high: number;
  low: number;
  pressure: string;
  uptime: string;
};

export const DEFAULT_WEATHER: WeatherInfo = {
  city: "定位中",
  temp: 0,
  humidity: 0,
  wind: "--",
  condition: "获取中",
  iconUrl: "",
  feelsLike: 0,
  high: 0,
  low: 0,
  pressure: "--",
  uptime: "",
};

const API_URL = "https://cn.apihz.cn/api/tianqi/tqybip.php";
/** 公共演示凭证（共享频次）；可在设置里换成自己的 id/key */
const DEFAULT_ID = "88888888";
const DEFAULT_KEY = "88888888";

const LS_ID = "wh-weather-api-id";
const LS_KEY = "wh-weather-api-key";
const LS_CACHE = "wh-weather-cache";

type WeatherCache = {
  info: WeatherInfo;
  savedAt: number;
};

type WeatherCredentials = { id: string; key: string };

let credsCache: WeatherCredentials = { id: DEFAULT_ID, key: DEFAULT_KEY };
let weatherCacheMem: WeatherInfo | null = null;
let weatherHydrated = false;

function readLegacyCreds(): WeatherCredentials | null {
  try {
    const id = localStorage.getItem(LS_ID);
    const key = localStorage.getItem(LS_KEY);
    if (id == null && key == null) return null;
    return {
      id: id?.trim() || DEFAULT_ID,
      key: key?.trim() || DEFAULT_KEY,
    };
  } catch {
    return null;
  }
}

function readLegacyCache(): WeatherCache | null {
  try {
    const raw = localStorage.getItem(LS_CACHE);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as WeatherCache;
    if (!parsed?.info || typeof parsed.info.city !== "string") return null;
    return parsed;
  } catch {
    return null;
  }
}

function clearLegacyWeatherLs() {
  try {
    localStorage.removeItem(LS_ID);
    localStorage.removeItem(LS_KEY);
    localStorage.removeItem(LS_CACHE);
  } catch {
    /* noop */
  }
}

export async function hydrateWeatherStorage(): Promise<void> {
  try {
    const legacyCreds = readLegacyCreds();
    const legacyCache = readLegacyCache();
    if (legacyCreds) {
      await invoke("set_weather_credentials", { creds: legacyCreds });
    }
    if (legacyCache) {
      await invoke("set_weather_cache", { cache: legacyCache });
    }
    if (legacyCreds || legacyCache) clearLegacyWeatherLs();

    credsCache = await invoke<WeatherCredentials>("get_weather_credentials");
    const cached = await invoke<WeatherCache | null>("get_weather_cache");
    if (cached?.info && typeof cached.info.city === "string") {
      weatherCacheMem = cached.info;
    }
  } catch {
    credsCache = { id: DEFAULT_ID, key: DEFAULT_KEY };
  }
  weatherHydrated = true;
}

export function getWeatherCredentials() {
  return { ...credsCache };
}

export async function setWeatherCredentials(id: string, key: string) {
  const creds = { id: id.trim() || DEFAULT_ID, key: key.trim() || DEFAULT_KEY };
  credsCache = creds;
  try {
    await invoke("set_weather_credentials", { creds });
  } catch {
    /* noop */
  }
  window.dispatchEvent(new Event("wh-weather-creds"));
}

/** 读取上次成功拉取的天气；无缓存则返回 null */
export function loadWeatherCache(): WeatherInfo | null {
  return weatherCacheMem;
}

async function saveWeatherCache(info: WeatherInfo) {
  weatherCacheMem = info;
  try {
    const payload: WeatherCache = { info, savedAt: Date.now() };
    await invoke("set_weather_cache", { cache: payload });
  } catch {
    /* noop */
  }
}

/** 启动用：有缓存先显示缓存，否则默认占位 */
export function initialWeather(): WeatherInfo {
  return loadWeatherCache() ?? DEFAULT_WEATHER;
}

type ApiNow = {
  precipitation?: number | string;
  temperature?: number | string;
  pressure?: number | string;
  humidity?: number | string;
  windDirection?: string;
  windSpeed?: number | string;
  windScale?: string;
  feelst?: number | string;
  uptime?: string;
};

type ApiResponse = {
  code?: number | string;
  msg?: string;
  place?: string;
  guo?: string;
  sheng?: string;
  shi?: string;
  name?: string;
  weather1?: string;
  weather2?: string;
  weather1img?: string;
  weather2img?: string;
  wd1?: number | string;
  wd2?: number | string;
  windleve1?: string;
  windleve2?: string;
  winddirection1?: string;
  uptime?: string;
  // flat shape (free-api.com/doc/670)
  precipitation?: number | string;
  temperature?: number | string;
  pressure?: number | string;
  humidity?: number | string;
  windDirection?: string;
  windSpeed?: number | string;
  windScale?: string;
  feelst?: number | string;
  nowinfo?: ApiNow;
};

function num(v: unknown, fallback = 0): number {
  if (typeof v === "number" && Number.isFinite(v)) return v;
  if (typeof v === "string") {
    const n = parseFloat(v.replace(/[^\d.-]/g, ""));
    if (Number.isFinite(n)) return n;
  }
  return fallback;
}

function pickCity(data: ApiResponse): string {
  if (data.shi) return data.shi;
  if (data.name) return data.name;
  if (data.place) {
    const parts = data.place
      .split(/[,，]/)
      .map((s) => s.trim())
      .filter(Boolean);
    return parts[parts.length - 1] || data.place;
  }
  if (data.sheng) return data.sheng;
  return "未知";
}

function dayPart(): "day" | "night" {
  const h = new Date().getHours();
  return h >= 6 && h < 18 ? "day" : "night";
}

export async function fetchWeather(ip?: string): Promise<WeatherInfo> {
  if (!weatherHydrated) {
    await hydrateWeatherStorage();
  }
  const { id, key } = getWeatherCredentials();
  const qs = new URLSearchParams({ id, key });
  if (ip) qs.set("ip", ip);

  const res = await fetch(`${API_URL}?${qs.toString()}`, {
    method: "GET",
    cache: "no-store",
  });
  if (!res.ok) {
    throw new Error(`天气请求失败 HTTP ${res.status}`);
  }

  const data = (await res.json()) as ApiResponse;
  const code = num(data.code, 0);
  if (code !== 200) {
    throw new Error(data.msg || `天气接口错误 code=${code}`);
  }

  const info = parseWeather(data);
  await saveWeatherCache(info);
  return info;
}

function parseWeather(data: ApiResponse): WeatherInfo {
  const now = data.nowinfo;
  const temp = num(now?.temperature ?? data.temperature);
  const humidity = Math.round(num(now?.humidity ?? data.humidity));
  const feelsLike = num(now?.feelst ?? data.feelst, temp);
  const pressure = String(now?.pressure ?? data.pressure ?? "--");
  const windScale = String(
    now?.windScale ?? data.windScale ?? data.windleve1 ?? "",
  ).trim();
  const windDir = String(
    now?.windDirection ?? data.windDirection ?? data.winddirection1 ?? "",
  ).trim();
  const wind = [windDir, windScale].filter(Boolean).join(" ") || "--";

  const part = dayPart();
  const condition =
    (part === "day" ? data.weather1 : data.weather2) ||
    data.weather1 ||
    data.weather2 ||
    "未知";
  const iconUrl =
    (part === "day" ? data.weather1img : data.weather2img) ||
    data.weather1img ||
    data.weather2img ||
    "";

  const high = Math.max(num(data.wd1, temp), num(data.wd2, temp), temp);
  const low = Math.min(
    num(data.wd1, temp) || temp,
    num(data.wd2, temp) || temp,
    temp,
  );

  return {
    city: pickCity(data),
    temp: Math.round(temp),
    humidity,
    wind,
    condition,
    iconUrl,
    feelsLike: Math.round(feelsLike),
    high: Math.round(high),
    low: Math.round(low),
    pressure,
    uptime: String(now?.uptime ?? data.uptime ?? ""),
  };
}

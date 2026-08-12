import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { getIslandPrefs, subscribeIslandPrefs } from "./islandPrefs";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import "./systemFlyout.css";

export type WifiNetwork = {
  ssid: string;
  signal: number;
  secured: boolean;
  connected: boolean;
  saved?: boolean;
  password?: string | null;
};

export type VolumeSnapshot = {
  level: number;
  muted: boolean;
};

export type AudioOutputDevice = {
  id: string;
  name: string;
  isDefault: boolean;
};

export type PowerSnapshot = {
  acLine: string;
  percent?: number | null;
  charging: boolean;
  lifeSec?: number | null;
  hasBattery: boolean;
};

export type PerfSnapshot = {
  memPercent: number;
  cpuPercent: number;
  cpuTempC?: number | null;
  gpuTempC?: number | null;
  gpuMemPercent?: number | null;
};

export type SystemRadioSnapshot = {
  wifi: {
    radioOn: boolean;
    connectedSsid?: string | null;
    ipv4: string[];
    ipv6: string[];
    networks: WifiNetwork[];
  };
  bluetooth: {
    radioOn: boolean;
    devices: Array<{
      id: string;
      name: string;
      connected: boolean;
      kind: string;
      battery?: number | null;
    }>;
  };
  ime: { name: string; layout: string; mark: string; mode: string; caps: boolean };
  volume: VolumeSnapshot;
  power: PowerSnapshot;
  perf: PerfSnapshot;
  peripherals: Array<{ id: string; kind: string; name: string }>;
};

export type FlyoutKind = "wifi" | "bluetooth" | "volume" | "ime" | "power" | "calendar";

function parseKind(raw: string | null | undefined): FlyoutKind {
  const v = (raw || "").toLowerCase();
  if (
    v === "bluetooth" ||
    v === "volume" ||
    v === "ime" ||
    v === "power" ||
    v === "calendar"
  ) {
    return v;
  }
  return "wifi";
}

function resolveKind(): FlyoutKind {
  const fromWin =
    typeof window.__WH_SYSTEM_FLYOUT_KIND__ === "string"
      ? window.__WH_SYSTEM_FLYOUT_KIND__
      : null;
  const q = new URLSearchParams(window.location.search).get("kind");
  return parseKind(fromWin || q || "wifi");
}

declare global {
  interface Window {
    __WH_IS_SYSTEM_FLYOUT__?: boolean;
    __WH_SYSTEM_FLYOUT_KIND__?: string;
  }
}

function signalBars(signal: number) {
  const n = signal >= 75 ? 4 : signal >= 50 ? 3 : signal >= 25 ? 2 : 1;
  return "▂".repeat(n) + "▁".repeat(4 - n);
}

function btKindLabel(kind: string) {
  switch (kind) {
    case "audio":
      return "音频";
    case "phone":
      return "手机";
    case "computer":
      return "电脑";
    case "peripheral":
      return "外设";
    case "imaging":
      return "影像";
    case "wearable":
      return "可穿戴";
    default:
      return "设备";
  }
}

function AudioOutGlyph({ name }: { name: string }) {
  const low = name.toLowerCase();
  const isHead =
    low.includes("head") ||
    low.includes("ear") ||
    low.includes("airpods") ||
    low.includes("headset") ||
    name.includes("耳机") ||
    name.includes("耳麦") ||
    name.includes("耳塞");
  if (isHead) {
    return (
      <svg className="sf-dev-ico" viewBox="0 0 24 24" fill="none" aria-hidden>
        <path
          d="M4.5 13v2.2A2.2 2.2 0 0 0 6.7 17.4H8v-4.4H6.7A2.2 2.2 0 0 0 4.5 13Zm15 0a2.2 2.2 0 0 0-2.2-2.2H16v4.4h1.3A2.2 2.2 0 0 0 19.5 13V13Z"
          stroke="currentColor"
          strokeWidth="1.5"
        />
        <path
          d="M4.5 13a7.5 7.5 0 0 1 15 0"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
        />
      </svg>
    );
  }
  return (
    <svg className="sf-dev-ico" viewBox="0 0 24 24" fill="none" aria-hidden>
      <path
        d="M5 9.5v5a1.5 1.5 0 0 0 1.5 1.5H8l3.2 2.6a.8.8 0 0 0 1.3-.6V6a.8.8 0 0 0-1.3-.6L8 8H6.5A1.5 1.5 0 0 0 5 9.5Z"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
      <path
        d="M15.2 9.2a3.2 3.2 0 0 1 0 5.6M17.4 7.2a5.6 5.6 0 0 1 0 9.6"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

function BtDeviceGlyph({ kind, name }: { kind: string; name: string }) {
  const low = name.toLowerCase();
  const isHead =
    kind === "audio" ||
    low.includes("head") ||
    low.includes("airpods") ||
    name.includes("耳机") ||
    name.includes("耳麦");
  const isPad =
    kind === "peripheral" ||
    low.includes("controller") ||
    low.includes("xbox") ||
    name.includes("手柄");
  if (isHead) {
    return (
      <svg className="sf-dev-ico" viewBox="0 0 24 24" fill="none" aria-hidden>
        <path
          d="M4.5 13v2.2A2.2 2.2 0 0 0 6.7 17.4H8v-4.4H6.7A2.2 2.2 0 0 0 4.5 13Zm15 0a2.2 2.2 0 0 0-2.2-2.2H16v4.4h1.3A2.2 2.2 0 0 0 19.5 13V13Z"
          stroke="currentColor"
          strokeWidth="1.5"
        />
        <path d="M4.5 13a7.5 7.5 0 0 1 15 0" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      </svg>
    );
  }
  if (isPad) {
    return (
      <svg className="sf-dev-ico" viewBox="0 0 24 24" fill="none" aria-hidden>
        <path
          d="M7 9h10a3.6 3.6 0 0 1 3.4 4.8l-1 2.4A2 2 0 0 1 17.5 17.5h-11a2 2 0 0 1-1.9-.9l-1-2.4A3.6 3.6 0 0 1 7 9Z"
          stroke="currentColor"
          strokeWidth="1.5"
        />
        <path d="M9 12.2v2.4M7.8 13.4h2.4" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        <circle cx="15.2" cy="12.6" r="0.85" fill="currentColor" />
        <circle cx="17" cy="14.2" r="0.85" fill="currentColor" />
      </svg>
    );
  }
  if (kind === "phone") {
    return (
      <svg className="sf-dev-ico" viewBox="0 0 24 24" fill="none" aria-hidden>
        <rect x="8" y="3.5" width="8" height="17" rx="2" stroke="currentColor" strokeWidth="1.5" />
        <path d="M11 17.5h2" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      </svg>
    );
  }
  return (
    <svg className="sf-dev-ico" viewBox="0 0 24 24" fill="none" aria-hidden>
      <path
        d="M7 7.5 17 16.5 12 21V3l5 4.5L7 16.5"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function BtBatt({ percent }: { percent: number }) {
  const low = percent <= 20;
  return (
    <span className={`sf-bt-batt${low ? " is-low" : ""}`} title={`电量 ${percent}%`}>
      {percent}%
    </span>
  );
}

function formatLife(sec?: number | null) {
  if (sec == null || sec <= 0) return "—";
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  if (h > 0) return `约 ${h} 小时 ${m} 分`;
  return `约 ${m} 分`;
}

async function closeSelf() {
  try {
    await invoke("close_system_flyout");
  } catch {
    try {
      await getCurrentWindow().hide();
    } catch {
      /* noop */
    }
  }
}

/** 公历固定节日 */
const SOLAR_FEST: Record<string, string> = {
  "1-1": "元旦",
  "2-14": "情人节",
  "3-8": "妇女节",
  "3-12": "植树节",
  "4-1": "愚人节",
  "5-1": "劳动节",
  "5-4": "青年节",
  "6-1": "儿童节",
  "7-1": "建党节",
  "8-1": "建军节",
  "9-10": "教师节",
  "10-1": "国庆",
  "12-24": "平安夜",
  "12-25": "圣诞",
};

/** 农历传统节日 → 公历（覆盖近年；未命中则仅显示公历节日） */
const LUNAR_SOLAR: Record<string, string> = {
  // 2024
  "2024-2-10": "春节",
  "2024-2-24": "元宵",
  "2024-4-4": "清明",
  "2024-6-10": "端午",
  "2024-8-10": "七夕",
  "2024-9-17": "中秋",
  "2024-10-11": "重阳",
  // 2025
  "2025-1-29": "春节",
  "2025-2-12": "元宵",
  "2025-4-4": "清明",
  "2025-5-31": "端午",
  "2025-7-31": "七夕",
  "2025-10-6": "中秋",
  "2025-10-29": "重阳",
  // 2026
  "2026-2-17": "春节",
  "2026-3-3": "元宵",
  "2026-4-5": "清明",
  "2026-6-19": "端午",
  "2026-8-19": "七夕",
  "2026-9-25": "中秋",
  "2026-10-18": "重阳",
  // 2027
  "2027-2-6": "春节",
  "2027-2-20": "元宵",
  "2027-4-5": "清明",
  "2027-6-9": "端午",
  "2027-8-8": "七夕",
  "2027-9-15": "中秋",
  "2027-10-7": "重阳",
  // 2028
  "2028-1-26": "春节",
  "2028-2-9": "元宵",
  "2028-4-4": "清明",
  "2028-5-28": "端午",
  "2028-7-28": "七夕",
  "2028-10-3": "中秋",
  "2028-10-24": "重阳",
};

/** 国务院放假调休：off=休，work=班（覆盖 2025–2026） */
type HolidayKind = "off" | "work";
const CN_HOLIDAY: Record<string, HolidayKind> = {
  // 2025
  "2025-1-1": "off",
  "2025-1-26": "work",
  "2025-1-28": "off",
  "2025-1-29": "off",
  "2025-1-30": "off",
  "2025-1-31": "off",
  "2025-2-1": "off",
  "2025-2-2": "off",
  "2025-2-3": "off",
  "2025-2-4": "off",
  "2025-2-8": "work",
  "2025-4-4": "off",
  "2025-4-5": "off",
  "2025-4-6": "off",
  "2025-4-27": "work",
  "2025-5-1": "off",
  "2025-5-2": "off",
  "2025-5-3": "off",
  "2025-5-4": "off",
  "2025-5-5": "off",
  "2025-5-31": "off",
  "2025-6-1": "off",
  "2025-6-2": "off",
  "2025-9-28": "work",
  "2025-10-1": "off",
  "2025-10-2": "off",
  "2025-10-3": "off",
  "2025-10-4": "off",
  "2025-10-5": "off",
  "2025-10-6": "off",
  "2025-10-7": "off",
  "2025-10-8": "off",
  "2025-10-11": "work",
  // 2026
  "2026-1-1": "off",
  "2026-1-2": "off",
  "2026-1-3": "off",
  "2026-1-4": "work",
  "2026-2-14": "work",
  "2026-2-15": "off",
  "2026-2-16": "off",
  "2026-2-17": "off",
  "2026-2-18": "off",
  "2026-2-19": "off",
  "2026-2-20": "off",
  "2026-2-21": "off",
  "2026-2-22": "off",
  "2026-2-23": "off",
  "2026-2-28": "work",
  "2026-4-4": "off",
  "2026-4-5": "off",
  "2026-4-6": "off",
  "2026-5-1": "off",
  "2026-5-2": "off",
  "2026-5-3": "off",
  "2026-5-4": "off",
  "2026-5-5": "off",
  "2026-5-9": "work",
  "2026-6-19": "off",
  "2026-6-20": "off",
  "2026-6-21": "off",
  "2026-9-20": "work",
  "2026-9-25": "off",
  "2026-9-26": "off",
  "2026-9-27": "off",
  "2026-10-1": "off",
  "2026-10-2": "off",
  "2026-10-3": "off",
  "2026-10-4": "off",
  "2026-10-5": "off",
  "2026-10-6": "off",
  "2026-10-7": "off",
  "2026-10-10": "work",
};

function ymdKey(d: Date): string {
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
}

function festivalName(d: Date): string | null {
  const ymd = ymdKey(d);
  const lunar = LUNAR_SOLAR[ymd];
  if (lunar) return lunar;
  const md = `${d.getMonth() + 1}-${d.getDate()}`;
  return SOLAR_FEST[md] ?? null;
}

function holidayKind(d: Date): HolidayKind | null {
  return CN_HOLIDAY[ymdKey(d)] ?? null;
}

function buildMonth(base: Date) {
  const y = base.getFullYear();
  const m = base.getMonth();
  const first = new Date(y, m, 1);
  const startPad = (first.getDay() + 6) % 7; // Mon-first
  const daysInMonth = new Date(y, m + 1, 0).getDate();
  const cells: Array<{
    day: number | null;
    date?: Date;
    fest?: string | null;
    holiday?: HolidayKind | null;
    weekend?: boolean;
  }> = [];
  for (let i = 0; i < startPad; i++) cells.push({ day: null });
  for (let d = 1; d <= daysInMonth; d++) {
    const date = new Date(y, m, d);
    const dow = date.getDay();
    cells.push({
      day: d,
      date,
      fest: festivalName(date),
      holiday: holidayKind(date),
      weekend: dow === 0 || dow === 6,
    });
  }
  while (cells.length % 7 !== 0) cells.push({ day: null });
  return { y, m, cells };
}

export default function SystemFlyoutApp() {
  const [kind, setKind] = useState<FlyoutKind>(() => resolveKind());
  const [snap, setSnap] = useState<SystemRadioSnapshot | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [msg, setMsg] = useState("");
  const [entering, setEntering] = useState(true);
  const [volLocal, setVolLocal] = useState<number | null>(null);
  const [audioDevices, setAudioDevices] = useState<AudioOutputDevice[]>([]);
  const [calCursor, setCalCursor] = useState(() => new Date());
  const [previewSound, setPreviewSound] = useState(() => getIslandPrefs().volumePreviewSound);
  const [calPick, setCalPick] = useState<Date | null>(null);
  const [wifiMenu, setWifiMenu] = useState<{ ssid: string; x: number; y: number } | null>(null);
  const [showMenuPw, setShowMenuPw] = useState(false);
  const [pwLoading, setPwLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const draggingRef = useRef(false);
  const previewTimer = useRef(0);
  const wheelCommitTimer = useRef(0);
  const glassReady = useRef(false);
  const shellRef = useRef<HTMLDivElement | null>(null);
  const volLocalRef = useRef<number | null>(null);
  const snapRef = useRef<SystemRadioSnapshot | null>(null);
  const previewSoundRef = useRef(previewSound);
  const pendingWheelVol = useRef<number | null>(null);
  const wheelUnmute = useRef(false);

  useEffect(() => {
    volLocalRef.current = volLocal;
  }, [volLocal]);
  useEffect(() => {
    snapRef.current = snap;
  }, [snap]);
  useEffect(() => {
    previewSoundRef.current = previewSound;
  }, [previewSound]);

  function applyKind(next: FlyoutKind) {
    setKind(next);
    window.__WH_SYSTEM_FLYOUT_KIND__ = next;
    // Keep shell opaque — never flip to is-enter/opacity:0 on slim Win10.
    setEntering(false);
    setMsg("");
    setWifiMenu(null);
    setShowMenuPw(false);
  }

  async function copyText(text: string, ok = "已复制") {
    try {
      await navigator.clipboard.writeText(text);
      setMsg(ok);
    } catch {
      setMsg("复制失败");
    }
  }

  function openWifiMenu(e: MouseEvent, ssid: string) {
    e.preventDefault();
    e.stopPropagation();
    if (!ssid) return;
    const shell = shellRef.current;
    if (!shell) return;
    const rect = shell.getBoundingClientRect();
    const menuW = 208;
    const menuH = 220;
    const rawX = e.clientX - rect.left;
    const rawY = e.clientY - rect.top;
    const x = Math.max(8, Math.min(rawX, rect.width - menuW - 8));
    const y = Math.max(8, Math.min(rawY, rect.height - menuH - 8));
    setShowMenuPw(false);
    setPwLoading(false);
    setWifiMenu({ ssid, x, y });
  }

  function closeWifiMenu() {
    setWifiMenu(null);
    setShowMenuPw(false);
    setPwLoading(false);
  }

  async function refresh(force = false) {
    try {
      // Cache-only read (force only schedules background collectors).
      const next = await invoke<SystemRadioSnapshot>("get_system_radio_snapshot", { force });
      setSnap((prev) => {
        if (draggingRef.current && prev) {
          return { ...next, volume: prev.volume };
        }
        // Preserve any on-demand passwords already fetched for this session.
        if (prev?.wifi?.networks?.length) {
          const pwMap = new Map(
            prev.wifi.networks
              .filter((n) => n.password)
              .map((n) => [n.ssid, n.password as string]),
          );
          if (pwMap.size > 0) {
            next.wifi = {
              ...next.wifi,
              networks: next.wifi.networks.map((n) =>
                pwMap.has(n.ssid) ? { ...n, password: pwMap.get(n.ssid) } : n,
              ),
            };
          }
        }
        return next;
      });
      snapRef.current = next;
      if (!draggingRef.current) setVolLocal(null);
      return next;
    } catch (e) {
      setMsg(String(e));
      return null;
    }
  }

  function requestSoftRefresh(domains?: string[]) {
    void invoke("refresh_system_status", { domains: domains ?? ["all"] }).catch(() => undefined);
  }

  async function manualRefresh() {
    if (refreshing) return;
    setRefreshing(true);
    setMsg("");
    const domains =
      kind === "wifi" ? ["wifi"] : kind === "bluetooth" ? ["bluetooth"] : ["all"];
    requestSoftRefresh(domains);
    try {
      await refresh(true);
      // 扫描结果稍后进缓存，再读一次
      await new Promise((r) => window.setTimeout(r, 700));
      await refresh(false);
      setMsg("已刷新");
      window.setTimeout(() => setMsg((m) => (m === "已刷新" ? "" : m)), 1600);
    } finally {
      setRefreshing(false);
    }
  }

  async function ensureWifiPassword(ssid: string): Promise<string | null> {
    const existing = snapRef.current?.wifi.networks.find((n) => n.ssid === ssid)?.password;
    if (existing) return existing;
    try {
      const pw = await invoke<string | null>("get_wifi_password", { ssid });
      if (pw) {
        setSnap((prev) => {
          if (!prev) return prev;
          return {
            ...prev,
            wifi: {
              ...prev.wifi,
              networks: prev.wifi.networks.map((n) =>
                n.ssid === ssid ? { ...n, password: pw } : n,
              ),
            },
          };
        });
      }
      return pw ?? null;
    } catch {
      return null;
    }
  }

  async function toggleBt(id: string, connect: boolean) {
    setBusy(id);
    setMsg("");
    // Optimistic UI — don't freeze the flyout waiting on a full radio rescan.
    setSnap((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        bluetooth: {
          ...prev.bluetooth,
          devices: prev.bluetooth.devices.map((d) =>
            d.id === id ? { ...d, connected: connect } : d,
          ),
        },
      };
    });
    void invoke("suppress_system_flyout_blur", { ms: 1200 }).catch(() => undefined);
    try {
      await invoke("set_bluetooth_device", { id, connect });
      setMsg("");
      requestSoftRefresh(["bluetooth"]);
      // Deferred re-read — don't await here (keeps mica shell painting).
      window.setTimeout(() => {
        void refresh(false);
      }, 700);
    } catch (e) {
      setMsg(String(e));
      requestSoftRefresh(["bluetooth"]);
      void refresh(false);
    } finally {
      setBusy(null);
    }
  }

  useEffect(() => {
    void (async () => {
      if (!glassReady.current) {
        try {
          const prefs = await invoke<GlassPrefs>("get_material_prefs");
          await syncGlassCss({
            kind: "mica-alt",
            dark: prefs.dark ?? null,
            acrylicAlpha: prefs.acrylicAlpha,
          });
        } catch {
          await syncGlassCss({ kind: "mica-alt", dark: true });
        }
        // Rust already applied material on create — skip apply_window_effect (DWM flash).
        glassReady.current = true;
      }

      try {
        const remote = await invoke<string>("get_system_flyout_kind");
        applyKind(parseKind(remote));
      } catch {
        applyKind(resolveKind());
      }
      // Reveal after CSS; fill cache after (never show empty HWND then repaint).
      void invoke("reveal_system_flyout").catch(() => undefined);
      void refresh(false);
      requestSoftRefresh(["audio", "power", "perf", "ime"]);
    })();

    const tEnter = window.setTimeout(() => setEntering(false), 10);
    const onCustom = (ev: Event) => {
      const detail = (ev as CustomEvent<string>).detail;
      applyKind(parseKind(detail));
      // Warm reopen arms FLYOUT_AWAIT — reveal after kind paints (avoids wifi→X flash).
      requestAnimationFrame(() => {
        void invoke("reveal_system_flyout").catch(() => undefined);
      });
      void refresh(false);
      requestSoftRefresh(["audio", "power", "perf", "ime", "wifi", "bluetooth"]);
    };
    window.addEventListener("wh-system-flyout-kind", onCustom);

    const unsubs: Array<() => void> = [];
    unsubs.push(
      subscribeSystemDark(() => {
        void syncGlassCss({ kind: "mica-alt", dark: null });
      }),
    );
    unsubs.push(
      subscribeIslandPrefs((p) => setPreviewSound(p.volumePreviewSound)),
    );
    void listen<string>("system-flyout-opened", (ev) => {
      const next = parseKind(typeof ev.payload === "string" ? ev.payload : "wifi");
      applyKind(next);
      // Show cache immediately — never force-wait for WiFi/BT scan.
      void refresh(false);
      if (next === "wifi") requestSoftRefresh(["wifi"]);
      else if (next === "bluetooth") requestSoftRefresh(["bluetooth"]);
      else if (next === "volume") requestSoftRefresh(["audio"]);
      else if (next === "power") requestSoftRefresh(["power"]);
      else if (next === "ime") requestSoftRefresh(["ime"]);
    }).then((fn) => unsubs.push(fn));
    void listen<SystemRadioSnapshot>("system-status-updated", (ev) => {
      const next = ev.payload;
      if (!next) return;
      setSnap((prev) => {
        if (draggingRef.current && prev) {
          return { ...next, volume: prev.volume };
        }
        if (prev?.wifi?.networks?.length) {
          const pwMap = new Map(
            prev.wifi.networks
              .filter((n) => n.password)
              .map((n) => [n.ssid, n.password as string]),
          );
          if (pwMap.size > 0) {
            next.wifi = {
              ...next.wifi,
              networks: next.wifi.networks.map((n) =>
                pwMap.has(n.ssid) ? { ...n, password: pwMap.get(n.ssid) } : n,
              ),
            };
          }
        }
        return next;
      });
      snapRef.current = next;
      if (!draggingRef.current) setVolLocal(null);
    }).then((fn) => unsubs.push(fn));

    return () => {
      window.clearTimeout(tEnter);
      window.clearTimeout(previewTimer.current);
      window.clearTimeout(wheelCommitTimer.current);
      window.removeEventListener("wh-system-flyout-kind", onCustom);
      unsubs.forEach((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (kind === "calendar" || kind === "volume") return;
    // Cache reads are cheap; soft-refresh domains in background at a low cadence.
    const ms = kind === "ime" || kind === "power" ? 4000 : 8000;
    const domains =
      kind === "wifi"
        ? ["wifi"]
        : kind === "bluetooth"
          ? ["bluetooth"]
          : kind === "ime"
            ? ["ime"]
            : ["power"];
    const poll = window.setInterval(() => {
      requestSoftRefresh(domains);
      void refresh(false);
    }, ms);
    return () => window.clearInterval(poll);
  }, [kind]);

  /** 声音面板：加载输出设备列表 */
  useEffect(() => {
    if (kind !== "volume") return;
    void loadAudioDevices();
    const poll = window.setInterval(() => void loadAudioDevices(), 4000);
    return () => window.clearInterval(poll);
  }, [kind]);

  /** 声音面板：滚轮调音量（非 passive，避免面板滚动抢事件） */
  useEffect(() => {
    if (kind !== "volume") return;
    const el = shellRef.current;
    if (!el) return;

    const onWheel = (e: WheelEvent) => {
      // 设备列表内保留滚动；其余区域滚轮调音量
      const t = e.target as HTMLElement | null;
      if (t?.closest?.(".sf-audio-list")) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.deltaY === 0) return;
      // 向下滚减小，向上滚增大；Shift / 触控板大步长时按像素比例缩放
      const stepBase = e.shiftKey ? 5 : 2;
      const steps = Math.max(1, Math.min(8, Math.round(Math.abs(e.deltaY) / 40)));
      const delta = (e.deltaY > 0 ? -1 : 1) * stepBase * steps;
      const cur = volLocalRef.current ?? snapRef.current?.volume.level ?? 0;
      const next = Math.max(0, Math.min(100, cur + delta));
      const wasMuted = Boolean(snapRef.current?.volume.muted) || wheelUnmute.current;
      if (next === cur && !wasMuted) return;

      if (wasMuted) wheelUnmute.current = true;
      pendingWheelVol.current = next;
      volLocalRef.current = next;
      setVolLocal(next);
      setSnap((prev) =>
        prev ? { ...prev, volume: { ...prev.volume, level: next, muted: false } } : prev,
      );

      window.clearTimeout(wheelCommitTimer.current);
      wheelCommitTimer.current = window.setTimeout(() => {
        const level = pendingWheelVol.current ?? next;
        const needUnmute = wheelUnmute.current;
        wheelUnmute.current = false;
        pendingWheelVol.current = null;
        void (async () => {
          try {
            if (needUnmute) {
              await invoke("set_system_volume_muted", { muted: false });
            }
            const vol = await invoke<VolumeSnapshot>("set_system_volume", { level });
            setSnap((prev) => (prev ? { ...prev, volume: vol } : prev));
            setVolLocal(vol.level);
            volLocalRef.current = vol.level;
            if (previewSoundRef.current) {
              window.clearTimeout(previewTimer.current);
              previewTimer.current = window.setTimeout(() => {
                void invoke("play_volume_preview").catch(() => undefined);
              }, 140);
            }
          } catch (err) {
            setMsg(String(err));
          }
        })();
      }, 60);
    };

    el.addEventListener("wheel", onWheel, { passive: false });
    return () => {
      el.removeEventListener("wheel", onWheel);
      window.clearTimeout(wheelCommitTimer.current);
    };
  }, [kind]);

  async function connectWifi(ssid: string) {
    setBusy(ssid);
    setMsg("");
    try {
      await invoke("connect_wifi_network", { ssid });
      requestSoftRefresh(["wifi"]);
      void refresh(false);
    } catch (e) {
      setMsg(String(e));
    } finally {
      setBusy(null);
    }
  }

  function schedulePreview() {
    if (!previewSound) return;
    window.clearTimeout(previewTimer.current);
    previewTimer.current = window.setTimeout(() => {
      void invoke("play_volume_preview").catch(() => undefined);
    }, 140);
  }

  async function commitVolume(level: number) {
    try {
      const vol = await invoke<VolumeSnapshot>("set_system_volume", { level });
      setSnap((prev) => (prev ? { ...prev, volume: vol } : prev));
      setVolLocal(vol.level);
      schedulePreview();
    } catch (e) {
      setMsg(String(e));
    }
  }

  async function toggleMute() {
    const muted = !(snap?.volume.muted ?? false);
    try {
      const vol = await invoke<VolumeSnapshot>("set_system_volume_muted", { muted });
      setSnap((prev) => (prev ? { ...prev, volume: vol } : prev));
      setVolLocal(vol.level);
    } catch (e) {
      setMsg(String(e));
    }
  }

  async function loadAudioDevices() {
    try {
      const list = await invoke<AudioOutputDevice[]>("list_audio_output_devices");
      setAudioDevices(list);
    } catch (e) {
      setAudioDevices([]);
      setMsg(String(e));
    }
  }

  async function selectAudioOutput(id: string) {
    if (!id || busy === id) return;
    const cur = audioDevices.find((d) => d.isDefault);
    if (cur?.id === id) return;
    setBusy(id);
    setMsg("");
    // Optimistic UI
    setAudioDevices((prev) => prev.map((d) => ({ ...d, isDefault: d.id === id })));
    try {
      const vol = await invoke<VolumeSnapshot>("set_audio_output_device", { id });
      setSnap((prev) => (prev ? { ...prev, volume: vol } : prev));
      setVolLocal(vol.level);
      await loadAudioDevices();
      schedulePreview();
    } catch (e) {
      setMsg(String(e));
      await loadAudioDevices();
    } finally {
      setBusy(null);
    }
  }

  const title =
    kind === "bluetooth"
      ? "蓝牙"
      : kind === "volume"
        ? "声音"
        : kind === "ime"
          ? "输入法"
          : kind === "power"
            ? "电源"
            : kind === "calendar"
              ? "日历"
              : "Wi‑Fi";

  const volLevel = volLocal ?? snap?.volume.level ?? 0;
  const volMuted = Boolean(snap?.volume.muted);
  const power = snap?.power;
  const month = useMemo(() => buildMonth(calCursor), [calCursor]);
  const today = new Date();
  const wifiNets = snap?.wifi.networks ?? [];
  const wifiConnected = wifiNets.find((n) => n.connected) ?? null;
  const wifiOthers = wifiNets.filter((n) => !n.connected);
  const btDevices = snap?.bluetooth.devices ?? [];
  const btConnected = btDevices.filter((d) => d.connected);
  const btOthers = btDevices.filter((d) => !d.connected);
  const wifiMenuNet =
    wifiMenu != null
      ? wifiNets.find((n) => n.ssid === wifiMenu.ssid) ??
        (wifiMenu.ssid === snap?.wifi.connectedSsid
          ? ({
              ssid: wifiMenu.ssid,
              signal: 0,
              secured: true,
              connected: true,
              saved: true,
              password: null,
            } satisfies WifiNetwork)
          : null)
      : null;
  const pickDay = calPick ?? today;
  const pickFest = festivalName(pickDay);

  return (
    <div
      ref={shellRef}
      className={`system-flyout-shell finder${entering ? " is-enter" : " is-in"}`}
      role="dialog"
      aria-label={title}
    >
      <header className="system-flyout-head">
        <h1>{title}</h1>
        <div className="system-flyout-head-actions">
          {(kind === "wifi" || kind === "bluetooth") && (
            <button
              type="button"
              className="system-flyout-link"
              disabled={refreshing}
              title={kind === "wifi" ? "重新扫描无线网络" : "刷新蓝牙设备"}
              onClick={() => void manualRefresh()}
            >
              {refreshing ? "刷新中…" : "刷新"}
            </button>
          )}
          <button type="button" className="system-flyout-link" onClick={() => void closeSelf()}>
            关闭
          </button>
        </div>
      </header>

      {kind === "wifi" && (
        <>
          <button
            type="button"
            className="sf-pref-link"
            onClick={() => void invoke("open_wifi_settings").catch(console.error)}
          >
            网络偏好设置
          </button>

          <div className="system-flyout-label">当前网络</div>
          {wifiConnected || snap?.wifi.connectedSsid ? (
            <div
              className="system-flyout-item static is-on sf-wifi-hero"
              onContextMenu={(e) =>
                openWifiMenu(e, wifiConnected?.ssid || snap?.wifi.connectedSsid || "")
              }
              title="右键查看详情 / 复制密码"
            >
              <span className="sf-wifi-badge" aria-hidden>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none">
                  <path
                    d="M12 18.5a1.2 1.2 0 1 0 0-2.4 1.2 1.2 0 0 0 0 2.4Z"
                    fill="currentColor"
                  />
                  <path
                    d="M8.6 14.3a6.2 6.2 0 0 1 6.8 0M5.8 11.2a10 10 0 0 1 12.4 0"
                    stroke="currentColor"
                    strokeWidth="1.7"
                    strokeLinecap="round"
                  />
                </svg>
              </span>
              <span className="system-flyout-item-main">
                <span className="system-flyout-item-title">
                  {wifiConnected?.ssid || snap?.wifi.connectedSsid}
                </span>
                <span className="system-flyout-item-sub">
                  {(snap?.wifi.ipv4?.[0] || "已连接") +
                    (wifiConnected && wifiConnected.signal > 0
                      ? ` · ${wifiConnected.signal}%`
                      : "")}
                </span>
              </span>
              {wifiConnected?.secured ? (
                <span className="sf-lock" title="受保护" aria-label="受保护">
                  🔒
                </span>
              ) : null}
            </div>
          ) : (
            <div className="system-flyout-empty">
              {snap?.wifi.radioOn ? "未连接" : "无线局域网已关闭"}
            </div>
          )}

          <div className="system-flyout-label">其他网络</div>
          <div className="system-flyout-list">
            {wifiOthers.length === 0 ? (
              <div className="system-flyout-empty">
                {wifiNets.length === 0 ? "扫描中…" : "暂无其他网络"}
              </div>
            ) : (
              wifiOthers.map((n) => (
                <button
                  key={`wifi-${n.ssid}`}
                  type="button"
                  className="system-flyout-item"
                  disabled={busy === n.ssid}
                  onClick={() => void connectWifi(n.ssid)}
                  onContextMenu={(e) => openWifiMenu(e, n.ssid)}
                  title="左键连接 · 右键详情"
                >
                  <span className="system-flyout-item-main">
                    <span className="system-flyout-item-title">{n.ssid}</span>
                    <span className="system-flyout-item-sub">
                      {n.saved ? "已保存" : n.secured ? "受保护" : "开放"}
                      {n.signal > 0 ? ` · ${n.signal}%` : ""}
                      {busy === n.ssid ? " · 连接中…" : ""}
                    </span>
                  </span>
                  <span className="system-flyout-signal" aria-hidden>
                    {n.signal > 0 ? signalBars(n.signal) : "·"}
                  </span>
                </button>
              ))
            )}
          </div>

          {wifiMenu && wifiMenuNet ? (
            <>
              <button
                type="button"
                className="sf-wifi-menu-backdrop"
                aria-label="关闭菜单"
                onClick={closeWifiMenu}
                onContextMenu={(e) => {
                  e.preventDefault();
                  closeWifiMenu();
                }}
              />
              <div
                className="sf-wifi-menu"
                role="menu"
                style={{ left: wifiMenu.x, top: wifiMenu.y }}
                onClick={(e) => e.stopPropagation()}
              >
                <div className="sf-wifi-menu-head">
                  <div className="sf-wifi-menu-title">{wifiMenuNet.ssid}</div>
                  <div className="sf-wifi-menu-sub">
                    {wifiMenuNet.connected
                      ? "已连接"
                      : wifiMenuNet.saved
                        ? "已保存密码"
                        : wifiMenuNet.secured
                          ? "受保护 · 未保存"
                          : "开放网络"}
                    {wifiMenuNet.signal > 0 ? ` · 信号 ${wifiMenuNet.signal}%` : ""}
                  </div>
                </div>

                {wifiMenuNet.connected ? (
                  <div className="sf-wifi-menu-block">
                    <div className="sf-wifi-menu-row">
                      <span className="sf-wifi-menu-k">IPv4</span>
                      <span className="sf-wifi-menu-v mono sf-ip-list">
                        {(snap?.wifi.ipv4?.length ?? 0) === 0 ? (
                          <span>—</span>
                        ) : (
                          snap!.wifi.ipv4.map((ip) => (
                            <button
                              key={`v4-${ip}`}
                              type="button"
                              className="sf-ip-chip"
                              title="点击复制"
                              onClick={() => void copyText(ip, "已复制 IPv4")}
                            >
                              {ip}
                            </button>
                          ))
                        )}
                      </span>
                    </div>
                    <div className="sf-wifi-menu-row">
                      <span className="sf-wifi-menu-k">IPv6</span>
                      <span className="sf-wifi-menu-v mono sf-ip-list">
                        {(snap?.wifi.ipv6?.length ?? 0) === 0 ? (
                          <span>—</span>
                        ) : (
                          snap!.wifi.ipv6.map((ip) => (
                            <button
                              key={`v6-${ip}`}
                              type="button"
                              className="sf-ip-chip"
                              title="点击复制"
                              onClick={() => void copyText(ip, "已复制 IPv6")}
                            >
                              {ip}
                            </button>
                          ))
                        )}
                      </span>
                    </div>
                  </div>
                ) : null}

                {wifiMenuNet.saved ? (
                  <div className="sf-wifi-menu-block">
                    {wifiMenuNet.password ? (
                      <div className="sf-pw-line">
                        密码：{showMenuPw ? wifiMenuNet.password : "••••••••"}
                        <button
                          type="button"
                          className="sf-mini-btn"
                          onClick={() => setShowMenuPw((v) => !v)}
                        >
                          {showMenuPw ? "隐藏" : "显示"}
                        </button>
                        <button
                          type="button"
                          className="sf-mini-btn"
                          onClick={() =>
                            void copyText(wifiMenuNet.password || "", "已复制密码")
                          }
                        >
                          复制
                        </button>
                      </div>
                    ) : (
                      <div className="sf-pw-line">
                        <button
                          type="button"
                          className="sf-mini-btn"
                          disabled={pwLoading}
                          onClick={() => {
                            setPwLoading(true);
                            void ensureWifiPassword(wifiMenuNet.ssid).then((pw) => {
                              setPwLoading(false);
                              if (pw) setShowMenuPw(true);
                              else setMsg("密码需管理员权限读取");
                            });
                          }}
                        >
                          {pwLoading ? "读取中…" : "显示密码"}
                        </button>
                      </div>
                    )}
                  </div>
                ) : null}

                <div className="sf-wifi-menu-actions">
                  <button
                    type="button"
                    className="sf-wifi-menu-btn"
                    onClick={() => void copyText(wifiMenuNet.ssid, "已复制名称")}
                  >
                    复制网络名称
                  </button>
                  {!wifiMenuNet.connected ? (
                    <button
                      type="button"
                      className="sf-wifi-menu-btn primary"
                      disabled={busy === wifiMenuNet.ssid}
                      onClick={() => {
                        closeWifiMenu();
                        void connectWifi(wifiMenuNet.ssid);
                      }}
                    >
                      {busy === wifiMenuNet.ssid ? "连接中…" : "连接"}
                    </button>
                  ) : null}
                </div>
              </div>
            </>
          ) : null}
        </>
      )}

      {kind === "bluetooth" && (
        <>
          <button
            type="button"
            className="sf-pref-link"
            onClick={() => void invoke("open_bluetooth_settings").catch(console.error)}
          >
            蓝牙偏好设置
          </button>

          <div className="system-flyout-label">已连接</div>
          {btConnected.length === 0 ? (
            <div className="system-flyout-empty sf-bt-empty">
              {snap?.bluetooth.radioOn === false ? "蓝牙已关闭" : "无已连接设备"}
            </div>
          ) : (
            <div className="sf-bt-connected">
              {btConnected.map((d) => (
                <div key={d.id} className="system-flyout-item static is-on sf-bt-hero">
                  <span className="sf-bt-badge" aria-hidden>
                    <BtDeviceGlyph kind={d.kind} name={d.name} />
                  </span>
                  <span className="system-flyout-item-main">
                    <span className="system-flyout-item-title">{d.name}</span>
                    <span className="system-flyout-item-sub">
                      已连接 · {btKindLabel(d.kind)}
                    </span>
                  </span>
                  {d.battery != null ? <BtBatt percent={d.battery} /> : null}
                  <button
                    type="button"
                    className="sf-bt-action"
                    disabled={busy === d.id}
                    onClick={() => void toggleBt(d.id, false)}
                  >
                    {busy === d.id ? "…" : "断开"}
                  </button>
                </div>
              ))}
            </div>
          )}

          <div className="system-flyout-label">其他设备</div>
          <div className="system-flyout-list">
            {btOthers.length === 0 ? (
              <div className="system-flyout-empty sf-bt-empty">
                {btDevices.length === 0 ? "暂无已配对设备" : "无其他设备"}
              </div>
            ) : (
              btOthers.map((d) => (
                <div key={d.id} className="system-flyout-item static sf-bt-row">
                  <span className="sf-dev-wrap">
                    <BtDeviceGlyph kind={d.kind} name={d.name} />
                  </span>
                  <span className="system-flyout-item-main">
                    <span className="system-flyout-item-title">{d.name}</span>
                    <span className="system-flyout-item-sub">{btKindLabel(d.kind)}</span>
                  </span>
                  {d.battery != null ? <BtBatt percent={d.battery} /> : null}
                  <button
                    type="button"
                    className="sf-bt-action primary"
                    disabled={busy === d.id}
                    onClick={() => void toggleBt(d.id, true)}
                  >
                    {busy === d.id ? "…" : "连接"}
                  </button>
                </div>
              ))
            )}
          </div>
        </>
      )}

      {kind === "volume" && (
        <>
          <button
            type="button"
            className="sf-pref-link"
            onClick={() => void invoke("open_sound_settings").catch(console.error)}
          >
            声音偏好设置
          </button>
          <section className="system-flyout-card system-flyout-volume compact">
            <div className="system-flyout-volume-top">
              <button
                type="button"
                className={`system-flyout-mute${volMuted ? " is-muted" : ""}`}
                title={volMuted ? "取消静音" : "静音"}
                onClick={() => void toggleMute()}
              >
                {volMuted ? "🔇" : "🔊"}
              </button>
              <span className="system-flyout-volume-pct">{volMuted ? "静音" : `${volLevel}%`}</span>
            </div>
            <input
              className="system-flyout-slider"
              type="range"
              min={0}
              max={100}
              step={1}
              value={volMuted ? 0 : volLevel}
              onPointerDown={() => {
                draggingRef.current = true;
              }}
              onPointerUp={(e) => {
                draggingRef.current = false;
                const v = Number((e.target as HTMLInputElement).value);
                void commitVolume(v);
              }}
              onPointerCancel={() => {
                draggingRef.current = false;
              }}
              onChange={(e) => {
                const v = Number(e.target.value);
                setVolLocal(v);
                setSnap((prev) =>
                  prev ? { ...prev, volume: { ...prev.volume, level: v, muted: false } } : prev,
                );
              }}
              aria-label="主音量"
            />
          </section>

          <div className="system-flyout-label">输出设备</div>
          <div className="system-flyout-list sf-audio-list">
            {audioDevices.length === 0 ? (
              <div className="system-flyout-empty">暂无可用输出设备</div>
            ) : (
              audioDevices.map((d) => (
                <button
                  key={d.id}
                  type="button"
                  className={`system-flyout-item sf-audio-row${d.isDefault ? " is-on" : ""}`}
                  disabled={busy === d.id}
                  onClick={() => void selectAudioOutput(d.id)}
                >
                  <span className="sf-dev-wrap" aria-hidden>
                    <AudioOutGlyph name={d.name} />
                  </span>
                  <span className="system-flyout-item-main">
                    <span className="system-flyout-item-title">{d.name}</span>
                    <span className="system-flyout-item-sub">
                      {d.isDefault ? "当前输出" : busy === d.id ? "切换中…" : "点击切换"}
                    </span>
                  </span>
                  {d.isDefault ? <span className="sf-audio-check" aria-hidden>✓</span> : null}
                </button>
              ))
            )}
          </div>
        </>
      )}

      {kind === "power" && (
        <>
          <button
            type="button"
            className="sf-pref-link"
            onClick={() => void invoke("open_power_settings").catch(console.error)}
          >
            电池偏好设置
          </button>
          <section className="system-flyout-card compact">
            <div className="sf-power-hero">
              <span className="sf-power-pct">
                {power?.hasBattery ? `${power.percent ?? "—"}%` : "交流电"}
              </span>
              <span className="sf-power-state">
                {!power?.hasBattery
                  ? "台式机 / 无电池"
                  : power.charging
                    ? "充电中"
                    : power.acLine === "online"
                      ? "已接通电源"
                      : "使用电池"}
              </span>
            </div>
            {power?.hasBattery ? (
              <div className="system-flyout-row">
                <span className="system-flyout-k">剩余</span>
                <span className="system-flyout-v">{formatLife(power?.lifeSec)}</span>
              </div>
            ) : null}
          </section>
        </>
      )}

      {kind === "ime" && (
        <>
          <section className="system-flyout-card system-flyout-ime compact">
            <div className="system-flyout-ime-marks" aria-hidden>
              <span className="system-flyout-ime-mark">{snap?.ime.mark || "—"}</span>
              {snap?.ime.caps && snap?.ime.mode === "zh" ? (
                <span className="system-flyout-caps">A</span>
              ) : null}
            </div>
            <div className="system-flyout-row">
              <span className="system-flyout-k">输入法</span>
              <span className="system-flyout-v">{snap?.ime.name || "输入法"}</span>
            </div>
            <div className="system-flyout-row">
              <span className="system-flyout-k">模式</span>
              <span className="system-flyout-v">
                {snap?.ime.mode === "zh"
                  ? "中文"
                  : snap?.ime.mode === "en"
                    ? snap?.ime.caps
                      ? "英文 · 大写"
                      : "英文 · 小写"
                    : "—"}
              </span>
            </div>
          </section>
          <button
            type="button"
            className="system-flyout-primary"
            onClick={() => {
              void invoke("open_ime_picker")
                .then(() => closeSelf())
                .catch(console.error);
            }}
          >
            切换输入法（Win + 空格）
          </button>
        </>
      )}

      {kind === "calendar" && (
        <>
          <div className="sf-cal-nav">
            <button
              type="button"
              className="sf-cal-nav-btn"
              onClick={() => setCalCursor(new Date(month.y, month.m - 1, 1))}
            >
              ‹
            </button>
            <span className="sf-cal-title">
              {month.y} 年 {month.m + 1} 月
            </span>
            <button
              type="button"
              className="sf-cal-nav-btn"
              onClick={() => setCalCursor(new Date(month.y, month.m + 1, 1))}
            >
              ›
            </button>
          </div>
          <div className="sf-cal-week">
            {["一", "二", "三", "四", "五", "六", "日"].map((d) => (
              <span key={d}>{d}</span>
            ))}
          </div>
          <div className="sf-cal-grid">
            {month.cells.map((c, i) => {
              const isToday =
                c.date &&
                c.date.getFullYear() === today.getFullYear() &&
                c.date.getMonth() === today.getMonth() &&
                c.date.getDate() === today.getDate();
              const isPick =
                c.date &&
                calPick &&
                c.date.getFullYear() === calPick.getFullYear() &&
                c.date.getMonth() === calPick.getMonth() &&
                c.date.getDate() === calPick.getDate();
              if (c.day == null) {
                return <span key={i} className="sf-cal-day is-empty" />;
              }
              const isOff = c.holiday === "off";
              const isWork = c.holiday === "work";
              const isWeekend = Boolean(c.weekend) && !isWork;
              const mark = isOff ? "休" : isWork ? "班" : null;
              return (
                <button
                  key={i}
                  type="button"
                  className={`sf-cal-day${isToday ? " is-today" : ""}${isPick ? " is-pick" : ""}${
                    c.fest ? " has-fest" : ""
                  }${isOff ? " is-off" : ""}${isWork ? " is-work" : ""}${
                    isWeekend ? " is-weekend" : ""
                  }`}
                  onClick={() => setCalPick(c.date ?? null)}
                >
                  {mark ? <span className="sf-cal-mark">{mark}</span> : null}
                  <span className="sf-cal-num">{c.day}</span>
                  {c.fest ? <span className="sf-cal-fest">{c.fest}</span> : null}
                </button>
              );
            })}
          </div>
          <div className="sf-cal-foot">
            {pickDay.getMonth() + 1}月{pickDay.getDate()}日
            {pickFest ? ` · ${pickFest}` : ""}
            {holidayKind(pickDay) === "off"
              ? " · 休"
              : holidayKind(pickDay) === "work"
                ? " · 班"
                : ""}
          </div>
          <button
            type="button"
            className="system-flyout-footer-btn"
            onClick={() => {
              void invoke("open_notification_center")
                .then(() => closeSelf())
                .catch(console.error);
            }}
          >
            打开通知中心
          </button>
        </>
      )}

      {msg ? <p className="system-flyout-msg">{msg}</p> : null}
    </div>
  );
}

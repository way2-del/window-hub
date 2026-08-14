import { useEffect, useMemo, useRef, useState, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { SystemRadioSnapshot } from "../SystemFlyoutApp";

export type TrayIconInfo = {
  id: string;
  tooltip: string;
  process: string;
  uid: number;
  hwnd: number;
  callback_msg: number;
  version?: number;
  icon_png_base64: string;
  area: string;
  flashing?: boolean;
};

export type TrayPrefs = {
  pinned: string[];
  menu_heights?: Record<string, number>;
  muted?: string[];
  muted_processes?: string[];
  system_chips?: SystemChipVisibility;
};

export type SystemChipVisibility = {
  perf: boolean;
  network: boolean;
  wifi: boolean;
  bluetooth: boolean;
  volume: boolean;
  power: boolean;
  peripherals: boolean;
  ime: boolean;
  clock: boolean;
};

export const DEFAULT_SYSTEM_CHIPS: SystemChipVisibility = {
  perf: true,
  network: true,
  wifi: true,
  bluetooth: true,
  volume: true,
  power: true,
  peripherals: true,
  ime: true,
  clock: true,
};

export function normalizeSystemChips(
  raw?: Partial<SystemChipVisibility> | null,
): SystemChipVisibility {
  return { ...DEFAULT_SYSTEM_CHIPS, ...(raw ?? {}) };
}

export function isTrayNotifyMuted(
  icon: Pick<TrayIconInfo, "id" | "process" | "tooltip">,
  prefs: Pick<TrayPrefs, "muted" | "muted_processes">,
): boolean {
  if ((prefs.muted ?? []).includes(icon.id)) return true;
  const proc = (icon.process || "").trim().toLowerCase();
  const mutedProcs = (prefs.muted_processes ?? []).map((p) => p.trim().toLowerCase()).filter(Boolean);
  if (proc && mutedProcs.includes(proc)) return true;
  const tip = (icon.tooltip || "").toLowerCase();
  return mutedProcs.some((p) => tip.includes(p));
}

const TRAY_POPUP_W = 280;
const SYSTEM_FLYOUT_W = 280;
const TRAY_POPUP_GAP = 6;

function pad2(n: number) {
  return n.toString().padStart(2, "0");
}

function formatMenuClock(d: Date) {
  const week = ["日", "一", "二", "三", "四", "五", "六"][d.getDay()];
  return `${d.getMonth() + 1}月${d.getDate()}日 周${week} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

export function trayLabel(icon: TrayIconInfo) {
  return icon.tooltip || icon.process || "未知应用";
}

/** Host may omit unchanged PNG (empty base64) — keep previous glyph. */
export function mergeTrayIcons(
  prev: TrayIconInfo[],
  next: TrayIconInfo[],
): TrayIconInfo[] {
  if (!prev.length) return next;
  const prevMap = new Map(prev.map((i) => [i.id, i]));
  return next.map((n) => {
    if (n.icon_png_base64) return n;
    const old = prevMap.get(n.id);
    if (old?.icon_png_base64) {
      return { ...n, icon_png_base64: old.icon_png_base64 };
    }
    return n;
  });
}

const TrayGlyph = memo(function TrayGlyph({ icon }: { icon: TrayIconInfo }) {
  if (icon.icon_png_base64) {
    return (
      <img
        className="tray-glyph"
        src={`data:image/png;base64,${icon.icon_png_base64}`}
        alt=""
        draggable={false}
      />
    );
  }
  const letter = trayLabel(icon).charAt(0).toUpperCase();
  return <span className="tray-glyph tray-glyph-fallback">{letter}</span>;
});

function TrayClockButton({
  open,
  onToggle,
}: {
  open: boolean;
  onToggle: (el: HTMLElement | null) => void;
}) {
  const [now, setNow] = useState(() => new Date());
  const clockRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const t = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(t);
  }, []);
  return (
    <button
      ref={clockRef}
      type="button"
      className={`tray-clock${open ? " is-open" : ""}`}
      title="日历"
      aria-expanded={open}
      onMouseDown={(e) => {
        e.preventDefault();
        void invoke("suppress_system_flyout_blur", { ms: 350 });
      }}
      onClick={() => onToggle(clockRef.current)}
    >
      <time dateTime={now.toISOString()}>{formatMenuClock(now)}</time>
    </button>
  );
}

function WifiGlyph({ on, signal = 0 }: { on: boolean; signal?: number }) {
  const level =
    !on || signal <= 0
      ? on
        ? 3
        : 0
      : signal >= 80
        ? 4
        : signal >= 55
          ? 3
          : signal >= 30
            ? 2
            : 1;
  return (
    <svg width="17" height="17" viewBox="0 0 24 24" fill="none" aria-hidden>
      <path
        d="M12 18.5a1.25 1.25 0 1 0 0-2.5 1.25 1.25 0 0 0 0 2.5Z"
        fill="currentColor"
        opacity={on ? 1 : 0.35}
      />
      <path
        d="M8.5 14.2a6.5 6.5 0 0 1 7 0"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        opacity={level >= 1 ? (on ? 0.95 : 0.3) : 0.14}
      />
      <path
        d="M5.4 11a11 11 0 0 1 13.2 0"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        opacity={level >= 2 ? (on ? 0.8 : 0.22) : 0.12}
      />
      <path
        d="M3.4 8.2a14.2 14.2 0 0 1 17.2 0"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        opacity={level >= 3 ? (on ? 0.65 : 0.18) : 0.1}
      />
      <path
        d="M1.6 5.4a17.8 17.8 0 0 1 20.8 0"
        stroke="currentColor"
        strokeWidth="1.7"
        strokeLinecap="round"
        opacity={level >= 4 ? (on ? 0.5 : 0.14) : 0.08}
      />
    </svg>
  );
}

function BtGlyph({ on }: { on: boolean }) {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden opacity={on ? 1 : 0.4}>
      <path
        d="M7 7.5 17 16.5 12 21V3l5 4.5L7 16.5"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

function VolumeGlyph({ muted, level }: { muted: boolean; level: number }) {
  if (muted || level <= 0) {
    return (
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden>
        <path d="M4 10v4h3.2L12 18V6L7.2 10H4Z" stroke="currentColor" strokeWidth="1.45" strokeLinejoin="round" />
        <path d="m16 9 5 6M21 9l-5 6" stroke="currentColor" strokeWidth="1.45" strokeLinecap="round" />
      </svg>
    );
  }
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden>
      <path d="M4 10v4h3.2L12 18V6L7.2 10H4Z" stroke="currentColor" strokeWidth="1.45" strokeLinejoin="round" />
      <path
        d="M15.2 9.2a4.2 4.2 0 0 1 0 5.6"
        stroke="currentColor"
        strokeWidth="1.45"
        strokeLinecap="round"
        opacity={level < 35 ? 0.35 : 1}
      />
      <path
        d="M17.6 6.8a7.2 7.2 0 0 1 0 10.4"
        stroke="currentColor"
        strokeWidth="1.45"
        strokeLinecap="round"
        opacity={level < 65 ? 0.25 : 0.9}
      />
    </svg>
  );
}

function PowerGlyph({ percent, charging, ac }: { percent?: number | null; charging: boolean; ac: boolean }) {
  const p = percent == null ? 100 : Math.max(0, Math.min(100, percent));
  const fillH = Math.round((p / 100) * 8);
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden>
      <rect x="6" y="6" width="12" height="12" rx="2" stroke="currentColor" strokeWidth="1.5" />
      <rect x="9.5" y="3.5" width="5" height="2" rx="0.6" fill="currentColor" opacity="0.85" />
      <rect
        x="8"
        y={16 - fillH}
        width="8"
        height={fillH}
        rx="1"
        fill="currentColor"
        opacity={charging || ac ? 0.95 : 0.75}
      />
      {charging ? (
        <path d="M12 9.2 10.6 12.2h2.6L11.8 15.2" stroke="#ffd60a" strokeWidth="1.2" strokeLinecap="round" />
      ) : null}
    </svg>
  );
}

function HeadphoneGlyph() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden>
      <path
        d="M4.5 13v2.5A2.5 2.5 0 0 0 7 18h1.2v-5H7A2.5 2.5 0 0 0 4.5 13Zm15 0A2.5 2.5 0 0 0 17 10.5h-1.2v5H17a2.5 2.5 0 0 0 2.5-2.5V13Z"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <path d="M4.5 13a7.5 7.5 0 0 1 15 0" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

function tempBarFill(c: number) {
  // Map ~30–95°C into a readable bar.
  return Math.max(8, Math.min(100, Math.round(((c - 28) / 67) * 100)));
}

/** 温度计 + 旁侧：固定上 CPU / 下 GPU（无读数时显示 —） */
function TempMeter({
  cpuTempC,
  gpuTempC,
  cpuPercent,
}: {
  cpuTempC?: number | null;
  gpuTempC?: number | null;
  cpuPercent: number;
}) {
  const hasCpu = cpuTempC != null;
  const hasGpu = gpuTempC != null;
  // 液位与「当前主要读数」一致，避免 CPU 空值时液位却跟着 GPU 走、看起来对不上
  const fillSrc =
    hasCpu && hasGpu
      ? Math.max(cpuTempC!, gpuTempC!)
      : hasCpu
        ? cpuTempC!
        : hasGpu
          ? gpuTempC!
          : null;
  const fill = fillSrc != null ? tempBarFill(fillSrc) : cpuPercent;
  const h = Math.max(2, Math.min(100, Math.round(fill)));
  const titleParts = [
    hasCpu
      ? `CPU ${cpuTempC}°C · 占用 ${cpuPercent}%`
      : `CPU 占用 ${cpuPercent}%`,
  ];
  if (hasGpu) titleParts.push(`GPU ${gpuTempC}°C`);
  if (!hasCpu && !hasGpu) titleParts.push("温度暂不可用");
  const title = titleParts.join(" · ");

  return (
    <span className="tray-perf-temp" title={title} aria-label={title}>
      <span className="tray-perf-thermo" aria-hidden>
        <span className="tray-perf-bar">
          <span className="tray-perf-fill" style={{ height: `${h}%` }} />
        </span>
        <span className="tray-perf-bulb" />
      </span>
      <span className="tray-perf-temps">
        <span className={`tray-perf-num is-cpu${!hasCpu ? " is-empty" : ""}`}>
          <span className="tray-perf-tag">C</span>
          {hasCpu ? `${cpuTempC}°` : "—"}
        </span>
        <span className={`tray-perf-num is-gpu${!hasGpu ? " is-empty" : ""}`}>
          <span className="tray-perf-tag">G</span>
          {hasGpu ? `${gpuTempC}°` : "—"}
        </span>
      </span>
    </span>
  );
}

/** 内存条 + 百分比数字 */
function MemMeter({ percent }: { percent: number }) {
  const h = Math.max(2, Math.min(100, Math.round(percent)));
  const title = `内存 ${percent}%`;
  return (
    <span className="tray-perf-mem" title={title} aria-label={title}>
      <span className="tray-perf-label">MEM</span>
      <span className="tray-perf-bar" aria-hidden>
        <span className="tray-perf-fill" style={{ height: `${h}%` }} />
      </span>
      <span className="tray-perf-num">{percent}%</span>
    </span>
  );
}

/** 托盘网速：数值 + 单位拆开，避免三位数把单位挤没 */
function formatRateParts(bps: number): { n: string; u: string } {
  if (!Number.isFinite(bps) || bps < 0) return { n: "0", u: "B" };
  if (bps < 1024) return { n: `${Math.round(bps)}`, u: "B" };
  if (bps < 1024 * 1024) {
    const kb = bps / 1024;
    if (kb >= 10) return { n: `${Math.round(kb)}`, u: "K" };
    return { n: kb.toFixed(1), u: "K" };
  }
  const mb = bps / (1024 * 1024);
  if (mb >= 100) return { n: `${Math.round(mb)}`, u: "M" };
  if (mb >= 10) return { n: mb.toFixed(1), u: "M" };
  return { n: mb.toFixed(2), u: "M" };
}

function formatRate(bps: number): string {
  const { n, u } = formatRateParts(bps);
  return `${n}${u}`;
}

function NetRateNum({ bps }: { bps: number }) {
  const { n, u } = formatRateParts(bps);
  return (
    <span className="tray-net-num">
      <span className="tray-net-val">{n}</span>
      <span className="tray-net-unit">{u}</span>
    </span>
  );
}

/** 上行 / 下行速率芯片 */
function NetMeter({ downBps, upBps }: { downBps: number; upBps: number }) {
  const title = `↑ ${formatRate(upBps)}/s · ↓ ${formatRate(downBps)}/s · 点击查看流量`;
  return (
    <span className="tray-net" title={title} aria-label={title}>
      <span className="tray-net-line is-up">
        <span className="tray-net-arrow" aria-hidden>
          ↑
        </span>
        <NetRateNum bps={upBps} />
      </span>
      <span className="tray-net-line is-down">
        <span className="tray-net-arrow" aria-hidden>
          ↓
        </span>
        <NetRateNum bps={downBps} />
      </span>
    </span>
  );
}

function GamepadGlyph() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" aria-hidden>
      <path
        d="M7 8.5h10a4 4 0 0 1 3.7 5.5l-1.2 3A2.2 2.2 0 0 1 17.5 18h-11a2.2 2.2 0 0 1-2-1l-1.2-3A4 4 0 0 1 7 8.5Z"
        stroke="currentColor"
        strokeWidth="1.6"
      />
      <path d="M9 12v3M7.5 13.5h3" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" />
      <circle cx="15.2" cy="12.6" r="0.9" fill="currentColor" />
      <circle cx="17.2" cy="14.4" r="0.9" fill="currentColor" />
    </svg>
  );
}

async function clickTray(icon: TrayIconInfo, action: "left" | "right") {
  try {
    await invoke("invoke_tray_icon", {
      id: icon.id,
      hwnd: icon.hwnd,
      callbackMsg: icon.callback_msg,
      uid: icon.uid,
      version: icon.version ?? 0,
      action,
    });
  } catch (e) {
    console.error(e);
  }
}

/** Cached window metrics — avoids 2 IPC awaits on every tray/flyout open. */
let cachedScale = 0;
let cachedOuterX = 0;
let cachedOuterY = 0;
let cacheAt = 0;

async function refreshWindowCache() {
  const win = getCurrentWindow();
  const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  cachedScale = factor;
  cachedOuterX = outer.x;
  cachedOuterY = outer.y;
  cacheAt = Date.now();
}

async function popupAnchor(el: HTMLElement, width: number) {
  const rect = el.getBoundingClientRect();
  const fresh = Date.now() - cacheAt < 2500 && cachedScale > 0;
  if (!fresh) {
    await refreshWindowCache();
  } else {
    void refreshWindowCache();
  }
  const factor = cachedScale || 1;
  const logicalX = cachedOuterX / factor;
  const logicalY = cachedOuterY / factor;
  return {
    x: logicalX + rect.right - width,
    y: logicalY + rect.bottom + TRAY_POPUP_GAP,
  };
}

export default function TrayCluster({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [icons, setIcons] = useState<TrayIconInfo[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  const [muted, setMuted] = useState<string[]>([]);
  const [mutedProcesses, setMutedProcesses] = useState<string[]>([]);
  const [systemChips, setSystemChips] = useState<SystemChipVisibility>(DEFAULT_SYSTEM_CHIPS);
  const [radio, setRadio] = useState<SystemRadioSnapshot | null>(null);
  const [flyoutKind, setFlyoutKind] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const chevronRef = useRef<HTMLButtonElement>(null);
  const wifiRef = useRef<HTMLButtonElement>(null);
  const btRef = useRef<HTMLButtonElement>(null);
  const volRef = useRef<HTMLButtonElement>(null);
  const powerRef = useRef<HTMLButtonElement>(null);
  const imeRef = useRef<HTMLButtonElement>(null);
  const memRef = useRef<HTMLButtonElement>(null);
  const netRef = useRef<HTMLButtonElement>(null);
  const togglingRef = useRef(false);
  const radioSigRef = useRef("");

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    void (async () => {
      try {
        const [list, prefs, snap] = await Promise.all([
          invoke<TrayIconInfo[]>("list_tray_icons"),
          invoke<TrayPrefs>("get_tray_prefs"),
          invoke<SystemRadioSnapshot>("get_system_radio_snapshot", { force: false }),
        ]);
        if (!cancelled) {
          setIcons(list);
          setPinned(prefs.pinned ?? []);
          setMuted(prefs.muted ?? []);
          setMutedProcesses(prefs.muted_processes ?? []);
          setSystemChips(normalizeSystemChips(prefs.system_chips));
          setRadio(snap);
          radioSigRef.current = JSON.stringify({
            w: snap?.wifi?.connectedSsid,
            b: snap?.bluetooth?.radioOn,
            v: snap?.volume?.level,
            m: snap?.volume?.muted,
            p: snap?.power?.percent,
            i: snap?.ime?.mark,
            c: snap?.perf?.cpuPercent,
            ct: snap?.perf?.cpuTempC ?? null,
            gt: snap?.perf?.gpuTempC ?? null,
            mp: snap?.perf?.memPercent,
            nd: Math.round((snap?.perf?.downBps ?? 0) / 256),
            nu: Math.round((snap?.perf?.upBps ?? 0) / 256),
          });
        }
      } catch {
        /* noop */
      }

      try {
        unsubs.push(
          await listen<TrayIconInfo[]>("tray-icons", (ev) => {
            setIcons((prev) => mergeTrayIcons(prev, ev.payload ?? []));
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen<TrayPrefs>("tray-prefs", (ev) => {
            setPinned(ev.payload.pinned ?? []);
            setMuted(ev.payload.muted ?? []);
            setMutedProcesses(ev.payload.muted_processes ?? []);
            setSystemChips(normalizeSystemChips(ev.payload.system_chips));
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("tray-popup-opened", () => {
            onOpenChange(true);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("tray-popup-closed", () => {
            onOpenChange(false);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen<string>("system-flyout-opened", (ev) => {
            setFlyoutKind(typeof ev.payload === "string" ? ev.payload : "wifi");
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("system-flyout-closed", () => {
            setFlyoutKind(null);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen<SystemRadioSnapshot>("system-status-updated", (ev) => {
            if (cancelled || !ev.payload) return;
            const snap = ev.payload;
            const sig = JSON.stringify({
              w: snap.wifi?.connectedSsid,
              b: snap.bluetooth?.radioOn,
              v: snap.volume?.level,
              m: snap.volume?.muted,
              p: snap.power?.percent,
              i: snap.ime?.mark,
              c: snap.perf?.cpuPercent,
              ct: snap.perf?.cpuTempC ?? null,
              gt: snap.perf?.gpuTempC ?? null,
              mp: snap.perf?.memPercent,
              nd: Math.round((snap.perf?.downBps ?? 0) / 256),
              nu: Math.round((snap.perf?.upBps ?? 0) / 256),
            });
            if (sig === radioSigRef.current) return;
            radioSigRef.current = sig;
            setRadio(snap);
          }),
        );
      } catch {
        /* noop */
      }
    })();

    return () => {
      cancelled = true;
      unsubs.forEach((fn) => fn());
    };
  }, [onOpenChange]);

  useEffect(() => {
    if (open) return;
    void invoke("is_tray_popup_open")
      .then((visible) => {
        if (visible) return invoke("close_tray_popup");
      })
      .catch(() => undefined);
  }, [open]);

  // Island / chrome clicks often never transfer HWND focus to the flyout (deferred
  // reveal). Close explicitly when pointer lands outside the flyout triggers.
  useEffect(() => {
    if (!flyoutKind) return;
    const onPtr = (e: PointerEvent) => {
      const t = e.target as Element | null;
      if (t?.closest?.(".tray-sys-btn, .tray-clock, .tray-ime-btn")) return;
      setFlyoutKind(null);
      void invoke("close_system_flyout").catch(() => undefined);
    };
    window.addEventListener("pointerdown", onPtr, true);
    return () => window.removeEventListener("pointerdown", onPtr, true);
  }, [flyoutKind]);

  // Same for the chevron tray popup — warm show often leaves main focused.
  useEffect(() => {
    if (!open) return;
    const onPtr = (e: PointerEvent) => {
      const t = e.target as Element | null;
      if (t?.closest?.(".tray-chevron")) return;
      onOpenChange(false);
      void invoke("close_tray_popup").catch(() => undefined);
    };
    window.addEventListener("pointerdown", onPtr, true);
    return () => window.removeEventListener("pointerdown", onPtr, true);
  }, [open, onOpenChange]);

  const pinnedSet = useMemo(() => new Set(pinned), [pinned]);
  const pinnedIcons = useMemo(
    () => icons.filter((i) => pinnedSet.has(i.id)),
    [icons, pinnedSet],
  );
  const railIcons = useMemo(() => {
    const mutePrefs = { muted, muted_processes: mutedProcesses };
    const seen = new Set(pinnedIcons.map((i) => i.id));
    const extra = icons.filter(
      (i) => i.flashing && !seen.has(i.id) && !isTrayNotifyMuted(i, mutePrefs),
    );
    return [...pinnedIcons, ...extra];
  }, [icons, pinnedIcons, muted, mutedProcesses]);

  const openOnDownRef = useRef(false);

  async function togglePopup() {
    if (togglingRef.current) return;
    togglingRef.current = true;
    try {
      // Prefer HWND truth over React `open` — blur-close can desync and make "close" reopen.
      let hwndOpen = open;
      try {
        hwndOpen = await invoke<boolean>("is_tray_popup_open");
      } catch {
        /* use React open */
      }
      if (hwndOpen) {
        onOpenChange(false);
        void invoke("close_tray_popup").catch(() => undefined);
        return;
      }
      const el = chevronRef.current;
      if (!el) return;
      // Suppress before any await — popupAnchor IPC used to outlive the short mousedown suppress.
      void invoke("suppress_tray_popup_blur", { ms: 400 });
      onOpenChange(true);
      const { x, y } = await popupAnchor(el, TRAY_POPUP_W);
      await invoke("open_tray_popup", { x, y });
    } catch (e) {
      onOpenChange(false);
      console.error(e);
    } finally {
      togglingRef.current = false;
    }
  }

  /** Instant close without toggle race — used by mousedown when already open. */
  function closePopupFast() {
    onOpenChange(false);
    void invoke("close_tray_popup").catch(() => undefined);
  }

  async function toggleFlyout(
    kind:
      | "wifi"
      | "bluetooth"
      | "volume"
      | "ime"
      | "power"
      | "calendar"
      | "memory"
      | "network",
    anchor: HTMLElement | null,
  ) {
    if (!anchor || togglingRef.current) return;
    togglingRef.current = true;
    // 先 suppress：点其它芯片时避免失焦先关窗，再热切换 kind
    void invoke("suppress_system_flyout_blur", { ms: 400 });
    try {
      // Closing same kind: optimistic, no round-trip before hide.
      if (flyoutKind === kind) {
        setFlyoutKind(null);
        void invoke("close_system_flyout").catch(() => undefined);
        return;
      }
      const { x, y } = await popupAnchor(anchor, SYSTEM_FLYOUT_W);
      setFlyoutKind(kind);
      await invoke("open_system_flyout", { kind, x, y });
    } catch (e) {
      console.error(e);
      setFlyoutKind(null);
    } finally {
      window.setTimeout(() => {
        togglingRef.current = false;
      }, 80);
    }
  }

  const wifiOn = Boolean(radio?.wifi.radioOn || radio?.wifi.connectedSsid);
  const wifiSignal =
    radio?.wifi.networks?.find((n) => n.connected)?.signal ??
    (wifiOn ? 70 : 0);
  const btOn = Boolean(radio?.bluetooth.radioOn);
  const volMuted = Boolean(radio?.volume?.muted);
  const volLevel = radio?.volume?.level ?? 0;
  const power = radio?.power;
  const powerAc = power?.acLine === "online";
  const imeName = radio?.ime.name || "输入法";
  const imeMark = radio?.ime.mark || "—";
  const imeCaps = Boolean(radio?.ime.caps);
  const peripherals = radio?.peripherals ?? [];
  const perf = radio?.perf;

  return (
    <div className="tray-cluster" ref={rootRef} onClick={(e) => e.stopPropagation()}>
      <div className="tray-rail">
        {systemChips.perf ? (
          <div className="tray-perf">
            <TempMeter
              cpuTempC={perf?.cpuTempC}
              gpuTempC={perf?.gpuTempC}
              cpuPercent={perf?.cpuPercent ?? 0}
            />
            <button
              ref={memRef}
              type="button"
              className={`tray-sys-btn tray-perf-btn${flyoutKind === "memory" ? " is-open" : ""}`}
              title={`内存 ${perf?.memPercent ?? 0}% · 点击查看占用`}
              aria-label="内存占用"
              aria-expanded={flyoutKind === "memory"}
              onMouseDown={(e) => {
                e.preventDefault();
                void invoke("suppress_system_flyout_blur", { ms: 400 });
              }}
              onClick={() => void toggleFlyout("memory", memRef.current)}
            >
              <MemMeter percent={perf?.memPercent ?? 0} />
            </button>
          </div>
        ) : null}

        {systemChips.network ? (
          <button
            ref={netRef}
            type="button"
            className={`tray-sys-btn tray-net-btn${flyoutKind === "network" ? " is-open" : ""}`}
            title={`↑ ${formatRate(perf?.upBps ?? 0)}/s · ↓ ${formatRate(perf?.downBps ?? 0)}/s`}
            aria-label="网速"
            aria-expanded={flyoutKind === "network"}
            onMouseDown={(e) => {
              e.preventDefault();
              void invoke("suppress_system_flyout_blur", { ms: 400 });
            }}
            onClick={() => void toggleFlyout("network", netRef.current)}
          >
            <NetMeter downBps={perf?.downBps ?? 0} upBps={perf?.upBps ?? 0} />
          </button>
        ) : null}

        {systemChips.wifi ? (
          <button
            ref={wifiRef}
            type="button"
            className={`tray-sys-btn${flyoutKind === "wifi" ? " is-open" : ""}${wifiOn ? " is-active" : ""}`}
            title={radio?.wifi.connectedSsid ? `Wi‑Fi：${radio.wifi.connectedSsid}` : "Wi‑Fi"}
            aria-label="Wi‑Fi"
            aria-expanded={flyoutKind === "wifi"}
            onMouseDown={(e) => {
              e.preventDefault();
              void invoke("suppress_system_flyout_blur", { ms: 400 });
            }}
            onClick={() => void toggleFlyout("wifi", wifiRef.current)}
          >
            <WifiGlyph on={wifiOn} signal={wifiSignal} />
          </button>
        ) : null}

        {systemChips.bluetooth ? (
          <button
            ref={btRef}
            type="button"
            className={`tray-sys-btn${flyoutKind === "bluetooth" ? " is-open" : ""}${btOn ? " is-active" : ""}`}
            title="蓝牙"
            aria-label="蓝牙"
            aria-expanded={flyoutKind === "bluetooth"}
            onMouseDown={(e) => {
              e.preventDefault();
              void invoke("suppress_system_flyout_blur", { ms: 400 });
            }}
            onClick={() => void toggleFlyout("bluetooth", btRef.current)}
          >
            <BtGlyph on={btOn} />
          </button>
        ) : null}

        {systemChips.volume ? (
          <button
            ref={volRef}
            type="button"
            className={`tray-sys-btn${flyoutKind === "volume" ? " is-open" : ""}${!volMuted ? " is-active" : ""}`}
            title={volMuted ? "声音：已静音" : `声音：${volLevel}%`}
            aria-label="声音"
            aria-expanded={flyoutKind === "volume"}
            onMouseDown={(e) => {
              e.preventDefault();
              void invoke("suppress_system_flyout_blur", { ms: 350 });
            }}
            onClick={() => void toggleFlyout("volume", volRef.current)}
          >
            <VolumeGlyph muted={volMuted} level={volLevel} />
          </button>
        ) : null}

        {systemChips.power ? (
          <button
            ref={powerRef}
            type="button"
            className={`tray-sys-btn${flyoutKind === "power" ? " is-open" : ""}${powerAc || power?.charging ? " is-active" : ""}`}
            title={
              power?.hasBattery
                ? `电源：${power.percent ?? "—"}%${power.charging ? " · 充电中" : ""}`
                : "电源"
            }
            aria-label="电源"
            aria-expanded={flyoutKind === "power"}
            onMouseDown={(e) => {
              e.preventDefault();
              void invoke("suppress_system_flyout_blur", { ms: 350 });
            }}
            onClick={() => void toggleFlyout("power", powerRef.current)}
          >
            <PowerGlyph
              percent={power?.percent}
              charging={Boolean(power?.charging)}
              ac={powerAc}
            />
          </button>
        ) : null}

        {systemChips.peripherals
          ? peripherals.map((p) => (
              <button
                key={p.id}
                type="button"
                className="tray-sys-btn is-active"
                title={p.name}
                aria-label={p.name}
                onClick={() =>
                  void toggleFlyout(
                    p.kind === "headphones" ? "volume" : "bluetooth",
                    p.kind === "headphones" ? volRef.current : btRef.current,
                  )
                }
              >
                {p.kind === "headphones" ? <HeadphoneGlyph /> : <GamepadGlyph />}
              </button>
            ))
          : null}

        {systemChips.ime ? (
          <button
            ref={imeRef}
            type="button"
            className={`tray-sys-btn tray-ime-btn${flyoutKind === "ime" ? " is-open" : ""}${imeCaps ? " is-caps" : ""}`}
            title={`${imeName} · ${
              radio?.ime.mode === "zh"
                ? "中文"
                : imeCaps
                  ? "英文 · 大写"
                  : "英文 · 小写"
            }`}
            aria-label={`输入法：${imeName}`}
            aria-expanded={flyoutKind === "ime"}
            onMouseDown={(e) => {
              e.preventDefault();
              void invoke("suppress_system_flyout_blur", { ms: 350 });
            }}
            onClick={() => void toggleFlyout("ime", imeRef.current)}
          >
            <span className="tray-ime-mark">{imeMark}</span>
            {imeCaps ? <span className="tray-ime-caps">A</span> : null}
          </button>
        ) : null}

        {railIcons.map((icon) => (
          <button
            key={icon.id}
            type="button"
            className={`tray-icon-btn${icon.flashing ? " is-flashing" : ""}`}
            title={trayLabel(icon)}
            onClick={() => void clickTray(icon, "left")}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              void clickTray(icon, "right");
            }}
          >
            <TrayGlyph icon={icon} />
          </button>
        ))}

        {systemChips.clock ? (
          <TrayClockButton
            open={flyoutKind === "calendar"}
            onToggle={(el) => void toggleFlyout("calendar", el)}
          />
        ) : null}

        <button
          ref={chevronRef}
          type="button"
          className={`tray-chevron${open ? " is-open" : ""}`}
          aria-label={open ? "收起托盘" : "展开托盘"}
          aria-expanded={open}
          onMouseDown={(e) => {
            e.preventDefault();
            // Always swallow the following click — setting false on close made click re-open.
            openOnDownRef.current = true;
            if (open) {
              void invoke("suppress_tray_popup_blur", { ms: 400 });
              closePopupFast();
              return;
            }
            void invoke("suppress_tray_popup_blur", { ms: 400 });
            void refreshWindowCache();
            if (!togglingRef.current) {
              void togglePopup();
            }
          }}
          onClick={() => {
            // Swallow click after mousedown; keyboard activation has no prior mousedown flag.
            if (openOnDownRef.current) {
              openOnDownRef.current = false;
              return;
            }
            void togglePopup();
          }}
        >
          <svg width="9" height="9" viewBox="0 0 12 12" aria-hidden>
            <path
              d="M2.2 7.8 L6 4 L9.8 7.8"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.7"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </div>
    </div>
  );
}

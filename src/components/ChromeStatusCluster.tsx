/**
 * Host chrome: Wi‑Fi + clock only.
 * Independent of tray hook / TrayCluster — stays visible when tray boot is disabled.
 */
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { hideChromeHoverTip, hostTipPointerProps, installChromeHoverTipGlobalDismiss } from "../chromeHoverTip";
import { clickTrace } from "../clickTrace";
import type { WifiState } from "./TrayCluster";

const FALLBACK_WIFI: WifiState = {
  enabled: true,
  connected: false,
  ssid: "",
  signal: 0,
  ip: "",
  linkMbps: 0,
  mac: "",
  secured: false,
  ethernetConnected: false,
  ethernetName: "",
  ethernetIp: "",
  ethernetLinkMbps: 0,
  ethernetMac: "",
};

const POPUP_GAP = 8;
const WIFI_POPUP_W = 280;

function pad2(n: number) {
  return n.toString().padStart(2, "0");
}

/** Same as TrayCluster menu-bar clock. */
function formatMenuClock(d: Date) {
  const week = ["日", "一", "二", "三", "四", "五", "六"][d.getDay()];
  return `${d.getMonth() + 1}月${d.getDate()}日 周${week} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

function EthernetGlyph() {
  return (
    <svg className="tray-wifi-glyph" width="15" height="15" viewBox="0 0 24 24" aria-hidden>
      <rect
        x="3"
        y="3"
        width="14"
        height="11"
        rx="1.8"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
      />
      <path
        d="M7 20h6M10 14v6"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
      <path
        d="M17.5 8.5h2.2a1.3 1.3 0 0 1 1.3 1.3v3.4a1.3 1.3 0 0 1-1.3 1.3H17.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M19.2 10.2v2.6"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}

function WifiGlyph({ state }: { state: WifiState }) {
  if (state.ethernetConnected) {
    return <EthernetGlyph />;
  }
  const level = !state.enabled
    ? 0
    : !state.connected
      ? 1
      : state.signal >= 55
        ? 3
        : state.signal >= 25
          ? 2
          : 1;
  const tip = level >= 1 ? 1 : 0.22;
  const mid = level >= 2 ? 1 : 0.22;
  const outer = level >= 3 ? 1 : 0.22;
  return (
    <svg className="tray-wifi-glyph" width="15" height="15" viewBox="1 5 22 18" aria-hidden>
      <circle cx="12" cy="19.2" r="2.3" fill="currentColor" opacity={tip} />
      <path
        d="M7.2 13.8a6.9 6.9 0 0 1 9.6 0"
        fill="none"
        stroke="currentColor"
        strokeWidth="3.15"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity={mid}
      />
      <path
        d="M3.6 9a12 12 0 0 1 16.8 0"
        fill="none"
        stroke="currentColor"
        strokeWidth="3.15"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity={outer}
      />
      {!state.enabled ? (
        <path
          d="M5 5.5 L19 20"
          fill="none"
          stroke="currentColor"
          strokeWidth="2.7"
          strokeLinecap="round"
        />
      ) : null}
    </svg>
  );
}

async function popupAnchor(el: HTMLElement, width: number) {
  const win = getCurrentWindow();
  const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  const rect = el.getBoundingClientRect();
  const logicalX = outer.x / factor;
  const logicalY = outer.y / factor;
  const x = logicalX + rect.right - width;
  const y = logicalY + rect.bottom + POPUP_GAP;
  return { x, y };
}

export default function ChromeStatusCluster() {
  const [now, setNow] = useState(() => new Date());
  const [wifi, setWifi] = useState<WifiState>(FALLBACK_WIFI);
  const [wifiMenuOpen, setWifiMenuOpen] = useState(false);
  const wifiChipRef = useRef<HTMLButtonElement>(null);

  useEffect(() => installChromeHoverTipGlobalDismiss(), []);

  useEffect(() => {
    const t = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(t);
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    void (async () => {
      try {
        const w = await invoke<WifiState>("get_wifi_state");
        if (!cancelled && w) setWifi(w);
      } catch {
        /* keep fallback */
      }
      try {
        unsubs.push(
          await listen<WifiState>("wifi-state", (ev) => {
            if (ev.payload) setWifi(ev.payload);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("wifi-popup-opened", () => {
            setWifiMenuOpen(true);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("wifi-popup-closed", () => {
            setWifiMenuOpen(false);
            void invoke<WifiState>("get_wifi_state")
              .then((w) => {
                if (w) setWifi(w);
              })
              .catch(() => undefined);
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
  }, []);

  async function openWifiMenu() {
    clickTrace("fe-chrome", "openWifiMenu");
    try {
      const visible = await invoke<boolean>("is_wifi_popup_open");
      if (visible || wifiMenuOpen) {
        await invoke("close_wifi_popup");
        return;
      }
      const el = wifiChipRef.current;
      if (!el) return;
      const { x, y } = await popupAnchor(el, WIFI_POPUP_W);
      await invoke("open_wifi_popup", { x, y });
    } catch (e) {
      clickTrace("fe-chrome", `openWifiMenu error ${String(e)}`);
    }
  }

  const wifiTip = wifi.ethernetConnected
    ? `${wifi.ethernetName || "以太网"}${
        wifi.ethernetIp ? ` · ${wifi.ethernetIp}` : ""
      }${wifi.ethernetLinkMbps ? ` · ${wifi.ethernetLinkMbps} Mbps` : ""}\n单击打开网络菜单`
    : !wifi.enabled
      ? "无线局域网已关闭\n单击打开 WLAN 菜单"
      : wifi.connected && wifi.ssid
        ? `${wifi.ssid}${wifi.signal ? ` · ${wifi.signal}%` : ""}\n单击打开 WLAN 菜单`
        : "未连接\n单击打开 WLAN 菜单";

  const wifiOn = Boolean(wifi.ethernetConnected || (wifi.enabled && wifi.connected));
  const wifiAria = wifi.ethernetConnected
    ? `有线网络 ${wifi.ethernetName || "已连接"}`
    : wifi.enabled
      ? wifi.connected
        ? `Wi‑Fi ${wifi.ssid || "已连接"}`
        : "Wi‑Fi 未连接"
      : "无线局域网已关闭";

  return (
    <div className="tray-cluster chrome-status-cluster" onClick={(e) => e.stopPropagation()}>
      <div className="tray-rail">
        <button
          ref={wifiChipRef}
          type="button"
          className={`tray-wifi-btn${wifiMenuOpen ? " is-open" : ""}${
            wifiOn ? " is-on" : ""
          }${!wifi.enabled && !wifi.ethernetConnected ? " is-off" : ""}`}
          {...hostTipPointerProps(wifiTip)}
          aria-label={wifiAria}
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => {
            clickTrace("fe-chrome", "wifi click");
            void hideChromeHoverTip();
            void openWifiMenu();
          }}
        >
          <WifiGlyph state={wifi} />
        </button>

        <button
          type="button"
          className="tray-clock"
          {...hostTipPointerProps("打开通知中心")}
          onClick={() => {
            clickTrace("fe-chrome", "clock click");
            void hideChromeHoverTip();
            void invoke("open_notification_center").catch((e) => console.error(e));
          }}
        >
          <time dateTime={now.toISOString()}>{formatMenuClock(now)}</time>
        </button>
      </div>
    </div>
  );
}

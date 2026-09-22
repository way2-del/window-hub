import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import { schedulePopupFit } from "./popupFit";
import type { WifiNetwork, WifiState } from "./components/TrayCluster";

const POPUP_W = 280;

const FALLBACK: WifiState = {
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

async function closeSelf() {
  try {
    await invoke("close_wifi_popup");
  } catch {
    try {
      await getCurrentWindow().hide();
    } catch {
      /* noop */
    }
  }
}

function fitWifiPopup() {
  schedulePopupFit({
    width: POPUP_W,
    selector: ".wifi-popup-shell",
    minHeight: 120,
    maxHeight: 720,
  });
}

function signalLevel(signal: number): 0 | 1 | 2 | 3 {
  if (signal >= 55) return 3;
  if (signal >= 25) return 2;
  if (signal >= 1) return 1;
  return 0;
}

/** Arc-style Wi‑Fi glyph — thicker 2-arc + tip; `accent` for connected. */
function WifiIcon({
  signal,
  accent,
}: {
  signal: number;
  accent?: boolean;
}) {
  const level = signalLevel(signal);
  const color = accent ? "var(--wifi-accent, var(--sys-accent, #34c759))" : "currentColor";
  const tip = level >= 1 || accent ? 1 : 0.22;
  const mid = level >= 2 ? 1 : 0.22;
  const outer = level >= 3 ? 1 : 0.22;
  return (
    <svg className="wifi-ico" width="18" height="18" viewBox="1 5 22 18" aria-hidden>
      <circle cx="12" cy="19.2" r="2.3" fill={color} opacity={tip} />
      <path
        d="M7.2 13.8a6.9 6.9 0 0 1 9.6 0"
        fill="none"
        stroke={color}
        strokeWidth="3.15"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity={mid}
      />
      <path
        d="M3.6 9a12 12 0 0 1 16.8 0"
        fill="none"
        stroke={color}
        strokeWidth="3.15"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity={outer}
      />
    </svg>
  );
}

function EthernetIcon({ accent }: { accent?: boolean }) {
  const color = accent ? "var(--wifi-accent, var(--sys-accent, #34c759))" : "currentColor";
  return (
    <svg className="wifi-ico wifi-eth-ico" width="18" height="18" viewBox="0 0 24 24" aria-hidden>
      <rect
        x="3"
        y="3"
        width="14"
        height="11"
        rx="1.8"
        fill="none"
        stroke={color}
        strokeWidth="2"
      />
      <path
        d="M7 20h6M10 14v6"
        fill="none"
        stroke={color}
        strokeWidth="2"
        strokeLinecap="round"
      />
      <path
        d="M17.5 8.5h2.2a1.3 1.3 0 0 1 1.3 1.3v3.4a1.3 1.3 0 0 1-1.3 1.3H17.5"
        fill="none"
        stroke={color}
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M19.2 10.2v2.6"
        fill="none"
        stroke={color}
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </svg>
  );
}

function LockIcon() {
  return (
    <svg className="wifi-lock-ico" width="11" height="12" viewBox="0 0 12 14" aria-hidden>
      <rect x="2" y="6" width="8" height="6.5" rx="1.4" fill="none" stroke="currentColor" strokeWidth="1.4" />
      <path
        d="M3.6 6 V4.4a2.4 2.4 0 0 1 4.8 0V6"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </svg>
  );
}

function RefreshIcon({ spinning }: { spinning?: boolean }) {
  return (
    <svg
      className={`wifi-refresh-ico${spinning ? " is-spin" : ""}`}
      width="14"
      height="14"
      viewBox="0 0 24 24"
      aria-hidden
    >
      <path
        d="M20 12a8 8 0 1 1-2.3-5.6"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
      />
      <path d="M20 4v5h-5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export default function WifiPopupApp() {
  const [state, setState] = useState<WifiState>(FALLBACK);
  const [networks, setNetworks] = useState<WifiNetwork[]>([]);
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fitWifiPopup();
  }, [state, networks, busy, scanning, error]);

  async function loadNetworks() {
    setScanning(true);
    try {
      const list = await invoke<WifiNetwork[]>("list_wifi_networks");
      setNetworks(list ?? []);
      setError(null);
    } catch (e) {
      console.error(e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setScanning(false);
      fitWifiPopup();
    }
  }

  useEffect(() => {
    void (async () => {
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
      await invoke("apply_window_effect", {}).catch(() => undefined);
      try {
        const s = await invoke<WifiState>("get_wifi_state");
        if (s) setState(s);
      } catch (e) {
        console.error(e);
        setError(e instanceof Error ? e.message : String(e));
      }
      await loadNetworks();
      fitWifiPopup();
    })();

    const retryA = window.setTimeout(() => {
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }, 120);
    const retryB = window.setTimeout(() => {
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }, 350);

    let cancelled = false;
    const unsubs: Array<() => void> = [];

    void listen<GlassPrefs>("material-prefs", (ev) => {
      void syncGlassCss({
        kind: "mica-alt",
        dark: ev.payload.dark ?? null,
        acrylicAlpha: ev.payload.acrylicAlpha,
      });
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    unsubs.push(
      subscribeSystemDark(() => {
        void (async () => {
          try {
            const prefs = await invoke<GlassPrefs>("get_material_prefs");
            await syncGlassCss({
              kind: "mica-alt",
              dark: prefs.dark ?? null,
              acrylicAlpha: prefs.acrylicAlpha,
            });
          } catch {
            /* noop */
          }
        })();
      }),
    );

    void listen<WifiState>("wifi-state", (ev) => {
      if (ev.payload) {
        setState(ev.payload);
        fitWifiPopup();
      }
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    // HWND reuse: refresh lists when the popup is shown again.
    void listen("wifi-popup-opened", () => {
      void (async () => {
        try {
          const s = await invoke<WifiState>("get_wifi_state");
          if (s) setState(s);
        } catch {
          /* noop */
        }
        await loadNetworks();
        fitWifiPopup();
      })();
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    const onBlur = () => {
      window.setTimeout(() => {
        void getCurrentWindow()
          .isFocused()
          .then((f) => {
            if (!f) void closeSelf();
          })
          .catch(() => undefined);
      }, 80);
    };
    window.addEventListener("blur", onBlur);

    return () => {
      cancelled = true;
      window.clearTimeout(retryA);
      window.clearTimeout(retryB);
      unsubs.forEach((fn) => fn());
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  async function toggleEnabled() {
    if (busy) return;
    setBusy(true);
    try {
      const next = await invoke<WifiState>("set_wifi_enabled", { enabled: !state.enabled });
      if (next) setState(next);
      if (next?.enabled) await loadNetworks();
      else setNetworks([]);
    } catch (e) {
      console.error(e);
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
      fitWifiPopup();
    }
  }

  async function onNetworkClick(net: WifiNetwork) {
    if (busy) return;
    if (net.secured && !net.hasProfile) {
      try {
        await invoke("open_wifi_auth_popup", { ssid: net.ssid });
      } catch (e) {
        console.error(e);
        setError(e instanceof Error ? e.message : String(e));
      }
      return;
    }
    setBusy(true);
    try {
      if (net.connected) {
        const next = await invoke<WifiState>("disconnect_wifi");
        if (next) setState(next);
      } else {
        const next = await invoke<WifiState>("connect_wifi", { ssid: net.ssid });
        if (next) setState(next);
      }
      await loadNetworks();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (msg.includes("NEED_PASSWORD")) {
        try {
          await invoke("open_wifi_auth_popup", { ssid: net.ssid });
          return;
        } catch (err) {
          console.error(err);
        }
      }
      console.error(e);
      setError(msg);
    } finally {
      setBusy(false);
      fitWifiPopup();
    }
  }

  async function disconnectPreferred() {
    if (busy) return;
    setBusy(true);
    try {
      const next = await invoke<WifiState>("disconnect_wifi");
      if (next) setState(next);
      await loadNetworks();
    } catch (e) {
      console.error(e);
    } finally {
      setBusy(false);
      fitWifiPopup();
    }
  }

  async function copyText(text: string) {
    const value = text.trim();
    if (!value || value === "—") return;
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      try {
        const ta = document.createElement("textarea");
        ta.value = value;
        ta.style.position = "fixed";
        ta.style.left = "-9999px";
        document.body.appendChild(ta);
        ta.select();
        document.execCommand("copy");
        document.body.removeChild(ta);
      } catch {
        /* noop */
      }
    }
  }

  const others = networks.filter((n) => !n.connected && n.ssid !== state.ssid);
  const preferredNet = networks.find((n) => n.connected || n.ssid === state.ssid);
  const preferred =
    state.connected && state.ssid
      ? {
          ...state,
          secured: state.secured || !!preferredNet?.secured,
          signal: state.signal || preferredNet?.signal || 99,
        }
      : null;
  const ethConnected = Boolean(state.ethernetConnected);
  const ethName = (state.ethernetName || "以太网").trim() || "以太网";
  const ethIp = (state.ethernetIp || "").trim();
  const ethMbps = state.ethernetLinkMbps || 0;
  const ethMac = (state.ethernetMac || "").trim();

  return (
    <div className="wifi-popup-shell" role="dialog" aria-label="网络">
      {ethConnected ? (
        <>
          <div className="wifi-head">
            <span className="wifi-title">有线网络</span>
            <span className="wifi-head-status is-on">已连接</span>
          </div>

          <button
            type="button"
            className="wifi-link-row"
            onClick={() => {
              void invoke("open_network_settings").catch(console.error);
              void closeSelf();
            }}
          >
            网络偏好设置
          </button>

          <div className="wifi-sep" />

          <div className="wifi-preferred wifi-ethernet">
            <div className="wifi-preferred-main is-static">
              <EthernetIcon accent />
              <span className="wifi-ssid">{ethName}</span>
            </div>
            <div className="wifi-meta">
              <div className="wifi-meta-row">
                <span
                  className="wifi-copyable"
                  title={ethIp ? "单击复制 IP" : undefined}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (ethIp) void copyText(ethIp);
                  }}
                  onMouseDown={(e) => e.stopPropagation()}
                >
                  {ethIp || "—"}
                </span>
                <span className="wifi-meta-speed">
                  {ethMbps > 0 ? `${ethMbps} Mbps` : ""}
                </span>
              </div>
              {ethMac ? (
                <div
                  className="wifi-meta-mac wifi-copyable"
                  title="单击复制 MAC"
                  onClick={(e) => {
                    e.stopPropagation();
                    void copyText(ethMac);
                  }}
                  onMouseDown={(e) => e.stopPropagation()}
                >
                  {ethMac}
                </div>
              ) : null}
            </div>
          </div>

          <div className="wifi-sep" />
        </>
      ) : null}

      <div className="wifi-head">
        <span className="wifi-title">WLAN</span>
        <button
          type="button"
          className={`wifi-switch${state.enabled ? " is-on" : ""}`}
          role="switch"
          aria-checked={state.enabled}
          disabled={busy}
          onClick={() => void toggleEnabled()}
        >
          <span className="wifi-switch-knob" />
        </button>
      </div>

      {!ethConnected ? (
        <button
          type="button"
          className="wifi-link-row"
          onClick={() => {
            void invoke("open_network_settings").catch(console.error);
            void closeSelf();
          }}
        >
          网络偏好设置
        </button>
      ) : null}

      {!ethConnected ? <div className="wifi-sep" /> : null}

      {error ? <div className="wifi-error">{error}</div> : null}

      {state.enabled ? (
        <>
          <div className="wifi-section-label">首选网络</div>
          {preferred ? (
            <div className="wifi-preferred">
              <button
                type="button"
                className="wifi-preferred-main"
                disabled={busy}
                title="单击断开连接"
                onClick={() => void disconnectPreferred()}
              >
                <WifiIcon signal={preferred.signal || 99} accent />
                <span className="wifi-ssid">{preferred.ssid}</span>
                {preferred.secured ? (
                  <span className="wifi-row-lock" aria-label="加密">
                    <LockIcon />
                  </span>
                ) : (
                  <span className="wifi-row-lock" />
                )}
              </button>
              <div className="wifi-meta">
                <div className="wifi-meta-row">
                  <span
                    className="wifi-copyable"
                    title={preferred.ip ? "单击复制 IP" : undefined}
                    onClick={(e) => {
                      e.stopPropagation();
                      if (preferred.ip) void copyText(preferred.ip);
                    }}
                    onMouseDown={(e) => e.stopPropagation()}
                  >
                    {preferred.ip || "—"}
                  </span>
                  <span className="wifi-meta-speed">
                    {preferred.linkMbps > 0 ? `${preferred.linkMbps} Mbps` : ""}
                  </span>
                </div>
                {preferred.mac ? (
                  <div
                    className="wifi-meta-mac wifi-copyable"
                    title="单击复制 MAC"
                    onClick={(e) => {
                      e.stopPropagation();
                      void copyText(preferred.mac);
                    }}
                    onMouseDown={(e) => e.stopPropagation()}
                  >
                    {preferred.mac}
                  </div>
                ) : null}
              </div>
            </div>
          ) : (
            <div className="wifi-empty">
              {ethConnected ? "WLAN 未连接（当前使用有线）" : "未连接"}
            </div>
          )}

          <div className="wifi-sep" />

          <div className="wifi-section-head">
            <span className="wifi-section-label">其他网络</span>
            <button
              type="button"
              className="wifi-refresh"
              disabled={scanning || busy}
              onClick={() => void loadNetworks()}
              title="刷新"
              aria-label="刷新网络列表"
            >
              <RefreshIcon spinning={scanning} />
            </button>
          </div>

          <div className="wifi-list">
            {others.length === 0 ? (
              <div className="wifi-empty">{scanning ? "正在扫描…" : "暂无其他网络"}</div>
            ) : (
              others.map((net) => (
                <button
                  key={net.ssid}
                  type="button"
                  className="wifi-row"
                  disabled={busy}
                  onClick={() => void onNetworkClick(net)}
                  title={
                    net.hasProfile
                      ? "单击连接"
                      : net.secured
                        ? "单击输入密码加入"
                        : "单击连接"
                  }
                >
                  <WifiIcon signal={net.signal} />
                  <span className="wifi-row-name">{net.ssid}</span>
                  {net.secured ? (
                    <span className="wifi-row-lock" aria-label="加密">
                      <LockIcon />
                    </span>
                  ) : (
                    <span className="wifi-row-lock" />
                  )}
                </button>
              ))
            )}
          </div>
        </>
      ) : (
        <div className="wifi-empty">
          {ethConnected ? "Wi‑Fi 已关闭（当前使用有线）" : "Wi‑Fi 已关闭"}
        </div>
      )}
    </div>
  );
}

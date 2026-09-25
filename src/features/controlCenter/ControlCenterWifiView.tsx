import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import type { WifiNetwork, WifiState } from "../../components/TrayCluster";
import Icon from "./ControlCenterIcon";

const native = isTauri();

const FALLBACK: WifiState = {
  enabled: true,
  connected: false,
  ssid: "",
  signal: 0,
  ip: "",
  linkMbps: 0,
  mac: "",
  secured: false,
};

function signalLevel(signal: number): 0 | 1 | 2 | 3 {
  if (signal >= 55) return 3;
  if (signal >= 25) return 2;
  if (signal >= 1) return 1;
  return 0;
}

function WifiGlyph({ signal, accent }: { signal: number; accent?: boolean }) {
  const level = signalLevel(signal);
  const color = accent ? "var(--sys-accent, #34c759)" : "currentColor";
  return (
    <svg className="cc-wifi-glyph" viewBox="1 5 22 18" aria-hidden="true">
      <circle cx="12" cy="19.2" r="2.3" fill={color} opacity={level >= 1 || accent ? 1 : 0.22} />
      <path d="M7.2 13.8a6.9 6.9 0 0 1 9.6 0" fill="none" stroke={color} strokeWidth="3.15" strokeLinecap="round" opacity={level >= 2 ? 1 : 0.22} />
      <path d="M3.6 9a12 12 0 0 1 16.8 0" fill="none" stroke={color} strokeWidth="3.15" strokeLinecap="round" opacity={level >= 3 ? 1 : 0.22} />
    </svg>
  );
}

export default function ControlCenterWifiView({
  onBack,
  onError,
}: {
  onBack: () => void;
  onError: (message: string) => void;
}) {
  const [state, setState] = useState<WifiState>(FALLBACK);
  const [networks, setNetworks] = useState<WifiNetwork[]>([]);
  const [busy, setBusy] = useState(false);
  const [scanning, setScanning] = useState(false);
  const active = useRef(true);

  async function refreshState() {
    if (!native || !active.current) return;
    try {
      const next = await invoke<WifiState>("get_wifi_state");
      if (active.current) setState(next);
    } catch (e) {
      onError(String(e));
    }
  }

  async function loadNetworks() {
    if (!native || !active.current) return;
    setScanning(true);
    try {
      const list = await invoke<WifiNetwork[]>("list_wifi_networks");
      if (active.current) setNetworks(list);
    } catch (e) {
      onError(String(e));
    } finally {
      setScanning(false);
    }
  }

  useEffect(() => {
    active.current = true;
    void refreshState();
    void loadNetworks();
    const timer = window.setInterval(() => {
      if (!busy) {
        void refreshState();
        void loadNetworks();
      }
    }, 4000);
    return () => {
      active.current = false;
      clearInterval(timer);
    };
  }, []);

  async function toggleEnabled() {
    if (!native || busy) return;
    setBusy(true);
    onError("");
    try {
      const next = await invoke<WifiState>("set_wifi_enabled", { enabled: !state.enabled });
      setState(next);
      if (next.enabled) await loadNetworks();
      else setNetworks([]);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onNetworkClick(net: WifiNetwork) {
    if (!native || busy) return;
    setBusy(true);
    onError("");
    try {
      if (net.connected) {
        setState(await invoke<WifiState>("disconnect_wifi"));
      } else if (net.hasProfile) {
        setState(await invoke<WifiState>("connect_wifi", { ssid: net.ssid }));
      } else if (net.secured) {
        await invoke("open_wifi_auth_popup", { ssid: net.ssid });
      } else {
        setState(await invoke<WifiState>("connect_wifi", { ssid: net.ssid }));
      }
      await loadNetworks();
    } catch (e) {
      const msg = String(e);
      if (net.secured && !net.hasProfile) {
        try {
          await invoke("open_wifi_auth_popup", { ssid: net.ssid });
        } catch {
          onError(msg);
        }
      } else {
        onError(msg);
      }
    } finally {
      setBusy(false);
    }
  }

  const preferred = state.connected && state.ssid
    ? networks.find((n) => n.ssid === state.ssid) ?? {
        ssid: state.ssid,
        signal: state.signal || 99,
        secured: state.secured,
        connected: true,
        hasProfile: true,
      }
    : null;
  const others = networks.filter((n) => !preferred || n.ssid !== preferred.ssid);

  return (
    <div className="cc-subview">
      <header className="cc-sound-header">
        <button type="button" className="cc-back" aria-label="返回" title="返回" onClick={onBack}>
          <Icon name="back" />
        </button>
        <strong>Wi-Fi</strong>
        <button
          type="button"
          className={`cc-switch${state.enabled ? " is-on" : ""}`}
          role="switch"
          aria-checked={state.enabled}
          disabled={busy}
          title={state.enabled ? "关闭 Wi-Fi" : "打开 Wi-Fi"}
          onClick={() => void toggleEnabled()}
        >
          <span className="cc-switch-knob" />
        </button>
      </header>

      <section className="cc-card cc-sub-list" aria-label="网络列表">
        {!state.enabled ? (
          <p className="cc-empty">Wi-Fi 已关闭</p>
        ) : (
          <>
            <h2 className="cc-section-title">当前网络</h2>
            {preferred ? (
              <button
                type="button"
                className="cc-net-row"
                data-active="true"
                disabled={busy}
                title="单击断开"
                onClick={() => void onNetworkClick({ ...preferred, connected: true, hasProfile: true })}
              >
                <span className="cc-net-icon"><WifiGlyph signal={preferred.signal || 99} accent /></span>
                <span className="cc-net-text">
                  <strong>{preferred.ssid}</strong>
                  <small>{state.ip || "已连接"}</small>
                </span>
              </button>
            ) : (
              <p className="cc-empty">未连接</p>
            )}

            <div className="cc-sub-head">
              <h2 className="cc-section-title">其他网络</h2>
              <button
                type="button"
                className="cc-refresh"
                disabled={scanning || busy}
                title="刷新"
                aria-label="刷新网络列表"
                onClick={() => void loadNetworks()}
              >
                <Icon name="refresh" />
              </button>
            </div>
            {others.length === 0 ? (
              <p className="cc-empty">{scanning ? "正在扫描…" : "暂无其他网络"}</p>
            ) : (
              others.map((net) => (
                <button
                  key={net.ssid}
                  type="button"
                  className="cc-net-row"
                  disabled={busy}
                  onClick={() => void onNetworkClick(net)}
                >
                  <span className="cc-net-icon"><WifiGlyph signal={net.signal} /></span>
                  <span className="cc-net-text">
                    <strong>{net.ssid}</strong>
                    <small>{net.hasProfile ? "已保存" : net.secured ? "需要密码" : "开放网络"}</small>
                  </span>
                </button>
              ))
            )}
          </>
        )}
      </section>

      <button
        type="button"
        className="cc-more-link"
        onClick={() => void invoke("open_network_settings").catch(() => undefined)}
      >
        网络偏好设置
      </button>
    </div>
  );
}

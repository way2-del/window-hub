import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import Icon from "./ControlCenterIcon";

const native = isTauri();

type BluetoothDeviceInfo = { id: string; name: string; connected: boolean };
type BluetoothState = { enabled: boolean; available: boolean; devices: BluetoothDeviceInfo[] };

export default function ControlCenterBluetoothView({
  onBack,
  onError,
}: {
  onBack: () => void;
  onError: (message: string) => void;
}) {
  const [state, setState] = useState<BluetoothState>(
    native
      ? { enabled: false, available: true, devices: [] }
      : {
          enabled: true,
          available: true,
          devices: [
            { id: "1", name: "示例耳机", connected: true },
            { id: "2", name: "示例鼠标", connected: false },
          ],
        },
  );
  const [busy, setBusy] = useState(false);
  const active = useRef(true);

  async function refresh() {
    if (!native || !active.current) return;
    try {
      const next = await invoke<BluetoothState>("control_center_bluetooth");
      if (active.current) setState(next);
    } catch (e) {
      onError(String(e));
    }
  }

  useEffect(() => {
    active.current = true;
    void refresh();
    const timer = window.setInterval(() => {
      if (!busy) void refresh();
    }, 3000);
    return () => {
      active.current = false;
      clearInterval(timer);
    };
  }, []);

  async function toggleEnabled() {
    if (!native || busy || !state.available) return;
    setBusy(true);
    onError("");
    try {
      const next = await invoke<BluetoothState>("control_center_set_bluetooth", {
        enabled: !state.enabled,
      });
      setState(next);
    } catch (e) {
      onError(String(e));
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="cc-subview">
      <header className="cc-sound-header">
        <button type="button" className="cc-back" aria-label="返回" title="返回" onClick={onBack}>
          <Icon name="back" />
        </button>
        <strong>蓝牙</strong>
        <button
          type="button"
          className={`cc-switch${state.enabled ? " is-on" : ""}`}
          role="switch"
          aria-checked={state.enabled}
          disabled={busy || !state.available}
          title={state.enabled ? "关闭蓝牙" : "打开蓝牙"}
          onClick={() => void toggleEnabled()}
        >
          <span className="cc-switch-knob" />
        </button>
      </header>

      <section className="cc-card cc-sub-list" aria-label="蓝牙设备">
        {!state.available ? (
          <p className="cc-empty">未找到蓝牙适配器</p>
        ) : !state.enabled ? (
          <p className="cc-empty">蓝牙已关闭</p>
        ) : state.devices.length === 0 ? (
          <p className="cc-empty">暂无已配对设备</p>
        ) : (
          <>
            <h2 className="cc-section-title">我的设备</h2>
            {state.devices.map((device) => (
              <div key={device.id} className="cc-net-row cc-net-row-static" data-active={device.connected}>
                <span className="cc-net-icon">
                  <Icon name="bluetooth" />
                </span>
                <span className="cc-net-text">
                  <strong>{device.name}</strong>
                  <small>{device.connected ? "已连接" : "已配对"}</small>
                </span>
              </div>
            ))}
          </>
        )}
      </section>

      <button
        type="button"
        className="cc-more-link"
        onClick={() =>
          void invoke("control_center_action", { action: "bluetooth" }).catch(() => undefined)
        }
      >
        更多蓝牙设置
      </button>
    </div>
  );
}

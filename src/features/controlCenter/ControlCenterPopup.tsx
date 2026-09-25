import { useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { syncGlassCss, subscribeSystemDark, type GlassPrefs } from "../../glassPrefs";
import { schedulePopupFit } from "../../popupFit";
import ChromePopupShell, {
  CHROME_POPUP_SHELL_SELECTOR,
} from "../chromePopup/ChromePopupShell";
import Icon from "./ControlCenterIcon";
import ControlCenterWifiView from "./ControlCenterWifiView";
import ControlCenterBluetoothView from "./ControlCenterBluetoothView";
import "./controlCenter.css";

type AudioState = { volume: number | null; muted: boolean; micMuted: boolean | null };
type WifiState = { enabled: boolean; connected: boolean; ssid: string };
type OutputDevice = { id: string; name: string; isDefault: boolean };
type MixerApp = {
  appKey: string;
  processId: number;
  name: string;
  volume: number;
  muted: boolean;
  deviceId: string;
  isSystem: boolean;
  iconPngBase64?: string | null;
};
type SoundMixerState = { devices: OutputDevice[]; apps: MixerApp[]; defaultDeviceId: string };
type View = "home" | "sound" | "wifi" | "bluetooth";

const native = isTauri();
const CC_W = 374;
const fit = () => {
  if (native) {
    schedulePopupFit({ width: CC_W, selector: CHROME_POPUP_SHELL_SELECTOR, minHeight: 120 }, [
      0, 50, 160,
    ]);
  }
};

function Slider({
  label,
  value,
  disabled,
  onCommit,
}: {
  label: string;
  value: number | null;
  disabled?: boolean;
  onCommit: (value: number) => Promise<void>;
}) {
  const [draft, setDraft] = useState(value ?? 0);
  const [saving, setSaving] = useState(false);
  const editing = useRef(false);
  useEffect(() => {
    if (!editing.current && !saving) setDraft(value ?? 0);
  }, [value, saving]);
  async function commit() {
    if (!editing.current) return;
    editing.current = false;
    setSaving(true);
    try {
      await onCommit(draft);
    } finally {
      setSaving(false);
    }
  }
  return (
    <div
      className="cc-slider"
      data-disabled={value == null || disabled || saving}
      style={{ "--cc-level": `${draft}%` } as CSSProperties}
    >
      <span aria-hidden="true">{value == null ? "不可用" : draft}</span>
      <input
        type="range"
        min="0"
        max="100"
        value={draft}
        aria-label={label}
        aria-valuetext={value == null ? "此设备不支持" : `${draft}%`}
        disabled={value == null || disabled || saving}
        onChange={(e) => {
          editing.current = true;
          setDraft(Number(e.target.value));
        }}
        onPointerUp={() => void commit()}
        onKeyUp={() => void commit()}
        onBlur={() => void commit()}
        onPointerCancel={() => {
          editing.current = false;
          setDraft(value ?? 0);
        }}
      />
    </div>
  );
}

function SoundOutputView({
  onBack,
  onError,
}: {
  onBack: () => void;
  onError: (message: string) => void;
}) {
  const [mixer, setMixer] = useState<SoundMixerState | null>(native ? null : {
    defaultDeviceId: "speakers",
    devices: [
      { id: "speakers", name: "扬声器 (Realtek(R) Audio)", isDefault: true },
      { id: "headset", name: "耳机 (示例设备)", isDefault: false },
    ],
    apps: [
      { appKey: "system", processId: 0, name: "系统声音", volume: 100, muted: false, deviceId: "speakers", isSystem: true },
      { appKey: "demo", processId: 1, name: "示例应用", volume: 80, muted: false, deviceId: "speakers", isSystem: false },
    ],
  });
  const [expanded, setExpanded] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const active = useRef(true);

  async function refresh() {
    if (!native || !active.current) return;
    try {
      const next = await invoke<SoundMixerState>("control_center_sound_mixer");
      if (active.current) setMixer(next);
    } catch (e) {
      onError(String(e));
    }
  }

  useEffect(() => {
    active.current = true;
    void refresh();
    const timer = window.setInterval(() => {
      if (!busy) void refresh();
    }, 2500);
    return () => {
      active.current = false;
      clearInterval(timer);
    };
  }, []);

  useEffect(() => fit(), [mixer, expanded]);

  async function selectDevice(deviceId: string) {
    if (!native || busy) return;
    setBusy(true);
    onError("");
    try {
      await invoke("control_center_set_output_device", { deviceId });
      await refresh();
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function setAppVolume(app: MixerApp, volume: number) {
    if (!native) {
      setMixer((m) =>
        m
          ? { ...m, apps: m.apps.map((a) => (a.appKey === app.appKey ? { ...a, volume, muted: false } : a)) }
          : m,
      );
      return;
    }
    setBusy(true);
    onError("");
    try {
      await invoke("control_center_set_app_volume", {
        appKey: app.appKey,
        processId: app.processId,
        isSystem: app.isSystem,
        volume,
      });
      setMixer((m) =>
        m
          ? { ...m, apps: m.apps.map((a) => (a.appKey === app.appKey ? { ...a, volume, muted: false } : a)) }
          : m,
      );
    } catch (e) {
      onError(String(e));
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  async function setAppDevice(app: MixerApp, deviceId: string) {
    if (app.isSystem) return;
    if (!native) {
      setMixer((m) =>
        m
          ? { ...m, apps: m.apps.map((a) => (a.appKey === app.appKey ? { ...a, deviceId } : a)) }
          : m,
      );
      setExpanded(null);
      return;
    }
    setBusy(true);
    onError("");
    try {
      await invoke("control_center_set_app_device", {
        appKey: app.appKey,
        processId: app.processId,
        isSystem: app.isSystem,
        deviceId,
      });
      setExpanded(null);
      await refresh();
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const devices = mixer?.devices ?? [];
  const apps = mixer?.apps ?? [];

  return (
    <div className="cc-sound-view">
      <header className="cc-sound-header">
        <button type="button" className="cc-back" aria-label="返回" title="返回" onClick={onBack}>
          <Icon name="back" />
        </button>
        <strong>声音输出</strong>
      </header>

      <section className="cc-card cc-device-list" aria-label="输出设备">
        <h2 className="cc-section-title">输出设备</h2>
        {devices.length === 0 ? (
          <p className="cc-empty">未找到输出设备</p>
        ) : (
          devices.map((device) => (
            <button
              key={device.id}
              type="button"
              className="cc-device-row"
              data-active={device.isDefault}
              disabled={busy}
              onClick={() => void selectDevice(device.id)}
            >
              <span className="cc-device-icon" aria-hidden="true">
                <Icon name="speaker" />
              </span>
              <span className="cc-device-name">{device.name}</span>
            </button>
          ))
        )}
      </section>

      <section className="cc-card cc-mixer" aria-label="音量合成器">
        <h2 className="cc-section-title">音量合成器</h2>
        {apps.length === 0 ? (
          <p className="cc-empty">暂无正在播放的应用</p>
        ) : (
          apps.map((app) => (
            <div key={app.appKey} className="cc-app-block" data-expanded={expanded === app.appKey}>
              <div className="cc-app-row">
                <span className="cc-app-icon" aria-hidden="true">
                  {app.iconPngBase64 ? (
                    <img src={`data:image/png;base64,${app.iconPngBase64}`} alt="" draggable={false} />
                  ) : app.isSystem ? (
                    <Icon name="speaker" />
                  ) : (
                    <span>{app.name.slice(0, 1)}</span>
                  )}
                </span>
                <Slider label={`${app.name} 音量`} value={app.volume} onCommit={(v) => setAppVolume(app, v)} />
                {!app.isSystem && (
                  <button
                    type="button"
                    className="cc-app-expand"
                    aria-expanded={expanded === app.appKey}
                    aria-label={`${app.name} 输出设备`}
                    title="选择输出设备"
                    disabled={busy}
                    onClick={() => setExpanded((cur) => (cur === app.appKey ? null : app.appKey))}
                  >
                    <Icon name="chevron" />
                  </button>
                )}
              </div>
              {expanded === app.appKey && (
                <div className="cc-app-devices" role="listbox" aria-label={`${app.name} 输出设备`}>
                  {devices.map((device) => (
                    <button
                      key={device.id}
                      type="button"
                      role="option"
                      className="cc-device-row cc-device-row-nested"
                      data-active={app.deviceId === device.id}
                      aria-selected={app.deviceId === device.id}
                      disabled={busy}
                      onClick={() => void setAppDevice(app, device.id)}
                    >
                      <span className="cc-device-icon" aria-hidden="true">
                        <Icon name="speaker" />
                      </span>
                      <span className="cc-device-name">{device.name}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          ))
        )}
      </section>

      <button
        type="button"
        className="cc-more-link"
        onClick={() => void invoke("control_center_action", { action: "apps-volume" }).catch(() => undefined)}
      >
        更多音量设置
      </button>
    </div>
  );
}

export default function ControlCenterPopup() {
  const [view, setView] = useState<View>("home");
  const [audio, setAudio] = useState<AudioState>({
    volume: native ? null : 54,
    muted: false,
    micMuted: native ? null : false,
  });
  const [brightness, setBrightness] = useState<number | null>(native ? null : 100);
  const [wifi, setWifi] = useState<WifiState>({ enabled: false, connected: false, ssid: "" });
  const [error, setError] = useState("");
  const active = useRef(true);
  const updating = useRef(false);
  const viewRef = useRef(view);
  viewRef.current = view;

  useEffect(() => {
    if (!native) {
      void syncGlassCss({
        kind: "mica-alt",
        dark: new URLSearchParams(location.search).get("theme") !== "light",
      });
      return;
    }
    let disposed = false;
    let refreshing = false;
    let brightnessReading = false;
    let lastBrightnessRead = 0;
    let prefs: GlassPrefs = { kind: "mica-alt", dark: null };
    const cleanups: Array<() => void> = [];
    const attach = (promise: Promise<() => void>) => {
      void promise.then((off) => (disposed ? off() : cleanups.push(off)));
    };
    async function refresh() {
      if (!active.current || disposed || refreshing || updating.current) return;
      refreshing = true;
      const result = await Promise.allSettled([
        invoke<AudioState>("control_center_audio"),
        invoke<WifiState>("get_wifi_state"),
      ]);
      if (!disposed && !updating.current) {
        if (result[0].status === "fulfilled") setAudio(result[0].value);
        if (result[1].status === "fulfilled") setWifi(result[1].value);
      }
      refreshing = false;
    }
    async function opened() {
      active.current = true;
      setError("");
      setView("home");
      void refresh();
      if (!brightnessReading && Date.now() - lastBrightnessRead > 5000) {
        brightnessReading = true;
        void invoke<number | null>("control_center_brightness")
          .then((v) => {
            if (!disposed) setBrightness(v);
          })
          .catch(() => {
            if (!disposed) setBrightness(null);
          })
          .finally(() => {
            brightnessReading = false;
            lastBrightnessRead = Date.now();
          });
      }
    }
    void invoke<GlassPrefs>("get_material_prefs")
      .then((p) => {
        prefs = p;
        return syncGlassCss(p);
      })
      .catch(() => syncGlassCss({ kind: "mica-alt", dark: true }));
    void invoke("apply_window_effect").catch(console.error);
    void opened();
    attach(listen("control-center-popup-opened", opened));
    attach(
      listen("control-center-popup-closed", () => {
        active.current = false;
      }),
    );
    attach(
      listen<GlassPrefs>("material-prefs", (e) => {
        prefs = e.payload;
        void syncGlassCss(prefs);
      }),
    );
    cleanups.push(
      subscribeSystemDark(() => {
        if (prefs.dark == null) void syncGlassCss(prefs);
      }),
    );
    const timer = window.setInterval(() => void refresh(), 2000);
    const escape = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (viewRef.current !== "home") setView("home");
        else void invoke("close_control_center");
      }
    };
    window.addEventListener("keydown", escape);
    return () => {
      disposed = true;
      clearInterval(timer);
      cleanups.forEach((off) => off());
      window.removeEventListener("keydown", escape);
    };
  }, []);

  useEffect(() => {
    fit();
  }, [view, error]);

  async function action(name: string, value?: number) {
    setError("");
    if (!native) return;
    await invoke("control_center_action", { action: name, value }).catch((e) => {
      setError(String(e));
      throw e;
    });
  }
  const launch = (name: string) => {
    void action(name).catch(() => undefined);
  };
  const openSub = (next: View) => {
    setError("");
    setView(next);
  };

  if (view === "sound") {
    return (
      <ChromePopupShell className="cc-panel" aria-label="声音输出">
        <SoundOutputView onBack={() => setView("home")} onError={setError} />
        {error && (
          <p className="cc-error" role="alert">
            {error}
          </p>
        )}
      </ChromePopupShell>
    );
  }

  if (view === "wifi") {
    return (
      <ChromePopupShell className="cc-panel" aria-label="Wi-Fi">
        <ControlCenterWifiView onBack={() => setView("home")} onError={setError} />
        {error && (
          <p className="cc-error" role="alert">
            {error}
          </p>
        )}
      </ChromePopupShell>
    );
  }

  if (view === "bluetooth") {
    return (
      <ChromePopupShell className="cc-panel" aria-label="蓝牙">
        <ControlCenterBluetoothView onBack={() => setView("home")} onError={setError} />
        {error && (
          <p className="cc-error" role="alert">
            {error}
          </p>
        )}
      </ChromePopupShell>
    );
  }

  return (
    <ChromePopupShell className="cc-panel" aria-label="控制中心">
      <div className="cc-top">
        <section className="cc-card cc-connections" aria-label="连接">
          <button className="cc-connection" onClick={() => openSub("wifi")} title="Wi-Fi">
            <span className="cc-orb" data-active={wifi.enabled}>
              <Icon name="wifi" />
            </span>
            <span className="cc-connection-text">
              <strong>Wi-Fi</strong>
              {wifi.connected && <small>{wifi.ssid}</small>}
            </span>
            <span className="cc-conn-chevron" aria-hidden="true">
              <Icon name="chevronRight" />
            </span>
          </button>
          <button className="cc-connection" onClick={() => openSub("bluetooth")} title="蓝牙">
            <span className="cc-orb">
              <Icon name="bluetooth" />
            </span>
            <span className="cc-connection-text">
              <strong>蓝牙</strong>
              <small>设备</small>
            </span>
            <span className="cc-conn-chevron" aria-hidden="true">
              <Icon name="chevronRight" />
            </span>
          </button>
          <button className="cc-connection" onClick={() => launch("hotspot")} title="打开移动热点设置">
            <span className="cc-orb">
              <Icon name="hotspot" />
            </span>
            <strong>热点</strong>
          </button>
        </section>
        <div className="cc-shortcuts">
          <button className="cc-card cc-focus cc-placeholder" aria-disabled="true" title="专注助手 · 暂未开放">
            <span className="cc-orb">
              <Icon name="moon" />
            </span>
            <strong>专注助手</strong>
          </button>
          <div className="cc-shortcut-pair">
            <button className="cc-card cc-tile cc-placeholder" aria-disabled="true" title="台前调度 · 暂未开放">
              <Icon name="stage" />
              <strong>台前调度</strong>
            </button>
            <button className="cc-card cc-tile" onClick={() => launch("cast")} title="打开无线投影">
              <Icon name="cast" />
              <strong>投影</strong>
            </button>
          </div>
        </div>
      </div>
      <section className="cc-card cc-level-card" aria-label="显示器">
        <button className="cc-heading" onClick={() => launch("display")} title="打开显示设置">
          显示器
        </button>
        <Slider
          label="显示器亮度"
          value={brightness}
          onCommit={async (value) => {
            try {
              const actual = native ? await invoke<number | null>("control_center_brightness", { value }) : value;
              setBrightness(actual);
            } catch (e) {
              setError(String(e));
              setBrightness(null);
            }
          }}
        />
      </section>
      <section className="cc-card cc-level-card" aria-label="声音">
        <button className="cc-heading" onClick={() => launch("sound")} title="打开声音设置">
          声音{audio.muted && <small>已静音</small>}
        </button>
        <div className="cc-sound-row">
          <Slider
            label="系统音量"
            value={audio.volume}
            onCommit={async (value) => {
              updating.current = true;
              try {
                await action("volume", value);
                setAudio((a) => ({ ...a, volume: value, muted: false }));
              } catch {
                if (native) {
                  await invoke<AudioState>("control_center_audio")
                    .then(setAudio)
                    .catch(() => setAudio((a) => ({ ...a, volume: null })));
                }
              } finally {
                updating.current = false;
              }
            }}
          />
          <button
            className="cc-mixer-btn"
            aria-label="声音输出与音量合成器"
            title="声音输出"
            onClick={() => {
              setError("");
              setView("sound");
            }}
          >
            <Icon name="mixer" />
            <Icon name="chevronRight" />
          </button>
        </div>
      </section>
      <section className="cc-card cc-media" aria-label="媒体控制">
        <span className="cc-music">
          <Icon name="music" />
        </span>
        <div className="cc-media-actions">
          <button aria-label="播放或暂停" title="播放或暂停" onClick={() => launch("play_pause")}>
            <Icon name="play" />
          </button>
          <button aria-label="下一首" title="下一首" onClick={() => launch("next")}>
            <Icon name="next" />
          </button>
        </div>
      </section>
      {error && (
        <p className="cc-error" role="alert">
          {error}
        </p>
      )}
    </ChromePopupShell>
  );
}

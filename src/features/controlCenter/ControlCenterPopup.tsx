import { useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { syncGlassCss, subscribeSystemDark, type GlassPrefs } from "../../glassPrefs";
import { schedulePopupFit } from "../../popupFit";
import Icon from "./ControlCenterIcon";
import "./controlCenter.css";

type AudioState = { volume: number | null; muted: boolean; micMuted: boolean | null };
type WifiState = { enabled: boolean; connected: boolean; ssid: string };
const native = isTauri();
const fit = () => { if (native) schedulePopupFit({ width: 374, selector: ".cc-panel", minHeight: 464, maxHeight: 620 }, [0]); };

function Slider({ label, value, disabled, onCommit }: { label: string; value: number | null; disabled?: boolean; onCommit: (value: number) => Promise<void> }) {
  const [draft, setDraft] = useState(value ?? 0);
  const [saving, setSaving] = useState(false);
  const editing = useRef(false);
  useEffect(() => { if (!editing.current && !saving) setDraft(value ?? 0); }, [value, saving]);
  async function commit() {
    if (!editing.current) return;
    editing.current = false;
    setSaving(true);
    try { await onCommit(draft); } finally { setSaving(false); }
  }
  return <div className="cc-slider" data-disabled={value == null || disabled || saving} style={{ "--cc-level": `${draft}%` } as CSSProperties}>
    <span aria-hidden="true">{value == null ? "不可用" : draft}</span>
    <input type="range" min="0" max="100" value={draft} aria-label={label} aria-valuetext={value == null ? "此设备不支持" : `${draft}%`}
      disabled={value == null || disabled || saving}
      onChange={e => { editing.current = true; setDraft(Number(e.target.value)); }}
      onPointerUp={() => void commit()} onKeyUp={() => void commit()} onBlur={() => void commit()}
      onPointerCancel={() => { editing.current = false; setDraft(value ?? 0); }} />
  </div>;
}

export default function ControlCenterPopup() {
  const [audio, setAudio] = useState<AudioState>({ volume: native ? null : 54, muted: false, micMuted: native ? null : false });
  const [brightness, setBrightness] = useState<number | null>(native ? null : 100);
  const [wifi, setWifi] = useState<WifiState>({ enabled: false, connected: false, ssid: "" });
  const [error, setError] = useState("");
  const [micBusy, setMicBusy] = useState(false);
  const active = useRef(true);
  const updating = useRef(false);

  useEffect(() => {
    if (!native) { void syncGlassCss({ kind: "mica-alt", dark: new URLSearchParams(location.search).get("theme") !== "light" }); return; }
    let disposed = false;
    let refreshing = false;
    let brightnessReading = false;
    let lastBrightnessRead = 0;
    let prefs: GlassPrefs = { kind: "mica-alt", dark: null };
    const cleanups: Array<() => void> = [];
    const attach = (promise: Promise<() => void>) => { void promise.then(off => disposed ? off() : cleanups.push(off)); };
    async function refresh() {
      if (!active.current || disposed || refreshing || updating.current) return;
      refreshing = true;
      const result = await Promise.allSettled([invoke<AudioState>("control_center_audio"), invoke<WifiState>("get_wifi_state")]);
      if (!disposed && !updating.current) {
        if (result[0].status === "fulfilled") setAudio(result[0].value);
        if (result[1].status === "fulfilled") setWifi(result[1].value);
      }
      refreshing = false;
    }
    async function opened() {
      active.current = true;
      setError("");
      void refresh();
      if (!brightnessReading && Date.now() - lastBrightnessRead > 5000) {
        brightnessReading = true;
        void invoke<number | null>("control_center_brightness")
          .then(v => { if (!disposed) setBrightness(v); })
          .catch(() => { if (!disposed) setBrightness(null); })
          .finally(() => { brightnessReading = false; lastBrightnessRead = Date.now(); });
      }
    }
    void invoke<GlassPrefs>("get_material_prefs").then(p => { prefs = p; return syncGlassCss(p); }).catch(() => syncGlassCss({ kind: "mica-alt", dark: true }));
    void invoke("apply_window_effect").catch(console.error);
    void opened();
    attach(listen("control-center-popup-opened", opened));
    attach(listen("control-center-popup-closed", () => { active.current = false; }));
    attach(listen<GlassPrefs>("material-prefs", e => { prefs = e.payload; void syncGlassCss(prefs); }));
    cleanups.push(subscribeSystemDark(() => { if (prefs.dark == null) void syncGlassCss(prefs); }));
    const timer = window.setInterval(() => void refresh(), 2000);
    const escape = (e: KeyboardEvent) => { if (e.key === "Escape") void invoke("close_control_center"); };
    window.addEventListener("keydown", escape);
    return () => { disposed = true; clearInterval(timer); cleanups.forEach(off => off()); window.removeEventListener("keydown", escape); };
  }, []);
  useEffect(fit, [error]);

  async function action(name: string, value?: number) {
    setError("");
    if (!native) return;
    await invoke("control_center_action", { action: name, value }).catch(e => { setError(String(e)); throw e; });
  }
  const launch = (name: string) => { void action(name).catch(() => undefined); };
  async function openWifi() {
    if (!native) return;
    try {
      const win = getCurrentWindow();
      const [position, scale] = await Promise.all([win.outerPosition(), win.scaleFactor()]);
      await invoke("open_wifi_popup", { x: position.x / scale + 94, y: position.y / scale });
    } catch (e) { setError(String(e)); }
  }

  return <main className="cc-panel" aria-label="控制中心">
    <div className="cc-top">
      <section className="cc-card cc-connections" aria-label="连接">
        <button className="cc-connection" onClick={() => void openWifi()} title="打开 Wi-Fi 菜单">
          <span className="cc-orb" data-active={wifi.enabled}><Icon name="wifi" /></span>
          <span className="cc-connection-text"><strong>Wi-Fi</strong>{wifi.connected && <small>{wifi.ssid}</small>}</span>
        </button>
        <button className="cc-connection" onClick={() => launch("bluetooth")} title="打开蓝牙设置">
          <span className="cc-orb"><Icon name="bluetooth" /></span><span className="cc-connection-text"><strong>蓝牙</strong><small>设置</small></span>
        </button>
        <button className="cc-connection" onClick={() => launch("hotspot")} title="打开移动热点设置">
          <span className="cc-orb"><Icon name="hotspot" /></span><strong>热点</strong>
        </button>
      </section>
      <div className="cc-shortcuts">
        <button className="cc-card cc-focus cc-placeholder" aria-disabled="true" title="专注助手 · 暂未开放"><span className="cc-orb"><Icon name="moon" /></span><strong>专注助手</strong></button>
        <div className="cc-shortcut-pair">
          <button className="cc-card cc-tile cc-placeholder" aria-disabled="true" title="台前调度 · 暂未开放"><Icon name="stage" /><strong>台前调度</strong></button>
          <button className="cc-card cc-tile" onClick={() => launch("cast")} title="打开无线投影"><Icon name="cast" /><strong>投影</strong></button>
        </div>
      </div>
    </div>
    <section className="cc-card cc-level-card" aria-label="显示器">
      <button className="cc-heading" onClick={() => launch("display")} title="打开显示设置">显示器</button>
      <Slider label="显示器亮度" value={brightness} onCommit={async value => {
        try { const actual = native ? await invoke<number | null>("control_center_brightness", { value }) : value; setBrightness(actual); }
        catch (e) { setError(String(e)); setBrightness(null); }
      }} />
    </section>
    <section className="cc-card cc-level-card" aria-label="声音">
      <button className="cc-heading" onClick={() => launch("sound")} title="打开声音设置">声音{audio.muted && <small>已静音</small>}</button>
      <div className="cc-sound-row">
        <Slider label="系统音量" value={audio.volume} onCommit={async value => {
          updating.current = true;
          try { await action("volume", value); setAudio(a => ({ ...a, volume: value, muted: false })); }
          catch { if (native) await invoke<AudioState>("control_center_audio").then(setAudio).catch(() => setAudio(a => ({ ...a, volume: null }))); }
          finally { updating.current = false; }
        }} />
        <button className="cc-mic" aria-label={audio.micMuted ? "取消麦克风静音" : "麦克风静音"} aria-pressed={audio.micMuted ?? false} disabled={audio.micMuted == null || micBusy} title={audio.micMuted ? "麦克风已静音" : "麦克风静音"} onClick={() => {
          setMicBusy(true); updating.current = true;
          void action("mic", audio.micMuted ? 0 : 1).then(() => setAudio(a => ({ ...a, micMuted: !a.micMuted }))).catch(() => undefined).finally(() => { setMicBusy(false); updating.current = false; });
        }}><Icon name="mic" /></button>
      </div>
    </section>
    <section className="cc-card cc-media" aria-label="媒体控制">
      <span className="cc-music"><Icon name="music" /></span>
      <div className="cc-media-actions"><button aria-label="播放或暂停" title="播放或暂停" onClick={() => launch("play_pause")}><Icon name="play" /></button><button aria-label="下一首" title="下一首" onClick={() => launch("next")}><Icon name="next" /></button></div>
    </section>
    {error && <p className="cc-error" role="alert">{error}</p>}
  </main>;
}

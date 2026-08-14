import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  getIslandPrefs,
  setIslandPrefs,
  subscribeIslandPrefs,
  type IslandPrefs,
} from "../islandPrefs";
import {
  getScenarioGate,
  upsertScenarioGate,
  windowKeyOf,
  type ScenarioGate,
} from "../scenarioGates";
import type { WindowInfo } from "../types";

type TrayIconInfo = {
  id: string;
  pin_key?: string;
  tooltip: string;
  process: string;
  icon_png_base64: string;
};

type PresenceKind = "tray" | "window";

type PresenceItem = {
  kind: PresenceKind;
  key: string;
  label: string;
  sub?: string;
  iconPng?: string;
  orphan?: boolean;
};

function trayPinKey(icon: TrayIconInfo): string {
  const k = (icon.pin_key || "").trim();
  return k || icon.id;
}

function trayLabel(icon: TrayIconInfo) {
  return (icon.tooltip || "").trim() || icon.process || icon.id;
}

function normalizeOpenWindow(raw: Record<string, unknown> | WindowInfo): WindowInfo {
  const w = raw as Record<string, unknown>;
  return {
    id: typeof w.id === "string" ? w.id : undefined,
    hwnd: Number(w.hwnd) || 0,
    title: String(w.title ?? ""),
    class_name: String(w.class_name ?? w.className ?? ""),
    pid: Number(w.pid) || 0,
    exe: (w.exe as string | null | undefined) ?? null,
    exe_name:
      (w.exe_name as string | null | undefined) ??
      (w.exeName as string | null | undefined) ??
      null,
  };
}

type Props = {
  pluginId: string;
  enabled?: boolean;
};

/**
 * Host presence gates only. Open-app tray binding lives in plugin settings (OpenTraySetting).
 */
export default function ScenarioGateSettings({ pluginId, enabled = true }: Props) {
  const [islandPrefs, setIslandPrefsState] = useState<IslandPrefs>(() => getIslandPrefs());
  const [trays, setTrays] = useState<TrayIconInfo[]>([]);
  const [openWindows, setOpenWindows] = useState<WindowInfo[]>([]);
  const [presencePickerOpen, setPresencePickerOpen] = useState(false);
  const presencePickerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    void setIslandPrefsState(getIslandPrefs());
    return subscribeIslandPrefs(setIslandPrefsState);
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
    void invoke<TrayIconInfo[]>("list_tray_icons")
      .then((list) => {
        if (!cancelled) setTrays(list ?? []);
      })
      .catch(() => undefined);
    void listen<TrayIconInfo[]>("tray-icons", (ev) => {
      setTrays(ev.payload ?? []);
    }).then((fn) => unsubs.push(fn));

    void invoke<WindowInfo[]>("list_open_windows")
      .then((list) => {
        if (!cancelled) setOpenWindows((list ?? []).map((w) => normalizeOpenWindow(w)));
      })
      .catch(() => undefined);
    void listen<{ windows?: WindowInfo[] } | WindowInfo[]>("hub-windows-changed", (ev) => {
      const raw = Array.isArray(ev.payload) ? ev.payload : ev.payload?.windows;
      if (Array.isArray(raw)) {
        setOpenWindows(raw.map((w) => normalizeOpenWindow(w as WindowInfo)));
      }
    }).then((fn) => unsubs.push(fn));

    return () => {
      cancelled = true;
      unsubs.forEach((fn) => fn());
    };
  }, []);

  useEffect(() => {
    if (!presencePickerOpen) return;
    const onPointer = (ev: MouseEvent) => {
      if (
        presencePickerRef.current &&
        !presencePickerRef.current.contains(ev.target as Node)
      ) {
        setPresencePickerOpen(false);
      }
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") setPresencePickerOpen(false);
    };
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [presencePickerOpen]);

  async function updateGate(next: ScenarioGate) {
    const scenarioGates = upsertScenarioGate(
      islandPrefs.scenarioGates ?? {},
      pluginId,
      next,
    );
    const prefs = await setIslandPrefs({ scenarioGates });
    setIslandPrefsState(prefs);
  }

  const gate = getScenarioGate(islandPrefs.scenarioGates, pluginId);
  const liveTrayKeys = new Set(trays.map((t) => trayPinKey(t)).filter(Boolean));
  const winByKey = new Map<string, WindowInfo>();
  for (const w of openWindows) {
    const k = windowKeyOf(w);
    if (k && !winByKey.has(k)) winByKey.set(k, w);
  }

  const presenceItems: PresenceItem[] = [];
  for (const icon of trays) {
    const key = trayPinKey(icon);
    if (!key) continue;
    presenceItems.push({
      kind: "tray",
      key,
      label: trayLabel(icon),
      iconPng: icon.icon_png_base64 || undefined,
    });
  }
  for (const [key, w] of winByKey) {
    presenceItems.push({
      kind: "window",
      key,
      label: (w.title || "").trim() || w.exe_name || w.exe || key,
      sub: w.exe_name || w.exe || undefined,
    });
  }
  for (const key of gate.trayKeys) {
    if (liveTrayKeys.has(key)) continue;
    if (presenceItems.some((i) => i.kind === "tray" && i.key === key)) continue;
    presenceItems.push({
      kind: "tray",
      key,
      label: "已选（当前不在托盘）",
      sub: key,
      orphan: true,
    });
  }
  for (const key of gate.windowKeys) {
    if (winByKey.has(key)) continue;
    if (presenceItems.some((i) => i.kind === "window" && i.key === key)) continue;
    presenceItems.push({
      kind: "window",
      key,
      label: "已选（当前无此窗口）",
      sub: key,
      orphan: true,
    });
  }

  const presenceCount = gate.trayKeys.length + gate.windowKeys.length;

  function isPresenceOn(item: PresenceItem) {
    return item.kind === "tray"
      ? gate.trayKeys.includes(item.key)
      : gate.windowKeys.includes(item.key);
  }

  function togglePresence(item: PresenceItem) {
    if (item.kind === "tray") {
      const on = gate.trayKeys.includes(item.key);
      const trayKeys = on
        ? gate.trayKeys.filter((k) => k !== item.key)
        : [...gate.trayKeys, item.key];
      void updateGate({ ...gate, trayKeys });
      return;
    }
    const on = gate.windowKeys.includes(item.key);
    const windowKeys = on
      ? gate.windowKeys.filter((k) => k !== item.key)
      : [...gate.windowKeys, item.key];
    void updateGate({ ...gate, windowKeys });
  }

  if (!enabled) {
    return (
      <div className="scenario-gate-settings">
        <h3 className="scenario-gate-settings-title">情景临时</h3>
        <p className="scenario-gate-hint">启用插件后可配置存在条件。</p>
      </div>
    );
  }

  return (
    <div className="scenario-gate-settings">
      <h3 className="scenario-gate-settings-title">情景临时</h3>
      <p className="scenario-gate-hint">
        存在条件由 Host 强制门禁。勾选后须全部当前存在才允许接管，全不勾选则不限制。
      </p>

      <div className="scenario-gate-block">
        <div className="tray-detail-row-title">存在条件</div>
        <p className="scenario-gate-hint scenario-gate-hint-tight">
          下拉多选。托盘与窗口在同一列表；勾选的项须全部当前存在才允许接管。
        </p>
        <div
          className={`scenario-tray-picker${presencePickerOpen ? " is-open" : ""}`}
          ref={presencePickerRef}
        >
          <button
            type="button"
            className="scenario-tray-picker-trigger"
            aria-haspopup="listbox"
            aria-expanded={presencePickerOpen}
            onClick={() => setPresencePickerOpen((v) => !v)}
          >
            <span className="scenario-tray-picker-icon scenario-tray-picker-fallback">
              {presenceCount > 0 ? String(Math.min(presenceCount, 9)) : "—"}
            </span>
            <span className="scenario-tray-picker-label">
              {presenceCount > 0 ? `已选 ${presenceCount} 项` : "未限制存在条件"}
            </span>
            <span className="scenario-tray-picker-chevron" aria-hidden>
              <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
                <path
                  d="M4 6l4 4 4-4"
                  stroke="currentColor"
                  strokeWidth="1.5"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                />
              </svg>
            </span>
          </button>
          {presencePickerOpen ? (
            <div
              className="scenario-tray-picker-menu"
              role="listbox"
              aria-multiselectable
              aria-label="存在条件"
            >
              {presenceItems.length === 0 ? (
                <p className="scenario-gate-empty scenario-tray-picker-empty">
                  暂无可用的托盘或窗口
                </p>
              ) : (
                presenceItems.map((item) => {
                  const on = isPresenceOn(item);
                  return (
                    <button
                      key={`${item.kind}:${item.key}`}
                      type="button"
                      role="option"
                      aria-selected={on}
                      className={`scenario-tray-picker-option${on ? " is-selected" : ""}${item.orphan ? " is-orphan" : ""}`}
                      onClick={() => togglePresence(item)}
                    >
                      {item.iconPng ? (
                        <img
                          className="scenario-tray-picker-icon"
                          src={`data:image/png;base64,${item.iconPng}`}
                          alt=""
                          draggable={false}
                        />
                      ) : (
                        <span className="scenario-tray-picker-icon scenario-tray-picker-fallback">
                          {item.kind === "tray" ? "托" : "窗"}
                        </span>
                      )}
                      <span className="scenario-tray-picker-label">
                        <span className="scenario-gate-win-title">{item.label}</span>
                        {item.sub ? (
                          <span className="scenario-gate-win-exe">{item.sub}</span>
                        ) : null}
                      </span>
                      <span className={`scenario-presence-badge is-${item.kind}`}>
                        {item.kind === "tray" ? "托盘" : "窗口"}
                      </span>
                      <span
                        className={`scenario-presence-check${on ? " is-on" : ""}`}
                        aria-hidden
                      >
                        {on ? "✓" : ""}
                      </span>
                    </button>
                  );
                })
              )}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

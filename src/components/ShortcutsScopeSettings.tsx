import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { windowKeyOf } from "../scenarioGates";
import {
  getPluginScope,
  upsertPluginScope,
  type ShortcutsPluginScope,
  type ShortcutsScopeMode,
} from "../shortcutsPrefs";
import type { WindowInfo } from "../types";

type ScopeMap = Record<string, ShortcutsPluginScope>;

type Props = {
  pluginId: string;
  pluginLabel: string;
  scopes: ScopeMap;
  onScopesChange: (next: ScopeMap) => void | Promise<void>;
  /** Pure island.bar workers — scope N/A (always mounted). */
  barWorker?: boolean;
};

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

type WinItem = {
  key: string;
  label: string;
  sub?: string;
  orphan?: boolean;
};

/**
 * Per shortcuts plugin: show for all programs, or only selected EXEs/windows.
 */
export default function ShortcutsScopeSettings({
  pluginId,
  pluginLabel,
  scopes,
  onScopesChange,
  barWorker = false,
}: Props) {
  const [openWindows, setOpenWindows] = useState<WindowInfo[]>([]);
  const [pickerOpen, setPickerOpen] = useState(false);
  const pickerRef = useRef<HTMLDivElement | null>(null);
  const scope = getPluginScope(scopes, pluginId);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
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
    if (!pickerOpen) return;
    const onPointer = (ev: MouseEvent) => {
      if (pickerRef.current && !pickerRef.current.contains(ev.target as Node)) {
        setPickerOpen(false);
      }
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") setPickerOpen(false);
    };
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [pickerOpen]);

  const winByKey = useMemo(() => {
    const map = new Map<string, WindowInfo>();
    for (const w of openWindows) {
      const k = windowKeyOf(w);
      if (k && !map.has(k)) map.set(k, w);
    }
    return map;
  }, [openWindows]);

  const items: WinItem[] = useMemo(() => {
    const out: WinItem[] = [];
    for (const [key, w] of winByKey) {
      out.push({
        key,
        label: (w.title || "").trim() || w.exe_name || w.exe || key,
        sub: w.exe_name || w.exe || undefined,
      });
    }
    for (const key of scope.windowKeys) {
      if (winByKey.has(key)) continue;
      out.push({
        key,
        label: "已选（当前无此窗口）",
        sub: key,
        orphan: true,
      });
    }
    return out;
  }, [winByKey, scope.windowKeys]);

  async function commit(next: ShortcutsPluginScope) {
    const map = upsertPluginScope(scopes, pluginId, next);
    await onScopesChange(map);
  }

  function setMode(mode: ShortcutsScopeMode) {
    void commit({
      mode,
      windowKeys: mode === "all" ? [] : scope.windowKeys,
    });
  }

  function toggleKey(key: string) {
    const on = scope.windowKeys.includes(key);
    const windowKeys = on
      ? scope.windowKeys.filter((k) => k !== key)
      : [...scope.windowKeys, key];
    void commit({ mode: "apps", windowKeys });
  }

  if (barWorker) {
    return (
      <div className="shortcuts-scope-card is-worker">
        <div className="shortcuts-scope-head">
          <span className="shortcuts-scope-name">{pluginLabel}</span>
          <span className="shortcuts-scope-badge">岛栏 worker</span>
        </div>
        <p className="card-desc">隐形轮询条，不占用快捷区宽度，始终挂载，无需按程序过滤。</p>
      </div>
    );
  }

  return (
    <div className="shortcuts-scope-card">
      <div className="shortcuts-scope-head">
        <span className="shortcuts-scope-name">{pluginLabel}</span>
        <span className="shortcuts-scope-id">{pluginId}</span>
      </div>
      <div className="mode-list shortcuts-scope-modes">
        <button
          type="button"
          className={`mode-item${scope.mode === "all" ? " is-selected" : ""}`}
          onClick={() => setMode("all")}
        >
          <span className="mode-label">全部程序</span>
          <span className="mode-desc">任意前台窗口都显示此快捷条</span>
        </button>
        <button
          type="button"
          className={`mode-item${scope.mode === "apps" ? " is-selected" : ""}`}
          onClick={() => setMode("apps")}
        >
          <span className="mode-label">仅以下程序</span>
          <span className="mode-desc">仅当前前台属于所选程序时显示</span>
        </button>
      </div>
      {scope.mode === "apps" ? (
        <div className="shortcuts-scope-picker-block">
          <p className="card-desc">
            从当前已打开窗口勾选程序；未勾选任何项时此条不显示。
          </p>
          <div
            className={`scenario-tray-picker scenario-tray-picker-text${pickerOpen ? " is-open" : ""}`}
            ref={pickerRef}
          >
            <button
              type="button"
              className="scenario-tray-picker-trigger"
              aria-haspopup="listbox"
              aria-expanded={pickerOpen}
              onClick={() => setPickerOpen((v) => !v)}
            >
              <span className="scenario-tray-picker-icon scenario-tray-picker-fallback">
                {scope.windowKeys.length > 0
                  ? String(Math.min(scope.windowKeys.length, 9))
                  : "—"}
              </span>
              <span className="scenario-tray-picker-label">
                {scope.windowKeys.length > 0
                  ? `已选 ${scope.windowKeys.length} 个程序`
                  : "选择程序…"}
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
            {pickerOpen ? (
              <div
                className="scenario-tray-picker-menu"
                role="listbox"
                aria-multiselectable
                aria-label={`${pluginLabel} 适用程序`}
              >
                {items.length === 0 ? (
                  <p className="scenario-gate-empty scenario-tray-picker-empty">
                    暂无打开的窗口，请先打开目标程序
                  </p>
                ) : (
                  items.map((item) => {
                    const on = scope.windowKeys.includes(item.key);
                    return (
                      <button
                        key={item.key}
                        type="button"
                        role="option"
                        aria-selected={on}
                        className={`scenario-tray-picker-option${on ? " is-selected" : ""}${
                          item.orphan ? " is-orphan" : ""
                        }`}
                        onClick={() => toggleKey(item.key)}
                      >
                        <span className="scenario-tray-picker-icon scenario-tray-picker-fallback">
                          窗
                        </span>
                        <span className="scenario-tray-picker-label">
                          <span className="scenario-gate-win-title">{item.label}</span>
                          {item.sub ? (
                            <span className="scenario-gate-win-exe">{item.sub}</span>
                          ) : null}
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
      ) : null}
    </div>
  );
}

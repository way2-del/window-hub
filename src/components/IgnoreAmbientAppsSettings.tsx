import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { processKeyOf, windowKeyOf } from "../scenarioGates";
import type { WindowInfo } from "../types";

type TrayIconInfo = {
  id: string;
  pin_key?: string;
  tooltip: string;
  process: string;
  icon_png_base64?: string;
};

type PickerItem = {
  key: string;
  kind: "window" | "tray";
  label: string;
  sub?: string;
  iconPng?: string;
  orphan?: boolean;
};

type Props = {
  keys: string[];
  onChange: (keys: string[]) => void | Promise<void>;
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

function trayLabel(icon: TrayIconInfo): string {
  return (icon.tooltip || "").trim() || icon.process || icon.id;
}

/** 顶栏吸色忽略列表 — 窗口 + 托盘程序多选（与快捷区 scope 同 proc:/exe: key）。 */
export default function IgnoreAmbientAppsSettings({ keys, onChange }: Props) {
  const [openWindows, setOpenWindows] = useState<WindowInfo[]>([]);
  const [trays, setTrays] = useState<TrayIconInfo[]>([]);
  const [pickerOpen, setPickerOpen] = useState(false);
  const pickerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
    void invoke<WindowInfo[]>("list_open_windows")
      .then((list) => {
        if (!cancelled) setOpenWindows((list ?? []).map((w) => normalizeOpenWindow(w)));
      })
      .catch(() => undefined);
    void invoke<TrayIconInfo[]>("list_tray_icons")
      .then((list) => {
        if (!cancelled) setTrays(list ?? []);
      })
      .catch(() => undefined);
    void listen<{ windows?: WindowInfo[] } | WindowInfo[]>("hub-windows-changed", (ev) => {
      const raw = Array.isArray(ev.payload) ? ev.payload : ev.payload?.windows;
      if (Array.isArray(raw)) {
        setOpenWindows(raw.map((w) => normalizeOpenWindow(w as WindowInfo)));
      }
    }).then((fn) => unsubs.push(fn));
    void listen<TrayIconInfo[]>("tray-icons", (ev) => {
      setTrays(ev.payload ?? []);
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

  const liveKeys = useMemo(() => {
    const set = new Set<string>();
    for (const w of openWindows) {
      const k = windowKeyOf(w);
      if (k) set.add(k);
    }
    for (const t of trays) {
      const k = processKeyOf(t.process);
      if (k) set.add(k);
    }
    return set;
  }, [openWindows, trays]);

  const items: PickerItem[] = useMemo(() => {
    const byKey = new Map<string, PickerItem>();

    for (const icon of trays) {
      const key = processKeyOf(icon.process);
      if (!key || byKey.has(key)) continue;
      byKey.set(key, {
        key,
        kind: "tray",
        label: trayLabel(icon),
        sub: icon.process || undefined,
        iconPng: icon.icon_png_base64 || undefined,
      });
    }

    for (const w of openWindows) {
      const key = windowKeyOf(w);
      if (!key) continue;
      const existing = byKey.get(key);
      if (existing) {
        if (!existing.iconPng) {
          existing.label = (w.title || "").trim() || existing.label;
          existing.sub = w.exe_name || w.exe || existing.sub;
        }
        continue;
      }
      byKey.set(key, {
        key,
        kind: "window",
        label: (w.title || "").trim() || w.exe_name || w.exe || key,
        sub: w.exe_name || w.exe || undefined,
      });
    }

    for (const key of keys) {
      if (byKey.has(key)) continue;
      byKey.set(key, {
        key,
        kind: key.startsWith("proc:") || key.startsWith("exe:") ? "tray" : "window",
        label: "已选（当前未运行）",
        sub: key,
        orphan: true,
      });
    }

    return [...byKey.values()].sort((a, b) => a.label.localeCompare(b.label, "zh-CN"));
  }, [openWindows, trays, keys]);

  function toggleKey(key: string) {
    const on = keys.includes(key);
    const next = on ? keys.filter((k) => k !== key) : [...keys, key];
    void onChange(next);
  }

  return (
    <div className="shortcuts-scope-picker-block">
      <p className="card-desc">
        所选程序处于前台、最大化或仅托盘运行时，顶栏不按其窗口吸色（仍跟桌面壁纸/材质）。适合
        PixPin 等截图、录屏工具。
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
            {keys.length > 0 ? String(Math.min(keys.length, 9)) : "—"}
          </span>
          <span className="scenario-tray-picker-label">
            {keys.length > 0 ? `已忽略 ${keys.length} 个程序` : "选择要忽略的程序…"}
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
            aria-label="忽略吸色的程序"
          >
            {items.length === 0 ? (
              <p className="scenario-gate-empty scenario-tray-picker-empty">
                暂无窗口或托盘程序；可先打开目标应用（如 PixPin）后再选
              </p>
            ) : (
              items.map((item) => {
                const on = keys.includes(item.key);
                const live = liveKeys.has(item.key);
                return (
                  <button
                    key={`${item.kind}:${item.key}`}
                    type="button"
                    role="option"
                    aria-selected={on}
                    className={`scenario-tray-picker-option${on ? " is-selected" : ""}${
                      item.orphan ? " is-orphan" : ""
                    }`}
                    onClick={() => toggleKey(item.key)}
                  >
                    {item.iconPng ? (
                      <img
                        className="scenario-tray-picker-icon"
                        src={`data:image/png;base64,${item.iconPng}`}
                        alt=""
                      />
                    ) : (
                      <span className="scenario-tray-picker-icon scenario-tray-picker-fallback">
                        {item.kind === "tray" ? "托" : "窗"}
                      </span>
                    )}
                    <span className="scenario-tray-picker-label">
                      <span className="scenario-gate-win-title">{item.label}</span>
                      {item.sub ? (
                        <span className="scenario-gate-win-exe">
                          {item.kind === "tray" && live && !item.orphan
                            ? `托盘 · ${item.sub}`
                            : item.sub}
                        </span>
                      ) : null}
                    </span>
                    <span className={`scenario-presence-check${on ? " is-on" : ""}`} aria-hidden>
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
  );
}

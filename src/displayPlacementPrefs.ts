/** Multi-monitor top-bar / Dock placement prefs (host IPC). */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type TopBarMode = "full" | "shortcuts" | "none";
export type PlacementPreset = "primaryOnly" | "allDisplays" | "custom";

export type MonitorPlacement = {
  id: string;
  topBar: TopBarMode;
  dock: boolean;
};

export type DisplayPlacementPrefs = {
  preset: PlacementPreset;
  monitors: MonitorPlacement[];
};

export type DisplayInfo = {
  id: string;
  name: string;
  isPrimary: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  scaleFactor: number;
};

export type ResolvedPlacement = DisplayInfo & {
  topBar: TopBarMode;
  dock: boolean;
};

export type PlacementSnapshot = {
  prefs: DisplayPlacementPrefs;
  displays: DisplayInfo[];
  resolved: ResolvedPlacement[];
  primaryTopBar: TopBarMode;
};

const DEFAULT_SNAP: PlacementSnapshot = {
  prefs: { preset: "primaryOnly", monitors: [] },
  displays: [],
  resolved: [],
  primaryTopBar: "full",
};

let cached: PlacementSnapshot = DEFAULT_SNAP;
const listeners = new Set<(s: PlacementSnapshot) => void>();

export function getDisplayPlacementSnapshot(): PlacementSnapshot {
  return cached;
}

export function getPrimaryTopBarMode(): TopBarMode {
  return cached.primaryTopBar || "full";
}

export function subscribeDisplayPlacement(
  fn: (s: PlacementSnapshot) => void,
): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

function notify(s: PlacementSnapshot) {
  cached = s;
  for (const fn of listeners) fn(s);
}

export async function hydrateDisplayPlacement(): Promise<PlacementSnapshot> {
  try {
    const s = await invoke<PlacementSnapshot>("get_display_placement");
    notify(s);
    return s;
  } catch {
    return cached;
  }
}

export async function setDisplayPlacement(
  prefs: DisplayPlacementPrefs,
): Promise<PlacementSnapshot> {
  const s = await invoke<PlacementSnapshot>("set_display_placement", { prefs });
  notify(s);
  return s;
}

export async function listDisplays(): Promise<DisplayInfo[]> {
  return invoke<DisplayInfo[]>("list_displays_cmd");
}

export function listenDisplayPlacement(
  onEvent: (s: PlacementSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<PlacementSnapshot>("display-placement", (ev) => {
    notify(ev.payload);
    onEvent(ev.payload);
  });
}

export const PLACEMENT_PRESETS: {
  id: PlacementPreset;
  label: string;
  desc: string;
}[] = [
  {
    id: "primaryOnly",
    label: "仅主屏",
    desc: "完整顶栏（含灵动岛）与 Dock 只在主显示器",
  },
  {
    id: "allDisplays",
    label: "全部显示器",
    desc: "主屏完整顶栏+岛+Dock；其它屏仅快捷区顶栏与 Dock",
  },
  {
    id: "custom",
    label: "自定义",
    desc: "按每块屏幕分别选择顶栏与 Dock",
  },
];

export const TOP_BAR_OPTIONS: { value: TopBarMode; label: string }[] = [
  { value: "full", label: "完整" },
  { value: "shortcuts", label: "仅快捷区" },
  { value: "none", label: "无" },
];

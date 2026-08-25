/** Scenario presence gates — Host-owned tray/window keys for claim allow. */

export type ScenarioGate = {
  trayKeys: string[];
  windowKeys: string[];
};

export type ScenarioGatesMap = Record<string, ScenarioGate>;

export type WindowKeySource = {
  exe?: string | null;
  exe_name?: string | null;
  /** Some IPC paths expose camelCase */
  exeName?: string | null;
};

export type TrayKeySource = {
  pin_key?: string;
  id: string;
};

/** Reboot-stable window key: exe path preferred, else process name. No hwnd/pid. */
export function windowKeyOf(w: WindowKeySource): string {
  const exe = String(w.exe || "")
    .trim()
    .replace(/\//g, "\\")
    .toLowerCase();
  if (exe) return `exe:${exe}`;
  const name = String(w.exe_name || w.exeName || "")
    .trim()
    .toLowerCase()
    .replace(/\.exe$/i, "");
  if (name) return `proc:${name}`;
  return "";
}

/** Tray `process` field → same bind key as `windowKeyOf` (for ambient ignore list). */
export function processKeyOf(raw: string): string {
  const t = String(raw || "").trim();
  if (!t) return "";
  if (t.includes("\\") || /^[a-zA-Z]:/.test(t)) {
    return windowKeyOf({ exe: t.replace(/\//g, "\\") });
  }
  return windowKeyOf({ exe_name: t });
}

export function trayKeyOf(t: TrayKeySource): string {
  const pk = String(t.pin_key || "").trim();
  if (pk) return pk;
  return String(t.id || "").trim();
}

export function emptyScenarioGate(): ScenarioGate {
  return { trayKeys: [], windowKeys: [] };
}

export function getScenarioGate(
  gates: ScenarioGatesMap | null | undefined,
  pluginId: string,
): ScenarioGate {
  const g = gates?.[pluginId];
  if (!g) return emptyScenarioGate();
  return {
    trayKeys: Array.isArray(g.trayKeys) ? g.trayKeys.map(String).filter(Boolean) : [],
    windowKeys: Array.isArray(g.windowKeys)
      ? g.windowKeys.map(String).filter(Boolean)
      : [],
  };
}

export function parseScenarioGates(raw: unknown): ScenarioGatesMap {
  if (!raw || typeof raw !== "object") return {};
  const out: ScenarioGatesMap = {};
  for (const [pid, val] of Object.entries(raw as Record<string, unknown>)) {
    const id = pid.trim();
    if (!id || !val || typeof val !== "object") continue;
    const v = val as Record<string, unknown>;
    const trayKeys = Array.isArray(v.trayKeys)
      ? [...new Set(v.trayKeys.map((x) => String(x || "").trim()).filter(Boolean))]
      : [];
    const windowKeys = Array.isArray(v.windowKeys)
      ? [...new Set(v.windowKeys.map((x) => String(x || "").trim()).filter(Boolean))]
      : [];
    if (trayKeys.length === 0 && windowKeys.length === 0) continue;
    out[id] = { trayKeys, windowKeys };
  }
  return out;
}

/**
 * Empty presence lists → allow. Non-empty → every selected tray and window key must be present.
 */
export function scenarioPresenceOk(
  pluginId: string,
  gates: ScenarioGatesMap | null | undefined,
  liveTrayKeys: Iterable<string>,
  liveWindowKeys: Iterable<string>,
): boolean {
  const g = getScenarioGate(gates, pluginId);
  if (g.trayKeys.length === 0 && g.windowKeys.length === 0) return true;
  const trays = new Set(
    [...liveTrayKeys].map((k) => k.trim()).filter(Boolean),
  );
  for (const k of g.trayKeys) {
    if (!trays.has(k)) return false;
  }
  const wins = new Set(
    [...liveWindowKeys].map((k) => k.trim()).filter(Boolean),
  );
  for (const k of g.windowKeys) {
    if (!wins.has(k)) return false;
  }
  return true;
}

export function upsertScenarioGate(
  gates: ScenarioGatesMap,
  pluginId: string,
  next: ScenarioGate,
): ScenarioGatesMap {
  const id = pluginId.trim();
  if (!id) return gates;
  const trayKeys = [...new Set(next.trayKeys.map((k) => k.trim()).filter(Boolean))];
  const windowKeys = [
    ...new Set(next.windowKeys.map((k) => k.trim()).filter(Boolean)),
  ];
  const out = { ...gates };
  if (trayKeys.length === 0 && windowKeys.length === 0) {
    delete out[id];
  } else {
    out[id] = { trayKeys, windowKeys };
  }
  return out;
}

/** Plugin settings key for scenario open-app tray binding. */
export const OPEN_TRAY_SETTING_KEY = "openTrayKey";

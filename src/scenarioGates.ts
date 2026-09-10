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

/** Strip tray uid suffix (`exe:path:1234` → `exe:path`) for fuzzy match when icon re-registers. */
export function trayPinKeyStem(key: string): string {
  const k = key.trim().toLowerCase();
  if (k.startsWith("exe:") || k.startsWith("proc:")) {
    const colon = k.indexOf(":");
    const rest = k.slice(colon + 1);
    const uidSep = rest.lastIndexOf(":");
    if (uidSep > 0 && /^\d+$/.test(rest.slice(uidSep + 1))) {
      return `${k.slice(0, colon + 1)}${rest.slice(0, uidSep)}`;
    }
    return k;
  }
  return k;
}

/** Exe path from tray pin_key (`exe:C:\\app.exe:uid` → `c:\\app.exe`). */
export function exePathFromTrayPinKey(key: string): string | null {
  const stem = trayPinKeyStem(key);
  if (!stem.startsWith("exe:")) return null;
  return stem.slice(4);
}

/** Numeric uid suffix on `exe:…:1234` / `proc:…:1234` keys. */
export function trayPinKeyUid(key: string): number | null {
  const k = key.trim().toLowerCase();
  const stem = trayPinKeyStem(k);
  if (stem === k) return null;
  const uid = k.slice(stem.length + 1);
  if (!/^\d+$/.test(uid)) return null;
  const n = Number(uid);
  return Number.isFinite(n) ? n : null;
}

function liveKeysWithStem(stem: string, live: string[]): string[] {
  return live.filter((k) => trayPinKeyStem(k) === stem);
}

/**
 * Match saved tray bind key to a live icon.
 * Multi-instance safe: same exe + different uid → NOT the same (dual WeChat).
 * Uid migration: when only one live icon shares the exe stem, allow uid change.
 */
export function trayKeysMatch(
  saved: string,
  live: string,
  liveTrayKeys?: Iterable<string>,
): boolean {
  const a = saved.trim();
  const b = live.trim();
  if (!a || !b) return false;
  if (a === b) return true;
  const stemA = trayPinKeyStem(a);
  const stemB = trayPinKeyStem(b);
  if (stemA !== stemB) return false;
  const uidA = trayPinKeyUid(a);
  const uidB = trayPinKeyUid(b);
  // Both keys carry uid → must match exactly (dual WeChat must not collapse).
  if (uidA != null && uidB != null) return uidA === uidB;
  const pool = [...(liveTrayKeys ?? [b])].map((k) => k.trim()).filter(Boolean);
  const sameStem = liveKeysWithStem(stemA, pool);
  // Stem-only / uid-migration: only when a single live instance shares the exe stem.
  if (sameStem.length === 1) return true;
  return false;
}

/** Tray gate: required key present among live tray keys (multi-instance aware). */
export function trayKeyPresent(
  required: string,
  liveTrayKeys: Iterable<string>,
): boolean {
  const req = required.trim();
  if (!req) return false;
  const live = [...liveTrayKeys].map((k) => k.trim()).filter(Boolean);
  if (live.includes(req)) return true;
  return live.some((k) => trayKeysMatch(req, k, live));
}

/** Pinned / 常显：saved key 与当前托盘 icon 是否同一实例。 */
export function isTrayPinnedKey(
  iconKey: string,
  iconId: string,
  pinned: Iterable<string>,
  liveTrayKeys?: Iterable<string>,
): boolean {
  const key = iconKey.trim();
  const id = iconId.trim();
  for (const p of pinned) {
    const raw = String(p || "").trim();
    if (!raw) continue;
    if (raw === key || raw === id) return true;
    if (key && trayKeysMatch(raw, key, liveTrayKeys)) return true;
    if (id && trayKeysMatch(raw, id, liveTrayKeys)) return true;
  }
  return false;
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
  for (const k of g.trayKeys) {
    if (!trayKeyPresent(k, liveTrayKeys)) return false;
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

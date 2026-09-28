/** Host-owned shortcuts bar prefs — exclusive + per-plugin app scope. */

import { windowKeyOf, type WindowKeySource } from "./scenarioGates";

export type ShortcutsScopeMode = "all" | "apps";

export type ShortcutsPluginScope = {
  /** `all` = every foreground; `apps` = only listed windowKeys. */
  mode: ShortcutsScopeMode;
  /** Same stable keys as scenarioGates (`exe:…` / `proc:…`). */
  windowKeys: string[];
};

export type ShortcutsSide = "left" | "right";

export type ShortcutsPrefs = {
  exclusivePluginId: string | null;
  pluginOrder: string[] | null;
  /** pluginId → scope. Missing → all programs. */
  scopes: Record<string, ShortcutsPluginScope>;
  /** pluginId → bar side. Missing → left. Right only valid in dual-shortcuts mode. */
  pluginSides: Record<string, ShortcutsSide>;
};

export function emptyPluginScope(): ShortcutsPluginScope {
  return { mode: "all", windowKeys: [] };
}

export function getPluginScope(
  scopes: Record<string, ShortcutsPluginScope> | null | undefined,
  pluginId: string,
): ShortcutsPluginScope {
  const id = pluginId.trim();
  const s = id ? scopes?.[id] : undefined;
  if (!s) return emptyPluginScope();
  const mode: ShortcutsScopeMode = s.mode === "apps" ? "apps" : "all";
  const windowKeys = Array.isArray(s.windowKeys)
    ? [...new Set(s.windowKeys.map((k) => String(k || "").trim()).filter(Boolean))]
    : [];
  return { mode, windowKeys };
}

export function parseScopes(raw: unknown): Record<string, ShortcutsPluginScope> {
  if (!raw || typeof raw !== "object") return {};
  const out: Record<string, ShortcutsPluginScope> = {};
  for (const [pid, val] of Object.entries(raw as Record<string, unknown>)) {
    const id = pid.trim();
    if (!id || !val || typeof val !== "object") continue;
    const v = val as Record<string, unknown>;
    const mode: ShortcutsScopeMode = v.mode === "apps" ? "apps" : "all";
    const windowKeys = Array.isArray(v.windowKeys)
      ? [...new Set(v.windowKeys.map((x) => String(x || "").trim()).filter(Boolean))]
      : [];
    // Persist only non-default scopes.
    if (mode === "all" && windowKeys.length === 0) continue;
    out[id] = { mode, windowKeys };
  }
  return out;
}

export function upsertPluginScope(
  scopes: Record<string, ShortcutsPluginScope>,
  pluginId: string,
  next: ShortcutsPluginScope,
): Record<string, ShortcutsPluginScope> {
  const id = pluginId.trim();
  if (!id) return scopes;
  const mode: ShortcutsScopeMode = next.mode === "apps" ? "apps" : "all";
  const windowKeys = [
    ...new Set(next.windowKeys.map((k) => k.trim()).filter(Boolean)),
  ];
  const out = { ...scopes };
  if (mode === "all" && windowKeys.length === 0) {
    delete out[id];
  } else {
    out[id] = { mode, windowKeys };
  }
  return out;
}

export function parsePluginSides(raw: unknown): Record<string, ShortcutsSide> {
  if (!raw || typeof raw !== "object") return {};
  const out: Record<string, ShortcutsSide> = {};
  for (const [pid, val] of Object.entries(raw as Record<string, unknown>)) {
    const id = pid.trim();
    if (!id) continue;
    if (val === "right") out[id] = "right";
    else if (val === "left") {
      // omit defaults to keep patch payloads small
    }
  }
  return out;
}

export function getPluginSide(
  sides: Record<string, ShortcutsSide> | null | undefined,
  pluginId: string,
): ShortcutsSide {
  const id = pluginId.trim();
  return id && sides?.[id] === "right" ? "right" : "left";
}

export function upsertPluginSide(
  sides: Record<string, ShortcutsSide>,
  pluginId: string,
  side: ShortcutsSide,
): Record<string, ShortcutsSide> {
  const id = pluginId.trim();
  if (!id) return sides;
  const out = { ...sides };
  if (side === "left") delete out[id];
  else out[id] = "right";
  return out;
}

/** When enabling a plugin: keep right only if dual shortcuts is active. */
export function resolvePluginSideOnEnable(
  sides: Record<string, ShortcutsSide>,
  pluginId: string,
  dualShortcuts: boolean,
): { sides: Record<string, ShortcutsSide>; side: ShortcutsSide; changed: boolean } {
  const current = getPluginSide(sides, pluginId);
  if (current === "right" && !dualShortcuts) {
    return { sides: upsertPluginSide(sides, pluginId, "left"), side: "left", changed: true };
  }
  return { sides, side: current, changed: false };
}

function exeStemFromKey(key: string): string {
  const k = key.trim().toLowerCase();
  if (k.startsWith("proc:")) {
    return k.slice(5).replace(/\.exe$/i, "");
  }
  if (k.startsWith("exe:")) {
    const path = k.slice(4).replace(/\//g, "\\");
    const base = path.split("\\").pop() || path;
    return base.replace(/\.exe$/i, "");
  }
  return "";
}

function foregroundStem(fg: WindowKeySource): string {
  const name = String(fg.exe_name || fg.exeName || "")
    .trim()
    .toLowerCase()
    .replace(/\.exe$/i, "");
  if (name) return name;
  const exe = String(fg.exe || "")
    .trim()
    .replace(/\//g, "\\")
    .toLowerCase();
  if (!exe) return "";
  const base = exe.split("\\").pop() || exe;
  return base.replace(/\.exe$/i, "");
}

/** True when stored picker keys match the current foreground app. */
export function windowKeysMatchForeground(
  windowKeys: string[],
  fg: WindowKeySource | null | undefined,
): boolean {
  if (!windowKeys.length || !fg) return false;
  const direct = windowKeyOf(fg);
  if (direct && windowKeys.includes(direct)) return true;
  const stem = foregroundStem(fg);
  if (!stem) return false;
  for (const key of windowKeys) {
    if (exeStemFromKey(key) === stem) return true;
  }
  return false;
}

/**
 * Whether this shortcuts plugin strip should be visible for the current foreground.
 * Island bar workers should bypass this (always mounted).
 */
export function shortcutsScopeVisible(
  scopes: Record<string, ShortcutsPluginScope> | null | undefined,
  pluginId: string,
  fg: WindowKeySource | null | undefined,
): boolean {
  const scope = getPluginScope(scopes, pluginId);
  if (scope.mode !== "apps") return true;
  if (!scope.windowKeys.length) return false;
  return windowKeysMatchForeground(scope.windowKeys, fg);
}

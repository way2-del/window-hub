/**
 * Scene / montage model for Bloub popup editor + shortcuts playback.
 */
import { clampDuration, makeBlock, type Block } from "../vendor/bot/cycles";
import type { StateId } from "../vendor/bot/states";
import { expressionLabel } from "./appearance";
import { blocksForPreset, type CyclePresetId } from "./cycle-presets";

export type MontageBlock = Block & {
  /** Rest-face expression applied when this block starts (best on idle / baseFace). */
  expression?: string;
};

export type Scene = {
  id: string;
  name: string;
  builtin?: boolean;
  blocks: MontageBlock[];
};

export const STATE_LABELS: Partial<Record<StateId, string>> = {
  idle: "待机",
  thinking: "思考",
  wink: "眨眼",
  wide: "睁大",
  alert: "警觉",
  notify: "通知",
  exclaim: "感叹",
  sleep: "睡眠",
  egg: "蛋形",
  hexagon: "六边",
  play: "播放",
  orbit: "环绕",
  burst: "爆发",
  comet: "彗星",
};

export const CATALOG_STATES: StateId[] = [
  "idle",
  "thinking",
  "wink",
  "wide",
  "alert",
  "notify",
  "exclaim",
  "sleep",
  "egg",
  "hexagon",
  "play",
  "orbit",
  "burst",
  "comet",
];

const BUILTIN: Array<{ id: CyclePresetId | string; name: string }> = [
  { id: "idle", name: "待机" },
  { id: "face", name: "表情向" },
  { id: "show", name: "展示" },
  { id: "calm", name: "安静" },
];

export function builtinScenes(): Scene[] {
  return BUILTIN.map((b) => ({
    id: b.id,
    name: b.name,
    builtin: true,
    blocks: blocksForPreset(b.id).map((block) => ({ ...block })),
  }));
}

export function holdBlock(state: StateId, seconds: number, expression?: string): MontageBlock {
  const b: MontageBlock = { state, duration: clampDuration(state, seconds) };
  if (expression) b.expression = expression;
  return b;
}

export function exprBlock(expression: string, seconds = 2.5): MontageBlock {
  return holdBlock("idle", seconds, expression);
}

export function animBlock(state: StateId): MontageBlock {
  return { ...makeBlock(state) };
}

export function totalSeconds(blocks: MontageBlock[]): number {
  return blocks.reduce((s, b) => s + b.duration, 0);
}

export function labelBlock(b: MontageBlock): string {
  const st = STATE_LABELS[b.state] || b.state;
  const expr = expressionLabel(b.expression);
  if (expr && b.state === "idle") return expr;
  if (expr) return `${st}/${expr}`;
  return st;
}

export function normalizeBlocks(raw: unknown): MontageBlock[] | null {
  if (!Array.isArray(raw) || !raw.length) return null;
  const out: MontageBlock[] = [];
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const state = String((item as MontageBlock).state || "") as StateId;
    if (!CATALOG_STATES.includes(state)) continue;
    const duration = Number((item as MontageBlock).duration);
    const expression =
      typeof (item as MontageBlock).expression === "string"
        ? (item as MontageBlock).expression
        : undefined;
    out.push({
      state,
      duration: clampDuration(state, Number.isFinite(duration) ? duration : 2),
      ...(expression ? { expression } : {}),
    });
  }
  return out.length ? out : null;
}

export function normalizeScenes(raw: unknown): Scene[] {
  if (!Array.isArray(raw)) return [];
  const out: Scene[] = [];
  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const id = String((item as Scene).id || "").trim();
    const name = String((item as Scene).name || "").trim() || id;
    if (!id || (item as Scene).builtin) continue;
    const blocks = normalizeBlocks((item as Scene).blocks);
    if (!blocks) continue;
    out.push({ id, name, blocks });
  }
  return out;
}

export function newSceneId(): string {
  return `s_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`;
}

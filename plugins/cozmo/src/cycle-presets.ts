/**
 * Shortcuts / popup cycle presets built on bloub Block montages.
 * Bar-sized chips should prefer face/idle — SEQUENCE includes play/egg/hex
 * silhouettes that read as random triangles at ~28px.
 */
import { clampDuration, defaultCycle, makeBlock, type Block } from "../vendor/bot/cycles";
import type { StateId } from "../vendor/bot/states";

export type CyclePresetId = "idle" | "face" | "show" | "calm" | "default";

const FACE: StateId[] = ["idle", "wink", "wide", "sleep", "idle", "wink"];
const SHOW: StateId[] = ["idle", "wink", "wide", "orbit", "burst", "comet"];
const CALM: StateId[] = ["idle", "sleep", "idle"];

function hold(state: StateId, seconds: number): Block {
  return { state, duration: clampDuration(state, seconds) };
}

export function blocksForPreset(id: string | undefined | null): Block[] {
  switch (id) {
    case "idle":
      // Liveliness only (blink + gaze drift) — clearest at bar size
      return [hold("idle", 8)];
    case "face":
      return FACE.map((s) => (s === "idle" ? hold(s, 3.5) : makeBlock(s)));
    case "show":
      return SHOW.map(makeBlock);
    case "calm":
      return CALM.map((s) => hold(s, s === "idle" ? 4 : 2.5));
    case "default":
      return defaultCycle().blocks;
    default:
      // Unknown / legacy → face (safe for shortcuts)
      return FACE.map((s) => (s === "idle" ? hold(s, 3.5) : makeBlock(s)));
  }
}

export const CYCLE_PRESET_OPTIONS: Array<{ value: CyclePresetId; label: string }> = [
  { value: "idle", label: "待机：眨眼 / 目光漂移" },
  { value: "face", label: "表情向：眨眼 / 睁大 / 睡眠" },
  { value: "show", label: "展示：环绕 / 爆发 / 彗星" },
  { value: "calm", label: "安静：睡眠" },
  { value: "default", label: "完整 SEQUENCE（含三角/感叹号，小尺寸慎用）" },
];

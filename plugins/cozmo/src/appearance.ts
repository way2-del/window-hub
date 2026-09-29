/**
 * Bloub appearance: shape / color / rest expression catalogues.
 */
import {
  COLOR_BY_ID,
  COLORS,
  DEFAULT_COLOR,
  DEFAULT_SHAPE,
  SHAPE_BY_ID,
  SHAPES,
  type ColorId,
  type ShapeId,
} from "../vendor/bot/skins";
import { DEFAULT_EXPRESSION, EXPRESSION_BY_ID } from "../vendor/bot/expressions";

export {
  SHAPES,
  COLORS,
  SHAPE_BY_ID,
  COLOR_BY_ID,
  DEFAULT_SHAPE,
  DEFAULT_COLOR,
  DEFAULT_EXPRESSION,
};

export const SHAPE_LABELS: Record<ShapeId, string> = {
  cercle: "圆",
  galet: "卵石",
  squircle: "方圆",
  capsule: "胶囊",
  triangle: "三角",
  hexagone: "六边",
  nuage: "云",
  goutte: "水滴",
};

export const COLOR_LABELS: Record<ColorId, string> = {
  encre: "墨",
  creme: "奶油",
  brun: "棕",
  rouge: "红",
  orange: "橙",
  ambre: "琥珀",
  vert: "绿",
  turquoise: "青",
  bleu: "蓝",
  violet: "紫",
  rose: "粉",
  gris: "灰",
};

export const EXPRESSION_LABELS: Record<string, string> = {
  neutre: "中性",
  attentif: "专注",
  surpris: "惊讶",
  excite: "兴奋",
  heureux: "开心",
  hilare: "大笑",
  colere: "愤怒",
  triste: "难过",
  effraye: "害怕",
  mefiant: "怀疑",
  confus: "困惑",
  curieux: "好奇",
  fier: "得意",
  timide: "害羞",
  blase: "无语",
  somnolent: "困倦",
  musique: "听歌",
};

export function expressionLabel(id: string | undefined | null): string {
  if (!id) return "";
  return EXPRESSION_LABELS[id] || id;
}

export type Appearance = {
  shapeId: string;
  colorId: string;
  expressionId: string;
};

export function defaultAppearance(): Appearance {
  return {
    shapeId: DEFAULT_SHAPE,
    colorId: DEFAULT_COLOR,
    expressionId: DEFAULT_EXPRESSION,
  };
}

export function normalizeAppearance(raw: unknown): Appearance {
  const d = defaultAppearance();
  if (!raw || typeof raw !== "object") return d;
  const o = raw as Record<string, unknown>;
  const shapeId = String(o.shapeId || d.shapeId);
  const colorId = String(o.colorId || d.colorId);
  const expressionId = String(o.expressionId || d.expressionId);
  return {
    shapeId: SHAPE_BY_ID.has(shapeId) ? shapeId : d.shapeId,
    colorId: COLOR_BY_ID.has(colorId) ? colorId : d.colorId,
    expressionId: EXPRESSION_BY_ID.has(expressionId) ? expressionId : d.expressionId,
  };
}

export function inkOf(colorId: string): string {
  return COLOR_BY_ID.get(colorId)?.hex ?? "#0a0a0c";
}

export function paperOf(colorId: string): string {
  const hex = inkOf(colorId);
  const m = hex.match(/^#?([0-9a-f]{6})$/i);
  if (!m) return "#ffffff";
  const v = parseInt(m[1]!, 16);
  const r = (v >> 16) & 255;
  const g = (v >> 8) & 255;
  const b = v & 255;
  const lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  return lum > 0.55 ? "#0a0a0c" : "#ffffff";
}

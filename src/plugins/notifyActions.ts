/** Island notify action buttons — Host-owned layout & validation. */

export type NotifyActionSlot = "start" | "end";

export type NotifyActionInput = {
  id: string;
  /** Only `start` (leading) or `end` (trailing). */
  slot: NotifyActionSlot;
  /**
   * Exactly 2 characters (Unicode code points), XOR with `iconPng`.
   * e.g. "完成" / "稍后"
   */
  label?: string;
  /** Small PNG base64 (no data: prefix). XOR with `label`. */
  iconPng?: string;
  /** Button background (CSS color). */
  background: string;
  /** Opaque payload echoed on click. */
  data?: unknown;
};

export type NotifyAction = {
  id: string;
  slot: NotifyActionSlot;
  label?: string;
  iconPng?: string;
  background: string;
  data?: unknown;
};

export type NotifyActionEvent = {
  pluginId: string;
  notifyId: string;
  actionId: string;
  data?: unknown;
};

const MAX_PER_SLOT = 1;

function codePointLen(s: string): number {
  return Array.from(s).length;
}

/** Reject CSS injection; allow common color forms. */
export function isSafeCssColor(raw: string): boolean {
  const s = raw.trim();
  if (!s || s.length > 64) return false;
  if (/[;{}<>]|url\s*\(/i.test(s)) return false;
  return (
    /^#([0-9a-f]{3}|[0-9a-f]{4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(s) ||
    /^rgba?\(\s*[\d.]+%?\s*,\s*[\d.]+%?\s*,\s*[\d.]+%?\s*(,\s*[\d.]+\s*)?\)$/i.test(s) ||
    /^hsla?\(\s*[\d.]+%?\s*,\s*[\d.]+%?\s*,\s*[\d.]+%?\s*(,\s*[\d.]+\s*)?\)$/i.test(s)
  );
}

/**
 * Normalize plugin actions for Host UI.
 * - slot ∈ {start,end}; at most one button per slot
 * - content = exactly 2-char label XOR iconPng
 * - background required & sanitized
 */
export function normalizeNotifyActions(raw: unknown): NotifyAction[] {
  if (!Array.isArray(raw)) return [];
  const bySlot: Partial<Record<NotifyActionSlot, NotifyAction>> = {};

  for (const item of raw) {
    if (!item || typeof item !== "object") continue;
    const o = item as Record<string, unknown>;
    const id = typeof o.id === "string" ? o.id.trim() : "";
    const slot = o.slot === "start" || o.slot === "end" ? o.slot : null;
    const background = typeof o.background === "string" ? o.background.trim() : "";
    if (!id || !slot || !isSafeCssColor(background)) continue;
    if (bySlot[slot]) continue;

    const iconPng =
      typeof o.iconPng === "string" && o.iconPng.trim() ? o.iconPng.trim() : undefined;
    const labelRaw = typeof o.label === "string" ? o.label.trim() : "";
    const label = labelRaw && !iconPng && codePointLen(labelRaw) === 2 ? labelRaw : undefined;
    if (!iconPng && !label) continue;

    bySlot[slot] = {
      id,
      slot,
      label,
      iconPng,
      background,
      data: "data" in o ? o.data : undefined,
    };
    if (Object.keys(bySlot).length >= MAX_PER_SLOT * 2) break;
  }

  const out: NotifyAction[] = [];
  if (bySlot.start) out.push(bySlot.start);
  if (bySlot.end) out.push(bySlot.end);
  return out;
}

export function actionsForSlot(
  actions: NotifyAction[] | undefined,
  slot: NotifyActionSlot,
): NotifyAction | undefined {
  return actions?.find((a) => a.slot === slot);
}

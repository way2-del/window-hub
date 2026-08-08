/** Staging summary types + helpers (plugin-agnostic). */

export type StagingSummary = {
  files: number;
  texts: number;
  images: number;
  total: number;
};

export type StagingChangedPayload = {
  pluginId?: string;
  plugin_id?: string;
  summary?: StagingSummary;
  files?: number;
  texts?: number;
  images?: number;
  total?: number;
};

export function emptyStagingSummary(): StagingSummary {
  return { files: 0, texts: 0, images: 0, total: 0 };
}

export function normalizeStagingChanged(
  payload: StagingChangedPayload | null | undefined,
): { pluginId: string | null; summary: StagingSummary } {
  if (!payload) return { pluginId: null, summary: emptyStagingSummary() };
  const pluginId = payload.pluginId ?? payload.plugin_id ?? null;
  if (payload.summary) {
    return { pluginId, summary: payload.summary };
  }
  if (typeof payload.total === "number") {
    return {
      pluginId,
      summary: {
        files: payload.files ?? 0,
        texts: payload.texts ?? 0,
        images: payload.images ?? 0,
        total: payload.total,
      },
    };
  }
  return { pluginId, summary: emptyStagingSummary() };
}

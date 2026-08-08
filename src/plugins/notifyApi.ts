import { emit } from "@tauri-apps/api/event";
import { assertCapability } from "./capGate";
import type { PluginManifest } from "./types";
import type { NotifyUrgency } from "./islandNotify";

export type HubNotifyArgs = {
  title: string;
  body?: string;
  iconPng?: string;
  urgency?: NotifyUrgency;
  ttlMs?: number;
  actions?: { id: string; label: string }[];
};

/** Plugin-facing notify — CapGate + event into main island. */
export async function hubNotify(
  manifest: PluginManifest,
  args: HubNotifyArgs,
): Promise<{ id: string }> {
  assertCapability(manifest, "notify");
  const maxPerMinute = manifest.slots?.["island.notify"]?.maxPerMinute ?? 6;
  const id = `req-${Date.now()}`;
  await emit("island-notify", {
    id,
    pluginId: manifest.id,
    maxPerMinute,
    title: args.title,
    body: args.body,
    iconPng: args.iconPng,
    urgency: args.urgency ?? "active",
    ttlMs: args.ttlMs,
    actions: args.actions,
  });
  return { id };
}

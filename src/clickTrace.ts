import { invoke } from "@tauri-apps/api/core";

/** HTTP sink — bypasses Tauri IPC so traces still land when the UI pump is dead. */
const TRACE_HTTP = "http://127.0.0.1:38765/t";

/**
 * Append to `%TEMP%/window-hub-click-trace.log`.
 * Prefer HTTP (non-IPC); also fire-and-forget invoke as backup.
 */
export function clickTrace(origin: string, msg: string): void {
  const line = `${origin}: ${msg}`;
  console.info("[click-trace]", line);
  // HTTP first — Chromium network stack is off the UI pump.
  try {
    void fetch(TRACE_HTTP, {
      method: "POST",
      mode: "cors",
      keepalive: true,
      headers: { "Content-Type": "text/plain" },
      body: `${origin}|${msg}`,
    }).catch(() => undefined);
  } catch {
    /* noop */
  }
  // Backup via IPC (may stall if already deadlocked — fire-and-forget).
  void invoke("debug_click_trace", { origin, msg }).catch(() => undefined);
}

export async function clickTracePath(): Promise<string> {
  try {
    return await invoke<string>("debug_click_trace_path");
  } catch {
    return "";
  }
}

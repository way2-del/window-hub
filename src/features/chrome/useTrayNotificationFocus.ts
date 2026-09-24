import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** Subscribe only while a tray banner is visible; no frontend polling. */
export function useTrayNotificationFocus(
  id: string | undefined,
  hwnd: number | undefined,
  dismiss: () => void,
) {
  const currentRef = useRef({ id, hwnd, dismiss });
  currentRef.current = { id, hwnd, dismiss };
  useEffect(() => {
    if (!id || !hwnd) return;
    const token = crypto.randomUUID();
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void (async () => {
      try {
        const stop = await listen<string>("tray-notification-viewed", ({ payload }) => {
          const current = currentRef.current;
          if (!cancelled && payload === token && current.id === id && current.hwnd === hwnd) {
            current.dismiss();
          }
        });
        if (cancelled) { stop(); return; }
        unlisten = stop;
        await invoke("watch_tray_notification", { token, hwnd });
        if (cancelled) await invoke("watch_tray_notification", { token, hwnd: null });
      } catch { /* Unsupported/invalid owner: retain the banner. */ }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
      void invoke("watch_tray_notification", { token, hwnd: null }).catch(() => undefined);
    };
  }, [id, hwnd]);
}

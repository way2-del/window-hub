import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/**
 * Glass page stays fully clear — SWCA + DWM corner preference paint the bar.
 * (CSS radius cannot round SWCA; Host owns the frost shape.)
 */
export default function DockGlassApp() {
  const [, setRadius] = useState(12);

  useEffect(() => {
    let cancelled = false;
    void invoke<{ cornerRadiusPx?: number }>("get_dock_prefs")
      .then((p) => {
        if (!cancelled && Number.isFinite(p?.cornerRadiusPx)) {
          setRadius(Math.min(28, Math.max(0, Number(p.cornerRadiusPx))));
        }
      })
      .catch(() => undefined);
    const unsubs: Array<() => void> = [];
    void listen<{ cornerRadiusPx?: number }>("dock-prefs", (e) => {
      const n = Number(e.payload?.cornerRadiusPx);
      if (Number.isFinite(n)) setRadius(Math.min(28, Math.max(0, n)));
    }).then((u) => unsubs.push(u));
    return () => {
      cancelled = true;
      for (const u of unsubs) u();
    };
  }, []);

  return (
    <div
      aria-hidden
      style={{
        boxSizing: "border-box",
        width: "100%",
        height: "100%",
        margin: 0,
        padding: 0,
        border: "none",
        borderRadius: 0,
        background: "transparent",
        boxShadow: "none",
        pointerEvents: "none",
      }}
    />
  );
}

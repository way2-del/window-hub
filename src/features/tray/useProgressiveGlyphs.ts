import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { createGlyphQueue } from "./glyphQueue";

/** Shared by the rail and tray popup; each surface requests only missing glyphs. */
export function useProgressiveGlyphs(ids: string[], onBatch: (map: Record<string, string>) => void) {
  const callback = useRef(onBatch);
  callback.current = onBatch;
  const queue = useRef<ReturnType<typeof createGlyphQueue> | null>(null);
  useEffect(() => {
    const worker = createGlyphQueue(
      ids => invoke<Record<string, string>>("get_tray_icon_glyphs", { ids }),
      map => callback.current(map),
    );
    queue.current = worker;
    return () => { worker.dispose(); queue.current = null; };
  }, []);
  const key = JSON.stringify(ids);
  useEffect(() => { queue.current?.update(JSON.parse(key) as string[]); }, [key]);
}

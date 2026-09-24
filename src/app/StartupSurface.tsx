import { useEffect, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";

/** Acknowledge a committed React tree after a paint opportunity, not module load. */
export function StartupSurface({ children }: { children: ReactNode }) {
  useEffect(() => {
    let second = 0;
    const first = requestAnimationFrame(() => {
      second = requestAnimationFrame(() => {
        void invoke("startup_surface_ready").catch(console.error);
      });
    });
    return () => {
      cancelAnimationFrame(first);
      cancelAnimationFrame(second);
    };
  }, []);
  return children;
}

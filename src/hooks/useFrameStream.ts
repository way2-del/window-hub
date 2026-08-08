import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import type { FrameEvent } from "../types";

/** Returns object-URL map for slot -> jpeg preview; revokes old URLs. */
export function useFrameStream() {
  const [urls, setUrls] = useState<Record<number, string>>({});
  const urlsRef = useRef<Record<number, string>>({});

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    listen<FrameEvent>("frame", (event) => {
      const { slot, jpeg_base64 } = event.payload;
      const binary = atob(jpeg_base64);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
      const blob = new Blob([bytes], { type: "image/jpeg" });
      const url = URL.createObjectURL(blob);

      const prev = urlsRef.current[slot];
      if (prev) URL.revokeObjectURL(prev);

      const next = { ...urlsRef.current, [slot]: url };
      urlsRef.current = next;
      setUrls(next);
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
      Object.values(urlsRef.current).forEach((u) => URL.revokeObjectURL(u));
    };
  }, []);

  return urls;
}

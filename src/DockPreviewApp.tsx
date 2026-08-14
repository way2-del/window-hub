import { useEffect, useRef, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import "./DockPreviewApp.css";

type DockPreviewFrame = {
  hwnd: number;
  title: string;
  jpegBase64: string;
  width: number;
  height: number;
};

type Payload = {
  itemId: string;
  label: string;
  frames: DockPreviewFrame[];
  iconPng?: string | null;
};

async function closeSelf() {
  try {
    await invoke("close_dock_preview");
  } catch {
    try {
      await getCurrentWindow().hide();
    } catch {
      /* noop */
    }
  }
}

function thumbStyle(frame: DockPreviewFrame): CSSProperties {
  const maxW = 168;
  const maxH = 100;
  const w = Math.max(1, frame.width);
  const h = Math.max(1, frame.height);
  const scale = Math.min(maxW / w, maxH / h, 1);
  return {
    width: Math.round(w * scale),
    height: Math.round(h * scale),
  };
}

export default function DockPreviewApp() {
  const [payload, setPayload] = useState<Payload | null>(null);
  const itemIdRef = useRef<string | null>(null);
  const refreshRef = useRef(0);

  useEffect(() => {
    void (async () => {
      try {
        const prefs = await invoke<GlassPrefs>("get_material_prefs");
        await syncGlassCss({
          kind: "mica-alt",
          dark: prefs.dark === false ? false : true,
          acrylicAlpha: prefs.acrylicAlpha,
        });
      } catch {
        await syncGlassCss({ kind: "mica-alt", dark: true });
      }
      await invoke("apply_window_effect", {}).catch(() => undefined);
    })();

    const unsubs: Array<() => void> = [];
    let cancelled = false;

    void listen<Payload>("dock-preview", (e) => {
      if (cancelled) return;
      setPayload(e.payload);
      itemIdRef.current = e.payload.itemId;
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    void listen("dock-preview-closed", () => {
      if (!cancelled) {
        setPayload(null);
        itemIdRef.current = null;
      }
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    unsubs.push(
      subscribeSystemDark(() => {
        void syncGlassCss({ kind: "mica-alt", dark: true });
      }),
    );

    // JPEG fallback only — DWM live thumbs don't need fast polling.
    refreshRef.current = window.setInterval(() => {
      const id = itemIdRef.current;
      if (!id) return;
      void (async () => {
        try {
          const frames = await invoke<DockPreviewFrame[]>("dock_capture_item_previews", {
            itemId: id,
          });
          if (!frames.length) return;
          setPayload((prev) => {
            if (!prev || prev.itemId !== id) return prev;
            const hadJpeg = prev.frames.some((f) => f.jpegBase64);
            if (!hadJpeg && !frames.some((f) => f.jpegBase64)) return prev;
            const merged = frames.map((f, i) => {
              if (f.jpegBase64) return f;
              const old = prev.frames.find((p) => p.hwnd === f.hwnd) ?? prev.frames[i];
              if (old?.jpegBase64) {
                return { ...f, jpegBase64: old.jpegBase64, width: old.width, height: old.height };
              }
              return f;
            });
            return { ...prev, frames: merged };
          });
        } catch {
          /* noop */
        }
      })();
    }, 2500);

    return () => {
      cancelled = true;
      window.clearInterval(refreshRef.current);
      unsubs.forEach((fn) => fn());
    };
  }, []);

  if (!payload || payload.frames.length === 0) {
    return <div className="dock-preview-shell dock-preview-loading" aria-hidden />;
  }

  return (
    <div
      className="dock-preview-shell"
      onPointerEnter={() => {
        void emit("dock-preview-pointer", { inside: true });
      }}
      onPointerLeave={() => {
        void emit("dock-preview-pointer", { inside: false });
      }}
    >
      <div className="dock-preview-row">
        {payload.frames.map((f) => (
          <button
            key={f.hwnd}
            type="button"
            className="dock-preview-card"
            title={f.title || payload.label}
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => {
              void (async () => {
                try {
                  await invoke("focus_open_window", { id: `hwnd:${f.hwnd}` });
                } catch (e) {
                  console.error(e);
                } finally {
                  await closeSelf();
                }
              })();
            }}
          >
            <span className={`dock-preview-thumb-wrap${f.jpegBase64 ? "" : " is-live"}`}>
              {f.jpegBase64 ? (
                <img
                  className="dock-preview-thumb"
                  style={thumbStyle(f)}
                  src={`data:image/jpeg;base64,${f.jpegBase64}`}
                  alt=""
                  draggable={false}
                />
              ) : (
                <span className="dock-preview-live-slot" aria-hidden />
              )}
            </span>
            <span className="dock-preview-title">{f.title || payload.label || "窗口"}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { resolveShortcutsStagingPluginId } from "../plugins/islandSlots";
import { SHORTCUTS_HEIGHT } from "../plugins/shortcutsGeometry";
import "./StagingCatcher.css";

/** Extra strip below top bar while system file-drag is active (same HWND). */
export const STAGING_CATCHER_EXTRA = 72;

type Props = {
  onActiveChange?: (active: boolean) => void;
};

function catcherAnchor(): { left: number; width: number } | null {
  const stagingId = resolveShortcutsStagingPluginId();
  const el = stagingId
    ? document.querySelector<HTMLElement>(
        `.shortcuts-plugin-strip[data-plugin="${CSS.escape(stagingId)}"]`,
      )
    : null;
  const fallback = document.querySelector<HTMLElement>(
    ".shortcuts-plugin-strip[data-plugin], .shortcuts-host",
  );
  const target = el ?? fallback;
  if (!target) return null;
  const r = target.getBoundingClientRect();
  const width = Math.max(148, Math.min(220, Math.max(r.width + 100, 148)));
  const left = Math.max(8, r.left + r.width / 2 - width / 2);
  return { left, width };
}

function hitCatcher(
  lx: number,
  ly: number,
  pad = 8,
): DOMRect | null {
  const node = document.querySelector(".staging-catcher");
  if (!node) return null;
  const r = node.getBoundingClientRect();
  const inside =
    lx >= r.left - pad &&
    lx <= r.right + pad &&
    ly >= r.top - pad &&
    ly <= r.bottom + pad;
  return inside ? r : null;
}

async function openStagingPopup(pluginId: string) {
  const win = getCurrentWindow();
  const factor = await win.scaleFactor();
  const chip =
    document.querySelector<HTMLElement>(
      `.shortcuts-plugin-strip[data-plugin="${CSS.escape(pluginId)}"]`,
    ) ?? document.querySelector<HTMLElement>(".shortcuts-host");
  if (!chip) return;
  const outer = await win.outerPosition();
  const rect = chip.getBoundingClientRect();
  const x = outer.x / factor + rect.left;
  const y = outer.y / factor + rect.bottom + 8;
  void invoke("suppress_plugin_popup_blur", { ms: 280 }).catch(() => undefined);
  await invoke("open_plugin_popup", {
    pluginId,
    x,
    y,
    forceOpen: true,
  }).catch(console.error);
}

/**
 * System file-drag catcher: mini drop zone inside the main top-bar HWND.
 * Host raises window height; this component never opens a new popup mid-drag.
 * Drop → staging ingest + open plugin popup after drop completes.
 */
export default function StagingCatcher({ onActiveChange }: Props) {
  const [active, setActive] = useState(false);
  const [hot, setHot] = useState(false);
  const [anchor, setAnchor] = useState<{ left: number; width: number } | null>(null);
  const activeRef = useRef(false);

  useEffect(() => {
    onActiveChange?.(active);
  }, [active, onActiveChange]);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    async function show() {
      const pluginId = resolveShortcutsStagingPluginId();
      if (!pluginId) return;
      try {
        const popupOpen = await invoke<boolean>("is_plugin_popup_open");
        const popupId = await invoke<string | null>("get_plugin_popup_id");
        if (popupOpen && popupId === pluginId) return;
      } catch {
        /* continue */
      }
      if (cancelled || activeRef.current) return;
      activeRef.current = true;
      setActive(true);
      setAnchor(catcherAnchor());
      document.documentElement.classList.add("is-staging-drag");
      document
        .querySelectorAll(".shortcuts-plugin-strip.is-staging-drop-target")
        .forEach((n) => n.classList.remove("is-staging-drop-target"));
      document
        .querySelector(
          `.shortcuts-plugin-strip[data-plugin="${CSS.escape(pluginId)}"]`,
        )
        ?.classList.add("is-staging-drop-target");
    }

    function hide() {
      activeRef.current = false;
      setActive(false);
      setHot(false);
      setAnchor(null);
      document.documentElement.classList.remove("is-staging-drag");
      document
        .querySelectorAll(".shortcuts-plugin-strip.is-staging-drop-target")
        .forEach((n) => n.classList.remove("is-staging-drop-target"));
    }

    void listen("system-drag-start", () => {
      void show();
    }).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });
    void listen("system-drag-end", () => {
      hide();
    }).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });

    return () => {
      cancelled = true;
      unsubs.forEach((fn) => fn());
      document.documentElement.classList.remove("is-staging-drag");
    };
  }, []);

  /** HTML5：不 preventDefault 时系统显示禁止光标（与 PluginPopupHost / Sousou 同款） */
  useEffect(() => {
    if (!active) return;
    const allow = (e: DragEvent) => {
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
      const node = document.querySelector(".staging-catcher");
      if (!node) return;
      const r = node.getBoundingClientRect();
      setHot(
        e.clientX >= r.left - 4 &&
          e.clientX <= r.right + 4 &&
          e.clientY >= r.top - 4 &&
          e.clientY <= r.bottom + 4,
      );
    };
    const onLeave = (e: DragEvent) => {
      if (e.relatedTarget) return;
      setHot(false);
    };
    document.addEventListener("dragenter", allow);
    document.addEventListener("dragover", allow);
    document.addEventListener("dragleave", onLeave);
    return () => {
      document.removeEventListener("dragenter", allow);
      document.removeEventListener("dragover", allow);
      document.removeEventListener("dragleave", onLeave);
    };
  }, [active]);

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    const unFns: Array<() => void> = [];
    const pluginId = resolveShortcutsStagingPluginId();
    if (!pluginId) return;

    let dropBusy = false;

    const finishHide = () => {
      activeRef.current = false;
      setActive(false);
      setHot(false);
      setAnchor(null);
      document.documentElement.classList.remove("is-staging-drag");
      document
        .querySelectorAll(".shortcuts-plugin-strip.is-staging-drop-target")
        .forEach((n) => n.classList.remove("is-staging-drop-target"));
    };

    const ingestAndOpen = async (paths: string[]) => {
      if (dropBusy) return;
      dropBusy = true;
      try {
        if (paths.length) {
          await invoke("hub_staging_add_paths", { pluginId, paths }).catch(
            console.error,
          );
        }
        await openStagingPopup(pluginId);
        finishHide();
      } finally {
        dropBusy = false;
      }
    };

    const onDragDrop = (ev: {
      payload: {
        type: string;
        paths?: string[];
        position?: { x: number; y: number };
      };
    }) => {
      const p = ev.payload;
      if (p.type === "leave") {
        setHot(false);
        return;
      }
      if (p.type === "enter" || p.type === "over") {
        void (async () => {
          const win = getCurrentWindow();
          const factor = await win.scaleFactor();
          const pos = p.position;
          if (!pos) {
            setHot(true);
            return;
          }
          setHot(!!hitCatcher(pos.x / factor, pos.y / factor, 4));
        })();
        return;
      }
      if (p.type !== "drop") return;

      void (async () => {
        const win = getCurrentWindow();
        const factor = await win.scaleFactor();
        const pos = p.position;
        if (!pos) return;
        if (!hitCatcher(pos.x / factor, pos.y / factor, 8)) return;
        await ingestAndOpen(p.paths ?? []);
      })();
    };

    const bind = (label: string, promise: Promise<() => void>) => {
      void promise
        .then((fn) => {
          if (cancelled) {
            fn();
            return;
          }
          unFns.push(fn);
        })
        .catch((err) => {
          console.error(`[StagingCatcher] onDragDropEvent (${label}) unavailable`, err);
        });
    };

    bind("window", getCurrentWindow().onDragDropEvent(onDragDrop));
    bind("webview", getCurrentWebview().onDragDropEvent(onDragDrop));

    const onHtml5Drop = (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const node = document.querySelector(".staging-catcher");
      if (!node) return;
      const r = node.getBoundingClientRect();
      const inside =
        e.clientX >= r.left - 8 &&
        e.clientX <= r.right + 8 &&
        e.clientY >= r.top - 8 &&
        e.clientY <= r.bottom + 8;
      if (!inside) return;

      const files = e.dataTransfer?.files;
      const paths: string[] = [];
      if (files?.length) {
        for (let i = 0; i < files.length; i++) {
          const f = files.item(i) as File & { path?: string };
          if (f?.path?.trim()) paths.push(f.path);
        }
      }
      void ingestAndOpen(paths);
    };
    document.addEventListener("drop", onHtml5Drop);

    return () => {
      cancelled = true;
      unFns.forEach((fn) => fn());
      document.removeEventListener("drop", onHtml5Drop);
    };
  }, [active]);

  if (!active || !anchor) return null;

  return (
    <div
      className={`staging-catcher${hot ? " is-hot" : ""}`}
      style={{ left: anchor.left, width: anchor.width, top: SHORTCUTS_HEIGHT + 6 }}
      role="status"
      aria-label="拖到此处暂存"
      onDragOver={(e) => {
        e.preventDefault();
        e.stopPropagation();
        e.dataTransfer.dropEffect = "copy";
        setHot(true);
      }}
      onDragEnter={(e) => {
        e.preventDefault();
        e.stopPropagation();
        setHot(true);
      }}
      onDragLeave={(e) => {
        const related = e.relatedTarget as Node | null;
        if (related && e.currentTarget.contains(related)) return;
        setHot(false);
      }}
    >
      <div className="staging-catcher-inner">
        <svg
          className="staging-catcher-icon"
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          aria-hidden
        >
          <path d="M4 7h16v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V7z" />
          <path d="M8 7V5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
          <path d="M12 11v5" />
          <path d="M9.5 13.5 12 16l2.5-2.5" />
        </svg>
        <span className="staging-catcher-label">拖到此处暂存</span>
      </div>
    </div>
  );
}

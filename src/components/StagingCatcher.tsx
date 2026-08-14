import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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

  useEffect(() => {
    if (!active) return;
    let un: (() => void) | undefined;
    const pluginId = resolveShortcutsStagingPluginId();
    if (!pluginId) return;

    void getCurrentWindow()
      .onDragDropEvent((ev) => {
        const p = ev.payload;
        if (p.type === "leave") {
          setHot(false);
          return;
        }
        if (p.type === "enter" || p.type === "over") {
          void (async () => {
            const win = getCurrentWindow();
            const factor = await win.scaleFactor();
            const pos = "position" in p ? p.position : null;
            const node = document.querySelector(".staging-catcher");
            if (!node) return;
            if (!pos) {
              setHot(true);
              return;
            }
            const lx = pos.x / factor;
            const ly = pos.y / factor;
            const r = node.getBoundingClientRect();
            setHot(
              lx >= r.left - 4 &&
                lx <= r.right + 4 &&
                ly >= r.top - 4 &&
                ly <= r.bottom + 4,
            );
          })();
          return;
        }
        if (p.type !== "drop") return;

        void (async () => {
          const win = getCurrentWindow();
          const factor = await win.scaleFactor();
          const pos = "position" in p ? p.position : null;
          const node = document.querySelector(".staging-catcher");
          if (!node || !pos) return;
          const r = node.getBoundingClientRect();
          const lx = pos.x / factor;
          const ly = pos.y / factor;
          const inside =
            lx >= r.left - 8 &&
            lx <= r.right + 8 &&
            ly >= r.top - 8 &&
            ly <= r.bottom + 8;
          if (!inside) return;

          const paths = p.paths ?? [];
          if (paths.length) {
            await invoke("hub_staging_add_paths", { pluginId, paths }).catch(
              console.error,
            );
          }

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

          activeRef.current = false;
          setActive(false);
          setHot(false);
          setAnchor(null);
          document.documentElement.classList.remove("is-staging-drag");
          document
            .querySelectorAll(".shortcuts-plugin-strip.is-staging-drop-target")
            .forEach((n) => n.classList.remove("is-staging-drop-target"));
        })();
      })
      .then((fn) => {
        un = fn;
      })
      .catch(() => undefined);

    return () => un?.();
  }, [active]);

  if (!active || !anchor) return null;

  return (
    <div
      className={`staging-catcher${hot ? " is-hot" : ""}`}
      style={{ left: anchor.left, width: anchor.width, top: SHORTCUTS_HEIGHT + 6 }}
      role="status"
      aria-label="拖到此处暂存"
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

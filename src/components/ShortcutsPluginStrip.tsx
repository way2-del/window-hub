import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  WH_SHORTCUTS_EVT,
  WH_SHORTCUTS_HUB,
  WH_SHORTCUTS_HUB_RES,
  buildShortcutsSrcdoc,
  isAllowedShortcutsHubCmd,
} from "../plugins/shortcutsHubBridge";
import { SHORTCUTS_HEIGHT } from "../plugins/shortcutsGeometry";

const POPUP_GAP = 8;

type Props = {
  pluginId: string;
  entryPath: string;
  width: number;
  maxWidth: number;
  onRequestWidth: (pluginId: string, width: number) => void;
};

async function popupAnchorFromEl(el: HTMLElement) {
  const win = getCurrentWindow();
  const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  const rect = el.getBoundingClientRect();
  return {
    x: Math.max(8, outer.x / factor + rect.left + Math.min(rect.width, 28) / 2),
    y: outer.y / factor + rect.bottom + POPUP_GAP,
  };
}

/**
 * Host shell: one short transparent iframe for a shortcuts plugin strip.
 */
export default function ShortcutsPluginStrip({
  pluginId,
  entryPath,
  width,
  maxWidth,
  onRequestWidth,
}: Props) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const openingRef = useRef(false);
  const onRequestWidthRef = useRef(onRequestWidth);
  onRequestWidthRef.current = onRequestWidth;
  const [srcdoc, setSrcdoc] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const measureAndReport = () => {
    const iframe = iframeRef.current;
    if (!iframe) return;
    try {
      const doc = iframe.contentDocument;
      if (!doc) return;
      const root = doc.documentElement;
      const hostEl = wrapRef.current?.closest(".shortcuts-host") ?? wrapRef.current;
      const titleEl =
        document.querySelector(".settings-label") ??
        document.querySelector(".settings-btn");
      if (root && hostEl) {
        const cs = getComputedStyle(hostEl);
        const titleCs = titleEl ? getComputedStyle(titleEl) : null;
        const fg =
          cs.getPropertyValue("--chrome-left-fg").trim() ||
          (titleCs?.color ?? cs.color) ||
          "rgba(255,255,255,0.94)";
        const shadow = cs.getPropertyValue("--chrome-left-shadow").trim();
        root.style.setProperty("--wh-chrome-fg", fg);
        if (shadow) root.style.setProperty("--wh-chrome-shadow", shadow);
        if (titleCs) {
          root.style.setProperty("--wh-chrome-font-size", titleCs.fontSize);
          root.style.setProperty("--wh-chrome-font-weight", titleCs.fontWeight);
          root.style.setProperty("--wh-chrome-font-family", titleCs.fontFamily);
        } else {
          root.style.setProperty("--wh-chrome-font-size", "12px");
          root.style.setProperty("--wh-chrome-font-weight", "700");
        }
        root.style.setProperty("--wh-bar-h", `${SHORTCUTS_HEIGHT}px`);
        if (doc.body) {
          doc.body.style.color = fg;
          doc.body.style.background = "transparent";
          doc.body.style.height = `${SHORTCUTS_HEIGHT}px`;
          doc.body.style.maxHeight = `${SHORTCUTS_HEIGHT}px`;
          if (titleCs) {
            doc.body.style.fontSize = titleCs.fontSize;
            doc.body.style.fontWeight = titleCs.fontWeight;
            doc.body.style.fontFamily = titleCs.fontFamily;
          }
        }
      }
      const bar = doc.getElementById("bar") ?? doc.body;
      if (!bar) return;
      const w = Math.ceil(
        Math.max(bar.scrollWidth, bar.getBoundingClientRect().width, 28),
      );
      if (w > 0) onRequestWidthRef.current(pluginId, w);
    } catch {
      /* sandbox / not ready */
    }
  };

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const doc = await buildShortcutsSrcdoc(pluginId, entryPath);
        if (!cancelled) {
          setSrcdoc(doc);
          setError(null);
        }
      } catch (err) {
        if (!cancelled) {
          setSrcdoc(null);
          setError(String(err));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pluginId, entryPath]);

  useEffect(() => {
    const iframe = iframeRef.current;
    if (!iframe || !srcdoc) return;
    let ro: ResizeObserver | null = null;
    let mo: MutationObserver | null = null;

    const attach = () => {
      measureAndReport();
      try {
        const doc = iframe.contentDocument;
        const bar = doc?.getElementById("bar") ?? doc?.body;
        if (!bar) return;
        if (typeof ResizeObserver !== "undefined") {
          ro = new ResizeObserver(() => measureAndReport());
          ro.observe(bar);
        }
        if (typeof MutationObserver !== "undefined") {
          mo = new MutationObserver(() => measureAndReport());
          mo.observe(bar, { childList: true, subtree: true, characterData: true });
        }
      } catch {
        /* noop */
      }
    };

    iframe.addEventListener("load", attach);
    // srcdoc may already be loaded
    window.setTimeout(attach, 0);
    window.setTimeout(measureAndReport, 50);
    window.setTimeout(measureAndReport, 200);

    return () => {
      iframe.removeEventListener("load", attach);
      ro?.disconnect();
      mo?.disconnect();
    };
  }, [pluginId, srcdoc]);

  useEffect(() => {
    const onMessage = (ev: MessageEvent) => {
      const d = ev.data as {
        channel?: string;
        id?: string;
        cmd?: string;
        args?: Record<string, unknown>;
        pluginId?: string;
      } | null;
      if (!d || d.channel !== WH_SHORTCUTS_HUB) return;
      if (d.pluginId && d.pluginId !== pluginId) return;
      const source = ev.source as Window | null;
      if (!source || source !== iframeRef.current?.contentWindow) return;

      if (d.cmd === "shortcuts.requestSize") {
        const w = Number(d.args?.width) || 0;
        onRequestWidth(pluginId, w);
        return;
      }
      if (d.cmd === "shortcuts.getBounds" && d.id) {
        const host = wrapRef.current?.closest(".shortcuts-host") as HTMLElement | null;
        const stripW = wrapRef.current?.getBoundingClientRect().width ?? 0;
        const hostW = host?.getBoundingClientRect().width ?? 0;
        source.postMessage(
          {
            channel: WH_SHORTCUTS_HUB_RES,
            id: d.id,
            result: {
              height: SHORTCUTS_HEIGHT,
              barHeight: SHORTCUTS_HEIGHT,
              width: Math.ceil(stripW),
              maxExpandWidth: Math.ceil(hostW),
            },
          },
          "*",
        );
        return;
      }
      if (d.cmd === "popup.open") {
        void (async () => {
          if (openingRef.current || !wrapRef.current) return;
          openingRef.current = true;
          try {
            const preferGroupId =
              typeof d.args?.preferGroupId === "string" && d.args.preferGroupId
                ? d.args.preferGroupId
                : null;
            const open = await invoke<boolean>("is_plugin_popup_open").catch(() => false);
            const { x, y } = await popupAnchorFromEl(wrapRef.current);
            await invoke("open_plugin_popup", {
              pluginId,
              x,
              y,
              preferGroupId,
            });
            // If already open, Rust emits prefer-group; still call open for idempotent path.
            void open;
          } catch (err) {
            console.error("[ShortcutsPluginStrip] popup", err);
          } finally {
            openingRef.current = false;
          }
        })();
        return;
      }
      if (!d.cmd || !d.id) return;
      if (!isAllowedShortcutsHubCmd(d.cmd) && d.cmd !== "close_plugin_popup") {
        source.postMessage(
          { channel: WH_SHORTCUTS_HUB_RES, id: d.id, error: "command not allowed" },
          "*",
        );
        return;
      }
      void (async () => {
        try {
          const args =
            d.cmd === "get_foreground_app" || d.cmd === "close_plugin_popup"
              ? { ...(d.args || {}) }
              : { pluginId, ...(d.args || {}) };
          const result = await invoke(d.cmd!, args);
          source.postMessage({ channel: WH_SHORTCUTS_HUB_RES, id: d.id, result }, "*");
        } catch (err) {
          source.postMessage(
            { channel: WH_SHORTCUTS_HUB_RES, id: d.id, error: String(err) },
            "*",
          );
        }
      })();
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [pluginId, onRequestWidth]);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
    const frame = () => iframeRef.current?.contentWindow;

    void (async () => {
      try {
        unsubs.push(
          await listen<{ windows: unknown }>("hub-windows-changed", (ev) => {
            frame()?.postMessage(
              {
                channel: WH_SHORTCUTS_EVT,
                type: "windows-changed",
                windows: ev.payload?.windows ?? [],
              },
              "*",
            );
            window.setTimeout(measureAndReport, 0);
          }),
        );
        unsubs.push(
          await listen<string>("plugin-popup-opened", (ev) => {
            const id = typeof ev.payload === "string" ? ev.payload : null;
            frame()?.postMessage(
              { channel: WH_SHORTCUTS_EVT, type: "popup-opened", pluginId: id },
              "*",
            );
          }),
        );
        unsubs.push(
          await listen("plugin-popup-closed", () => {
            frame()?.postMessage({ channel: WH_SHORTCUTS_EVT, type: "popup-closed" }, "*");
            frame()?.postMessage({ channel: WH_SHORTCUTS_EVT, type: "refresh" }, "*");
            window.setTimeout(measureAndReport, 80);
          }),
        );
        unsubs.push(
          await listen<{ pluginId?: string; settings?: Record<string, unknown> }>(
            "plugin-settings-changed",
            (ev) => {
              if (ev.payload?.pluginId && ev.payload.pluginId !== pluginId) return;
              frame()?.postMessage(
                {
                  channel: WH_SHORTCUTS_EVT,
                  type: "settings-changed",
                  settings: ev.payload?.settings ?? {},
                },
                "*",
              );
            },
          ),
        );
        unsubs.push(
          await listen<{
            pluginId?: string;
            notifyId: string;
            actionId: string;
            data?: unknown;
          }>("island-notify-action", (ev) => {
            if (ev.payload?.pluginId && ev.payload.pluginId !== pluginId) return;
            frame()?.postMessage(
              {
                channel: WH_SHORTCUTS_EVT,
                type: "notify-action",
                notifyId: ev.payload.notifyId,
                actionId: ev.payload.actionId,
                data: ev.payload.data,
              },
              "*",
            );
          }),
        );
      } catch {
        /* noop */
      }
    })();

    const tick = async () => {
      try {
        const fg = await invoke<{
          isSelf?: boolean;
          windowId?: string | null;
        }>("get_foreground_app");
        if (cancelled) return;
        if (fg.isSelf) return;
        frame()?.postMessage(
          {
            channel: WH_SHORTCUTS_EVT,
            type: "foreground-changed",
            windowId: fg.windowId ?? null,
          },
          "*",
        );
      } catch {
        /* noop */
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 450);

    return () => {
      cancelled = true;
      window.clearInterval(id);
      unsubs.forEach((fn) => fn());
    };
  }, [pluginId, srcdoc]);

  const w = Math.max(28, Math.min(maxWidth, width || 28));

  return (
    <div
      ref={wrapRef}
      className="shortcuts-plugin-strip"
      style={{ width: w, minWidth: w, height: SHORTCUTS_HEIGHT, flexShrink: 0 }}
      data-plugin={pluginId}
    >
      {error ? (
        <span className="shortcuts-strip-error" title={error}>
          !
        </span>
      ) : srcdoc ? (
        <iframe
          ref={iframeRef}
          className="shortcuts-plugin-frame"
          title={`shortcuts-${pluginId}`}
          srcDoc={srcdoc}
          scrolling="no"
          frameBorder={0}
          sandbox="allow-scripts allow-same-origin"
          style={{
            border: "none",
            outline: "none",
            background: "transparent",
            boxShadow: "none",
            display: "block",
            width: "100%",
            height: 28,
            margin: 0,
            padding: 0,
          }}
        />
      ) : (
        <span className="shortcuts-strip-loading" aria-hidden />
      )}
    </div>
  );
}

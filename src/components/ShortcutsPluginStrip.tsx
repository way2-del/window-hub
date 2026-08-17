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

export type ShortcutsHoverTip = {
  pluginId: string;
  lines: string[];
  /** Viewport coords (CSS px) for tip top-center anchor. */
  x: number;
  y: number;
};

type Props = {
  pluginId: string;
  entryPath: string;
  width: number;
  maxWidth: number;
  onRequestWidth: (pluginId: string, width: number) => void;
  onHoverTip?: (tip: ShortcutsHoverTip | null) => void;
  /** 岛栏隐形 worker：不轮询前台，避免多插件 × 450ms IPC 拖垮主线程 */
  barWorker?: boolean;
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
  onHoverTip,
  barWorker: _barWorker = false,
}: Props) {
  void _barWorker;
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const openingRef = useRef(false);
  const onRequestWidthRef = useRef(onRequestWidth);
  onRequestWidthRef.current = onRequestWidth;
  const onHoverTipRef = useRef(onHoverTip);
  onHoverTipRef.current = onHoverTip;
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
      const measured = Math.ceil(
        Math.max(bar.scrollWidth, bar.getBoundingClientRect().width, 0),
      );
      // Empty worker / pinless strips may be 0–1px; don't floor to 28.
      const w = measured <= 1 ? measured : Math.max(measured, 28);
      if (w >= 0) onRequestWidthRef.current(pluginId, w);
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

  /**
   * Host chrome (--chrome-left-fg) updates with ambient, but iframe copies are
   * one-shot unless we re-push. Status bar CSS vars update live; strips used to
   * stay stale until a click remasured — sync on shell chrome + ambient.
   */
  useEffect(() => {
    if (!srcdoc) return;
    let cancelled = false;
    let raf = 0;
    const timers: number[] = [];

    const pushChrome = () => {
      if (cancelled) return;
      measureAndReport();
    };

    const schedule = (extraMs: number[] = [0, 48, 120]) => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        pushChrome();
        for (const ms of extraMs) {
          timers.push(window.setTimeout(pushChrome, ms));
        }
      });
    };

    const shell =
      (document.querySelector(".shell[data-chrome-left]") as HTMLElement | null) ||
      (wrapRef.current?.closest(".shell") as HTMLElement | null);

    let shellMo: MutationObserver | null = null;
    if (shell && typeof MutationObserver !== "undefined") {
      shellMo = new MutationObserver(() => schedule([0, 32]));
      shellMo.observe(shell, {
        attributes: true,
        attributeFilter: ["style", "data-chrome-left", "data-chrome-right"],
      });
    }

    let unAmbient: (() => void) | undefined;
    void (async () => {
      try {
        unAmbient = await listen("ambient-color", () => schedule([0, 48, 140]));
      } catch {
        /* noop */
      }
    })();

    const onChromeTokens = () => schedule([0, 16]);
    window.addEventListener("wh-chrome-tokens", onChromeTokens);

    schedule([0, 80]);
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      for (const id of timers) window.clearTimeout(id);
      shellMo?.disconnect();
      unAmbient?.();
      window.removeEventListener("wh-chrome-tokens", onChromeTokens);
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
      if (d.cmd === "shortcuts.showTip") {
        const iframe = iframeRef.current;
        if (!iframe || !onHoverTipRef.current) return;
        const rawLines = Array.isArray(d.args?.lines) ? d.args!.lines : [];
        const lines = rawLines
          .map((l) => String(l ?? "").trim())
          .filter(Boolean)
          .slice(0, 8);
        if (!lines.length) {
          onHoverTipRef.current(null);
          return;
        }
        const fr = iframe.getBoundingClientRect();
        const ax =
          typeof d.args?.x === "number" && Number.isFinite(d.args.x)
            ? fr.left + Number(d.args.x)
            : fr.left + fr.width / 2;
        const ay =
          typeof d.args?.y === "number" && Number.isFinite(d.args.y)
            ? fr.top + Number(d.args.y)
            : fr.bottom + 4;
        onHoverTipRef.current({ pluginId, lines, x: ax, y: ay });
        return;
      }
      if (d.cmd === "shortcuts.hideTip") {
        onHoverTipRef.current?.(null);
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
            const width =
              typeof d.args?.width === "number" && Number.isFinite(d.args.width)
                ? d.args.width
                : null;
            const height =
              typeof d.args?.height === "number" && Number.isFinite(d.args.height)
                ? d.args.height
                : null;
            const windowedFullscreen = d.args?.windowedFullscreen === true;
            const nativeFrame = d.args?.nativeFrame === true;
            const resizable =
              d.args?.resizable === true || windowedFullscreen || nativeFrame;
            const open = await invoke<boolean>("is_plugin_popup_open").catch(() => false);
            const { x, y } = await popupAnchorFromEl(wrapRef.current);
            await invoke("open_plugin_popup", {
              pluginId,
              x,
              y,
              preferGroupId,
              width,
              height,
              windowedFullscreen,
              resizable,
              nativeFrame,
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
        unsubs.push(
          await listen<{ pluginId?: string }>("island-bar-click", (ev) => {
            if (ev.payload?.pluginId && ev.payload.pluginId !== pluginId) return;
            frame()?.postMessage({ channel: WH_SHORTCUTS_EVT, type: "bar-click" }, "*");
          }),
        );
        // island-prefs：改由 ShortcutsHost 统一广播
      } catch {
        /* noop */
      }
    })();

    // 前台轮询改由 ShortcutsHost 统一广播，避免 N 条 × get_foreground_app
    return () => {
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
          onLoad={() => {
            // iframe 晚于 Host 启动时补推 prefs，避免 worker 不知 barResident 仍狂轮询
            void invoke<Record<string, unknown>>("get_island_prefs")
              .then((prefs) => {
                iframeRef.current?.contentWindow?.postMessage(
                  {
                    channel: WH_SHORTCUTS_EVT,
                    type: "island-prefs",
                    prefs: prefs ?? {},
                  },
                  "*",
                );
              })
              .catch(() => undefined);
            window.setTimeout(measureAndReport, 0);
          }}
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

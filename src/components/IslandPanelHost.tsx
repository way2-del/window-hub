import { useEffect, useRef, useState } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { parsePluginPanelId } from "../plugins/panelProviders";
import { pluginRegistry } from "../plugins/registry";
import {
  isAllowedPanelHubCmd,
  panelHubBootstrapScript,
  WH_PANEL_HUB,
  WH_PANEL_HUB_RES,
} from "../plugins/panelHubBridge";
import { normalizeStagingChanged } from "../stagingApi";
import "./IslandPanelHost.css";

type Props = {
  pullContent: string;
  /** 岛完全展开后为 true；收起一开始为 false。驱动 panel onEnter/onLeave */
  active: boolean;
  onPanelClose?: () => void;
  /** Alt+Space 岛栏回车：把 query 转发给当前面板 iframe */
  searchSubmit?: { nonce: number; query: string; action?: string } | null;
};

type PendingPanelScripts = {
  token: string;
  boardJs: string;
  panelJs: string;
};

function postPanelLifecycle(
  frame: Window | null | undefined,
  pluginId: string,
  active: boolean,
) {
  frame?.postMessage(
    {
      channel: "island-panel-lifecycle-fwd",
      pluginId,
      phase: active ? "enter" : "leave",
    },
    "*",
  );
}

/** Load optional board.js — prefer asset protocol, fall back to IPC read. */
async function loadOptionalBoardJs(
  pluginId: string,
  relativePath: string,
): Promise<string> {
  try {
    const abs = await invoke<string>("hub_plugin_asset_path", {
      pluginId,
      relativePath,
    });
    const res = await fetch(convertFileSrc(abs));
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const text = await res.text();
    if (!text.includes("FileSearchBoard")) {
      throw new Error("board.js missing FileSearchBoard (truncated?)");
    }
    return text;
  } catch (assetErr) {
    try {
      const text = await invoke<string>("hub_plugin_read_text", {
        pluginId,
        relativePath,
      });
      if (text && !text.includes("FileSearchBoard")) {
        console.warn(
          "[IslandPanelHost] board.js IPC payload missing FileSearchBoard",
          text.length,
        );
      }
      return text ?? "";
    } catch (ipcErr) {
      console.warn("[IslandPanelHost] board.js load failed", {
        assetErr,
        ipcErr,
      });
      return "";
    }
  }
}

/**
 * Sync classic-script inject (NOT blob URL, NOT srcdoc inline).
 *
 * - Blob `script.src` hangs in WebView2 srcdoc iframes (no onload/onerror).
 * - Inlining ~270KB board.js into srcdoc truncates / fails silently →
 *   FileSearchBoard 未定义 while panel.js still paints the chrome.
 * - `HTMLScriptElement.text` + appendChild executes synchronously and reliably.
 */
function injectClassicScript(doc: Document, code: string, label: string): void {
  if (!code) return;
  const win = doc.defaultView as
    | (Window & { __whBoardErr?: string; FileSearchBoard?: unknown })
    | null;
  try {
    const el = doc.createElement("script");
    el.dataset.whSrc = label;
    // Prefer `.text` over textContent for classic script execution semantics.
    el.text = code;
    (doc.body ?? doc.documentElement).appendChild(el);
  } catch (e1) {
    try {
      if (!win) throw e1;
      // Fallback for edge engines: direct eval in iframe realm
      const frameEval = (
        win as Window & { eval: (x: string) => unknown }
      ).eval.bind(win);
      frameEval(code);
    } catch (e2) {
      const err = e2 instanceof Error ? e2 : new Error(String(e2));
      if (win) win.__whBoardErr = `${label}: ${err.message}`;
      throw err;
    }
  }
}

function injectPanelScripts(
  doc: Document,
  pending: PendingPanelScripts,
  pluginId: string,
): void {
  if (doc.documentElement.dataset.whPanelScripts === pending.token) return;
  doc.documentElement.dataset.whPanelScripts = pending.token;

  const win = doc.defaultView as
    | (Window & { FileSearchBoard?: unknown; __whBoardErr?: string; hub?: unknown })
    | null;

  // Always (re)inject hub bootstrap before panel.js — srcdoc head scripts can
  // race or fail silently in WebView2; panel boot must never see missing hub.
  if (!win?.hub || (win as Window & { __WH_PLUGIN_ID__?: string }).__WH_PLUGIN_ID__ !== pluginId) {
    injectClassicScript(doc, panelHubBootstrapScript(pluginId), "hub-boot");
  }
  if (win && !win.hub) {
    throw new Error("window.hub still missing after hub-boot inject");
  }

  if (pending.boardJs) {
    injectClassicScript(doc, pending.boardJs, "board.js");
    if (win && !win.FileSearchBoard) {
      win.__whBoardErr =
        win.__whBoardErr ||
        `board.js 已注入但未导出 FileSearchBoard (${pending.boardJs.length} chars)`;
      console.error("[IslandPanelHost]", win.__whBoardErr);
    }
  }
  if (!pending.panelJs.trim()) {
    throw new Error("panel.js 为空，无法启动面板");
  }
  injectClassicScript(doc, pending.panelJs, "panel.js");
}

/** Shell HTML only — scripts injected after load (see injectPanelScripts). */
function buildPanelSrcdoc(opts: {
  pluginId: string;
  html: string;
  css: string;
}): string {
  let html = opts.html;
  html = html.replace(/<link[^>]*href=["'][^"']*panel\.css["'][^>]*>/gi, "");
  html = html.replace(
    /<script[^>]*src=["'][^"']*panel\.js["'][^>]*>\s*<\/script>/gi,
    "",
  );
  html = html.replace(
    /<script[^>]*src=["'][^"']*board\.js["'][^>]*>\s*<\/script>/gi,
    "",
  );

  const baseReset = `<style>
html,body{margin:0;height:100%;background:#000;color:#f4f4f5;color-scheme:dark;overflow:hidden}
::-webkit-scrollbar{width:0!important;height:0!important;display:none!important}
*{scrollbar-width:none;-ms-overflow-style:none}
</style>`;
  const styleTag = opts.css
    ? `${baseReset}<style>${opts.css}</style>`
    : baseReset;
  html = /<head[^>]*>/i.test(html)
    ? html.replace(/<head[^>]*>/i, (m) => `${m}${styleTag}`)
    : `${styleTag}${html}`;

  const boot = `<script>${panelHubBootstrapScript(opts.pluginId)}</script>`;
  const themeAttr = ` data-theme="dark"`;
  if (/<html\b/i.test(html)) {
    html = html.replace(/<html\b([^>]*)>/i, (_m, attrs: string) => {
      const cleaned = String(attrs).replace(/\s*data-theme=("|')[^"']*\1/i, "");
      return `<html${cleaned}${themeAttr}>`;
    });
  } else {
    html = `<html${themeAttr}>${html}</html>`;
  }

  return /<head[^>]*>/i.test(html)
    ? html.replace(/<head[^>]*>/i, (m) => `${m}${boot}`)
    : `${boot}${html}`;
}

export default function IslandPanelHost({
  pullContent,
  active,
  onPanelClose,
  searchSubmit,
}: Props) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const pendingScriptsRef = useRef<PendingPanelScripts | null>(null);
  const [srcdoc, setSrcdoc] = useState<string | null>(null);
  const [panelError, setPanelError] = useState<string | null>(null);
  const [registryEpoch, setRegistryEpoch] = useState(0);
  const activeRef = useRef(active);
  activeRef.current = active;
  const pluginIdRef = useRef<string | null>(null);

  const pluginId = parsePluginPanelId(pullContent);
  pluginIdRef.current = pluginId;
  const enabled = pluginId
    ? Boolean(pluginRegistry.get(pluginId)?.enabled)
    : false;

  useEffect(
    () => pluginRegistry.subscribe(() => setRegistryEpoch((n) => n + 1)),
    [],
  );

  useEffect(() => {
    const onMessage = (ev: MessageEvent) => {
      const d = ev.data as {
        channel?: string;
        id?: string;
        cmd?: string;
        args?: Record<string, unknown>;
        action?: string;
        query?: string;
        pluginId?: string;
        kind?: string;
      } | null;
      if (!d) return;

      if (d.channel !== WH_PANEL_HUB) return;
      const source = ev.source as Window | null;
      if (!source) return;
      const pid = pluginIdRef.current;

      if (d.cmd === "panel.close") {
        onPanelClose?.();
        return;
      }
      if (!pid) return;
      if (!d.cmd || !d.id || !isAllowedPanelHubCmd(d.cmd)) {
        source.postMessage(
          {
            channel: WH_PANEL_HUB_RES,
            id: d.id,
            error: "command not allowed",
          },
          "*",
        );
        return;
      }
      void (async () => {
        try {
          // Host-global cmds: do not force caller pluginId
          const globalCmds = new Set([
            "list_installed_plugins",
            "open_settings_window",
          ]);
          const args = globalCmds.has(d.cmd!)
            ? { ...(d.args || {}) }
            : { pluginId: pid, ...(d.args || {}) };
          const result = await invoke(d.cmd!, args);
          source.postMessage(
            { channel: WH_PANEL_HUB_RES, id: d.id, result },
            "*",
          );
        } catch (err) {
          source.postMessage(
            {
              channel: WH_PANEL_HUB_RES,
              id: d.id,
              error: String(err),
            },
            "*",
          );
        }
      })();
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [onPanelClose]);

  useEffect(() => {
    if (!pluginId || !enabled) {
      pendingScriptsRef.current = null;
      setSrcdoc(null);
      setPanelError(pluginId && !enabled ? "插件已禁用" : null);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const runtime = pluginRegistry.get(pluginId);
        const panel = runtime?.manifest.entry?.panel ?? "panel.html";
        if ((runtime?.manifest.capabilities ?? []).includes("media.camera")) {
          await invoke("hub_camera_prepare", { pluginId }).catch(() => undefined);
        }
        const html = await invoke<string>("hub_plugin_read_text", {
          pluginId,
          relativePath: panel,
        });
        const dir = panel.includes("/")
          ? panel.slice(0, panel.lastIndexOf("/") + 1)
          : panel.includes("\\")
            ? panel.slice(0, panel.lastIndexOf("\\") + 1)
            : "";
        const wantsBoard = (runtime?.manifest.capabilities ?? []).includes(
          "everything.search",
        );
        const [css, js, boardJs] = await Promise.all([
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: `${dir}panel.css`,
          }).catch(() => ""),
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: `${dir}panel.js`,
          }),
          wantsBoard
            ? loadOptionalBoardJs(pluginId, `${dir}board.js`)
            : Promise.resolve(""),
        ]);
        if (cancelled) return;
        if (!js?.trim()) {
          throw new Error("panel.js 读取失败或为空");
        }
        if (wantsBoard && !boardJs) {
          console.warn(
            "[IslandPanelHost] board.js missing — home cards will degrade",
            pluginId,
          );
        } else if (boardJs) {
          console.info(
            `[IslandPanelHost] board.js ready (${boardJs.length} chars)`,
          );
        }

        const token = `${pluginId}:${Date.now()}:${boardJs.length}:${js.length}`;
        pendingScriptsRef.current = {
          token,
          boardJs,
          panelJs: js,
        };
        const injected = buildPanelSrcdoc({ pluginId, html, css });
        if (cancelled) return;
        setSrcdoc(injected);
        setPanelError(null);
      } catch (e) {
        if (!cancelled) {
          pendingScriptsRef.current = null;
          setSrcdoc(null);
          setPanelError(String(e));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pluginId, pullContent, enabled, registryEpoch]);

  useEffect(() => {
    if (!pluginId || !enabled) return;
    let un: (() => void) | undefined;
    void listen("island-prefs", () => {
      iframeRef.current?.contentWindow?.postMessage(
        { channel: "island-prefs-fwd", pluginId },
        "*",
      );
    }).then((fn) => {
      un = fn;
    });
    return () => un?.();
  }, [pluginId, enabled]);

  useEffect(() => {
    if (!pluginId || !enabled) return;
    let un: (() => void) | undefined;
    void listen("staging-changed", (ev) => {
      const { pluginId: pid, summary } = normalizeStagingChanged(
        ev.payload as Parameters<typeof normalizeStagingChanged>[0],
      );
      if (pid && pid !== pluginId) return;
      iframeRef.current?.contentWindow?.postMessage(
        {
          channel: "staging-changed-fwd",
          pluginId: pid ?? pluginId,
          summary,
        },
        "*",
      );
    }).then((fn) => {
      un = fn;
    });
    return () => un?.();
  }, [pluginId, enabled]);

  useEffect(() => {
    if (!pluginId || !enabled) return;
    let un: (() => void) | undefined;
    void listen<{ pluginId?: string; settings?: Record<string, unknown> }>(
      "plugin-settings-changed",
      (ev) => {
        if (ev.payload?.pluginId && ev.payload.pluginId !== pluginId) return;
        iframeRef.current?.contentWindow?.postMessage(
          {
            channel: "plugin-settings-changed-fwd",
            pluginId: ev.payload?.pluginId ?? pluginId,
            settings: ev.payload?.settings ?? {},
          },
          "*",
        );
      },
    ).then((fn) => {
      un = fn;
    });
    return () => un?.();
  }, [pluginId, enabled]);

  useEffect(() => {
    if (!pluginId || !enabled) return;
    let un: (() => void) | undefined;
    void listen<{
      pluginId?: string;
      notifyId: string;
      actionId: string;
      data?: unknown;
    }>("island-notify-action", (ev) => {
      if (ev.payload?.pluginId && ev.payload.pluginId !== pluginId) return;
      iframeRef.current?.contentWindow?.postMessage(
        {
          channel: "island-notify-action-fwd",
          pluginId: ev.payload?.pluginId ?? pluginId,
          notifyId: ev.payload.notifyId,
          actionId: ev.payload.actionId,
          data: ev.payload.data,
        },
        "*",
      );
    }).then((fn) => {
      un = fn;
    });
    return () => un?.();
  }, [pluginId, enabled]);

  useEffect(() => {
    if (!pluginId || !enabled || !searchSubmit) return;
    const payload = {
      channel: "island-search-fwd",
      pluginId,
      action: searchSubmit.action || "submit",
      query: searchSubmit.query ?? "",
      nonce: searchSubmit.nonce,
    };
    const post = () => {
      iframeRef.current?.contentWindow?.postMessage(payload, "*");
    };
    post();
    // 仅一次短延迟：iframe 刚注入时可能丢消息；避免 5 次连发导致卡顿
    const t1 = window.setTimeout(post, 80);
    return () => {
      window.clearTimeout(t1);
    };
  }, [pluginId, enabled, searchSubmit]);

  const tryInjectAndEnter = () => {
    const iframe = iframeRef.current;
    const pending = pendingScriptsRef.current;
    if (!iframe || !pending || !pluginId) return;
    const doc = iframe.contentDocument;
    if (!doc || doc.readyState === "loading") return;
    try {
      injectPanelScripts(doc, pending, pluginId);
      postPanelLifecycle(iframe.contentWindow, pluginId, activeRef.current);
    } catch (e) {
      console.error("[IslandPanelHost] inject failed", e);
      setPanelError(String((e as Error)?.message || e));
      try {
        delete doc.documentElement.dataset.whPanelScripts;
      } catch {
        /* ignore */
      }
    }
  };

  useEffect(() => {
    if (!pluginId || !srcdoc || !enabled) return;
    tryInjectAndEnter();
    const iframe = iframeRef.current;
    if (!iframe) return;
    const onLoad = () => tryInjectAndEnter();
    iframe.addEventListener("load", onLoad);
    // srcdoc can finish before listener attaches
    const t0 = window.setTimeout(tryInjectAndEnter, 0);
    const t1 = window.setTimeout(tryInjectAndEnter, 50);
    return () => {
      iframe.removeEventListener("load", onLoad);
      window.clearTimeout(t0);
      window.clearTimeout(t1);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, pluginId, srcdoc, enabled]);

  useEffect(() => {
    if (!pluginId || !enabled || !active || !srcdoc) return;
    const frame = iframeRef.current?.contentWindow;
    postPanelLifecycle(frame, pluginId, true);
    if (!searchSubmit) return;
    frame?.postMessage(
      {
        channel: "island-search-fwd",
        pluginId,
        action: searchSubmit.action || "submit",
        query: searchSubmit.query ?? "",
        nonce: searchSubmit.nonce,
      },
      "*",
    );
  }, [active, pluginId, enabled, srcdoc, searchSubmit]);

  if (!pullContent || !pluginId) {
    return <div className="panel-plugin-empty">未选择下拉内容</div>;
  }
  if (!enabled) {
    return (
      <div className="panel-plugin-empty">
        插件已禁用
        <span>请在设置中启用，或改选其它下拉内容</span>
      </div>
    );
  }
  if (panelError) {
    return (
      <div className="panel-plugin-empty">
        插件面板未就绪
        <span>{panelError}</span>
      </div>
    );
  }
  if (!srcdoc) {
    return <div className="panel-plugin-empty">加载面板…</div>;
  }

  return (
    <iframe
      ref={iframeRef}
      key={String(pluginId)}
      className="panel-plugin-frame"
      title={`plugin-panel-${pluginId}`}
      srcDoc={srcdoc}
      sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
      allow="camera; clipboard-read; clipboard-write"
      onLoad={() => {
        tryInjectAndEnter();
      }}
    />
  );
}

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

/** Load optional board.js — prefer asset protocol (no IPC size risk), fall back to read_text. */
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
      return text;
    } catch (ipcErr) {
      console.warn("[IslandPanelHost] board.js load failed", {
        assetErr,
        ipcErr,
      });
      return "";
    }
  }
}

/** Inject classic scripts via textContent — avoids srcdoc HTML/`</script>`/blob races. */
function injectPanelScripts(
  doc: Document,
  pending: PendingPanelScripts,
): void {
  if (doc.documentElement.dataset.whPanelScripts === pending.token) return;
  doc.documentElement.dataset.whPanelScripts = pending.token;

  const run = (code: string, label: string) => {
    if (!code) return;
    const el = doc.createElement("script");
    el.textContent = code;
    try {
      (doc.body ?? doc.documentElement).appendChild(el);
    } catch (e) {
      console.error(`[IslandPanelHost] inject ${label} failed`, e);
      try {
        (doc.defaultView as Window & { __whBoardErr?: string }).__whBoardErr =
          String(e);
      } catch {
        /* ignore */
      }
    }
  };

  // board first so window.FileSearchBoard exists before panel.js boots
  run(pending.boardJs, "board.js");
  const win = doc.defaultView as (Window & { FileSearchBoard?: unknown; __whBoardErr?: string }) | null;
  if (pending.boardJs && win && !win.FileSearchBoard) {
    win.__whBoardErr =
      win.__whBoardErr ||
      `board.js 已注入但未导出 FileSearchBoard (${pending.boardJs.length} chars)`;
    console.error("[IslandPanelHost]", win.__whBoardErr);
  }
  run(pending.panelJs, "panel.js");
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

  const pluginId = parsePluginPanelId(pullContent);
  const enabled = pluginId ? Boolean(pluginRegistry.get(pluginId)?.enabled) : false;

  useEffect(() => pluginRegistry.subscribe(() => setRegistryEpoch((n) => n + 1)), []);

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
        // Camera PermissionRequested: only when a media.camera panel is opened.
        if ((runtime?.manifest.capabilities ?? []).includes("media.camera")) {
          await invoke("hub_camera_prepare", { pluginId }).catch(() => undefined);
        }
        const panel = runtime?.manifest.entry?.panel ?? "panel.html";
        let html = await invoke<string>("hub_plugin_read_text", {
          pluginId,
          relativePath: panel,
        });
        const dir = panel.includes("/")
          ? panel.slice(0, panel.lastIndexOf("/") + 1)
          : panel.includes("\\")
            ? panel.slice(0, panel.lastIndexOf("\\") + 1)
            : "";
        const [css, js, boardJs] = await Promise.all([
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: `${dir}panel.css`,
          }).catch(() => ""),
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: `${dir}panel.js`,
          }).catch(() => ""),
          loadOptionalBoardJs(pluginId, `${dir}board.js`),
        ]);
        if (cancelled) return;
        if (boardJs) {
          console.info(
            `[IslandPanelHost] board.js ready (${boardJs.length} chars)`,
          );
        } else {
          console.warn("[IslandPanelHost] board.js not available for", pluginId);
        }
        // Strip link/script tags whether href is panel.css or ./panel.css
        html = html.replace(/<link[^>]*href=["'][^"']*panel\.css["'][^>]*>/gi, "");
        html = html.replace(
          /<script[^>]*src=["'][^"']*panel\.js["'][^>]*>\s*<\/script>/gi,
          "",
        );
        html = html.replace(
          /<script[^>]*src=["'][^"']*board\.js["'][^>]*>\s*<\/script>/gi,
          "",
        );
        // Base dark shell before plugin CSS — avoids white flash / system scrollbar
        // when panel opens from collapsed (direct island-bar click).
        const baseReset = `<style>
html,body{margin:0;height:100%;background:#000;color:#f4f4f5;color-scheme:dark;overflow:hidden}
::-webkit-scrollbar{width:0!important;height:0!important;display:none!important}
*{scrollbar-width:none;-ms-overflow-style:none}
</style>`;
        const styleTag = css ? `${baseReset}<style>${css}</style>` : baseReset;
        html = /<head[^>]*>/i.test(html)
          ? html.replace(/<head[^>]*>/i, (m) => `${m}${styleTag}`)
          : `${styleTag}${html}`;
        const boot = `<script>${panelHubBootstrapScript(pluginId)}</script>`;
        // Island shell is always black — never inject host light theme into panel iframe
        const themeAttr = ` data-theme="dark"`;
        if (/<html\b/i.test(html)) {
          html = html.replace(/<html\b([^>]*)>/i, (_m, attrs: string) => {
            const cleaned = String(attrs).replace(/\s*data-theme=("|')[^"']*\1/i, "");
            return `<html${cleaned}${themeAttr}>`;
          });
        } else {
          html = `<html${themeAttr}>${html}</html>`;
        }
        const injected = /<head[^>]*>/i.test(html)
          ? html.replace(/<head[^>]*>/i, (m) => `${m}${boot}`)
          : `${boot}${html}`;
        // Do NOT embed board/panel into srcdoc — inject via textContent on iframe load
        // (same pattern as PluginPopupHost; avoids HTML parse + blob/src races).
        const token = `${pluginId}:${Date.now()}:${boardJs.length}:${js.length}`;
        pendingScriptsRef.current = {
          token,
          boardJs,
          panelJs: js,
        };
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
    const onMessage = (ev: MessageEvent) => {
      const d = ev.data as {
        channel?: string;
        id?: string;
        cmd?: string;
        args?: Record<string, unknown>;
      } | null;
      if (!d || d.channel !== WH_PANEL_HUB) return;
      const source = ev.source as Window | null;
      if (!source) return;

      if (d.cmd === "panel.close") {
        onPanelClose?.();
        return;
      }
      if (!d.cmd || !d.id || !isAllowedPanelHubCmd(d.cmd)) {
        source.postMessage(
          { channel: WH_PANEL_HUB_RES, id: d.id, error: "command not allowed" },
          "*",
        );
        return;
      }
      void (async () => {
        try {
          const args = { pluginId, ...(d.args || {}) };
          const result = await invoke(d.cmd!, args);
          source.postMessage({ channel: WH_PANEL_HUB_RES, id: d.id, result }, "*");
        } catch (err) {
          source.postMessage(
            { channel: WH_PANEL_HUB_RES, id: d.id, error: String(err) },
            "*",
          );
        }
      })();
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [pluginId, enabled, onPanelClose]);

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
      const frame = iframeRef.current?.contentWindow;
      frame?.postMessage(
        { channel: "staging-changed-fwd", pluginId: pid ?? pluginId, summary },
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
        const frame = iframeRef.current?.contentWindow;
        frame?.postMessage(
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
      const frame = iframeRef.current?.contentWindow;
      frame?.postMessage(
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
    const frame = iframeRef.current?.contentWindow;
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
  }, [pluginId, enabled, searchSubmit]);

  useEffect(() => {
    if (!pluginId || !srcdoc || !enabled) return;
    const iframe = iframeRef.current;
    const pending = pendingScriptsRef.current;
    if (!iframe || !pending) return;
    const tryInject = () => {
      const doc = iframe.contentDocument;
      if (!doc || doc.readyState === "loading") return false;
      injectPanelScripts(doc, pending);
      return true;
    };
    if (tryInject()) {
      postPanelLifecycle(iframe.contentWindow, pluginId, activeRef.current);
      return;
    }
    const onLoad = () => {
      tryInject();
      postPanelLifecycle(iframe.contentWindow, pluginId, activeRef.current);
    };
    iframe.addEventListener("load", onLoad);
    return () => iframe.removeEventListener("load", onLoad);
  }, [active, pluginId, srcdoc, enabled]);

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
      key={pluginId}
      className="panel-plugin-frame"
      title={`plugin-panel-${pluginId}`}
      srcDoc={srcdoc}
      sandbox="allow-scripts allow-same-origin"
      allow="camera"
      onLoad={() => {
        if (!pluginId) return;
        const doc = iframeRef.current?.contentDocument;
        const pending = pendingScriptsRef.current;
        if (doc && pending) injectPanelScripts(doc, pending);
        postPanelLifecycle(
          iframeRef.current?.contentWindow,
          pluginId,
          activeRef.current,
        );
      }}
    />
  );
}

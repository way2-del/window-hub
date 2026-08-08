import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isBuiltinPanel, parsePluginPanelId } from "../plugins/panelProviders";
import { pluginRegistry } from "../plugins/registry";
import {
  isAllowedPanelHubCmd,
  panelHubBootstrapScript,
  WH_PANEL_HUB,
  WH_PANEL_HUB_RES,
} from "../plugins/panelHubBridge";
import { normalizeStagingChanged } from "../stagingApi";
import MirrorPreview from "./MirrorPreview";
import type { WeatherInfo } from "../weather";
import type { ReactNode } from "react";
import "./IslandPanelHost.css";

type Props = {
  pullContent: string;
  weather: WeatherInfo;
  mirrorLive: boolean;
  IconPin: () => ReactNode;
  IconCloud: (p: { className?: string }) => ReactNode;
  IconDroplets: (p: { className?: string }) => ReactNode;
  IconWind: (p: { className?: string }) => ReactNode;
  onPanelClose?: () => void;
};

export default function IslandPanelHost({
  pullContent,
  weather,
  mirrorLive,
  IconPin,
  IconCloud,
  IconDroplets,
  IconWind,
  onPanelClose,
}: Props) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const [srcdoc, setSrcdoc] = useState<string | null>(null);
  const [panelError, setPanelError] = useState<string | null>(null);

  const pluginId = isBuiltinPanel(pullContent) ? null : parsePluginPanelId(pullContent);

  useEffect(() => {
    if (!pluginId) {
      setSrcdoc(null);
      setPanelError(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const runtime = pluginRegistry.get(pluginId);
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
        const [css, js] = await Promise.all([
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: `${dir}panel.css`,
          }).catch(() => ""),
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: `${dir}panel.js`,
          }).catch(() => ""),
        ]);
        if (cancelled) return;
        if (css) {
          html = html.replace(
            /<link[^>]*href=["']\.\/panel\.css["'][^>]*>/i,
            `<style>${css}</style>`,
          );
        }
        const boot = `<script>${panelHubBootstrapScript(pluginId)}</script>`;
        // Island shell is always black — never inject host light theme into panel iframe
        const themeAttr = ` data-theme="dark"`;
        const bodyJs = js ? `<script>${js}</script>` : "";
        html = html.replace(
          /<script[^>]*src=["']\.\/panel\.js["'][^>]*>\s*<\/script>/i,
          "",
        );
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
        const withJs = /<\/body>/i.test(injected)
          ? injected.replace(/<\/body>/i, `${bodyJs}</body>`)
          : `${injected}${bodyJs}`;
        setSrcdoc(withJs);
        setPanelError(null);
      } catch (e) {
        if (!cancelled) {
          setSrcdoc(null);
          setPanelError(String(e));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pluginId, pullContent]);

  useEffect(() => {
    if (!pluginId) return;
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
  }, [pluginId, onPanelClose]);

  useEffect(() => {
    if (!pluginId) return;
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
  }, [pluginId]);

  useEffect(() => {
    if (!pluginId) return;
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
  }, [pluginId]);

  if (isBuiltinPanel(pullContent)) {
    if (pullContent === "mirror") {
      return <MirrorPreview active={mirrorLive} />;
    }
    return (
      <>
        <div className="panel-top">
          <div className="panel-loc">
            <IconPin />
            <span>{weather.city}</span>
          </div>
          <div className="panel-temp">{weather.temp}°</div>
        </div>
        <div className="panel-body">
          <div className="panel-condition">
            {weather.iconUrl ? (
              <img className="condition-img" src={weather.iconUrl} alt="" draggable={false} />
            ) : (
              <IconCloud className="condition-cloud" />
            )}
            <span>
              {weather.condition}
              {weather.uptime ? ` · 更新 ${weather.uptime}` : ""}
            </span>
          </div>
          <div className="panel-cards">
            <div className="panel-card">
              <IconDroplets className="card-icon droplets" />
              <span>湿度</span>
              <strong>{weather.humidity}%</strong>
            </div>
            <div className="panel-card">
              <IconWind className="card-icon wind" />
              <span>风力</span>
              <strong>{weather.wind}</strong>
            </div>
            <div className="panel-card">
              <span className="feel-badge" aria-hidden>
                {weather.feelsLike}°
              </span>
              <span>体感</span>
              <strong>
                {weather.low}° / {weather.high}°
              </strong>
            </div>
          </div>
        </div>
      </>
    );
  }

  if (!pluginId) {
    return <div className="panel-plugin-empty">未知面板</div>;
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
      className="panel-plugin-frame"
      title={`plugin-panel-${pluginId}`}
      srcDoc={srcdoc}
      sandbox="allow-scripts allow-same-origin"
    />
  );
}

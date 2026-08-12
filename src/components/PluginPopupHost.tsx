import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { normalizeGlassKind, subscribeSystemDark, syncGlassCss, type GlassPrefs } from "../glassPrefs";
import "./PluginPopupHost.css";

declare global {
  interface Window {
    __WH_PLUGIN_ID__?: string;
    hub?: {
      pluginId: string;
      windows: {
        list: () => Promise<unknown>;
        get: (id: string) => Promise<unknown>;
        focus: (id: string) => Promise<unknown>;
        subscribe: (cb: (windows: unknown) => void) => () => void;
      };
      storage: {
        get: (key: string) => Promise<unknown>;
        set: (key: string, value: unknown) => Promise<unknown>;
      };
      shortcuts: {
        setBadge: (badge: unknown) => Promise<unknown>;
      };
      notify: ((opts: {
        title: string;
        body?: string;
        iconPng?: string;
        urgency?: string;
        ttlMs?: number;
        actions?: unknown[];
        data?: unknown;
      }) => Promise<unknown>) & {
        onAction: (
          cb: (ev: { notifyId: string; actionId: string; data?: unknown }) => void,
        ) => () => void;
      };
      fetch: (url: string, opts?: Record<string, unknown>) => Promise<unknown>;
      media: { sendKey: (action: string) => Promise<unknown> };
      popup: { close: () => Promise<unknown> };
      applyEffect: (material?: string) => Promise<unknown>;
    };
  }
}

function resolvePluginId(): string {
  return (
    new URLSearchParams(window.location.search).get("plugin") ??
    window.__WH_PLUGIN_ID__ ??
    ""
  );
}

function ensureHub(pluginId: string) {
  if (window.hub?.pluginId === pluginId) return;
  window.__WH_PLUGIN_ID__ = pluginId;

  const withPlugin = (args?: Record<string, unknown>) => ({
    pluginId,
    ...(args ?? {}),
  });

  type NotifyFn = NonNullable<Window["hub"]>["notify"];

  const notifyFn = ((opts: {
    title: string;
    body?: string;
    iconPng?: string;
    urgency?: string;
    ttlMs?: number;
    actions?: unknown[];
    data?: unknown;
  }) =>
    invoke("hub_notify", withPlugin({
      opts: {
        title: opts?.title || "",
        body: opts?.body,
        iconPng: opts?.iconPng,
        urgency: opts?.urgency,
        ttlMs: opts?.ttlMs,
        actions: opts?.actions,
        data: opts?.data,
      },
    }))) as NotifyFn;

  notifyFn.onAction = (cb) => {
    let un = () => {};
    void listen<{
      pluginId?: string;
      notifyId: string;
      actionId: string;
      data?: unknown;
    }>("island-notify-action", (ev) => {
      if (ev.payload?.pluginId && ev.payload.pluginId !== pluginId) return;
      cb({
        notifyId: ev.payload.notifyId,
        actionId: ev.payload.actionId,
        data: ev.payload.data,
      });
    }).then((fn) => {
      un = fn;
    });
    return () => un();
  };

  window.hub = {
    pluginId,
    windows: {
      list: () => invoke("hub_windows_list", withPlugin()),
      get: (id) => invoke("hub_windows_get", withPlugin({ id })),
      focus: (id) => invoke("hub_windows_focus", withPlugin({ id })),
      subscribe: (cb) => {
        let un = () => {};
        void listen<{ windows: unknown }>("hub-windows-changed", (ev) => {
          if (ev.payload?.windows) cb(ev.payload.windows);
        }).then((fn) => {
          un = fn;
        });
        void invoke("hub_windows_list", withPlugin())
          .then((wins) => cb(wins))
          .catch(() => undefined);
        return () => un();
      },
    },
    storage: {
      get: (key) => invoke("hub_storage_get", withPlugin({ key })),
      set: (key, value) => invoke("hub_storage_set", withPlugin({ key, value })),
    },
    shortcuts: {
      setBadge: (badge) => invoke("hub_shortcuts_set_badge", withPlugin({ badge })),
    },
    notify: notifyFn,
    fetch: (url: string, opts?: Record<string, unknown>) =>
      invoke("hub_fetch", withPlugin({ url, opts: opts ?? null })),
    media: {
      sendKey: (action: string) =>
        invoke("hub_media_send_key", withPlugin({ action })),
    },
    popup: {
      close: () => invoke("close_plugin_popup"),
    },
    applyEffect: (material?: string) =>
      material
        ? invoke("apply_window_effect", { material })
        : invoke("apply_window_effect", {}),
  };
}

type Boot = { css: string; js: string; pinyin?: string };

/**
 * Host shell: Tauri IPC + inject plugin CSS/JS from disk (independent package).
 */
export default function PluginPopupHost() {
  const pluginId = resolvePluginId();
  const [boot, setBoot] = useState<Boot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const injectedRef = useRef(false);

  useEffect(() => {
    if (!pluginId) {
      setError("缺少插件 ID");
      return;
    }
    ensureHub(pluginId);
    void (async () => {
      try {
        const prefs = await invoke<GlassPrefs>("get_material_prefs");
        await syncGlassCss({ ...prefs, kind: normalizeGlassKind(prefs.kind) });
      } catch {
        await syncGlassCss({ kind: "mica-alt", dark: true });
      }
      await invoke("apply_window_effect", {}).catch(() => undefined);
    })();

    let cancelled = false;
    let unGlass: (() => void) | undefined;
    void listen<GlassPrefs>("material-prefs", (ev) => {
      void syncGlassCss({ ...ev.payload, kind: normalizeGlassKind(ev.payload.kind) });
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }).then((fn) => {
      if (cancelled) fn();
      else unGlass = fn;
    });

    const unSystem = subscribeSystemDark(() => {
      void (async () => {
        try {
          const prefs = await invoke<GlassPrefs>("get_material_prefs");
          if (prefs.dark != null) return;
          await syncGlassCss({ ...prefs, kind: normalizeGlassKind(prefs.kind) });
          await invoke("apply_window_effect", {}).catch(() => undefined);
        } catch {
          /* noop */
        }
      })();
    });

    void (async () => {
      try {
        const list = await invoke<
          Array<{ id: string; manifest?: { entry?: { popup?: string } } }>
        >("list_installed_plugins");
        const rec = list.find((p) => p.id === pluginId);
        const entry = rec?.manifest?.entry?.popup ?? "popup.html";
        const dir = entry.includes("/")
          ? entry.slice(0, entry.lastIndexOf("/") + 1)
          : entry.includes("\\")
            ? entry.slice(0, entry.lastIndexOf("\\") + 1)
            : "";

        const [css, js, pinyinPro, pinyinLite] = await Promise.all([
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: `${dir}popup.css`,
          }).catch(() => ""),
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: `${dir}popup.js`,
          }),
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: `${dir}pinyin-pro.min.js`,
          }).catch(() => ""),
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: `${dir}pinyinlite.min.js`,
          }).catch(() => ""),
        ]);
        if (cancelled) return;
        setBoot({ css, js, pinyin: pinyinPro || pinyinLite });
      } catch (err) {
        if (!cancelled) setError(String(err));
      }
    })();

    return () => {
      cancelled = true;
      unGlass?.();
      unSystem();
    };
  }, [pluginId]);

  useEffect(() => {
    if (!boot || injectedRef.current) return;
    ensureHub(pluginId);
    injectedRef.current = true;

    if (boot.css) {
      const style = document.createElement("style");
      style.textContent = boot.css;
      document.head.appendChild(style);
    }

    // Defer so #app from this render is in the DOM
    const t = window.setTimeout(() => {
      if (!document.getElementById("app")) {
        setError("插件挂载点 #app 缺失");
        return;
      }
      if (boot.pinyin) {
        const py = document.createElement("script");
        py.id = "wh-pinyin-lib";
        py.textContent = boot.pinyin;
        document.body.appendChild(py);
      }
      const script = document.createElement("script");
      script.textContent = boot.js;
      document.body.appendChild(script);
    }, 0);

    return () => window.clearTimeout(t);
  }, [boot, pluginId]);

  if (error) {
    return (
      <div className="plugin-popup-frame">
        <div className="plugin-popup-empty">{error}</div>
        <button
          type="button"
          className="plugin-popup-close"
          onClick={() => void invoke("close_plugin_popup")}
        >
          关闭
        </button>
      </div>
    );
  }

  if (!boot) {
    return (
      <div className="plugin-popup-frame">
        <div className="plugin-popup-empty">加载插件…</div>
      </div>
    );
  }

  return <main id="app" className="wg-shell" />;
}

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { normalizeGlassKind, subscribeSystemDark, syncGlassCss, type GlassPrefs } from "../glassPrefs";
import {
  normalizeStagingChanged,
  type StagingChangedPayload,
  type StagingSummary,
} from "../stagingApi";
import "./PluginPopupHost.css";

/** 同会话内复用已读的 popup.css/js，换开同插件时少两次读盘 */
const popupAssetCache = new Map<string, { css: string; js: string }>();

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
        remove: (key: string) => Promise<unknown>;
        listKeys: () => Promise<unknown>;
        subscribe: (
          cb: (ev: { key: string; value: unknown; removed: boolean }) => void,
        ) => () => void;
      };
      shortcuts: {
        setBadge: (badge: unknown) => Promise<unknown>;
      };
      staging: {
        list: () => Promise<unknown>;
        summary: () => Promise<StagingSummary>;
        addText: (text: string) => Promise<unknown>;
        addPaths: (paths: string[]) => Promise<unknown>;
        addImageBytes: (label: string, bytes: number[], ext?: string) => Promise<unknown>;
        remove: (id: string) => Promise<unknown>;
        clear: () => Promise<unknown>;
        copy: (id: string) => Promise<unknown>;
        copyFiles: (id: string) => Promise<unknown>;
        copyAllPaths: () => Promise<unknown>;
        thumb: (id: string) => Promise<unknown>;
        reveal: (id: string) => Promise<unknown>;
        startDrag: (ids: string[]) => Promise<unknown>;
        pickFiles: () => Promise<unknown>;
        pickFolders: () => Promise<unknown>;
        subscribe: (cb: (summary: StagingSummary) => void) => () => void;
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
  // staging 补全后需重建；旧会话可能只有半套 hub
  if (window.hub?.pluginId === pluginId && window.hub.staging) return;
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
      remove: (key) => invoke("hub_storage_remove", withPlugin({ key })),
      listKeys: () => invoke("hub_storage_list_keys", withPlugin()),
      subscribe: (cb) => {
        let un = () => {};
        void listen<{
          pluginId?: string;
          key?: string;
          value?: unknown;
          removed?: boolean;
        }>("plugin-storage-changed", (ev) => {
          if (ev.payload?.pluginId && ev.payload.pluginId !== pluginId) return;
          if (!ev.payload?.key) return;
          cb({
            key: ev.payload.key,
            value: ev.payload.removed ? null : ev.payload.value,
            removed: !!ev.payload.removed,
          });
        }).then((fn) => {
          un = fn;
        });
        return () => un();
      },
    },
    shortcuts: {
      setBadge: (badge) => invoke("hub_shortcuts_set_badge", withPlugin({ badge })),
    },
    staging: {
      list: () => invoke("hub_staging_list", withPlugin()),
      summary: () => invoke("hub_staging_summary", withPlugin()),
      addText: (text) => invoke("hub_staging_add_text", withPlugin({ text })),
      addPaths: (paths) => invoke("hub_staging_add_paths", withPlugin({ paths })),
      addImageBytes: (label, bytes, ext) =>
        invoke("hub_staging_add_image_bytes", withPlugin({ label, bytes, ext })),
      remove: (id) => invoke("hub_staging_remove", withPlugin({ id })),
      clear: () => invoke("hub_staging_clear", withPlugin()),
      copy: (id) => invoke("hub_staging_copy", withPlugin({ id })),
      copyFiles: (id) => invoke("hub_staging_copy_files", withPlugin({ id })),
      copyAllPaths: () => invoke("hub_staging_copy_all_paths", withPlugin()),
      thumb: (id) => invoke("hub_staging_thumb", withPlugin({ id })),
      reveal: (id) => invoke("hub_staging_reveal", withPlugin({ id })),
      startDrag: (ids) => invoke("hub_staging_start_drag", withPlugin({ ids })),
        pickFiles: () => invoke("hub_staging_pick_files", withPlugin()),
        pickFolders: () => invoke("hub_staging_pick_folders", withPlugin()),
        subscribe: (cb) => {
        let un = () => {};
        void listen<StagingChangedPayload>("staging-changed", (ev) => {
          const { pluginId: pid, summary } = normalizeStagingChanged(ev.payload);
          if (pid && pid !== pluginId) return;
          cb(summary);
        }).then((fn) => {
          un = fn;
        });
        void invoke<StagingSummary>("hub_staging_summary", withPlugin())
          .then(cb)
          .catch(() => undefined);
        return () => un();
      },
    },
    notify: notifyFn,
    fetch: (url: string, opts?: Record<string, unknown>) =>
      invoke("hub_fetch", withPlugin({ url, opts: opts ?? null })),
    popup: {
      close: () => invoke("close_plugin_popup"),
    },
    applyEffect: (material?: string) =>
      material
        ? invoke("apply_window_effect", { material })
        : invoke("apply_window_effect", {}),
  };
}

type Boot = { css: string; js: string };

/**
 * Host shell: Tauri IPC + inject plugin CSS/JS from disk (independent package).
 */
export default function PluginPopupHost() {
  const pluginId = resolvePluginId();
  const [boot, setBoot] = useState<Boot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [phase, setPhase] = useState<"enter" | "in">("enter");
  const injectedRef = useRef(false);
  const glassReady = useRef(false);
  const phaseRef = useRef(phase);
  phaseRef.current = phase;

  /** Snap opaque — soft fade-from-0 while HWND is shown = empty mica flash. */
  function fadeIn() {
    if (phaseRef.current === "in") return;
    const root = document.querySelector(".plugin-popup-root") as HTMLElement | null;
    if (root) {
      root.style.transition = "none";
      root.classList.remove("is-enter");
      root.classList.add("is-in");
      window.requestAnimationFrame(() => {
        root.style.transition = "";
      });
    }
    phaseRef.current = "in";
    setPhase("in");
  }

  useEffect(() => {
    if (!pluginId) {
      setError("缺少插件 ID");
      return;
    }
    ensureHub(pluginId);

    // CSS vars only — Rust already applied DWM material on window create.
    // Re-calling apply_window_effect here causes a second visible flash.
    void (async () => {
      if (glassReady.current) return;
      try {
        const prefs = await invoke<GlassPrefs>("get_material_prefs");
        await syncGlassCss({ ...prefs, kind: normalizeGlassKind(prefs.kind) });
      } catch {
        await syncGlassCss({ kind: "mica-alt", dark: true });
      }
      glassReady.current = true;
    })();

    let cancelled = false;
    let unGlass: (() => void) | undefined;
    void listen<GlassPrefs>("material-prefs", (ev) => {
      void syncGlassCss({ ...ev.payload, kind: normalizeGlassKind(ev.payload.kind) });
      // Prefer CSS vars only; DWM reapply on every prefs event flashes popups.
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
        } catch {
          /* noop */
        }
      })();
    });

    let unOpened: (() => void) | undefined;
    let unClosed: (() => void) | undefined;
    void listen<string>("plugin-popup-opened", (ev) => {
      if (ev.payload && ev.payload !== pluginId) return;
      fadeIn();
    }).then((fn) => {
      if (cancelled) fn();
      else unOpened = fn;
    });
    void listen("plugin-popup-closed", () => {
      // Stay transparent while hidden so the next show() isn't an opaque flash.
      phaseRef.current = "enter";
      setPhase("enter");
    }).then((fn) => {
      if (cancelled) fn();
      else unClosed = fn;
    });

    void (async () => {
      try {
        const cached = popupAssetCache.get(pluginId);
        if (cached) {
          if (!cancelled) setBoot(cached);
          return;
        }
        // 约定：entry.popup 为 popup.html（或同目录），直接读兄弟 css/js，跳过 list_installed_plugins
        const [css, js] = await Promise.all([
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: "popup.css",
          }).catch(() => ""),
          invoke<string>("hub_plugin_read_text", {
            pluginId,
            relativePath: "popup.js",
          }),
        ]);
        if (cancelled) return;
        const next = { css, js };
        popupAssetCache.set(pluginId, next);
        setBoot(next);
        // Fade waits for inject + reveal_plugin_popup → plugin-popup-opened.
      } catch (err) {
        if (!cancelled) setError(String(err));
      }
    })();

    return () => {
      cancelled = true;
      unGlass?.();
      unOpened?.();
      unClosed?.();
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

    // Defer so #app from this render is in the DOM, then reveal HWND once painted.
    const t = window.setTimeout(() => {
      if (!document.getElementById("app")) {
        setError("插件挂载点 #app 缺失");
        return;
      }
      const script = document.createElement("script");
      script.textContent = boot.js;
      document.body.appendChild(script);
      // Opaque while still hidden, then show — avoids empty mica → content flash.
      const root = document.querySelector(".plugin-popup-root") as HTMLElement | null;
      if (root) {
        root.style.transition = "none";
        root.classList.remove("is-enter");
        root.classList.add("is-in");
      }
      phaseRef.current = "in";
      setPhase("in");
      void invoke("reveal_plugin_popup").catch(() => undefined);
      if (root) {
        window.requestAnimationFrame(() => {
          root.style.transition = "";
        });
      }
    }, 0);

    return () => window.clearTimeout(t);
  }, [boot, pluginId]);

  /** Explorer → 弹窗：走 Tauri paths（HTML5 File.path 经常为空） */
  useEffect(() => {
    if (!pluginId) return;
    let un: (() => void) | undefined;
    void getCurrentWindow()
      .onDragDropEvent((ev) => {
        const p = ev.payload;
        if (p.type !== "drop") return;
        const paths = p.paths ?? [];
        if (!paths.length) return;
        // 无 staging 能力的弹窗会失败，静默忽略
        void invoke("hub_staging_add_paths", { pluginId, paths }).catch(() => undefined);
      })
      .then((fn) => {
        un = fn;
      })
      .catch(() => undefined);
    return () => un?.();
  }, [pluginId]);

  if (error) {
    return (
      <div className={`plugin-popup-root is-${phase}`}>
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
      </div>
    );
  }

  // Always keep #app mounted — swapping "加载中" ↔ shell remounts and flashes.
  return (
    <div className={`plugin-popup-root is-${phase}`}>
      <main id="app" className="wg-shell" />
    </div>
  );
}

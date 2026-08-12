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
  const q = new URLSearchParams(window.location.search);
  // Prefetched warm shell has no plugin yet.
  if (q.get("warm") === "1" && !q.get("plugin")) return "";
  return q.get("plugin") ?? window.__WH_PLUGIN_ID__ ?? "";
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

type InstalledRow = {
  id: string;
  enabled: boolean;
  manifest?: { entry?: { popup?: string } };
};

function clearInjectedDom() {
  document.querySelectorAll("[data-wh-popup-css]").forEach((el) => el.remove());
  document.querySelectorAll("[data-wh-popup-js]").forEach((el) => el.remove());
  const app = document.getElementById("app");
  if (app) app.innerHTML = "";
  window.hub = undefined;
}

async function readPopupAssets(pluginId: string): Promise<Boot> {
  const cached = popupAssetCache.get(pluginId);
  if (cached) return cached;
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
  const next = { css, js };
  popupAssetCache.set(pluginId, next);
  return next;
}

function injectBoot(pluginId: string, boot: Boot) {
  if (boot.css) {
    const style = document.createElement("style");
    style.setAttribute("data-wh-popup-css", pluginId);
    style.textContent = boot.css;
    document.head.appendChild(style);
  }
  const mount = document.getElementById("app");
  if (!mount) throw new Error("插件挂载点 #app 缺失");
  const script = document.createElement("script");
  script.setAttribute("data-wh-popup-js", pluginId);
  script.textContent = boot.js;
  document.body.appendChild(script);
}

function snapOpaque() {
  const root = document.querySelector(".plugin-popup-root") as HTMLElement | null;
  if (root) {
    root.style.transition = "none";
    root.classList.remove("is-enter");
    root.classList.add("is-in");
    window.requestAnimationFrame(() => {
      root.style.transition = "";
    });
  }
}

/**
 * Host shell: Tauri IPC + inject plugin CSS/JS from disk (independent package).
 * Reuses one warm WebView and hot-swaps plugins (same pattern as Wi‑Fi flyout kind).
 */
export default function PluginPopupHost() {
  const [pluginId, setPluginId] = useState(() => resolvePluginId());
  const [error, setError] = useState<string | null>(null);
  const [phase, setPhase] = useState<"enter" | "in">("enter");
  const activeIdRef = useRef(pluginId);
  const loadSeqRef = useRef(0);
  const glassReady = useRef(false);
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  activeIdRef.current = pluginId;

  const activatePlugin = async (nextId: string) => {
    if (!nextId) return;
    const seq = ++loadSeqRef.current;
    setError(null);

    // Same plugin already painted — just reveal (Wi‑Fi reopen path).
    // Do NOT set opacity 0 first — that paints empty mica if HWND is/gets shown.
    if (activeIdRef.current === nextId && document.querySelector("[data-wh-popup-js]")) {
      snapOpaque();
      phaseRef.current = "in";
      setPhase("in");
      void invoke("reveal_plugin_popup").catch(() => undefined);
      return;
    }

    clearInjectedDom();
    activeIdRef.current = nextId;
    setPluginId(nextId);
    ensureHub(nextId);

    try {
      const boot = await readPopupAssets(nextId);
      if (seq !== loadSeqRef.current) return;
      injectBoot(nextId, boot);
      snapOpaque();
      phaseRef.current = "in";
      setPhase("in");
      void invoke("reveal_plugin_popup").catch(() => undefined);
    } catch (err) {
      if (seq !== loadSeqRef.current) return;
      setError(String(err));
      snapOpaque();
      phaseRef.current = "in";
      setPhase("in");
      // Show error UI rather than leaving a hidden/zombie shell.
      void invoke("reveal_plugin_popup").catch(() => undefined);
    }
  };

  // Glass + lifecycle listeners (once).
  useEffect(() => {
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

    const unsubs: Array<() => void> = [];
    void listen<GlassPrefs>("material-prefs", (ev) => {
      void syncGlassCss({ ...ev.payload, kind: normalizeGlassKind(ev.payload.kind) });
    }).then((fn) => unsubs.push(fn));

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

    void listen<string>("plugin-popup-opened", (ev) => {
      if (ev.payload && ev.payload !== activeIdRef.current) return;
      snapOpaque();
      phaseRef.current = "in";
      setPhase("in");
    }).then((fn) => unsubs.push(fn));

    void listen("plugin-popup-closed", () => {
      // HWND is hidden — keep content opaque for next show (never opacity 0 + mica).
      phaseRef.current = "in";
      setPhase("in");
    }).then((fn) => unsubs.push(fn));

    return () => {
      unsubs.forEach((fn) => fn());
      unSystem();
    };
  }, []);

  // Warm shell: prefetch popup.css/js so first open injects from memory.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await invoke<InstalledRow[]>("list_installed_plugins");
        if (cancelled) return;
        await Promise.all(
          list
            .filter((p) => p.enabled && p.manifest?.entry?.popup)
            .map(async (p) => {
              if (popupAssetCache.has(p.id)) return;
              try {
                await readPopupAssets(p.id);
              } catch {
                /* skip broken plugins */
              }
            }),
        );
      } catch {
        /* noop */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Hot-swap from Rust (CustomEvent + Tauri event), like flyout kind push.
  useEffect(() => {
    let lastId = "";
    let lastAt = 0;
    const requestActivate = (id: string) => {
      const now = Date.now();
      // eval CustomEvent + emit arrive together — activate once.
      if (id === lastId && now - lastAt < 120) return;
      lastId = id;
      lastAt = now;
      void activatePlugin(id);
    };

    const onCustom = (ev: Event) => {
      const detail = (ev as CustomEvent<{ pluginId?: string; preferGroupId?: string | null }>)
        .detail;
      const id = detail?.pluginId?.trim();
      if (!id) return;
      requestActivate(id);
    };
    window.addEventListener("wh-plugin-popup-load", onCustom);

    let unListen: (() => void) | undefined;
    void listen<{ pluginId?: string; preferGroupId?: string | null }>(
      "plugin-popup-load",
      (ev) => {
        const id = ev.payload?.pluginId?.trim();
        if (!id) return;
        requestActivate(id);
      },
    ).then((fn) => {
      unListen = fn;
    });

    return () => {
      window.removeEventListener("wh-plugin-popup-load", onCustom);
      unListen?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Cold start with ?plugin= in URL (no warm shell yet).
  useEffect(() => {
    const initial = resolvePluginId();
    if (!initial) return;
    void activatePlugin(initial);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      void invoke("close_plugin_popup").catch(() => undefined);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

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
        {/* Keep mount for next hot-swap */}
        <main id="app" className="wg-shell" hidden />
      </div>
    );
  }

  // Always keep #app mounted — warm shell + hot-swap inject into it.
  return (
    <div className={`plugin-popup-root is-${phase}`}>
      <main id="app" className="wg-shell" />
    </div>
  );
}

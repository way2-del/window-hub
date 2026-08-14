import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
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
        open: (id: string) => Promise<unknown>;
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

  const trackListen = (un: () => void) => {
    popupHubUnsubs.push(un);
  };

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
      trackListen(fn);
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
        let alive = true;
        let un = () => {};
        const wrapped = (windows: unknown) => {
          if (!alive) return;
          cb(windows);
        };
        void listen<{ windows: unknown }>("hub-windows-changed", (ev) => {
          if (ev.payload?.windows) wrapped(ev.payload.windows);
        }).then((fn) => {
          un = fn;
        });
        void invoke("hub_windows_list", withPlugin())
          .then((wins) => wrapped(wins))
          .catch(() => undefined);
        const kill = () => {
          alive = false;
          try {
            un();
          } catch {
            /* noop */
          }
        };
        trackListen(kill);
        return kill;
      },
    },
    storage: {
      get: (key) => invoke("hub_storage_get", withPlugin({ key })),
      set: (key, value) => invoke("hub_storage_set", withPlugin({ key, value })),
      remove: (key) => invoke("hub_storage_remove", withPlugin({ key })),
      listKeys: () => invoke("hub_storage_list_keys", withPlugin()),
      subscribe: (cb) => {
        let alive = true;
        let un = () => {};
        const wrapped = (ev: { key: string; value: unknown; removed: boolean }) => {
          if (!alive) return;
          cb(ev);
        };
        void listen<{
          pluginId?: string;
          key?: string;
          value?: unknown;
          removed?: boolean;
        }>("plugin-storage-changed", (ev) => {
          if (ev.payload?.pluginId && ev.payload.pluginId !== pluginId) return;
          if (!ev.payload?.key) return;
          wrapped({
            key: ev.payload.key,
            value: ev.payload.removed ? null : ev.payload.value,
            removed: !!ev.payload.removed,
          });
        }).then((fn) => {
          un = fn;
        });
        const kill = () => {
          alive = false;
          try {
            un();
          } catch {
            /* noop */
          }
        };
        trackListen(kill);
        return kill;
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
      open: (id) => invoke("hub_staging_open", withPlugin({ id })),
      startDrag: (ids) => invoke("hub_staging_start_drag", withPlugin({ ids })),
      pickFiles: () => invoke("hub_staging_pick_files", withPlugin()),
      pickFolders: () => invoke("hub_staging_pick_folders", withPlugin()),
      subscribe: (cb) => {
        let alive = true;
        let un = () => {};
        const wrapped = (summary: StagingSummary) => {
          if (!alive) return;
          cb(summary);
        };
        void listen<StagingChangedPayload>("staging-changed", (ev) => {
          const { pluginId: pid, summary } = normalizeStagingChanged(ev.payload);
          if (pid && pid !== pluginId) return;
          wrapped(summary);
        }).then((fn) => {
          un = fn;
        });
        void invoke<StagingSummary>("hub_staging_summary", withPlugin())
          .then(wrapped)
          .catch(() => undefined);
        const kill = () => {
          alive = false;
          try {
            un();
          } catch {
            /* noop */
          }
        };
        trackListen(kill);
        return kill;
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

/** Host-owned Tauri listens created via window.hub.* — must unlisten on hot-swap. */
const popupHubUnsubs: Array<() => void> = [];

function bumpPopupGen(): number {
  const w = window as Window & { __WH_POPUP_GEN__?: number };
  const next = (w.__WH_POPUP_GEN__ ?? 0) + 1;
  w.__WH_POPUP_GEN__ = next;
  return next;
}

function clearInjectedDom() {
  bumpPopupGen();
  try {
    window.dispatchEvent(new CustomEvent("wh-plugin-popup-dispose"));
  } catch {
    /* noop */
  }
  while (popupHubUnsubs.length) {
    const un = popupHubUnsubs.pop();
    try {
      un?.();
    } catch {
      /* noop */
    }
  }
  document.querySelectorAll("[data-wh-popup-css]").forEach((el) => el.remove());
  document.querySelectorAll("[data-wh-popup-js]").forEach((el) => el.remove());
  const app = document.getElementById("app");
  if (app) app.innerHTML = "";
  window.hub = undefined;
}

function popupScriptAttrSelector(pluginId: string): string {
  const safe = pluginId.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  return `[data-wh-popup-js="${safe}"]`;
}

/** 已绘制且 #app 有内容（仅有 script 标签不够，否则会 early-return 出白屏） */
function isPluginPainted(pluginId: string): boolean {
  if (!document.querySelector(popupScriptAttrSelector(pluginId))) return false;
  const app = document.getElementById("app");
  return !!app && app.childElementCount > 0;
}

/**
 * 等插件把 #app 画出来。很多 popup.js 会先 await storage 再 render，
 * 只等 1 帧会误判失败 → 用户必须再点一次。
 */
async function waitForPluginPaint(
  pluginId: string,
  isCurrent: () => boolean,
  budgetMs: number,
): Promise<boolean> {
  const start = performance.now();
  while (performance.now() - start < budgetMs) {
    if (!isCurrent()) return false;
    if (isPluginPainted(pluginId)) return true;
    await new Promise<void>((r) => requestAnimationFrame(() => r()));
  }
  return isCurrent() && isPluginPainted(pluginId);
}

async function readPopupAssets(pluginId: string, force = false): Promise<Boot> {
  if (!force) {
    const cached = popupAssetCache.get(pluginId);
    if (cached) return cached;
  }
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
  const gen =
    (window as Window & { __WH_POPUP_GEN__?: number }).__WH_POPUP_GEN__ ?? 0;
  const script = document.createElement("script");
  script.setAttribute("data-wh-popup-js", pluginId);
  // 必须 IIFE：经典脚本顶层 const/let 会污染全局，二次注入直接 SyntaxError → 清完 DOM 却画不出来（白屏）
  script.textContent =
    `window.__WH_POPUP_SCRIPT_GEN__=${gen};\n` +
    `(function(){\n"use strict";\n${boot.js}\n})();\n`;
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

function setLoadingMask(on: boolean) {
  const root = document.querySelector(".plugin-popup-root") as HTMLElement | null;
  if (!root) return;
  root.classList.toggle("is-loading", on);
}

function stashPreferGroup(gid?: string | null) {
  const id = typeof gid === "string" ? gid.trim() : "";
  if (!id) return;
  const w = window as Window & { __WH_PENDING_PREFER_GROUP__?: string | null };
  w.__WH_PENDING_PREFER_GROUP__ = id;
}

function takePreferGroup(): string | null {
  const w = window as Window & { __WH_PENDING_PREFER_GROUP__?: string | null };
  const id = typeof w.__WH_PENDING_PREFER_GROUP__ === "string"
    ? w.__WH_PENDING_PREFER_GROUP__.trim()
    : "";
  w.__WH_PENDING_PREFER_GROUP__ = null;
  return id || null;
}

function dispatchPreferGroup(gid: string) {
  try {
    window.dispatchEvent(
      new CustomEvent("wh-plugin-popup-prefer-group", { detail: gid }),
    );
  } catch {
    /* noop */
  }
}

/**
 * Host shell: Tauri IPC + inject plugin CSS/JS from disk (independent package).
 * Reuses one warm WebView and hot-swaps plugins (same pattern as Wi‑Fi flyout kind).
 *
 * `#app` is created imperatively — React must NOT own its children, or any
 * setState after injectBoot will reconcile `<main />` and wipe the plugin DOM
 * (white / empty mica shell after a few group switches).
 */
export default function PluginPopupHost() {
  const [error, setError] = useState<string | null>(null);
  const [phase, setPhase] = useState<"enter" | "in">("enter");
  const activeIdRef = useRef(resolvePluginId());
  const loadSeqRef = useRef(0);
  const glassReady = useRef(false);
  const phaseRef = useRef(phase);
  const mountHostRef = useRef<HTMLDivElement | null>(null);
  phaseRef.current = phase;

  // Imperative #app — stable across Host React re-renders.
  useEffect(() => {
    const host = mountHostRef.current;
    if (!host) return;
    let mount = document.getElementById("app") as HTMLElement | null;
    if (!mount) {
      mount = document.createElement("main");
      mount.id = "app";
      mount.className = "wg-shell";
      host.appendChild(mount);
    } else if (mount.parentElement !== host) {
      host.appendChild(mount);
    }
    // Never remove #app on Host unmount while HWND is recycled — keep warm.
  }, []);

  const activatePlugin = async (nextId: string) => {
    if (!nextId) return;
    const seq = ++loadSeqRef.current;

    // 同插件且 #app 仍有内容 → 只 reveal / 切换 prefer（避免无意义清空）
    if (activeIdRef.current === nextId && isPluginPainted(nextId)) {
      const prefer = takePreferGroup();
      if (prefer) dispatchPreferGroup(prefer);
      setError(null);
      setLoadingMask(false);
      snapOpaque();
      if (phaseRef.current !== "in") {
        phaseRef.current = "in";
        setPhase("in");
      }
      void invoke("reveal_plugin_popup").catch(() => undefined);
      return;
    }

    // 先读资源；清空必须紧贴 inject，否则 await 后被更新的 seq abort 会留下空壳白板
    let boot: Boot;
    try {
      boot = await readPopupAssets(nextId, true);
    } catch (err) {
      if (seq !== loadSeqRef.current) return;
      setLoadingMask(false);
      setError(String(err));
      activeIdRef.current = nextId;
      snapOpaque();
      phaseRef.current = "in";
      setPhase("in");
      void invoke("reveal_plugin_popup").catch(() => undefined);
      return;
    }
    if (seq !== loadSeqRef.current) return;

    setError(null);
    activeIdRef.current = nextId;

    try {
      // Ensure imperative mount exists before inject
      const host = mountHostRef.current;
      if (host && !document.getElementById("app")) {
        const mount = document.createElement("main");
        mount.id = "app";
        mount.className = "wg-shell";
        host.appendChild(mount);
      }
      if (!document.getElementById("app")) {
        await new Promise<void>((r) => requestAnimationFrame(() => r()));
      }
      if (seq !== loadSeqRef.current) return;

      // Cover clear→inject while HWND still visible (rapid click / hot-swap 卡白).
      setLoadingMask(true);
      clearInjectedDom();
      ensureHub(nextId);
      injectBoot(nextId, boot);

      const stillCurrent = () => seq === loadSeqRef.current;
      // 先等异步 boot 画壳；勿过早 clear+重注（会 dispose 掉进行中的首次 boot）。
      let painted = await waitForPluginPaint(nextId, stillCurrent, 700);
      if (!stillCurrent()) {
        setLoadingMask(false);
        return;
      }
      // 仍空：旧 WebView 失败态 / 脚本未挂上 → 再注一次
      if (!painted) {
        try {
          clearInjectedDom();
          ensureHub(nextId);
          injectBoot(nextId, boot);
        } catch (err) {
          setLoadingMask(false);
          setError(String(err));
          snapOpaque();
          phaseRef.current = "in";
          setPhase("in");
          void invoke("reveal_plugin_popup").catch(() => undefined);
          return;
        }
        painted = await waitForPluginPaint(nextId, stillCurrent, 500);
        if (!stillCurrent()) {
          setLoadingMask(false);
          return;
        }
      }

      let failed = false;
      if (!painted) {
        failed = true;
        setError("插件界面未能加载");
      }
      setLoadingMask(false);
      snapOpaque();
      phaseRef.current = "in";
      setPhase("in");
      // 有内容或错误提示后再 reveal，避免把空壳顶到前台
      if (painted || failed) {
        void invoke("reveal_plugin_popup").catch(() => undefined);
      }
    } catch (err) {
      if (seq !== loadSeqRef.current) return;
      setLoadingMask(false);
      setError(String(err));
      snapOpaque();
      phaseRef.current = "in";
      setPhase("in");
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

    void listen<string>("plugin-popup-opened", () => {
      // 勿在此写入 activeIdRef：hot-swap 时 opened 早于 inject，会让 activatePlugin 误 early-return。
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
      // push_plugin_popup_load 会同时 CustomEvent + Tauri emit，必须合并
      if (id === lastId && now - lastAt < 120) {
        if (isPluginPainted(id)) {
          const prefer = takePreferGroup();
          if (prefer) dispatchPreferGroup(prefer);
          return;
        }
        // 白屏空壳：不要 early-return，继续 activate 自愈
      }
      lastId = id;
      lastAt = now;
      void activatePlugin(id);
    };

    const onCustom = (ev: Event) => {
      const detail = (ev as CustomEvent<{ pluginId?: string; preferGroupId?: string | null }>)
        .detail;
      const id = detail?.pluginId?.trim();
      if (!id) return;
      stashPreferGroup(detail?.preferGroupId);
      requestActivate(id);
    };
    window.addEventListener("wh-plugin-popup-load", onCustom);

    let unListen: (() => void) | undefined;
    void listen<{ pluginId?: string; preferGroupId?: string | null }>(
      "plugin-popup-load",
      (ev) => {
        const id = ev.payload?.pluginId?.trim();
        if (!id) return;
        const prefer = ev.payload?.preferGroupId;
        stashPreferGroup(
          typeof prefer === "string" ? prefer : prefer != null ? String(prefer) : null,
        );
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

  /** Explorer → 弹窗：走 Tauri paths（HTML5 File.path 在 WebView2 上经常为空） */
  useEffect(() => {
    let cancelled = false;
    let un: (() => void) | undefined;
    void getCurrentWebview()
      .onDragDropEvent((ev) => {
        const p = ev.payload;
        if (p.type === "enter" || p.type === "over") {
          document.documentElement.classList.add("is-file-drag");
          return;
        }
        if (p.type === "leave" || p.type === "drop") {
          document.documentElement.classList.remove("is-file-drag");
        }
        if (p.type !== "drop") return;
        const paths = p.paths ?? [];
        if (!paths.length) return;
        // 用 activeIdRef：热切换后 React state 可能尚未提交
        const pid = activeIdRef.current;
        if (!pid) return;
        void invoke("hub_staging_add_paths", { pluginId: pid, paths }).catch((err) => {
          console.error("[PluginPopupHost] staging drop failed", pid, err);
        });
      })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        un = fn;
      })
      .catch((err) => {
        console.error("[PluginPopupHost] onDragDropEvent unavailable", err);
      });
    return () => {
      cancelled = true;
      document.documentElement.classList.remove("is-file-drag");
      un?.();
    };
    // 只挂一次；目标插件读 activeIdRef
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** HTML5 兜底：整窗 dragover 必须 preventDefault，否则系统显示禁止光标 */
  useEffect(() => {
    const allow = (e: DragEvent) => {
      e.preventDefault();
      if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    };
    document.addEventListener("dragenter", allow);
    document.addEventListener("dragover", allow);
    return () => {
      document.removeEventListener("dragenter", allow);
      document.removeEventListener("dragover", allow);
    };
  }, []);

  // Always keep imperative #app host — never swap trees on error (that remounts #app).
  return (
    <div className={`plugin-popup-root is-${phase}`}>
      {error ? (
        <div className="plugin-popup-frame plugin-popup-error-overlay">
          <div className="plugin-popup-empty">{error}</div>
          <button
            type="button"
            className="plugin-popup-close"
            onClick={() => void invoke("close_plugin_popup")}
          >
            关闭
          </button>
        </div>
      ) : null}
      <div ref={mountHostRef} className="plugin-popup-mount" />
    </div>
  );
}

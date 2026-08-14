import { invoke } from "@tauri-apps/api/core";
import { SHORTCUTS_HEIGHT } from "./shortcutsGeometry";

/** Shortcuts strip iframe ↔ host hub bridge (postMessage). */

export const WH_SHORTCUTS_HUB = "wh-shortcuts-hub";
export const WH_SHORTCUTS_HUB_RES = "wh-shortcuts-hub-res";
export const WH_SHORTCUTS_EVT = "wh-shortcuts-evt";

const ALLOWED_CMDS = new Set([
  "hub_storage_get",
  "hub_storage_set",
  "hub_storage_remove",
  "hub_storage_list_keys",
  "hub_settings_get_all",
  "hub_settings_get",
  "hub_settings_set",
  "hub_windows_list",
  "hub_windows_get",
  "hub_windows_focus",
  "get_foreground_app",
  "hub_notify",
  "hub_fetch",
  "hub_island_set_bar",
  "hub_island_clear_bar",
  "hub_netease_now_playing",
  "hub_panel_open_session",
  "hub_panel_close_session",
]);

export function isAllowedShortcutsHubCmd(cmd: string): boolean {
  return ALLOWED_CMDS.has(cmd);
}

/** Injected into shortcuts HTML so `window.hub` works without Tauri in the iframe. */
export function shortcutsHubBootstrapScript(
  pluginId: string,
  opts?: { suppressBlurOnPointerDown?: boolean },
): string {
  const suppressBlur = opts?.suppressBlurOnPointerDown === true;
  return `
(function () {
  const PLUGIN_ID = ${JSON.stringify(pluginId)};
  window.__WH_PLUGIN_ID__ = PLUGIN_ID;
  window.__WH_IS_PLUGIN_SHORTCUTS__ = true;
  const pending = new Map();
  window.addEventListener("message", function (ev) {
    const d = ev && ev.data;
    if (!d) return;
    if (d.channel === "${WH_SHORTCUTS_HUB_RES}") {
      const p = pending.get(d.id);
      if (!p) return;
      pending.delete(d.id);
      if (d.error) p.reject(new Error(d.error));
      else p.resolve(d.result);
      return;
    }
    if (d.channel === "${WH_SHORTCUTS_EVT}") {
      try {
        window.dispatchEvent(new CustomEvent("wh-shortcuts-evt", { detail: d }));
      } catch (_) {}
    }
  });
  function invoke(cmd, args) {
    return new Promise(function (resolve, reject) {
      const id = Math.random().toString(36).slice(2);
      pending.set(id, { resolve: resolve, reject: reject });
      window.parent.postMessage(
        { channel: "${WH_SHORTCUTS_HUB}", id: id, cmd: cmd, args: args || {}, pluginId: PLUGIN_ID },
        "*"
      );
    });
  }
  function hostCmd(cmd, args) {
    window.parent.postMessage(
      { channel: "${WH_SHORTCUTS_HUB}", cmd: cmd, args: args || {}, pluginId: PLUGIN_ID },
      "*"
    );
  }
  function withPlugin(args) {
    return Object.assign({ pluginId: PLUGIN_ID }, args || {});
  }
  window.hub = {
    pluginId: PLUGIN_ID,
    storage: {
      get: function (key) { return invoke("hub_storage_get", withPlugin({ key: key })); },
      set: function (key, value) { return invoke("hub_storage_set", withPlugin({ key: key, value: value })); },
      remove: function (key) { return invoke("hub_storage_remove", withPlugin({ key: key })); },
      listKeys: function () { return invoke("hub_storage_list_keys", withPlugin()); },
      subscribe: function (cb) {
        if (typeof cb !== "function") return function () {};
        function onEvt(ev) {
          var d = ev && ev.detail;
          if (!d || d.type !== "storage-changed") return;
          try {
            cb({
              key: d.key,
              value: d.removed ? null : d.value,
              removed: !!d.removed
            });
          } catch (_) {}
        }
        window.addEventListener("wh-shortcuts-evt", onEvt);
        return function () { window.removeEventListener("wh-shortcuts-evt", onEvt); };
      }
    },
    settings: {
      getAll: function () { return invoke("hub_settings_get_all", withPlugin()); },
      get: function (key) { return invoke("hub_settings_get", withPlugin({ key: key })); },
      set: function (key, value) { return invoke("hub_settings_set", withPlugin({ key: key, value: value })); },
      subscribe: function (cb) {
        function onEvt(ev) {
          var d = ev && ev.detail;
          if (!d || d.type !== "settings-changed") return;
          try { cb(d.settings || {}); } catch (_) {}
        }
        window.addEventListener("wh-shortcuts-evt", onEvt);
        invoke("hub_settings_get_all", withPlugin()).then(cb).catch(function () {});
        return function () { window.removeEventListener("wh-shortcuts-evt", onEvt); };
      }
    },
    windows: {
      list: function () { return invoke("hub_windows_list", withPlugin()); },
      get: function (id) { return invoke("hub_windows_get", withPlugin({ id: id })); },
      focus: function (id) { return invoke("hub_windows_focus", withPlugin({ id: id })); },
      subscribe: function (cb) {
        function onEvt(ev) {
          var d = ev && ev.detail;
          if (!d || d.type !== "windows-changed") return;
          try { cb(d.windows || []); } catch (_) {}
        }
        window.addEventListener("wh-shortcuts-evt", onEvt);
        invoke("hub_windows_list", withPlugin()).then(cb).catch(function () {});
        return function () { window.removeEventListener("wh-shortcuts-evt", onEvt); };
      }
    },
    shortcuts: {
      requestSize: function (size) {
        var w = size && typeof size.width === "number" ? size.width : 0;
        hostCmd("shortcuts.requestSize", { width: w });
      },
      /** Status-menu bar / shortcuts strip geometry (logical px). */
      getBounds: function () {
        return invoke("shortcuts.getBounds", {});
      }
    },
    popup: {
      open: function (opts) { hostCmd("popup.open", opts || {}); },
      close: function () { return invoke("close_plugin_popup", {}); }
    },
    island: {
      setBar: function (opts) {
        return invoke("hub_island_set_bar", withPlugin({
          text: (opts && opts.text) || "",
          title: opts && opts.title,
          image: opts && opts.image,
          mirror: !!(opts && opts.mirror)
        }));
      },
      clearBar: function () { return invoke("hub_island_clear_bar", withPlugin()); }
    },
    media: {
      neteaseNowPlaying: function () {
        return invoke("hub_netease_now_playing", withPlugin());
      }
    },
    panel: {
      openSession: function () { return invoke("hub_panel_open_session", withPlugin()); },
      closeSession: function () { return invoke("hub_panel_close_session", {}); },
      close: function () { return invoke("hub_panel_close_session", {}); }
    },
    fetch: function (url, opts) {
      return invoke("hub_fetch", withPlugin({ url: url, opts: opts || null }));
    },
    foreground: {
      get: function () { return invoke("get_foreground_app", {}); },
      subscribe: function (cb) {
        function onEvt(ev) {
          var d = ev && ev.detail;
          if (!d || d.type !== "foreground-changed") return;
          try { cb(d); } catch (_) {}
        }
        window.addEventListener("wh-shortcuts-evt", onEvt);
        invoke("get_foreground_app", {}).then(cb).catch(function () {});
        return function () { window.removeEventListener("wh-shortcuts-evt", onEvt); };
      }
    }
  };
  var notifyFn = function (opts) {
    return invoke("hub_notify", withPlugin({
      opts: {
        title: (opts && opts.title) || "",
        body: opts && opts.body,
        iconPng: opts && opts.iconPng,
        urgency: opts && opts.urgency,
        ttlMs: opts && opts.ttlMs,
        actions: opts && opts.actions,
        data: opts && opts.data
      }
    }));
  };
  notifyFn.onAction = function (cb) {
    function onEvt(ev) {
      var d = ev && ev.detail;
      if (!d || d.type !== "notify-action") return;
      try {
        cb({
          notifyId: d.notifyId,
          actionId: d.actionId,
          data: d.data
        });
      } catch (_) {}
    }
    window.addEventListener("wh-shortcuts-evt", onEvt);
    return function () { window.removeEventListener("wh-shortcuts-evt", onEvt); };
  };
  window.hub.notify = notifyFn;
  // 仅 popup 入口插件：点条内芯片时 suppress，避免失焦先关弹窗再竞态。
  // command / panel 插件禁止 suppress，否则窗口组会 always-on-top 残留。
  ${
    suppressBlur
      ? `document.addEventListener("pointerdown", function () {
    try { hostCmd("suppress_plugin_popup_blur", { ms: 280 }); } catch (_) {}
  }, true);`
      : ""
  }
  window.addEventListener("wh-shortcuts-evt", function (ev) {
    var d = ev && ev.detail;
    if (!d || d.type !== "refresh") return;
    try {
      window.dispatchEvent(new CustomEvent("wh-shortcuts-refresh"));
    } catch (_) {}
  });
})();
`;
}

export async function buildShortcutsSrcdoc(
  pluginId: string,
  entryPath: string,
  opts?: { suppressBlurOnPointerDown?: boolean },
): Promise<string> {
  let html = await invoke<string>("hub_plugin_read_text", {
    pluginId,
    relativePath: entryPath,
  });
  const dir = entryPath.includes("/")
    ? entryPath.slice(0, entryPath.lastIndexOf("/") + 1)
    : entryPath.includes("\\")
      ? entryPath.slice(0, entryPath.lastIndexOf("\\") + 1)
      : "";
  const stem = entryPath.replace(/^.*[\\/]/, "").replace(/\.html?$/i, "") || "shortcuts";
  const [css, js] = await Promise.all([
    invoke<string>("hub_plugin_read_text", {
      pluginId,
      relativePath: `${dir}${stem}.css`,
    }).catch(() => ""),
    invoke<string>("hub_plugin_read_text", {
      pluginId,
      relativePath: `${dir}${stem}.js`,
    }).catch(() => ""),
  ]);
  if (css) {
    const linkRe = new RegExp(
      `<link[^>]*href=["'](?:\\.\\/)?${stem}\\.css["'][^>]*>`,
      "i",
    );
    if (linkRe.test(html)) {
      html = html.replace(linkRe, `<style id="wh-plugin-shortcuts-css">${css}</style>`);
    } else if (/<\/head>/i.test(html)) {
      html = html.replace(/<\/head>/i, `<style id="wh-plugin-shortcuts-css">${css}</style></head>`);
    } else {
      html = `<style id="wh-plugin-shortcuts-css">${css}</style>${html}`;
    }
  }
  const barH = SHORTCUTS_HEIGHT;
  // Host chrome + manage-icon center (must not depend on plugin CSS link resolve)
  const shellCss = `<style id="wh-shortcuts-shell">
*{box-sizing:border-box;border:none!important;outline:none!important;box-shadow:none!important}
:root{--wh-bar-h:${barH}px}
html,body{margin:0;padding:0;overflow:hidden!important;background:transparent!important;height:var(--wh-bar-h,${barH}px);max-height:var(--wh-bar-h,${barH}px);width:max-content;min-width:${barH}px;scrollbar-width:none;color:var(--wh-chrome-fg,rgba(255,255,255,.94));text-shadow:var(--wh-chrome-shadow,0 1px 2px rgba(0,0,0,.35));display:flex;align-items:center}
button{border:none!important;background:transparent!important;outline:none!important;box-shadow:none!important;-webkit-appearance:none!important;appearance:none!important;border-radius:0!important;color:inherit;font:inherit}
.wg-chip.is-manage{position:relative!important;display:inline-flex!important;align-items:center!important;justify-content:center!important;width:${barH}px!important;min-width:${barH}px!important;height:var(--wh-bar-h,${barH}px)!important;max-height:var(--wh-bar-h,${barH}px)!important;padding:0!important;margin:0!important;line-height:0!important;text-shadow:none!important}
.wg-chip.is-manage .wg-chip-icon{position:absolute!important;left:50%!important;top:50%!important;width:13px!important;height:13px!important;margin:0!important;padding:0!important;transform:translate(-50%,-50%)!important;display:block!important;overflow:visible!important;text-shadow:none!important;filter:none!important;pointer-events:none!important}
::-webkit-scrollbar{display:none!important;width:0!important;height:0!important}
</style>`;
  const boot = `${shellCss}<script>${shortcutsHubBootstrapScript(pluginId, {
    suppressBlurOnPointerDown: opts?.suppressBlurOnPointerDown,
  })}</script>`;
  const bodyJs = js ? `<script>${js}</script>` : "";
  html = html.replace(
    new RegExp(
      `<script[^>]*src=["'](?:\\.\\/)?${stem}\\.js["'][^>]*>\\s*</script>`,
      "i",
    ),
    "",
  );
  if (!/<html\b/i.test(html)) {
    html = `<html>${html}</html>`;
  }
  const injected = /<head[^>]*>/i.test(html)
    ? html.replace(/<head[^>]*>/i, (m) => `${m}${boot}`)
    : `${boot}${html}`;
  return /<\/body>/i.test(injected)
    ? injected.replace(/<\/body>/i, `${bodyJs}</body>`)
    : `${injected}${bodyJs}`;
}

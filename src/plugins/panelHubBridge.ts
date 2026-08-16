/** Shared panel ↔ host hub bridge (postMessage). */

export const WH_PANEL_HUB = "wh-panel-hub";
export const WH_PANEL_HUB_RES = "wh-panel-hub-res";

const ALLOWED_CMDS = new Set([
  "hub_storage_get",
  "hub_storage_set",
  "hub_storage_remove",
  "hub_storage_list_keys",
  "hub_settings_get_all",
  "hub_settings_get",
  "hub_settings_set",
  "hub_staging_list",
  "hub_staging_summary",
  "hub_staging_add_text",
  "hub_staging_add_paths",
  "hub_staging_add_image_bytes",
  "hub_staging_remove",
  "hub_staging_clear",
  "hub_staging_copy",
  "hub_staging_copy_all_paths",
  "hub_staging_thumb",
  "hub_staging_reveal",
  "hub_staging_start_drag",
  "hub_island_set_bar",
  "hub_island_clear_bar",
  "hub_island_claim_scenario",
  "hub_island_release_scenario",
  "hub_island_get_bound_tray",
  "hub_island_open_bound_tray",
  "hub_panel_open_session",
  "hub_panel_close_session",
  "hub_windows_list",
  "hub_windows_get",
  "hub_windows_focus",
  "hub_media_send_key",
  "hub_notify",
  "hub_fetch",
  "hub_everything_status",
  "hub_everything_search",
  "hub_everything_open",
  "hub_everything_reveal",
  "hub_sysmon_snapshot",
]);

export function isAllowedPanelHubCmd(cmd: string): boolean {
  return ALLOWED_CMDS.has(cmd);
}

/** Injected into panel HTML so `window.hub` works without Tauri in the iframe. */
export function panelHubBootstrapScript(pluginId: string): string {
  return `
(function () {
  const PLUGIN_ID = ${JSON.stringify(pluginId)};
  window.__WH_PLUGIN_ID__ = PLUGIN_ID;
  window.__WH_IS_PLUGIN_PANEL__ = true;
  const pending = new Map();
  window.addEventListener("message", function (ev) {
    const d = ev && ev.data;
    if (!d || d.channel !== "${WH_PANEL_HUB_RES}") return;
    const p = pending.get(d.id);
    if (!p) return;
    pending.delete(d.id);
    if (d.error) p.reject(new Error(d.error));
    else p.resolve(d.result);
  });
  function invoke(cmd, args) {
    return new Promise(function (resolve, reject) {
      const id = Math.random().toString(36).slice(2);
      pending.set(id, { resolve: resolve, reject: reject });
      window.parent.postMessage(
        { channel: "${WH_PANEL_HUB}", id: id, cmd: cmd, args: args || {} },
        "*"
      );
    });
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
      listKeys: function () { return invoke("hub_storage_list_keys", withPlugin()); }
    },
    settings: {
      getAll: function () { return invoke("hub_settings_get_all", withPlugin()); },
      get: function (key) { return invoke("hub_settings_get", withPlugin({ key: key })); },
      set: function (key, value) { return invoke("hub_settings_set", withPlugin({ key: key, value: value })); },
      subscribe: function (cb) {
        function onMsg(ev) {
          var d = ev && ev.data;
          if (!d || d.channel !== "plugin-settings-changed-fwd") return;
          if (d.pluginId && d.pluginId !== PLUGIN_ID) return;
          try { cb(d.settings || {}); } catch (_) {}
        }
        window.addEventListener("message", onMsg);
        invoke("hub_settings_get_all", withPlugin()).then(cb).catch(function () {});
        return function () { window.removeEventListener("message", onMsg); };
      }
    },
    staging: {
      list: function () { return invoke("hub_staging_list", withPlugin()); },
      summary: function () { return invoke("hub_staging_summary", withPlugin()); },
      addText: function (text) { return invoke("hub_staging_add_text", withPlugin({ text: text })); },
      addPaths: function (paths) { return invoke("hub_staging_add_paths", withPlugin({ paths: paths })); },
      addImageBytes: function (label, bytes, ext) {
        return invoke("hub_staging_add_image_bytes", withPlugin({ label: label, bytes: bytes, ext: ext }));
      },
      remove: function (id) { return invoke("hub_staging_remove", withPlugin({ id: id })); },
      clear: function () { return invoke("hub_staging_clear", withPlugin()); },
      copy: function (id) { return invoke("hub_staging_copy", withPlugin({ id: id })); },
      copyAllPaths: function () { return invoke("hub_staging_copy_all_paths", withPlugin()); },
      thumb: function (id) { return invoke("hub_staging_thumb", withPlugin({ id: id })); },
      reveal: function (id) { return invoke("hub_staging_reveal", withPlugin({ id: id })); },
      startDrag: function (ids) { return invoke("hub_staging_start_drag", withPlugin({ ids: ids })); },
      subscribe: function (cb) {
        function onMsg(ev) {
          var d = ev && ev.data;
          if (!d || d.channel !== "staging-changed-fwd") return;
          if (d.pluginId && d.pluginId !== PLUGIN_ID) return;
          try { cb(d.summary); } catch (_) {}
        }
        window.addEventListener("message", onMsg);
        invoke("hub_staging_summary", withPlugin()).then(cb).catch(function () {});
        return function () { window.removeEventListener("message", onMsg); };
      }
    },
    island: {
      setBar: function (opts) {
        return invoke("hub_island_set_bar", withPlugin({
          text: (opts && opts.text) || "",
          title: opts && opts.title
        }));
      },
      clearBar: function () { return invoke("hub_island_clear_bar", withPlugin()); },
      claimScenario: function () { return invoke("hub_island_claim_scenario", withPlugin()); },
      releaseScenario: function () { return invoke("hub_island_release_scenario", withPlugin()); },
      /** Plugin settings openTrayKey — null if unbound. */
      getBoundTray: function () { return invoke("hub_island_get_bound_tray", withPlugin()); },
      /** Left-click the bound tray icon (open app). */
      openBoundTray: function () { return invoke("hub_island_open_bound_tray", withPlugin()); }
    },
    fetch: function (url, opts) {
      return invoke("hub_fetch", withPlugin({ url: url, opts: opts || null }));
    },
    media: {
      sendKey: function (action) {
        return invoke("hub_media_send_key", withPlugin({ action: action }));
      }
    },
    everything: {
      status: function () { return invoke("hub_everything_status", withPlugin()); },
      search: function (query, opts) {
        return invoke("hub_everything_search", withPlugin({
          query: query || "",
          opts: opts || null
        }));
      },
      open: function (path) {
        return invoke("hub_everything_open", withPlugin({ path: path || "" }));
      },
      reveal: function (path) {
        return invoke("hub_everything_reveal", withPlugin({ path: path || "" }));
      }
    },
    sysmon: {
      snapshot: function () { return invoke("hub_sysmon_snapshot", withPlugin()); }
    },
    panel: {
      close: function () {
        window.parent.postMessage({ channel: "${WH_PANEL_HUB}", cmd: "panel.close", args: {} }, "*");
      },
      openSession: function () { return invoke("hub_panel_open_session", withPlugin()); },
      closeSession: function () { return invoke("hub_panel_close_session", {}); },
      /** Host 在岛完全展开后 enter；收起一开始 leave。摄像头等重资源只在 onEnter 开。 */
      onEnter: function (cb) {
        if (typeof cb !== "function") return function () {};
        enterCbs.push(cb);
        if (panelPhase === "enter") {
          try { cb(); } catch (_) {}
        }
        return function () {
          var i = enterCbs.indexOf(cb);
          if (i >= 0) enterCbs.splice(i, 1);
        };
      },
      onLeave: function (cb) {
        if (typeof cb !== "function") return function () {};
        leaveCbs.push(cb);
        if (panelPhase === "leave") {
          try { cb(); } catch (_) {}
        }
        return function () {
          var i = leaveCbs.indexOf(cb);
          if (i >= 0) leaveCbs.splice(i, 1);
        };
      }
    }
  };
  var panelPhase = "leave";
  var enterCbs = [];
  var leaveCbs = [];
  window.addEventListener("message", function (ev) {
    var d = ev && ev.data;
    if (!d || d.channel !== "island-panel-lifecycle-fwd") return;
    if (d.pluginId && d.pluginId !== PLUGIN_ID) return;
    var phase = d.phase === "enter" ? "enter" : "leave";
    if (phase === panelPhase) return;
    panelPhase = phase;
    var list = phase === "enter" ? enterCbs : leaveCbs;
    for (var i = 0; i < list.length; i++) {
      try { list[i](); } catch (_) {}
    }
  });
  window.addEventListener("message", function (ev) {
    var d = ev && ev.data;
    if (!d || d.channel !== "island-search-fwd") return;
    if (d.pluginId && d.pluginId !== PLUGIN_ID) return;
    try {
      window.dispatchEvent(new CustomEvent("wh-island-search", { detail: d }));
    } catch (_) {}
  });
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
    function onMsg(ev) {
      var d = ev && ev.data;
      if (!d || d.channel !== "island-notify-action-fwd") return;
      if (d.pluginId && d.pluginId !== PLUGIN_ID) return;
      try {
        cb({
          notifyId: d.notifyId,
          actionId: d.actionId,
          data: d.data
        });
      } catch (_) {}
    }
    window.addEventListener("message", onMsg);
    return function () { window.removeEventListener("message", onMsg); };
  };
  window.hub.notify = notifyFn;
})();
`;
}

import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import HotkeyRecorder from "./HotkeyRecorder";
import { pushSettingsToast } from "./settingsToastBus";

export type HotkeyBinding = {
  id: string;
  scope: string;
  pluginId?: string;
  pluginName?: string;
  key: string;
  label: string;
  action: string;
  chord: string;
  aliasesSystemSearch?: boolean;
};

function errText(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error && err.message) return err.message;
  if (err && typeof err === "object") {
    const o = err as { message?: unknown; error?: unknown };
    if (typeof o.message === "string" && o.message.trim()) return o.message;
    if (typeof o.error === "string" && o.error.trim()) return o.error;
  }
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

export default function HotkeysSettingsPanel() {
  const [bindings, setBindings] = useState<HotkeyBinding[]>([]);
  const [busy, setBusy] = useState(false);

  const showToast = useCallback((message: string | null) => {
    pushSettingsToast(message == null || message === "" ? null : errText(message));
  }, []);

  const refresh = useCallback(async () => {
    try {
      const list = await invoke<HotkeyBinding[]>("list_hotkey_bindings");
      setBindings(list ?? []);
    } catch (err) {
      showToast(errText(err));
    }
  }, [showToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let un: (() => void) | undefined;
    void listen("hotkeys-changed", () => {
      void refresh();
    }).then((fn) => {
      un = fn;
    });
    return () => un?.();
  }, [refresh]);

  useEffect(() => {
    return () => pushSettingsToast(null);
  }, []);

  const system = useMemo(
    () =>
      bindings.filter(
        (b) => b.scope === "system" && b.id !== "system.islandSearch",
      ),
    [bindings],
  );
  const plugins = useMemo(() => {
    const map = new Map<string, { name: string; rows: HotkeyBinding[] }>();
    for (const b of bindings) {
      if (b.scope !== "plugin" || !b.pluginId) continue;
      const g = map.get(b.pluginId) ?? {
        name: b.pluginName || b.pluginId,
        rows: [],
      };
      g.rows.push(b);
      map.set(b.pluginId, g);
    }
    return [...map.entries()].map(([id, g]) => ({ id, ...g }));
  }, [bindings]);

  const commit = async (id: string, chord: string) => {
    setBusy(true);
    try {
      const list = await invoke<HotkeyBinding[]>("set_hotkey_binding", {
        id,
        chord,
      });
      setBindings(list ?? []);
      pushSettingsToast(null);
    } catch (err) {
      showToast(errText(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="hotkeys-settings">
      <section className="settings-card">
        <h2>系统快捷键</h2>
        <p className="card-desc">
          主机内置全局热键。岛栏搜索热键在下方「文件搜索」插件中配置。留空表示禁用。
        </p>
        {system.length === 0 ? (
          <p className="card-desc">暂无其它系统热键。</p>
        ) : (
          system.map((b) => (
            <div key={b.id} className="pref-row">
              <span className="pref-row-text">
                <span className="pref-row-label">{b.label}</span>
                <span className="pref-row-desc">{b.action}</span>
              </span>
              <HotkeyRecorder
                bindingId={b.id}
                value={b.chord}
                disabled={busy}
                onError={showToast}
                onCommit={(c) => commit(b.id, c)}
              />
            </div>
          ))
        )}
      </section>

      <section className="settings-card">
        <h2>插件快捷键</h2>
        <p className="card-desc">
          来自已启用插件的热键项；同一组合键只能绑定一处。
        </p>
        {plugins.length === 0 ? (
          <p className="card-desc">暂无插件声明全局热键。</p>
        ) : (
          plugins.map((g) => (
            <div key={g.id} className="hotkeys-plugin-group">
              <h3 className="hotkeys-plugin-title">{g.name}</h3>
              {g.rows.map((b) => (
                <div key={b.id} className="pref-row">
                  <span className="pref-row-text">
                    <span className="pref-row-label">{b.label}</span>
                    <span className="pref-row-desc">
                      {b.aliasesSystemSearch
                        ? "岛栏搜索（系统动作 island.search.toggle）"
                        : b.action}
                    </span>
                  </span>
                  <HotkeyRecorder
                    bindingId={
                      b.aliasesSystemSearch ? "system.islandSearch" : b.id
                    }
                    value={b.chord}
                    disabled={busy}
                    onError={showToast}
                    onCommit={(c) =>
                      commit(
                        b.aliasesSystemSearch ? "system.islandSearch" : b.id,
                        c,
                      )
                    }
                  />
                </div>
              ))}
            </div>
          ))
        )}
      </section>
    </div>
  );
}

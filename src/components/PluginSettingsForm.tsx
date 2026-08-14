import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PluginSettingField } from "../plugins/types";
import { OPEN_TRAY_SETTING_KEY } from "../scenarioGates";
import OpenTraySetting from "./OpenTraySetting";

type Props = {
  pluginId: string;
  fields: PluginSettingField[];
  description?: string;
};

function optionKey(v: unknown): string {
  return JSON.stringify(v);
}

/**
 * Renders plugin.json `settings[]` and persists via hub_settings_set → __settings.
 */
export default function PluginSettingsForm({ pluginId, fields, description }: Props) {
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void invoke<Record<string, unknown>>("hub_settings_get_all", { pluginId })
      .then((all) => {
        if (!cancelled) setValues(all ?? {});
      })
      .catch((err) => {
        if (!cancelled) setError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [pluginId]);

  useEffect(() => {
    let un: (() => void) | undefined;
    void listen<{ pluginId?: string; settings?: Record<string, unknown> }>(
      "plugin-settings-changed",
      (ev) => {
        if (ev.payload?.pluginId !== pluginId) return;
        setValues(ev.payload.settings ?? {});
      },
    ).then((fn) => {
      un = fn;
    });
    return () => un?.();
  }, [pluginId]);

  const setField = async (key: string, value: unknown) => {
    setBusy(true);
    setError(null);
    try {
      const next = await invoke<Record<string, unknown>>("hub_settings_set", {
        pluginId,
        key,
        value,
      });
      setValues(next ?? {});
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  if (!fields.length) return null;

  return (
    <div className="plugin-settings">
      {description ? <p className="plugin-settings-desc">{description}</p> : null}
      {fields.map((field) => {
        if (field.key === OPEN_TRAY_SETTING_KEY) {
          return (
            <OpenTraySetting
              key={field.key}
              pluginId={pluginId}
              label={field.label}
              description={
                field.description ||
                "面板右下角「打开应用」会左键该托盘"
              }
              disabled={busy}
            />
          );
        }
        if (field.uiHidden) return null;
        const value = values[field.key] ?? field.default;
        const desc = field.description;
        if (field.type === "boolean") {
          const on = Boolean(value);
          return (
            <label key={field.key} className="pref-row">
              <span className="pref-row-text">
                <span className="pref-row-label">{field.label}</span>
                {desc ? <span className="pref-row-desc">{desc}</span> : null}
              </span>
              <button
                type="button"
                className={`pref-switch${on ? " is-on" : ""}`}
                role="switch"
                aria-checked={on}
                disabled={busy}
                onClick={() => void setField(field.key, !on)}
              >
                <span className="pref-switch-knob" />
              </button>
            </label>
          );
        }
        if (field.type === "select") {
          return (
            <label key={field.key} className="pref-row">
              <span className="pref-row-text">
                <span className="pref-row-label">{field.label}</span>
                {desc ? <span className="pref-row-desc">{desc}</span> : null}
              </span>
              <select
                className="pref-select"
                disabled={busy}
                value={optionKey(value)}
                onChange={(e) => {
                  const opt = (field.options ?? []).find(
                    (o) => optionKey(o.value) === e.target.value,
                  );
                  if (opt) void setField(field.key, opt.value);
                }}
              >
                {(field.options ?? []).map((o) => (
                  <option key={optionKey(o.value)} value={optionKey(o.value)}>
                    {o.label}
                  </option>
                ))}
              </select>
            </label>
          );
        }
        if (field.type === "radio") {
          return (
            <div key={field.key} className="pref-row is-stack">
              <span className="pref-row-text">
                <span className="pref-row-label">{field.label}</span>
                {desc ? <span className="pref-row-desc">{desc}</span> : null}
              </span>
              <div className="plugin-settings-radios">
                {(field.options ?? []).map((o) => (
                  <label key={optionKey(o.value)} className="plugin-settings-radio">
                    <input
                      type="radio"
                      name={`${pluginId}-${field.key}`}
                      disabled={busy}
                      checked={optionKey(value) === optionKey(o.value)}
                      onChange={() => void setField(field.key, o.value)}
                    />
                    <span>{o.label}</span>
                  </label>
                ))}
              </div>
            </div>
          );
        }
        if (field.type === "multiSelect") {
          const selected = Array.isArray(value) ? (value as unknown[]) : [];
          return (
            <div key={field.key} className="pref-row is-stack">
              <span className="pref-row-text">
                <span className="pref-row-label">{field.label}</span>
                {desc ? <span className="pref-row-desc">{desc}</span> : null}
              </span>
              <div className="plugin-settings-checks">
                {(field.options ?? []).map((o) => {
                  const checked = selected.some((s) => optionKey(s) === optionKey(o.value));
                  return (
                    <label key={optionKey(o.value)} className="plugin-settings-check">
                      <input
                        type="checkbox"
                        disabled={busy}
                        checked={checked}
                        onChange={() => {
                          const next = checked
                            ? selected.filter((s) => optionKey(s) !== optionKey(o.value))
                            : [...selected, o.value];
                          void setField(field.key, next);
                        }}
                      />
                      <span>{o.label}</span>
                    </label>
                  );
                })}
              </div>
            </div>
          );
        }
        if (field.type === "number") {
          return (
            <label key={field.key} className="pref-row">
              <span className="pref-row-text">
                <span className="pref-row-label">{field.label}</span>
                {desc ? <span className="pref-row-desc">{desc}</span> : null}
              </span>
              <input
                type="number"
                className="pref-input"
                disabled={busy}
                min={field.min}
                max={field.max}
                step={field.step ?? 1}
                value={typeof value === "number" ? value : Number(value) || 0}
                onChange={(e) => void setField(field.key, Number(e.target.value))}
              />
            </label>
          );
        }
        // string
        return (
          <label key={field.key} className="pref-row">
            <span className="pref-row-text">
              <span className="pref-row-label">{field.label}</span>
              {desc ? <span className="pref-row-desc">{desc}</span> : null}
            </span>
            <input
              type="text"
              className="pref-input"
              disabled={busy}
              maxLength={field.maxLength}
              value={typeof value === "string" ? value : String(value ?? "")}
              onChange={(e) => void setField(field.key, e.target.value)}
            />
          </label>
        );
      })}
      {error ? <p className="plugin-msg">{error}</p> : null}
    </div>
  );
}

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { OPEN_TRAY_SETTING_KEY } from "../scenarioGates";

type TrayIconInfo = {
  id: string;
  pin_key?: string;
  tooltip: string;
  process: string;
  icon_png_base64: string;
};

type Props = {
  pluginId: string;
  label?: string;
  description?: string;
  disabled?: boolean;
};

function trayPinKey(icon: TrayIconInfo): string {
  const k = (icon.pin_key || "").trim();
  return k || icon.id;
}

function trayLabel(icon: TrayIconInfo) {
  return (icon.tooltip || "").trim() || icon.process || icon.id;
}

/**
 * Plugin settings row for `openTrayKey` — tray single-select (not a plain string input).
 */
export default function OpenTraySetting({
  pluginId,
  label = "打开应用（绑定托盘）",
  description,
  disabled = false,
}: Props) {
  const [trays, setTrays] = useState<TrayIconInfo[]>([]);
  const [openTrayKey, setOpenTrayKey] = useState("");
  const [pickerOpen, setPickerOpen] = useState(false);
  const pickerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      try {
        const all = await invoke<Record<string, unknown>>("hub_settings_get_all", {
          pluginId,
        });
        let key = String(all?.[OPEN_TRAY_SETTING_KEY] ?? "").trim();
        if (!key) {
          const legacy = await invoke<string | null>("hub_island_get_bound_tray", {
            pluginId,
          }).catch(() => null);
          key = typeof legacy === "string" ? legacy.trim() : "";
          if (key) {
            await invoke("hub_settings_set", {
              pluginId,
              key: OPEN_TRAY_SETTING_KEY,
              value: key,
            });
          }
        }
        if (!cancelled) setOpenTrayKey(key);
      } catch {
        if (!cancelled) setOpenTrayKey("");
      }
    }
    void load();

    let un: (() => void) | undefined;
    void listen<{ pluginId?: string; settings?: Record<string, unknown> }>(
      "plugin-settings-changed",
      (ev) => {
        if (ev.payload?.pluginId !== pluginId) return;
        setOpenTrayKey(String(ev.payload.settings?.[OPEN_TRAY_SETTING_KEY] ?? "").trim());
      },
    ).then((fn) => {
      un = fn;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, [pluginId]);

  useEffect(() => {
    let cancelled = false;
    let un: (() => void) | undefined;
    void invoke<TrayIconInfo[]>("list_tray_icons")
      .then((list) => {
        if (!cancelled) setTrays(list ?? []);
      })
      .catch(() => undefined);
    void listen<TrayIconInfo[]>("tray-icons", (ev) => {
      setTrays(ev.payload ?? []);
    }).then((fn) => {
      un = fn;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  useEffect(() => {
    if (!pickerOpen) return;
    const onPointer = (ev: MouseEvent) => {
      if (pickerRef.current && !pickerRef.current.contains(ev.target as Node)) {
        setPickerOpen(false);
      }
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") setPickerOpen(false);
    };
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [pickerOpen]);

  async function setOpenTray(key: string) {
    const value = String(key || "").trim();
    const next = await invoke<Record<string, unknown>>("hub_settings_set", {
      pluginId,
      key: OPEN_TRAY_SETTING_KEY,
      value,
    });
    setOpenTrayKey(String(next?.[OPEN_TRAY_SETTING_KEY] ?? value).trim());
  }

  const selected = trays.find((t) => trayPinKey(t) === openTrayKey) ?? null;
  const orphan = Boolean(openTrayKey) && !trays.some((t) => trayPinKey(t) === openTrayKey);
  const triggerLabel = selected
    ? trayLabel(selected)
    : orphan
      ? `已绑定（当前不在托盘）`
      : "不绑定";

  return (
    <div className={`pref-row${disabled ? " is-disabled" : ""}`}>
      <span className="pref-row-text">
        <span className="pref-row-label">{label}</span>
        {description ? <span className="pref-row-desc">{description}</span> : null}
      </span>
      <div
        className={`scenario-tray-picker scenario-tray-picker-inline${pickerOpen ? " is-open" : ""}`}
        ref={pickerRef}
      >
        <button
          type="button"
          className="scenario-tray-picker-trigger"
          aria-haspopup="listbox"
          aria-expanded={pickerOpen}
          disabled={disabled}
          onClick={() => setPickerOpen((v) => !v)}
        >
          {selected?.icon_png_base64 ? (
            <img
              className="scenario-tray-picker-icon"
              src={`data:image/png;base64,${selected.icon_png_base64}`}
              alt=""
              draggable={false}
            />
          ) : (
            <span className="scenario-tray-picker-icon scenario-tray-picker-fallback">
              {orphan ? "?" : "—"}
            </span>
          )}
          <span className="scenario-tray-picker-label">{triggerLabel}</span>
          <span className="scenario-tray-picker-chevron" aria-hidden>
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
              <path
                d="M4 6l4 4 4-4"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </span>
        </button>
        {pickerOpen ? (
          <div className="scenario-tray-picker-menu" role="listbox" aria-label="绑定托盘">
            <button
              type="button"
              role="option"
              aria-selected={!openTrayKey}
              className={`scenario-tray-picker-option${!openTrayKey ? " is-selected" : ""}`}
              onClick={() => {
                void setOpenTray("");
                setPickerOpen(false);
              }}
            >
              <span className="scenario-tray-picker-icon scenario-tray-picker-fallback">
                —
              </span>
              <span className="scenario-tray-picker-label">不绑定</span>
            </button>
            {orphan ? (
              <button
                type="button"
                role="option"
                aria-selected
                className="scenario-tray-picker-option is-selected is-orphan"
                onClick={() => {
                  void setOpenTray("");
                  setPickerOpen(false);
                }}
              >
                <span className="scenario-tray-picker-icon scenario-tray-picker-fallback">
                  ?
                </span>
                <span className="scenario-tray-picker-label">
                  已绑定（当前不在托盘）· {openTrayKey}
                </span>
              </button>
            ) : null}
            {trays.map((icon) => {
              const key = trayPinKey(icon);
              if (!key) return null;
              const on = openTrayKey === key;
              return (
                <button
                  key={key}
                  type="button"
                  role="option"
                  aria-selected={on}
                  className={`scenario-tray-picker-option${on ? " is-selected" : ""}`}
                  onClick={() => {
                    void setOpenTray(key);
                    setPickerOpen(false);
                  }}
                >
                  {icon.icon_png_base64 ? (
                    <img
                      className="scenario-tray-picker-icon"
                      src={`data:image/png;base64,${icon.icon_png_base64}`}
                      alt=""
                      draggable={false}
                    />
                  ) : (
                    <span className="scenario-tray-picker-icon scenario-tray-picker-fallback">
                      {trayLabel(icon).charAt(0).toUpperCase()}
                    </span>
                  )}
                  <span className="scenario-tray-picker-label">{trayLabel(icon)}</span>
                </button>
              );
            })}
          </div>
        ) : null}
      </div>
    </div>
  );
}

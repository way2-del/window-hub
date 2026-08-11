import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import GlobalDialog from "./components/GlobalDialog";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";

declare global {
  interface Window {
    __WH_IS_WIFI_AUTH_POPUP__?: boolean;
    __WH_WIFI_AUTH_SSID__?: string;
  }
}

function resolveSsid(): string {
  if (typeof window.__WH_WIFI_AUTH_SSID__ === "string" && window.__WH_WIFI_AUTH_SSID__) {
    return window.__WH_WIFI_AUTH_SSID__;
  }
  try {
    return new URLSearchParams(window.location.search).get("ssid") || "";
  } catch {
    return "";
  }
}

async function closeSelf() {
  try {
    await invoke("close_wifi_auth_popup");
  } catch {
    try {
      await getCurrentWindow().close();
    } catch {
      /* noop */
    }
  }
}

function WifiAuthIcon() {
  return (
    <svg width="40" height="40" viewBox="0 0 24 24" aria-hidden>
      <circle cx="12" cy="18.2" r="1.5" fill="currentColor" />
      <path
        d="M8.1 14.5a5.7 5.7 0 0 1 7.8 0"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
      />
      <path
        d="M5.2 11.4a9.7 9.7 0 0 1 13.6 0"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
      />
      <path
        d="M2.5 8.2a14 14 0 0 1 19 0"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
      />
    </svg>
  );
}

export default function WifiAuthPopupApp() {
  const [ssid, setSsid] = useState(resolveSsid);
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    void (async () => {
      try {
        const prefs = await invoke<GlassPrefs>("get_material_prefs");
        await syncGlassCss({
          kind: "mica-alt",
          dark: prefs.dark ?? null,
          acrylicAlpha: prefs.acrylicAlpha,
        });
      } catch {
        await syncGlassCss({ kind: "mica-alt", dark: true });
      }
      await invoke("apply_window_effect", {}).catch(() => undefined);
    })();

    const retryA = window.setTimeout(() => {
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }, 120);
    const focusT = window.setTimeout(() => inputRef.current?.focus(), 80);

    let cancelled = false;
    const unsubs: Array<() => void> = [];

    void listen<GlassPrefs>("material-prefs", (ev) => {
      void syncGlassCss({
        kind: "mica-alt",
        dark: ev.payload.dark ?? null,
        acrylicAlpha: ev.payload.acrylicAlpha,
      });
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    unsubs.push(
      subscribeSystemDark(() => {
        void (async () => {
          try {
            const prefs = await invoke<GlassPrefs>("get_material_prefs");
            await syncGlassCss({
              kind: "mica-alt",
              dark: prefs.dark ?? null,
              acrylicAlpha: prefs.acrylicAlpha,
            });
          } catch {
            /* noop */
          }
        })();
      }),
    );

    void listen<string>("wifi-auth-ssid", (ev) => {
      if (typeof ev.payload === "string" && ev.payload) {
        setSsid(ev.payload);
        setPassword("");
        setError(null);
        window.setTimeout(() => inputRef.current?.focus(), 40);
      }
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    void listen<string>("wifi-auth-popup-opened", (ev) => {
      if (typeof ev.payload === "string" && ev.payload) {
        setSsid(ev.payload);
        setPassword("");
        setError(null);
      }
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    return () => {
      cancelled = true;
      window.clearTimeout(retryA);
      window.clearTimeout(focusT);
      unsubs.forEach((fn) => fn());
    };
  }, []);

  async function onJoin() {
    if (busy) return;
    const pwd = password.trim();
    if (!ssid) {
      setError("未指定网络");
      return;
    }
    if (pwd.length < 8) {
      setError("密码至少 8 位");
      inputRef.current?.focus();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await invoke("connect_wifi", { ssid, password: pwd });
      await closeSelf();
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg || "加入失败");
      setBusy(false);
      inputRef.current?.focus();
    }
  }

  return (
    <GlobalDialog
      className="wifi-auth-dialog"
      aria-label={`加入网络 ${ssid}`}
      icon={<WifiAuthIcon />}
      title={<span className="wifi-auth-ssid">{ssid || "未知网络"}</span>}
      footer={
        <>
          <button
            type="button"
            className="global-dialog-btn"
            disabled={busy}
            onClick={() => void closeSelf()}
          >
            取消
          </button>
          <button
            type="button"
            className="global-dialog-btn is-primary"
            disabled={busy || !password.trim()}
            onClick={() => void onJoin()}
          >
            {busy ? "加入中…" : "加入"}
          </button>
        </>
      }
    >
      <label className="wifi-auth-field">
        <span className="wifi-auth-label">密码：</span>
        <input
          ref={inputRef}
          className="wifi-auth-input"
          type={showPassword ? "text" : "password"}
          value={password}
          autoComplete="off"
          spellCheck={false}
          disabled={busy}
          onChange={(e) => setPassword(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void onJoin();
            }
          }}
        />
      </label>
      <label className="wifi-auth-show">
        <input
          type="checkbox"
          checked={showPassword}
          disabled={busy}
          onChange={(e) => setShowPassword(e.target.checked)}
        />
        <span>显示密码</span>
      </label>
      {error ? <div className="wifi-auth-error">{error}</div> : null}
    </GlobalDialog>
  );
}

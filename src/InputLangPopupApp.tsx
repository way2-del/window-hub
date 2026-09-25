import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeSystemDark, syncGlassCss, type GlassPrefs } from "./glassPrefs";
import { schedulePopupFit } from "./popupFit";
import type { InputLangState } from "./components/TrayCluster";
import ChromePopupShell, {
  CHROME_POPUP_SHELL_SELECTOR,
} from "./features/chromePopup/ChromePopupShell";

export type InputLayoutItem = {
  id: string;
  profileType: number;
  langId: number;
  clsid: string;
  guidProfile: string;
  hkl: number;
  langAbbr: string;
  langName: string;
  imeName: string;
  displayName: string;
  mark: string;
  active: boolean;
};

const POPUP_W = 240;

async function closeSelf() {
  try {
    await invoke("close_input_lang_popup");
  } catch {
    try {
      await getCurrentWindow().hide();
    } catch {
      /* noop */
    }
  }
}

function fitImePopup() {
  schedulePopupFit({
    width: POPUP_W,
    selector: CHROME_POPUP_SHELL_SELECTOR,
    minHeight: 72,
  });
}

export default function InputLangPopupApp() {
  const [layouts, setLayouts] = useState<InputLayoutItem[]>([]);

  useEffect(() => {
    fitImePopup();
  }, [layouts]);

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
      try {
        const list = await invoke<InputLayoutItem[]>("list_input_layouts");
        setLayouts(list);
        fitImePopup();
      } catch (e) {
        console.error(e);
        fitImePopup();
      }
    })();

    const retryA = window.setTimeout(() => {
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }, 120);
    const retryB = window.setTimeout(() => {
      void invoke("apply_window_effect", {}).catch(() => undefined);
    }, 350);

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

    void listen<InputLangState>("input-lang", () => {
      void invoke<InputLayoutItem[]>("list_input_layouts")
        .then((list) => {
          setLayouts(list);
          fitImePopup();
        })
        .catch(() => undefined);
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });

    const onBlur = () => {
      window.setTimeout(() => {
        void getCurrentWindow()
          .isFocused()
          .then((f) => {
            if (!f) void closeSelf();
          })
          .catch(() => undefined);
      }, 80);
    };
    window.addEventListener("blur", onBlur);

    return () => {
      cancelled = true;
      window.clearTimeout(retryA);
      window.clearTimeout(retryB);
      unsubs.forEach((fn) => fn());
      window.removeEventListener("blur", onBlur);
    };
  }, []);

  async function pickLayout(item: InputLayoutItem) {
    try {
      await invoke("select_input_layout", {
        profileType: item.profileType,
        langId: item.langId,
        clsid: item.clsid || null,
        guidProfile: item.guidProfile || null,
        hkl: item.hkl,
      });
    } catch (e) {
      console.error(e);
    } finally {
      await closeSelf();
    }
  }

  return (
    <ChromePopupShell density="compact" role="menu" aria-label="输入法">
      <div className="ilang-section">
        {layouts.length === 0 ? (
          <div className="ilang-empty">未检测到输入法</div>
        ) : (
          layouts.map((item) => (
            <button
              key={item.id || `${item.profileType}:${item.langId}:${item.hkl}`}
              type="button"
              className={`ilang-row${item.active ? " is-active" : ""}`}
              onClick={() => void pickLayout(item)}
            >
              <span className="ilang-check" aria-hidden>
                {item.active ? "✓" : ""}
              </span>
              <span className="ilang-mark" aria-hidden>
                {item.mark || item.langAbbr || "?"}
              </span>
              <span className="ilang-label">{item.displayName || item.langName}</span>
            </button>
          ))
        )}
      </div>

      <div className="ilang-sep" />

      <div className="ilang-section">
        <button
          type="button"
          className="ilang-row"
          onClick={() => {
            void (async () => {
              try {
                await invoke("open_keyboard_settings");
              } catch (e) {
                console.error(e);
              } finally {
                await closeSelf();
              }
            })();
          }}
        >
          <span className="ilang-check" />
          <span className="ilang-label">键盘偏好设置</span>
        </button>
      </div>
    </ChromePopupShell>
  );
}

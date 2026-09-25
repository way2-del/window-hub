import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useRef } from "react";
import { hideChromeHoverTip, hostTipPointerProps } from "../../chromeHoverTip";
import { clickTrace } from "../../clickTrace";

const CC_W = 374;
const POPUP_GAP = 8;

/** Same right-align + gap math as Wi‑Fi / tray chrome popups. */
async function popupOrigin(el: HTMLElement, width: number) {
  const win = getCurrentWindow();
  const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  const rect = el.getBoundingClientRect();
  const x = outer.x / factor + rect.right - width;
  const y = outer.y / factor + rect.bottom + POPUP_GAP;
  return { x, y };
}

/** Shared by the standalone host status rail and the tray's host controls. */
export default function ControlCenterButton() {
  const pressedOpen = useRef<Promise<boolean> | null>(null);
  async function toggle(button: HTMLButtonElement) {
    const beforePress = pressedOpen.current;
    pressedOpen.current = null;
    void hideChromeHoverTip();
    clickTrace("fe-control", "toggle requested");
    try {
      if (beforePress && (await beforePress)) await invoke("close_control_center");
      else {
        const { x, y } = await popupOrigin(button, CC_W);
        await invoke("toggle_control_center", { x, y });
      }
      clickTrace("fe-control", "toggle completed");
    } catch (error) {
      clickTrace("fe-chrome", `open_control_center error ${String(error)}`);
      console.error("打开控制中心失败", error);
    }
  }
  return (
    <button
      type="button"
      className="chrome-control-center-btn"
      aria-label="打开控制中心"
      aria-haspopup="dialog"
      {...hostTipPointerProps("控制中心")}
      onPointerDown={(event) => {
        if (event.button === 0) {
          pressedOpen.current = invoke<boolean>("is_control_center_open").catch(() => false);
        }
      }}
      onPointerCancel={() => {
        pressedOpen.current = null;
      }}
      onMouseDown={(event) => event.preventDefault()}
      onClick={(event) => void toggle(event.currentTarget)}
    >
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden="true">
        <rect x="3" y="4" width="18" height="6" rx="3" />
        <rect x="3" y="14" width="18" height="6" rx="3" />
        <circle cx="8" cy="7" r="1.4" fill="currentColor" stroke="none" />
        <circle cx="16" cy="17" r="1.4" fill="currentColor" stroke="none" />
      </svg>
    </button>
  );
}

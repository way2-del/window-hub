import { invoke } from "@tauri-apps/api/core";
import { useRef } from "react";
import { hideChromeHoverTip, hostTipPointerProps } from "../../chromeHoverTip";
import { clickTrace } from "../../clickTrace";

/** Shared by the standalone host status rail and the tray's host controls. */
export default function ControlCenterButton() {
  const pressedOpen = useRef<Promise<boolean> | null>(null);
  async function toggle(button: HTMLButtonElement) {
    const rect = button.getBoundingClientRect();
    const beforePress = pressedOpen.current;
    pressedOpen.current = null;
    void hideChromeHoverTip();
    clickTrace("fe-control", "toggle requested");
    try {
      if (beforePress && await beforePress) await invoke("close_control_center");
      else await invoke("toggle_control_center", { right: rect.right, bottom: rect.bottom });
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
        if (event.button === 0) pressedOpen.current = invoke<boolean>("is_control_center_open").catch(() => false);
      }}
      onPointerCancel={() => { pressedOpen.current = null; }}
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

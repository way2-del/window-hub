import { invoke } from "@tauri-apps/api/core";
import { hideChromeHoverTip, hostTipPointerProps } from "../../chromeHoverTip";
import { clickTrace } from "../../clickTrace";

/** Shared by the standalone host status rail and the tray's host controls. */
export default function ControlCenterButton() {
  return (
    <button
      type="button"
      className="chrome-control-center-btn"
      aria-label="打开控制中心"
      {...hostTipPointerProps("控制中心（Win+A）")}
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => {
        void hideChromeHoverTip();
        void invoke("open_control_center").catch((error) => {
          clickTrace("fe-chrome", `open_control_center error ${String(error)}`);
          console.error("打开控制中心失败", error);
        });
      }}
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

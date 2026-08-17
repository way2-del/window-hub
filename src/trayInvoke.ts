import { invoke } from "@tauri-apps/api/core";

export type TrayInvokeIcon = {
  id: string;
  hwnd: number;
  callback_msg: number;
  uid: number;
  version?: number;
};

export type TrayClickAction = "left" | "left-double" | "right";

/**
 * Wait for a possible second click before firing single-click.
 * Double-click cancels the pending single and sends only WM_LBUTTONDBLCLK —
 * so single-click apps stay single-fire, double-click-only apps get a clean dblclk.
 */
const TRAY_SINGLE_DELAY_MS = 280;

let pendingLeftTimer: ReturnType<typeof setTimeout> | null = null;
let pendingLeftIcon: TrayInvokeIcon | null = null;

function clearPendingLeft() {
  if (pendingLeftTimer != null) {
    clearTimeout(pendingLeftTimer);
    pendingLeftTimer = null;
  }
  pendingLeftIcon = null;
}

export async function invokeTrayIcon(
  icon: TrayInvokeIcon,
  action: TrayClickAction,
): Promise<void> {
  await invoke("invoke_tray_icon", {
    id: icon.id,
    hwnd: icon.hwnd,
    callbackMsg: icon.callback_msg,
    uid: icon.uid,
    version: icon.version ?? 0,
    action,
  });
}

/** First half of a left interaction — arm delayed single-click. */
export function armTrayLeftClick(icon: TrayInvokeIcon): void {
  clearPendingLeft();
  pendingLeftIcon = icon;
  pendingLeftTimer = setTimeout(() => {
    const target = pendingLeftIcon;
    pendingLeftTimer = null;
    pendingLeftIcon = null;
    if (!target) return;
    void invokeTrayIcon(target, "left").catch((e) => console.error(e));
  }, TRAY_SINGLE_DELAY_MS);
}

/** Native dblclick — cancel pending single, send left-double only. */
export function fireTrayLeftDouble(icon: TrayInvokeIcon): void {
  clearPendingLeft();
  void invokeTrayIcon(icon, "left-double").catch((e) => console.error(e));
}

/** @deprecated Prefer armTrayLeftClick + fireTrayLeftDouble from pointer handlers. */
export async function invokeTrayLeftClick(icon: TrayInvokeIcon): Promise<void> {
  armTrayLeftClick(icon);
}

export async function invokeTrayRightClick(icon: TrayInvokeIcon): Promise<void> {
  clearPendingLeft();
  await invokeTrayIcon(icon, "right");
}

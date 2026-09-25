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
const TRAY_SINGLE_DELAY_MS = 220;

let pendingLeftTimer: ReturnType<typeof setTimeout> | null = null;
let pendingLeftIcon: TrayInvokeIcon | null = null;
/** Physical screen point captured at pointer-down (VERSION_4 packing). */
let pendingLeftCursor: { x: number; y: number } | null = null;

function clearPendingLeft() {
  if (pendingLeftTimer != null) {
    clearTimeout(pendingLeftTimer);
    pendingLeftTimer = null;
  }
  pendingLeftIcon = null;
  pendingLeftCursor = null;
}

async function captureCursor(): Promise<{ x: number; y: number } | null> {
  try {
    const [x, y] = await invoke<[number, number]>("tray_cursor_pos");
    return { x, y };
  } catch {
    return null;
  }
}

export async function invokeTrayIcon(
  icon: TrayInvokeIcon,
  action: TrayClickAction,
  cursor?: { x: number; y: number } | null,
): Promise<void> {
  await invoke("invoke_tray_icon", {
    id: icon.id,
    hwnd: icon.hwnd,
    callbackMsg: icon.callback_msg,
    uid: icon.uid,
    version: icon.version ?? 0,
    action,
    cursorX: cursor?.x ?? null,
    cursorY: cursor?.y ?? null,
  });
}

/** First half of a left interaction — arm delayed single-click. */
export function armTrayLeftClick(icon: TrayInvokeIcon): void {
  clearPendingLeft();
  pendingLeftIcon = icon;
  // Capture press-time cursor immediately (don't wait for the 220ms timer).
  void captureCursor().then((pos) => {
    if (pendingLeftIcon?.id === icon.id) {
      pendingLeftCursor = pos;
    }
  });
  pendingLeftTimer = setTimeout(() => {
    const target = pendingLeftIcon;
    const cursor = pendingLeftCursor;
    pendingLeftTimer = null;
    pendingLeftIcon = null;
    pendingLeftCursor = null;
    if (!target) return;
    void invokeTrayIcon(target, "left", cursor).catch((e) => console.error(e));
  }, TRAY_SINGLE_DELAY_MS);
}

/** Native dblclick — cancel pending single, send left-double only. */
export function fireTrayLeftDouble(icon: TrayInvokeIcon): void {
  const cursor = pendingLeftCursor;
  clearPendingLeft();
  void invokeTrayIcon(icon, "left-double", cursor).catch((e) => console.error(e));
}

/** @deprecated Prefer armTrayLeftClick + fireTrayLeftDouble from pointer handlers. */
export async function invokeTrayLeftClick(icon: TrayInvokeIcon): Promise<void> {
  armTrayLeftClick(icon);
}

export async function invokeTrayRightClick(icon: TrayInvokeIcon): Promise<void> {
  clearPendingLeft();
  const cursor = await captureCursor();
  await invokeTrayIcon(icon, "right", cursor);
}

/** 全岛共享的前台应用轮询，避免 StatusMenu / Shortcuts 各自狂刷 IPC */

import { invoke } from "@tauri-apps/api/core";

export type ForegroundAppInfo = {
  isSelf?: boolean;
  windowId?: string | null;
  label?: string;
  title?: string;
  iconPng?: string | null;
};

const EVENT = "wh-foreground";
const INTERVAL_MS = 1800;
const INTERVAL_HIDDEN_MS = 4000;

let timer: number | null = null;
let refs = 0;
let lastKey = "";

async function tick() {
  try {
    const app = await invoke<ForegroundAppInfo>("get_foreground_app");
    // 忽略 title 闪烁；允许同窗图标从空→有时再推一次
    const key = [
      app.windowId ?? "",
      app.isSelf ? "1" : "0",
      app.label ?? "",
      app.iconPng?.trim() ? "1" : "0",
    ].join("|");
    if (key === lastKey) return;
    lastKey = key;
    window.dispatchEvent(new CustomEvent(EVENT, { detail: app }));
  } catch {
    /* noop */
  }
}

function delayMs() {
  return document.hidden ? INTERVAL_HIDDEN_MS : INTERVAL_MS;
}

function start() {
  if (timer != null) return;
  void tick();
  const arm = () => {
    timer = window.setTimeout(() => {
      void tick().finally(() => {
        if (refs > 0) arm();
        else timer = null;
      });
    }, delayMs());
  };
  arm();
}

function stop() {
  if (timer == null) return;
  window.clearTimeout(timer);
  timer = null;
  lastKey = "";
}

/** 订阅前台变化；无人订阅时自动停表。 */
export function subscribeForeground(
  onChange: (app: ForegroundAppInfo) => void,
): () => void {
  refs += 1;
  start();
  const handler = (e: Event) => {
    onChange((e as CustomEvent<ForegroundAppInfo>).detail);
  };
  window.addEventListener(EVENT, handler);
  return () => {
    window.removeEventListener(EVENT, handler);
    refs = Math.max(0, refs - 1);
    if (refs === 0) stop();
  };
}

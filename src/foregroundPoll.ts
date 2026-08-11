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
const INTERVAL_MS = 1200;

let timer: number | null = null;
let refs = 0;
let lastJson = "";

async function tick() {
  try {
    const app = await invoke<ForegroundAppInfo>("get_foreground_app");
    const json = JSON.stringify(app);
    if (json === lastJson) return;
    lastJson = json;
    window.dispatchEvent(new CustomEvent(EVENT, { detail: app }));
  } catch {
    /* noop */
  }
}

function start() {
  if (timer != null) return;
  void tick();
  timer = window.setInterval(() => void tick(), INTERVAL_MS);
}

function stop() {
  if (timer == null) return;
  window.clearInterval(timer);
  timer = null;
  lastJson = "";
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

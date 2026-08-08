import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

export type TrayIconInfo = {
  id: string;
  tooltip: string;
  process: string;
  uid: number;
  hwnd: number;
  callback_msg: number;
  version?: number;
  icon_png_base64: string;
  area: string;
  flashing?: boolean;
};

export type TrayPrefs = {
  pinned: string[];
  menu_heights?: Record<string, number>;
};

const TRAY_POPUP_W = 280;
/** 避开顶栏下方 4px 吸色带，防止弹窗像素污染任务栏色带 */
const TRAY_POPUP_GAP = 8;

function pad2(n: number) {
  return n.toString().padStart(2, "0");
}

/** 参考菜单栏：8月4日 周二 14:43 */
function formatMenuClock(d: Date) {
  const week = ["日", "一", "二", "三", "四", "五", "六"][d.getDay()];
  return `${d.getMonth() + 1}月${d.getDate()}日 周${week} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}

export function trayLabel(icon: TrayIconInfo) {
  return icon.tooltip || icon.process || "未知应用";
}

function TrayGlyph({ icon }: { icon: TrayIconInfo }) {
  if (icon.icon_png_base64) {
    return (
      <img
        className="tray-glyph"
        src={`data:image/png;base64,${icon.icon_png_base64}`}
        alt=""
        draggable={false}
      />
    );
  }
  const letter = trayLabel(icon).charAt(0).toUpperCase();
  return <span className="tray-glyph tray-glyph-fallback">{letter}</span>;
}

async function clickTray(icon: TrayIconInfo, action: "left" | "right") {
  try {
    await invoke("invoke_tray_icon", {
      id: icon.id,
      hwnd: icon.hwnd,
      callbackMsg: icon.callback_msg,
      uid: icon.uid,
      version: icon.version ?? 0,
      action,
    });
  } catch (e) {
    console.error(e);
  }
}

async function popupAnchor(chevron: HTMLElement) {
  const win = getCurrentWindow();
  const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  const rect = chevron.getBoundingClientRect();
  const logicalX = outer.x / factor;
  const logicalY = outer.y / factor;
  const x = logicalX + rect.right - TRAY_POPUP_W;
  const y = logicalY + rect.bottom + TRAY_POPUP_GAP;
  return { x, y };
}

export default function TrayCluster({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [icons, setIcons] = useState<TrayIconInfo[]>([]);
  const [pinned, setPinned] = useState<string[]>([]);
  const [now, setNow] = useState(() => new Date());
  const rootRef = useRef<HTMLDivElement>(null);
  const chevronRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const t = window.setInterval(() => setNow(new Date()), 1000);
    return () => window.clearInterval(t);
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    void (async () => {
      try {
        const [list, prefs] = await Promise.all([
          invoke<TrayIconInfo[]>("list_tray_icons"),
          invoke<TrayPrefs>("get_tray_prefs"),
        ]);
        if (!cancelled) {
          setIcons(list);
          setPinned(prefs.pinned ?? []);
        }
      } catch {
        /* noop */
      }

      try {
        unsubs.push(
          await listen<TrayIconInfo[]>("tray-icons", (ev) => {
            setIcons(ev.payload);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen<TrayPrefs>("tray-prefs", (ev) => {
            setPinned(ev.payload.pinned ?? []);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("tray-popup-opened", () => {
            onOpenChange(true);
          }),
        );
      } catch {
        /* noop */
      }
      try {
        unsubs.push(
          await listen("tray-popup-closed", () => {
            onOpenChange(false);
          }),
        );
      } catch {
        /* noop */
      }
    })();

    return () => {
      cancelled = true;
      unsubs.forEach((fn) => fn());
    };
  }, [onOpenChange]);

  // 父级要求关闭时（展开灵动岛等）同步关掉独立弹窗
  useEffect(() => {
    if (open) return;
    void invoke("is_tray_popup_open")
      .then((visible) => {
        if (visible) return invoke("close_tray_popup");
      })
      .catch(() => undefined);
  }, [open]);

  const pinnedSet = useMemo(() => new Set(pinned), [pinned]);
  const pinnedIcons = useMemo(
    () => icons.filter((i) => pinnedSet.has(i.id)),
    [icons, pinnedSet],
  );
  // Flashing / attention icons surface on the rail even when unpinned
  // (Explorer-like: WeChat etc. must be visible while blinking).
  const railIcons = useMemo(() => {
    const seen = new Set(pinnedIcons.map((i) => i.id));
    const extra = icons.filter((i) => i.flashing && !seen.has(i.id));
    return [...pinnedIcons, ...extra];
  }, [icons, pinnedIcons]);

  async function togglePopup() {
    try {
      const visible = await invoke<boolean>("is_tray_popup_open");
      if (visible || open) {
        await invoke("close_tray_popup");
        onOpenChange(false);
        return;
      }
      const el = chevronRef.current;
      if (!el) return;
      const { x, y } = await popupAnchor(el);
      await invoke("open_tray_popup", { x, y });
      onOpenChange(true);
    } catch (e) {
      console.error(e);
    }
  }

  return (
    <div className="tray-cluster" ref={rootRef} onClick={(e) => e.stopPropagation()}>
      <div className="tray-rail">
        {railIcons.map((icon) => (
          <button
            key={icon.id}
            type="button"
            className={`tray-icon-btn${icon.flashing ? " is-flashing" : ""}`}
            title={trayLabel(icon)}
            onClick={() => void clickTray(icon, "left")}
            onContextMenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              void clickTray(icon, "right");
            }}
          >
            <TrayGlyph icon={icon} />
          </button>
        ))}

        <button
          type="button"
          className="tray-clock"
          title="打开通知中心"
          onClick={() => {
            void invoke("open_notification_center").catch((e) => console.error(e));
          }}
        >
          <time dateTime={now.toISOString()}>{formatMenuClock(now)}</time>
        </button>

        <button
          ref={chevronRef}
          type="button"
          className={`tray-chevron${open ? " is-open" : ""}`}
          aria-label={open ? "收起托盘" : "展开托盘"}
          aria-expanded={open}
          onMouseDown={(e) => {
            // 避免抢焦点导致弹窗先 blur 再被误开
            e.preventDefault();
          }}
          onClick={() => void togglePopup()}
        >
          <svg width="9" height="9" viewBox="0 0 12 12" aria-hidden>
            <path
              d="M2.2 7.8 L6 4 L9.8 7.8"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.7"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </div>
    </div>
  );
}

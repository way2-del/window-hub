import { useEffect, useRef, useState, type RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { hostTipPointerProps } from "../chromeHoverTip";
import { elementScreenRect } from "../genieAnchor";
import "./StatusMenu.css";

type ForegroundApp = {
  title: string;
  exeName?: string | null;
  label: string;
  isSelf?: boolean;
};

type Props = {
  anchorRef: RefObject<HTMLElement | null>;
  menuOpen: boolean;
  onMenuOpenChange: (open: boolean) => void;
};

const STATUS_MENU_W = 200;
const STATUS_MENU_GAP = 8;

function truncateLabel(s: string, maxChars = 10): string {
  const t = s.trim();
  if (!t) return "桌面";
  const chars = [...t];
  if (chars.length <= maxChars) return t;
  return `${chars.slice(0, maxChars - 1).join("")}…`;
}

async function popupAnchor(el: HTMLElement) {
  const win = getCurrentWindow();
  const [factor, outer] = await Promise.all([win.scaleFactor(), win.outerPosition()]);
  const rect = el.getBoundingClientRect();
  const logicalX = outer.x / factor;
  const logicalY = outer.y / factor;
  const x = logicalX + rect.left;
  const y = logicalY + rect.bottom + STATUS_MENU_GAP;
  return { x, y, w: STATUS_MENU_W };
}

export default function StatusMenu({
  anchorRef,
  menuOpen,
  onMenuOpenChange,
}: Props) {
  const [label, setLabel] = useState("桌面");
  const lastLabel = useRef("桌面");
  const btnRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const app = await invoke<ForegroundApp>("get_foreground_app");
        if (cancelled) return;
        if (app.isSelf) return;
        const next = truncateLabel(app.label || app.title || "桌面");
        if (next === lastLabel.current) return;
        lastLabel.current = next;
        setLabel(next);
      } catch {
        /* noop */
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 1500);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
    void listen("status-menu-popup-opened", () => {
      onMenuOpenChange(true);
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });
    void listen("status-menu-popup-closed", () => {
      onMenuOpenChange(false);
    }).then((fn) => {
      if (!cancelled) unsubs.push(fn);
      else fn();
    });
    return () => {
      cancelled = true;
      unsubs.forEach((fn) => fn());
    };
  }, [onMenuOpenChange]);

  useEffect(() => {
    if (menuOpen) return;
    void invoke("is_status_menu_popup_open")
      .then((visible) => {
        if (visible) return invoke("close_status_menu_popup");
      })
      .catch(() => undefined);
  }, [menuOpen]);

  async function toggleMenu() {
    try {
      const visible = await invoke<boolean>("is_status_menu_popup_open");
      const el = btnRef.current ?? anchorRef.current;
      if (visible || menuOpen) {
        if (el) {
          const anchor = await elementScreenRect(el);
          await invoke("genie_hide_popup", {
            slotId: "status-menu",
            windowLabel: "status-menu-popup",
            anchor,
          }).catch(() => invoke("close_status_menu_popup"));
        } else {
          await invoke("close_status_menu_popup");
        }
        onMenuOpenChange(false);
        return;
      }
      if (!el) return;
      const { x, y } = await popupAnchor(el);
      const anchor = await elementScreenRect(el);
      const shown = await invoke<boolean>("genie_show_popup", {
        slotId: "status-menu",
        windowLabel: "status-menu-popup",
        anchor,
        x,
        y,
      }).catch(() => false);
      if (!shown) {
        await invoke("open_status_menu_popup", { x, y });
      }
      onMenuOpenChange(true);
    } catch (e) {
      console.error(e);
    }
  }

  return (
    <button
      ref={btnRef}
      type="button"
      className={`settings-btn${menuOpen ? " is-active" : ""}`}
      aria-label="状态菜单"
      aria-expanded={menuOpen}
      onClick={() => void toggleMenu()}
    >
      <span className="settings-label" {...hostTipPointerProps(lastLabel.current)}>
        {label}
      </span>
    </button>
  );
}

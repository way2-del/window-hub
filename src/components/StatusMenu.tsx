import { useEffect, useRef, useState, type RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { clickTrace } from "../clickTrace";
import { hideChromeHoverTip, hostTipPointerProps } from "../chromeHoverTip";
import { anchorPopupBelowElement } from "../popupAnchor";
import { clearShortcutsFoldMenuItems } from "../features/chrome/shortcutsFoldMenuBus";
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
const STATUS_MENU_H_EST = 280;
const STATUS_MENU_GAP = 8;

function truncateLabel(s: string, maxChars = 10): string {
  const t = s.trim();
  if (!t) return "桌面";
  const chars = [...t];
  if (chars.length <= maxChars) return t;
  return `${chars.slice(0, maxChars - 1).join("")}…`;
}

async function popupAnchor(el: HTMLElement) {
  const { x, y } = await anchorPopupBelowElement(
    el,
    STATUS_MENU_W,
    STATUS_MENU_H_EST,
    STATUS_MENU_GAP,
  );
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
    clickTrace("fe-status", "toggleMenu click");
    try {
      clickTrace("fe-status", "before is_status_menu_popup_open");
      const visible = await invoke<boolean>("is_status_menu_popup_open");
      clickTrace("fe-status", `is_open=${visible} menuOpen=${menuOpen}`);
      if (visible || menuOpen) {
        clickTrace("fe-status", "before close_status_menu_popup");
        await invoke("close_status_menu_popup");
        clickTrace("fe-status", "after close");
        onMenuOpenChange(false);
        return;
      }
      const el = btnRef.current ?? anchorRef.current;
      if (!el) {
        clickTrace("fe-status", "no anchor el");
        return;
      }
      clickTrace("fe-status", "before popupAnchor");
      const { x, y } = await popupAnchor(el);
      clickTrace("fe-status", `anchor x=${x.toFixed(0)} y=${y.toFixed(0)}`);
      clearShortcutsFoldMenuItems();
      clickTrace("fe-status", "before open_status_menu_popup");
      await invoke("open_status_menu_popup", { x, y });
      clickTrace("fe-status", "after open_status_menu_popup");
      onMenuOpenChange(true);
    } catch (e) {
      clickTrace("fe-status", `error ${String(e)}`);
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
      onPointerDown={() => {
        clickTrace("fe-status", "pointerdown");
        // Cancel in-flight tip create/show before menu IPC (hang race #10+#13).
        void hideChromeHoverTip();
      }}
      onClick={() => void toggleMenu()}
    >
      <span className="settings-label" {...hostTipPointerProps(lastLabel.current)}>
        {label}
      </span>
    </button>
  );
}

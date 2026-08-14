import { useEffect, useRef, useState, type RefObject } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { subscribeForeground } from "../foregroundPoll";
import "./StatusMenu.css";

type Props = {
  anchorRef: RefObject<HTMLElement | null>;
  menuOpen: boolean;
  onMenuOpenChange: (open: boolean) => void;
};

const STATUS_MENU_W = 220;
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
  const [iconPng, setIconPng] = useState<string | null>(null);
  const lastLabel = useRef("桌面");
  const btnRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    return subscribeForeground((app) => {
      if (app.isSelf) return;
      const next = truncateLabel(app.label || app.title || "桌面");
      lastLabel.current = next;
      setLabel(next);
      setIconPng(app.iconPng?.trim() ? app.iconPng : null);
    });
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
      if (visible || menuOpen) {
        await invoke("close_status_menu_popup");
        onMenuOpenChange(false);
        return;
      }
      const el = btnRef.current ?? anchorRef.current;
      if (!el) return;
      const { x, y } = await popupAnchor(el);
      await invoke("open_status_menu_popup", { x, y });
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
      {iconPng ? (
        <img
          className="settings-app-icon"
          src={`data:image/png;base64,${iconPng}`}
          alt=""
          draggable={false}
        />
      ) : null}
      <span className="settings-label" title={lastLabel.current}>
        {label}
      </span>
    </button>
  );
}

import { useState, type KeyboardEvent, type MouseEvent } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { IslandHomeTab } from "./types";

export function IconHome({ active }: { active?: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden>
      <path
        fill={active ? "currentColor" : "none"}
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinejoin="round"
        d="M4 10.5 12 4l8 6.5V20a1 1 0 0 1-1 1h-5v-6H10v6H5a1 1 0 0 1-1-1v-9.5z"
      />
    </svg>
  );
}

export function IconTray({ active }: { active?: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden>
      <path
        fill={active ? "currentColor" : "none"}
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinejoin="round"
        d="M4 8h16v3H4V8zm0 5h16l-1.5 6.5a1 1 0 0 1-1 .5H6.5a1 1 0 0 1-1-.5L4 13z"
      />
      <path
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        d="M8 5h8"
      />
    </svg>
  );
}

export function IconSearch({ active }: { active?: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden>
      <circle
        cx="10.5"
        cy="10.5"
        r="6.25"
        fill={active ? "currentColor" : "none"}
        fillOpacity={active ? 0.18 : undefined}
        stroke="currentColor"
        strokeWidth="1.8"
      />
      <path
        d="M15.2 15.2L20 20"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
      />
    </svg>
  );
}

type NavProps = {
  tab: IslandHomeTab;
  onTabChange: (tab: IslandHomeTab) => void;
};

/** 岛栏左侧：首页 / 中转站 / 文件搜索 */
export function IslandHomeNav({ tab, onTabChange }: NavProps) {
  return (
    <div className="ih-bar-nav" role="tablist" aria-label="下拉导航">
      <button
        type="button"
        role="tab"
        aria-selected={tab === "home"}
        className={`ih-bar-nav-btn${tab === "home" ? " is-active" : ""}`}
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          onTabChange("home");
        }}
        title="首页"
      >
        <IconHome active={tab === "home"} />
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={tab === "transfer"}
        className={`ih-bar-nav-btn${tab === "transfer" ? " is-active" : ""}`}
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          onTabChange("transfer");
        }}
        title="中转站"
      >
        <IconTray active={tab === "transfer"} />
      </button>
      <button
        type="button"
        role="tab"
        aria-selected={tab === "search"}
        className={`ih-bar-nav-btn${tab === "search" ? " is-active" : ""}`}
        onPointerDown={(e) => e.stopPropagation()}
        onClick={(e) => {
          e.stopPropagation();
          onTabChange("search");
        }}
        title="文件搜索"
      >
        <IconSearch active={tab === "search"} />
      </button>
    </div>
  );
}

/** 岛栏右侧：截取灵动岛（含展开仪表盘） */
export function IslandHomeStatus() {
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);

  async function capture(e: MouseEvent | KeyboardEvent) {
    e.stopPropagation();
    if (busy) return;
    setBusy(true);
    setDone(false);
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
    });
    try {
      await invoke("capture_island_screenshot");
      setDone(true);
      window.setTimeout(() => setDone(false), 1600);
    } catch (err) {
      console.warn("[island] screenshot", err);
    } finally {
      setBusy(false);
    }
  }

  return (
    <button
      type="button"
      className={`ih-bar-shot${busy ? " is-busy" : ""}${done ? " is-done" : ""}`}
      aria-label="截图灵动岛"
      title={done ? "已复制到剪贴板" : "截图灵动岛（含仪表盘）"}
      disabled={busy}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => void capture(e)}
    >
      <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden>
        <path
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinejoin="round"
          d="M4 8.5h3.2l1.3-2h7l1.3 2H20a1 1 0 0 1 1 1V18a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V9.5a1 1 0 0 1 1-1z"
        />
        <circle
          cx="12"
          cy="13.2"
          r="3.2"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
        />
      </svg>
    </button>
  );
}

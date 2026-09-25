import type { HTMLAttributes, ReactNode } from "react";
import "./chromePopupShell.css";

/** Shared class for chrome flyout shells (Wi‑Fi / tray / control center / IME). */
export const CHROME_POPUP_SHELL_CLASS = "wh-chrome-popup-shell";

/** Use with `schedulePopupFit` / `fitPopupToContent`. */
export const CHROME_POPUP_SHELL_SELECTOR = `.${CHROME_POPUP_SHELL_CLASS}`;

export type ChromePopupShellProps = {
  children?: ReactNode;
  /** Extra classes (e.g. tray reveal states). */
  className?: string;
  role?: HTMLAttributes<HTMLDivElement>["role"];
  "aria-label"?: string;
  "aria-hidden"?: boolean | "true" | "false";
  /**
   * Compact padding for narrow menus (IME). Default matches Wi‑Fi / tray.
   */
  density?: "default" | "compact";
};

/**
 * Host chrome popup shell — DWM owns outer round corners; CSS fills the HWND.
 * Use for Wi‑Fi, tray overflow, control center, input-language, and similar flyouts.
 * Do not add a second `border-radius` on this node.
 */
export default function ChromePopupShell({
  children,
  className,
  role = "dialog",
  "aria-label": ariaLabel,
  "aria-hidden": ariaHidden,
  density = "default",
}: ChromePopupShellProps) {
  const classes = [
    CHROME_POPUP_SHELL_CLASS,
    density === "compact" ? "is-compact" : null,
    className,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      className={classes}
      role={role}
      aria-label={ariaLabel}
      aria-hidden={ariaHidden}
    >
      {children}
    </div>
  );
}

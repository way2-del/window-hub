import type { ReactNode } from "react";

export type GlobalDialogProps = {
  /** Optional leading icon (e.g. Wi‑Fi glyph). */
  icon?: ReactNode;
  /** Primary title next to the icon. */
  title: ReactNode;
  children?: ReactNode;
  /** Footer actions (typically Cancel / Confirm). */
  footer?: ReactNode;
  className?: string;
  role?: string;
  "aria-label"?: string;
};

/**
 * Encapsulated global modal chrome — dark glass panel used by centered
 * host dialogs (Wi‑Fi password, etc.). Keep content/footer in slots so
 * callers don't re-implement layout.
 */
export default function GlobalDialog({
  icon,
  title,
  children,
  footer,
  className,
  role = "dialog",
  "aria-label": ariaLabel,
}: GlobalDialogProps) {
  return (
    <div
      className={`global-dialog${className ? ` ${className}` : ""}`}
      role={role}
      aria-label={ariaLabel}
      aria-modal="true"
    >
      <div className="global-dialog-head">
        {icon ? <div className="global-dialog-icon">{icon}</div> : null}
        <div className="global-dialog-title">{title}</div>
      </div>
      {children ? <div className="global-dialog-body">{children}</div> : null}
      {footer ? <div className="global-dialog-footer">{footer}</div> : null}
    </div>
  );
}

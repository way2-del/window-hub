import { useEffect, useRef, useState } from "react";

export type PrefSelectOption = {
  value: string;
  label: string;
};

type Props = {
  value: string;
  options: PrefSelectOption[];
  disabled?: boolean;
  /** Accessible name for the listbox. */
  ariaLabel?: string;
  className?: string;
  onChange: (value: string) => void;
};

/**
 * Custom dropdown matching plugin-market / OpenTraySetting (`scenario-tray-picker`).
 * Prefer this over native `<select class="pref-select">` in settings rows.
 */
export default function PrefSelect({
  value,
  options,
  disabled = false,
  ariaLabel,
  className = "",
  onChange,
}: Props) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const selected = options.find((o) => o.value === value) ?? options[0] ?? null;

  useEffect(() => {
    if (!open) return;
    const onPointer = (ev: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(ev.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (ev: KeyboardEvent) => {
      if (ev.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div
      className={`scenario-tray-picker scenario-tray-picker-inline scenario-tray-picker-text${open ? " is-open" : ""}${className ? ` ${className}` : ""}`}
      ref={rootRef}
    >
      <button
        type="button"
        className="scenario-tray-picker-trigger"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={ariaLabel}
        disabled={disabled}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="scenario-tray-picker-label">{selected?.label ?? ""}</span>
        <span className="scenario-tray-picker-chevron" aria-hidden>
          <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
            <path
              d="M4 6l4 4 4-4"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </span>
      </button>
      {open ? (
        <div
          className="scenario-tray-picker-menu"
          role="listbox"
          aria-label={ariaLabel}
        >
          {options.map((opt) => {
            const on = opt.value === value;
            return (
              <button
                key={opt.value}
                type="button"
                role="option"
                aria-selected={on}
                className={`scenario-tray-picker-option${on ? " is-selected" : ""}`}
                onClick={() => {
                  onChange(opt.value);
                  setOpen(false);
                }}
              >
                <span className="scenario-tray-picker-label">{opt.label}</span>
              </button>
            );
          })}
        </div>
      ) : null}
    </div>
  );
}

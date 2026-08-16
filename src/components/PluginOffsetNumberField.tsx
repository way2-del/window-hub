import { useEffect, useRef, useState } from "react";
import type { PluginSettingField } from "../plugins/types";

type Props = {
  field: PluginSettingField;
  value: unknown;
  disabled?: boolean;
  onCommit: (n: number) => void;
};

function clamp(n: number, min: number, max: number) {
  return Math.max(min, Math.min(max, n));
}

function toNum(v: unknown, fallback: number) {
  const n = typeof v === "number" ? v : Number(v);
  return Number.isFinite(n) ? n : fallback;
}

/** Circular undo / redo arrow (no numeral). */
function StepIcon({ dir }: { dir: "minus" | "plus" }) {
  const ccw = dir === "minus";
  return (
    <svg className="plugin-offset-step-icon" viewBox="0 0 24 24" aria-hidden>
      <path
        fill="none"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
        d={
          ccw
            ? "M8.2 7.4a6.4 6.4 0 1 0 3.8-4.2"
            : "M15.8 7.4a6.4 6.4 0 1 1-3.8-4.2"
        }
      />
      <path
        fill="none"
        stroke="currentColor"
        strokeWidth="1.9"
        strokeLinecap="round"
        strokeLinejoin="round"
        d={ccw ? "M7.5 3.6v3.4H10.9" : "M16.5 3.6v3.4H13.1"}
      />
    </svg>
  );
}

/**
 * Offset control: label row + control row
 * (−50 | slider | +50 | editable ms | reset)
 */
export default function PluginOffsetNumberField({
  field,
  value,
  disabled,
  onCommit,
}: Props) {
  const min = typeof field.min === "number" ? field.min : -10000;
  const max = typeof field.max === "number" ? field.max : 10000;
  const step = typeof field.step === "number" && field.step > 0 ? field.step : 50;
  const def = toNum(field.default, 0);
  const committed = clamp(Math.round(toNum(value, def)), min, max);

  const [local, setLocal] = useState(committed);
  const [draft, setDraft] = useState(String(committed));
  const [editing, setEditing] = useState(false);
  const persistTimer = useRef<number | null>(null);
  const latestLocal = useRef(local);
  latestLocal.current = local;

  useEffect(() => {
    if (editing) return;
    setLocal(committed);
    setDraft(String(committed));
  }, [committed, editing]);

  useEffect(() => {
    return () => {
      if (persistTimer.current != null) window.clearTimeout(persistTimer.current);
    };
  }, []);

  const flush = (n: number) => {
    const next = clamp(Math.round(n), min, max);
    setLocal(next);
    latestLocal.current = next;
    if (!editing) setDraft(String(next));
    onCommit(next);
  };

  const scheduleFlush = (n: number) => {
    const next = clamp(Math.round(n), min, max);
    setLocal(next);
    latestLocal.current = next;
    if (persistTimer.current != null) window.clearTimeout(persistTimer.current);
    persistTimer.current = window.setTimeout(() => {
      persistTimer.current = null;
      onCommit(latestLocal.current);
    }, 120);
  };

  const nudge = (delta: number) => {
    if (disabled) return;
    flush(local + delta);
  };

  const commitDraft = () => {
    setEditing(false);
    const raw = draft.trim().replace(/ms$/i, "").trim();
    if (raw === "" || raw === "-" || raw === "+") {
      setDraft(String(local));
      return;
    }
    const parsed = Number(raw);
    if (!Number.isFinite(parsed)) {
      setDraft(String(local));
      return;
    }
    flush(parsed);
  };

  const stepLabel = `${Math.abs(Math.round(step))}ms`;

  return (
    <div className={`plugin-offset${disabled ? " is-disabled" : ""}`}>
      <div className="plugin-offset-text">
        <span className="plugin-offset-label">{field.label}</span>
        {field.description ? (
          <span className="plugin-offset-desc">{field.description}</span>
        ) : null}
      </div>

      <div className="plugin-offset-row">
        <button
          type="button"
          className="plugin-offset-step"
          disabled={disabled || local <= min}
          title={`减少 ${stepLabel}`}
          aria-label={`减少 ${stepLabel}`}
          onClick={() => nudge(-step)}
        >
          <StepIcon dir="minus" />
          <span className="plugin-offset-step-caption">{stepLabel}</span>
        </button>

        <input
          type="range"
          className="plugin-offset-slider"
          disabled={disabled}
          min={min}
          max={max}
          step={step}
          value={local}
          aria-label={field.label}
          onChange={(e) => scheduleFlush(Number(e.target.value))}
          onPointerUp={() => {
            if (persistTimer.current != null) {
              window.clearTimeout(persistTimer.current);
              persistTimer.current = null;
            }
            onCommit(latestLocal.current);
          }}
        />

        <button
          type="button"
          className="plugin-offset-step"
          disabled={disabled || local >= max}
          title={`增加 ${stepLabel}`}
          aria-label={`增加 ${stepLabel}`}
          onClick={() => nudge(step)}
        >
          <StepIcon dir="plus" />
          <span className="plugin-offset-step-caption">{stepLabel}</span>
        </button>

        <label className="plugin-offset-value">
          <input
            type="text"
            inputMode="numeric"
            className="plugin-offset-input"
            disabled={disabled}
            aria-label={`${field.label}（毫秒）`}
            value={editing ? draft : String(local)}
            onFocus={() => {
              setEditing(true);
              setDraft(String(local));
            }}
            onChange={(e) => {
              setDraft(e.target.value.replace(/[^\d+\-]/g, ""));
            }}
            onBlur={() => commitDraft()}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                (e.target as HTMLInputElement).blur();
              } else if (e.key === "Escape") {
                e.preventDefault();
                setDraft(String(local));
                setEditing(false);
                (e.target as HTMLInputElement).blur();
              }
            }}
          />
          <span className="plugin-offset-unit" aria-hidden>
            ms
          </span>
        </label>

        <button
          type="button"
          className="plugin-offset-reset"
          disabled={disabled || local === def}
          title="重置为 0"
          aria-label="重置偏移"
          onClick={() => flush(def)}
        >
          <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden>
            <circle
              cx="12"
              cy="12"
              r="3.2"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
            />
            <circle cx="12" cy="12" r="1.15" fill="currentColor" />
            <path
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              d="M12 3.8v2.2M12 18v2.2M3.8 12h2.2M18 12h2.2"
            />
          </svg>
        </button>
      </div>
    </div>
  );
}

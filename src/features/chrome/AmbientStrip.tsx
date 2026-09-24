import { useState, type CSSProperties } from "react";
import "./ambientStrip.css";

/** Two bounded layers: animate opacity, including PNG ribbons (which CSS
 * background-image cannot interpolate). New samples replace, never queue. */
export function AmbientStrip({ style }: { style: CSSProperties }) {
  const key = JSON.stringify(style);
  const [frame, setFrame] = useState({ key, current: style, previous: null as CSSProperties | null });
  if (frame.key !== key) {
    setFrame({ key, current: style, previous: frame.current });
  }
  return (
    <div className="ambient-strip" aria-hidden>
      {frame.previous && <div className="chrome-ambient-layer" style={frame.previous} />}
      <div
        key={frame.key}
        className={`chrome-ambient-layer${frame.previous ? " chrome-ambient-enter" : ""}`}
        style={frame.current}
      />
    </div>
  );
}

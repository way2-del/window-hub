/**
 * Glass page stays fully clear — Host paints frost:
 * SWCA (radius ≤ 8) or Composition HostBackdrop under this WebView2 (radius ≥ 9).
 */
export default function DockGlassApp() {
  return (
    <div
      aria-hidden
      style={{
        boxSizing: "border-box",
        width: "100%",
        height: "100%",
        margin: 0,
        padding: 0,
        border: "none",
        borderRadius: 0,
        background: "transparent",
        boxShadow: "none",
        pointerEvents: "none",
      }}
    />
  );
}

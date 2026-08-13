/** Built-in Dock glyphs — no plate fill; Host tile owns solid background. */

type DockGlyphProps = {
  className?: string;
};

/** Windows-style Start glyph (white panes). */
export function DockStartIcon({ className }: DockGlyphProps) {
  return (
    <svg
      className={`dock-glyph is-start ${className ?? ""}`.trim()}
      viewBox="0 0 32 32"
      width="100%"
      height="100%"
      aria-hidden
    >
      <g fill="#fff">
        <rect x="7" y="7" width="7.5" height="7.5" rx="1.2" />
        <rect x="17.5" y="7" width="7.5" height="7.5" rx="1.2" />
        <rect x="7" y="17.5" width="7.5" height="7.5" rx="1.2" />
        <rect x="17.5" y="17.5" width="7.5" height="7.5" rx="1.2" />
      </g>
    </svg>
  );
}

/** Recycle-bin glyph. */
export function DockTrashIcon({ className }: DockGlyphProps) {
  return (
    <svg
      className={`dock-glyph is-trash ${className ?? ""}`.trim()}
      viewBox="0 0 32 32"
      width="100%"
      height="100%"
      aria-hidden
    >
      <g
        fill="none"
        stroke="#F5F5F7"
        strokeWidth="1.7"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path d="M10.5 12.2h11" />
        <path d="M13.2 12.2V10.6c0-.7.5-1.2 1.2-1.2h3.2c.7 0 1.2.5 1.2 1.2v1.6" />
        <path d="M12.2 12.2l.7 11.2c.05.7.6 1.2 1.3 1.2h3.6c.7 0 1.25-.5 1.3-1.2l.7-11.2" />
        <path d="M14.4 15.2v6.2M17.6 15.2v6.2" />
      </g>
      <path
        d="M10.5 12.2h11"
        fill="none"
        stroke="#34C759"
        strokeWidth="1.7"
        strokeLinecap="round"
        opacity="0.85"
      />
    </svg>
  );
}

export const DOCK_START_BG = "#0078D4";
export const DOCK_TRASH_BG = "#3A3A3C";
export const DOCK_AUTO_PLATE_BG = "#f2f2f7";

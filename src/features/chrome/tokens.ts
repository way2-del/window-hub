export type Rgb = { r: number; g: number; b: number };

/** sRGB 相对亮度，用于顶栏文字黑白切换 */
export function srgbLuma({ r, g, b }: Rgb) {
  const toLin = (c: number) => {
    const s = c / 255;
    return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * toLin(r) + 0.7152 * toLin(g) + 0.0722 * toLin(b);
}

export type ChromeTokens = {
  fg: string;
  fgHover: string;
  shadow: string;
  glyphShadow: string;
  scheme: "light" | "dark";
};

/** 浅色背景用纯黑字，深色背景用白字，避免顶栏看不见 */
export function chromeTokens(rgb: Rgb): ChromeTokens {
  if (srgbLuma(rgb) >= 0.52) {
    return {
      fg: "#000000",
      fgHover: "#000000",
      shadow: "none",
      glyphShadow: "drop-shadow(0 0.5px 0.5px rgba(255, 255, 255, 0.7))",
      scheme: "light",
    };
  }
  return {
    fg: "rgba(255, 255, 255, 0.94)",
    fgHover: "#ffffff",
    shadow: "0 1px 2px rgba(0, 0, 0, 0.35)",
    glyphShadow: "drop-shadow(0 1px 1px rgba(0, 0, 0, 0.28))",
    scheme: "dark",
  };
}

export function chromeCssVars(prefix: "left" | "right" | "center", t: ChromeTokens): Record<string, string> {
  return {
    [`--chrome-${prefix}-fg`]: t.fg,
    [`--chrome-${prefix}-fg-hover`]: t.fgHover,
    [`--chrome-${prefix}-shadow`]: t.shadow,
    [`--chrome-${prefix}-glyph-shadow`]: t.glyphShadow,
  };
}


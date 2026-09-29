/**
 * Cozmo / Bloub — shortcuts strip
 * Plays saved montage + appearance (shape / color / expression).
 */
import {
  defaultAppearance,
  expressionLabel,
  inkOf,
  normalizeAppearance,
  paperOf,
  SHAPE_LABELS,
  type Appearance,
} from "./appearance";
import { blocksForPreset } from "./cycle-presets";
import { createBloubMount, type BloubMount } from "./mount";
import { normalizeBlocks, type MontageBlock } from "./sequence";

declare const window: Window & { hub?: any };

(function () {
  const chip = document.getElementById("chip");
  if (!chip) return;

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  function barHeightPx() {
    const raw = getComputedStyle(document.documentElement).getPropertyValue("--wh-bar-h").trim();
    const n = parseFloat(raw);
    return Number.isFinite(n) && n > 0 ? n : 28;
  }

  let mount: BloubMount | null = null;
  let cycleKey = "face";
  let appearance: Appearance = defaultAppearance();
  let montage: MontageBlock[] = blocksForPreset("face");
  let lookSig = "";

  function reportSize(px: number) {
    try {
      const h = hub();
      if (h.shortcuts?.requestSize) {
        h.shortcuts.requestSize({ width: Math.max(px + 2, barHeightPx()) });
      }
    } catch (_) { /* hub not ready */ }
  }

  function resolveCycle(): MontageBlock[] {
    return montage.length ? montage : blocksForPreset(cycleKey === "custom" ? "face" : cycleKey);
  }

  function applyLook() {
    if (!mount) return;
    const ink = inkOf(appearance.colorId);
    mount.setColors({ ink, paper: paperOf(appearance.colorId) });
    mount.setShape(appearance.shapeId);
    mount.setRestExpression(appearance.expressionId);
    chip.style.color = ink;
  }

  function bootMount() {
    const s = Math.max(18, barHeightPx());
    const cycle = resolveCycle();
    const ink = inkOf(appearance.colorId);

    if (mount) {
      mount.setSize(s);
      applyLook();
      reportSize(s);
      return;
    }

    chip.innerHTML = "";
    chip.style.color = ink;
    try {
      mount = createBloubMount(chip, {
        size: s,
        ink,
        paper: paperOf(appearance.colorId),
        expression: appearance.expressionId,
        shape: appearance.shapeId,
        cycle,
        playing: true,
        tight: true,
        simple: true,
        className: "cz-bloub",
      });
      reportSize(s);
      window.setTimeout(() => reportSize(s), 50);
      window.setTimeout(() => reportSize(s), 200);
    } catch (err) {
      console.error("[cozmo] mount failed", err);
      chip.innerHTML =
        '<svg class="cz-bloub" viewBox="0 0 28 28" width="' +
        s +
        '" height="' +
        s +
        '" aria-hidden="true">' +
        '<circle cx="14" cy="14" r="11" fill="currentColor"/>' +
        '<ellipse cx="10" cy="13" rx="2.2" ry="4.2" fill="#fff" transform="rotate(-26 10 13)"/>' +
        '<ellipse cx="18.5" cy="13.2" rx="1.6" ry="3.8" fill="#fff" transform="rotate(-26 18.5 13.2)"/>' +
        "</svg>";
      reportSize(s);
    }
  }

  function signature(): string {
    const c = resolveCycle()
      .map((b) => `${b.state}:${b.duration}:${b.expression || ""}`)
      .join("|");
    return `${appearance.shapeId}|${appearance.colorId}|${appearance.expressionId}|${c}`;
  }

  function applyConfig(
    preset: string,
    nextAppearance: Appearance,
    custom: MontageBlock[] | null,
  ) {
    let nextPreset = preset || "face";
    if (nextPreset === "default") nextPreset = "face";
    appearance = nextAppearance;

    if (custom && custom.length) {
      montage = custom;
      cycleKey = "custom";
    } else {
      cycleKey = nextPreset === "custom" ? "face" : nextPreset;
      montage = blocksForPreset(cycleKey);
    }

    const sig = signature();
    const changed = sig !== lookSig;
    lookSig = sig;

    bootMount();
    if (mount && changed) {
      applyLook();
      mount.setCycle(resolveCycle(), true);
      mount.setPlaying(true);
    }
    chip.title = `Bloub · ${SHAPE_LABELS[appearance.shapeId as keyof typeof SHAPE_LABELS] || appearance.shapeId}/${expressionLabel(appearance.expressionId)}`;
  }

  async function loadConfig() {
    try {
      const h = hub();
      const settings = await h.settings.getAll();
      let preset = String(settings.shortcutsCycle || "face");
      const savedCycle = await h.storage.get("shortcutsCycle");
      if (typeof savedCycle === "string" && savedCycle) preset = savedCycle;

      let app = normalizeAppearance(await h.storage.get("appearance"));
      const shapeId = await h.storage.get("shapeId");
      const colorId = await h.storage.get("colorId");
      const expression = await h.storage.get("expression");
      if (typeof shapeId === "string" && shapeId) app = { ...app, shapeId };
      if (typeof colorId === "string" && colorId) app = { ...app, colorId };
      if (typeof expression === "string" && expression) app = { ...app, expressionId: expression };
      app = normalizeAppearance(app);

      const custom = normalizeBlocks(await h.storage.get("montage"));
      applyConfig(preset, app, custom);
    } catch (_) {
      applyConfig("face", defaultAppearance(), null);
    }
  }

  chip.addEventListener("click", async () => {
    try {
      const h = hub();
      const settings = await h.settings.getAll();
      await h.popup.open({
        width: Number(settings.popupWidth) || 640,
        height: Number(settings.popupHeight) || 520,
      });
    } catch (err) {
      console.warn("[cozmo] popup.open failed", err);
    }
  });

  bootMount();
  loadConfig();
  // Appearance can change from popup; poll often enough that shape/color feel live.
  const poll = setInterval(loadConfig, 800);

  window.addEventListener("beforeunload", () => {
    clearInterval(poll);
    mount?.destroy();
  });
})();

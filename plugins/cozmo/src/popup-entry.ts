/**
 * Cozmo / Bloub — popup: appearance (shape/color/expr) + sequence editor
 */
import { clampDuration } from "../vendor/bot/cycles";
import {
  COLOR_LABELS,
  COLORS,
  defaultAppearance,
  expressionLabel,
  EXPRESSION_LABELS,
  inkOf,
  normalizeAppearance,
  paperOf,
  SHAPE_LABELS,
  SHAPES,
  type Appearance,
} from "./appearance";
import { EXPRESSIONS, createBloubMount, type BloubMount } from "./mount";
import {
  animBlock,
  builtinScenes,
  CATALOG_STATES,
  exprBlock,
  labelBlock,
  newSceneId,
  normalizeBlocks,
  normalizeScenes,
  STATE_LABELS,
  totalSeconds,
  type MontageBlock,
  type Scene,
} from "./sequence";
import type { StateId } from "../vendor/bot/states";

declare const window: Window & { hub?: any };

(function () {
  const root = document.getElementById("app");
  if (!root) throw new Error("#app missing");

  function hub() {
    if (!window.hub) throw new Error("window.hub missing");
    return window.hub;
  }

  root.innerHTML = `
    <div class="cz-top">
      <aside class="cz-side">
        <div class="cz-stage" id="stage"></div>
        <div class="cz-meta">
          <span>BLOUB</span>
          <strong id="status">—</strong>
        </div>
        <div class="cz-side-actions">
          <button type="button" class="cz-btn dashed" id="btnAddAnim">+ 动画</button>
          <button type="button" class="cz-btn dashed" id="btnAddExpr">+ 表情</button>
          <button type="button" class="cz-btn" id="btnPlay">播放</button>
          <button type="button" class="cz-btn active" id="btnSave">保存</button>
        </div>
      </aside>

      <div class="cz-main" id="main">
        <div class="cz-section">
          <div class="cz-section-label">形状</div>
          <div class="cz-panel" id="shapePanel" role="toolbar" aria-label="形状"></div>
        </div>

        <div class="cz-section">
          <div class="cz-section-label">颜色</div>
          <div class="cz-panel cz-colors" id="colorPanel" role="toolbar" aria-label="颜色"></div>
        </div>

        <div class="cz-section">
          <div class="cz-section-label">默认表情</div>
          <div class="cz-panel cz-expr" id="restExprPanel" role="toolbar" aria-label="默认表情"></div>
        </div>

        <div class="cz-section">
          <div class="cz-section-label">情景</div>
          <div class="cz-panel cz-scenes" id="scenePanel" role="tablist" aria-label="情景"></div>
        </div>

        <div class="cz-section cz-follow-section">
          <label class="cz-follow" title="关闭后视线回到形状正中，不再跟随鼠标">
            <span>是否跟随鼠标</span>
            <input type="checkbox" id="followMouse" checked />
          </label>
        </div>
      </div>
    </div>

    <div class="cz-apply-row" id="saveRow" hidden>
      <input id="sceneName" type="text" maxlength="24" placeholder="情景名称（可选）" />
      <button type="button" class="cz-btn active" id="btnSaveConfirm">另存</button>
      <button type="button" class="cz-btn" id="btnSaveCancel">关闭</button>
    </div>

    <div class="cz-bottom">
      <div class="cz-section-label">序列 <span class="cz-muted" id="seqMeta"></span></div>
      <div class="cz-track" id="track" role="list" aria-label="动画序列"></div>

      <div class="cz-editor" id="editor">
        <label class="cz-field">动画
          <select id="edState"></select>
        </label>
        <label class="cz-field">块表情
          <select id="edExpr"></select>
        </label>
        <label class="cz-field">秒
          <input id="edDur" type="number" min="0.6" max="10" step="0.1" />
        </label>
        <div class="cz-edit-actions">
          <button type="button" class="cz-btn" id="btnUp" title="上移">↑</button>
          <button type="button" class="cz-btn" id="btnDown" title="下移">↓</button>
          <button type="button" class="cz-btn" id="btnDel" title="删除">删</button>
        </div>
      </div>
    </div>
  `;

  const stage = document.getElementById("stage")!;
  const statusEl = document.getElementById("status")!;
  const shapePanel = document.getElementById("shapePanel")!;
  const colorPanel = document.getElementById("colorPanel")!;
  const restExprPanel = document.getElementById("restExprPanel")!;
  const scenePanel = document.getElementById("scenePanel")!;
  const track = document.getElementById("track")!;
  const seqMeta = document.getElementById("seqMeta")!;
  const edState = document.getElementById("edState") as HTMLSelectElement;
  const edExpr = document.getElementById("edExpr") as HTMLSelectElement;
  const edDur = document.getElementById("edDur") as HTMLInputElement;
  const saveRow = document.getElementById("saveRow")!;
  const sceneNameInput = document.getElementById("sceneName") as HTMLInputElement;
  const followMouseEl = document.getElementById("followMouse") as HTMLInputElement;

  let popupWidth = 640;
  let popupHeightCap = 900;
  let fitTimer = 0;
  let followMouse = true;

  function measureContentHeight() {
    const style = getComputedStyle(root);
    const padY =
      (parseFloat(style.paddingTop) || 0) + (parseFloat(style.paddingBottom) || 0);
    const gap = parseFloat(style.gap) || 0;
    const kids = Array.from(root.children) as HTMLElement[];
    let body = 0;
    let visible = 0;
    for (const child of kids) {
      if (child.hidden || child.hasAttribute("hidden")) continue;
      const rect = child.getBoundingClientRect();
      if (rect.height <= 0) continue;
      body += rect.height;
      visible += 1;
    }
    return Math.ceil(body + Math.max(0, visible - 1) * gap + padY + 4);
  }

  function fitPopupHeight() {
    window.clearTimeout(fitTimer);
    fitTimer = window.setTimeout(() => {
      try {
        const h = hub();
        if (!h.popup?.resize) return;
        const natural = measureContentHeight();
        const height = Math.max(420, Math.min(popupHeightCap, natural));
        h.popup.resize({ width: popupWidth, height }).catch(() => {});
      } catch (_) { /* hub not ready */ }
    }, 32);
  }

  CATALOG_STATES.forEach((id) => {
    const o = document.createElement("option");
    o.value = id;
    o.textContent = STATE_LABELS[id] || id;
    edState.appendChild(o);
  });
  const exprNone = document.createElement("option");
  exprNone.value = "";
  exprNone.textContent = "跟默认";
  edExpr.appendChild(exprNone);
  EXPRESSIONS.forEach((e) => {
    const o = document.createElement("option");
    o.value = e.id;
    o.textContent = expressionLabel(e.id);
    edExpr.appendChild(o);
  });

  let mount: BloubMount | null = null;
  let appearance: Appearance = defaultAppearance();
  let blocks: MontageBlock[] = builtinScenes()[1]!.blocks.slice();
  let customScenes: Scene[] = [];
  let activeSceneId = "face";
  let selected = 0;
  let playing = true;

  function allScenes(): Scene[] {
    return [...builtinScenes(), ...customScenes];
  }

  function updateStatus() {
    const b = blocks[selected];
    const tag = playing ? "▶" : "❚❚";
    const look = `${SHAPE_LABELS[appearance.shapeId as keyof typeof SHAPE_LABELS] || appearance.shapeId}/${expressionLabel(appearance.expressionId)}`;
    statusEl.textContent = b
      ? `${tag} ${selected + 1}/${blocks.length} · ${labelBlock(b)} · ${look}`
      : `${tag} ${look}`;
    seqMeta.textContent = blocks.length
      ? `· ${blocks.length} 块 · ${totalSeconds(blocks).toFixed(1)}s`
      : "· 空";
  }

  function sizeFromStage() {
    const rect = stage.getBoundingClientRect();
    return Math.max(72, Math.floor(Math.min(rect.width, rect.height) * 0.9));
  }

  function applyAppearance() {
    ensureMount();
    mount?.setShape(appearance.shapeId);
    mount?.setColors({
      ink: inkOf(appearance.colorId),
      paper: paperOf(appearance.colorId),
    });
    mount?.setRestExpression(appearance.expressionId);
    renderAppearance();
    updateStatus();
    // Appearance applies to shortcuts immediately — don't wait for「保存并应用」.
    persistAppearance();
  }

  function persistAppearance() {
    try {
      const h = hub();
      h.storage.set("appearance", appearance);
      h.storage.set("expression", appearance.expressionId);
      h.storage.set("shapeId", appearance.shapeId);
      h.storage.set("colorId", appearance.colorId);
      h.storage.set("followMouse", followMouse);
    } catch (_) { /* ignore */ }
  }

  function applyFollow() {
    followMouseEl.checked = followMouse;
    mount?.setFollow(followMouse);
  }

  function ensureMount() {
    if (mount) {
      mount.setSize(sizeFromStage());
      mount.setFollow(followMouse);
      return;
    }
    stage.innerHTML = "";
    mount = createBloubMount(stage, {
      size: sizeFromStage(),
      ink: inkOf(appearance.colorId),
      paper: paperOf(appearance.colorId),
      expression: appearance.expressionId,
      shape: appearance.shapeId,
      cycle: blocks,
      playing: true,
      follow: followMouse,
      simple: true,
      tight: false,
      className: "cz-bloub",
    });
    mount.onBlockChange((index) => {
      selected = index;
      renderTrack();
      syncEditor();
      updateStatus();
    });
  }

  function applyCycle(restart = true) {
    ensureMount();
    mount?.setCycle(blocks, restart);
    mount?.setPlaying(playing);
    if (!restart && blocks[selected]) mount?.seekBlock(selected);
  }

  function persistAll() {
    try {
      const h = hub();
      h.storage.set("montage", blocks);
      h.storage.set("activeSceneId", activeSceneId);
      h.storage.set("shortcutsCycle", "custom");
      persistAppearance();
    } catch (_) { /* ignore */ }
  }

  function persistScenes() {
    try {
      hub().storage.set("scenes", customScenes);
    } catch (_) { /* ignore */ }
  }

  function renderAppearance() {
    shapePanel.innerHTML = "";
    SHAPES.forEach((s) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "cz-btn" + (s.id === appearance.shapeId ? " active" : "");
      btn.textContent = SHAPE_LABELS[s.id] || s.id;
      btn.addEventListener("click", () => {
        appearance = { ...appearance, shapeId: s.id };
        applyAppearance();
      });
      shapePanel.appendChild(btn);
    });

    colorPanel.innerHTML = "";
    COLORS.forEach((c) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "cz-swatch" + (c.id === appearance.colorId ? " active" : "");
      btn.title = COLOR_LABELS[c.id] || c.id;
      btn.style.setProperty("--swatch", c.hex);
      btn.addEventListener("click", () => {
        appearance = { ...appearance, colorId: c.id };
        applyAppearance();
      });
      colorPanel.appendChild(btn);
    });

    restExprPanel.innerHTML = "";
    EXPRESSIONS.forEach((e) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "cz-btn" + (e.id === appearance.expressionId ? " active" : "");
      btn.textContent = EXPRESSION_LABELS[e.id] || e.id;
      btn.title = e.id;
      btn.addEventListener("click", () => {
        appearance = { ...appearance, expressionId: e.id };
        applyAppearance();
      });
      restExprPanel.appendChild(btn);
    });
  }

  function renderScenes() {
    scenePanel.innerHTML = "";
    allScenes().forEach((sc) => {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "cz-btn" + (sc.id === activeSceneId ? " active" : "");
      btn.textContent = sc.name;
      btn.title = sc.builtin ? "内置情景" : "自定义（右键删除）";
      btn.addEventListener("click", () => loadScene(sc.id));
      if (!sc.builtin) {
        btn.addEventListener("contextmenu", (ev) => {
          ev.preventDefault();
          customScenes = customScenes.filter((s) => s.id !== sc.id);
          persistScenes();
          if (activeSceneId === sc.id) loadScene("face");
          else renderScenes();
        });
      }
      scenePanel.appendChild(btn);
    });
  }

  function renderTrack() {
    track.innerHTML = "";
    blocks.forEach((b, i) => {
      const pill = document.createElement("button");
      pill.type = "button";
      pill.className =
        "cz-pill" +
        (i === selected ? " is-selected" : "") +
        (i === mount?.getBlockIndex() && playing ? " is-playing" : "");
      pill.innerHTML = `<span class="cz-pill-i">${i + 1}</span><span class="cz-pill-t">${labelBlock(b)}</span><span class="cz-pill-d">${b.duration.toFixed(1)}s</span>`;
      pill.addEventListener("click", () => {
        selected = i;
        playing = false;
        mount?.setPlaying(false);
        mount?.seekBlock(i);
        renderTrack();
        syncEditor();
        updateStatus();
      });
      track.appendChild(pill);
    });
    if (!blocks.length) {
      const empty = document.createElement("div");
      empty.className = "cz-track-empty";
      empty.textContent = "点「+ 动画」或「+ 表情」开始编排";
      track.appendChild(empty);
    }
    fitPopupHeight();
  }

  function syncEditor() {
    const b = blocks[selected];
    const disabled = !b;
    edState.disabled = disabled;
    edExpr.disabled = disabled;
    edDur.disabled = disabled;
    (document.getElementById("btnUp") as HTMLButtonElement).disabled = disabled || selected <= 0;
    (document.getElementById("btnDown") as HTMLButtonElement).disabled =
      disabled || selected >= blocks.length - 1;
    (document.getElementById("btnDel") as HTMLButtonElement).disabled = disabled;
    if (!b) return;
    edState.value = b.state;
    edExpr.value = b.expression || "";
    edDur.value = String(b.duration);
  }

  function mutateSelected(patch: Partial<MontageBlock>) {
    const b = blocks[selected];
    if (!b) return;
    const next: MontageBlock = { ...b, ...patch };
    if (patch.state) next.duration = clampDuration(patch.state, next.duration);
    if (patch.duration != null) next.duration = clampDuration(next.state, patch.duration);
    if (patch.expression === "") delete next.expression;
    blocks[selected] = next;
    activeSceneId = "custom";
    renderScenes();
    renderTrack();
    syncEditor();
    applyCycle(false);
    mount?.seekBlock(selected);
    updateStatus();
  }

  function loadScene(id: string) {
    const sc = allScenes().find((s) => s.id === id);
    if (!sc) return;
    activeSceneId = id;
    blocks = sc.blocks.map((b) => ({ ...b }));
    selected = 0;
    playing = true;
    renderScenes();
    renderTrack();
    syncEditor();
    applyCycle(true);
    updateStatus();
  }

  edState.addEventListener("change", () => mutateSelected({ state: edState.value as StateId }));
  edExpr.addEventListener("change", () => mutateSelected({ expression: edExpr.value || "" }));
  edDur.addEventListener("change", () => mutateSelected({ duration: Number(edDur.value) || 1 }));

  document.getElementById("btnUp")!.addEventListener("click", () => {
    if (selected <= 0) return;
    [blocks[selected - 1], blocks[selected]] = [blocks[selected]!, blocks[selected - 1]!];
    selected -= 1;
    activeSceneId = "custom";
    renderScenes();
    renderTrack();
    syncEditor();
    applyCycle(false);
  });
  document.getElementById("btnDown")!.addEventListener("click", () => {
    if (selected >= blocks.length - 1) return;
    [blocks[selected + 1], blocks[selected]] = [blocks[selected]!, blocks[selected + 1]!];
    selected += 1;
    activeSceneId = "custom";
    renderScenes();
    renderTrack();
    syncEditor();
    applyCycle(false);
  });
  document.getElementById("btnDel")!.addEventListener("click", () => {
    if (!blocks.length) return;
    blocks.splice(selected, 1);
    selected = Math.max(0, Math.min(selected, blocks.length - 1));
    activeSceneId = "custom";
    renderScenes();
    renderTrack();
    syncEditor();
    applyCycle(true);
    updateStatus();
  });
  document.getElementById("btnAddAnim")!.addEventListener("click", () => {
    blocks.splice(selected + 1, 0, animBlock("wink"));
    selected += 1;
    activeSceneId = "custom";
    renderScenes();
    renderTrack();
    syncEditor();
    applyCycle(false);
    updateStatus();
  });
  document.getElementById("btnAddExpr")!.addEventListener("click", () => {
    blocks.splice(selected + 1, 0, exprBlock(appearance.expressionId || "heureux", 2.5));
    selected += 1;
    activeSceneId = "custom";
    renderScenes();
    renderTrack();
    syncEditor();
    applyCycle(false);
    updateStatus();
  });
  document.getElementById("btnPlay")!.addEventListener("click", () => {
    playing = true;
    applyCycle(true);
    updateStatus();
    renderTrack();
  });
  document.getElementById("btnSave")!.addEventListener("click", () => {
    persistAll();
    saveRow.hidden = false;
    sceneNameInput.focus();
    statusEl.textContent = "已保存并应用到快捷区";
    fitPopupHeight();
  });
  document.getElementById("btnSaveCancel")!.addEventListener("click", () => {
    saveRow.hidden = true;
    fitPopupHeight();
  });
  document.getElementById("btnSaveConfirm")!.addEventListener("click", () => {
    const name = sceneNameInput.value.trim() || `情景 ${customScenes.length + 1}`;
    const id = newSceneId();
    customScenes.push({ id, name, blocks: blocks.map((b) => ({ ...b })) });
    activeSceneId = id;
    persistScenes();
    persistAll();
    saveRow.hidden = true;
    renderScenes();
    statusEl.textContent = `已保存「${name}」`;
    fitPopupHeight();
  });

  followMouseEl.addEventListener("change", () => {
    followMouse = !!followMouseEl.checked;
    applyFollow();
    persistAppearance();
  });

  window.addEventListener("resize", () => {
    mount?.setSize(sizeFromStage());
  });

  async function boot() {
    try {
      const h = hub();
      const settings = await h.settings.getAll();
      popupWidth = Number(settings.popupWidth) || 640;
      popupHeightCap = Number(settings.popupHeight) || 900;
      if (h.popup?.resize) {
        // Width from settings; height fitted after first paint.
        h.popup.resize({ width: popupWidth, height: 480 }).catch(() => {});
      }
      appearance = normalizeAppearance(await h.storage.get("appearance"));
      // legacy keys
      const shapeId = await h.storage.get("shapeId");
      const colorId = await h.storage.get("colorId");
      const expression = await h.storage.get("expression");
      if (typeof shapeId === "string" && shapeId) appearance.shapeId = shapeId;
      if (typeof colorId === "string" && colorId) appearance.colorId = colorId;
      if (typeof expression === "string" && expression) appearance.expressionId = expression;
      appearance = normalizeAppearance(appearance);

      const savedFollow = await h.storage.get("followMouse");
      if (typeof savedFollow === "boolean") followMouse = savedFollow;
      else if (savedFollow === "false" || savedFollow === 0) followMouse = false;
      else if (savedFollow === "true" || savedFollow === 1) followMouse = true;

      customScenes = normalizeScenes(await h.storage.get("scenes"));
      const savedMontage = normalizeBlocks(await h.storage.get("montage"));
      const savedSceneId = String((await h.storage.get("activeSceneId")) || "");
      if (savedMontage) {
        blocks = savedMontage;
        activeSceneId = savedSceneId || "custom";
      } else if (savedSceneId && allScenes().some((s) => s.id === savedSceneId)) {
        renderAppearance();
        loadScene(savedSceneId);
        applyAppearance();
        applyFollow();
        fitPopupHeight();
        window.setTimeout(fitPopupHeight, 80);
        window.setTimeout(fitPopupHeight, 200);
        return;
      } else {
        const preset = String(settings.shortcutsCycle || "face");
        renderAppearance();
        loadScene(preset === "default" || preset === "custom" ? "face" : preset);
        applyAppearance();
        applyFollow();
        fitPopupHeight();
        window.setTimeout(fitPopupHeight, 80);
        window.setTimeout(fitPopupHeight, 200);
        return;
      }
    } catch (_) {
      blocks = builtinScenes()[1]!.blocks.slice();
      activeSceneId = "face";
    }
    renderAppearance();
    renderScenes();
    renderTrack();
    syncEditor();
    ensureMount();
    applyAppearance();
    applyFollow();
    applyCycle(true);
    updateStatus();
    fitPopupHeight();
    window.setTimeout(fitPopupHeight, 80);
    window.setTimeout(fitPopupHeight, 200);
  }

  boot();
  window.addEventListener("beforeunload", () => mount?.destroy());
})();

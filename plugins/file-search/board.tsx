/**
 * File-search home board — react-grid-layout (drag + corner resize, grid-snapped).
 * Specs: 2 cols × 2 rows, w/h ∈ {1,2}. Built to board.js IIFE.
 */
import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import GridLayout, {
  noCompactor,
  type Layout,
  type LayoutItem,
} from "react-grid-layout";
import rglCss from "react-grid-layout/css/styles.css";
import resizeCss from "react-resizable/css/styles.css";

export type Pin = { id: string; name: string; path: string; color: string };
export type BoardCard = {
  id: string;
  kind: "pins" | "history";
  title: string;
  x: number;
  y: number;
  w: number;
  h: number;
  pins: Pin[];
};

export type LayoutOut = {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
};

export type BoardProps = {
  cards: BoardCard[];
  history: string[];
  editing: boolean;
  /** Inline rename target inside card chrome */
  renameCardId?: string | null;
  renameValue?: string;
  onLayoutChange: (items: LayoutOut[]) => void;
  onOpenPin: (path: string) => void;
  onHistoryClick: (query: string) => void;
  onAddPin: (cardId: string) => void;
  onRemovePin: (cardId: string, pinId: string) => void;
  onRenameCard: (cardId: string) => void;
  onRenameCommit: (cardId: string, title: string) => void;
  onRenameCancel: () => void;
  onDeleteCard: (cardId: string) => void;
};

const GRID_COLS = 2;
const MAX_SPAN = 2;
export const MAX_ROWS = 2;

const SNAP_COMPACTOR = {
  ...noCompactor,
  preventCollision: true,
  allowOverlap: false,
};

function clampSpan(n: number) {
  const v = Math.round(Number(n));
  if (!Number.isFinite(v)) return 1;
  return Math.max(1, Math.min(MAX_SPAN, v));
}

function pinCols(w: number) {
  // 1× → 4 cols, 2× → 8 cols (icon size stays fixed; more slots when wider)
  return 4 * clampSpan(w);
}

function pinRows(h: number) {
  // 1× → 2 rows, 2× → 4 rows
  return 2 * clampSpan(h);
}

/** 1×1→8, 2×1→16, 1×2→16, 2×2→32 — icons keep fixed size */
function pinCapacity(w: number, h: number) {
  return pinCols(w) * pinRows(h);
}

function rectsOverlap(
  a: { x: number; y: number; w: number; h: number },
  b: { x: number; y: number; w: number; h: number },
) {
  return !(
    a.x + a.w <= b.x ||
    b.x + b.w <= a.x ||
    a.y + a.h <= b.y ||
    b.y + b.h <= a.y
  );
}

export function packCards(cards: BoardCard[], maxRows = MAX_ROWS): BoardCard[] {
  const placed: BoardCard[] = [];
  for (const raw of cards) {
    let w = clampSpan(raw.w);
    let h = clampSpan(raw.h);
    if (h > maxRows) h = maxRows;
    let placedOne: BoardCard | null = null;
    const candidates: Array<{ w: number; h: number }> = [
      { w, h },
      { w, h: 1 },
      { w: 1, h },
      { w: 1, h: 1 },
    ];
    outer: for (const size of candidates) {
      for (let y = 0; y <= maxRows - size.h; y += 1) {
        for (let x = 0; x <= GRID_COLS - size.w; x += 1) {
          const probe = { x, y, w: size.w, h: size.h };
          if (!placed.some((p) => rectsOverlap(probe, p))) {
            placedOne = { ...raw, ...probe };
            break outer;
          }
        }
      }
    }
    if (placedOne) placed.push(placedOne);
  }
  return placed;
}

function ensureCss() {
  if (document.getElementById("file-search-rgl-css")) return;
  const style = document.createElement("style");
  style.id = "file-search-rgl-css";
  style.textContent = `${String(rglCss)}\n${String(resizeCss)}`;
  document.head.appendChild(style);
}

function normalizeLayout(layout: Layout): LayoutOut[] {
  return layout.map((l) => ({
    id: l.i,
    x: Math.max(0, Math.min(GRID_COLS - 1, l.x)),
    y: Math.max(0, Math.min(MAX_ROWS - 1, l.y)),
    w: clampSpan(l.w),
    h: Math.min(MAX_ROWS, clampSpan(l.h)),
  }));
}

function PinGrid({
  card,
  editing,
  onOpenPin,
  onAddPin,
  onRemovePin,
}: {
  card: BoardCard;
  editing: boolean;
  onOpenPin: (path: string) => void;
  onAddPin: (cardId: string) => void;
  onRemovePin: (cardId: string, pinId: string) => void;
}) {
  const cap = pinCapacity(card.w, card.h);
  const cols = pinCols(card.w);
  const rows = pinRows(card.h);
  const pins = card.pins.slice(0, cap);
  return (
    <div
      className="pin-grid"
      style={
        {
          ["--pin-cols" as string]: cols,
          ["--pin-rows" as string]: rows,
        } as React.CSSProperties
      }
    >      {pins.length === 0 && !editing ? (
        <div className="empty-mini">编辑模式可添加快捷</div>
      ) : null}
      {pins.map((p) => (
        <button
          key={p.id}
          type="button"
          className="pin-tile"
          title={p.path}
          onClick={(e) => {
            if ((e.target as HTMLElement).closest("[data-remove-pin]")) return;
            if (editing) return;
            if (p.path) onOpenPin(p.path);
          }}
        >
          <span className="pin-icon" style={{ background: p.color }}>
            {(p.name || "?").slice(0, 1)}
          </span>
          <span className="pin-name">{p.name}</span>
          {editing ? (
            <span
              className="pin-remove"
              data-remove-pin=""
              title="移除"
              onClick={(e) => {
                e.stopPropagation();
                onRemovePin(card.id, p.id);
              }}
            >
              ×
            </span>
          ) : null}
        </button>
      ))}
      {editing && pins.length < cap ? (
        <button
          type="button"
          className="pin-tile is-add"
          title="添加快捷"
          onClick={() => onAddPin(card.id)}
        >
          <span className="pin-icon is-add">+</span>
          <span className="pin-name">添加</span>
        </button>
      ) : null}
    </div>
  );
}

function HistoryBody({
  history,
  editing,
  onHistoryClick,
}: {
  history: string[];
  editing: boolean;
  onHistoryClick: (q: string) => void;
}) {
  return (
    <div className="hist-list">
      {history.length === 0 ? (
        <div className="empty-mini">回车搜索后会出现在这里</div>
      ) : (
        history.slice(0, 24).map((h, i) => (
          <button
            key={`${i}-${h}`}
            type="button"
            className="hist-row"
            onClick={() => {
              if (!editing) onHistoryClick(h);
            }}
          >
            <span className="hist-dot" aria-hidden />
            <span className="hist-text">{h}</span>
          </button>
        ))
      )}
    </div>
  );
}

function fitInputToText(el: HTMLInputElement, minSample = "字字", maxPx = 140) {
  const cs = getComputedStyle(el);
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
  const sample = el.value || minSample;
  const raw = Math.ceil(ctx.measureText(sample).width) + 4;
  const minW = Math.ceil(ctx.measureText(minSample).width) + 4;
  el.style.width = `${Math.max(minW, Math.min(maxPx, raw))}px`;
}

function TitleRenameChip({
  value,
  onCommit,
  onCancel,
}: {
  value: string;
  onCommit: (title: string) => void;
  onCancel: () => void;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [text, setText] = useState(value);

  useEffect(() => {
    const el = inputRef.current;
    if (!el) return;
    fitInputToText(el);
    el.focus();
    el.select();
  }, []);

  useEffect(() => {
    if (inputRef.current) fitInputToText(inputRef.current);
  }, [text]);

  return (
    <div
      className="chip-edit is-on-card"
      onMouseDown={(e) => e.stopPropagation()}
      onPointerDown={(e) => e.stopPropagation()}
    >
      <button
        type="button"
        className="chip-btn cancel"
        title="取消"
        aria-label="取消"
        onClick={(e) => {
          e.stopPropagation();
          onCancel();
        }}
      >
        <svg className="chip-ico" viewBox="0 0 16 16" aria-hidden>
          <path
            d="M4.2 4.2l7.6 7.6M11.8 4.2L4.2 11.8"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
          />
        </svg>
      </button>
      <input
        ref={inputRef}
        className="chip-input"
        type="text"
        value={text}
        placeholder="名称"
        autoComplete="off"
        spellCheck={false}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Enter") {
            e.preventDefault();
            onCommit(text);
          } else if (e.key === "Escape") {
            e.preventDefault();
            onCancel();
          }
        }}
      />
      <button
        type="button"
        className="chip-btn ok"
        title="确认"
        aria-label="确认"
        onClick={(e) => {
          e.stopPropagation();
          onCommit(text);
        }}
      >
        <svg className="chip-ico" viewBox="0 0 16 16" aria-hidden>
          <path
            d="M3.2 8.2l3.2 3.2 6.4-6.8"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
      </button>
    </div>
  );
}

function CardChrome({
  card,
  history,
  editing,
  renaming,
  renameValue,
  onOpenPin,
  onHistoryClick,
  onAddPin,
  onRemovePin,
  onRenameCard,
  onRenameCommit,
  onRenameCancel,
  onDeleteCard,
}: {
  card: BoardCard;
  history: string[];
  editing: boolean;
  renaming: boolean;
  renameValue: string;
  onOpenPin: (path: string) => void;
  onHistoryClick: (q: string) => void;
  onAddPin: (cardId: string) => void;
  onRemovePin: (cardId: string, pinId: string) => void;
  onRenameCard: (cardId: string) => void;
  onRenameCommit: (cardId: string, title: string) => void;
  onRenameCancel: () => void;
  onDeleteCard: (cardId: string) => void;
}) {
  return (
    <>
      <div
        className={`card-head${editing && !renaming ? " is-drag" : ""}${renaming ? " is-renaming" : ""}`}
      >
        {renaming ? (
          <TitleRenameChip
            value={renameValue || card.title}
            onCommit={(title) => onRenameCommit(card.id, title)}
            onCancel={onRenameCancel}
          />
        ) : (
          <div className="card-title">{card.title}</div>
        )}
        {editing && !renaming ? (
          <div className="card-actions">
            {card.kind === "pins" ? (
              <>
                <button
                  type="button"
                  className="card-action"
                  title="重命名"
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    onRenameCard(card.id);
                  }}
                >
                  名
                </button>
                <button
                  type="button"
                  className="card-action danger"
                  title="删除"
                  onMouseDown={(e) => e.stopPropagation()}
                  onClick={(e) => {
                    e.stopPropagation();
                    onDeleteCard(card.id);
                  }}
                >
                  删
                </button>
              </>
            ) : (
              <span className="card-badge">系统</span>
            )}
          </div>
        ) : null}
      </div>
      {card.kind === "history" ? (
        <HistoryBody
          history={history}
          editing={editing}
          onHistoryClick={onHistoryClick}
        />
      ) : (
        <PinGrid
          card={card}
          editing={editing}
          onOpenPin={onOpenPin}
          onAddPin={onAddPin}
          onRemovePin={onRemovePin}
        />
      )}
    </>
  );
}

function BoardApp(props: BoardProps) {
  const {
    cards,
    history,
    editing,
    renameCardId = null,
    renameValue = "",
    onLayoutChange,
    onOpenPin,
    onHistoryClick,
    onAddPin,
    onRemovePin,
    onRenameCard,
    onRenameCommit,
    onRenameCancel,
    onDeleteCard,
  } = props;

  const wrapRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(480);
  const [rowHeight, setRowHeight] = useState(96);
  const [margin, setMargin] = useState<[number, number]>([10, 10]);
  const interacting = useRef(false);

  const layoutFromCards: Layout = useMemo(
    () =>
      cards.map(
        (c): LayoutItem => ({
          i: c.id,
          x: Math.max(0, Math.min(GRID_COLS - clampSpan(c.w), c.x)),
          y: Math.max(0, Math.min(MAX_ROWS - Math.min(MAX_ROWS, clampSpan(c.h)), c.y)),
          w: clampSpan(c.w),
          h: Math.min(MAX_ROWS, clampSpan(c.h)),
          minW: 1,
          maxW: MAX_SPAN,
          minH: 1,
          maxH: MAX_ROWS,
        }),
      ),
    [cards],
  );

  const [liveLayout, setLiveLayout] = useState<Layout>(layoutFromCards);
  useEffect(() => {
    if (!interacting.current) setLiveLayout(layoutFromCards);
  }, [layoutFromCards]);

  const measure = useCallback(() => {
    const el = wrapRef.current;
    if (!el) return;
    const w = Math.max(160, Math.floor(el.clientWidth));
    // 仪表台矮壳（~140–180）勿再抬到 160，否则行高溢出裁切底角
    const h = Math.max(1, Math.floor(el.clientHeight));
    const compact = h < 200;
    const nextMargin: [number, number] = compact ? [8, 6] : [10, 10];
    const gaps = nextMargin[1] * (MAX_ROWS - 1);
    const rh = Math.max(40, Math.floor((h - gaps) / MAX_ROWS));
    setWidth(w);
    setRowHeight(rh);
    setMargin(nextMargin);
  }, []);

  useEffect(() => {
    ensureCss();
    measure();
    const el = wrapRef.current;
    if (!el || typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", measure);
      return () => window.removeEventListener("resize", measure);
    }
    const ro = new ResizeObserver(() => measure());
    ro.observe(el);
    return () => ro.disconnect();
  }, [measure, cards.length, editing]);

  const geomOf = useCallback(
    (id: string) => {
      const l = liveLayout.find((x) => x.i === id);
      return l
        ? { w: clampSpan(l.w), h: Math.min(MAX_ROWS, clampSpan(l.h)) }
        : { w: 1, h: 1 };
    },
    [liveLayout],
  );

  const commit = useCallback(
    (next: Layout) => {
      const normalized = normalizeLayout(next);
      setLiveLayout(
        normalized.map((n) => ({
          i: n.id,
          x: n.x,
          y: n.y,
          w: n.w,
          h: n.h,
          minW: 1,
          maxW: MAX_SPAN,
          minH: 1,
          maxH: MAX_ROWS,
        })),
      );
      onLayoutChange(normalized);
    },
    [onLayoutChange],
  );

  return (
    <div className="board-wrap" ref={wrapRef}>
      <GridLayout
        className={`board-rgl${editing ? " is-editing" : ""}`}
        width={width}
        autoSize={false}
        style={{ height: "100%" }}
        gridConfig={{
          cols: GRID_COLS,
          rowHeight,
          margin,
          containerPadding: [0, 0],
          maxRows: MAX_ROWS,
        }}
        layout={liveLayout}
        dragConfig={{
          enabled: editing && !renameCardId,
          handle: ".card-head",
          cancel: ".card-action, .pin-tile, .hist-row, button, .chip-edit, input",
          bounded: true,
        }}
        resizeConfig={{
          enabled: editing,
          handles: ["se"],
        }}
        compactor={SNAP_COMPACTOR}
        onDragStart={(next) => {
          interacting.current = true;
          setLiveLayout(next);
        }}
        onDrag={(next) => setLiveLayout(next)}
        onDragStop={(next) => {
          interacting.current = false;
          commit(next);
        }}
        onResizeStart={(next) => {
          interacting.current = true;
          setLiveLayout(next);
        }}
        onResize={(next) => setLiveLayout(next)}
        onResizeStop={(next) => {
          interacting.current = false;
          commit(next);
        }}
      >
        {cards.map((card) => {
          const g = geomOf(card.id);
          const liveCard = { ...card, w: g.w, h: g.h };
          return (
            <div
              key={card.id}
              className={`card-group${editing ? " is-editing" : ""}${card.kind === "history" ? " is-history" : ""}`}
              data-card-id={card.id}
            >
              <CardChrome
                card={liveCard}
                history={history}
                editing={editing}
                renaming={renameCardId === card.id}
                renameValue={renameCardId === card.id ? renameValue : ""}
                onOpenPin={onOpenPin}
                onHistoryClick={onHistoryClick}
                onAddPin={onAddPin}
                onRemovePin={onRemovePin}
                onRenameCard={onRenameCard}
                onRenameCommit={onRenameCommit}
                onRenameCancel={onRenameCancel}
                onDeleteCard={onDeleteCard}
              />
            </div>
          );
        })}
      </GridLayout>
    </div>
  );
}

type MountApi = {
  update: (props: BoardProps) => void;
  unmount: () => void;
};

let root: Root | null = null;
let hostEl: HTMLElement | null = null;

function mount(el: HTMLElement, props: BoardProps): MountApi {
  ensureCss();
  if (hostEl !== el) {
    root?.unmount();
    hostEl = el;
    root = createRoot(el);
  }
  root!.render(<BoardApp {...props} />);
  return {
    update(next) {
      root?.render(<BoardApp {...next} />);
    },
    unmount() {
      root?.unmount();
      root = null;
      hostEl = null;
    },
  };
}

window.FileSearchBoard = { mount, packCards, MAX_ROWS };

declare global {
  interface Window {
    FileSearchBoard?: {
      mount: typeof mount;
      packCards: typeof packCards;
      MAX_ROWS: number;
    };
  }
}

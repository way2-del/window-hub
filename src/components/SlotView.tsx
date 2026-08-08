import { useCallback, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SlotInfo } from "../types";

type Props = {
  slot: SlotInfo;
  frameUrl?: string;
  onRequestAttach: (slot: number) => void;
  onDetach: (slot: number) => void;
  onSwapNext: (slot: number) => void;
};

type DragBox = { x0: number; y0: number; x1: number; y1: number } | null;

export default function SlotView({
  slot,
  frameUrl,
  onRequestAttach,
  onDetach,
  onSwapNext,
}: Props) {
  const surfaceRef = useRef<HTMLDivElement>(null);
  const [roiMode, setRoiMode] = useState(false);
  const [drag, setDrag] = useState<DragBox>(null);
  const occupied = slot.hwnd != null;

  const normFromEvent = useCallback((e: React.PointerEvent) => {
    const el = surfaceRef.current;
    if (!el) return { x: 0, y: 0 };
    const r = el.getBoundingClientRect();
    return {
      x: (e.clientX - r.left) / Math.max(r.width, 1),
      y: (e.clientY - r.top) / Math.max(r.height, 1),
    };
  }, []);

  async function sendPointer(
    kind: "down" | "move" | "up" | "wheel",
    e: React.PointerEvent | React.WheelEvent,
    extra?: { delta_y?: number },
  ) {
    if (!occupied || roiMode) return;
    const el = surfaceRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const clientX = "clientX" in e ? e.clientX : 0;
    const clientY = "clientY" in e ? e.clientY : 0;
    const norm_x = (clientX - r.left) / Math.max(r.width, 1);
    const norm_y = (clientY - r.top) / Math.max(r.height, 1);
    let buttons = 0;
    if ("buttons" in e) {
      buttons = e.buttons;
      if (kind === "down" && buttons === 0 && "button" in e) {
        buttons = e.button === 2 ? 2 : e.button === 1 ? 4 : 1;
      }
      if (kind === "up" && "button" in e) {
        buttons = e.button === 2 ? 2 : e.button === 1 ? 4 : 1;
      }
    }
    await invoke("forward_pointer", {
      args: {
        slot: slot.slot,
        kind,
        norm_x,
        norm_y,
        buttons,
        delta_y: extra?.delta_y ?? 0,
      },
    });
  }

  async function onKey(kind: "down" | "up" | "char", e: React.KeyboardEvent) {
    if (!occupied || roiMode) return;
    e.preventDefault();
    const vk = e.keyCode || e.which;
    if (kind === "char" && e.key.length === 1) {
      await invoke("forward_key", {
        args: {
          slot: slot.slot,
          kind: "char",
          vk,
          scan: 0,
          text: e.key,
        },
      });
      return;
    }
    await invoke("forward_key", {
      args: {
        slot: slot.slot,
        kind,
        vk,
        scan: 0,
        text: null,
      },
    });
  }

  function onRoiPointerDown(e: React.PointerEvent) {
    if (!roiMode) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const p = normFromEvent(e);
    setDrag({ x0: p.x, y0: p.y, x1: p.x, y1: p.y });
  }

  function onRoiPointerMove(e: React.PointerEvent) {
    if (!roiMode || !drag) return;
    const p = normFromEvent(e);
    setDrag({ ...drag, x1: p.x, y1: p.y });
  }

  async function onRoiPointerUp() {
    if (!roiMode || !drag) {
      setDrag(null);
      return;
    }
    const img = surfaceRef.current?.querySelector("img");
    const nw = img?.naturalWidth || 0;
    const nh = img?.naturalHeight || 0;
    if (nw > 0 && nh > 0) {
      const left = Math.min(drag.x0, drag.x1);
      const top = Math.min(drag.y0, drag.y1);
      const right = Math.max(drag.x0, drag.x1);
      const bottom = Math.max(drag.y0, drag.y1);
      const w = Math.max(right - left, 0.02);
      const h = Math.max(bottom - top, 0.02);
      const localX = Math.round(left * nw);
      const localY = Math.round(top * nh);
      const localW = Math.round(w * nw);
      const localH = Math.round(h * nh);

      const base = slot.roi && !slot.roi.use_full ? slot.roi : null;
      await invoke("set_roi", {
        args: {
          slot: slot.slot,
          x: (base?.x ?? 0) + localX,
          y: (base?.y ?? 0) + localY,
          w: localW,
          h: localH,
          use_full: false,
        },
      });
    }
    setDrag(null);
    setRoiMode(false);
  }

  async function resetRoi() {
    await invoke("set_roi", {
      args: { slot: slot.slot, x: 0, y: 0, w: 0, h: 0, use_full: true },
    });
    setRoiMode(false);
  }

  return (
    <section className="slot">
      <header className="slot-bar">
        <div className="slot-title">
          <span className="slot-index">#{slot.slot + 1}</span>
          <span className="slot-name">{occupied ? slot.title : "空槽位"}</span>
        </div>
        <div className="slot-actions">
          {!occupied && (
            <button type="button" onClick={() => onRequestAttach(slot.slot)}>
              附着窗口
            </button>
          )}
          {occupied && (
            <>
              <button
                type="button"
                className={roiMode ? "active" : ""}
                onClick={() => setRoiMode((v) => !v)}
                title="拖拽框选截取区域"
              >
                {roiMode ? "框选中…" : "截取区域"}
              </button>
              <button type="button" onClick={resetRoi}>
                全窗
              </button>
              <button type="button" onClick={() => onSwapNext(slot.slot)} title="与下一槽交换">
                换位
              </button>
              <button type="button" onClick={() => onDetach(slot.slot)}>
                分离
              </button>
            </>
          )}
        </div>
      </header>

      <div
        ref={surfaceRef}
        className={`slot-surface ${occupied ? "live" : "empty"} ${roiMode ? "roi-mode" : ""}`}
        tabIndex={occupied ? 0 : -1}
        onPointerDown={(e) => {
          if (roiMode) {
            onRoiPointerDown(e);
            return;
          }
          if (!occupied) return;
          e.currentTarget.setPointerCapture(e.pointerId);
          e.currentTarget.focus();
          void sendPointer("down", e);
        }}
        onPointerMove={(e) => {
          if (roiMode) {
            onRoiPointerMove(e);
            return;
          }
          void sendPointer("move", e);
        }}
        onPointerUp={(e) => {
          if (roiMode) {
            void onRoiPointerUp();
            return;
          }
          void sendPointer("up", e);
        }}
        onWheel={(e) => {
          e.preventDefault();
          void sendPointer("wheel", e, { delta_y: Math.round(-e.deltaY) });
        }}
        onKeyDown={(e) => void onKey(e.key.length === 1 ? "char" : "down", e)}
        onKeyUp={(e) => {
          if (e.key.length !== 1) void onKey("up", e);
        }}
        onContextMenu={(e) => e.preventDefault()}
      >
        {frameUrl ? (
          <img src={frameUrl} alt={slot.title ?? `slot ${slot.slot}`} draggable={false} />
        ) : (
          <div className="slot-placeholder">
            {occupied ? "等待画面…" : "点击「附着窗口」选择目标"}
          </div>
        )}
        {drag && (
          <div
            className="roi-rect"
            style={{
              left: `${Math.min(drag.x0, drag.x1) * 100}%`,
              top: `${Math.min(drag.y0, drag.y1) * 100}%`,
              width: `${Math.abs(drag.x1 - drag.x0) * 100}%`,
              height: `${Math.abs(drag.y1 - drag.y0) * 100}%`,
            }}
          />
        )}
      </div>
    </section>
  );
}

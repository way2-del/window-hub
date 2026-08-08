import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { WindowInfo } from "../types";

type Props = {
  open: boolean;
  targetSlot: number | null;
  onClose: () => void;
  onAttached: () => void;
};

export default function WindowPicker({ open, targetSlot, onClose, onAttached }: Props) {
  const [windows, setWindows] = useState<WindowInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      const list = await invoke<WindowInfo[]>("list_open_windows");
      setWindows(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (open) void refresh();
  }, [open]);

  if (!open || targetSlot === null) return null;

  const q = filter.trim().toLowerCase();
  const filtered = windows.filter(
    (w) =>
      !q ||
      w.title.toLowerCase().includes(q) ||
      w.class_name.toLowerCase().includes(q) ||
      String(w.pid).includes(q),
  );

  async function attach(w: WindowInfo) {
    try {
      await invoke("attach_window", {
        args: {
          hwnd: w.hwnd,
          slot: targetSlot,
          title: w.title,
          class_name: w.class_name,
          pid: w.pid,
        },
      });
      onAttached();
      onClose();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <div className="picker-backdrop" onClick={onClose}>
      <div className="picker" onClick={(e) => e.stopPropagation()}>
        <header className="picker-header">
          <h2>附着到槽位 {targetSlot + 1}</h2>
          <button type="button" onClick={onClose}>
            关闭
          </button>
        </header>
        <div className="picker-toolbar">
          <input
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder="搜索标题 / 类名 / PID"
          />
          <button type="button" onClick={refresh} disabled={loading}>
            {loading ? "刷新中…" : "刷新"}
          </button>
        </div>
        {error && <p className="error">{error}</p>}
        <ul className="picker-list">
          {filtered.map((w) => (
            <li key={w.hwnd}>
              <button type="button" className="picker-item" onClick={() => attach(w)}>
                <span className="picker-title">{w.title}</span>
                <span className="picker-meta">
                  {w.class_name} · pid {w.pid} · hwnd {w.hwnd}
                </span>
              </button>
            </li>
          ))}
          {!loading && filtered.length === 0 && (
            <li className="picker-empty">没有匹配的窗口</li>
          )}
        </ul>
      </div>
    </div>
  );
}

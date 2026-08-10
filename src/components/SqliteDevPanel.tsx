import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type DbTableInfo = { name: string; rowCount: number };
type DbDevInfo = {
  path: string;
  sizeBytes: number;
  schemaVersion: number;
  tables: DbTableInfo[];
};
type DbDevRows = {
  columns: string[];
  rows: Record<string, unknown>[];
  total: number;
};

const TABLE_LABELS: Record<string, string> = {
  schema_meta: "迁移标记 schema_meta",
  prefs_material: "材质 prefs_material",
  prefs_tray: "托盘 prefs_tray",
  prefs_ambient: "沉浸采样 prefs_ambient",
  prefs_island: "灵动岛 prefs_island",
  prefs_shortcuts: "快捷区 prefs_shortcuts",
  prefs_dock: "底栏 Dock prefs_dock",
  script_launchers: "Companion 脚本",
  plugin_kv: "插件数据 plugin_kv（含天气凭证/缓存）",
};

const PKS: Record<string, string[]> = {
  schema_meta: ["key"],
  prefs_material: ["id"],
  prefs_tray: ["id"],
  prefs_ambient: ["id"],
  prefs_island: ["id"],
  prefs_shortcuts: ["id"],
  prefs_dock: ["id"],
  script_launchers: ["id"],
  plugin_kv: ["plugin_id", "key"],
};

function formatBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(2)} MB`;
}

function cellPreview(v: unknown): string {
  if (v == null) return "null";
  if (typeof v === "string") {
    return v.length > 80 ? `${v.slice(0, 80)}…` : v;
  }
  try {
    const s = JSON.stringify(v);
    return s.length > 80 ? `${s.slice(0, 80)}…` : s;
  } catch {
    return String(v);
  }
}

function emptyRow(columns: string[], table: string): Record<string, string> {
  const row: Record<string, string> = {};
  const now = String(Date.now());
  for (const c of columns) {
    if (c === "updated_at" || c === "created_at") row[c] = now;
    else if (c === "value_json" || c === "pins_json") row[c] = "{}";
    else if (c === "kind") row[c] = "file";
    else if (c === "id" && table === "staging_items") row[c] = `dev-${now}`;
    else row[c] = "";
  }
  return row;
}

function toPayload(draft: Record<string, string>, columns: string[]): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const c of columns) {
    const raw = draft[c] ?? "";
    if (c === "updated_at" || c === "created_at") {
      const n = Number(raw);
      out[c] = Number.isFinite(n) ? Math.trunc(n) : Date.now();
    } else {
      out[c] = raw;
    }
  }
  return out;
}

export default function SqliteDevPanel() {
  const [info, setInfo] = useState<DbDevInfo | null>(null);
  const [table, setTable] = useState("prefs_island");
  const [rows, setRows] = useState<DbDevRows | null>(null);
  const [msg, setMsg] = useState("");
  const [busy, setBusy] = useState(false);
  const [editDraft, setEditDraft] = useState<Record<string, string> | null>(null);
  const [editMode, setEditMode] = useState<"create" | "edit" | null>(null);

  const columns = rows?.columns ?? [];

  const refreshInfo = useCallback(async () => {
    const next = await invoke<DbDevInfo>("db_dev_info");
    setInfo(next);
  }, []);

  const refreshRows = useCallback(async (t = table) => {
    const next = await invoke<DbDevRows>("db_dev_list_rows", {
      table: t,
      limit: 200,
      offset: 0,
    });
    setRows(next);
  }, [table]);

  const refreshAll = useCallback(async () => {
    setBusy(true);
    setMsg("");
    try {
      await refreshInfo();
      await refreshRows();
    } catch (e) {
      setMsg(`加载失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }, [refreshInfo, refreshRows]);

  useEffect(() => {
    void refreshAll();
  }, [refreshAll]);

  useEffect(() => {
    void refreshRows(table);
    setEditDraft(null);
    setEditMode(null);
  }, [table, refreshRows]);

  const tableMeta = useMemo(
    () => info?.tables.find((t) => t.name === table),
    [info, table],
  );

  async function onBackup() {
    setBusy(true);
    setMsg("");
    try {
      const path = await invoke<string | null>("db_dev_backup");
      setMsg(path ? `已备份到：${path}` : "已取消备份");
      await refreshInfo();
    } catch (e) {
      setMsg(`备份失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function onRestore() {
    setBusy(true);
    setMsg("");
    try {
      const path = await invoke<string | null>("db_dev_pick_restore_file");
      if (!path) {
        setMsg("已取消恢复");
        return;
      }
      const ok = window.confirm(
        `将用备份覆盖当前数据库全部表数据：\n${path}\n\n此操作不可撤销（建议先备份）。继续？`,
      );
      if (!ok) {
        setMsg("已取消恢复");
        return;
      }
      await invoke("db_dev_restore", { path });
      setMsg("恢复成功。若界面仍显示旧数据，请重启应用。");
      await refreshAll();
    } catch (e) {
      setMsg(`恢复失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  function startCreate() {
    if (!columns.length) return;
    setEditMode("create");
    setEditDraft(emptyRow(columns, table));
  }

  function startEdit(row: Record<string, unknown>) {
    const draft: Record<string, string> = {};
    for (const c of columns) {
      const v = row[c];
      draft[c] = v == null ? "" : typeof v === "string" ? v : JSON.stringify(v);
    }
    setEditMode("edit");
    setEditDraft(draft);
  }

  async function saveEdit() {
    if (!editDraft || !editMode) return;
    setBusy(true);
    setMsg("");
    try {
      await invoke("db_dev_upsert_row", {
        table,
        row: toPayload(editDraft, columns),
      });
      setEditDraft(null);
      setEditMode(null);
      setMsg(editMode === "create" ? "已新增" : "已保存");
      await refreshAll();
    } catch (e) {
      setMsg(`保存失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function deleteRow(row: Record<string, unknown>) {
    const pks = PKS[table] ?? [];
    const keys: Record<string, unknown> = {};
    for (const pk of pks) keys[pk] = row[pk];
    const label = pks.map((k) => `${k}=${String(row[k] ?? "")}`).join(", ");
    if (!window.confirm(`删除行？\n${label}`)) return;
    setBusy(true);
    try {
      await invoke("db_dev_delete_row", { table, keys });
      setMsg("已删除");
      await refreshAll();
    } catch (e) {
      setMsg(`删除失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function clearTable() {
    if (!window.confirm(`清空整表 ${table}？此操作不可撤销。`)) return;
    setBusy(true);
    try {
      const n = await invoke<number>("db_dev_clear_table", { table });
      setMsg(`已清空 ${n} 行`);
      await refreshAll();
    } catch (e) {
      setMsg(`清空失败：${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <section className="settings-card">
        <h2>SQLite 本地库</h2>
        <p className="card-desc">
          开发者工具：浏览并修改 <code>window-hub.db</code>。误改可能导致设置/插件异常，操作前请先备份。
        </p>
        {info ? (
          <ul className="dev-db-meta">
            <li>
              <span>路径</span>
              <code title={info.path}>{info.path}</code>
            </li>
            <li>
              <span>大小</span>
              <strong>{formatBytes(info.sizeBytes)}</strong>
            </li>
            <li>
              <span>schema</span>
              <strong>v{info.schemaVersion}</strong>
            </li>
          </ul>
        ) : (
          <p className="card-desc">加载中…</p>
        )}
        <div className="dev-db-actions">
          <button type="button" className="settings-ghost-btn" disabled={busy} onClick={() => void refreshAll()}>
            刷新
          </button>
          <button type="button" className="settings-primary-btn" disabled={busy} onClick={() => void onBackup()}>
            备份数据库
          </button>
          <button type="button" className="settings-ghost-btn" disabled={busy} onClick={() => void onRestore()}>
            从备份恢复
          </button>
        </div>
        {msg ? <p className="dev-db-msg">{msg}</p> : null}
      </section>

      <section className="settings-card">
        <h2>表数据</h2>
        <div className="dev-db-table-tabs">
          {(info?.tables ?? Object.keys(TABLE_LABELS).map((name) => ({ name, rowCount: 0 }))).map(
            (t) => (
              <button
                key={t.name}
                type="button"
                className={`dev-db-tab${table === t.name ? " is-active" : ""}`}
                onClick={() => setTable(t.name)}
              >
                {TABLE_LABELS[t.name] ?? t.name}
                <em>{t.rowCount}</em>
              </button>
            ),
          )}
        </div>
        <div className="dev-db-actions">
          <button type="button" className="settings-ghost-btn" disabled={busy} onClick={startCreate}>
            新增行
          </button>
          <button type="button" className="settings-ghost-btn is-danger" disabled={busy} onClick={() => void clearTable()}>
            清空本表
          </button>
          <span className="dev-db-count">
            {tableMeta ? `${tableMeta.rowCount} 行` : rows ? `${rows.total} 行` : ""}
            {rows && rows.rows.length < rows.total ? `（显示前 ${rows.rows.length}）` : ""}
          </span>
        </div>

        {editDraft && editMode ? (
          <div className="dev-db-editor">
            <h3>{editMode === "create" ? "新增行" : "编辑行"}</h3>
            <div className="dev-db-fields">
              {columns.map((c) => {
                const isPk = (PKS[table] ?? []).includes(c);
                const isJson = c.endsWith("_json") || c === "value_json" || c === "pins_json";
                return (
                  <label key={c} className="dev-db-field">
                    <span>
                      {c}
                      {isPk ? " · PK" : ""}
                    </span>
                    {isJson ? (
                      <textarea
                        rows={5}
                        spellCheck={false}
                        value={editDraft[c] ?? ""}
                        disabled={editMode === "edit" && isPk}
                        onChange={(e) =>
                          setEditDraft((d) => (d ? { ...d, [c]: e.target.value } : d))
                        }
                      />
                    ) : (
                      <input
                        spellCheck={false}
                        value={editDraft[c] ?? ""}
                        disabled={editMode === "edit" && isPk}
                        onChange={(e) =>
                          setEditDraft((d) => (d ? { ...d, [c]: e.target.value } : d))
                        }
                      />
                    )}
                  </label>
                );
              })}
            </div>
            <div className="dev-db-actions">
              <button type="button" className="settings-primary-btn" disabled={busy} onClick={() => void saveEdit()}>
                保存
              </button>
              <button
                type="button"
                className="settings-ghost-btn"
                disabled={busy}
                onClick={() => {
                  setEditDraft(null);
                  setEditMode(null);
                }}
              >
                取消
              </button>
            </div>
          </div>
        ) : null}

        <div className="dev-db-table-wrap">
          <table className="dev-db-table">
            <thead>
              <tr>
                {columns.map((c) => (
                  <th key={c}>{c}</th>
                ))}
                <th className="dev-db-ops">操作</th>
              </tr>
            </thead>
            <tbody>
              {!rows?.rows.length ? (
                <tr>
                  <td colSpan={Math.max(1, columns.length + 1)} className="dev-db-empty">
                    暂无数据
                  </td>
                </tr>
              ) : (
                rows.rows.map((row, idx) => (
                  <tr key={idx}>
                    {columns.map((c) => (
                      <td key={c} title={cellPreview(row[c])}>
                        {cellPreview(row[c])}
                      </td>
                    ))}
                    <td className="dev-db-ops">
                      <button type="button" className="wg-text-btn" onClick={() => startEdit(row)}>
                        编辑
                      </button>
                      <button type="button" className="wg-text-btn is-danger" onClick={() => void deleteRow(row)}>
                        删除
                      </button>
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </section>
    </>
  );
}

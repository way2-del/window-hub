import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import PluginSettingsForm from "./components/PluginSettingsForm";
import ScenarioGateSettings from "./components/ScenarioGateSettings";
import { describeCapabilities } from "./plugins/capGate";
import { pluginHasScenario } from "./plugins/islandSlots";
import type { PluginCapability, PluginSettingField } from "./plugins/types";
import {
  MARKET_CATEGORIES,
  findMarketPlugin,
  isMarketPluginInstalled,
  marketPluginsInCategory,
  searchMarketPlugins,
  type MarketCategoryId,
  type MarketPlugin,
} from "./pluginMarketCatalog";

export type PluginsView =
  | "market"
  | "category"
  | "marketDetail"
  | "installed"
  | "installedDetail";

export type PluginListEntry = {
  id: string;
  name: string;
  version: string;
  enabled: boolean;
  official: boolean;
  dev: boolean;
  capabilities: PluginCapability[];
  settings?: PluginSettingField[];
  settingsIntro?: string;
};

type ScriptEnv = "python" | "node" | "powershell" | "cmd" | "exe" | "custom";

export type ScriptLauncherRow = {
  id: string;
  name: string;
  scriptPath: string;
  environment: ScriptEnv | string;
  envPath: string;
  args: string;
  pluginId: string;
  startWithHub: boolean;
  startOnBoot: boolean;
  enabled: boolean;
  running: boolean;
  pid?: number | null;
};

type LauncherDraft = {
  id: string;
  name: string;
  scriptPath: string;
  environment: ScriptEnv | string;
  envPath: string;
  args: string;
  pluginId: string;
  startWithHub: boolean;
  startOnBoot: boolean;
  enabled: boolean;
};

const SCRIPT_ENVS: { id: ScriptEnv; label: string }[] = [
  { id: "python", label: "Python" },
  { id: "node", label: "Node.js" },
  { id: "powershell", label: "PowerShell" },
  { id: "cmd", label: "CMD" },
  { id: "exe", label: "可执行文件" },
  { id: "custom", label: "自定义运行时" },
];

function BackBtn({
  label,
  onClick,
}: {
  label: string;
  onClick: () => void;
}) {
  return (
    <button type="button" className="pm-back" onClick={onClick}>
      <svg
        className="pm-back-icon"
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2.25"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <path d="M15 18l-6-6 6-6" />
      </svg>
      <span className="pm-back-label">{label}</span>
    </button>
  );
}

function PluginGlyph({
  letter,
  tint,
  large,
}: {
  letter: string;
  tint: string;
  large?: boolean;
}) {
  return (
    <span
      className={`pm-glyph${large ? " is-lg" : ""}`}
      style={{ background: tint }}
      aria-hidden
    >
      {letter}
    </span>
  );
}

function InstallIcon({ installed }: { installed: boolean }) {
  if (installed) {
    return (
      <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden>
        <path
          d="M20 6L9 17l-5-5"
          stroke="currentColor"
          strokeWidth="2.4"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    );
  }
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden>
      <path
        d="M12 3v12M12 15l-4-4M12 15l4-4M5 21h14"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

type Props = {
  pluginEntries: PluginListEntry[];
  pluginMsg: string;
  pluginBusy: boolean;
  focusPluginId: string | null;
  onClearFocusPlugin: () => void;
  onBeginInstallFromPath: (path: string) => Promise<void>;
  onBeginInstallExample: (exampleId: string) => Promise<void>;
  onToggleInstalled: (id: string, enabled: boolean) => void;
  onDeleteInstalled: (id: string, name: string) => void;
  launchers: ScriptLauncherRow[];
  launcherDraft: LauncherDraft;
  setLauncherDraft: Dispatch<SetStateAction<LauncherDraft>>;
  launcherMsg: string;
  launcherBusy: boolean;
  onPickLauncherScript: () => void;
  onSaveLauncher: () => void;
  onEditLauncher: (row: ScriptLauncherRow) => void;
  onRunLauncher: (id: string, start: boolean) => void;
  onRemoveLauncher: (id: string, name: string) => void;
  onResetLauncherDraft: () => void;
};

export default function PluginsMarketPanel({
  pluginEntries,
  pluginMsg,
  pluginBusy,
  focusPluginId,
  onClearFocusPlugin,
  onBeginInstallFromPath,
  onBeginInstallExample,
  onToggleInstalled,
  onDeleteInstalled,
  launchers,
  launcherDraft,
  setLauncherDraft,
  launcherMsg,
  launcherBusy,
  onPickLauncherScript,
  onSaveLauncher,
  onEditLauncher,
  onRunLauncher,
  onRemoveLauncher,
  onResetLauncherDraft,
}: Props) {
  const [view, setView] = useState<PluginsView>("market");
  const [categoryId, setCategoryId] = useState<MarketCategoryId | null>(null);
  const [marketDetailId, setMarketDetailId] = useState<string | null>(null);
  const [installedDetailId, setInstalledDetailId] = useState<string | null>(null);
  const [marketQuery, setMarketQuery] = useState("");
  const [installedQuery, setInstalledQuery] = useState("");
  const [installedFilter, setInstalledFilter] = useState<"all" | "enabled" | "disabled">(
    "all",
  );
  const [moreOpen, setMoreOpen] = useState(false);
  const moreRef = useRef<HTMLDivElement | null>(null);

  const installedIds = useMemo(
    () => pluginEntries.map((p) => p.id),
    [pluginEntries],
  );

  useEffect(() => {
    if (!focusPluginId) return;
    setView("installedDetail");
    setInstalledDetailId(focusPluginId);
    setCategoryId(null);
    setMarketDetailId(null);
    const id = focusPluginId;
    const t = window.setTimeout(() => {
      const el = document.querySelector(
        `[data-plugin-id="${CSS.escape(id)}"]`,
      ) as HTMLElement | null;
      if (el) {
        el.scrollIntoView({ block: "nearest", behavior: "smooth" });
        el.classList.add("is-focus-flash");
        window.setTimeout(() => el.classList.remove("is-focus-flash"), 1600);
      }
      onClearFocusPlugin();
    }, 80);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- hand off once per focus id
  }, [focusPluginId]);

  useEffect(() => {
    if (!moreOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (!moreRef.current?.contains(e.target as Node)) setMoreOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [moreOpen]);

  const searchHits = useMemo(
    () => searchMarketPlugins(marketQuery),
    [marketQuery],
  );

  const category = categoryId
    ? MARKET_CATEGORIES.find((c) => c.id === categoryId) ?? null
    : null;

  const categoryPlugins = categoryId
    ? marketPluginsInCategory(categoryId)
    : [];

  const marketPlugin = marketDetailId
    ? findMarketPlugin(marketDetailId) ?? null
    : null;

  const installedDetail = installedDetailId
    ? pluginEntries.find((p) => p.id === installedDetailId) ?? null
    : null;

  useEffect(() => {
    if (view === "installedDetail" && installedDetailId && !installedDetail) {
      setView("installed");
      setInstalledDetailId(null);
    }
  }, [view, installedDetailId, installedDetail]);

  const filteredInstalled = useMemo(() => {
    const q = installedQuery.trim().toLowerCase();
    return pluginEntries.filter((p) => {
      if (installedFilter === "enabled" && !p.enabled) return false;
      if (installedFilter === "disabled" && p.enabled) return false;
      if (!q) return true;
      return (
        p.name.toLowerCase().includes(q) ||
        p.id.toLowerCase().includes(q) ||
        p.version.toLowerCase().includes(q)
      );
    });
  }, [pluginEntries, installedQuery, installedFilter]);

  function goMarket() {
    setView("market");
    setCategoryId(null);
    setMarketDetailId(null);
    setInstalledDetailId(null);
    setMoreOpen(false);
  }

  function openCategory(id: MarketCategoryId) {
    setCategoryId(id);
    setMarketDetailId(null);
    setView("category");
  }

  function openMarketDetail(p: MarketPlugin) {
    setMarketDetailId(p.exampleId);
    setView("marketDetail");
  }

  function openInstalled() {
    setView("installed");
    setInstalledDetailId(null);
    setMoreOpen(false);
  }

  function openInstalledDetail(id: string) {
    setInstalledDetailId(id);
    setView("installedDetail");
  }

  async function pickWhpx() {
    setMoreOpen(false);
    try {
      const path = await invoke<string | null>("pick_whpx_file");
      if (!path) return;
      await onBeginInstallFromPath(path);
    } catch (err) {
      /* parent surfaces via pluginMsg after beginInstall */
      console.error(err);
    }
  }

  async function pickDevDir() {
    setMoreOpen(false);
    try {
      const path = await invoke<string | null>("pick_plugin_directory");
      if (!path) return;
      await onBeginInstallFromPath(path);
    } catch (err) {
      console.error(err);
    }
  }

  function renderMarketTopBar(extra?: ReactNode) {
    return (
      <div className="pm-topbar">
        <div className="pm-search-wrap">
          <svg
            className="pm-search-icon"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            aria-hidden
          >
            <circle cx="11" cy="11" r="7" />
            <path d="M20 20l-3.5-3.5" />
          </svg>
          <input
            className="pm-search"
            type="search"
            placeholder="搜索插件市场…"
            value={marketQuery}
            onChange={(e) => setMarketQuery(e.target.value)}
          />
        </div>
        {extra}
        <button type="button" className="pm-installed-entry" onClick={openInstalled}>
          已安装
          {pluginEntries.length > 0 ? (
            <span className="pm-installed-count">{pluginEntries.length}</span>
          ) : null}
        </button>
      </div>
    );
  }

  function renderPluginCard(p: MarketPlugin) {
    const installed = isMarketPluginInstalled(p.pluginId, installedIds);
    return (
      <button
        key={p.exampleId}
        type="button"
        className="pm-plugin-card"
        onClick={() => openMarketDetail(p)}
      >
        <PluginGlyph letter={p.letter} tint={p.tint} />
        <span className="pm-plugin-card-meta">
          <span className="pm-plugin-card-name">{p.name}</span>
          <span className="pm-plugin-card-desc">{p.summary}</span>
        </span>
        <span
          className={`pm-plugin-card-action${installed ? " is-installed" : ""}`}
          title={installed ? "已安装" : "可安装"}
        >
          <InstallIcon installed={installed} />
        </span>
      </button>
    );
  }

  /* ── Market home ─────────────────────────────────────────── */
  if (view === "market") {
    const showSearch = marketQuery.trim().length > 0;
    return (
      <section className="settings-card settings-card-grow pm-root">
        {renderMarketTopBar()}
        {pluginMsg ? <p className="plugin-msg">{pluginMsg}</p> : null}
        {showSearch ? (
          <>
            <h3 className="pm-section-title">搜索结果</h3>
            {searchHits.length === 0 ? (
              <p className="pm-empty">未找到匹配的插件</p>
            ) : (
              <div className="pm-plugin-grid">{searchHits.map(renderPluginCard)}</div>
            )}
          </>
        ) : (
          <>
            <div className="pm-banner">
              <div className="pm-banner-logo" aria-hidden>
                WH
              </div>
              <div className="pm-banner-text">
                <strong>Window Hub</strong>
                <span>内置示例插件 · 也可安装 .whpx / 开发目录</span>
              </div>
            </div>
<h3 className="pm-section-title">插件分类</h3>
            <div className="pm-category-grid">
              {MARKET_CATEGORIES.map((c) => {
                const count = marketPluginsInCategory(c.id).length;
                return (
                  <button
                    key={c.id}
                    type="button"
                    className="pm-category-card"
                    onClick={() => openCategory(c.id)}
                  >
                    <PluginGlyph letter={c.name.charAt(0)} tint={c.tint} />
                    <span className="pm-category-meta">
                      <span className="pm-category-name">{c.name}</span>
                      <span className="pm-category-desc">{c.description}</span>
                    </span>
                    <span className="pm-category-trail">
                      <span className="pm-category-count">{count}</span>
                      <span className="pm-chevron" aria-hidden>
                        ›
                      </span>
                    </span>
                  </button>
                );
              })}
            </div>
          </>
        )}
      </section>
    );
  }

  /* ── Category list ───────────────────────────────────────── */
  if (view === "category") {
    if (!category) {
      return (
        <section className="settings-card settings-card-grow pm-root">
          {renderMarketTopBar()}
          <BackBtn label="插件市场" onClick={goMarket} />
          <p className="pm-empty">分类不存在</p>
        </section>
      );
    }
    return (
      <section className="settings-card settings-card-grow pm-root">
        {renderMarketTopBar()}
        {pluginMsg ? <p className="plugin-msg">{pluginMsg}</p> : null}
        <BackBtn label={category.name} onClick={goMarket} />
        <div className="pm-cat-banner">
          <PluginGlyph letter={category.name.charAt(0)} tint={category.tint} large />
          <div className="pm-cat-banner-text">
            <strong>{category.blurb}</strong>
          </div>
        </div>
        {categoryPlugins.length > 0 ? (
          <div className="pm-plugin-grid">{categoryPlugins.map(renderPluginCard)}</div>
        ) : (
          <p className="pm-empty">该分类暂无插件</p>
        )}
      </section>
    );
  }

  /* ── Market detail ───────────────────────────────────────── */
  if (view === "marketDetail") {
    if (!marketPlugin) {
      return (
        <section className="settings-card settings-card-grow pm-root">
          {renderMarketTopBar()}
          <BackBtn label="插件详情" onClick={goMarket} />
          <p className="pm-empty">插件不存在</p>
        </section>
      );
    }
    const installed = isMarketPluginInstalled(marketPlugin.pluginId, installedIds);
    return (
      <section className="settings-card settings-card-grow pm-root">
        {renderMarketTopBar()}
        {pluginMsg ? <p className="plugin-msg">{pluginMsg}</p> : null}
        <BackBtn
          label="插件详情"
          onClick={() => {
            if (categoryId) setView("category");
            else goMarket();
            setMarketDetailId(null);
          }}
        />
        <div className="pm-detail-hero">
          <PluginGlyph letter={marketPlugin.letter} tint={marketPlugin.tint} large />
          <div className="pm-detail-hero-meta">
            <h2 className="pm-detail-name">{marketPlugin.name}</h2>
            <p className="pm-detail-desc">{marketPlugin.description}</p>
          </div>
          {installed ? (
            <button
              type="button"
              className="pm-detail-install is-done"
              onClick={() => {
                const hit = pluginEntries.find((e) =>
                  isMarketPluginInstalled(marketPlugin.pluginId, [e.id]),
                );
                if (hit) openInstalledDetail(hit.id);
                else openInstalled();
              }}
            >
              已安装
            </button>
          ) : (
            <button
              type="button"
              className="pm-detail-install"
              disabled={pluginBusy}
              title="安装示例"
              onClick={() => void onBeginInstallExample(marketPlugin.exampleId)}
            >
              <InstallIcon installed={false} />
            </button>
          )}
        </div>
        <div className="pm-metrics">
          <div className="pm-metric">
            <span className="pm-metric-label">来源</span>
            <span className="pm-metric-value">官方示例</span>
          </div>
          <div className="pm-metric">
            <span className="pm-metric-label">版本</span>
            <span className="pm-metric-value">{marketPlugin.version}</span>
          </div>
          <div className="pm-metric">
            <span className="pm-metric-label">分类</span>
            <span className="pm-metric-value">
              {MARKET_CATEGORIES.find((c) => c.id === marketPlugin.categoryId)?.name ??
                "—"}
            </span>
          </div>
          <div className="pm-metric">
            <span className="pm-metric-label">状态</span>
            <span className="pm-metric-value">{installed ? "已安装" : "未安装"}</span>
          </div>
        </div>
        <div className="pm-tabs">
          <span className="pm-tab is-active">详情</span>
        </div>
        <div className="pm-detail-body">
          <p>{marketPlugin.description}</p>
          <p className="pm-detail-muted">
            安装后可在「已安装」中启用、配置与删除。也可通过「更多」安装自定义 `.whpx`
            或开发目录。
          </p>
        </div>
      </section>
    );
  }

  /* ── Installed detail ────────────────────────────────────── */
  if (view === "installedDetail" && installedDetail) {
    return (
      <section className="settings-card settings-card-grow pm-root">
        <BackBtn
          label="已安装插件"
          onClick={() => {
            setView("installed");
            setInstalledDetailId(null);
          }}
        />
        {pluginMsg ? <p className="plugin-msg">{pluginMsg}</p> : null}
        <div
          className="pm-detail-hero"
          data-plugin-id={installedDetail.id}
        >
          <PluginGlyph
            letter={installedDetail.name.charAt(0)}
            tint={
              findMarketPlugin(installedDetail.id)?.tint ??
              (installedDetail.official ? "#30d158" : "#8e8e93")
            }
            large
          />
          <div className="pm-detail-hero-meta">
            <h2 className="pm-detail-name">
              {installedDetail.name}
              {installedDetail.dev ? " (dev)" : ""}
            </h2>
            <p className="pm-detail-desc">
              {installedDetail.id} · v{installedDetail.version}
              {installedDetail.official ? " · 官方" : ""}
            </p>
          </div>
          <button
            type="button"
            className={`pref-switch${installedDetail.enabled ? " is-on" : ""}`}
            role="switch"
            aria-checked={installedDetail.enabled}
            onClick={() =>
              onToggleInstalled(installedDetail.id, installedDetail.enabled)
            }
          >
            <span className="pref-switch-knob" />
          </button>
        </div>
        <div className="pm-metrics">
          <div className="pm-metric">
            <span className="pm-metric-label">版本</span>
            <span className="pm-metric-value">{installedDetail.version}</span>
          </div>
          <div className="pm-metric">
            <span className="pm-metric-label">状态</span>
            <span className="pm-metric-value">
              {installedDetail.enabled ? "已启用" : "已禁用"}
            </span>
          </div>
          <div className="pm-metric">
            <span className="pm-metric-label">类型</span>
            <span className="pm-metric-value">
              {installedDetail.dev ? "开发目录" : installedDetail.official ? "官方" : "本地"}
            </span>
          </div>
          <div className="pm-metric">
            <span className="pm-metric-label">能力</span>
            <span className="pm-metric-value">
              {installedDetail.capabilities.length || 0}
            </span>
          </div>
        </div>
        <div className="pm-tabs">
          <span className="pm-tab is-active">详情</span>
        </div>
        <div className="pm-detail-body">
          {installedDetail.settingsIntro ? (
            <p>{installedDetail.settingsIntro}</p>
          ) : null}
          {installedDetail.capabilities.length ? (
            <p className="pm-detail-muted">
              {describeCapabilities(installedDetail.capabilities).join(" · ")}
            </p>
          ) : null}
          {installedDetail.enabled &&
          installedDetail.settings &&
          installedDetail.settings.length > 0 ? (
            <PluginSettingsForm
              pluginId={installedDetail.id}
              fields={installedDetail.settings}
              description={installedDetail.settingsIntro}
            />
          ) : null}
          {pluginHasScenario(installedDetail.id) ? (
            <ScenarioGateSettings
              pluginId={installedDetail.id}
              enabled={installedDetail.enabled}
            />
          ) : null}
          <div className="pm-detail-actions">
            <button
              type="button"
              className="wg-text-btn is-danger"
              onClick={() =>
                onDeleteInstalled(installedDetail.id, installedDetail.name)
              }
            >
              删除插件
            </button>
          </div>
        </div>
      </section>
    );
  }

  /* ── Installed list (+ launchers) ────────────────────────── */
  return (
    <section className="settings-card settings-card-grow pm-root">
      <BackBtn label="插件市场" onClick={goMarket} />
      <div className="pm-installed-toolbar">
        <div className="pm-search-wrap is-grow">
          <svg
            className="pm-search-icon"
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            aria-hidden
          >
            <circle cx="11" cy="11" r="7" />
            <path d="M20 20l-3.5-3.5" />
          </svg>
          <input
            className="pm-search"
            type="search"
            placeholder="搜索已安装插件…"
            value={installedQuery}
            onChange={(e) => setInstalledQuery(e.target.value)}
          />
        </div>
        <div className="pm-more" ref={moreRef}>
          <button
            type="button"
            className="pm-more-btn"
            aria-expanded={moreOpen}
            onClick={() => setMoreOpen((v) => !v)}
          >
            更多
            <span aria-hidden>▾</span>
          </button>
          {moreOpen ? (
            <div className="pm-more-menu" role="menu">
              <button
                type="button"
                role="menuitem"
                disabled={pluginBusy}
                onClick={() => void pickWhpx()}
              >
                安装 .whpx
              </button>
              <button
                type="button"
                role="menuitem"
                disabled={pluginBusy}
                onClick={() => void pickDevDir()}
              >
                添加开发目录
              </button>
            </div>
          ) : null}
        </div>
      </div>

      <div className="pm-filter-tabs">
        <button
          type="button"
          className={`pm-filter-tab${installedFilter === "all" ? " is-active" : ""}`}
          onClick={() => setInstalledFilter("all")}
        >
          全部 {pluginEntries.length}
        </button>
        <button
          type="button"
          className={`pm-filter-tab${installedFilter === "enabled" ? " is-active" : ""}`}
          onClick={() => setInstalledFilter("enabled")}
        >
          已启用 {pluginEntries.filter((p) => p.enabled).length}
        </button>
        <button
          type="button"
          className={`pm-filter-tab${installedFilter === "disabled" ? " is-active" : ""}`}
          onClick={() => setInstalledFilter("disabled")}
        >
          已禁用 {pluginEntries.filter((p) => !p.enabled).length}
        </button>
      </div>

      {pluginMsg ? <p className="plugin-msg">{pluginMsg}</p> : null}

      <div className="pm-installed-list">
        {filteredInstalled.length === 0 ? (
          <p className="pm-empty">暂无已安装插件</p>
        ) : (
          filteredInstalled.map((entry) => (
            <div
              key={entry.id}
              data-plugin-id={entry.id}
              className={`pm-installed-row${entry.enabled ? "" : " is-disabled"}${
                entry.official ? " is-official" : ""
              }`}
            >
              <button
                type="button"
                className="pm-installed-main"
                onClick={() => openInstalledDetail(entry.id)}
              >
                <PluginGlyph
                  letter={entry.name.charAt(0)}
                  tint={
                    findMarketPlugin(entry.id)?.tint ??
                    (entry.official ? "#30d158" : "#8e8e93")
                  }
                />
                <span className="pm-installed-meta">
                  <span className="pm-installed-name">
                    {entry.name}
                    {entry.dev ? " (dev)" : ""}
                    <span className="pm-installed-ver">v{entry.version}</span>
                  </span>
                  <span className="pm-installed-sub">
                    {entry.id}
                    {entry.official ? " · 官方" : ""}
                  </span>
                </span>
                <span className="pm-chevron" aria-hidden>
                  ›
                </span>
              </button>
              <div className="pm-installed-actions" onClick={(e) => e.stopPropagation()}>
                <button
                  type="button"
                  className={`pref-switch${entry.enabled ? " is-on" : ""}`}
                  role="switch"
                  aria-checked={entry.enabled}
                  aria-label={`${entry.enabled ? "禁用" : "启用"} ${entry.name}`}
                  onClick={() => onToggleInstalled(entry.id, entry.enabled)}
                >
                  <span className="pref-switch-knob" />
                </button>
                <button
                  type="button"
                  className="wg-text-btn is-danger"
                  onClick={() => onDeleteInstalled(entry.id, entry.name)}
                >
                  删除
                </button>
              </div>
            </div>
          ))
        )}
      </div>

      <h3 className="plugin-subhead">脚本启动器</h3>
      <p className="settings-lead">
        登记本机 Companion 脚本（独立进程，不注入 Window Hub）。可设置路径、运行环境、关联插件，以及随
        Hub / 开机启动。
      </p>
      <div className="launcher-form">
        <label className="launcher-field">
          <span>名称</span>
          <input
            type="text"
            value={launcherDraft.name}
            placeholder="显示名称"
            onChange={(e) =>
              setLauncherDraft((d) => ({ ...d, name: e.target.value }))
            }
          />
        </label>
        <label className="launcher-field is-wide">
          <span>脚本路径</span>
          <div className="launcher-path-row">
            <input
              type="text"
              value={launcherDraft.scriptPath}
              placeholder="选择 .py / .js / .ps1 / .exe …"
              onChange={(e) =>
                setLauncherDraft((d) => ({ ...d, scriptPath: e.target.value }))
              }
            />
            <button
              type="button"
              className="settings-secondary-btn"
              disabled={launcherBusy}
              onClick={onPickLauncherScript}
            >
              浏览
            </button>
          </div>
        </label>
        <label className="launcher-field">
          <span>运行环境</span>
          <select
            value={launcherDraft.environment}
            onChange={(e) =>
              setLauncherDraft((d) => ({
                ...d,
                environment: e.target.value as ScriptEnv,
              }))
            }
          >
            {SCRIPT_ENVS.map((env) => (
              <option key={env.id} value={env.id}>
                {env.label}
              </option>
            ))}
          </select>
        </label>
        <label className="launcher-field">
          <span>运行时路径（可选）</span>
          <input
            type="text"
            value={launcherDraft.envPath}
            placeholder={
              launcherDraft.environment === "custom"
                ? "必填：解释器/运行时绝对路径"
                : "留空则用 PATH 中的 python / node …"
            }
            onChange={(e) =>
              setLauncherDraft((d) => ({ ...d, envPath: e.target.value }))
            }
          />
        </label>
        <label className="launcher-field">
          <span>关联插件</span>
          <select
            value={launcherDraft.pluginId}
            onChange={(e) =>
              setLauncherDraft((d) => ({ ...d, pluginId: e.target.value }))
            }
          >
            <option value="">不关联</option>
            {pluginEntries.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
                {p.dev ? " (dev)" : ""}
              </option>
            ))}
          </select>
        </label>
        <label className="launcher-field">
          <span>额外参数</span>
          <input
            type="text"
            value={launcherDraft.args}
            placeholder="可选 CLI 参数"
            onChange={(e) =>
              setLauncherDraft((d) => ({ ...d, args: e.target.value }))
            }
          />
        </label>
        <div className="launcher-checks">
          <label className="launcher-check">
            <input
              type="checkbox"
              checked={launcherDraft.startWithHub}
              onChange={(e) =>
                setLauncherDraft((d) => ({
                  ...d,
                  startWithHub: e.target.checked,
                }))
              }
            />
            随 Window Hub 启动
          </label>
          <label className="launcher-check">
            <input
              type="checkbox"
              checked={launcherDraft.startOnBoot}
              onChange={(e) =>
                setLauncherDraft((d) => ({
                  ...d,
                  startOnBoot: e.target.checked,
                }))
              }
            />
            开机自启（用户 Startup）
          </label>
          <label className="launcher-check">
            <input
              type="checkbox"
              checked={launcherDraft.enabled}
              onChange={(e) =>
                setLauncherDraft((d) => ({ ...d, enabled: e.target.checked }))
              }
            />
            启用
          </label>
        </div>
        <div className="plugin-actions">
          <button
            type="button"
            className="settings-primary-btn"
            disabled={launcherBusy}
            onClick={onSaveLauncher}
          >
            {launcherDraft.id ? "保存修改" : "添加启动器"}
          </button>
          {launcherDraft.id ? (
            <button
              type="button"
              className="settings-secondary-btn"
              disabled={launcherBusy}
              onClick={onResetLauncherDraft}
            >
              取消编辑
            </button>
          ) : null}
        </div>
      </div>
      {launcherMsg ? <p className="plugin-msg">{launcherMsg}</p> : null}
      <div className="plugin-list">
        {launchers.length === 0 ? (
          <p className="settings-lead">暂无脚本启动器</p>
        ) : (
          launchers.map((row) => (
            <div
              key={row.id}
              className={`plugin-row${row.enabled ? "" : " is-disabled"}`}
            >
              <div className="plugin-meta">
                <strong>
                  {row.name}
                  {row.running ? " · 运行中" : ""}
                </strong>
                <span>
                  {row.environment}
                  {row.pluginId ? ` · 关联 ${row.pluginId}` : " · 未关联插件"}
                  {row.startWithHub ? " · 随 Hub" : ""}
                  {row.startOnBoot ? " · 开机" : ""}
                </span>
                <span className="launcher-path-preview" title={row.scriptPath}>
                  {row.scriptPath}
                </span>
              </div>
              <div className="plugin-row-actions">
                <button
                  type="button"
                  className="settings-secondary-btn"
                  disabled={launcherBusy}
                  onClick={() => onEditLauncher(row)}
                >
                  编辑
                </button>
                {row.running ? (
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={launcherBusy}
                    onClick={() => onRunLauncher(row.id, false)}
                  >
                    停止
                  </button>
                ) : (
                  <button
                    type="button"
                    className="settings-secondary-btn"
                    disabled={launcherBusy || !row.enabled}
                    onClick={() => onRunLauncher(row.id, true)}
                  >
                    启动
                  </button>
                )}
                <button
                  type="button"
                  className="wg-text-btn is-danger"
                  disabled={launcherBusy}
                  onClick={() => onRemoveLauncher(row.id, row.name)}
                >
                  删除
                </button>
              </div>
            </div>
          ))
        )}
      </div>
    </section>
  );
}

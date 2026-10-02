import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type WheelEvent as ReactWheelEvent,
} from "react";
import IslandPanelHost from "../../components/IslandPanelHost";
import { listDashboardPanelProviders } from "../../plugins/panelPullMode";
import { isPluginSurfaceEnabled } from "../../plugins/surfacePrefs";
import { pluginRegistry } from "../../plugins/registry";
import { buildDemoCalendar } from "./demoData";
import {
  FILE_SEARCH_PLUGIN_ID,
  NOW_PLAYING_PLUGIN_ID,
  RIGHT_CAROUSEL_MAX,
  TRANSFER_STATION_PLUGIN_ID,
  WEATHER_PLUGIN_ID,
} from "./geometry";
import type { IslandHomeTab } from "./types";
import "./IslandHomeDashboard.css";

type Props = {
  active: boolean;
  tab: IslandHomeTab;
  /** 情景/会话强制占用左卡（dashboard pullMode） */
  forcedLeftPluginId?: string | null;
  onPanelClose?: () => void;
  /** Alt+Space 岛栏回车 → 转发给搜索页签 iframe */
  searchSubmit?: { nonce: number; query: string; action?: string } | null;
};

function baseId(id: string): string {
  return id.replace(/__dev$/, "");
}

function IconCalCheck() {
  return (
    <svg viewBox="0 0 64 64" width="40" height="40" aria-hidden>
      <rect
        x="10"
        y="14"
        width="36"
        height="36"
        rx="6"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
        opacity="0.55"
      />
      <path
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
        d="M18 10v8M38 10v8M10 24h36"
        opacity="0.55"
      />
      <circle cx="42" cy="42" r="14" fill="#1d4ed8" />
      <path
        fill="none"
        stroke="#fff"
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
        d="M35 42.5 40 47.5 50 36.5"
      />
    </svg>
  );
}

const DEFAULT_LEFT_FRAC = 0.52;
/** 左卡拉过此占比 → 满宽（藏右卡） */
const LEFT_EXPAND_SNAP = 0.86;
/** 右卡拉过此占比（即 leftFrac 低于此）→ 满宽（藏左卡） */
const RIGHT_EXPAND_SNAP = 0.14;
const LEFT_COLLAPSE_SNAP = 0.78;
const RIGHT_COLLAPSE_SNAP = 0.22;

export default function IslandHomeDashboard({
  active,
  tab,
  forcedLeftPluginId,
  onPanelClose,
  searchSubmit,
}: Props) {
  const [epoch, setEpoch] = useState(0);
  const [rightId, setRightId] = useState<string | null>(null);
  /** 左卡宽度占比：双向跟手；≥ LEFT_EXPAND 左满宽，≤ RIGHT_EXPAND 右满宽 */
  const [leftFrac, setLeftFrac] = useState(DEFAULT_LEFT_FRAC);
  const [resizing, setResizing] = useState(false);

  const bodyRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{
    pointerId: number;
    startX: number;
    startFrac: number;
    width: number;
  } | null>(null);
  const handleGapPx = 16;
  const swipeRef = useRef<{ pointerId: number; startY: number } | null>(null);
  const leftFracRef = useRef(leftFrac);
  leftFracRef.current = leftFrac;

  useEffect(() => pluginRegistry.subscribe(() => setEpoch((n) => n + 1)), []);

  const providers = useMemo(
    () =>
      listDashboardPanelProviders(pluginRegistry.listPanelManifests()).filter((p) =>
        isPluginSurfaceEnabled(p.id, "island.panel", p.manifest),
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [epoch],
  );

  const calendar = useMemo(() => buildDemoCalendar(), []);

  /** 左卡：情景/常驻强制 / 优先正在播放 / 否则第一个 dashboard 面板 */
  const leftPluginId = useMemo(() => {
    const matchForced = (id: string) => {
      if (providers.some((p) => p.id === id)) return id;
      const base = baseId(id);
      return providers.find((p) => baseId(p.id) === base)?.id ?? null;
    };
    if (forcedLeftPluginId) {
      const hit = matchForced(forcedLeftPluginId);
      if (hit) return hit;
      if (pluginRegistry.get(forcedLeftPluginId)?.enabled) return forcedLeftPluginId;
    }
    return (
      providers.find((p) => baseId(p.id) === NOW_PLAYING_PLUGIN_ID)?.id ??
      providers[0]?.id ??
      null
    );
  }, [forcedLeftPluginId, providers]);

  const leftIsNowPlaying =
    Boolean(leftPluginId) && baseId(leftPluginId!) === NOW_PLAYING_PLUGIN_ID;
  const leftIsWeather =
    Boolean(leftPluginId) && baseId(leftPluginId!) === WEATHER_PLUGIN_ID;

  /** 右卡轮播：排除左卡同一插件 + 正在播放（歌词走左卡扩展）；最多 5 个 */
  const rightProviders = useMemo(() => {
    const leftBase = leftPluginId ? baseId(leftPluginId) : null;
    return providers
      .filter((p) => {
        const b = baseId(p.id);
        if (b === NOW_PLAYING_PLUGIN_ID) return false;
        if (leftBase && b === leftBase) return false;
        return true;
      })
      .slice(0, RIGHT_CAROUSEL_MAX);
  }, [providers, leftPluginId]);

  const rightPluginId = useMemo(() => {
    if (rightId && rightProviders.some((p) => p.id === rightId)) return rightId;
    return rightProviders[0]?.id ?? null;
  }, [rightId, rightProviders]);

  const rightIndex = useMemo(() => {
    if (!rightPluginId) return -1;
    return rightProviders.findIndex((p) => p.id === rightPluginId);
  }, [rightPluginId, rightProviders]);

  useEffect(() => {
    if (!rightPluginId) {
      if (rightId) setRightId(null);
      return;
    }
    if (!rightId || !rightProviders.some((p) => p.id === rightId)) {
      setRightId(rightPluginId);
    }
  }, [rightPluginId, rightId, rightProviders]);

  const leftExpanded = leftFrac >= LEFT_EXPAND_SNAP;
  const rightExpanded = leftFrac <= RIGHT_EXPAND_SNAP;
  const expandCompanion = leftIsNowPlaying
    ? "lyrics.html"
    : leftIsWeather
      ? "forecast.html"
      : null;
  const showExpandSplit = leftExpanded && Boolean(expandCompanion);

  const selectRightByOffset = useCallback(
    (delta: number) => {
      if (rightProviders.length <= 1) return;
      const cur = Math.max(0, rightIndex);
      const next =
        (cur + delta + rightProviders.length) % rightProviders.length;
      setRightId(rightProviders[next]!.id);
    },
    [rightIndex, rightProviders],
  );

  const dotsWheelLock = useRef(0);
  const onDotsWheel = useCallback(
    (ev: ReactWheelEvent<HTMLDivElement>) => {
      if (rightProviders.length <= 1) return;
      ev.preventDefault();
      ev.stopPropagation();
      const now = performance.now();
      if (now - dotsWheelLock.current < 140) return;
      const dy = ev.deltaY;
      if (Math.abs(dy) < 2) return;
      dotsWheelLock.current = now;
      // 滚轮向下 → 下一个；向上 → 上一个
      selectRightByOffset(dy > 0 ? 1 : -1);
    },
    [rightProviders.length, selectRightByOffset],
  );

  const onResizePointerDown = (ev: ReactPointerEvent<HTMLButtonElement>) => {
    const body = bodyRef.current;
    if (!body) return;
    ev.preventDefault();
    ev.stopPropagation();
    ev.currentTarget.setPointerCapture(ev.pointerId);
    const width = Math.max(1, body.getBoundingClientRect().width);
    dragRef.current = {
      pointerId: ev.pointerId,
      startX: ev.clientX,
      startFrac: leftFracRef.current,
      width,
    };
    setResizing(true);
  };

  const onResizePointerMove = (ev: ReactPointerEvent<HTMLButtonElement>) => {
    const d = dragRef.current;
    if (!d || d.pointerId !== ev.pointerId) return;
    const dx = ev.clientX - d.startX;
    const next = Math.min(1, Math.max(0, d.startFrac + dx / d.width));
    setLeftFrac(next);
  };

  const onResizePointerUp = (ev: ReactPointerEvent<HTMLButtonElement>) => {
    const d = dragRef.current;
    if (!d || d.pointerId !== ev.pointerId) return;
    dragRef.current = null;
    setResizing(false);
    try {
      ev.currentTarget.releasePointerCapture(ev.pointerId);
    } catch {
      /* ignore */
    }
    const cur = leftFracRef.current;
    if (cur >= LEFT_EXPAND_SNAP) setLeftFrac(1);
    else if (cur <= RIGHT_EXPAND_SNAP) setLeftFrac(0);
    else if (cur >= LEFT_COLLAPSE_SNAP) {
      setLeftFrac(
        cur < (LEFT_EXPAND_SNAP + LEFT_COLLAPSE_SNAP) / 2
          ? DEFAULT_LEFT_FRAC
          : 1,
      );
    } else if (cur <= RIGHT_COLLAPSE_SNAP) {
      setLeftFrac(
        cur > (RIGHT_EXPAND_SNAP + RIGHT_COLLAPSE_SNAP) / 2
          ? DEFAULT_LEFT_FRAC
          : 0,
      );
    } else {
      setLeftFrac(cur);
    }
  };

  const onResizeDoubleClick = () => {
    setLeftFrac((v) => {
      if (v >= LEFT_EXPAND_SNAP || v <= RIGHT_EXPAND_SNAP) return DEFAULT_LEFT_FRAC;
      // 双击：往当前较大一侧拉满
      return v >= 0.5 ? 1 : 0;
    });
  };

  const onSidePointerDown = (ev: ReactPointerEvent<HTMLElement>) => {
    if (leftExpanded || rightExpanded) return;
    if (ev.pointerType === "mouse" && ev.button !== 0) return;
    swipeRef.current = { pointerId: ev.pointerId, startY: ev.clientY };
  };

  const onSidePointerUp = (ev: ReactPointerEvent<HTMLElement>) => {
    const s = swipeRef.current;
    if (!s || s.pointerId !== ev.pointerId) return;
    swipeRef.current = null;
    const dy = ev.clientY - s.startY;
    if (Math.abs(dy) < 40) return;
    // 上滑 → 下一个；下滑 → 上一个
    selectRightByOffset(dy < 0 ? 1 : -1);
  };

  if (tab === "transfer") {
    return (
      <div className="ih-root" data-active={active ? "1" : "0"}>
        <div className="ih-transfer">
          <IslandPanelHost
            pullContent={`plugin:${TRANSFER_STATION_PLUGIN_ID}`}
            active={active && tab === "transfer"}
            onPanelClose={onPanelClose}
          />
        </div>
      </div>
    );
  }

  if (tab === "search") {
    return (
      <div className="ih-root" data-active={active ? "1" : "0"}>
        <div className="ih-transfer ih-search">
          <IslandPanelHost
            pullContent={`plugin:${FILE_SEARCH_PLUGIN_ID}`}
            active={active && tab === "search"}
            searchSubmit={searchSubmit}
            onPanelClose={onPanelClose}
          />
        </div>
      </div>
    );
  }

  const leftLabel =
    providers.find((p) => p.id === leftPluginId)?.label ??
    pluginRegistry.get(leftPluginId ?? "")?.manifest.name ??
    "面板";
  const rightLabel =
    rightProviders.find((p) => p.id === rightPluginId)?.label ?? "侧栏";

  return (
    <div className="ih-root" data-active={active ? "1" : "0"}>
      <div
        ref={bodyRef}
        className={`ih-body${leftExpanded ? " is-expanded" : ""}${rightExpanded ? " is-right-expanded" : ""}${resizing ? " is-resizing" : ""}`}
      >
        {!rightExpanded ? (
          <section
            className={`ih-card ih-main${showExpandSplit ? " is-split" : ""}`}
            aria-label={leftLabel}
            style={
              leftExpanded
                ? undefined
                : {
                    flex: "0 0 auto",
                    width: `calc((100% - ${handleGapPx}px) * ${leftFrac})`,
                  }
            }
          >
            {leftPluginId ? (
              <>
                <div className="ih-embed">
                  <IslandPanelHost
                    key={`left:${leftPluginId}`}
                    pullContent={`plugin:${leftPluginId}`}
                    active={active && Boolean(leftPluginId)}
                    onPanelClose={onPanelClose}
                  />
                </div>
                {showExpandSplit && expandCompanion ? (
                  <div
                    className="ih-embed ih-expand-pane"
                    aria-label={leftIsWeather ? "7天预报" : "歌词"}
                  >
                    <IslandPanelHost
                      key={`left-expand:${leftPluginId}:${expandCompanion}`}
                      pullContent={`plugin:${leftPluginId}`}
                      panelEntry={expandCompanion}
                      active={active && Boolean(leftPluginId)}
                      onPanelClose={onPanelClose}
                    />
                  </div>
                ) : null}
              </>
            ) : (
              <div className="ih-placeholder">
                <strong>暂无面板插件</strong>
                <span>启用天气 / 镜子 / 正在播放等 dashboard 面板后出现在此</span>
              </div>
            )}
          </section>
        ) : null}

        <button
          type="button"
          className="ih-handle"
          aria-label={
            leftExpanded
              ? "向左拖收回右侧卡片"
              : rightExpanded
                ? "向右拖收回左侧卡片"
                : "左右拖动手把调整大小"
          }
          aria-pressed={leftExpanded || rightExpanded}
          title={
            leftExpanded
              ? "向左拖收回右侧卡片"
              : rightExpanded
                ? "向右拖收回左侧卡片"
                : "向右拖大左卡，向左拖大右卡；拉满可单卡展开"
          }
          onPointerDown={onResizePointerDown}
          onPointerMove={onResizePointerMove}
          onPointerUp={onResizePointerUp}
          onPointerCancel={onResizePointerUp}
          onDoubleClick={onResizeDoubleClick}
        />

        {!leftExpanded ? (
          <aside
            className="ih-card ih-side"
            aria-label={rightLabel}
            style={
              rightExpanded
                ? undefined
                : {
                    flex: "1 1 0",
                    minWidth: 0,
                  }
            }
            onPointerDown={onSidePointerDown}
            onPointerUp={onSidePointerUp}
            onPointerCancel={() => {
              swipeRef.current = null;
            }}
            title={
              rightProviders.length > 1 ? "上下滑动切换插件" : undefined
            }
          >
            {rightPluginId ? (
              <div className="ih-side-stack">
                <div className="ih-embed">
                  <IslandPanelHost
                    key={`right:${rightPluginId}`}
                    pullContent={`plugin:${rightPluginId}`}
                    active={active && Boolean(rightPluginId)}
                    onPanelClose={onPanelClose}
                  />
                </div>
              </div>
            ) : (
              <div className="ih-cal">
                <div className="ih-cal-head">
                  <div className="ih-cal-month">
                    <strong>{calendar.monthLabel}</strong>
                    <span>{calendar.year}</span>
                  </div>
                  <div className="ih-cal-strip">
                    {calendar.days.map((d) => (
                      <div
                        key={`${d.weekday}-${d.day}`}
                        className={`ih-cal-day${d.selected ? " is-selected" : ""}`}
                      >
                        <span className="ih-cal-wd">{d.weekday}</span>
                        <span className="ih-cal-num">{d.day}</span>
                      </div>
                    ))}
                  </div>
                </div>
                <div className="ih-cal-empty">
                  <IconCalCheck />
                  <strong>No events today</strong>
                  <span>Enjoy your free time!</span>
                </div>
              </div>
            )}
          </aside>
        ) : null}

        {!leftExpanded && rightProviders.length > 1 ? (
          <div
            className="ih-dots"
            role="tablist"
            aria-label="右侧插件"
            title="悬停后滚轮切换"
            onWheel={onDotsWheel}
          >
            {rightProviders.map((p) => (
              <button
                key={p.id}
                type="button"
                role="tab"
                className={`ih-dot${rightPluginId === p.id ? " is-active" : ""}`}
                aria-selected={rightPluginId === p.id}
                aria-label={p.label}
                title={p.label}
                onPointerDown={(e) => e.stopPropagation()}
                onClick={(e) => {
                  e.stopPropagation();
                  setRightId(p.id);
                }}
              />
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}

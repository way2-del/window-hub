/** 情景临时 > 显式会话 > Host 固定首页（不再读用户插件下拉偏好） */
export const ISLAND_PULL_HOME = "home";

export function resolveIslandPullContent(
  opts: {
    scenarioOwner: string | null;
    scenarioPull: string | null;
    sessionOverride: string | null;
    sessionOverrideActive: boolean;
  },
  enabledPullContent: (raw: string) => string,
): string {
  if (opts.scenarioOwner) {
    const sp = opts.scenarioPull ?? `plugin:${opts.scenarioOwner}`;
    const v = enabledPullContent(sp);
    if (v) return v;
  }
  if (opts.sessionOverrideActive && opts.sessionOverride) {
    const v = enabledPullContent(opts.sessionOverride);
    if (v) return v;
  }
  return ISLAND_PULL_HOME;
}

export function isIslandHomePull(raw: string | null | undefined): boolean {
  const t = String(raw ?? "").trim();
  return !t || t === ISLAND_PULL_HOME || t === "none" || t === "off";
}

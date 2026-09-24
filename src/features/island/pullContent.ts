/** 情景临时 > 显式会话 > 用户「下拉内容」 */
export function resolveIslandPullContent(opts: {
  scenarioOwner: string | null;
  scenarioPull: string | null;
  sessionOverride: string | null;
  sessionOverrideActive: boolean;
  pullContent: string;
}, enabledPullContent: (raw: string) => string): string {
  if (opts.scenarioOwner) {
    const sp = opts.scenarioPull ?? `plugin:${opts.scenarioOwner}`;
    const v = enabledPullContent(sp);
    if (v) return v;
  }
  if (opts.sessionOverrideActive && opts.sessionOverride) {
    const v = enabledPullContent(opts.sessionOverride);
    if (v) return v;
  }
  return enabledPullContent(opts.pullContent);
}

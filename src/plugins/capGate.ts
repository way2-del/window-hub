import type { PluginCapability, PluginManifest } from "./types";

const CAP_LABELS: Record<PluginCapability, string> = {
  storage: "可在本机保存数据",
  shortcuts: "可在状态菜单快捷区显示",
  notify: "可推送灵动岛通知",
  popup: "可打开托管弹窗",
  "island.panel": "可在灵动岛展开显示界面",
  "island.bar": "可写入灵动岛栏摘要（可竞选常驻）",
  "island.drop": "可接收拖到灵动岛上的文件/文字",
  "windows.read": "可枚举窗口标题与进程",
  "windows.focus": "可切换其它窗口焦点",
  "media.keys": "可发送系统媒体键（播放/暂停/切歌）",
  "clipboard.read": "可读剪贴板",
  "clipboard.write": "可写剪贴板",
  network: "可访问网络（受白名单限制）",
  staging: "可读写岛上暂存区（文件/文字/图片）",
  "everything.search": "可经 Everything 搜索本机文件并打开路径",
};

/** Frontend CapGate — reject hub calls when capability missing. */
export function assertCapability(
  manifest: PluginManifest,
  cap: PluginCapability,
): void {
  const caps = manifest.capabilities ?? [];
  if (!caps.includes(cap)) {
    throw new Error(
      `plugin ${manifest.id} missing capability "${cap}" (${CAP_LABELS[cap]})`,
    );
  }
}

export function describeCapabilities(caps: PluginCapability[]): string[] {
  return caps.map((c) => CAP_LABELS[c] ?? c);
}

export function isSensitiveCapability(cap: PluginCapability): boolean {
  return (
    cap === "windows.focus" ||
    cap === "media.keys" ||
    cap === "clipboard.read" ||
    cap === "clipboard.write" ||
    cap === "staging" ||
    cap === "island.drop" ||
    cap === "network" ||
    cap === "everything.search"
  );
}

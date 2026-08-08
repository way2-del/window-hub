/**
 * 灵动岛外形：主体 + 左侧内凹衔接弧；右侧 = 左侧水平翻转（保证完全一致）。
 */
export default function IslandShape({ className }: { className?: string }) {
  return (
    <div className={className} aria-hidden>
      <span className="island-ear island-ear-left" />
      <span className="island-body" />
      <span className="island-ear island-ear-right" />
    </div>
  );
}

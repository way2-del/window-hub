import SlotView from "./SlotView";
import type { SlotInfo } from "../types";

type Props = {
  slots: SlotInfo[];
  frameUrls: Record<number, string>;
  onRequestAttach: (slot: number) => void;
  onDetach: (slot: number) => void;
  onSwapNext: (slot: number) => void;
};

export default function MosaicGrid({
  slots,
  frameUrls,
  onRequestAttach,
  onDetach,
  onSwapNext,
}: Props) {
  return (
    <div className="mosaic">
      {slots.map((s) => (
        <SlotView
          key={s.slot}
          slot={s}
          frameUrl={frameUrls[s.slot]}
          onRequestAttach={onRequestAttach}
          onDetach={onDetach}
          onSwapNext={onSwapNext}
        />
      ))}
    </div>
  );
}

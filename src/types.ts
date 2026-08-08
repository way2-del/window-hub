export type CaptureRoi = {
  x: number;
  y: number;
  w: number;
  h: number;
  use_full: boolean;
};

export type WindowInfo = {
  id?: string;
  hwnd: number;
  title: string;
  class_name: string;
  pid: number;
  exe?: string | null;
  exe_name?: string | null;
};

export type SlotInfo = {
  slot: number;
  hwnd: number | null;
  title: string | null;
  class_name: string | null;
  pid: number | null;
  roi: CaptureRoi | null;
};

export type FrameEvent = {
  slot: number;
  width: number;
  height: number;
  seq: number;
  jpeg_base64: string;
};

export type SlotStateEvent = {
  slots: SlotInfo[];
};

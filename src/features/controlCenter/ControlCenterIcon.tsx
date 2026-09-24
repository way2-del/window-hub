import type { ReactNode } from "react";

export type ControlIconName = "wifi" | "bluetooth" | "hotspot" | "moon" | "stage" | "cast" | "music" | "play" | "next" | "mic";

const paths: Record<ControlIconName, ReactNode> = {
  wifi: <><path d="M3 8a15 15 0 0 1 18 0M6 12a10 10 0 0 1 12 0M9 16a5 5 0 0 1 6 0" /><circle cx="12" cy="20" r="1" fill="currentColor" stroke="none" /></>,
  bluetooth: <path d="m7 7 10 10-5 4V3l5 4L7 17" />,
  hotspot: <><path d="m10 8 2-2a4 4 0 0 1 6 6l-2 2M14 16l-2 2a4 4 0 0 1-6-6l2-2M9 15l6-6" /></>,
  moon: <path d="M20 14a8 8 0 0 1-10-10 8.5 8.5 0 1 0 10 10Z" fill="currentColor" stroke="none" />,
  stage: <><rect x="2" y="3" width="12" height="7" rx="2" /><rect x="2" y="14" width="12" height="7" rx="2" /><rect x="18" y="6" width="4" height="12" rx="1.5" /></>,
  cast: <><rect x="3" y="4" width="14" height="12" rx="3" fill="currentColor" stroke="none" opacity=".55" /><rect x="8" y="8" width="14" height="12" rx="3" fill="currentColor" stroke="none" /></>,
  music: <><path d="M9 18V5l11-2v13M9 8l11-2" /><ellipse cx="6" cy="19" rx="3" ry="2" fill="currentColor" /><ellipse cx="17" cy="17" rx="3" ry="2" fill="currentColor" /></>,
  play: <path d="m7 4 14 8-14 8Z" fill="currentColor" stroke="none" />,
  next: <path d="m2 5 10 7L2 19Zm10 0 10 7-10 7Z" fill="currentColor" stroke="none" />,
  mic: <><rect x="9" y="2" width="6" height="13" rx="3" /><path d="M5 11v1a7 7 0 0 0 14 0v-1M12 19v3M8 22h8" /></>,
};

export default function ControlCenterIcon({ name }: { name: ControlIconName }) {
  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{paths[name]}</svg>;
}

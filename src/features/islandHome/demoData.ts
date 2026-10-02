/** 首页演示数据（后续可接 SMTC / 系统日历） */

export type MainCardId = "music" | "cloud" | "code" | "files";

export const DEMO_MUSIC = {
  title: "LEMONADE",
  artist: "aespa",
  currentSec: 89,
  durationSec: 187,
  /** 简易封面：CSS 渐变占位，不依赖外链图 */
  coverHue: 95,
  lyrics: [
    { t: 0, text: "When life gives you lemons" },
    { t: 12, text: "Make lemonade, make lemonade" },
    { t: 28, text: "Sip it slow under the sun" },
    { t: 44, text: "We're dancing till the night is done" },
    { t: 62, text: "Sweet and sour, take a bite" },
    { t: 80, text: "LEMONADE — shining bright" },
    { t: 98, text: "Don't look back, keep the vibe" },
    { t: 116, text: "Pour it up, we're still alive" },
  ],
};

export type DemoDay = {
  weekday: string;
  day: number;
  selected?: boolean;
};

export function buildDemoCalendar(now = new Date()): {
  monthLabel: string;
  year: number;
  days: DemoDay[];
} {
  const year = now.getFullYear();
  const month = now.getMonth();
  const selected = now.getDate();
  const weekdays = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  const days: DemoDay[] = [];
  for (let offset = -2; offset <= 2; offset++) {
    const d = new Date(year, month, selected + offset);
    days.push({
      weekday: weekdays[d.getDay()]!,
      day: d.getDate(),
      selected: offset === 0,
    });
  }
  const monthShort = dMonth(month);
  return { monthLabel: monthShort, year, days };
}

function dMonth(m: number): string {
  return ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][
    m
  ]!;
}

export function formatClock(sec: number): string {
  const n = Math.max(0, Math.floor(sec));
  const m = Math.floor(n / 60);
  const s = n % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

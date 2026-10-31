import type { HistoryRow } from "@/types/api";

export const RANGES = [
  { label: "15 min", secs: 15 * 60 },
  { label: "1 h", secs: 3600 },
  { label: "6 h", secs: 6 * 3600 },
  { label: "24 h", secs: 24 * 3600 },
  { label: "7 d", secs: 7 * 24 * 3600 },
  { label: "30 d", secs: 30 * 24 * 3600 },
] as const;

const GB = 1024 ** 3;

export interface Point {
  t: number;
  ramAvg: number;
  ramMax: number;
  cpuAvg: number;
  cpuMax: number;
  [free: `free:${string}`]: number | null;
}

/** Rows -> flat chart points, with one `free:<drive>` column (GB) per drive seen in the data. */
export function chartData(rows: HistoryRow[]): { points: Point[]; drives: string[] } {
  const drives = [...new Set(rows.flatMap((r) => r.disks.map((d) => d.mount)))].sort();
  const points = rows.map((r) => {
    const p: Point = { t: r.tsMs, ramAvg: round(r.ramAvg), ramMax: round(r.ramMax), cpuAvg: round(r.cpuAvg), cpuMax: round(r.cpuMax) };
    for (const d of drives) {
      const hit = r.disks.find((x) => x.mount === d);
      p[`free:${d}`] = hit ? round(hit.freeBytes / GB) : null;
    }
    return p;
  });
  return { points, drives };
}

function round(n: number): number {
  return Math.round(n * 10) / 10;
}

/** Axis label: clock time for short ranges, date for long ones. */
export function tickLabel(ms: number, rangeSecs: number): string {
  const d = new Date(ms);
  if (rangeSecs <= 24 * 3600) return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

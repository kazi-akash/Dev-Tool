import { describe, expect, it } from "vitest";
import { RANGES, chartData, tickLabel } from "@/features/history/series";
import type { HistoryRow } from "@/types/api";

const GB = 1024 ** 3;
const row = (ts: number, ram: number, disks: HistoryRow["disks"] = []): HistoryRow => ({
  tsMs: ts,
  samples: 1,
  ramAvg: ram,
  ramMax: ram + 5.04,
  cpuAvg: 10.26,
  cpuMax: 30,
  ramUsedAvg: 1,
  ramTotal: 2,
  disks,
  top: [],
});

describe("history series", () => {
  it("rounds values and flattens drives into free:<drive> columns in GB", () => {
    const { points, drives } = chartData([
      row(1000, 50.04, [
        { mount: "C:\\", freeBytes: 40 * GB, totalBytes: 100 * GB },
        { mount: "S:\\", freeBytes: 10.55 * GB, totalBytes: 100 * GB },
      ]),
    ]);
    expect(drives).toEqual(["C:\\", "S:\\"]);
    expect(points[0]).toMatchObject({ t: 1000, ramAvg: 50, ramMax: 55.1, cpuAvg: 10.3, "free:C:\\": 40, "free:S:\\": 10.6 });
  });

  it("uses null for a drive that was absent at that time (unplugged)", () => {
    const { points } = chartData([row(1, 1, [{ mount: "D:\\", freeBytes: GB, totalBytes: 2 * GB }]), row(2, 1, [])]);
    expect(points[0]["free:D:\\"]).toBe(1);
    expect(points[1]["free:D:\\"]).toBeNull();
  });

  it("handles empty data", () => {
    expect(chartData([])).toEqual({ points: [], drives: [] });
  });

  it("offers ranges from 15 minutes to 30 days in increasing order", () => {
    expect(RANGES[0].label).toBe("15 min");
    expect(RANGES.at(-1)!.secs).toBe(30 * 24 * 3600);
    expect(RANGES.map((r) => r.secs)).toEqual([...RANGES.map((r) => r.secs)].sort((a, b) => a - b));
  });

  it("labels short ranges with time and long ranges with dates", () => {
    const ms = new Date(2026, 5, 15, 14, 30).getTime();
    expect(tickLabel(ms, 3600)).toMatch(/14:30|2:30/);
    expect(tickLabel(ms, 7 * 24 * 3600)).toMatch(/Jun|15/);
  });
});

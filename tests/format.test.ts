import { describe, expect, it } from "vitest";
import { formatBytes, formatPercent, formatUptime } from "@/lib/format";

describe("formatBytes", () => {
  it("formats common sizes", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(16 * 1024 ** 3)).toBe("16.0 GB");
  });

  it("is safe for bad input", () => {
    expect(formatBytes(NaN)).toBe("0 B");
    expect(formatBytes(-5)).toBe("0 B");
    expect(formatBytes(Infinity)).toBe("0 B");
  });

  it("caps at TB", () => {
    expect(formatBytes(2048 * 1024 ** 4)).toBe("2048.0 TB");
  });
});

describe("formatUptime", () => {
  it("formats days, hours, minutes", () => {
    expect(formatUptime(93784)).toBe("1d 2h 3m");
    expect(formatUptime(3700)).toBe("1h 1m");
    expect(formatUptime(59)).toBe("0m");
  });

  it("is safe for bad input", () => {
    expect(formatUptime(NaN)).toBe("0m");
    expect(formatUptime(-1)).toBe("0m");
  });
});

describe("formatPercent", () => {
  it("rounds and guards NaN", () => {
    expect(formatPercent(85.6)).toBe("86%");
    expect(formatPercent(NaN)).toBe("0%");
  });
});

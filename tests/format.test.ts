import { describe, expect, it } from "vitest";
import { formatBytes, formatDate, formatHours, formatPercent, formatUptime, timeAgo } from "@/lib/format";
import { errorCode, errorMessage } from "@/lib/ipc";

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

describe("formatPercent / formatHours / timeAgo / formatDate", () => {
  it("rounds and guards NaN", () => {
    expect(formatPercent(85.6)).toBe("86%");
    expect(formatPercent(NaN)).toBe("0%");
  });

  it("formats durations", () => {
    expect(formatHours(0.5)).toBe("30 min");
    expect(formatHours(0.001)).toBe("1 min");
    expect(formatHours(5)).toBe("5.0 h");
    expect(formatHours(100)).toBe("4 days");
    expect(formatHours(NaN)).toBe("unknown");
    expect(formatHours(-1)).toBe("unknown");
  });

  it("formats relative time", () => {
    const now = 10_000_000_000;
    expect(timeAgo(now - 10_000, now)).toBe("just now");
    expect(timeAgo(now - 5 * 60_000, now)).toBe("5 min ago");
    expect(timeAgo(now - 3 * 3600_000, now)).toBe("3 h ago");
    expect(timeAgo(now - 2 * 86400_000, now)).toBe("2 days ago");
    expect(timeAgo(now + 5000, now)).toBe("just now");
  });

  it("handles missing dates", () => {
    expect(formatDate(null)).toBe("unknown");
    expect(formatDate(0)).toBe("unknown");
    expect(formatDate(1_700_000_000)).toMatch(/2023/);
  });
});

describe("ipc error helpers", () => {
  it("extracts readable messages from every shape Rust and the bridge produce", () => {
    expect(errorMessage("plain string")).toBe("plain string");
    expect(errorMessage({ code: "refused", message: "nope" })).toBe("nope");
    expect(errorMessage(new Error("boom"))).toBe("boom");
    expect(errorMessage(undefined)).toBe("Something went wrong.");
    expect(errorMessage({})).toBe("Something went wrong.");
    expect(errorCode({ code: "auth", message: "x" })).toBe("auth");
    expect(errorCode("x")).toBeUndefined();
  });
});

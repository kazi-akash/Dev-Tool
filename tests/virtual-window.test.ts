import { describe, expect, it } from "vitest";
import { windowRange } from "@/hooks/useVirtualWindow";

describe("windowRange", () => {
  it("renders the viewport plus overscan and pads the rest", () => {
    // 10 000 rows of 36 px, a 540 px viewport (15 rows), scrolled to row 100.
    const r = windowRange(10_000, 36, 100 * 36, 540, 8);
    expect(r.start).toBe(92);
    expect(r.end).toBe(100 + 15 + 8);
    expect(r.end - r.start).toBeLessThan(40);
    expect(r.padTop).toBe(92 * 36);
    expect(r.padBottom).toBe((10_000 - r.end) * 36);
    // Total height is preserved so the scrollbar stays honest.
    expect(r.padTop + (r.end - r.start) * 36 + r.padBottom).toBe(10_000 * 36);
  });

  it("starts at the top and clamps at the bottom", () => {
    expect(windowRange(1000, 36, 0, 540, 8)).toMatchObject({ start: 0, padTop: 0 });
    const bottom = windowRange(1000, 36, 1000 * 36, 540, 8);
    expect(bottom.end).toBe(1000);
    expect(bottom.padBottom).toBe(0);
    expect(bottom.start).toBeLessThanOrEqual(1000);
  });

  it("handles empty lists, negative scroll and a zero-height viewport", () => {
    expect(windowRange(0, 36, 0, 540, 8)).toEqual({ start: 0, end: 0, padTop: 0, padBottom: 0 });
    expect(windowRange(50, 36, -200, 540, 8).start).toBe(0);
    const r = windowRange(50, 36, 0, 0, 2);
    expect(r.end).toBeGreaterThan(0);
  });

  it("never renders more than viewport + 2 x overscan rows", () => {
    for (const top of [0, 123, 5000, 99_999, 360_000]) {
      const r = windowRange(10_000, 36, top, 600, 8);
      expect(r.end - r.start).toBeLessThanOrEqual(Math.ceil(600 / 36) + 16 + 1);
    }
  });
});

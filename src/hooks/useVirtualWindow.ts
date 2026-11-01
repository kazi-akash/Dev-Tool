import { useCallback, useEffect, useRef, useState } from "react";

export interface WindowRange {
  /** First row to render. */
  start: number;
  /** One past the last row to render. */
  end: number;
  /** Height of the rows above `start` that are not rendered. */
  padTop: number;
  /** Height of the rows at or after `end` that are not rendered. */
  padBottom: number;
}

/**
 * Which rows of a fixed-height list to render. Pure: scroll position and viewport are inputs.
 * Rows outside the viewport (plus `overscan`) are replaced by padding, so the DOM stays small.
 */
export function windowRange(count: number, rowHeight: number, scrollTop: number, viewport: number, overscan: number): WindowRange {
  if (count <= 0 || rowHeight <= 0) return { start: 0, end: 0, padTop: 0, padBottom: 0 };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const visible = Math.ceil(Math.max(viewport, rowHeight) / rowHeight);
  const start = Math.min(Math.max(0, first - overscan), count);
  const end = Math.min(count, first + visible + overscan);
  return { start, end, padTop: start * rowHeight, padBottom: (count - end) * rowHeight };
}

export interface VirtualOptions {
  rowHeight: number;
  overscan?: number;
  /** Lists at or below this many rows render fully: no scrolling machinery needed. */
  threshold?: number;
  /** Viewport height to assume when the element has no layout yet (tests, first paint). */
  fallbackViewport?: number;
}

/**
 * Attach `ref` to the scrolling container and render only `range.start..range.end` of your rows,
 * with `padTop`/`padBottom` spacers. Below `threshold` it is disabled and the whole list renders.
 */
export function useVirtualWindow(count: number, { rowHeight, overscan = 8, threshold = 100, fallbackViewport = 560 }: VirtualOptions) {
  const ref = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(fallbackViewport);
  const enabled = count > threshold;

  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    setViewport(el.clientHeight || fallbackViewport);
    setScrollTop(el.scrollTop);
  }, [fallbackViewport]);

  useEffect(() => {
    const el = ref.current;
    if (!el || !enabled) return;
    measure();
    el.addEventListener("scroll", measure, { passive: true });
    const ro = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure);
    ro?.observe(el);
    return () => {
      el.removeEventListener("scroll", measure);
      ro?.disconnect();
    };
  }, [enabled, measure]);

  const range: WindowRange = enabled ? windowRange(count, rowHeight, scrollTop, viewport, overscan) : { start: 0, end: count, padTop: 0, padBottom: 0 };
  return { ref, enabled, range };
}

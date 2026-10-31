import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "@/lib/ipc";

export interface AsyncState<T> {
  data: T | null;
  error: string | null;
  loading: boolean;
}

/**
 * Run an async loader on mount, on demand, and optionally on an interval. Overlapping runs are
 * dropped, results from unmounted components are ignored, and the previous data stays visible
 * while a refresh is in flight.
 */
export function useAsync<T>(loader: () => Promise<T>, opts: { intervalMs?: number; auto?: boolean; deps?: unknown[] } = {}) {
  const { intervalMs, auto = true, deps = [] } = opts;
  const [state, setState] = useState<AsyncState<T>>({ data: null, error: null, loading: false });
  const loaderRef = useRef(loader);
  loaderRef.current = loader;
  const alive = useRef(true);
  const busy = useRef(false);

  const run = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    setState((s) => ({ ...s, loading: true }));
    try {
      const data = await loaderRef.current();
      if (alive.current) setState({ data, error: null, loading: false });
    } catch (e) {
      if (alive.current) setState((s) => ({ data: s.data, error: errorMessage(e), loading: false }));
    } finally {
      busy.current = false;
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    if (auto) void run();
    const id = intervalMs ? window.setInterval(() => void run(), intervalMs) : undefined;
    return () => {
      alive.current = false;
      if (id) window.clearInterval(id);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auto, intervalMs, run, ...deps]);

  return { ...state, reload: run };
}

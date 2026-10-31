import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ProcessList, ProcessQuery } from "@/types/process";

/**
 * Process list from the Rust monitor. Re-queried whenever a `system-metrics` tick arrives
 * (which only happens while the window is visible) and whenever the query changes.
 */
export function useProcesses(query: ProcessQuery): ProcessList | null {
  const [list, setList] = useState<ProcessList | null>(null);
  const queryRef = useRef(query);
  queryRef.current = query;

  const { limit, sortBy, category } = query;

  useEffect(() => {
    let cancelled = false;
    let inFlight = false;
    let unlisten: (() => void) | undefined;

    const refresh = () => {
      if (inFlight) return; // drop ticks while a query is pending
      inFlight = true;
      invoke<ProcessList>("get_processes", { query: queryRef.current })
        .then((l) => {
          if (!cancelled) setList(l);
        })
        .catch(() => {})
        .finally(() => {
          inFlight = false;
        });
    };

    refresh();
    listen("system-metrics", refresh)
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [limit, sortBy, category]);

  return list;
}

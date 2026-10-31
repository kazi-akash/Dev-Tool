import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { SystemSnapshot } from "@/types/system";

/**
 * Live system snapshot from the Rust monitor. `null` until the first sample arrives
 * (or permanently outside Tauri, e.g. plain browser / unit tests).
 */
export function useSystemMetrics(): SystemSnapshot | null {
  const [snapshot, setSnapshot] = useState<SystemSnapshot | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    // Subscribe first so no sample is missed, then fetch the latest for instant first paint.
    listen<SystemSnapshot>("system-metrics", (e) => setSnapshot(e.payload))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    invoke<SystemSnapshot | null>("get_system_snapshot")
      .then((s) => {
        if (!cancelled && s) setSnapshot((prev) => prev ?? s);
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  return snapshot;
}

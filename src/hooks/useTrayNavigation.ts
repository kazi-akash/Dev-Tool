import { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";
import { call } from "@/lib/ipc";

/**
 * Tray menu navigation. Rust either emits `navigate` (window already open) or leaves a pending
 * route that this page claims once on startup (window was closed and just recreated). Rust only
 * ever sends routes from a fixed allow-list; unknown values are ignored here too.
 */
const KNOWN = new Set(["/", "/processes", "/storage", "/diagnosis", "/actions", "/history", "/settings"]);

export function useTrayNavigation() {
  const navigate = useNavigate();

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    const go = (route: unknown) => {
      if (typeof route === "string" && KNOWN.has(route)) navigate(route);
    };

    call<string | null>("take_pending_route")
      .then((r) => !cancelled && go(r))
      .catch(() => {
        // not running inside Tauri (plain browser / unit tests)
      });

    listen<string>("navigate", (event) => go(event.payload))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [navigate]);
}

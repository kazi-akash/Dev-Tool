import { useEffect } from "react";
import { useNavigate } from "react-router-dom";
import { listen } from "@tauri-apps/api/event";

/** Rust tray menu emits `navigate` with an app route; this wires it to the router. */
export function useTrayNavigation() {
  const navigate = useNavigate();

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    listen<string>("navigate", (event) => navigate(event.payload))
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // not running inside Tauri (e.g. plain browser / Vitest)
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [navigate]);
}

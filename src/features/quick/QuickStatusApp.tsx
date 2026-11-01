import { useEffect } from "react";
import { QuickStatusPage } from "./QuickStatusPage";

/** Same theme choice as the main window (stored by Settings), without loading the settings provider. */
function useStoredTheme() {
  useEffect(() => {
    let mode = "system";
    try {
      mode = (JSON.parse(localStorage.getItem("devpilot.settings.v1") ?? "{}") as { theme?: string }).theme ?? "system";
    } catch {
      // unreadable settings: follow the system
    }
    const dark = mode === "dark" || (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
    document.documentElement.classList.toggle("dark", dark);
  }, []);
}

export function QuickStatusApp() {
  useStoredTheme();
  return <QuickStatusPage />;
}

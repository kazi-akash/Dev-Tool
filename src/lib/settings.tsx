import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { call } from "@/lib/ipc";
import type { AiConfig } from "@/types/api";
import type { DiskThresholds, PressureThresholds } from "@/types/system";

export type ThemeMode = "system" | "light" | "dark";

export interface AiSettings extends AiConfig {
  /** Master switch. Off by default: nothing is ever sent unless the user turns this on. */
  enabled: boolean;
}

export interface Settings {
  theme: ThemeMode;
  /** System metrics polling interval in seconds; applied to the Rust monitor (valid range 1-60). */
  monitorIntervalSec: number;
  pressureThresholds: PressureThresholds;
  diskThresholds: DiskThresholds;
  /** Opt-in: clean old temporary files automatically when a drive runs low. */
  safeAutomation: boolean;
  ai: AiSettings;
}

const STORAGE_KEY = "devpilot.settings.v1";

export const DEFAULT_PRESSURE: PressureThresholds = { moderate: 70, high: 85, critical: 92 };
export const DEFAULT_DISK: DiskThresholds = { warningFree: 20, highFree: 10, criticalFree: 5 };

export const DEFAULT_SETTINGS: Settings = {
  theme: "system",
  monitorIntervalSec: 2,
  pressureThresholds: DEFAULT_PRESSURE,
  diskThresholds: DEFAULT_DISK,
  safeAutomation: false,
  ai: { enabled: false, kind: "openAi", baseUrl: "https://api.openai.com/v1", model: "gpt-4o-mini" },
};

function load(): Settings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_SETTINGS;
    const s = JSON.parse(raw) as Partial<Settings>;
    return {
      ...DEFAULT_SETTINGS,
      ...s,
      pressureThresholds: { ...DEFAULT_PRESSURE, ...s.pressureThresholds },
      diskThresholds: { ...DEFAULT_DISK, ...s.diskThresholds },
      ai: { ...DEFAULT_SETTINGS.ai, ...s.ai },
    };
  } catch {
    return DEFAULT_SETTINGS;
  }
}

function applyTheme(mode: ThemeMode) {
  const dark = mode === "dark" || (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", dark);
}

interface SettingsContextValue {
  settings: Settings;
  update: (patch: Partial<Settings>) => void;
}

const SettingsContext = createContext<SettingsContextValue | null>(null);

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [settings, setSettings] = useState<Settings>(load);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    } catch {
      // storage unavailable: keep in-memory settings only
    }
  }, [settings]);

  // Push values the Rust side owns. Rust validates every one; failures leave Rust's current value.
  useEffect(() => {
    call("set_monitor_interval", { secs: settings.monitorIntervalSec }).catch(() => {});
  }, [settings.monitorIntervalSec]);

  useEffect(() => {
    const t = settings.pressureThresholds;
    call("set_pressure_thresholds", { moderate: t.moderate, high: t.high, critical: t.critical }).catch(() => {});
  }, [settings.pressureThresholds]);

  useEffect(() => {
    const t = settings.diskThresholds;
    call("set_disk_thresholds", { warningFree: t.warningFree, highFree: t.highFree, criticalFree: t.criticalFree }).catch(() => {});
  }, [settings.diskThresholds]);

  useEffect(() => {
    call("set_safe_automation", { enabled: settings.safeAutomation }).catch(() => {});
  }, [settings.safeAutomation]);

  useEffect(() => {
    applyTheme(settings.theme);
    if (settings.theme !== "system") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => applyTheme("system");
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, [settings.theme]);

  const value = useMemo<SettingsContextValue>(() => ({ settings, update: (patch) => setSettings((s) => ({ ...s, ...patch })) }), [settings]);

  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

export function useSettings() {
  const ctx = useContext(SettingsContext);
  if (!ctx) throw new Error("useSettings must be used inside SettingsProvider");
  return ctx;
}

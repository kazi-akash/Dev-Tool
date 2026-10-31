import type { Tone } from "@/components/ui/badge";
import type { RiskLevel, Safety, Severity } from "@/types/api";
import type { DiskState, PressureState } from "@/types/system";

export const PRESSURE_TONE: Record<PressureState, Tone> = {
  NORMAL: "ok",
  MODERATE: "warn",
  HIGH: "danger",
  CRITICAL: "danger",
};

export const DISK_TONE: Record<DiskState, Tone> = {
  NORMAL: "ok",
  WARNING: "warn",
  HIGH: "danger",
  CRITICAL: "danger",
};

export const SEVERITY_TONE: Record<Severity, Tone> = {
  info: "info",
  warning: "warn",
  high: "danger",
  critical: "danger",
};

export const RISK_TONE: Record<RiskLevel, Tone> = {
  SAFE: "ok",
  LOW: "ok",
  MEDIUM: "warn",
  HIGH: "danger",
  DESTRUCTIVE: "danger",
};

export const SAFETY_TONE: Record<Safety, Tone> = {
  SAFE_TO_CLEAN: "ok",
  SAFE_TO_REVIEW: "info",
  REQUIRES_REVIEW: "warn",
  DO_NOT_TOUCH: "danger",
};

export const SAFETY_LABEL: Record<Safety, string> = {
  SAFE_TO_CLEAN: "Safe to clean",
  SAFE_TO_REVIEW: "Safe to review",
  REQUIRES_REVIEW: "Requires review",
  DO_NOT_TOUCH: "Do not touch",
};

/** Percent used -> tone for a plain usage bar (CPU, disks without a state engine). */
export function usageTone(percent: number): Tone {
  if (percent >= 85) return "danger";
  if (percent >= 70) return "warn";
  return "ok";
}

export function pressureTone(state: PressureState | undefined): Tone {
  return state ? PRESSURE_TONE[state] : "neutral";
}

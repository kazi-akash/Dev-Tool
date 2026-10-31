import { formatHours } from "@/lib/format";
import type { GrowthFinding } from "@/types/api";
import type { SystemSnapshot } from "@/types/system";
import type { Tone } from "@/components/ui/badge";

export interface Alert {
  id: string;
  tone: Tone;
  text: string;
}

/** Turn engine state into short alert lines. The thresholds live in Rust; this only words them. */
export function buildAlerts(snap: SystemSnapshot | null, growth: GrowthFinding[] | null): Alert[] {
  const alerts: Alert[] = [];
  const p = snap?.pressure;
  if (p && p.state !== "NORMAL") {
    const trend = p.trend === "ESCALATING" ? ", rising quickly" : p.trend === "RISING" ? ", rising" : "";
    const eta = p.minutesToCritical != null && p.minutesToCritical < 60 ? ` Critical in about ${formatHours(p.minutesToCritical / 60)}.` : "";
    alerts.push({
      id: "ram",
      tone: p.state === "MODERATE" ? "warn" : "danger",
      text: `Memory pressure ${p.state.toLowerCase()} (${Math.round(p.percent)}%${trend}).${eta}`,
    });
  }
  for (const d of snap?.diskHealth ?? []) {
    if (d.state === "NORMAL") continue;
    const eta = d.hoursToFull != null ? ` Full in about ${formatHours(d.hoursToFull)} at the current rate.` : "";
    alerts.push({
      id: `disk-${d.mountPoint}`,
      tone: d.state === "WARNING" ? "warn" : "danger",
      text: `${d.mountPoint} is ${d.state.toLowerCase()}: ${Math.round(d.freePercent)}% free.${eta}`,
    });
  }
  for (const g of (growth ?? []).slice(0, 2)) {
    alerts.push({ id: `growth-${g.pid}`, tone: "warn", text: `Possible memory growth/leak in ${g.name} (confidence ${Math.round(g.confidence * 100)}%).` });
  }
  return alerts;
}

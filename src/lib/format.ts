const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** 1536 -> "1.5 KB". Non-finite or negative input renders as "0 B". */
export function formatBytes(bytes: number, digits = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), UNITS.length - 1);
  const value = bytes / 1024 ** i;
  return `${i === 0 ? value : value.toFixed(digits)} ${UNITS[i]}`;
}

/** 93784 -> "1d 2h 3m". Shows at most the two most significant units below days. */
export function formatUptime(secs: number): string {
  if (!Number.isFinite(secs) || secs < 0) return "0m";
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (d > 0) return `${d}d ${h}h ${m}m`;
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function formatPercent(value: number): string {
  return `${Number.isFinite(value) ? Math.round(value) : 0}%`;
}

/** 0.5 -> "30 min", 5 -> "5.0 h", 100 -> "4 days". */
export function formatHours(h: number): string {
  if (!Number.isFinite(h) || h < 0) return "unknown";
  if (h < 1) return `${Math.max(1, Math.round(h * 60))} min`;
  if (h < 48) return `${h.toFixed(1)} h`;
  return `${Math.round(h / 24)} days`;
}

/** Relative time: "just now", "5 min ago", "3 h ago", "2 days ago". */
export function timeAgo(thenMs: number, nowMs: number = Date.now()): string {
  const s = Math.max(0, Math.round((nowMs - thenMs) / 1000));
  if (s < 45) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} days ago`;
}

export function formatDateTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/** Unix seconds -> short date, or "unknown". */
export function formatDate(secs: number | null): string {
  return secs ? new Date(secs * 1000).toLocaleDateString(undefined, { dateStyle: "medium" }) : "unknown";
}

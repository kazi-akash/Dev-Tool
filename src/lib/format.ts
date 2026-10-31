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

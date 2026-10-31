/** Mirrors `monitoring::metrics` and friends in Rust (camelCase via serde). */
export interface MemoryMetrics {
  totalBytes: number;
  usedBytes: number;
  availableBytes: number;
  percent: number;
}

export interface DiskMetrics {
  name: string;
  mountPoint: string;
  totalBytes: number;
  usedBytes: number;
  freeBytes: number;
  usedPercent: number;
}

export type PressureState = "NORMAL" | "MODERATE" | "HIGH" | "CRITICAL";
export type Trend = "FALLING" | "STABLE" | "RISING" | "ESCALATING";

export interface PressureReport {
  state: PressureState;
  trend: Trend;
  percent: number;
  availableBytes: number;
  commitPercent: number | null;
  pagefilePercent: number | null;
  /** Percentage points per minute. */
  ratePerMin: number;
  minutesToCritical: number | null;
  reasons: string[];
}

export type DiskState = "NORMAL" | "WARNING" | "HIGH" | "CRITICAL";

export interface DiskReport {
  mountPoint: string;
  state: DiskState;
  freePercent: number;
  freeBytes: number;
  totalBytes: number;
  trendBytesPerHour: number | null;
  hoursToCritical: number | null;
  hoursToFull: number | null;
  trendConfidence: number | null;
  reasons: string[];
}

export interface SystemSnapshot {
  timestampMs: number;
  memory: MemoryMetrics;
  cpuPercent: number;
  disks: DiskMetrics[];
  diskHealth: DiskReport[];
  pressure: PressureReport | null;
  uptimeSecs: number;
  processCount: number;
}

export interface PressureThresholds {
  moderate: number;
  high: number;
  critical: number;
}

export interface DiskThresholds {
  warningFree: number;
  highFree: number;
  criticalFree: number;
}

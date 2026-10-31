/** Mirrors `monitoring::metrics` in Rust (camelCase via serde). */
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

export interface SystemSnapshot {
  timestampMs: number;
  memory: MemoryMetrics;
  cpuPercent: number;
  disks: DiskMetrics[];
  uptimeSecs: number;
  processCount: number;
}

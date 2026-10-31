/** Mirrors `processes::model` in Rust. */
export const CATEGORIES = [
  "SYSTEM",
  "BROWSER",
  "DEVELOPMENT",
  "DOCKER",
  "NODE",
  "PHP",
  "DATABASE",
  "JAVA",
  "ANDROID",
  "FLUTTER",
  "IDE",
  "OTHER",
] as const;

export type Category = (typeof CATEGORIES)[number];

/** What the category was based on; `name` is the weakest evidence. */
export type Evidence = "path" | "commandLine" | "name" | "none";

export interface ProcessInfo {
  pid: number;
  name: string;
  exePath: string | null;
  /** Already redacted and truncated by Rust. */
  commandLine: string | null;
  parentPid: number | null;
  /** Unix seconds. */
  startTime: number;
  /** Share of total system CPU, 0-100. */
  cpuPercent: number;
  memoryBytes: number;
  category: Category;
  evidence: Evidence;
}

export type SortBy = "memory" | "cpu" | "name";

export interface ProcessQuery {
  limit?: number;
  sortBy?: SortBy;
  category?: Category;
}

export interface ProcessList {
  total: number;
  matched: number;
  processes: ProcessInfo[];
}

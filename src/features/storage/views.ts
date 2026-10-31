import type { EntryKind, ScanEntry } from "@/types/api";

export type SortKey = "size" | "modified" | "files";
export type ViewId = "overview" | "folders" | "files" | "caches" | "cleanup";

export const DEV_KINDS: ReadonlySet<EntryKind> = new Set<EntryKind>([
  "tempCache",
  "packageCache",
  "gradleCache",
  "flutterCache",
  "nodeModules",
  "buildOutput",
  "androidSdk",
  "androidAvd",
  "dockerData",
  "wslDisk",
]);

export function sortEntries(entries: ScanEntry[], key: SortKey): ScanEntry[] {
  const by: Record<SortKey, (e: ScanEntry) => number> = {
    size: (e) => e.sizeBytes,
    modified: (e) => e.lastModifiedSecs ?? 0,
    files: (e) => e.fileCount,
  };
  const f = by[key];
  return [...entries].sort((a, b) => f(b) - f(a) || b.sizeBytes - a.sizeBytes);
}

/** Entries the user may be offered for deletion. DO_NOT_TOUCH never is. */
export function isCandidate(e: ScanEntry): boolean {
  return e.safety !== "DO_NOT_TOUCH";
}

export function viewEntries(entries: ScanEntry[], view: Exclude<ViewId, "overview">, key: SortKey): ScanEntry[] {
  const pick: Record<typeof view, (e: ScanEntry) => boolean> = {
    folders: (e) => e.isDir,
    files: (e) => !e.isDir,
    caches: (e) => DEV_KINDS.has(e.kind),
    cleanup: isCandidate,
  };
  return sortEntries(entries.filter(pick[view]), key);
}

export interface Overview {
  totalBytes: number;
  byKind: { kind: EntryKind; label: string; bytes: number; count: number }[];
  /** Space that could be recovered from SAFE_TO_CLEAN and SAFE_TO_REVIEW items. */
  recoverableBytes: number;
  cleanBytes: number;
  reviewBytes: number;
  protectedBytes: number;
}

export function overview(entries: ScanEntry[]): Overview {
  const by = new Map<EntryKind, { kind: EntryKind; label: string; bytes: number; count: number }>();
  let recoverable = 0;
  let clean = 0;
  let review = 0;
  let prot = 0;
  for (const e of entries) {
    const g = by.get(e.kind) ?? { kind: e.kind, label: e.kindLabel, bytes: 0, count: 0 };
    g.bytes += e.sizeBytes;
    g.count += 1;
    by.set(e.kind, g);
    if (e.safety === "SAFE_TO_CLEAN") {
      clean += e.sizeBytes;
      recoverable += e.sizeBytes;
    } else if (e.safety === "SAFE_TO_REVIEW") {
      recoverable += e.sizeBytes;
    } else if (e.safety === "REQUIRES_REVIEW") {
      review += e.sizeBytes;
    } else {
      prot += e.sizeBytes;
    }
  }
  const byKind = [...by.values()].sort((a, b) => b.bytes - a.bytes);
  return {
    totalBytes: byKind.reduce((s, g) => s + g.bytes, 0),
    byKind,
    recoverableBytes: recoverable,
    cleanBytes: clean,
    reviewBytes: review,
    protectedBytes: prot,
  };
}

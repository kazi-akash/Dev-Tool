import { useCallback, useEffect, useRef, useState } from "react";
import { ActionDialog } from "@/components/ActionDialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { call, errorMessage } from "@/lib/ipc";
import { formatBytes, formatDate } from "@/lib/format";
import { SAFETY_LABEL, SAFETY_TONE } from "@/lib/status";
import { cn } from "@/lib/utils";
import type { ActionRecord, ScanEntry, ScanStatus } from "@/types/api";
import { useVirtualWindow } from "@/hooks/useVirtualWindow";
import { DeepScanCard } from "./DeepScanCard";
import { isCandidate, overview, viewEntries, type SortKey, type ViewId } from "./views";

const VIEWS: { id: ViewId; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "folders", label: "Largest folders" },
  { id: "files", label: "Largest files" },
  { id: "caches", label: "Developer caches" },
  { id: "cleanup", label: "Cleanup candidates" },
];

const SORTS: { id: SortKey; label: string }[] = [
  { id: "size", label: "Size" },
  { id: "modified", label: "Modified" },
  { id: "files", label: "File count" },
];

const PAGE = 100;
const ENTRY_ROW_HEIGHT = 56;

interface Pending {
  actionId: string;
  params: Record<string, unknown>;
}

export function StoragePage() {
  const [status, setStatus] = useState<ScanStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [view, setView] = useState<ViewId>("overview");
  const [sort, setSort] = useState<SortKey>("size");
  const [shown, setShown] = useState(PAGE);
  const [pending, setPending] = useState<Pending | null>(null);
  const [removed, setRemoved] = useState<Set<string>>(new Set());
  const alive = useRef(true);

  const refresh = useCallback(async () => {
    try {
      const s = await call<ScanStatus>("get_storage_scan");
      if (alive.current) {
        setStatus(s);
        setError(s.error);
      }
      return s;
    } catch (e) {
      if (alive.current) setError(errorMessage(e));
      return null;
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void refresh();
    return () => {
      alive.current = false;
    };
  }, [refresh]);

  // Poll only while a scan is running.
  const running = status?.running ?? false;
  useEffect(() => {
    if (!running) return;
    const id = window.setInterval(() => void refresh(), 1000);
    return () => window.clearInterval(id);
  }, [running, refresh]);

  async function start() {
    setRemoved(new Set());
    setShown(PAGE);
    try {
      await call<boolean>("start_storage_scan");
    } catch (e) {
      setError(errorMessage(e));
    }
    void refresh();
  }

  async function startDeep() {
    try {
      await call<boolean>("start_deep_appdata_scan");
    } catch (e) {
      setError(errorMessage(e));
    }
    void refresh();
  }

  async function cancel() {
    await call("cancel_storage_scan").catch(() => {});
    void refresh();
  }

  function closeDialog(record: ActionRecord | null) {
    if (record?.success && pending?.params.path) {
      setRemoved((r) => new Set(r).add(String(pending.params.path)));
    }
    setPending(null);
  }

  const result = status?.result ?? null;
  const entries = (result?.entries ?? []).filter((e) => !removed.has(e.path));
  const ov = overview(entries);
  const rows = view === "overview" ? [] : viewEntries(entries, view, sort);
  const visibleRows = rows.slice(0, shown);
  // After a few "Show more" clicks the table can hold thousands of rows: render only a window of them.
  const { ref: tableRef, range } = useVirtualWindow(visibleRows.length, { rowHeight: ENTRY_ROW_HEIGHT });

  return (
    <div className="mx-auto max-w-5xl space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">Storage</h1>
          <p className="mt-1 text-sm text-muted-foreground">DevPilot only measures here. Nothing is deleted without a preview and your confirmation.</p>
        </div>
        {running ? (
          <Button variant="outline" size="sm" onClick={cancel}>
            Cancel scan
          </Button>
        ) : (
          <Button size="sm" onClick={start}>
            {result ? "Scan again" : "Scan storage"}
          </Button>
        )}
      </div>

      {error && (
        <p role="alert" className="rounded-md border border-danger/40 bg-danger/10 p-2 text-sm">
          {error}
        </p>
      )}

      {running && status && (
        <Card className="p-4 text-sm">
          <div className="font-medium">{status.progress.phase}</div>
          <div className="mt-1 truncate text-xs text-muted-foreground">{status.progress.currentPath || "Starting..."}</div>
          <div className="mt-1 text-xs text-muted-foreground tabular-nums">
            {status.progress.itemsVisited.toLocaleString()} items checked · {status.progress.entriesFound} found
          </div>
        </Card>
      )}

      {!result && !running && !error && <Card className="p-8 text-center text-sm text-muted-foreground">No scan yet. Press “Scan storage” to find caches, node_modules, build output and large files. A scan runs in the background and can be cancelled.</Card>}

      {result && (
        <>
          {result.notes.length > 0 && (
            <ul className="space-y-1 rounded-md border border-warn/40 bg-warn/10 p-2 text-xs">
              {result.notes.map((n) => (
                <li key={n}>{n}</li>
              ))}
            </ul>
          )}

          <div className="flex flex-wrap items-center justify-between gap-2">
            <div role="tablist" aria-label="Storage views" className="flex flex-wrap gap-1">
              {VIEWS.map((v) => (
                <Button
                  key={v.id}
                  role="tab"
                  aria-selected={view === v.id}
                  size="sm"
                  variant={view === v.id ? "default" : "outline"}
                  onClick={() => {
                    setView(v.id);
                    setShown(PAGE);
                  }}
                >
                  {v.label}
                </Button>
              ))}
            </div>
            {view !== "overview" && (
              <div className="flex items-center gap-1" role="group" aria-label="Sort by">
                <span className="mr-1 text-xs text-muted-foreground">Sort</span>
                {SORTS.map((s) => (
                  <Button key={s.id} size="sm" variant={sort === s.id ? "secondary" : "ghost"} aria-pressed={sort === s.id} onClick={() => setSort(s.id)}>
                    {s.label}
                  </Button>
                ))}
              </div>
            )}
          </div>

          {view === "overview" && (
            <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
              <Card className="lg:col-span-1">
                <CardHeader>
                  <CardTitle>What could be recovered</CardTitle>
                </CardHeader>
                <CardContent className="space-y-2 text-sm">
                  <Stat label="Safe to clean" value={ov.cleanBytes} tone="text-ok" />
                  <Stat label="Safe to review" value={ov.recoverableBytes - ov.cleanBytes} tone="text-primary" />
                  <Stat label="Requires review" value={ov.reviewBytes} tone="text-warn" />
                  <Stat label="Do not touch" value={ov.protectedBytes} tone="text-muted-foreground" />
                  <p className="pt-2 text-xs text-muted-foreground">
                    Scanned in {(result.durationMs / 1000).toFixed(1)} s{result.complete ? "" : " (partial)"}.
                  </p>
                </CardContent>
              </Card>
              <Card className="lg:col-span-2">
                <CardHeader>
                  <CardTitle>By category</CardTitle>
                </CardHeader>
                <CardContent>
                  <ul className="space-y-2">
                    {ov.byKind.slice(0, 12).map((g) => (
                      <li key={g.kind}>
                        <div className="flex justify-between text-sm">
                          <span>
                            {g.label} <span className="text-xs text-muted-foreground">({g.count})</span>
                          </span>
                          <span className="tabular-nums">{formatBytes(g.bytes)}</span>
                        </div>
                        <div className="mt-1 h-1.5 overflow-hidden rounded-full bg-secondary">
                          <div className="h-full rounded-full bg-primary" style={{ width: `${ov.totalBytes ? Math.max(2, (g.bytes / ov.totalBytes) * 100) : 0}%` }} />
                        </div>
                      </li>
                    ))}
                  </ul>
                </CardContent>
              </Card>
            </div>
          )}

          {view !== "overview" && (
            <Card className="overflow-hidden">
              <div ref={tableRef} className={visibleRows.length > 100 ? "max-h-[40rem] overflow-y-auto" : undefined}>
              <table className="w-full text-sm">
                <thead className="sticky top-0 z-10 border-b bg-muted text-left text-xs text-muted-foreground">
                  <tr>
                    <th className="px-4 py-2 font-medium">Item</th>
                    <th className="px-3 py-2 font-medium">Category</th>
                    <th className="px-3 py-2 text-right font-medium">Size</th>
                    <th className="px-3 py-2 text-right font-medium">Files</th>
                    <th className="px-3 py-2 font-medium">Modified</th>
                    <th className="px-3 py-2 font-medium">Safety</th>
                    <th className="px-4 py-2" />
                  </tr>
                </thead>
                <tbody>
                  {range.padTop > 0 && (
                    <tr aria-hidden style={{ height: range.padTop }}>
                      <td colSpan={7} />
                    </tr>
                  )}
                  {visibleRows.slice(range.start, range.end).map((e) => (
                    <EntryRow key={e.path} e={e} onClean={() => setPending({ actionId: "cleanup.path", params: { path: e.path } })} />
                  ))}
                  {range.padBottom > 0 && (
                    <tr aria-hidden style={{ height: range.padBottom }}>
                      <td colSpan={7} />
                    </tr>
                  )}
                  {rows.length === 0 && (
                    <tr>
                      <td colSpan={7} className="px-4 py-8 text-center text-muted-foreground">
                        Nothing found in this view.
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
              </div>
              {rows.length > shown && (
                <div className="border-t p-2 text-center">
                  <Button size="sm" variant="ghost" onClick={() => setShown((n) => n + PAGE)}>
                    Show {Math.min(PAGE, rows.length - shown)} more ({rows.length - shown} remaining)
                  </Button>
                </div>
              )}
            </Card>
          )}
        </>
      )}

      <DeepScanCard running={running && (status?.deep ?? false)} busy={running} phase={status?.progress.phase ?? ""} result={status?.deepResult ?? null} onStart={() => void startDeep()} onCancel={() => void cancel()} />

      {pending && <ActionDialog actionId={pending.actionId} params={pending.params} onClose={closeDialog} />}
    </div>
  );
}

function Stat({ label, value, tone }: { label: string; value: number; tone: string }) {
  return (
    <div className="flex justify-between">
      <span className={tone}>{label}</span>
      <span className="tabular-nums">{formatBytes(value)}</span>
    </div>
  );
}

function EntryRow({ e, onClean }: { e: ScanEntry; onClean: () => void }) {
  return (
    <tr className={cn("border-b last:border-0 hover:bg-accent/40")} style={{ height: ENTRY_ROW_HEIGHT }}>
      <td className="max-w-[22rem] px-4 py-2">
        <div className="truncate font-medium" title={e.path}>
          {e.name}
        </div>
        <div className="truncate text-xs text-muted-foreground" title={e.path}>
          {e.path}
        </div>
      </td>
      <td className="px-3 py-2 text-xs">{e.kindLabel}</td>
      <td className="px-3 py-2 text-right tabular-nums">{formatBytes(e.sizeBytes)}</td>
      <td className="px-3 py-2 text-right tabular-nums text-muted-foreground">{e.isDir ? e.fileCount.toLocaleString() : "-"}</td>
      <td className="px-3 py-2 text-xs text-muted-foreground">{formatDate(e.lastModifiedSecs)}</td>
      <td className="px-3 py-2">
        <Badge tone={SAFETY_TONE[e.safety]} title={e.reason}>
          {SAFETY_LABEL[e.safety]}
        </Badge>
      </td>
      <td className="px-4 py-2 text-right">
        {isCandidate(e) ? (
          <Button size="sm" variant="outline" onClick={onClean} aria-label={`Review and delete ${e.name}`}>
            Review
          </Button>
        ) : null}
      </td>
    </tr>
  );
}

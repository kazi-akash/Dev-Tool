import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { formatBytes } from "@/lib/format";
import type { ScanResult } from "@/types/api";

interface Props {
  /** A deep scan is the one running. */
  running: boolean;
  /** Any scan (standard or deep) is running; only one runs at a time. */
  busy: boolean;
  phase: string;
  result: ScanResult | null;
  onStart: () => void;
  onCancel: () => void;
}

/**
 * Optional, explicit, report-only. Never part of a normal scan. It asks before starting because it
 * can take several minutes, and it lists app folders without any delete button.
 */
export function DeepScanCard({ running, busy, phase, result, onStart, onCancel }: Props) {
  const [confirming, setConfirming] = useState(false);
  const paused = running && phase.startsWith("Paused");

  return (
    <Card>
      <CardHeader>
        <CardTitle>Deep scan of AppData (optional)</CardTitle>
        <CardDescription>Finds which apps keep the most data in your AppData folders. Normal scans do not look there.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3 text-sm">
        {!running && !confirming && (
          <Button size="sm" variant="outline" disabled={busy} onClick={() => setConfirming(true)}>
            Deep scan AppData
          </Button>
        )}

        {!running && confirming && (
          <div role="group" aria-label="Confirm deep scan" className="space-y-2 rounded-md border border-warn/40 bg-warn/10 p-3 text-xs">
            <p className="font-medium">This can take several minutes.</p>
            <ul className="list-disc space-y-0.5 pl-5">
              <li>It runs in the background at low priority and reads only folder sizes.</li>
              <li>It pauses while memory is critical, a performance incident is recorded, or an Android, Gradle or Flutter build runs, and continues afterwards.</li>
              <li>Windows and system folders are skipped. Nothing is deleted; results are for your information.</li>
            </ul>
            <div className="flex gap-2">
              <Button
                size="sm"
                onClick={() => {
                  setConfirming(false);
                  onStart();
                }}
              >
                Start deep scan
              </Button>
              <Button size="sm" variant="outline" onClick={() => setConfirming(false)}>
                Not now
              </Button>
            </div>
          </div>
        )}

        {running && (
          <div className="flex flex-wrap items-center gap-2">
            <Badge tone={paused ? "warn" : "info"}>{paused ? phase : "Scanning AppData"}</Badge>
            <Button size="sm" variant="outline" onClick={onCancel}>
              Cancel
            </Button>
          </div>
        )}

        {result && (
          <>
            {result.notes.map((n) => (
              <p key={n} className="text-xs text-muted-foreground">
                {n}
              </p>
            ))}
            {result.entries.length === 0 && <p className="text-muted-foreground">No app folder over 100 MB was found.</p>}
            <ul className="divide-y">
              {result.entries.slice(0, 30).map((e) => (
                <li key={e.path} className="flex flex-wrap items-baseline justify-between gap-2 py-1.5">
                  <span className="min-w-0 truncate" title={e.path}>
                    {e.name} <span className="text-xs text-muted-foreground">{e.path}</span>
                  </span>
                  <span className="shrink-0 tabular-nums">{formatBytes(e.sizeBytes)}</span>
                </li>
              ))}
            </ul>
            {result.entries.length > 0 && <p className="text-xs text-muted-foreground">To free this space, use each app's own settings (clear cache, remove old data). DevPilot does not offer to delete application data.</p>}
          </>
        )}
      </CardContent>
    </Card>
  );
}

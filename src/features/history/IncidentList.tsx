import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatBytes, formatDateTime } from "@/lib/format";
import type { IncidentSummary } from "@/types/api";

const na = (v: number | null, unit: string) => (v == null ? "unavailable" : `${v.toFixed(0)}${unit}`);

/** Recorded performance incidents: peaks, who held the memory, and a hedged diagnosis with evidence. */
export function IncidentList({ items, available }: { items: IncidentSummary[] | undefined; available: boolean }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Performance incidents</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {!available && <p className="text-sm text-muted-foreground">History is unavailable, so incidents are not recorded.</p>}
        {available && items?.length === 0 && <p className="text-sm text-muted-foreground">No incidents recorded. DevPilot records one when memory, commit or paging becomes severe.</p>}
        {items?.map((i) => (
          <article key={i.id} className="space-y-1.5 border-b pb-3 last:border-b-0 last:pb-0">
            <h3 className="text-sm font-medium">
              Incident #{String(i.id).padStart(4, "0")} <span className="text-xs font-normal text-muted-foreground">{formatDateTime(i.startMs)} · {i.durationSecs} s</span>
            </h3>
            <p className="text-xs text-muted-foreground tabular-nums">
              Peak RAM {formatBytes(i.peakRamUsedBytes)} ({i.peakRamPercent.toFixed(0)}%) · commit {na(i.peakCommitPercent, "%")} · disk {na(i.peakDiskPercent, "%")} · hard faults {na(i.peakHardFaultsPerSec, "/s")}
            </p>
            {i.contributors.length > 0 && (
              <p className="text-xs">
                Primary contributors: {i.contributors.slice(0, 3).map((c) => `${c.name} ${formatBytes(c.bytes)}`).join(", ")}
              </p>
            )}
            {i.sessions.length > 0 && <p className="text-xs">Active sessions: {i.sessions.join(", ")}</p>}
            <p className="text-sm">
              {i.diagnosis} <span className="text-xs text-muted-foreground">Confidence {Math.round(i.confidence * 100)}%.</span>
            </p>
            {i.evidence.length > 0 && (
              <ul className="list-disc pl-5 text-xs text-muted-foreground">
                {i.evidence.map((e) => (
                  <li key={e}>{e}</li>
                ))}
              </ul>
            )}
          </article>
        ))}
      </CardContent>
    </Card>
  );
}

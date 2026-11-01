import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatBytes } from "@/lib/format";
import type { MemoryTelemetry } from "@/types/system";

const KIND_LABEL = {
  NONE: "No pressure",
  HIGH_RAM_STABLE: "High but stable",
  HEAVY_PAGING: "Heavy paging",
  COMMIT_EXHAUSTION: "Commit exhaustion",
  DISK_SATURATION: "Disk saturation",
} as const;

function value(v: number | null, unit: string, digits = 0) {
  return v == null ? "unavailable" : `${v.toFixed(digits)}${unit}`;
}

/** Memory details beyond RAM %: commit, paging, hard faults, disk activity and the likely mechanism. */
export function MemoryMechanismCard({ telemetry }: { telemetry: MemoryTelemetry }) {
  const m = telemetry.mechanism;
  const pressured = m.kind !== "NONE";
  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="text-muted-foreground">Memory details</CardTitle>
        {pressured && <Badge tone={m.kind === "HIGH_RAM_STABLE" ? "ok" : "danger"}>{KIND_LABEL[m.kind]}</Badge>}
      </CardHeader>
      <CardContent className="space-y-3">
        <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm tabular-nums sm:grid-cols-4">
          <div>
            <dt className="text-xs text-muted-foreground">Commit</dt>
            <dd>
              {value(telemetry.commitPercent, "%")}
              {telemetry.commitBytes != null && telemetry.commitLimitBytes != null && (
                <span className="block text-xs text-muted-foreground">
                  {formatBytes(telemetry.commitBytes)} / {formatBytes(telemetry.commitLimitBytes)}
                </span>
              )}
            </dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Pagefile</dt>
            <dd>{telemetry.pagefileUsagePercent != null ? `${telemetry.pagefileUsagePercent.toFixed(0)}% used` : value(telemetry.pagefileEstimatePercent, "% (estimate)")}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Hard faults</dt>
            <dd>{value(telemetry.hardFaultsPerSec, " pages/s")}</dd>
          </div>
          <div>
            <dt className="text-xs text-muted-foreground">Disk active</dt>
            <dd>{value(telemetry.diskActivePercent, "%")}</dd>
          </div>
        </dl>
        {pressured && (
          <p className="text-sm">
            {m.summary} <span className="text-xs text-muted-foreground">Confidence {Math.round(m.confidence * 100)}%.</span>
          </p>
        )}
      </CardContent>
    </Card>
  );
}

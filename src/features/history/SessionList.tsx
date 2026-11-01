import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatBytes, formatDateTime } from "@/lib/format";
import type { BuildSession } from "@/types/api";

const signed = (n: number) => `${n >= 0 ? "+" : "-"}${formatBytes(Math.abs(n))}`;

/** Developer build sessions: what ran, memory before/peak, and a carefully worded incident correlation. */
export function SessionList({ items, available }: { items: BuildSession[] | undefined; available: boolean }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Build sessions</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {!available && <p className="text-sm text-muted-foreground">History is unavailable, so sessions are not recorded.</p>}
        {available && items?.length === 0 && <p className="text-sm text-muted-foreground">No builds recorded. DevPilot notes Gradle, Flutter, Next.js and npm/pnpm builds, Docker compose startups and emulator startups.</p>}
        {items?.map((s) => (
          <article key={s.id} className="space-y-1.5 border-b pb-3 last:border-b-0 last:pb-0">
            <h3 className="flex flex-wrap items-center gap-2 text-sm font-medium">
              {s.label}
              <span className="text-xs font-normal text-muted-foreground">
                {formatDateTime(s.startMs)} · {s.durationSecs} s
              </span>
              <Badge tone={s.result === "NORMAL" ? "ok" : s.result.startsWith("CRITICAL") ? "danger" : "warn"}>{s.result}</Badge>
            </h3>
            <p className="text-xs text-muted-foreground tabular-nums">
              RAM before {formatBytes(s.ramBeforeBytes)} · peak {formatBytes(s.ramPeakBytes)} ({s.ramPeakPercent.toFixed(0)}%) · CPU peak {s.cpuPeakPercent.toFixed(0)}% · disk peak{" "}
              {s.diskPeakPercent == null ? "unavailable" : `${s.diskPeakPercent.toFixed(0)}%`} · {s.processesCreated} new processes
            </p>
            {(s.workloadGrowth.length > 0 || s.dockerChangeBytes !== 0) && (
              <p className="text-xs">
                {s.workloadGrowth
                  .slice(0, 3)
                  .map((g) => `${g.name} +${formatBytes(g.bytes)}`)
                  .join(", ")}
                {s.dockerChangeBytes !== 0 && `${s.workloadGrowth.length > 0 ? " · " : ""}Docker/WSL2 host memory ${signed(s.dockerChangeBytes)}`}
              </p>
            )}
            {s.correlation && s.correlation.level !== "NONE" && <p className="text-sm">{s.correlation.statement}</p>}
          </article>
        ))}
      </CardContent>
    </Card>
  );
}

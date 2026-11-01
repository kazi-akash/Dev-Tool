import { useState } from "react";
import { ActionDialog } from "@/components/ActionDialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useAsync } from "@/hooks/useAsync";
import { formatBytes } from "@/lib/format";
import { call } from "@/lib/ipc";
import type { DockerBreakdown, DockerStatus } from "@/types/api";

/** Containers and what they hold. Docker is optional: when it is missing or stopped this just says why. */
export function DockerPanel() {
  const docker = useAsync(() => call<DockerStatus>("get_docker_status"), { intervalMs: 15_000 });
  const breakdown = useAsync(() => call<DockerBreakdown | null>("get_docker_breakdown"), { intervalMs: 15_000 });
  const [stopName, setStopName] = useState<string | null>(null);
  const s = docker.data;

  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between space-y-0">
        <CardTitle>Docker</CardTitle>
        <Button size="sm" variant="ghost" onClick={() => void docker.reload()} disabled={docker.loading}>
          Refresh
        </Button>
      </CardHeader>
      <CardContent>
        {!s && !docker.error && <p className="text-sm text-muted-foreground">Checking Docker...</p>}
        {!s && docker.error && <p className="text-sm text-muted-foreground">Docker details unavailable: {docker.error}</p>}
        {s && !s.available && <p className="text-sm text-muted-foreground">Docker details unavailable: {s.error ?? "unknown reason"}. Everything else keeps working.</p>}
        {breakdown.data && <BreakdownSummary b={breakdown.data} />}
        {s?.available && s.containers.length === 0 && <p className="text-sm text-muted-foreground">No containers.</p>}
        {s?.available && s.containers.length > 0 && (
          <ul className="divide-y">
            {s.containers.map((c) => (
              <li key={c.id} className="flex flex-wrap items-center gap-2 py-1.5 text-sm">
                <Badge tone={c.running ? "ok" : "neutral"}>{c.running ? "running" : "stopped"}</Badge>
                <span className="font-medium">{c.name}</span>
                <span className="text-xs text-muted-foreground">{c.image}</span>
                <span className="ml-auto text-xs tabular-nums text-muted-foreground">
                  {c.memoryBytes != null ? formatBytes(c.memoryBytes) : ""}
                  {c.cpuPercent != null ? ` · ${c.cpuPercent.toFixed(1)}% CPU` : ""}
                </span>
                {c.running && (
                  <Button size="sm" variant="outline" aria-label={`Stop ${c.name}`} onClick={() => setStopName(c.name)}>
                    Stop
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}
      </CardContent>
      {stopName && (
        <ActionDialog
          actionId="docker.stop_container"
          params={{ container: stopName }}
          onClose={() => {
            setStopName(null);
            void docker.reload();
          }}
        />
      )}
    </Card>
  );
}

/** Host memory first (what reduces free RAM); container figures are inside it and are never added. */
function BreakdownSummary({ b }: { b: DockerBreakdown }) {
  return (
    <div className="mb-3 space-y-1 border-b pb-3 text-sm">
      <p>
        <span className="font-medium">Docker / WSL2 host memory: {formatBytes(b.hostTotalBytes)}</span>
        <span className="text-xs text-muted-foreground">
          {" "}
          ({b.wslVmBytes != null ? `WSL2 VM ${formatBytes(b.wslVmBytes)} · ` : ""}Docker Desktop/backend {formatBytes(b.desktopBackendBytes)})
        </span>
      </p>
      {b.containersReportedBytes != null && (
        <p className="text-xs text-muted-foreground">
          Containers report {formatBytes(b.containersReportedBytes)} inside the VM (included above, not added)
          {b.unattributedVmBytes != null ? ` · ${formatBytes(b.unattributedVmBytes)} of the VM is not attributable to a container` : ""}.
        </p>
      )}
      {b.containers.length > 0 && (
        <ul className="text-xs tabular-nums">
          {b.containers.map((c) => (
            <li key={c.id} className="flex gap-2">
              <span className="min-w-0 flex-1 truncate">{c.name}</span>
              <span>{c.memoryBytes != null ? formatBytes(c.memoryBytes) : "memory unavailable"}</span>
              <span className="text-muted-foreground">{c.memoryLimitBytes != null ? `limit ${formatBytes(c.memoryLimitBytes)}` : "no limit set"}</span>
            </li>
          ))}
        </ul>
      )}
      {b.notes.map((n) => (
        <p key={n} className="text-xs text-muted-foreground">
          {n}
        </p>
      ))}
    </div>
  );
}

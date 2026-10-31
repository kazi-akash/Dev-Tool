import { useState } from "react";
import { ActionDialog } from "@/components/ActionDialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useAsync } from "@/hooks/useAsync";
import { formatBytes } from "@/lib/format";
import { call } from "@/lib/ipc";
import type { DockerStatus } from "@/types/api";

/** Containers and what they hold. Docker is optional: when it is missing or stopped this just says why. */
export function DockerPanel() {
  const docker = useAsync(() => call<DockerStatus>("get_docker_status"), { intervalMs: 15_000 });
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

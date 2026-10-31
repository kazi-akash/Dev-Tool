import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatBytes } from "@/lib/format";
import type { Workload } from "@/types/api";

interface Props {
  workloads: Workload[] | null;
  totalRamBytes: number;
}

/** Developer workloads, each with the processes that make it up (Docker: Desktop, backend, VM...). */
export function WorkloadList({ workloads, totalRamBytes }: Props) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Developer workloads</CardTitle>
      </CardHeader>
      <CardContent>
        {!workloads && <p className="text-sm text-muted-foreground">Looking for workloads...</p>}
        {workloads?.length === 0 && <p className="text-sm text-muted-foreground">No Docker, Node, Android, Laragon or IDE processes running.</p>}
        <ul className="space-y-3">
          {workloads?.slice(0, 6).map((w) => (
            <li key={w.kind}>
              <div className="flex items-baseline justify-between gap-2">
                <span className="text-sm font-medium">{w.name}</span>
                <span className="text-sm tabular-nums">
                  {formatBytes(w.memoryBytes)}
                  <span className="ml-1.5 text-xs text-muted-foreground">{totalRamBytes ? `${Math.round((w.memoryBytes / totalRamBytes) * 100)}%` : ""}</span>
                </span>
              </div>
              <ul className="mt-1 space-y-0.5 border-l pl-3">
                {w.members.slice(0, 4).map((m) => (
                  <li key={m.label} className="flex justify-between gap-2 text-xs text-muted-foreground">
                    <span>
                      {m.label}
                      {m.processCount > 1 ? ` ×${m.processCount}` : ""}
                    </span>
                    <span className="tabular-nums">{formatBytes(m.memoryBytes)}</span>
                  </li>
                ))}
              </ul>
            </li>
          ))}
        </ul>
      </CardContent>
    </Card>
  );
}

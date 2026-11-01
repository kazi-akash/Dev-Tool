import { Badge, type Tone } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useAsync } from "@/hooks/useAsync";
import { formatBytes, formatPercent } from "@/lib/format";
import { call } from "@/lib/ipc";
import type { QuickStatus } from "@/types/system";

const SEVERITY: Record<QuickStatus["severity"], { label: string; tone: Tone }> = {
  HEALTHY: { label: "Healthy", tone: "ok" },
  ATTENTION: { label: "Attention", tone: "warn" },
  HIGH: { label: "High", tone: "danger" },
  CRITICAL: { label: "Critical", tone: "danger" },
};

/** The five buttons. The backend accepts exactly these ids and nothing else. */
const BUTTONS = [
  { id: "open", label: "Open DevPilot" },
  { id: "diagnosis", label: "Run Diagnosis" },
  { id: "scan_storage", label: "Scan Storage" },
  { id: "settings", label: "Settings" },
  { id: "quit", label: "Quit" },
] as const;

/**
 * The tray popup. One cheap command, refreshed every two seconds; it adds no monitoring of its
 * own. It has no sidebar and loads nothing else, so it opens instantly and is destroyed on blur.
 */
export function QuickStatusPage() {
  const status = useAsync(() => call<QuickStatus | null>("get_quick_status"), { intervalMs: 2_000 });
  const s = status.data;

  return (
    <main className="flex h-full flex-col gap-3 bg-background p-4 text-sm text-foreground">
      <header className="flex items-center justify-between">
        <h1 className="font-semibold">DevPilot</h1>
        {s && <Badge tone={SEVERITY[s.severity].tone}>{SEVERITY[s.severity].label}</Badge>}
      </header>

      {!s && <p className="text-muted-foreground">{status.error ? "Status unavailable." : "Waiting for the first sample..."}</p>}

      {s && (
        <>
          <dl className="grid grid-cols-3 gap-2 tabular-nums">
            <div>
              <dt className="text-xs text-muted-foreground">RAM</dt>
              <dd className="text-base font-semibold">{formatPercent(s.ramPercent)}</dd>
              <dd className="text-xs text-muted-foreground">
                {formatBytes(s.ramUsedBytes)} / {formatBytes(s.ramTotalBytes)}
              </dd>
            </div>
            <div>
              <dt className="text-xs text-muted-foreground">CPU</dt>
              <dd className="text-base font-semibold">{formatPercent(s.cpuPercent)}</dd>
            </div>
            <div>
              <dt className="text-xs text-muted-foreground">Disk {s.disk?.mountPoint ?? ""}</dt>
              <dd className="text-base font-semibold">{s.disk ? `${formatBytes(s.disk.freeBytes)} free` : "n/a"}</dd>
            </div>
          </dl>

          <p>
            {s.headline}
            {s.incidentActive && <span className="ml-1 font-medium text-danger">An incident is being recorded.</span>}
          </p>
        </>
      )}

      <nav aria-label="Actions" className="mt-auto grid grid-cols-2 gap-2">
        {BUTTONS.map((b) => (
          <Button key={b.id} size="sm" variant={b.id === "quit" ? "outline" : "default"} onClick={() => void call("quick_status_action", { action: b.id })}>
            {b.label}
          </Button>
        ))}
      </nav>
    </main>
  );
}

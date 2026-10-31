import { useSystemMetrics } from "@/hooks/useSystemMetrics";
import { formatBytes, formatPercent, formatUptime } from "@/lib/format";
import { MetricCard, type Level } from "./MetricCard";

// Provisional UI thresholds. The Rust pressure/disk state engines (Phases 5 and 7) replace these.
function usageLevel(percent: number): Level {
  if (percent >= 85) return "danger";
  if (percent >= 70) return "warn";
  return "ok";
}

export function DashboardPage() {
  const snap = useSystemMetrics();

  return (
    <div className="mx-auto max-w-4xl space-y-4">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Dashboard</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          {snap ? "Live system metrics." : "Waiting for the first sample..."}
        </p>
      </div>

      {snap && (
        <>
          <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
            <MetricCard
              title="CPU"
              value={formatPercent(snap.cpuPercent)}
              percent={snap.cpuPercent}
              level={usageLevel(snap.cpuPercent)}
            />
            <MetricCard
              title="RAM"
              value={formatPercent(snap.memory.percent)}
              detail={`${formatBytes(snap.memory.usedBytes)} / ${formatBytes(snap.memory.totalBytes)} (${formatBytes(snap.memory.availableBytes)} available)`}
              percent={snap.memory.percent}
              level={usageLevel(snap.memory.percent)}
            />
            <MetricCard
              title="System"
              value={`${snap.processCount} processes`}
              detail={`Uptime ${formatUptime(snap.uptimeSecs)}`}
            />
          </div>

          <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
            {snap.disks.map((d) => (
              <MetricCard
                key={d.mountPoint}
                title={`Disk ${d.mountPoint}`}
                value={`${formatBytes(d.freeBytes)} free`}
                detail={`${formatBytes(d.usedBytes)} used of ${formatBytes(d.totalBytes)}`}
                percent={d.usedPercent}
                level={usageLevel(d.usedPercent)}
              />
            ))}
          </div>
        </>
      )}
    </div>
  );
}

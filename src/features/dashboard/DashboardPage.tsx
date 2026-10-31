import { Link } from "react-router-dom";
import { Badge, TONE_TEXT } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useAsync } from "@/hooks/useAsync";
import { useProcesses } from "@/hooks/useProcesses";
import { useSystemMetrics } from "@/hooks/useSystemMetrics";
import { formatBytes, formatHours, formatPercent, formatUptime, timeAgo } from "@/lib/format";
import { call } from "@/lib/ipc";
import { DISK_TONE, PRESSURE_TONE, SEVERITY_TONE, usageTone } from "@/lib/status";
import type { DiagnosisReport, GrowthFinding, HistoryResponse, StoredDiagnosis, Workload } from "@/types/api";
import { buildAlerts } from "./alerts";
import { HealthBanner } from "./HealthBanner";
import { MetricCard } from "./MetricCard";
import { WorkloadList } from "./WorkloadList";

const TREND_TEXT = { FALLING: "falling", STABLE: "stable", RISING: "rising", ESCALATING: "rising fast" } as const;

export function DashboardPage() {
  const snap = useSystemMetrics();
  const diagnosis = useAsync(() => call<DiagnosisReport>("run_diagnosis"), { intervalMs: 20_000 });
  const workloads = useAsync(() => call<Workload[]>("get_workloads"), { intervalMs: 4_000 });
  const growth = useAsync(() => call<GrowthFinding[]>("get_memory_growth"), { intervalMs: 30_000 });
  const recent = useAsync(() => call<HistoryResponse<StoredDiagnosis>>("get_recent_diagnoses", { limit: 5 }), { intervalMs: 60_000 });
  const top = useProcesses({ limit: 5, sortBy: "memory" });

  const alerts = buildAlerts(snap, growth.data);
  const p = snap?.pressure;

  return (
    <div className="mx-auto max-w-5xl space-y-4">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Dashboard</h1>
        <p className="mt-1 text-sm text-muted-foreground">{snap ? `${snap.processCount} processes · up ${formatUptime(snap.uptimeSecs)}` : "Waiting for the first sample..."}</p>
      </div>

      <HealthBanner report={diagnosis.data} loading={diagnosis.loading} error={diagnosis.error} onAnalyze={() => void diagnosis.reload()} />

      {snap && (
        <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <MetricCard title="CPU" value={formatPercent(snap.cpuPercent)} percent={snap.cpuPercent} tone={usageTone(snap.cpuPercent)} />
          <MetricCard
            title="RAM"
            value={formatPercent(snap.memory.percent)}
            detail={`${formatBytes(snap.memory.usedBytes)} / ${formatBytes(snap.memory.totalBytes)} · ${formatBytes(snap.memory.availableBytes)} available`}
            percent={snap.memory.percent}
            tone={p ? PRESSURE_TONE[p.state] : usageTone(snap.memory.percent)}
            badge={p?.state}
            footer={p && p.state !== "NORMAL" ? `Usage is ${TREND_TEXT[p.trend]}${p.minutesToCritical != null && p.minutesToCritical < 120 ? `, critical in ~${formatHours(p.minutesToCritical / 60)}` : ""}` : undefined}
          />
          {snap.disks.map((d) => {
            const h = snap.diskHealth.find((x) => x.mountPoint === d.mountPoint);
            return (
              <MetricCard
                key={d.mountPoint}
                title={`Disk ${d.mountPoint}`}
                value={`${formatBytes(d.freeBytes)} free`}
                detail={`${formatBytes(d.usedBytes)} used of ${formatBytes(d.totalBytes)}`}
                percent={d.usedPercent}
                tone={h ? DISK_TONE[h.state] : usageTone(d.usedPercent)}
                badge={h && h.state !== "NORMAL" ? h.state : undefined}
                footer={h?.hoursToFull != null ? `Full in about ${formatHours(h.hoursToFull)} at the current rate` : undefined}
              />
            );
          })}
        </div>
      )}

      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <WorkloadList workloads={workloads.data} totalRamBytes={snap?.memory.totalBytes ?? 0} />

        <div className="space-y-4">
          <Card>
            <CardHeader className="flex-row items-center justify-between space-y-0">
              <CardTitle>Top processes</CardTitle>
              <Link to="/processes" className="text-xs text-primary hover:underline">
                All processes
              </Link>
            </CardHeader>
            <CardContent>
              {!top && <p className="text-sm text-muted-foreground">Loading...</p>}
              <ul className="space-y-1.5">
                {top?.processes.map((proc) => (
                  <li key={`${proc.pid}-${proc.startTime}`} className="flex items-center justify-between gap-2 text-sm">
                    <span className="min-w-0 truncate">
                      {proc.name} <Badge className="ml-1 align-middle">{proc.category}</Badge>
                    </span>
                    <span className="shrink-0 tabular-nums">{formatBytes(proc.memoryBytes)}</span>
                  </li>
                ))}
              </ul>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>Current alerts</CardTitle>
            </CardHeader>
            <CardContent>
              {alerts.length === 0 && <p className="text-sm text-muted-foreground">No alerts. Memory and disks are within thresholds.</p>}
              <ul className="space-y-1.5">
                {alerts.map((a) => (
                  <li key={a.id} className={`text-sm ${TONE_TEXT[a.tone]}`}>
                    {a.text}
                  </li>
                ))}
              </ul>
            </CardContent>
          </Card>
        </div>
      </div>

      <Card>
        <CardHeader className="flex-row items-center justify-between space-y-0">
          <CardTitle>Recent diagnoses</CardTitle>
          <Link to="/history" className="text-xs text-primary hover:underline">
            History
          </Link>
        </CardHeader>
        <CardContent>
          {recent.data && !recent.data.available && <p className="text-sm text-muted-foreground">History is unavailable (the local database could not be opened).</p>}
          {recent.data?.available && recent.data.items.length === 0 && <p className="text-sm text-muted-foreground">Nothing recorded yet. Diagnoses are saved in the background every couple of minutes.</p>}
          <ul className="space-y-2">
            {recent.data?.items.map((d) => (
              <li key={d.id} className="text-sm">
                <div className="flex items-center gap-2">
                  <Badge tone={SEVERITY_TONE[d.severity]}>{d.severity}</Badge>
                  <span className="font-medium">{d.title}</span>
                  <span className="ml-auto text-xs text-muted-foreground">{timeAgo(d.tsMs)}</span>
                </div>
                <p className="mt-0.5 text-xs text-muted-foreground">{d.summary}</p>
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>
    </div>
  );
}

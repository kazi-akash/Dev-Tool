import { useMemo, useState } from "react";
import { Area, AreaChart, CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useAsync } from "@/hooks/useAsync";
import { call } from "@/lib/ipc";
import { formatBytes, formatDateTime, timeAgo } from "@/lib/format";
import { SEVERITY_TONE } from "@/lib/status";
import type { HistoryResponse, HistoryRow, IncidentSummary, StoredAction, StoredDiagnosis } from "@/types/api";
import { IncidentList } from "./IncidentList";
import { chartData, RANGES, tickLabel } from "./series";

const AXIS = { fontSize: 11, fill: "var(--muted-foreground)" };
const TIP = { background: "var(--card)", border: "1px solid var(--border)", borderRadius: 6, fontSize: 12 };

export default function HistoryPage() {
  const [range, setRange] = useState<(typeof RANGES)[number]>(RANGES[1]);
  const hist = useAsync(() => call<HistoryResponse<HistoryRow>>("get_history", { rangeSecs: range.secs }), { intervalMs: 30_000, deps: [range.secs] });
  const diags = useAsync(() => call<HistoryResponse<StoredDiagnosis>>("get_recent_diagnoses", { limit: 15 }), { intervalMs: 60_000 });
  const acts = useAsync(() => call<HistoryResponse<StoredAction>>("get_recent_actions", { limit: 15 }), { intervalMs: 60_000 });
  const incidents = useAsync(() => call<HistoryResponse<IncidentSummary>>("get_incidents", { limit: 10 }), { intervalMs: 60_000 });

  const rows = hist.data?.items ?? [];
  const { points, drives } = useMemo(() => chartData(rows), [rows]);
  const label = (ms: number) => tickLabel(ms, range.secs);

  return (
    <div className="mx-auto max-w-5xl space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">History</h1>
          <p className="mt-1 text-sm text-muted-foreground">Recent samples are kept at full detail, then averaged into 1 minute, 5 minute and 1 hour buckets.</p>
        </div>
        <div className="flex gap-1" role="group" aria-label="Time range">
          {RANGES.map((r) => (
            <Button key={r.label} size="sm" variant={r.secs === range.secs ? "default" : "outline"} aria-pressed={r.secs === range.secs} onClick={() => setRange(r)}>
              {r.label}
            </Button>
          ))}
        </div>
      </div>

      {hist.data && !hist.data.available && <p className="rounded-md border border-warn/40 bg-warn/10 p-2 text-sm">History is unavailable because the local database could not be opened. Live monitoring still works.</p>}
      {hist.data?.available && points.length < 2 && <p className="text-sm text-muted-foreground">Not enough data for this range yet. DevPilot records in the background while it runs, including from the tray.</p>}

      {points.length >= 2 && (
        <>
          <Card>
            <CardHeader>
              <CardTitle>RAM usage (%)</CardTitle>
            </CardHeader>
            <CardContent className="h-56">
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart data={points} margin={{ left: 0, right: 8, top: 4, bottom: 0 }}>
                  <CartesianGrid stroke="var(--border)" strokeDasharray="3 3" />
                  <XAxis dataKey="t" tickFormatter={label} tick={AXIS} minTickGap={40} />
                  <YAxis domain={[0, 100]} tick={AXIS} width={34} />
                  <Tooltip contentStyle={TIP} labelFormatter={(v) => formatDateTime(Number(v))} formatter={(v, n) => [`${Number(v).toFixed(1)}%`, n === "ramAvg" ? "Average" : "Peak"]} />
                  <Area type="monotone" dataKey="ramMax" stroke="var(--danger)" fill="var(--danger)" fillOpacity={0.08} strokeWidth={1} dot={false} isAnimationActive={false} />
                  <Area type="monotone" dataKey="ramAvg" stroke="var(--primary)" fill="var(--primary)" fillOpacity={0.2} strokeWidth={2} dot={false} isAnimationActive={false} />
                </AreaChart>
              </ResponsiveContainer>
            </CardContent>
          </Card>

          <Card>
            <CardHeader>
              <CardTitle>CPU usage (%)</CardTitle>
            </CardHeader>
            <CardContent className="h-44">
              <ResponsiveContainer width="100%" height="100%">
                <LineChart data={points} margin={{ left: 0, right: 8, top: 4, bottom: 0 }}>
                  <CartesianGrid stroke="var(--border)" strokeDasharray="3 3" />
                  <XAxis dataKey="t" tickFormatter={label} tick={AXIS} minTickGap={40} />
                  <YAxis domain={[0, 100]} tick={AXIS} width={34} />
                  <Tooltip contentStyle={TIP} labelFormatter={(v) => formatDateTime(Number(v))} formatter={(v, n) => [`${Number(v).toFixed(1)}%`, n === "cpuAvg" ? "Average" : "Peak"]} />
                  <Line type="monotone" dataKey="cpuMax" stroke="var(--warn)" strokeWidth={1} dot={false} isAnimationActive={false} />
                  <Line type="monotone" dataKey="cpuAvg" stroke="var(--primary)" strokeWidth={2} dot={false} isAnimationActive={false} />
                </LineChart>
              </ResponsiveContainer>
            </CardContent>
          </Card>

          {drives.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle>Free disk space (GB)</CardTitle>
              </CardHeader>
              <CardContent className="h-48">
                <ResponsiveContainer width="100%" height="100%">
                  <LineChart data={points} margin={{ left: 0, right: 8, top: 4, bottom: 0 }}>
                    <CartesianGrid stroke="var(--border)" strokeDasharray="3 3" />
                    <XAxis dataKey="t" tickFormatter={label} tick={AXIS} minTickGap={40} />
                    <YAxis tick={AXIS} width={40} />
                    <Tooltip contentStyle={TIP} labelFormatter={(v) => formatDateTime(Number(v))} formatter={(v, n) => [`${Number(v).toFixed(1)} GB`, String(n)]} />
                    {drives.map((d, i) => (
                      <Line key={d} type="monotone" dataKey={`free:${d}`} name={d} stroke={i === 0 ? "var(--primary)" : "var(--ok)"} strokeWidth={2} dot={false} isAnimationActive={false} connectNulls />
                    ))}
                  </LineChart>
                </ResponsiveContainer>
              </CardContent>
            </Card>
          )}
        </>
      )}

      <IncidentList items={incidents.data?.items} available={incidents.data?.available ?? true} />

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Past diagnoses</CardTitle>
          </CardHeader>
          <CardContent>
            {diags.data?.items.length === 0 && <p className="text-sm text-muted-foreground">None recorded yet.</p>}
            <ul className="space-y-2">
              {diags.data?.items.map((d) => (
                <li key={d.id} className="text-sm">
                  <div className="flex items-center gap-2">
                    <Badge tone={SEVERITY_TONE[d.severity]}>{d.severity}</Badge>
                    <span className="font-medium">{d.title}</span>
                    <span className="ml-auto shrink-0 text-xs text-muted-foreground">{timeAgo(d.tsMs)}</span>
                  </div>
                  <p className="mt-0.5 text-xs text-muted-foreground">{d.summary}</p>
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>Past actions</CardTitle>
          </CardHeader>
          <CardContent>
            {acts.data?.items.length === 0 && <p className="text-sm text-muted-foreground">No actions have been run.</p>}
            <ul className="space-y-2">
              {acts.data?.items.map((a) => (
                <li key={a.id} className="text-sm">
                  <div className="flex items-center gap-2">
                    <Badge tone={a.success ? "ok" : "danger"}>{a.success ? "done" : "failed"}</Badge>
                    <span className="font-medium">{a.name}</span>
                    {a.automated && <Badge>auto</Badge>}
                    <span className="ml-auto shrink-0 text-xs text-muted-foreground">{timeAgo(a.tsMs)}</span>
                  </div>
                  <p className="mt-0.5 text-xs text-muted-foreground">
                    {a.summary}
                    {a.freedBytes ? ` (${formatBytes(a.freedBytes)} freed)` : ""}
                  </p>
                </li>
              ))}
            </ul>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

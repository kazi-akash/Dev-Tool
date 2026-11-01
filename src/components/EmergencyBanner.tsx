import { useState } from "react";
import { ActionDialog } from "@/components/ActionDialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useAsync } from "@/hooks/useAsync";
import { useSystemMetrics } from "@/hooks/useSystemMetrics";
import { formatBytes } from "@/lib/format";
import { call } from "@/lib/ipc";
import type { EmergencyReport, RecoveryStep } from "@/types/api";

const na = (v: number | null, unit: string) => (v == null ? "unavailable" : `${v.toFixed(0)}${unit}`);

/**
 * Shown on every page while memory pressure is CRITICAL. It only lists steps; each button opens the
 * normal action dialog (preview, confirmation, verification). Nothing runs from here.
 */
export function EmergencyBanner() {
  const snap = useSystemMetrics();
  const critical = snap?.pressure?.state === "CRITICAL";
  // Only ask the backend (and so Docker) while pressure is actually critical.
  const report = useAsync(() => (critical ? call<EmergencyReport | null>("get_emergency_status") : Promise.resolve(null)), { intervalMs: 10_000, deps: [critical] });
  const [step, setStep] = useState<RecoveryStep | null>(null);
  const r = critical ? report.data : null;
  if (!r) return null;

  return (
    <section role="alert" aria-label="Critical system pressure" className="mb-4 rounded-lg border-2 border-danger/60 bg-danger/10 p-4">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="text-base font-semibold text-danger">CRITICAL SYSTEM PRESSURE</h2>
        {r.incidentActive && <Badge tone="danger">recording incident</Badge>}
        {r.activeSessions.map((s) => (
          <Badge key={s} tone="info">
            {s}
          </Badge>
        ))}
      </div>

      <dl className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1 text-sm tabular-nums sm:grid-cols-4">
        <div>
          <dt className="text-xs text-muted-foreground">RAM</dt>
          <dd>
            {formatBytes(r.ramUsedBytes)} / {formatBytes(r.ramTotalBytes)}
          </dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Commit</dt>
          <dd>{na(r.commitPercent, "%")}</dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Disk activity</dt>
          <dd>{na(r.diskActivePercent, "%")}</dd>
        </div>
        <div>
          <dt className="text-xs text-muted-foreground">Hard faults</dt>
          <dd>{na(r.hardFaultsPerSec, " pages/s")}</dd>
        </div>
      </dl>

      {r.mechanism && <p className="mt-2 text-sm">{r.mechanism}</p>}
      {r.causeSummary && (
        <p className="mt-1 text-sm">
          <span className="font-medium">Likely contributors:</span> {r.causeSummary} ({r.causes.map((c) => `${c.name} ${formatBytes(c.bytes)}`).join(", ")})
        </p>
      )}

      <h3 className="mt-3 text-sm font-medium">Recommended recovery</h3>
      {r.steps.length === 0 && <p className="text-sm">{r.recoveryNote}</p>}
      {r.steps.length > 0 && (
        <>
          <ul className="mt-1 space-y-1.5">
            {r.steps.map((s) => (
              <li key={s.id} className="flex flex-wrap items-center gap-2 text-sm">
                <Badge tone={s.risk === "LOW" ? "ok" : "warn"}>{s.risk}</Badge>
                <span className="font-medium">{s.title}</span>
                <span className="text-xs text-muted-foreground">{s.why}</span>
                {s.estimatedBytes != null && (
                  <span className="text-xs tabular-nums" title={s.estimateBasis}>
                    up to {formatBytes(s.estimatedBytes)}
                  </span>
                )}
                <Button size="sm" variant="outline" className="ml-auto" aria-label={`Review: ${s.title}`} onClick={() => setStep(s)}>
                  Review
                </Button>
              </li>
            ))}
          </ul>
          <p className="mt-2 text-xs text-muted-foreground">
            {r.estimatedRecoveryBytes != null && (
              <>
                Estimated recovery if all are done: up to <span className="font-medium text-foreground">{formatBytes(r.estimatedRecoveryBytes)}</span>.{" "}
              </>
            )}
            {r.recoveryNote}
          </p>
        </>
      )}

      {step && (
        <ActionDialog
          actionId={step.action.actionId}
          params={step.action.params}
          onClose={() => {
            setStep(null);
            void report.reload();
          }}
        />
      )}
    </section>
  );
}

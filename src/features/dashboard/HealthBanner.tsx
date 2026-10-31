import { Link } from "react-router-dom";
import { Badge, TONE_BAR, type Tone } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { SEVERITY_TONE } from "@/lib/status";
import type { DiagnosisReport } from "@/types/api";

const STATUS: Record<DiagnosisReport["status"], { label: string; tone: Tone }> = {
  healthy: { label: "Healthy", tone: "ok" },
  attention: { label: "Needs attention", tone: "warn" },
  problem: { label: "Problem detected", tone: "danger" },
};

interface Props {
  report: DiagnosisReport | null;
  loading: boolean;
  error: string | null;
  onAnalyze: () => void;
}

/** The answer to "is my PC healthy, and why not?" in one glance. */
export function HealthBanner({ report, loading, error, onAnalyze }: Props) {
  const status = report ? STATUS[report.status] : null;
  const h = report?.headline;
  const actionable = report ? report.findings.some((f) => f.recommendations.some((r) => r.action)) : false;

  return (
    <Card className="p-4">
      <div className="flex flex-wrap items-start justify-between gap-4">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            <span className={`inline-block size-2.5 rounded-full ${status ? TONE_BAR[status.tone] : "bg-muted-foreground"}`} aria-hidden />
            System health
          </div>
          {!report && !error && <p className="mt-2 text-sm text-muted-foreground">{loading ? "Analyzing your system..." : "Waiting for the first analysis..."}</p>}
          {error && !report && <p className="mt-2 text-sm text-muted-foreground">{error}</p>}
          {report && h && status && (
            <>
              <h2 className="mt-1 flex flex-wrap items-center gap-2 text-lg font-semibold tracking-tight">
                {status.label}
                {h.ruleId !== "healthy" && <Badge tone={SEVERITY_TONE[h.severity]}>{h.severity}</Badge>}
              </h2>
              <p className="mt-1 text-sm">{h.ruleId === "healthy" ? h.summary : h.title}</p>
              {h.ruleId !== "healthy" && <p className="mt-1 text-sm text-muted-foreground">{h.summary}</p>}
              {h.primaryCause && (
                <p className="mt-2 text-xs text-muted-foreground">
                  Likely cause: <span className="font-medium text-foreground">{h.primaryCause}</span> · confidence {Math.round(h.confidence * 100)}%
                </p>
              )}
            </>
          )}
        </div>
        <div className="flex shrink-0 gap-2">
          <Button size="sm" variant="outline" onClick={onAnalyze} disabled={loading}>
            {loading ? "Analyzing..." : "Analyze"}
          </Button>
          <Button size="sm" asChild disabled={!actionable}>
            <Link to="/diagnosis" aria-disabled={!actionable} className={actionable ? "" : "pointer-events-none opacity-50"}>
              Optimize
            </Link>
          </Button>
        </div>
      </div>
    </Card>
  );
}

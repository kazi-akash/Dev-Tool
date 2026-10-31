import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { formatBytes } from "@/lib/format";
import { SEVERITY_TONE } from "@/lib/status";
import type { ActionRef, Diagnosis } from "@/types/api";

interface Props {
  d: Diagnosis;
  emphasized?: boolean;
  onAction: (a: ActionRef) => void;
}

const RISK_TONE = { low: "ok", medium: "warn", high: "danger" } as const;

export function FindingCard({ d, emphasized, onAction }: Props) {
  return (
    <Card className={emphasized ? "border-primary/40" : undefined}>
      <CardHeader>
        <div className="flex flex-wrap items-center gap-2">
          <Badge tone={SEVERITY_TONE[d.severity]}>{d.severity}</Badge>
          <CardTitle className="text-base">{d.title}</CardTitle>
          <span className="ml-auto text-xs text-muted-foreground">Confidence {Math.round(d.confidence * 100)}%</span>
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm">{d.summary}</p>
        {d.primaryCause && (
          <p className="text-xs text-muted-foreground">
            Primary cause: <span className="font-medium text-foreground">{d.primaryCause}</span>
          </p>
        )}

        {d.contributors.length > 0 && (
          <div>
            <h3 className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Contributors</h3>
            <ul className="space-y-1.5">
              {d.contributors.map((c) => (
                <li key={c.name}>
                  <div className="flex justify-between text-sm">
                    <span>
                      {c.name}
                      {c.detail && <span className="text-xs text-muted-foreground"> · {c.detail}</span>}
                    </span>
                    <span className="tabular-nums">
                      {formatBytes(c.memoryBytes)} <span className="text-xs text-muted-foreground">({c.percent.toFixed(1)}% of RAM)</span>
                    </span>
                  </div>
                  <div className="mt-0.5 h-1 overflow-hidden rounded-full bg-secondary">
                    <div className="h-full rounded-full bg-primary" style={{ width: `${Math.min(100, c.percent)}%` }} />
                  </div>
                </li>
              ))}
            </ul>
          </div>
        )}

        {d.evidence.length > 0 && (
          <details className="text-xs">
            <summary className="cursor-pointer text-muted-foreground">Evidence ({d.evidence.length})</summary>
            <ul className="mt-1 list-disc space-y-0.5 pl-5">
              {d.evidence.map((e) => (
                <li key={e}>{e}</li>
              ))}
            </ul>
          </details>
        )}

        {d.recommendations.length > 0 && (
          <div>
            <h3 className="mb-1 text-xs font-semibold uppercase tracking-wide text-muted-foreground">Recommendations</h3>
            <ul className="space-y-3">
              {d.recommendations.map((r) => (
                <li key={r.id} className="rounded-md border p-3">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-sm font-medium">{r.problem}</span>
                    <Badge tone={RISK_TONE[r.risk]}>{r.risk} risk</Badge>
                  </div>
                  <p className="mt-1 text-sm text-muted-foreground">{r.explanation}</p>
                  <p className="mt-1 text-xs">
                    Expected benefit: <span className="font-medium">{r.expectedBenefit}</span>
                  </p>
                  {r.manualSteps && <pre className="mt-2 whitespace-pre-wrap rounded bg-muted/50 p-2 text-xs">{r.manualSteps}</pre>}
                  {r.action && (
                    <Button className="mt-2" size="sm" variant="outline" onClick={() => onAction(r.action as ActionRef)}>
                      {r.action.label}
                    </Button>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

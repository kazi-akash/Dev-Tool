import { useState } from "react";
import { ActionDialog } from "@/components/ActionDialog";
import { Button } from "@/components/ui/button";
import { useAsync } from "@/hooks/useAsync";
import { call } from "@/lib/ipc";
import { formatDateTime } from "@/lib/format";
import type { ActionRef, DiagnosisReport } from "@/types/api";
import { AiAssistant } from "./AiAssistant";
import { FindingCard } from "./FindingCard";

export function DiagnosisPage() {
  const report = useAsync(() => call<DiagnosisReport>("run_diagnosis"));
  const [pending, setPending] = useState<ActionRef | null>(null);
  const data = report.data;

  return (
    <div className="mx-auto max-w-4xl space-y-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">Diagnosis</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            Why your machine is struggling and what to do about it. {data ? `Analyzed ${formatDateTime(data.generatedAtMs)}.` : ""}
          </p>
        </div>
        <Button size="sm" onClick={() => void report.reload()} disabled={report.loading}>
          {report.loading ? "Analyzing..." : "Run diagnosis"}
        </Button>
      </div>

      {report.error && !data && (
        <p role="alert" className="rounded-md border border-danger/40 bg-danger/10 p-2 text-sm">
          {report.error}
        </p>
      )}
      {!data && !report.error && <p className="text-sm text-muted-foreground">Analyzing your system...</p>}

      {data?.findings.map((f, i) => (
        <FindingCard key={f.ruleId} d={f} emphasized={i === 0} onAction={setPending} />
      ))}

      <AiAssistant />

      {pending && (
        <ActionDialog
          actionId={pending.actionId}
          params={pending.params}
          onClose={(rec) => {
            setPending(null);
            if (rec) void report.reload();
          }}
        />
      )}
    </div>
  );
}

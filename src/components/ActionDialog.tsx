import { useEffect, useRef, useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { call, errorMessage } from "@/lib/ipc";
import { formatBytes } from "@/lib/format";
import { RISK_TONE } from "@/lib/status";
import type { ActionPreview, ActionRecord } from "@/types/api";

interface Props {
  actionId: string;
  params: Record<string, unknown>;
  /** Called after the dialog closes. `record` is set when the action ran. */
  onClose: (record: ActionRecord | null) => void;
}

/**
 * Every action goes through this dialog: preview first, then explicit confirmation (two steps for
 * destructive actions), then the result. The backend re-validates and re-checks confirmations; this
 * dialog is the user-facing half of that contract, not the enforcement.
 */
export function ActionDialog({ actionId, params, onClose }: Props) {
  const [preview, setPreview] = useState<ActionPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [doubleConfirmed, setDoubleConfirmed] = useState(false);
  const [running, setRunning] = useState(false);
  const [record, setRecord] = useState<ActionRecord | null>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let alive = true;
    call<ActionPreview>("preview_action", { actionId, params })
      .then((p) => alive && setPreview(p))
      .catch((e) => alive && setError(errorMessage(e)));
    return () => {
      alive = false;
    };
  }, [actionId, params]);

  useEffect(() => {
    cancelRef.current?.focus();
  }, [preview, error]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !running) onClose(record);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, record, running]);

  const needsConfirm = preview?.requiresConfirmation ?? false;
  const needsDouble = preview?.requiresDoubleConfirmation ?? false;
  const canRun = !!preview && !running && !record && (!needsConfirm || confirmed) && (!needsDouble || doubleConfirmed);

  async function run() {
    setRunning(true);
    setError(null);
    try {
      setRecord(await call<ActionRecord>("execute_action", { actionId, params, confirmed, doubleConfirmed }));
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setRunning(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 p-4" onMouseDown={(e) => e.target === e.currentTarget && !running && onClose(record)}>
      <div role="dialog" aria-modal="true" aria-labelledby="action-title" className="w-full max-w-lg rounded-lg border bg-card p-5 text-card-foreground shadow-lg">
        {!preview && !error && <p className="text-sm text-muted-foreground">Checking what this would do...</p>}

        {preview && (
          <>
            <div className="flex items-start justify-between gap-3">
              <h2 id="action-title" className="text-base font-semibold">
                {preview.title}
              </h2>
              <Badge tone={RISK_TONE[preview.risk]}>{preview.risk}</Badge>
            </div>
            <p className="mt-2 text-sm text-muted-foreground">{preview.summary}</p>

            <ul className="mt-3 max-h-40 space-y-1 overflow-y-auto rounded-md border bg-muted/40 p-2 text-xs">
              {preview.items.map((it) => (
                <li key={it.label} className="flex justify-between gap-3">
                  <span className="break-all">
                    {it.label}
                    {it.detail && <span className="text-muted-foreground"> · {it.detail}</span>}
                  </span>
                  {it.bytes != null && <span className="shrink-0 tabular-nums">{formatBytes(it.bytes)}</span>}
                </li>
              ))}
            </ul>

            {preview.estimatedBenefitBytes != null && (
              <p className="mt-2 text-xs text-muted-foreground">
                Expected recovery: <span className="font-medium text-foreground">{formatBytes(preview.estimatedBenefitBytes)}</span>
                {preview.reversible ? " · can be undone" : " · cannot be undone"}
              </p>
            )}

            {preview.warnings.length > 0 && (
              <ul className="mt-3 space-y-1 rounded-md border border-warn/40 bg-warn/10 p-2 text-xs">
                {preview.warnings.map((w) => (
                  <li key={w}>{w}</li>
                ))}
              </ul>
            )}

            {!record && needsConfirm && (
              <label className="mt-4 flex items-start gap-2 text-sm">
                <input type="checkbox" className="mt-1" checked={confirmed} onChange={(e) => setConfirmed(e.target.checked)} />
                <span>I have read the preview and want to do this.</span>
              </label>
            )}
            {!record && needsDouble && (
              <label className="mt-2 flex items-start gap-2 rounded-md border border-danger/40 bg-danger/10 p-2 text-sm">
                <input type="checkbox" className="mt-1" checked={doubleConfirmed} onChange={(e) => setDoubleConfirmed(e.target.checked)} />
                <span>This is destructive. I understand it is permanent and I am sure.</span>
              </label>
            )}
          </>
        )}

        {record && (
          <p role="status" className={`mt-4 rounded-md border p-2 text-sm ${record.success ? "border-ok/40 bg-ok/10" : "border-danger/40 bg-danger/10"}`}>
            {record.summary}
          </p>
        )}
        {error && (
          <p role="alert" className="mt-4 rounded-md border border-danger/40 bg-danger/10 p-2 text-sm">
            {error}
          </p>
        )}

        <div className="mt-5 flex justify-end gap-2">
          <Button ref={cancelRef} variant="outline" size="sm" disabled={running} onClick={() => onClose(record)}>
            {record ? "Close" : "Cancel"}
          </Button>
          {!record && preview && (
            <Button size="sm" variant={needsDouble ? "destructive" : "default"} disabled={!canRun} onClick={run}>
              {running ? "Working..." : needsDouble ? "Delete permanently" : "Run"}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

import { useState } from "react";
import { ReversibilityBadge } from "@/components/ActionResult";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { useAsync } from "@/hooks/useAsync";
import { call, errorMessage } from "@/lib/ipc";
import { formatBytes, formatDateTime } from "@/lib/format";
import { RISK_TONE } from "@/lib/status";
import { useSettings } from "@/lib/settings";
import type { ActionRecord, ActionSpec } from "@/types/api";

export function ActionsPage() {
  const { settings } = useSettings();
  const specs = useAsync(() => call<ActionSpec[]>("list_actions"));
  const history = useAsync(() => call<ActionRecord[]>("get_action_history"), { intervalMs: 5000 });
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  async function rollback(id: number) {
    try {
      const text = await call<string>("rollback_action", { recordId: id });
      setMsg({ ok: true, text });
    } catch (e) {
      setMsg({ ok: false, text: errorMessage(e) });
    }
    void history.reload();
  }

  return (
    <div className="mx-auto max-w-4xl space-y-4">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Actions</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          Everything DevPilot can change, and everything it has changed. Actions start from a recommendation or the Storage page, always show a preview first, and Medium risk and above need your confirmation (Destructive needs two).
        </p>
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Safe automation</CardTitle>
        </CardHeader>
        <CardContent className="text-sm">
          {settings.safeAutomation ? "On: old temporary files are cleaned automatically when a drive runs low." : "Off. Turn it on in Settings if you want DevPilot to clean old temporary files for you."}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Available actions</CardTitle>
        </CardHeader>
        <CardContent>
          {specs.error && <p className="text-sm text-muted-foreground">{specs.error}</p>}
          <ul className="divide-y">
            {specs.data?.map((a) => (
              <li key={a.id} className="py-2">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-medium">{a.name}</span>
                  <Badge tone={RISK_TONE[a.riskLevel]}>{a.riskLevel}</Badge>
                  <ReversibilityBadge value={a.reversibility} />
                  {a.reversible && <Badge tone="info">undoable</Badge>}
                  {a.automationEligible && <Badge tone="neutral">automation</Badge>}
                </div>
                <p className="mt-0.5 text-xs text-muted-foreground">{a.description}</p>
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>This session</CardTitle>
        </CardHeader>
        <CardContent>
          {msg && (
            <p role={msg.ok ? "status" : "alert"} className={`mb-2 rounded-md border p-2 text-sm ${msg.ok ? "border-ok/40 bg-ok/10" : "border-danger/40 bg-danger/10"}`}>
              {msg.text}
            </p>
          )}
          {history.data?.length === 0 && <p className="text-sm text-muted-foreground">No actions have run since DevPilot started. Older ones are on the History page.</p>}
          <ul className="space-y-2">
            {history.data?.map((r) => (
              <li key={r.id} className="flex flex-wrap items-center gap-2 text-sm">
                <Badge tone={r.status === "SUCCESS" ? "ok" : r.status === "FAILED" ? "danger" : "warn"}>{{ SUCCESS: "verified", UNVERIFIED: "not verified", NOT_EFFECTIVE: "no visible effect", FAILED: "failed" }[r.status]}</Badge>
                {r.success && <ReversibilityBadge value={r.reversibility} />}
                <span className="font-medium">{r.name}</span>
                {r.automated && <Badge>auto</Badge>}
                <span className="text-muted-foreground">{r.summary}</span>
                {r.freedBytes != null && r.freedBytes > 0 && <span className="text-xs tabular-nums">{formatBytes(r.freedBytes)} freed</span>}
                <span className="ml-auto text-xs text-muted-foreground">{formatDateTime(r.atMs)}</span>
                {r.success && r.reversible && !r.rolledBack && (
                  <Button size="sm" variant="outline" onClick={() => void rollback(r.id)}>
                    Undo
                  </Button>
                )}
                {r.rolledBack && <Badge>undone</Badge>}
              </li>
            ))}
          </ul>
        </CardContent>
      </Card>
    </div>
  );
}

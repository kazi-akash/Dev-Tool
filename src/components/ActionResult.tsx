import { Badge } from "@/components/ui/badge";
import type { Tone } from "@/components/ui/badge";
import { formatBytes } from "@/lib/format";
import type { ActionMeasurement, ActionRecord, ActionStatus, Reversibility } from "@/types/api";

export const REVERSIBILITY_TEXT: Record<Reversibility, { label: string; tone: Tone; help: string }> = {
  REVERSIBLE: { label: "Reversible", tone: "ok", help: "DevPilot can undo this with one click." },
  RECOVERABLE: { label: "Recoverable", tone: "info", help: "No undo button, but you can get it back by normal means (for example the Recycle Bin, or the program starts again)." },
  IRREVERSIBLE: { label: "Irreversible", tone: "danger", help: "Permanent. DevPilot cannot bring it back." },
};

const STATUS_TEXT: Record<ActionStatus, { label: string; tone: Tone }> = {
  SUCCESS: { label: "Success", tone: "ok" },
  UNVERIFIED: { label: "Done, not verified", tone: "warn" },
  NOT_EFFECTIVE: { label: "No visible effect", tone: "warn" },
  FAILED: { label: "Failed", tone: "danger" },
};

export function ReversibilityBadge({ value }: { value: Reversibility }) {
  const t = REVERSIBILITY_TEXT[value];
  return (
    <Badge tone={t.tone} title={t.help}>
      {t.label}
    </Badge>
  );
}

function signed(n: number) {
  return `${n >= 0 ? "+" : "-"}${formatBytes(Math.abs(n))}`;
}

function Line({ label, before, after, delta }: { label: string; before: number | null; after: number | null; delta: number | null }) {
  if (before == null || after == null) return <li>{label}: not measurable</li>;
  return (
    <li className="tabular-nums">
      {label}: {formatBytes(before)} → {formatBytes(after)}
      {delta != null && <span className="font-medium"> ({signed(delta)})</span>}
    </li>
  );
}

function Measured({ m }: { m: ActionMeasurement }) {
  return (
    <div className="mt-2 text-xs">
      <ul className="space-y-0.5">
        <Line label="Available RAM" before={m.before.ramAvailableBytes} after={m.after.ramAvailableBytes} delta={m.ramRecoveredBytes} />
        {(m.before.diskFreeBytes != null || m.after.diskFreeBytes != null) && <Line label="Free disk space" before={m.before.diskFreeBytes} after={m.after.diskFreeBytes} delta={m.diskRecoveredBytes} />}
      </ul>
      <p className="mt-1 text-muted-foreground">{m.note}</p>
    </div>
  );
}

/** The VERIFY and MEASURE steps of an action: what was confirmed, and what changed, measured not assumed. */
export function ActionResult({ record }: { record: ActionRecord }) {
  const s = STATUS_TEXT[record.status];
  const ok = record.status === "SUCCESS";
  const tone = record.status === "FAILED" ? "border-danger/40 bg-danger/10" : ok ? "border-ok/40 bg-ok/10" : "border-warn/40 bg-warn/10";
  return (
    <div role="status" className={`mt-4 rounded-md border p-2 text-sm ${tone}`}>
      <div className="flex flex-wrap items-center gap-2">
        <Badge tone={s.tone}>{s.label}</Badge>
        {record.status !== "FAILED" && <ReversibilityBadge value={record.reversibility} />}
      </div>
      <p className="mt-1">{record.summary}</p>
      {record.verification && (
        <p className="mt-1 text-xs">
          <span className="font-medium">Verified:</span> {record.verification.detail}
        </p>
      )}
      {record.measurement && <Measured m={record.measurement} />}
    </div>
  );
}

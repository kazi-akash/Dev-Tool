import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { cn } from "@/lib/utils";

export type Level = "ok" | "warn" | "danger";

const LEVEL_BAR: Record<Level, string> = {
  ok: "bg-ok",
  warn: "bg-warn",
  danger: "bg-danger",
};

interface Props {
  title: string;
  value: string;
  detail?: string;
  /** 0-100; omit to hide the bar. */
  percent?: number;
  level?: Level;
}

export function MetricCard({ title, value, detail, percent, level = "ok" }: Props) {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-muted-foreground">{title}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="text-2xl font-semibold tabular-nums">{value}</div>
        {detail && <div className="mt-0.5 text-xs text-muted-foreground tabular-nums">{detail}</div>}
        {percent !== undefined && (
          <div
            className="mt-3 h-1.5 overflow-hidden rounded-full bg-secondary"
            role="progressbar"
            aria-label={title}
            aria-valuenow={Math.round(percent)}
            aria-valuemin={0}
            aria-valuemax={100}
          >
            <div
              className={cn("h-full rounded-full transition-[width] duration-300", LEVEL_BAR[level])}
              style={{ width: `${Math.min(100, Math.max(0, percent))}%` }}
            />
          </div>
        )}
      </CardContent>
    </Card>
  );
}

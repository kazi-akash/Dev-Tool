import type { ReactNode } from "react";
import { Badge, TONE_BAR, type Tone } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";

interface Props {
  title: string;
  value: string;
  detail?: string;
  /** 0-100; omit to hide the bar. */
  percent?: number;
  tone?: Tone;
  /** Status label shown next to the title, e.g. the pressure state. */
  badge?: string;
  footer?: ReactNode;
}

export function MetricCard({ title, value, detail, percent, tone = "ok", badge, footer }: Props) {
  return (
    <Card>
      <CardHeader className="flex-row items-center justify-between gap-2 space-y-0">
        <CardTitle className="text-muted-foreground">{title}</CardTitle>
        {badge && <Badge tone={tone}>{badge}</Badge>}
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
            <div className={`h-full rounded-full transition-[width] duration-300 ${TONE_BAR[tone]}`} style={{ width: `${Math.min(100, Math.max(0, percent))}%` }} />
          </div>
        )}
        {footer && <div className="mt-2 text-xs text-muted-foreground">{footer}</div>}
      </CardContent>
    </Card>
  );
}

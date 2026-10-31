import { cva, type VariantProps } from "class-variance-authority";
import type * as React from "react";
import { cn } from "@/lib/utils";

const badgeVariants = cva("inline-flex items-center rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide", {
  variants: {
    tone: {
      neutral: "bg-secondary text-secondary-foreground",
      ok: "bg-ok/15 text-ok",
      warn: "bg-warn/20 text-warn",
      danger: "bg-danger/15 text-danger",
      info: "bg-primary/15 text-primary",
    },
  },
  defaultVariants: { tone: "neutral" },
});

export type Tone = NonNullable<VariantProps<typeof badgeVariants>["tone"]>;

export function Badge({ tone, className, ...props }: React.HTMLAttributes<HTMLSpanElement> & VariantProps<typeof badgeVariants>) {
  return <span className={cn(badgeVariants({ tone }), className)} {...props} />;
}

/** Status text colors, shared so every screen means the same thing by the same color. */
export const TONE_TEXT: Record<Tone, string> = {
  neutral: "text-muted-foreground",
  ok: "text-ok",
  warn: "text-warn",
  danger: "text-danger",
  info: "text-primary",
};

export const TONE_BAR: Record<Tone, string> = {
  neutral: "bg-muted-foreground",
  ok: "bg-ok",
  warn: "bg-warn",
  danger: "bg-danger",
  info: "bg-primary",
};

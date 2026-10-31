import type { ReactNode } from "react";

interface Props {
  title: string;
  description: string;
  phase: string;
  children?: ReactNode;
}

export function PagePlaceholder({ title, description, phase, children }: Props) {
  return (
    <div className="mx-auto max-w-3xl">
      <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
      <p className="mt-1 text-sm text-muted-foreground">{description}</p>
      <div className="mt-6 rounded-lg border border-dashed p-8 text-center text-sm text-muted-foreground">
        Not implemented yet. Planned for {phase}.
      </div>
      {children}
    </div>
  );
}

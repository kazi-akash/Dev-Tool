import { Fragment, useState } from "react";
import { Card } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { useProcesses } from "@/hooks/useProcesses";
import { formatBytes } from "@/lib/format";
import { cn } from "@/lib/utils";
import { CATEGORIES, type Category, type ProcessInfo, type SortBy } from "@/types/process";

const SORTS: { value: SortBy; label: string }[] = [
  { value: "memory", label: "Memory" },
  { value: "cpu", label: "CPU" },
  { value: "name", label: "Name" },
];

const LIMITS = [50, 100, 250];

const EVIDENCE_LABEL: Record<ProcessInfo["evidence"], string> = {
  path: "path",
  commandLine: "command line",
  name: "name only",
  none: "no match",
};

function CategoryBadge({ category }: { category: Category }) {
  return (
    <span className="inline-block rounded bg-secondary px-1.5 py-0.5 text-[10px] font-medium tracking-wide text-secondary-foreground">
      {category}
    </span>
  );
}

function Detail({ p }: { p: ProcessInfo }) {
  return (
    <div className="space-y-1 bg-muted/40 px-4 py-3 text-xs">
      <div>
        <span className="text-muted-foreground">Path: </span>
        <span className="break-all">{p.exePath ?? "not available (protected process)"}</span>
      </div>
      <div>
        <span className="text-muted-foreground">Command line (secrets redacted): </span>
        <span className="break-all">{p.commandLine ?? "not available"}</span>
      </div>
      <div>
        <span className="text-muted-foreground">Started: </span>
        {p.startTime > 0 ? new Date(p.startTime * 1000).toLocaleString() : "unknown"}
        <span className="text-muted-foreground"> · Parent PID: </span>
        {p.parentPid ?? "none"}
        <span className="text-muted-foreground"> · Category based on: </span>
        {EVIDENCE_LABEL[p.evidence]}
      </div>
    </div>
  );
}

export function ProcessesPage() {
  const [sortBy, setSortBy] = useState<SortBy>("memory");
  const [category, setCategory] = useState<Category | undefined>(undefined);
  const [limit, setLimit] = useState(100);
  const [expanded, setExpanded] = useState<number | null>(null);

  const list = useProcesses({ limit, sortBy, category });

  return (
    <div className="mx-auto max-w-5xl space-y-4">
      <div>
        <h1 className="text-xl font-semibold tracking-tight">Processes</h1>
        <p className="mt-1 text-sm text-muted-foreground">
          {list
            ? `${list.matched} of ${list.total} processes${list.matched > list.processes.length ? `, showing top ${list.processes.length}` : ""}.`
            : "Waiting for the first sample..."}
        </p>
      </div>

      <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
        <div className="flex items-center gap-1" role="group" aria-label="Sort by">
          <span className="mr-1 text-xs text-muted-foreground">Sort</span>
          {SORTS.map((s) => (
            <Button
              key={s.value}
              size="sm"
              variant={sortBy === s.value ? "default" : "outline"}
              aria-pressed={sortBy === s.value}
              onClick={() => setSortBy(s.value)}
            >
              {s.label}
            </Button>
          ))}
        </div>

        <label className="flex items-center gap-2 text-xs text-muted-foreground">
          Category
          <select
            className="h-8 rounded-md border border-input bg-background px-2 text-sm text-foreground"
            value={category ?? ""}
            onChange={(e) => setCategory((e.target.value || undefined) as Category | undefined)}
          >
            <option value="">All</option>
            {CATEGORIES.map((c) => (
              <option key={c} value={c}>
                {c}
              </option>
            ))}
          </select>
        </label>

        <div className="flex items-center gap-1" role="group" aria-label="Rows">
          <span className="mr-1 text-xs text-muted-foreground">Rows</span>
          {LIMITS.map((n) => (
            <Button
              key={n}
              size="sm"
              variant={limit === n ? "default" : "outline"}
              aria-pressed={limit === n}
              onClick={() => setLimit(n)}
            >
              {n}
            </Button>
          ))}
        </div>
      </div>

      <Card className="overflow-hidden">
        <table className="w-full text-sm">
          <thead className="border-b bg-muted/40 text-left text-xs text-muted-foreground">
            <tr>
              <th className="px-4 py-2 font-medium">Name</th>
              <th className="px-3 py-2 font-medium">PID</th>
              <th className="px-3 py-2 font-medium">Category</th>
              <th className="px-3 py-2 text-right font-medium">CPU</th>
              <th className="px-4 py-2 text-right font-medium">Memory</th>
            </tr>
          </thead>
          <tbody>
            {list?.processes.map((p) => {
              const open = expanded === p.pid;
              return (
                <Fragment key={`${p.pid}-${p.startTime}`}>
                  <tr
                    className={cn("cursor-pointer border-b last:border-0 hover:bg-accent/50", open && "bg-accent/40")}
                    onClick={() => setExpanded(open ? null : p.pid)}
                    aria-expanded={open}
                  >
                    <td className="max-w-[18rem] truncate px-4 py-1.5 font-medium">{p.name}</td>
                    <td className="px-3 py-1.5 tabular-nums text-muted-foreground">{p.pid}</td>
                    <td className="px-3 py-1.5">
                      <CategoryBadge category={p.category} />
                    </td>
                    <td className="px-3 py-1.5 text-right tabular-nums">{p.cpuPercent.toFixed(1)}%</td>
                    <td className="px-4 py-1.5 text-right tabular-nums">{formatBytes(p.memoryBytes)}</td>
                  </tr>
                  {open && (
                    <tr className="border-b">
                      <td colSpan={5} className="p-0">
                        <Detail p={p} />
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
            {list && list.processes.length === 0 && (
              <tr>
                <td colSpan={5} className="px-4 py-8 text-center text-muted-foreground">
                  No processes match this filter.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </Card>
    </div>
  );
}

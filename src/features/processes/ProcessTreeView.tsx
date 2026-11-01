import { useMemo, useState } from "react";
import { Card } from "@/components/ui/card";
import { useAsync } from "@/hooks/useAsync";
import { useVirtualWindow } from "@/hooks/useVirtualWindow";
import { formatBytes } from "@/lib/format";
import { call } from "@/lib/ipc";
import type { ProcessTree, TreeNode } from "@/types/process";

const ROW_HEIGHT = 32;

export interface TreeRow {
  node: TreeNode;
  depth: number;
}

/** The rows currently visible: a node's children appear only while it is expanded. Pure. */
export function flattenTree(roots: TreeNode[], expanded: ReadonlySet<number>): TreeRow[] {
  const rows: TreeRow[] = [];
  const walk = (n: TreeNode, depth: number) => {
    rows.push({ node: n, depth });
    if (expanded.has(n.pid)) n.children.forEach((c) => walk(c, depth + 1));
  };
  roots.forEach((r) => walk(r, 0));
  return rows;
}

/** Which nodes start expanded: the roots, so the first level is visible without a click. */
export function initialExpanded(roots: TreeNode[]): Set<number> {
  return new Set(roots.filter((r) => r.children.length > 0).map((r) => r.pid));
}

/** Parent/child view of the biggest process subtrees, so "what is behind this 4 GB node.exe" has an answer. */
export function ProcessTreeView() {
  const tree = useAsync(() => call<ProcessTree>("get_process_tree"), { intervalMs: 5_000 });
  const [expanded, setExpanded] = useState<Set<number> | null>(null);
  const t = tree.data;
  const open = expanded ?? (t ? initialExpanded(t.roots) : new Set<number>());
  const rows = useMemo(() => (t ? flattenTree(t.roots, open) : []), [t, open]);
  const { ref, range } = useVirtualWindow(rows.length, { rowHeight: ROW_HEIGHT });

  function toggle(pid: number) {
    const next = new Set(open);
    if (next.has(pid)) next.delete(pid);
    else next.add(pid);
    setExpanded(next);
  }

  return (
    <Card className="overflow-hidden">
      {!t && !tree.error && <p className="p-4 text-sm text-muted-foreground">Building the process tree...</p>}
      {!t && tree.error && <p className="p-4 text-sm text-muted-foreground">The process tree is unavailable: {tree.error}</p>}
      {t && t.roots.length === 0 && <p className="p-4 text-sm text-muted-foreground">No process group over 20 MB.</p>}
      {t && t.roots.length > 0 && (
        <>
          <div ref={ref} className="max-h-[32rem] overflow-y-auto" role="tree" aria-label="Process tree">
            <div style={{ height: range.padTop }} aria-hidden />
            {rows.slice(range.start, range.end).map(({ node, depth }) => {
              const hasKids = node.children.length > 0;
              const isOpen = open.has(node.pid);
              return (
                <div
                  key={node.pid}
                  role="treeitem"
                  aria-level={depth + 1}
                  aria-expanded={hasKids ? isOpen : undefined}
                  className="flex items-center gap-2 border-b px-3 text-sm hover:bg-accent/50"
                  style={{ height: ROW_HEIGHT, paddingLeft: 12 + depth * 18 }}
                >
                  {hasKids ? (
                    <button type="button" className="w-4 text-xs text-muted-foreground" aria-label={`${isOpen ? "Collapse" : "Expand"} ${node.name}`} onClick={() => toggle(node.pid)}>
                      {isOpen ? "▾" : "▸"}
                    </button>
                  ) : (
                    <span className="w-4" />
                  )}
                  <span className="min-w-0 flex-1 truncate font-medium">
                    {node.name} <span className="text-xs font-normal text-muted-foreground">PID {node.pid}</span>
                    {node.hiddenChildren > 0 && <span className="ml-1 text-xs font-normal text-muted-foreground">+{node.hiddenChildren} small</span>}
                  </span>
                  <span className="w-14 text-right text-xs tabular-nums text-muted-foreground">{node.cpuPercent.toFixed(1)}%</span>
                  <span className="w-20 text-right tabular-nums">{formatBytes(node.memoryBytes)}</span>
                  <span className="w-24 text-right tabular-nums text-muted-foreground" title="This process and everything below it">
                    {hasKids || node.hiddenChildren > 0 ? `Σ ${formatBytes(node.subtreeBytes)}` : ""}
                  </span>
                </div>
              );
            })}
            <div style={{ height: range.padBottom }} aria-hidden />
          </div>
          <p className="border-t p-2 text-xs text-muted-foreground">
            {t.shownNodes} of {t.totalProcesses} processes shown{t.truncated ? " (limited to the largest groups)" : ""}. Σ is the sum of each process's own memory; shared pages are counted once per process.
          </p>
        </>
      )}
    </Card>
  );
}

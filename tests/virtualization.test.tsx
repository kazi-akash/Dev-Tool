import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ProcessesPage } from "@/features/processes/ProcessesPage";
import { flattenTree, initialExpanded } from "@/features/processes/ProcessTreeView";
import type { ProcessInfo, ProcessTree, TreeNode } from "@/types/process";
import { GB } from "./fixtures";
import { ipc } from "./ipc-mock";

const proc = (pid: number): ProcessInfo => ({ pid, name: `proc${pid}.exe`, exePath: null, commandLine: null, parentPid: null, startTime: 1, cpuPercent: 1, memoryBytes: (5000 - pid) * 1024 * 1024, category: "NODE", evidence: "name", tags: [] });

const node = (pid: number, mb: number, children: TreeNode[] = [], over: Partial<TreeNode> = {}): TreeNode => ({
  pid,
  name: `n${pid}.exe`,
  category: "NODE",
  cpuPercent: 1.5,
  memoryBytes: mb * 1024 * 1024,
  subtreeBytes: (mb + children.reduce((s, c) => s + c.subtreeBytes / 1024 / 1024, 0)) * 1024 * 1024,
  hiddenChildren: 0,
  children,
  ...over,
});

function setup(processCount: number, tree?: ProcessTree) {
  ipc.on("get_docker_status", { available: false, error: "Docker CLI not found", containers: [], totalMemoryBytes: 0 });
  ipc.on("get_docker_breakdown", null);
  const processes = Array.from({ length: processCount }, (_, i) => proc(i + 1));
  ipc.on("get_processes", { total: processCount, matched: processCount, processes });
  if (tree) ipc.on("get_process_tree", tree);
}

describe("virtualized process list", () => {
  it("renders every row of a short list", async () => {
    setup(40);
    render(<ProcessesPage />);
    expect(await screen.findByText("proc40.exe")).toBeInTheDocument();
    expect(screen.getAllByRole("row")).toHaveLength(41); // header + 40
  });

  it("renders only a window of a long list, not thousands of nodes", async () => {
    setup(500);
    render(<ProcessesPage />);
    expect(await screen.findByText("proc1.exe")).toBeInTheDocument();
    const rows = screen.getAllByRole("row").length;
    expect(rows).toBeLessThan(60);
    expect(rows).toBeGreaterThan(10);
    expect(screen.queryByText("proc400.exe")).not.toBeInTheDocument();
  });
});

describe("process tree", () => {
  const tree: ProcessTree = {
    roots: [node(1, 100, [node(2, 800, [node(3, 300)]), node(4, 50, [], { hiddenChildren: 3 })]), node(9, 60)],
    totalProcesses: 120,
    shownNodes: 5,
    truncated: false,
  };

  it("flattens only expanded branches and starts with the roots open", () => {
    expect(initialExpanded(tree.roots)).toEqual(new Set([1]));
    expect(flattenTree(tree.roots, new Set()).map((r) => r.node.pid)).toEqual([1, 9]);
    expect(flattenTree(tree.roots, new Set([1])).map((r) => [r.node.pid, r.depth])).toEqual([[1, 0], [2, 1], [4, 1], [9, 0]]);
    expect(flattenTree(tree.roots, new Set([1, 2])).map((r) => r.node.pid)).toEqual([1, 2, 3, 4, 9]);
  });

  it("is an optional view: the list stays the default and the tree is only fetched on request", async () => {
    const user = userEvent.setup();
    setup(10, tree);
    render(<ProcessesPage />);
    await screen.findByText("proc1.exe");
    expect(ipc.calledWith("get_process_tree")).toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Tree" }));
    const t = await screen.findByRole("tree", { name: "Process tree" });
    expect(within(t).getByText(/n1\.exe/)).toBeInTheDocument();
    expect(within(t).getByText(/n2\.exe/)).toBeInTheDocument();
    expect(within(t).queryByText(/n3\.exe/)).not.toBeInTheDocument();
    expect(ipc.calledWith("get_process_tree")).toHaveLength(1);
  });

  it("expands and collapses a branch and shows subtree totals and hidden small children", async () => {
    const user = userEvent.setup();
    setup(10, tree);
    render(<ProcessesPage />);
    await user.click(await screen.findByRole("button", { name: "Tree" }));
    const t = await screen.findByRole("tree");
    await user.click(within(t).getByRole("button", { name: "Expand n2.exe" }));
    expect(within(t).getByText(/n3\.exe/)).toBeInTheDocument();
    await user.click(within(t).getByRole("button", { name: "Collapse n2.exe" }));
    expect(within(t).queryByText(/n3\.exe/)).not.toBeInTheDocument();
    expect(within(t).getByText(/\+3 small/)).toBeInTheDocument();
    expect(within(t).getByText(/Σ 1\.2 GB/)).toBeInTheDocument(); // 100 + 800 + 300 + 50 MB
    expect(screen.getByText(/5 of 120 processes shown/)).toBeInTheDocument();
  });

  it("says so when nothing is big enough and when the tree is unavailable", async () => {
    const user = userEvent.setup();
    setup(3, { roots: [], totalProcesses: 3, shownNodes: 0, truncated: false });
    render(<ProcessesPage />);
    await user.click(await screen.findByRole("button", { name: "Tree" }));
    expect(await screen.findByText(/No process group over 20 MB/)).toBeInTheDocument();
  });

  it("GB constant sanity (fixtures import)", () => {
    expect(GB).toBe(1024 ** 3);
  });
});

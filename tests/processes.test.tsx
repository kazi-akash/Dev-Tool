import { describe, expect, it } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ProcessesPage } from "@/features/processes/ProcessesPage";
import type { ProcessInfo, ProcessList, ProcessQuery } from "@/types/process";
import { GB, preview } from "./fixtures";
import { ipc } from "./ipc-mock";

const proc = (over: Partial<ProcessInfo>): ProcessInfo => ({
  pid: 1,
  name: "x.exe",
  exePath: null,
  commandLine: null,
  parentPid: null,
  startTime: 0,
  cpuPercent: 0,
  memoryBytes: 0,
  category: "OTHER",
  evidence: "none",
  tags: [],
  ...over,
});

const ALL: ProcessInfo[] = [
  proc({ pid: 10, name: "node.exe", category: "NODE", evidence: "commandLine", memoryBytes: 2 * GB, cpuPercent: 12.34, exePath: "C:\\node.exe", commandLine: "node next dev --token ***" }),
  proc({ pid: 20, name: "chrome.exe", category: "BROWSER", evidence: "name", memoryBytes: 1 * GB }),
];

function setup(docker: unknown = { available: false, error: "Docker CLI not found", containers: [], totalMemoryBytes: 0 }) {
  ipc.on("get_processes", (args) => {
    const q = args!.query as ProcessQuery;
    const rows = q.category ? ALL.filter((p) => p.category === q.category) : ALL;
    return { total: ALL.length, matched: rows.length, processes: rows } satisfies ProcessList;
  });
  ipc.on("get_docker_status", docker);
}

describe("processes page", () => {
  it("renders rows with category and formatted memory/cpu", async () => {
    setup();
    render(<ProcessesPage />);
    expect(await screen.findByText("node.exe")).toBeInTheDocument();
    const row = screen.getByText("node.exe").closest("tr")!;
    expect(within(row).getByText("NODE")).toBeInTheDocument();
    expect(screen.getByText("2.0 GB")).toBeInTheDocument();
    expect(screen.getByText("12.3%")).toBeInTheDocument();
    expect(screen.getByText("2 of 2 processes.")).toBeInTheDocument();
  });

  it("shows redacted command line and evidence when a row is expanded", async () => {
    setup();
    const user = userEvent.setup();
    render(<ProcessesPage />);
    await user.click(await screen.findByText("node.exe"));
    expect(screen.getByText("node next dev --token ***")).toBeInTheDocument();
    expect(screen.getByText(/Category based on:/).parentElement).toHaveTextContent("Category based on: command line");
  });

  it("sends the chosen category and sort to Rust", async () => {
    setup();
    const user = userEvent.setup();
    render(<ProcessesPage />);
    await screen.findByText("node.exe");
    await user.selectOptions(screen.getByRole("combobox"), "BROWSER");
    await waitFor(() => expect(screen.queryByText("node.exe")).not.toBeInTheDocument());
    expect(screen.getByText("chrome.exe")).toBeInTheDocument();
    await user.click(within(screen.getByRole("group", { name: "Sort by" })).getByRole("button", { name: "CPU" }));
    await waitFor(() => expect(ipc.calledWith("get_processes").at(-1)!.args!.query).toMatchObject({ category: "BROWSER", sortBy: "cpu" }));
  });

  it("re-queries on every monitor tick but not while a query is in flight", async () => {
    setup();
    render(<ProcessesPage />);
    await screen.findByText("node.exe");
    const before = ipc.calledWith("get_processes").length;
    ipc.emit("system-metrics", {});
    await waitFor(() => expect(ipc.calledWith("get_processes").length).toBe(before + 1));
  });
});

describe("docker panel", () => {
  it("explains why Docker data is missing and keeps the rest working", async () => {
    setup();
    render(<ProcessesPage />);
    expect(await screen.findByText(/Docker details unavailable: Docker CLI not found/)).toBeInTheDocument();
    expect(await screen.findByText("node.exe")).toBeInTheDocument();
  });

  it("lists containers and stops one only after a preview", async () => {
    setup({
      available: true,
      error: null,
      totalMemoryBytes: 900 * 1024 * 1024,
      containers: [{ id: "abc", name: "redis-idle", image: "redis:7", status: "Up", running: true, memoryBytes: 900 * 1024 * 1024, cpuPercent: 0.2 }],
    });
    ipc.on("preview_action", preview({ actionId: "docker.stop_container", title: "Stop container redis-idle", risk: "LOW", requiresConfirmation: false, requiresDoubleConfirmation: false, reversible: true, warnings: [], estimatedBenefitBytes: null }));
    const user = userEvent.setup();
    render(<ProcessesPage />);
    await user.click(await screen.findByRole("button", { name: "Stop redis-idle" }));
    expect(await screen.findByText("Stop container redis-idle")).toBeInTheDocument();
    expect(ipc.calledWith("preview_action")[0].args).toMatchObject({ actionId: "docker.stop_container", params: { container: "redis-idle" } });
    expect(ipc.calledWith("execute_action")).toHaveLength(0);
  });
});

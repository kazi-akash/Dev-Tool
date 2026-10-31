import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ProcessInfo, ProcessList, ProcessQuery } from "@/types/process";

const GB = 1024 ** 3;
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
  ...over,
});

const ALL: ProcessInfo[] = [
  proc({ pid: 10, name: "node.exe", category: "NODE", evidence: "commandLine", memoryBytes: 2 * GB, cpuPercent: 12.34, exePath: "C:\\node.exe", commandLine: "node next dev --token ***" }),
  proc({ pid: 20, name: "chrome.exe", category: "BROWSER", evidence: "name", memoryBytes: 1 * GB }),
];

const calls: ProcessQuery[] = [];
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string, args: { query: ProcessQuery }) => {
    if (cmd !== "get_processes") return Promise.resolve(undefined);
    calls.push(args.query);
    const rows = args.query.category ? ALL.filter((p) => p.category === args.query.category) : ALL;
    const list: ProcessList = { total: ALL.length, matched: rows.length, processes: rows };
    return Promise.resolve(list);
  },
}));

import { ProcessesPage } from "@/features/processes/ProcessesPage";

describe("processes page", () => {
  it("renders rows with category and formatted memory/cpu", async () => {
    render(<ProcessesPage />);
    expect(await screen.findByText("node.exe")).toBeInTheDocument();
    const row = screen.getByText("node.exe").closest("tr")!;
    expect(within(row).getByText("NODE")).toBeInTheDocument();
    expect(screen.getByText("2.0 GB")).toBeInTheDocument();
    expect(screen.getByText("12.3%")).toBeInTheDocument();
    expect(screen.getByText("2 of 2 processes.")).toBeInTheDocument();
  });

  it("shows redacted command line and evidence when a row is expanded", async () => {
    const user = userEvent.setup();
    render(<ProcessesPage />);
    await user.click(await screen.findByText("node.exe"));
    expect(screen.getByText("node next dev --token ***")).toBeInTheDocument();
    expect(screen.getByText(/Category based on:/).parentElement).toHaveTextContent(
      "Category based on: command line",
    );
  });

  it("sends the chosen category and sort to Rust", async () => {
    const user = userEvent.setup();
    render(<ProcessesPage />);
    await screen.findByText("node.exe");

    await user.selectOptions(screen.getByRole("combobox"), "BROWSER");
    await waitFor(() => expect(screen.queryByText("node.exe")).not.toBeInTheDocument());
    expect(screen.getByText("chrome.exe")).toBeInTheDocument();

    await user.click(within(screen.getByRole("group", { name: "Sort by" })).getByRole("button", { name: "CPU" }));
    await waitFor(() => expect(calls.at(-1)).toMatchObject({ category: "BROWSER", sortBy: "cpu" }));
  });
});

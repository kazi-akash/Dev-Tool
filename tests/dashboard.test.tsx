import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { DashboardPage } from "@/features/dashboard/DashboardPage";
import { buildAlerts } from "@/features/dashboard/alerts";
import type { GrowthFinding } from "@/types/api";
import { diagnosis, report, snapshot, workloads, GB } from "./fixtures";
import { ipc } from "./ipc-mock";

function mountWith(over: Parameters<typeof snapshot>[0] = {}, rep = report()) {
  ipc.on("get_system_snapshot", snapshot(over));
  ipc.on("run_diagnosis", rep);
  ipc.on("get_workloads", workloads());
  ipc.on("get_memory_growth", []);
  ipc.on("get_recent_diagnoses", { available: true, items: [{ id: 1, tsMs: Date.now() - 5 * 60_000, status: "problem", severity: "high", ruleId: "memory-pressure", title: "Memory pressure detected", summary: "RAM was at 90%.", primaryCause: "Docker/WSL2", confidence: 0.9 }] });
  ipc.on("get_processes", {
    total: 2,
    matched: 2,
    processes: [
      { pid: 1, name: "vmmemWSL", exePath: null, commandLine: null, parentPid: null, startTime: 1, cpuPercent: 1, memoryBytes: 4.4 * GB, category: "DOCKER", evidence: "name", tags: [] },
      { pid: 2, name: "chrome.exe", exePath: null, commandLine: null, parentPid: null, startTime: 1, cpuPercent: 1, memoryBytes: 1 * GB, category: "BROWSER", evidence: "name", tags: [] },
    ],
  });
  return render(
    <MemoryRouter>
      <DashboardPage />
    </MemoryRouter>,
  );
}

describe("dashboard", () => {
  it("answers 'is my PC healthy, what is using resources, why, what can I do' on one screen", async () => {
    mountWith();
    // 1. Healthy? + 3. Why?
    expect(await screen.findByText("Problem detected")).toBeInTheDocument();
    expect(screen.getByText(/primary contributors/)).toBeInTheDocument();
    expect(screen.getByText(/confidence 91%/)).toBeInTheDocument();
    // 2. What is consuming resources?
    expect(await screen.findByText("Docker / WSL2")).toBeInTheDocument();
    expect(screen.getByText("WSL2 VM (vmmem)")).toBeInTheDocument();
    expect(await screen.findByText("vmmemWSL")).toBeInTheDocument();
    // 4. What can I do?
    expect(screen.getByRole("button", { name: "Analyze" })).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Optimize" })).toHaveAttribute("href", "/diagnosis");
  });

  it("shows RAM pressure state, trend and per-drive state with a prediction", async () => {
    mountWith();
    expect(await screen.findByText("HIGH")).toBeInTheDocument();
    expect(screen.getByText(/rising, critical in/)).toBeInTheDocument();
    expect(screen.getByText("CRITICAL")).toBeInTheDocument();
    // Shown on the drive card and again as an alert.
    expect(screen.getAllByText(/Full in about 2.0 h at the current rate/)).toHaveLength(2);
    expect(screen.getByRole("progressbar", { name: "RAM" })).toHaveAttribute("aria-valuenow", "88");
  });

  it("lists current alerts in plain words", async () => {
    mountWith();
    const alerts = await screen.findByText(/Memory pressure high/);
    expect(alerts).toBeInTheDocument();
    expect(screen.getByText(/C:\\ is critical: 2% free/)).toBeInTheDocument();
  });

  it("shows recent diagnoses from history and says so when history is unavailable", async () => {
    mountWith();
    const card = (await screen.findByText("Recent diagnoses")).closest("div")!.parentElement!;
    expect(await within(card).findByText("RAM was at 90%.")).toBeInTheDocument();
  });

  it("stays useful when diagnosis fails", async () => {
    ipc.on("get_system_snapshot", snapshot());
    ipc.on("run_diagnosis", () => {
      throw "Monitoring has not produced a sample yet.";
    });
    ipc.on("get_workloads", []);
    ipc.on("get_memory_growth", []);
    ipc.on("get_recent_diagnoses", { available: false, items: [] });
    ipc.on("get_processes", { total: 0, matched: 0, processes: [] });
    render(
      <MemoryRouter>
        <DashboardPage />
      </MemoryRouter>,
    );
    expect(await screen.findByText(/Monitoring has not produced a sample yet/)).toBeInTheDocument();
    expect(await screen.findByText(/History is unavailable/)).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "CPU" })).toBeInTheDocument();
  });

  it("a healthy system shows a calm banner and a disabled Optimize", async () => {
    const healthy = diagnosis({ ruleId: "healthy", severity: "info", title: "System looks healthy", summary: "RAM is at 40% and no drive is low on space.", primaryCause: null, contributors: [], recommendations: [] });
    mountWith({ pressure: { ...snapshot().pressure!, state: "NORMAL", trend: "STABLE", minutesToCritical: null }, diskHealth: [{ ...snapshot().diskHealth[0], state: "NORMAL", hoursToFull: null }] }, report({ status: "healthy" }, healthy));
    expect(await screen.findByText("Healthy")).toBeInTheDocument();
    expect(screen.getByText("RAM is at 40% and no drive is low on space.")).toBeInTheDocument();
    expect(screen.getByText(/No alerts/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Optimize" })).toHaveAttribute("aria-disabled", "true");
  });
});

describe("alerts", () => {
  const growth = (pid: number): GrowthFinding => ({ pid, name: "node.exe", startBytes: 1, currentBytes: 2, growthBytes: 1, growthPercent: 100, bytesPerMin: 1, windowMinutes: 30, confidence: 0.8, summary: "" });

  it("is empty for a healthy machine", () => {
    const s = snapshot({ pressure: { ...snapshot().pressure!, state: "NORMAL" }, diskHealth: [{ ...snapshot().diskHealth[0], state: "NORMAL" }] });
    expect(buildAlerts(s, [])).toEqual([]);
    expect(buildAlerts(null, null)).toEqual([]);
  });

  it("words pressure, disk and growth alerts, capping growth at two", () => {
    const a = buildAlerts(snapshot(), [growth(1), growth(2), growth(3)]);
    expect(a.map((x) => x.id)).toEqual(["ram", "disk-C:\\", "growth-1", "growth-2"]);
    expect(a[0].text).toContain("Memory pressure high (88%, rising)");
    expect(a[2].text).toContain("Possible memory growth/leak");
    expect(a[2].text).not.toMatch(/is leaking/i);
  });
});

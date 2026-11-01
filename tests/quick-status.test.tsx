import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QuickStatusPage } from "@/features/quick/QuickStatusPage";
import type { QuickStatus } from "@/types/system";
import { GB } from "./fixtures";
import { ipc } from "./ipc-mock";

const status = (over: Partial<QuickStatus> = {}): QuickStatus => ({
  ramPercent: 87.4,
  ramUsedBytes: 14 * GB,
  ramTotalBytes: 16 * GB,
  cpuPercent: 12.3,
  disk: { mountPoint: "C:\\", freeBytes: 120 * GB, totalBytes: 500 * GB, freePercent: 24, hoursToFull: null },
  severity: "HIGH",
  headline: "RAM is at 87%. Docker / WSL2 holds the most (4.8 GB).",
  incidentActive: false,
  ...over,
});

describe("tray quick status", () => {
  it("shows RAM, CPU, disk, severity and the primary diagnosis", async () => {
    ipc.on("get_quick_status", status());
    render(<QuickStatusPage />);
    expect(await screen.findByText("High")).toBeInTheDocument();
    expect(screen.getByText("87%")).toBeInTheDocument();
    expect(screen.getByText("12%")).toBeInTheDocument();
    expect(screen.getByText("120.0 GB free")).toBeInTheDocument();
    expect(screen.getByText(/Docker \/ WSL2 holds the most/)).toBeInTheDocument();
  });

  it("mentions a running incident", async () => {
    ipc.on("get_quick_status", status({ incidentActive: true, severity: "CRITICAL" }));
    render(<QuickStatusPage />);
    expect(await screen.findByText(/An incident is being recorded/)).toBeInTheDocument();
  });

  it("has exactly the five actions and each sends only its own id", async () => {
    const user = userEvent.setup();
    ipc.on("get_quick_status", status());
    ipc.on("quick_status_action", null);
    render(<QuickStatusPage />);
    const labels = ["Open DevPilot", "Run Diagnosis", "Scan Storage", "Settings", "Quit"];
    const ids = ["open", "diagnosis", "scan_storage", "settings", "quit"];
    expect(screen.getAllByRole("button").map((b) => b.textContent)).toEqual(labels);
    for (const [i, label] of labels.entries()) {
      await user.click(screen.getByRole("button", { name: label }));
      expect(ipc.calledWith("quick_status_action")[i].args).toEqual({ action: ids[i] });
    }
  });

  it("says so when no status can be read, and still offers the buttons", async () => {
    render(<QuickStatusPage />);
    expect(await screen.findByText("Status unavailable.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open DevPilot" })).toBeInTheDocument();
  });

  it("calls only the cheap status command: no diagnosis, docker or scan", async () => {
    ipc.on("get_quick_status", status());
    render(<QuickStatusPage />);
    await screen.findByText("High");
    for (const heavy of ["run_diagnosis", "get_docker_status", "get_docker_breakdown", "start_storage_scan", "get_processes"]) {
      expect(ipc.calledWith(heavy)).toHaveLength(0);
    }
  });
});

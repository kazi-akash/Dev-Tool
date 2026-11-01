import { describe, expect, it } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { EmergencyBanner } from "@/components/EmergencyBanner";
import type { EmergencyReport } from "@/types/api";
import { GB, preview, snapshot } from "./fixtures";
import { ipc } from "./ipc-mock";

const report = (over: Partial<EmergencyReport> = {}): EmergencyReport => ({
  ramUsedBytes: 15.5 * GB,
  ramTotalBytes: 16 * GB,
  ramPercent: 97,
  commitPercent: 97,
  diskActivePercent: 100,
  hardFaultsPerSec: 900,
  causes: [
    { name: "Android/Gradle", bytes: 4.2 * GB },
    { name: "Docker / WSL2", bytes: 4.8 * GB },
  ],
  causeSummary: "Android/Gradle + Docker / WSL2",
  mechanism: "RAM reached 97%, but the main cause of system slowdown appears to be heavy paging.",
  steps: [
    { id: "gradle-stop", title: "Stop Gradle daemons", why: "1 Gradle daemon(s) hold JVM memory.", action: { actionId: "gradle.stop", params: {}, label: "Stop Gradle daemons" }, risk: "LOW", estimatedBytes: 4.2 * GB, estimateBasis: "memory of the running Gradle daemon processes" },
    { id: "emulator-close", title: "Close the Android emulator", why: "qemu holds 2.0 GB.", action: { actionId: "process.terminate", params: { pid: 30, name: "qemu-system-x86_64.exe" }, label: "Close qemu" }, risk: "MEDIUM", estimatedBytes: 2 * GB, estimateBasis: "memory of the emulator process" },
  ],
  estimatedRecoveryBytes: 6.2 * GB,
  recoveryNote: "Estimates are upper bounds from measured memory. Steps overlap, so the total is not their sum.",
  incidentActive: true,
  activeSessions: ["Android build"],
  ...over,
});

const critical = () => snapshot({ pressure: { ...snapshot().pressure!, state: "CRITICAL", percent: 97 } });

describe("emergency banner", () => {
  it("shows nothing and does not query the backend while pressure is not critical", async () => {
    ipc.on("get_system_snapshot", snapshot());
    ipc.on("get_emergency_status", report());
    render(<EmergencyBanner />);
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(ipc.calledWith("get_emergency_status")).toHaveLength(0);
  });

  it("explains the state, the likely cause and the recovery steps with an honest estimate", async () => {
    ipc.on("get_system_snapshot", critical());
    ipc.on("get_emergency_status", report());
    render(<EmergencyBanner />);
    const banner = await screen.findByRole("alert");
    expect(within(banner).getByText("CRITICAL SYSTEM PRESSURE")).toBeInTheDocument();
    expect(within(banner).getByText("recording incident")).toBeInTheDocument();
    expect(within(banner).getByText("Android build")).toBeInTheDocument();
    expect(within(banner).getByText(/15.5 GB \/ 16.0 GB/)).toBeInTheDocument();
    expect(within(banner).getByText(/appears to be heavy paging/)).toBeInTheDocument();
    expect(within(banner).getByText(/Likely contributors:/)).toBeInTheDocument();
    expect(within(banner).getByText("Stop Gradle daemons")).toBeInTheDocument();
    expect(within(banner).getByText(/up to 4.2 GB/)).toBeInTheDocument();
    expect(within(banner).getByText(/not their sum/)).toBeInTheDocument();
  });

  it("shows unavailable counters as unavailable", async () => {
    ipc.on("get_system_snapshot", critical());
    ipc.on("get_emergency_status", report({ commitPercent: null, diskActivePercent: null, hardFaultsPerSec: null, mechanism: null }));
    render(<EmergencyBanner />);
    const banner = await screen.findByRole("alert");
    expect(within(banner).getAllByText("unavailable")).toHaveLength(3);
  });

  it("a step only opens the preview dialog; nothing executes without confirmation", async () => {
    const user = userEvent.setup();
    ipc.on("get_system_snapshot", critical());
    ipc.on("get_emergency_status", report());
    ipc.on("preview_action", preview({ actionId: "gradle.stop", title: "Stop Gradle daemons", risk: "LOW", requiresConfirmation: false, requiresDoubleConfirmation: false, reversibility: "RECOVERABLE", warnings: [], estimatedBenefitBytes: null }));
    render(<EmergencyBanner />);
    await user.click(await screen.findByRole("button", { name: "Review: Stop Gradle daemons" }));
    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText("Recoverable")).toBeInTheDocument();
    expect(ipc.calledWith("preview_action")[0].args).toMatchObject({ actionId: "gradle.stop", params: {} });
    expect(ipc.calledWith("execute_action")).toHaveLength(0);
  });

  it("with no safe step it says so instead of inventing one", async () => {
    ipc.on("get_system_snapshot", critical());
    ipc.on("get_emergency_status", report({ steps: [], estimatedRecoveryBytes: null, recoveryNote: "No safe recovery step was found automatically. Close applications you are not using." }));
    render(<EmergencyBanner />);
    expect(await screen.findByText(/No safe recovery step was found/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Review/ })).not.toBeInTheDocument();
  });
});

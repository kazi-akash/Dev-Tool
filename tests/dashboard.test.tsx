import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import type { SystemSnapshot } from "@/types/system";

const GB = 1024 ** 3;
const snapshot: SystemSnapshot = {
  timestampMs: 1,
  memory: { totalBytes: 16 * GB, usedBytes: 14 * GB, availableBytes: 2 * GB, percent: 87.5 },
  cpuPercent: 38,
  disks: [
    { name: "Windows", mountPoint: "C:\\", totalBytes: 200 * GB, usedBytes: 188 * GB, freeBytes: 12 * GB, usedPercent: 94 },
  ],
  uptimeSecs: 3700,
  processCount: 321,
};

vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (cmd: string) => Promise.resolve(cmd === "get_system_snapshot" ? snapshot : undefined),
}));

import { DashboardPage } from "@/features/dashboard/DashboardPage";

describe("dashboard", () => {
  it("renders live metrics from the monitor snapshot", async () => {
    render(<DashboardPage />);
    expect(await screen.findByText("38%")).toBeInTheDocument();
    expect(screen.getByText("88%")).toBeInTheDocument();
    expect(screen.getByText(/14\.0 GB \/ 16\.0 GB/)).toBeInTheDocument();
    expect(screen.getByText("321 processes")).toBeInTheDocument();
    expect(screen.getByText("12.0 GB free")).toBeInTheDocument();
    expect(screen.getByRole("progressbar", { name: "RAM" })).toHaveAttribute("aria-valuenow", "88");
  });
});

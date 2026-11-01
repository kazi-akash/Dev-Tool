import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { SessionList } from "@/features/history/SessionList";
import type { BuildSession } from "@/types/api";
import { GB } from "./fixtures";

const session = (over: Partial<BuildSession> = {}): BuildSession => ({
  id: 1,
  kind: "ANDROID_BUILD",
  label: "Android build",
  startMs: 1_700_000_000_000,
  endMs: 1_700_000_060_000,
  durationSecs: 60,
  ramBeforeBytes: 8.2 * GB,
  ramPeakBytes: 15.4 * GB,
  ramPeakPercent: 96,
  cpuPeakPercent: 80,
  diskPeakPercent: null,
  processesCreated: 12,
  workloadGrowth: [{ name: "Android/Gradle", bytes: 4.3 * GB }],
  dockerChangeBytes: 1.7 * GB,
  result: "CRITICAL MEMORY PRESSURE",
  correlation: { level: "STRONG", incidentId: 42, statement: "Strong correlation detected. This android build overlapped incident #0042." },
  ...over,
});

describe("session list", () => {
  it("shows before/peak, growth, host-memory docker change and the correlation wording", () => {
    render(<SessionList available items={[session()]} />);
    expect(screen.getByText("Android build")).toBeInTheDocument();
    expect(screen.getByText("CRITICAL MEMORY PRESSURE")).toBeInTheDocument();
    expect(screen.getByText(/RAM before 8.2 GB · peak 15.4 GB/)).toBeInTheDocument();
    expect(screen.getByText(/Android\/Gradle \+4.3 GB/)).toBeInTheDocument();
    expect(screen.getByText(/Docker\/WSL2 host memory \+1.7 GB/)).toBeInTheDocument();
    expect(screen.getByText(/disk peak unavailable/)).toBeInTheDocument();
    expect(screen.getByText(/^Strong correlation detected/)).toBeInTheDocument();
  });

  it("hides the correlation line when no incident overlapped", () => {
    render(<SessionList available items={[session({ correlation: { level: "NONE", incidentId: null, statement: "No performance incident overlapped this android build." } })]} />);
    expect(screen.queryByText(/No performance incident overlapped/)).not.toBeInTheDocument();
  });

  it("explains empty and unavailable states", () => {
    const { rerender } = render(<SessionList available items={[]} />);
    expect(screen.getByText(/No builds recorded/)).toBeInTheDocument();
    rerender(<SessionList available={false} items={undefined} />);
    expect(screen.getByText(/History is unavailable/)).toBeInTheDocument();
  });
});

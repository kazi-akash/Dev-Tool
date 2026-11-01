import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { IncidentList } from "@/features/history/IncidentList";
import type { IncidentSummary } from "@/types/api";
import { GB } from "./fixtures";

const incident = (over: Partial<IncidentSummary> = {}): IncidentSummary => ({
  id: 42,
  startMs: 1_700_000_000_000,
  endMs: 1_700_000_048_000,
  durationSecs: 48,
  triggers: ["RAM above the critical threshold"],
  peakRamPercent: 98,
  peakRamUsedBytes: 15.7 * GB,
  minAvailableBytes: 0.3 * GB,
  peakCommitBytes: 19.1 * GB,
  peakCommitPercent: 98,
  peakPagefilePercent: 80,
  peakHardFaultsPerSec: 900,
  peakDiskPercent: 100,
  peakCpuPercent: 60,
  contributors: [
    { name: "WSL2", bytes: 4.8 * GB },
    { name: "Gradle", bytes: 4.2 * GB },
    { name: "Next.js", bytes: 2.6 * GB },
  ],
  sessions: [],
  mechanism: "HEAVY_PAGING",
  confidence: 0.9,
  diagnosis: "Memory exhaustion caused heavy paging, which saturated the disk and likely caused the system responsiveness problem.",
  evidence: ["commit charge is 98% of the limit"],
  ...over,
});

describe("incident list", () => {
  it("shows peaks, contributors, hedged diagnosis, confidence and evidence", () => {
    render(<IncidentList available items={[incident()]} />);
    expect(screen.getByText(/Incident #0042/)).toBeInTheDocument();
    expect(screen.getByText(/Primary contributors: WSL2 4.8 GB, Gradle 4.2 GB, Next.js 2.6 GB/)).toBeInTheDocument();
    expect(screen.getByText(/likely caused/)).toBeInTheDocument();
    expect(screen.getByText(/Confidence 90%/)).toBeInTheDocument();
    expect(screen.getByText("commit charge is 98% of the limit")).toBeInTheDocument();
  });

  it("says unavailable for counters that were not measured", () => {
    render(<IncidentList available items={[incident({ peakCommitPercent: null, peakDiskPercent: null, peakHardFaultsPerSec: null })]} />);
    expect(screen.getByText(/commit unavailable · disk unavailable · hard faults unavailable/)).toBeInTheDocument();
  });

  it("explains empty and unavailable states", () => {
    const { rerender } = render(<IncidentList available items={[]} />);
    expect(screen.getByText(/No incidents recorded/)).toBeInTheDocument();
    rerender(<IncidentList available={false} items={undefined} />);
    expect(screen.getByText(/History is unavailable/)).toBeInTheDocument();
  });
});

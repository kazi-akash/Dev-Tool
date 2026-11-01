import type { ActionPreview, Diagnosis, DiagnosisReport, ScanEntry, ScanStatus, Workload } from "@/types/api";
import type { SystemSnapshot } from "@/types/system";

export const GB = 1024 ** 3;

export function snapshot(over: Partial<SystemSnapshot> = {}): SystemSnapshot {
  return {
    timestampMs: 1,
    memory: { totalBytes: 16 * GB, usedBytes: 14 * GB, availableBytes: 2 * GB, percent: 87.5 },
    cpuPercent: 38,
    disks: [{ name: "Win", mountPoint: "C:\\", totalBytes: 275 * GB, usedBytes: 269 * GB, freeBytes: 6 * GB, usedPercent: 98 }],
    diskHealth: [
      {
        mountPoint: "C:\\",
        state: "CRITICAL",
        freePercent: 2.2,
        freeBytes: 6 * GB,
        totalBytes: 275 * GB,
        trendBytesPerHour: -3 * GB,
        hoursToCritical: null,
        hoursToFull: 2,
        trendConfidence: 0.9,
        reasons: [],
      },
    ],
    pressure: {
      state: "HIGH",
      trend: "RISING",
      percent: 87.5,
      availableBytes: 2 * GB,
      commitPercent: null,
      pagefilePercent: null,
      ratePerMin: 1.2,
      minutesToCritical: 4,
      reasons: ["RAM usage is 88%"],
    },
    telemetry: null,
    uptimeSecs: 3700,
    processCount: 321,
    ...over,
  };
}

export function diagnosis(over: Partial<Diagnosis> = {}): Diagnosis {
  return {
    ruleId: "memory-pressure",
    severity: "high",
    title: "Memory pressure detected",
    summary: "RAM is at 88%. Docker/WSL2 and Android Gradle build account for approximately 8.9 GB, making them the primary contributors.",
    primaryCause: "Docker/WSL2",
    contributors: [
      { name: "Docker/WSL2", memoryBytes: 4.8 * GB, percent: 30, processCount: 3, detail: "vmmemWSL" },
      { name: "Android Gradle build", memoryBytes: 4.1 * GB, percent: 25.6, processCount: 2, detail: "java.exe" },
    ],
    confidence: 0.91,
    evidence: ["RAM usage is 88%"],
    recommendations: [
      {
        id: "docker-stop-aaa",
        problem: "Container redis-idle looks idle",
        explanation: "It uses 900 MB but its CPU is under 1%.",
        expectedBenefit: "About 900.0 MB of RAM",
        expectedBenefitBytes: 900 * 1024 * 1024,
        risk: "low",
        action: { actionId: "docker.stop_container", params: { container: "redis-idle" }, label: "Stop container redis-idle" },
        manualSteps: null,
      },
      {
        id: "wsl-memory-limit",
        problem: "WSL2 has no memory limit",
        explanation: "By default WSL2 can take up to half of your RAM.",
        expectedBenefit: "Caps Docker/WSL2 memory permanently",
        expectedBenefitBytes: null,
        risk: "low",
        action: null,
        manualSteps: "Create %UserProfile%\\.wslconfig with:\n[wsl2]\nmemory=6GB",
      },
    ],
    ...over,
  };
}

export function report(over: Partial<DiagnosisReport> = {}, headline: Diagnosis = diagnosis()): DiagnosisReport {
  return { generatedAtMs: 1_700_000_000_000, status: "problem", headline, findings: [headline], ...over };
}

export function workloads(): Workload[] {
  return [
    {
      kind: "docker",
      name: "Docker / WSL2",
      memoryBytes: 4.8 * GB,
      cpuPercent: 3,
      processCount: 3,
      members: [
        { label: "WSL2 VM (vmmem)", processCount: 1, memoryBytes: 4.4 * GB },
        { label: "Docker Desktop", processCount: 2, memoryBytes: 0.4 * GB },
      ],
      topProcesses: [{ pid: 1, name: "vmmemWSL", memoryBytes: 4.4 * GB }],
      signals: ["wsl-vm"],
    },
    {
      kind: "node",
      name: "Next.js / Node.js",
      memoryBytes: 2.4 * GB,
      cpuPercent: 2,
      processCount: 4,
      members: [{ label: "Next.js", processCount: 2, memoryBytes: 2 * GB }],
      topProcesses: [],
      signals: ["nextjs"],
    },
  ];
}

export function entry(over: Partial<ScanEntry> = {}): ScanEntry {
  return {
    path: "S:\\proj\\node_modules",
    name: "node_modules",
    isDir: true,
    sizeBytes: 6.4 * GB,
    fileCount: 52000,
    lastModifiedSecs: 1_700_000_000,
    kind: "nodeModules",
    kindLabel: "Developer dependency",
    safety: "REQUIRES_REVIEW",
    reason: "Reinstallable with the package manager, but this project may be in use.",
    ...over,
  };
}

export function scanStatus(entries: ScanEntry[], over: Partial<ScanStatus> = {}): ScanStatus {
  return {
    running: false,
    progress: { phase: "Done", currentPath: "", entriesFound: entries.length, itemsVisited: 1000 },
    error: null,
    result: { startedAtMs: 1, durationMs: 4200, complete: true, unreadable: 0, notes: [], entries },
    ...over,
  };
}

export function preview(over: Partial<ActionPreview> = {}): ActionPreview {
  return {
    actionId: "cleanup.path",
    title: "Delete node_modules",
    summary: "6.4 GB Reinstallable with the package manager.",
    risk: "DESTRUCTIVE",
    items: [{ label: "S:\\proj\\node_modules", detail: "NodeModules", bytes: 6.4 * GB }],
    estimatedBenefitBytes: 6.4 * GB,
    warnings: ["Deleted permanently. It is not moved to the Recycle Bin."],
    requiresConfirmation: true,
    requiresDoubleConfirmation: true,
    reversible: false,
    ...over,
  };
}

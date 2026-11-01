import type { Page } from "@playwright/test";

const GB = 1024 ** 3;

/** Canned backend responses, shaped exactly like the Rust commands' JSON. */
export function defaultMocks(): Record<string, unknown> {
  const snapshot = {
    timestampMs: Date.now(),
    memory: { totalBytes: 16 * GB, usedBytes: 14 * GB, availableBytes: 2 * GB, percent: 87.5 },
    cpuPercent: 38,
    disks: [{ name: "Win", mountPoint: "C:\\", totalBytes: 275 * GB, usedBytes: 269 * GB, freeBytes: 6 * GB, usedPercent: 98 }],
    diskHealth: [{ mountPoint: "C:\\", state: "CRITICAL", freePercent: 2.2, freeBytes: 6 * GB, totalBytes: 275 * GB, trendBytesPerHour: -3 * GB, hoursToCritical: null, hoursToFull: 2, trendConfidence: 0.9, reasons: [] }],
    pressure: { state: "HIGH", trend: "RISING", percent: 87.5, availableBytes: 2 * GB, commitPercent: null, pagefilePercent: null, ratePerMin: 1.2, minutesToCritical: 4, reasons: ["RAM usage is 88%"] },
    uptimeSecs: 3700,
    processCount: 321,
  };
  const headline = {
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
        id: "wsl-shutdown",
        problem: "The WSL2 VM is holding memory while no containers are running",
        explanation: "WSL2 keeps its memory after workloads stop.",
        expectedBenefit: "About 4.4 GB of RAM",
        expectedBenefitBytes: 4.4 * GB,
        risk: "medium",
        action: { actionId: "wsl.shutdown", params: {}, label: "Shut down WSL" },
        manualSteps: null,
      },
    ],
  };
  const now = Date.now();
  const rows = Array.from({ length: 60 }, (_, i) => ({
    tsMs: now - (60 - i) * 60_000,
    samples: 30,
    ramAvg: 60 + i * 0.4,
    ramMax: 63 + i * 0.4,
    cpuAvg: 20 + (i % 10) * 3,
    cpuMax: 40 + (i % 10) * 3,
    ramUsedAvg: 1,
    ramTotal: 2,
    disks: [{ mount: "C:\\", freeBytes: (40 - i * 0.2) * GB, totalBytes: 275 * GB }],
    top: [],
  }));
  return {
    get_system_snapshot: snapshot,
    run_diagnosis: { generatedAtMs: now, status: "problem", headline, findings: [headline] },
    get_workloads: [
      { kind: "docker", name: "Docker / WSL2", memoryBytes: 4.8 * GB, cpuPercent: 3, processCount: 3, members: [{ label: "WSL2 VM (vmmem)", processCount: 1, memoryBytes: 4.4 * GB }], topProcesses: [], signals: ["wsl-vm"] },
    ],
    get_memory_growth: [],
    get_recent_diagnoses: { available: true, items: [] },
    get_recent_actions: { available: true, items: [] },
    get_incidents: { available: true, items: [] },
    get_incident_status: { active: false, sinceMs: null },
    get_build_sessions: { available: true, items: [] },
    get_history: { available: true, items: rows },
    get_processes: {
      total: 2,
      matched: 2,
      processes: [
        { pid: 1, name: "vmmemWSL", exePath: null, commandLine: null, parentPid: null, startTime: 1, cpuPercent: 1, memoryBytes: 4.4 * GB, category: "DOCKER", evidence: "name", tags: [] },
        { pid: 2, name: "node.exe", exePath: "C:\\node.exe", commandLine: "node next dev --token ***", parentPid: 1, startTime: 1, cpuPercent: 2, memoryBytes: 1 * GB, category: "NODE", evidence: "commandLine", tags: ["nextjs"] },
      ],
    },
    get_docker_status: { available: false, error: "Docker CLI not found", containers: [], totalMemoryBytes: 0 },
    get_storage_scan: { running: false, progress: { phase: "", currentPath: "", entriesFound: 0, itemsVisited: 0 }, result: null, error: null },
    list_actions: [{ id: "wsl.shutdown", name: "Shut down WSL", description: "Runs wsl --shutdown.", riskLevel: "MEDIUM", estimatedBenefit: "Returns memory", automationEligible: false, reversible: false }],
    get_action_history: [],
    ai_key_status: false,
    take_pending_route: null,
    set_monitor_interval: null,
    set_pressure_thresholds: null,
    set_disk_thresholds: null,
    set_safe_automation: null,
  };
}

/**
 * Replace the Tauri bridge before the page loads. `mocks` values are returned as-is (or computed if
 * given as a function source string via `fns`). Every call is recorded on `window.__calls`.
 */
export async function installBridge(page: Page, overrides: Record<string, unknown> = {}, fns: Record<string, string> = {}) {
  const mocks = { ...defaultMocks(), ...overrides };
  await page.addInitScript(
    ({ mocks, fns }) => {
      const w = window as unknown as Record<string, any>;
      w.__calls = [] as { cmd: string; args: unknown }[];
      w.__mocks = mocks;
      const callbacks = new Map<number, (e: unknown) => void>();
      const listeners: { event: string; id: number }[] = [];
      let next = 1;
      w.__emit = (event: string, payload: unknown) => {
        for (const l of listeners.filter((x) => x.event === event)) callbacks.get(l.id)?.({ event, id: l.id, payload });
      };
      const handlers: Record<string, (args: any) => unknown> = {};
      for (const [k, src] of Object.entries(fns)) handlers[k] = new Function("args", `return (${src})(args)`) as (a: unknown) => unknown;
      w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
      w.__TAURI_INTERNALS__ = {
        transformCallback(cb: (e: unknown) => void) {
          const id = next++;
          callbacks.set(id, cb);
          return id;
        },
        unregisterCallback(id: number) {
          callbacks.delete(id);
        },
        convertFileSrc: (p: string) => p,
        async invoke(cmd: string, args: any) {
          w.__calls.push({ cmd, args });
          if (cmd === "plugin:event|listen") {
            listeners.push({ event: args.event, id: args.handler });
            return args.handler;
          }
          if (cmd === "plugin:event|unlisten") return;
          if (handlers[cmd]) return handlers[cmd](args);
          if (!(cmd in w.__mocks)) throw `no mock for ${cmd}`;
          return w.__mocks[cmd];
        },
      };
    },
    { mocks, fns },
  );
}

export async function calls(page: Page, cmd: string): Promise<{ cmd: string; args: any }[]> {
  return page.evaluate((c) => (window as any).__calls.filter((x: { cmd: string }) => x.cmd === c), cmd);
}

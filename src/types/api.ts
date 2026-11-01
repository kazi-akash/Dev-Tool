/** Mirrors the Rust command payloads: workloads, Docker, diagnosis, storage, actions, history, AI. */

// --- workloads & Docker --------------------------------------------------------------------------
export type WorkloadKind =
  | "docker"
  | "android"
  | "flutter"
  | "laragon"
  | "laravel"
  | "node"
  | "java"
  | "database"
  | "ide"
  | "browser";

export interface WorkloadMember {
  label: string;
  processCount: number;
  memoryBytes: number;
}

export interface Workload {
  kind: WorkloadKind;
  name: string;
  memoryBytes: number;
  cpuPercent: number;
  processCount: number;
  members: WorkloadMember[];
  topProcesses: { pid: number; name: string; memoryBytes: number }[];
  signals: string[];
}

export interface DockerContainer {
  id: string;
  name: string;
  image: string;
  status: string;
  running: boolean;
  memoryBytes: number | null;
  cpuPercent: number | null;
}

export interface DockerStatus {
  available: boolean;
  error: string | null;
  containers: DockerContainer[];
  totalMemoryBytes: number;
}

// --- diagnosis -----------------------------------------------------------------------------------
export type Severity = "info" | "warning" | "high" | "critical";
export type Risk = "low" | "medium" | "high";

export interface Contributor {
  name: string;
  memoryBytes: number;
  percent: number;
  processCount: number;
  detail: string | null;
}

export interface ActionRef {
  actionId: string;
  params: Record<string, unknown>;
  label: string;
}

export interface Recommendation {
  id: string;
  problem: string;
  explanation: string;
  expectedBenefit: string;
  expectedBenefitBytes: number | null;
  risk: Risk;
  action: ActionRef | null;
  manualSteps: string | null;
}

export interface Diagnosis {
  ruleId: string;
  severity: Severity;
  title: string;
  summary: string;
  primaryCause: string | null;
  contributors: Contributor[];
  confidence: number;
  evidence: string[];
  recommendations: Recommendation[];
}

export interface DiagnosisReport {
  generatedAtMs: number;
  status: "healthy" | "attention" | "problem";
  headline: Diagnosis;
  findings: Diagnosis[];
}

export interface GrowthFinding {
  pid: number;
  name: string;
  startBytes: number;
  currentBytes: number;
  growthBytes: number;
  growthPercent: number;
  bytesPerMin: number;
  windowMinutes: number;
  confidence: number;
  summary: string;
}

// --- storage -------------------------------------------------------------------------------------
export type EntryKind =
  | "tempCache"
  | "packageCache"
  | "gradleCache"
  | "androidSdk"
  | "androidAvd"
  | "flutterCache"
  | "dockerData"
  | "wslDisk"
  | "nodeModules"
  | "buildOutput"
  | "apk"
  | "archive"
  | "diskImage"
  | "video"
  | "installer"
  | "virtualDisk"
  | "userData"
  | "largeFile"
  | "unknown";

export type Safety = "SAFE_TO_REVIEW" | "SAFE_TO_CLEAN" | "REQUIRES_REVIEW" | "DO_NOT_TOUCH";

export interface ScanEntry {
  path: string;
  name: string;
  isDir: boolean;
  sizeBytes: number;
  fileCount: number;
  lastModifiedSecs: number | null;
  kind: EntryKind;
  kindLabel: string;
  safety: Safety;
  reason: string;
}

export interface ScanResult {
  startedAtMs: number;
  durationMs: number;
  complete: boolean;
  unreadable: number;
  notes: string[];
  entries: ScanEntry[];
}

export interface ScanProgress {
  phase: string;
  currentPath: string;
  entriesFound: number;
  itemsVisited: number;
}

export interface ScanStatus {
  running: boolean;
  progress: ScanProgress;
  result: ScanResult | null;
  error: string | null;
}

// --- actions -------------------------------------------------------------------------------------
export type RiskLevel = "SAFE" | "LOW" | "MEDIUM" | "HIGH" | "DESTRUCTIVE";

export interface ActionSpec {
  id: string;
  name: string;
  description: string;
  riskLevel: RiskLevel;
  estimatedBenefit: string;
  automationEligible: boolean;
  reversible: boolean;
}

export interface PreviewItem {
  label: string;
  detail: string | null;
  bytes: number | null;
}

export interface ActionPreview {
  actionId: string;
  title: string;
  summary: string;
  risk: RiskLevel;
  items: PreviewItem[];
  estimatedBenefitBytes: number | null;
  warnings: string[];
  requiresConfirmation: boolean;
  requiresDoubleConfirmation: boolean;
  reversible: boolean;
}

export interface ActionRecord {
  id: number;
  actionId: string;
  name: string;
  atMs: number;
  risk: RiskLevel;
  automated: boolean;
  success: boolean;
  summary: string;
  freedBytes: number | null;
  rolledBack: boolean;
  reversible: boolean;
}

/** Errors from the action engine and AI gateway arrive as `{ code, message }`. */
export interface CodedError {
  code: string;
  message: string;
}

// --- history -------------------------------------------------------------------------------------
export interface HistoryRow {
  tsMs: number;
  samples: number;
  ramAvg: number;
  ramMax: number;
  cpuAvg: number;
  cpuMax: number;
  ramUsedAvg: number;
  ramTotal: number;
  disks: { mount: string; freeBytes: number; totalBytes: number }[];
  top: { name: string; memoryBytes: number }[];
}

export interface HistoryResponse<T> {
  available: boolean;
  items: T[];
}

export interface StoredDiagnosis {
  id: number;
  tsMs: number;
  status: string;
  severity: Severity;
  ruleId: string;
  title: string;
  summary: string;
  primaryCause: string | null;
  confidence: number;
}

export interface StoredAction {
  id: number;
  tsMs: number;
  actionId: string;
  name: string;
  risk: string;
  automated: boolean;
  success: boolean;
  summary: string;
  freedBytes: number | null;
}

// --- AI ------------------------------------------------------------------------------------------
export type ProviderKind = "openAi" | "anthropic";

export interface AiConfig {
  kind: ProviderKind;
  baseUrl: string;
  model: string;
}

export interface AiPayload {
  system: {
    ram: string;
    cpu: string;
    pressure: string;
    trend: string;
    disks: { drive: string; free: string; total: string; state: string }[];
  };
  workloads: { name: string; memory: string; processes: number; members: string[] }[];
  topProcesses: { name: string; category: string; memory: string }[];
  diagnosisStatus: string;
  findings: unknown[];
}

// --- performance incidents ---------------------------------------------------------------------
export interface IncidentContributor {
  name: string;
  bytes: number;
}

export interface IncidentSummary {
  id: number;
  startMs: number;
  endMs: number;
  durationSecs: number;
  triggers: string[];
  peakRamPercent: number;
  peakRamUsedBytes: number;
  minAvailableBytes: number;
  peakCommitBytes: number | null;
  peakCommitPercent: number | null;
  peakPagefilePercent: number | null;
  peakHardFaultsPerSec: number | null;
  peakDiskPercent: number | null;
  peakCpuPercent: number;
  contributors: IncidentContributor[];
  sessions: string[];
  mechanism: "NONE" | "HIGH_RAM_STABLE" | "HEAVY_PAGING" | "COMMIT_EXHAUSTION" | "DISK_SATURATION";
  confidence: number;
  diagnosis: string;
  evidence: string[];
}

// --- build sessions ----------------------------------------------------------------------------
export interface SessionCorrelation {
  level: "STRONG" | "POSSIBLE" | "NONE";
  incidentId: number | null;
  statement: string;
}

export interface BuildSession {
  id: number;
  kind: string;
  label: string;
  startMs: number;
  endMs: number;
  durationSecs: number;
  ramBeforeBytes: number;
  ramPeakBytes: number;
  ramPeakPercent: number;
  cpuPeakPercent: number;
  diskPeakPercent: number | null;
  processesCreated: number;
  workloadGrowth: IncidentContributor[];
  dockerChangeBytes: number;
  result: string;
  correlation: SessionCorrelation | null;
}

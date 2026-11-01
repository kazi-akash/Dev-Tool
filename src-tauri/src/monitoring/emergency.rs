//! Emergency recovery assessment. Pure: everything it looks at is passed in.
//!
//! When memory pressure is CRITICAL this builds a plain-language picture (what is exhausted, who
//! holds the memory) and a short list of recovery steps. A step is only a *reference* to an action
//! the action engine already knows (`ActionRef`): it is previewed, confirmed and verified like any
//! other action, through the same command ACL. Nothing here executes anything, and nothing here
//! reads AI output; steps are derived from measured process and container facts only.

use serde::Serialize;
use serde_json::json;

use super::incident::NamedBytes;
use super::metrics::SystemSnapshot;
use super::pressure::PressureState;
use crate::developers::workloads::{Workload, WorkloadKind};
use crate::diagnosis::model::ActionRef;
use crate::docker::DockerStatus;
use crate::processes::model::{Category, ProcessInfo};
use crate::util::fmt_bytes;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;
/// A workload counts as a likely cause above this size or share of installed RAM.
const CAUSE_MIN_BYTES: u64 = GIB;
const CAUSE_MIN_SHARE: f32 = 0.10;
const MAX_CAUSES: usize = 3;
/// Containers below this CPU share are "idle" candidates (same threshold as the diagnosis rules).
const IDLE_CPU: f32 = 1.0;
const MIN_CONTAINER_BYTES: u64 = 50 * MIB;
const MAX_CONTAINER_STEPS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StepRisk {
    Low,
    Medium,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryStep {
    /// Stable id for the UI.
    pub id: String,
    pub title: String,
    pub why: String,
    /// The action to preview. Executing it still needs the normal preview and confirmation.
    pub action: ActionRef,
    pub risk: StepRisk,
    /// What the step may free, when known.
    pub estimated_bytes: Option<u64>,
    /// Where the estimate comes from, so it is not mistaken for a promise.
    pub estimate_basis: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmergencyReport {
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    pub ram_percent: f32,
    pub commit_percent: Option<f32>,
    pub disk_active_percent: Option<f32>,
    pub hard_faults_per_sec: Option<f32>,
    /// Largest workloads holding memory, largest first.
    pub causes: Vec<NamedBytes>,
    /// For example "Gradle + Docker/WSL2". Empty when no workload stands out.
    pub cause_summary: String,
    /// The mechanism sentence from memory telemetry, when available.
    pub mechanism: Option<String>,
    pub steps: Vec<RecoveryStep>,
    /// Combined estimate over non-overlapping steps; `None` when no step has an estimate.
    pub estimated_recovery_bytes: Option<u64>,
    pub recovery_note: String,
    pub incident_active: bool,
    pub active_sessions: Vec<String>,
}

pub struct EmergencyInput<'a> {
    pub snapshot: &'a SystemSnapshot,
    pub workloads: &'a [Workload],
    pub processes: &'a [ProcessInfo],
    pub docker: Option<&'a DockerStatus>,
    pub incident_active: bool,
    pub active_sessions: Vec<String>,
}

fn gradle_daemon_bytes(processes: &[ProcessInfo]) -> (u64, usize) {
    let daemons = processes
        .iter()
        .filter(|p| p.tags.contains(&"gradle"))
        .filter(|p| {
            p.command_line
                .as_deref()
                .is_some_and(|c| c.to_lowercase().contains("gradledaemon"))
        });
    daemons.fold((0, 0), |(b, n), p| (b + p.memory_bytes, n + 1))
}

fn is_emulator(p: &ProcessInfo) -> bool {
    let n = p.name.to_lowercase();
    let n = n.strip_suffix(".exe").unwrap_or(&n);
    p.category != Category::System && (n.starts_with("qemu-system") || n == "emulator")
}

/// `None` unless pressure is CRITICAL (the only state that justifies an emergency banner).
pub fn assess(i: &EmergencyInput) -> Option<EmergencyReport> {
    let pressure = i.snapshot.pressure.as_ref()?;
    if pressure.state != PressureState::Critical {
        return None;
    }
    let total = i.snapshot.memory.total_bytes;
    let t = i.snapshot.telemetry.as_ref();

    let causes: Vec<NamedBytes> = i
        .workloads
        .iter()
        .filter(|w| {
            w.memory_bytes >= CAUSE_MIN_BYTES
                || (total > 0 && w.memory_bytes as f32 / total as f32 >= CAUSE_MIN_SHARE)
        })
        .take(MAX_CAUSES)
        .map(|w| NamedBytes {
            name: w.name.clone(),
            bytes: w.memory_bytes,
        })
        .collect();
    let cause_summary = causes
        .iter()
        .map(|c| c.name.as_str())
        .collect::<Vec<_>>()
        .join(" + ");

    let mut steps: Vec<RecoveryStep> = Vec::new();

    // 1. Gradle daemons: only when a daemon is actually running.
    let (gradle_bytes, gradle_n) = gradle_daemon_bytes(i.processes);
    if gradle_n > 0 {
        steps.push(RecoveryStep {
            id: "gradle-stop".into(),
            title: "Stop Gradle daemons".into(),
            why: format!(
                "{gradle_n} Gradle daemon(s) hold JVM memory. The next build starts a new one."
            ),
            action: ActionRef {
                action_id: "gradle.stop".into(),
                params: json!({}),
                label: "Stop Gradle daemons".into(),
            },
            risk: StepRisk::Low,
            estimated_bytes: Some(gradle_bytes),
            estimate_basis: "memory of the running Gradle daemon processes".into(),
        });
    }

    // 2. Idle containers (largest first). Container-reported memory sits inside the WSL2 VM.
    let docker_ok = i.docker.filter(|d| d.available);
    let mut container_steps: Vec<RecoveryStep> = Vec::new();
    if let Some(d) = docker_ok {
        let mut idle: Vec<_> = d
            .containers
            .iter()
            .filter(|c| c.running && crate::docker::is_valid_container_ref(&c.name))
            .filter(|c| c.cpu_percent.is_some_and(|cpu| cpu < IDLE_CPU))
            .filter(|c| c.memory_bytes.is_some_and(|m| m >= MIN_CONTAINER_BYTES))
            .collect();
        idle.sort_by_key(|c| std::cmp::Reverse(c.memory_bytes));
        for c in idle.into_iter().take(MAX_CONTAINER_STEPS) {
            container_steps.push(RecoveryStep {
                id: format!("container-{}", c.name),
                title: format!("Stop idle container {}", c.name),
                why: format!(
                    "{} is running at {:.1}% CPU and reports {}.",
                    c.name,
                    c.cpu_percent.unwrap_or(0.0),
                    fmt_bytes(c.memory_bytes.unwrap_or(0))
                ),
                action: ActionRef {
                    action_id: "docker.stop_container".into(),
                    params: json!({ "container": c.name }),
                    label: format!("Stop {}", c.name),
                },
                risk: StepRisk::Low,
                estimated_bytes: c.memory_bytes,
                estimate_basis: "container-reported memory; the WSL2 VM may keep it after the stop"
                    .into(),
            });
        }
    }
    let container_sum: u64 = container_steps
        .iter()
        .filter_map(|s| s.estimated_bytes)
        .sum();
    steps.extend(container_steps);

    // 3. Android emulator.
    let emulator = i
        .processes
        .iter()
        .filter(|p| is_emulator(p))
        .max_by_key(|p| p.memory_bytes);
    if let Some(p) = emulator {
        steps.push(RecoveryStep {
            id: "emulator-close".into(),
            title: "Close the Android emulator".into(),
            why: format!("{} holds {}.", p.name, fmt_bytes(p.memory_bytes)),
            action: ActionRef {
                action_id: "process.terminate".into(),
                params: json!({ "pid": p.pid, "name": p.name }),
                label: format!("Close {}", p.name),
            },
            risk: StepRisk::Medium,
            estimated_bytes: Some(p.memory_bytes),
            estimate_basis: "memory of the emulator process".into(),
        });
    }

    // 4. WSL shutdown: the biggest single step, and it subsumes the container steps.
    let vm_bytes = i
        .workloads
        .iter()
        .find(|w| matches!(w.kind, WorkloadKind::Docker))
        .and_then(|w| {
            w.members
                .iter()
                .find(|m| m.label.starts_with("WSL2 VM"))
                .map(|m| m.memory_bytes)
        });
    if let Some(vm) = vm_bytes.filter(|v| *v >= CAUSE_MIN_BYTES) {
        steps.push(RecoveryStep {
            id: "wsl-shutdown".into(),
            title: "Shut down WSL (stops every container)".into(),
            why: format!(
                "The WSL2 VM holds {} on the host. This stops all containers and distros; WSL restarts on next use.",
                fmt_bytes(vm)
            ),
            action: ActionRef {
                action_id: "wsl.shutdown".into(),
                params: json!({}),
                label: "Shut down WSL".into(),
            },
            risk: StepRisk::Medium,
            estimated_bytes: Some(vm),
            estimate_basis: "host memory of the WSL2 VM process (includes the containers above)"
                .into(),
        });
    }

    // Least disruptive first; within a risk level, the larger estimate first.
    steps.sort_by(|a, b| {
        a.risk
            .cmp_rank()
            .cmp(&b.risk.cmp_rank())
            .then(b.estimated_bytes.cmp(&a.estimated_bytes))
    });

    // Non-overlapping total: WSL shutdown includes the container steps, so take the larger of the two.
    let wsl = vm_bytes.filter(|v| *v >= CAUSE_MIN_BYTES).unwrap_or(0);
    let docker_part = wsl.max(container_sum);
    let emulator_part = emulator.map_or(0, |p| p.memory_bytes);
    let estimated = gradle_bytes + docker_part + emulator_part;
    let estimated_recovery_bytes = (!steps.is_empty() && estimated > 0).then_some(estimated);

    let recovery_note = if steps.is_empty() {
        "No safe recovery step was found automatically. Close applications you are not using, starting with the largest contributors above.".to_string()
    } else {
        "Estimates are upper bounds from measured memory. Steps overlap (WSL shutdown includes the container steps), so the total is not their sum, and Windows may not return all of it at once.".to_string()
    };

    Some(EmergencyReport {
        ram_used_bytes: i.snapshot.memory.used_bytes,
        ram_total_bytes: total,
        ram_percent: i.snapshot.memory.percent,
        commit_percent: t.and_then(|t| t.commit_percent),
        disk_active_percent: t.and_then(|t| t.disk_active_percent),
        hard_faults_per_sec: t.and_then(|t| t.hard_faults_per_sec),
        causes,
        cause_summary,
        mechanism: t
            .map(|t| &t.mechanism)
            .filter(|m| m.kind != crate::monitoring::telemetry::MechanismKind::None)
            .map(|m| m.summary.clone()),
        steps,
        estimated_recovery_bytes,
        recovery_note,
        incident_active: i.incident_active,
        active_sessions: i.active_sessions.clone(),
    })
}

impl StepRisk {
    fn cmp_rank(self) -> u8 {
        match self {
            StepRisk::Low => 0,
            StepRisk::Medium => 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::developers::workloads::detect_workloads;
    use crate::docker::Container;
    use crate::monitoring::metrics::memory_metrics;
    use crate::monitoring::pressure::{PressureEngine, PressureInputs};
    use crate::monitoring::telemetry::{TelemetryEngine, TelemetryInputs};
    use crate::processes::model::{Evidence, sample};
    use crate::windows::perf::PerfSample;

    fn snapshot(percent: f32, with_telemetry: bool) -> SystemSnapshot {
        let total = 16 * GIB;
        let used = (total as f64 * percent as f64 / 100.0) as u64;
        let memory = memory_metrics(total, total - used);
        let pressure = PressureEngine::default().update(
            PressureInputs {
                percent: memory.percent,
                available_bytes: memory.available_bytes,
                total_bytes: total,
                commit_percent: Some(97.0),
                pagefile_percent: None,
            },
            0.0,
        );
        let telemetry = with_telemetry.then(|| {
            TelemetryEngine::default().update(TelemetryInputs {
                total_bytes: total,
                available_bytes: memory.available_bytes,
                commit: None,
                perf: PerfSample {
                    hard_faults_per_sec: Some(900.0),
                    pagefile_usage_percent: None,
                    disk_active_percent: Some(100.0),
                },
                pressure: &pressure,
            })
        });
        SystemSnapshot {
            timestamp_ms: 1,
            memory,
            cpu_percent: 40.0,
            disks: vec![],
            disk_health: vec![],
            pressure: Some(pressure),
            telemetry,
            uptime_secs: 1,
            process_count: 1,
        }
    }

    fn gradle_daemon(pid: u32, mem: u64) -> ProcessInfo {
        ProcessInfo {
            command_line: Some("java org.gradle.launcher.daemon.bootstrap.GradleDaemon 8.5".into()),
            tags: vec!["gradle", "android"],
            evidence: Evidence::CommandLine,
            ..sample(pid, "java.exe", mem, 2.0, Category::Android)
        }
    }

    fn container(name: &str, mb: u64, cpu: f32) -> Container {
        Container {
            id: format!("id-{name}"),
            name: name.into(),
            image: "img".into(),
            status: "Up".into(),
            running: true,
            memory_bytes: Some(mb * MIB),
            cpu_percent: Some(cpu),
            memory_limit_bytes: None,
        }
    }

    fn docker(containers: Vec<Container>) -> DockerStatus {
        DockerStatus {
            available: true,
            error: None,
            total_memory_bytes: 0,
            containers,
        }
    }

    fn input<'a>(
        snap: &'a SystemSnapshot,
        ws: &'a [Workload],
        ps: &'a [ProcessInfo],
        d: Option<&'a DockerStatus>,
    ) -> EmergencyInput<'a> {
        EmergencyInput {
            snapshot: snap,
            workloads: ws,
            processes: ps,
            docker: d,
            incident_active: true,
            active_sessions: vec!["Android build".into()],
        }
    }

    fn machine() -> Vec<ProcessInfo> {
        vec![
            gradle_daemon(10, 4 * GIB + 200 * MIB),
            sample(20, "vmmemWSL", 4 * GIB + 800 * MIB, 3.0, Category::Docker),
            sample(21, "Docker Desktop.exe", 600 * MIB, 1.0, Category::Docker),
            sample(
                30,
                "qemu-system-x86_64.exe",
                2 * GIB,
                20.0,
                Category::Android,
            ),
            sample(40, "chrome.exe", 500 * MIB, 1.0, Category::Browser),
        ]
    }

    #[test]
    fn only_critical_pressure_is_an_emergency() {
        let ps = machine();
        let ws = detect_workloads(&ps);
        for pct in [50.0, 80.0, 90.0] {
            let snap = snapshot(pct, true);
            assert!(assess(&input(&snap, &ws, &ps, None)).is_none(), "{pct}%");
        }
        assert!(assess(&input(&snapshot(97.0, true), &ws, &ps, None)).is_some());
    }

    #[test]
    fn reports_pressure_cause_and_mechanism_with_evidence_not_certainty() {
        let ps = machine();
        let ws = detect_workloads(&ps);
        let snap = snapshot(97.0, true);
        let r = assess(&input(&snap, &ws, &ps, None)).unwrap();
        assert!((r.ram_percent - 97.0).abs() < 0.5);
        assert_eq!(r.commit_percent, None, "no commit info in this fixture");
        assert_eq!(r.disk_active_percent, Some(100.0));
        assert!(
            r.cause_summary.contains("Docker / WSL2"),
            "{}",
            r.cause_summary
        );
        assert!(r.cause_summary.contains("Android"), "{}", r.cause_summary);
        assert!(r.mechanism.as_deref().unwrap().contains("paging"));
        assert!(r.incident_active);
        assert_eq!(r.active_sessions, vec!["Android build"]);
        assert!(r.causes.len() <= MAX_CAUSES);
    }

    #[test]
    fn builds_steps_from_measured_facts_only() {
        let ps = machine();
        let ws = detect_workloads(&ps);
        let d = docker(vec![
            container("api", 1400, 0.2),
            container("busy", 900, 55.0),
        ]);
        let snap = snapshot(97.0, true);
        let r = assess(&input(&snap, &ws, &ps, Some(&d))).unwrap();
        let ids: Vec<&str> = r.steps.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"gradle-stop"));
        assert!(ids.contains(&"container-api"));
        assert!(
            !ids.contains(&"container-busy"),
            "a busy container is not idle"
        );
        assert!(ids.contains(&"emulator-close"));
        assert!(ids.contains(&"wsl-shutdown"));
        // Least disruptive first: LOW before MEDIUM.
        let first_medium = r
            .steps
            .iter()
            .position(|s| s.risk == StepRisk::Medium)
            .unwrap();
        assert!(
            r.steps[..first_medium]
                .iter()
                .all(|s| s.risk == StepRisk::Low)
        );
    }

    #[test]
    fn every_step_is_a_reference_to_a_registered_action() {
        let ps = machine();
        let ws = detect_workloads(&ps);
        let d = docker(vec![container("api", 1400, 0.2)]);
        let r = assess(&input(&snapshot(97.0, true), &ws, &ps, Some(&d))).unwrap();
        let known: Vec<&str> = crate::actions::builtin::all()
            .iter()
            .map(|a| a.spec().id)
            .collect();
        for s in &r.steps {
            assert!(
                known.contains(&s.action.action_id.as_str()),
                "{}",
                s.action.action_id
            );
        }
    }

    #[test]
    fn estimate_does_not_double_count_wsl_and_containers() {
        let ps = machine();
        let ws = detect_workloads(&ps);
        let d = docker(vec![
            container("api", 1400, 0.2),
            container("redis", 380, 0.1),
        ]);
        let r = assess(&input(&snapshot(97.0, true), &ws, &ps, Some(&d))).unwrap();
        let vm = 4 * GIB + 800 * MIB;
        let gradle = 4 * GIB + 200 * MIB;
        let emulator = 2 * GIB;
        // containers (1780 MB) < VM (4.8 GB): the VM figure stands in for both.
        assert_eq!(r.estimated_recovery_bytes, Some(gradle + vm + emulator));
        let naive: u64 = r.steps.iter().filter_map(|s| s.estimated_bytes).sum();
        assert!(
            naive > r.estimated_recovery_bytes.unwrap(),
            "the naive sum would double count"
        );
        assert!(r.recovery_note.contains("not their sum"));
    }

    #[test]
    fn without_docker_or_known_workloads_it_says_no_safe_step_exists() {
        let ps = vec![sample(40, "chrome.exe", 9 * GIB, 1.0, Category::Browser)];
        let ws = detect_workloads(&ps);
        let r = assess(&input(&snapshot(97.0, false), &ws, &ps, None)).unwrap();
        assert!(r.steps.is_empty());
        assert_eq!(r.estimated_recovery_bytes, None);
        assert!(r.recovery_note.contains("No safe recovery step"));
        assert_eq!(r.mechanism, None, "no telemetry, no mechanism claim");
    }

    #[test]
    fn unavailable_docker_gives_no_container_steps() {
        let ps = vec![gradle_daemon(10, 2 * GIB)];
        let ws = detect_workloads(&ps);
        let d = DockerStatus {
            available: false,
            error: Some("Docker CLI not found".into()),
            containers: vec![],
            total_memory_bytes: 0,
        };
        let r = assess(&input(&snapshot(97.0, true), &ws, &ps, Some(&d))).unwrap();
        assert!(r.steps.iter().all(|s| !s.id.starts_with("container-")));
        assert_eq!(r.steps.len(), 1);
    }

    #[test]
    fn container_names_that_could_be_flags_never_become_steps() {
        let ps = machine();
        let ws = detect_workloads(&ps);
        let d = docker(vec![
            container("--rm", 2000, 0.0),
            container("ok_name", 2000, 0.0),
        ]);
        let r = assess(&input(&snapshot(97.0, true), &ws, &ps, Some(&d))).unwrap();
        assert!(r.steps.iter().all(|s| s.id != "container---rm"));
        assert!(r.steps.iter().any(|s| s.id == "container-ok_name"));
    }

    #[test]
    fn system_processes_named_like_an_emulator_are_not_offered() {
        let mut p = sample(30, "emulator.exe", 2 * GIB, 1.0, Category::System);
        p.pid = 31;
        let ps = vec![p];
        let ws = detect_workloads(&ps);
        let r = assess(&input(&snapshot(97.0, false), &ws, &ps, None)).unwrap();
        assert!(r.steps.is_empty());
    }

    #[test]
    fn report_serializes_camel_case() {
        let ps = machine();
        let ws = detect_workloads(&ps);
        let r = assess(&input(&snapshot(97.0, true), &ws, &ps, None)).unwrap();
        let j = serde_json::to_string(&r).unwrap();
        assert!(
            j.contains("estimatedRecoveryBytes") && j.contains("\"actionId\":\"gradle.stop\""),
            "{j}"
        );
    }
}

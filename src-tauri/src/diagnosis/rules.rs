//! Deterministic diagnosis rules. Each rule looks at one `Input` and returns a finding or nothing.
//! No AI, no I/O: everything the rules know is passed in, so every rule is unit-testable.

use serde_json::json;

use super::model::{ActionRef, Contributor, Diagnosis, Recommendation, Risk, Severity};
use crate::developers::workloads::{Workload, WorkloadKind};
use crate::docker::DockerStatus;
use crate::monitoring::disks::{DiskReport, DiskState};
use crate::monitoring::growth::GrowthFinding;
use crate::monitoring::metrics::{DiskMetrics, MemoryMetrics};
use crate::monitoring::pressure::{PressureReport, PressureState, Trend};
use crate::storage::safety::Safety;
use crate::storage::scanner::ScanEntry;
use crate::util::fmt_bytes;

const GIB: u64 = 1024 * 1024 * 1024;

/// Everything a rule may look at.
pub struct Input<'a> {
    pub memory: &'a MemoryMetrics,
    pub pressure: &'a PressureReport,
    pub workloads: &'a [Workload],
    pub growth: &'a [GrowthFinding],
    pub disks: &'a [DiskMetrics],
    pub disk_health: &'a [DiskReport],
    pub docker: Option<&'a DockerStatus>,
    pub storage: Option<&'a [ScanEntry]>,
}

/// Rule thresholds, defaults from the product spec.
#[derive(Debug, Clone)]
pub struct Config {
    /// Top contributors holding at least this share of *used* RAM count as "concentrated".
    pub concentrated_share: f32,
    pub docker_min_bytes: u64,
    pub docker_ram_percent: f32,
    pub java_min_bytes: u64,
    pub node_min_bytes: u64,
    pub low_free_bytes: u64,
    pub cache_candidate_min_bytes: u64,
    /// Containers below this CPU share are offered as "idle".
    pub idle_container_cpu: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            concentrated_share: 0.50,
            docker_min_bytes: 4 * GIB,
            docker_ram_percent: 85.0,
            java_min_bytes: 3 * GIB,
            node_min_bytes: 2 * GIB,
            low_free_bytes: 10 * GIB,
            cache_candidate_min_bytes: 2 * GIB,
            idle_container_cpu: 1.0,
        }
    }
}

pub type Rule = fn(&Input, &Config) -> Option<Diagnosis>;

/// Rule order is also the tie-break order for equal severity.
pub const RULES: &[Rule] = &[
    memory_pressure,
    docker_wsl,
    android_gradle,
    node_dev,
    memory_growth,
    low_disk,
    cleanable_caches,
];

fn join_names(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        _ => format!(
            "{} and {}",
            names[..names.len() - 1].join(", "),
            names[names.len() - 1]
        ),
    }
}

fn pct_of(bytes: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (bytes as f64 / total as f64 * 100.0) as f32
    }
}

fn contributor(w: &Workload, total_ram: u64) -> Contributor {
    Contributor {
        name: w.name.clone(),
        memory_bytes: w.memory_bytes,
        percent: pct_of(w.memory_bytes, total_ram),
        process_count: w.process_count,
        detail: w.top_processes.first().map(|p| p.name.clone()),
    }
}

fn find_kind(ws: &[Workload], kind: WorkloadKind) -> Option<&Workload> {
    ws.iter().find(|w| w.kind == kind)
}

/// Name the cause the way a developer would, using workload signals.
fn cause_label(w: &Workload) -> String {
    match w.kind {
        WorkloadKind::Android if w.has_signal("gradle") => "Android Gradle build".into(),
        WorkloadKind::Docker => "Docker/WSL2".into(),
        WorkloadKind::Node if w.has_signal("nextjs") => "Next.js dev server".into(),
        WorkloadKind::Java if w.has_signal("gradle") => "Gradle daemons".into(),
        _ => w.name.clone(),
    }
}

fn severity_for_memory(state: PressureState) -> Option<Severity> {
    match state {
        PressureState::Normal => None,
        PressureState::Moderate => Some(Severity::Warning),
        PressureState::High => Some(Severity::High),
        PressureState::Critical => Some(Severity::Critical),
    }
}

fn manual(
    id: &str,
    problem: &str,
    explanation: &str,
    benefit: &str,
    bytes: Option<u64>,
    risk: Risk,
    steps: &str,
) -> Recommendation {
    Recommendation {
        id: id.into(),
        problem: problem.into(),
        explanation: explanation.into(),
        expected_benefit: benefit.into(),
        expected_benefit_bytes: bytes,
        risk,
        action: None,
        manual_steps: Some(steps.into()),
    }
}

// ---------------------------------------------------------------------------------------------

/// RAM is high: say how high, who holds it, and whether it is concentrated.
pub fn memory_pressure(i: &Input, c: &Config) -> Option<Diagnosis> {
    let severity = severity_for_memory(i.pressure.state)?;
    let total = i.memory.total_bytes;
    let used = i.memory.used_bytes.max(1);

    let named: Vec<&Workload> = i
        .workloads
        .iter()
        .filter(|w| !matches!(w.kind, WorkloadKind::Database) || w.memory_bytes > GIB)
        .filter(|w| pct_of(w.memory_bytes, total) >= 3.0)
        .take(3)
        .collect();
    let contributors: Vec<Contributor> = named.iter().map(|w| contributor(w, total)).collect();
    let top_bytes: u64 = named.iter().map(|w| w.memory_bytes).sum();
    let share_of_used = top_bytes as f32 / used as f32;
    let concentrated = !named.is_empty() && share_of_used >= c.concentrated_share;

    let names: Vec<String> = named.iter().map(|w| cause_label(w)).collect();
    let ram = format!("RAM is at {:.0}%", i.memory.percent);
    let summary = if named.is_empty() {
        format!(
            "{ram}. No single developer workload dominates; usage is spread across many processes."
        )
    } else if concentrated {
        format!(
            "{ram}. {} account for approximately {}, making them the primary contributors.",
            join_names(&names),
            fmt_bytes(top_bytes)
        )
    } else {
        format!(
            "{ram}. The largest contributors are {} (approximately {} together), but memory use is spread across many processes.",
            join_names(&names),
            fmt_bytes(top_bytes)
        )
    };

    let confidence = if named.is_empty() {
        0.4
    } else {
        let coverage = (top_bytes as f32 / used as f32).min(1.0);
        (0.45 + 0.45 * coverage + if concentrated { 0.1 } else { 0.0 }).min(0.95)
    };

    let mut recommendations = Vec::new();
    if let Some(top) = named.first() {
        let (problem, steps) = match top.kind {
            WorkloadKind::Browser => (
                "Browsers are the largest memory consumer".to_string(),
                "Close unused tabs and windows, or use the browser's memory saver mode.",
            ),
            WorkloadKind::Ide => (
                "Editors are the largest memory consumer".to_string(),
                "Close unused editor windows and restart the editor to release extension memory.",
            ),
            _ => (
                format!("{} is the largest memory consumer", cause_label(top)),
                "Stop or restart the workload you are not actively using.",
            ),
        };
        recommendations.push(manual(
            "memory-top-contributor",
            &problem,
            &format!(
                "{} uses {} ({:.0}% of installed RAM).",
                top.name,
                fmt_bytes(top.memory_bytes),
                pct_of(top.memory_bytes, total)
            ),
            &format!("Up to {} of RAM", fmt_bytes(top.memory_bytes)),
            Some(top.memory_bytes),
            Risk::Low,
            steps,
        ));
    }

    Some(Diagnosis {
        rule_id: "memory-pressure".into(),
        severity,
        title: "Memory pressure detected".into(),
        summary,
        primary_cause: named.first().map(|w| cause_label(w)),
        contributors,
        confidence,
        evidence: i.pressure.reasons.clone(),
        recommendations,
    })
}

/// Docker/WSL2 is large while RAM is tight.
pub fn docker_wsl(i: &Input, c: &Config) -> Option<Diagnosis> {
    let d = find_kind(i.workloads, WorkloadKind::Docker)?;
    if d.memory_bytes <= c.docker_min_bytes || i.memory.percent <= c.docker_ram_percent {
        return None;
    }
    let total = i.memory.total_bytes;
    let mut recs = Vec::new();

    let running: Vec<&crate::docker::Container> = i
        .docker
        .filter(|s| s.available)
        .map(|s| s.containers.iter().filter(|c| c.running).collect())
        .unwrap_or_default();

    // Idle containers: offered one by one, never in bulk.
    let mut idle: Vec<&&crate::docker::Container> = running
        .iter()
        .filter(|ct| ct.cpu_percent.is_some_and(|p| p < c.idle_container_cpu))
        .collect();
    idle.sort_by_key(|ct| std::cmp::Reverse(ct.memory_bytes.unwrap_or(0)));
    for ct in idle.into_iter().take(3) {
        let mem = ct.memory_bytes;
        recs.push(Recommendation {
            id: format!("docker-stop-{}", ct.id),
            problem: format!("Container {} looks idle", ct.name),
            explanation: format!(
                "It uses {} but its CPU is under {:.0}%.",
                mem.map(fmt_bytes).unwrap_or_else(|| "memory".into()),
                c.idle_container_cpu
            ),
            expected_benefit: mem
                .map(|m| format!("About {} of RAM", fmt_bytes(m)))
                .unwrap_or_else(|| "Frees its memory".into()),
            expected_benefit_bytes: mem,
            risk: Risk::Low,
            action: Some(ActionRef {
                action_id: "docker.stop_container".into(),
                params: json!({ "container": ct.name }),
                label: format!("Stop container {}", ct.name),
            }),
            manual_steps: None,
        });
    }

    // The WSL VM keeps memory after containers stop.
    if d.has_signal("wsl-vm") && running.is_empty() {
        let vm = d
            .members
            .iter()
            .find(|m| m.label.starts_with("WSL2 VM"))
            .map(|m| m.memory_bytes)
            .unwrap_or(d.memory_bytes);
        recs.push(Recommendation {
            id: "wsl-shutdown".into(),
            problem: "The WSL2 VM is holding memory while no containers are running".into(),
            explanation: "WSL2 keeps its memory after workloads stop. Shutting it down returns it to Windows; it restarts on next use.".into(),
            expected_benefit: format!("About {} of RAM", fmt_bytes(vm)),
            expected_benefit_bytes: Some(vm),
            risk: Risk::Medium,
            action: Some(ActionRef {
                action_id: "wsl.shutdown".into(),
                params: json!({}),
                label: "Shut down WSL".into(),
            }),
            manual_steps: None,
        });
    }

    recs.push(manual(
        "wsl-memory-limit",
        "WSL2 has no memory limit",
        "By default WSL2 can take up to half of your RAM and release it slowly.",
        "Caps Docker/WSL2 memory permanently",
        None,
        Risk::Low,
        "Create or edit %UserProfile%\\.wslconfig with:\n[wsl2]\nmemory=6GB\nthen run `wsl --shutdown` once.",
    ));

    let mut evidence = vec![format!(
        "Docker/WSL2 uses {} ({:.0}% of RAM) while RAM is at {:.0}%",
        fmt_bytes(d.memory_bytes),
        pct_of(d.memory_bytes, total),
        i.memory.percent
    )];
    if let Some(s) = i.docker {
        if s.available {
            evidence.push(format!("{} containers running", running.len()));
        } else if let Some(e) = &s.error {
            evidence.push(format!("Docker details unavailable: {e}"));
        }
    }
    for m in d.members.iter().take(3) {
        evidence.push(format!("{}: {}", m.label, fmt_bytes(m.memory_bytes)));
    }

    Some(Diagnosis {
        rule_id: "docker-wsl".into(),
        severity: if i.pressure.state == PressureState::Critical {
            Severity::Critical
        } else {
            Severity::High
        },
        title: "Docker/WSL2 is a significant memory contributor".into(),
        summary: format!(
            "Docker/WSL2 uses {} ({:.0}% of installed RAM) while RAM is at {:.0}%.",
            fmt_bytes(d.memory_bytes),
            pct_of(d.memory_bytes, total),
            i.memory.percent
        ),
        primary_cause: Some("Docker/WSL2".into()),
        contributors: vec![contributor(d, total)],
        confidence: if d.has_signal("wsl-vm") { 0.9 } else { 0.75 },
        evidence,
        recommendations: recs,
    })
}

/// Gradle (Android or plain Java) is large while the machine is under pressure.
pub fn android_gradle(i: &Input, c: &Config) -> Option<Diagnosis> {
    if i.pressure.state < PressureState::Moderate {
        return None;
    }
    let (w, android) = match (
        find_kind(i.workloads, WorkloadKind::Android),
        find_kind(i.workloads, WorkloadKind::Java),
    ) {
        (Some(a), _) if a.has_signal("gradle") => (a, true),
        (_, Some(j)) if j.has_signal("gradle") => (j, false),
        _ => return None,
    };
    if w.memory_bytes <= c.java_min_bytes {
        return None;
    }
    let total = i.memory.total_bytes;
    let gradle = w
        .members
        .iter()
        .find(|m| m.label == "Gradle daemon")
        .map(|m| m.memory_bytes)
        .unwrap_or(w.memory_bytes);
    let what = if android {
        "Android Gradle build"
    } else {
        "Gradle daemons"
    };

    let recs = vec![
        Recommendation {
            id: "gradle-stop".into(),
            problem: "Gradle daemons stay resident after a build".into(),
            explanation: "Daemons keep their JVM heap for faster rebuilds. Stopping them frees it; the next build starts a new one.".into(),
            expected_benefit: format!("About {} of RAM", fmt_bytes(gradle)),
            expected_benefit_bytes: Some(gradle),
            risk: Risk::Low,
            action: Some(ActionRef { action_id: "gradle.stop".into(), params: json!({}), label: "Stop Gradle daemons".into() }),
            manual_steps: None,
        },
        manual(
            "gradle-heap-limit",
            "Gradle heap is not capped",
            "Large heaps are the usual reason Gradle grows past 3 GB.",
            "Caps Gradle memory permanently",
            None,
            Risk::Low,
            "In gradle.properties set:\norg.gradle.jvmargs=-Xmx2g -XX:MaxMetaspaceSize=512m\norg.gradle.daemon.idletimeout=600000",
        ),
    ];

    Some(Diagnosis {
        rule_id: "android-gradle".into(),
        severity: if i.pressure.state >= PressureState::High {
            Severity::High
        } else {
            Severity::Warning
        },
        title: format!("{what} is a significant memory contributor"),
        summary: format!(
            "{what} uses {} ({:.0}% of installed RAM) across {} processes while RAM is at {:.0}%.",
            fmt_bytes(w.memory_bytes),
            pct_of(w.memory_bytes, total),
            w.process_count,
            i.memory.percent
        ),
        primary_cause: Some(what.into()),
        contributors: vec![contributor(w, total)],
        confidence: if android { 0.9 } else { 0.8 },
        evidence: w
            .members
            .iter()
            .take(3)
            .map(|m| format!("{}: {}", m.label, fmt_bytes(m.memory_bytes)))
            .collect(),
        recommendations: recs,
    })
}

/// Node/Next.js/Vite dev servers are large while RAM is under pressure.
pub fn node_dev(i: &Input, c: &Config) -> Option<Diagnosis> {
    if i.pressure.state < PressureState::Moderate {
        return None;
    }
    let w = find_kind(i.workloads, WorkloadKind::Node)?;
    if w.memory_bytes <= c.node_min_bytes {
        return None;
    }
    let total = i.memory.total_bytes;
    let next = w.has_signal("nextjs");
    let who = if next {
        "Next.js dev servers"
    } else {
        "Node.js processes"
    };

    Some(Diagnosis {
        rule_id: "node-dev".into(),
        severity: Severity::Warning,
        title: format!("{who} are using a lot of memory"),
        summary: format!(
            "{who} use {} ({:.0}% of installed RAM) across {} processes while RAM is at {:.0}%.",
            fmt_bytes(w.memory_bytes),
            pct_of(w.memory_bytes, total),
            w.process_count,
            i.memory.percent
        ),
        primary_cause: Some(if next {
            "Next.js dev server".into()
        } else {
            "Node.js".into()
        }),
        contributors: vec![contributor(w, total)],
        confidence: 0.7,
        evidence: w
            .members
            .iter()
            .take(3)
            .map(|m| {
                format!(
                    "{}: {} ({} processes)",
                    m.label,
                    fmt_bytes(m.memory_bytes),
                    m.process_count
                )
            })
            .collect(),
        recommendations: vec![manual(
            "node-restart",
            "Dev servers accumulate memory while running",
            "Hot reload and build caches grow over a long session; a restart resets them.",
            &format!("Up to {} of RAM", fmt_bytes(w.memory_bytes)),
            Some(w.memory_bytes),
            Risk::Low,
            "Stop and restart the dev servers you are not actively using (Ctrl+C in their terminals).",
        )],
    })
}

/// Sustained growth in a process. Always worded as "possible", with the detector's confidence.
pub fn memory_growth(i: &Input, _c: &Config) -> Option<Diagnosis> {
    let top: Vec<&GrowthFinding> = i.growth.iter().take(3).collect();
    let first = top.first()?;
    let total = i.memory.total_bytes;

    let recs = top
        .iter()
        .map(|g| Recommendation {
            id: format!("growth-restart-{}", g.pid),
            problem: format!("{} keeps growing", g.name),
            explanation: format!("{} Confidence {:.0}%.", g.summary, g.confidence * 100.0),
            expected_benefit: format!("Up to {} of RAM", fmt_bytes(g.current_bytes)),
            expected_benefit_bytes: Some(g.current_bytes),
            risk: Risk::Medium,
            action: Some(ActionRef {
                action_id: "process.terminate".into(),
                params: json!({ "pid": g.pid, "name": g.name }),
                label: format!("Close {} (PID {})", g.name, g.pid),
            }),
            manual_steps: Some("Save your work first. If it keeps growing after a restart, investigate that program for a leak.".into()),
        })
        .collect();

    Some(Diagnosis {
        rule_id: "memory-growth".into(),
        severity: if i.pressure.state >= PressureState::High {
            Severity::High
        } else {
            Severity::Warning
        },
        title: "Possible memory growth/leak".into(),
        summary: format!(
            "{} Confidence {:.0}%. This may be normal caching; it is only a leak if it never levels off.",
            first.summary,
            first.confidence * 100.0
        ),
        primary_cause: Some(first.name.clone()),
        contributors: top
            .iter()
            .map(|g| Contributor {
                name: g.name.clone(),
                memory_bytes: g.current_bytes,
                percent: pct_of(g.current_bytes, total),
                process_count: 1,
                detail: Some(format!("PID {}", g.pid)),
            })
            .collect(),
        confidence: first.confidence,
        evidence: top
            .iter()
            .map(|g| {
                format!(
                    "{}: +{} in {:.0} min",
                    g.name,
                    fmt_bytes(g.growth_bytes),
                    g.window_minutes
                )
            })
            .collect(),
        recommendations: recs,
    })
}

fn state_severity(s: DiskState) -> Option<Severity> {
    match s {
        DiskState::Normal => None,
        DiskState::Warning => Some(Severity::Warning),
        DiskState::High => Some(Severity::High),
        DiskState::Critical => Some(Severity::Critical),
    }
}

fn candidate_recs(i: &Input, mount: Option<&str>) -> Vec<Recommendation> {
    let Some(entries) = i.storage else {
        return vec![manual(
            "storage-scan",
            "No storage scan has been run yet",
            "DevPilot needs a scan to find caches and build output it can offer to clean.",
            "Finds cleanup candidates",
            None,
            Risk::Low,
            "Open Storage and press Scan.",
        )];
    };
    let mut recs = Vec::new();
    let mut cands: Vec<&ScanEntry> = entries
        .iter()
        .filter(|e| e.safety != Safety::DoNotTouch && e.size_bytes >= 100 * 1024 * 1024)
        .filter(|e| mount.is_none_or(|m| e.path.to_lowercase().starts_with(&m.to_lowercase())))
        .collect();
    cands.sort_by_key(|e| std::cmp::Reverse((e.safety == Safety::SafeToClean, e.size_bytes)));
    for e in cands.into_iter().take(4) {
        let (risk, action) = match e.safety {
            Safety::SafeToClean => (Risk::Low, true),
            Safety::SafeToReview => (Risk::Medium, true),
            _ => (Risk::High, false),
        };
        recs.push(Recommendation {
            id: format!("cleanup-{}", e.path),
            problem: format!("{} ({})", e.name, e.kind_label),
            explanation: format!("{} {}", fmt_bytes(e.size_bytes), e.reason),
            expected_benefit: format!("About {} of disk space", fmt_bytes(e.size_bytes)),
            expected_benefit_bytes: Some(e.size_bytes),
            risk,
            action: action.then(|| ActionRef {
                action_id: "cleanup.path".into(),
                params: json!({ "path": e.path }),
                label: format!("Delete {}", e.name),
            }),
            manual_steps: (!action).then(|| "Review this in Storage and decide manually.".into()),
        });
    }
    recs
}

/// A drive is low on free space, now or by trend.
pub fn low_disk(i: &Input, c: &Config) -> Option<Diagnosis> {
    let (disk, health) = i
        .disks
        .iter()
        .zip(i.disk_health.iter())
        .filter(|(d, h)| h.state != DiskState::Normal || d.free_bytes < c.low_free_bytes)
        .max_by_key(|(d, h)| (h.state, std::cmp::Reverse(d.free_bytes)))?;

    let mut severity = state_severity(health.state).unwrap_or(Severity::Warning);
    if disk.free_bytes < c.low_free_bytes {
        severity = severity.max(Severity::High);
    }

    let mut summary = format!(
        "{} has {} free ({:.0}% of {}).",
        disk.mount_point,
        fmt_bytes(disk.free_bytes),
        health.free_percent,
        fmt_bytes(disk.total_bytes)
    );
    if let Some(h) = health.hours_to_full {
        summary.push_str(&format!(
            " At the current rate it will be full in about {}.",
            fmt_duration_hours(h)
        ));
    } else if let Some(h) = health.hours_to_critical {
        summary.push_str(&format!(
            " At the current rate it reaches the critical level in about {}.",
            fmt_duration_hours(h)
        ));
    }

    let mut recs = candidate_recs(i, Some(&disk.mount_point));
    let est: u64 = recs
        .iter()
        .filter(|r| r.action.is_some())
        .filter_map(|r| r.expected_benefit_bytes)
        .sum();
    if est > 0 {
        summary.push_str(&format!(
            " Cleanup candidates could recover about {}.",
            fmt_bytes(est)
        ));
    }
    if recs.is_empty() {
        recs.push(manual(
            "disk-manual",
            "No safe cleanup candidates found on this drive",
            "The last scan found nothing DevPilot considers safe to offer.",
            "Frees space you choose",
            None,
            Risk::Low,
            "Review the largest folders and files in Storage.",
        ));
    }

    Some(Diagnosis {
        rule_id: "low-disk".into(),
        severity,
        title: format!("Low disk space on {}", disk.mount_point),
        summary,
        primary_cause: Some("Storage pressure".into()),
        contributors: vec![],
        confidence: if health.trend_confidence.is_some() {
            0.9
        } else {
            0.85
        },
        evidence: health.reasons.clone(),
        recommendations: recs,
    })
}

fn fmt_duration_hours(h: f64) -> String {
    if h < 1.0 {
        format!("{:.0} minutes", (h * 60.0).max(1.0))
    } else if h < 48.0 {
        format!("{h:.1} hours")
    } else {
        format!("{:.0} days", h / 24.0)
    }
}

/// Disks are fine but known caches add up to something worth reclaiming.
pub fn cleanable_caches(i: &Input, c: &Config) -> Option<Diagnosis> {
    let entries = i.storage?;
    let clean: Vec<&ScanEntry> = entries
        .iter()
        .filter(|e| e.safety == Safety::SafeToClean)
        .collect();
    let total: u64 = clean.iter().map(|e| e.size_bytes).sum();
    if total < c.cache_candidate_min_bytes {
        return None;
    }
    let recs: Vec<Recommendation> = candidate_recs(i, None)
        .into_iter()
        .filter(|r| r.risk == Risk::Low && r.action.is_some())
        .collect();
    Some(Diagnosis {
        rule_id: "cleanable-caches".into(),
        severity: Severity::Info,
        title: "Disposable caches can be cleaned".into(),
        summary: format!(
            "{} of temporary files and package caches were found. They are recreated on demand.",
            fmt_bytes(total)
        ),
        primary_cause: None,
        contributors: vec![],
        confidence: 0.9,
        evidence: clean
            .iter()
            .take(3)
            .map(|e| format!("{}: {}", e.name, fmt_bytes(e.size_bytes)))
            .collect(),
        recommendations: recs,
    })
}

/// Shown when no rule fires.
pub fn healthy(i: &Input) -> Diagnosis {
    let trend = match i.pressure.trend {
        Trend::Escalating | Trend::Rising => " Memory usage is rising, so keep an eye on it.",
        _ => "",
    };
    Diagnosis {
        rule_id: "healthy".into(),
        severity: Severity::Info,
        title: "System looks healthy".into(),
        summary: format!(
            "RAM is at {:.0}% and no drive is low on space.{trend}",
            i.memory.percent
        ),
        primary_cause: None,
        contributors: vec![],
        confidence: 0.9,
        evidence: i.pressure.reasons.clone(),
        recommendations: vec![],
    }
}

//! The only data that may be sent to an AI provider: a short, summarized, sanitized snapshot.
//!
//! Built from an explicit allow-list of fields. Nothing is "filtered out of" a bigger object, so a
//! new field added elsewhere can never leak by accident. Never included: file contents, file paths,
//! command lines, environment variables, tokens, PIDs, user names.

use serde::Serialize;

use super::sanitize::sanitize_text;
use crate::developers::workloads::Workload;
use crate::diagnosis::model::{Diagnosis, DiagnosisReport};
use crate::monitoring::metrics::SystemSnapshot;
use crate::processes::model::ProcessInfo;
use crate::util::fmt_bytes;

const TOP_PROCESSES: usize = 8;
const MAX_FINDINGS: usize = 4;
const MAX_RECOMMENDATIONS: usize = 4;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SystemSummary {
    pub ram: String,
    pub cpu: String,
    pub pressure: String,
    pub trend: String,
    pub disks: Vec<DiskSummary>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiskSummary {
    pub drive: String,
    pub free: String,
    pub total: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadSummary {
    pub name: String,
    pub memory: String,
    pub processes: usize,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProcessSummary {
    pub name: String,
    pub category: String,
    pub memory: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationSummary {
    pub problem: String,
    pub explanation: String,
    pub expected_benefit: String,
    pub risk: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FindingSummary {
    pub severity: String,
    pub title: String,
    pub summary: String,
    pub primary_cause: Option<String>,
    pub confidence: String,
    pub contributors: Vec<String>,
    pub recommendations: Vec<RecommendationSummary>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AiPayload {
    pub system: SystemSummary,
    pub workloads: Vec<WorkloadSummary>,
    pub top_processes: Vec<ProcessSummary>,
    pub diagnosis_status: String,
    pub findings: Vec<FindingSummary>,
}

fn enum_text<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn finding(d: &Diagnosis) -> FindingSummary {
    FindingSummary {
        severity: enum_text(&d.severity),
        title: sanitize_text(&d.title),
        summary: sanitize_text(&d.summary),
        primary_cause: d.primary_cause.as_deref().map(sanitize_text),
        confidence: format!("{:.0}%", d.confidence * 100.0),
        contributors: d
            .contributors
            .iter()
            .map(|c| {
                sanitize_text(&format!(
                    "{}: {} ({:.0}% of RAM)",
                    c.name,
                    fmt_bytes(c.memory_bytes),
                    c.percent
                ))
            })
            .collect(),
        recommendations: d
            .recommendations
            .iter()
            .take(MAX_RECOMMENDATIONS)
            .map(|r| RecommendationSummary {
                problem: sanitize_text(&r.problem),
                explanation: sanitize_text(&r.explanation),
                expected_benefit: sanitize_text(&r.expected_benefit),
                risk: enum_text(&r.risk),
            })
            .collect(),
    }
}

/// Build the payload. Every string passes through the sanitizer.
pub fn build_payload(
    snapshot: &SystemSnapshot,
    workloads: &[Workload],
    processes: &[ProcessInfo],
    report: &DiagnosisReport,
) -> AiPayload {
    let mem = &snapshot.memory;
    let pressure = snapshot.pressure.as_ref();

    let mut top: Vec<&ProcessInfo> = processes.iter().collect();
    top.sort_by_key(|p| std::cmp::Reverse(p.memory_bytes));

    AiPayload {
        system: SystemSummary {
            ram: format!(
                "{}/{} ({:.0}%)",
                fmt_bytes(mem.used_bytes),
                fmt_bytes(mem.total_bytes),
                mem.percent
            ),
            cpu: format!("{:.0}%", snapshot.cpu_percent),
            pressure: pressure.map(|p| enum_text(&p.state)).unwrap_or_default(),
            trend: pressure.map(|p| enum_text(&p.trend)).unwrap_or_default(),
            disks: snapshot
                .disks
                .iter()
                .zip(snapshot.disk_health.iter())
                .map(|(d, h)| DiskSummary {
                    // Drive letter only: mount points can contain folder names.
                    drive: d.mount_point.chars().take(2).collect(),
                    free: fmt_bytes(d.free_bytes),
                    total: fmt_bytes(d.total_bytes),
                    state: enum_text(&h.state),
                })
                .collect(),
        },
        workloads: workloads
            .iter()
            .take(8)
            .map(|w| WorkloadSummary {
                name: sanitize_text(&w.name),
                memory: fmt_bytes(w.memory_bytes),
                processes: w.process_count,
                members: w
                    .members
                    .iter()
                    .take(5)
                    .map(|m| sanitize_text(&format!("{}: {}", m.label, fmt_bytes(m.memory_bytes))))
                    .collect(),
            })
            .collect(),
        top_processes: top
            .into_iter()
            .take(TOP_PROCESSES)
            .map(|p| ProcessSummary {
                name: sanitize_text(&p.name),
                category: enum_text(&p.category),
                memory: fmt_bytes(p.memory_bytes),
            })
            .collect(),
        diagnosis_status: enum_text(&report.status),
        findings: report
            .findings
            .iter()
            .take(MAX_FINDINGS)
            .map(finding)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::developers::workloads::detect_workloads;
    use crate::diagnosis::engine::diagnose;
    use crate::diagnosis::rules::{Config, Input};
    use crate::monitoring::disks::{DiskThresholds, build_report};
    use crate::monitoring::metrics::{disk_metrics, memory_metrics};
    use crate::monitoring::pressure::{PressureEngine, PressureInputs};
    use crate::processes::model::{Category, sample};

    const GIB: u64 = 1024 * 1024 * 1024;

    fn world() -> (
        SystemSnapshot,
        Vec<Workload>,
        Vec<ProcessInfo>,
        DiagnosisReport,
    ) {
        let mut p1 = sample(4242, "java.exe", 4 * GIB, 5.0, Category::Java);
        p1.exe_path = Some("C:\\Users\\alice\\.jdks\\bin\\java.exe".into());
        p1.command_line = Some(
            "java -Dpassword=hunter2 --token sk-ant-api03-SECRETSECRET1234567890 -jar app.jar"
                .into(),
        );
        p1.tags = vec!["gradle", "android"];
        let p2 = sample(7, "vmmemWSL", 5 * GIB, 1.0, Category::Docker);
        let procs = vec![p1, p2];

        let memory = memory_metrics(16 * GIB, GIB / 2);
        let pressure = PressureEngine::default().update(
            PressureInputs {
                percent: memory.percent,
                available_bytes: memory.available_bytes,
                total_bytes: memory.total_bytes,
                commit_percent: None,
                pagefile_percent: None,
            },
            0.0,
        );
        let d = disk_metrics("Win".into(), "C:\\".into(), 275 * GIB, 6 * GIB);
        let h = build_report(&DiskThresholds::default(), &d, &[], 0.0);
        let snap = SystemSnapshot {
            timestamp_ms: 1,
            memory: memory.clone(),
            cpu_percent: 38.0,
            disks: vec![d.clone()],
            disk_health: vec![h.clone()],
            pressure: Some(pressure.clone()),
            telemetry: None,
            uptime_secs: 10,
            process_count: 2,
        };
        let workloads = detect_workloads(&procs);
        let report = diagnose(
            &Input {
                memory: &memory,
                pressure: &pressure,
                workloads: &workloads,
                growth: &[],
                disks: &[d],
                disk_health: &[h],
                docker: None,
                storage: None,
                telemetry: None,
            },
            &Config::default(),
            1,
        );
        (snap, workloads, procs, report)
    }

    #[test]
    fn payload_has_the_documented_shape() {
        let (s, w, p, r) = world();
        let v = serde_json::to_value(build_payload(&s, &w, &p, &r)).unwrap();
        assert!(
            v["system"]["ram"]
                .as_str()
                .unwrap()
                .contains("15.5 GB/16.0 GB"),
            "{}",
            v["system"]["ram"]
        );
        assert_eq!(v["system"]["pressure"], "CRITICAL");
        assert_eq!(v["system"]["disks"][0]["drive"], "C:");
        assert_eq!(v["system"]["disks"][0]["free"], "6.0 GB");
        assert!(v["workloads"].as_array().unwrap().len() >= 2);
        assert_eq!(v["topProcesses"][0]["name"], "vmmemWSL");
        assert_eq!(v["diagnosisStatus"], "problem");
        assert!(
            v["findings"][0]["summary"]
                .as_str()
                .unwrap()
                .contains("RAM is at")
        );
        assert!(v["findings"][0]["recommendations"][0]["risk"].is_string());
    }

    #[test]
    fn nothing_private_leaves_the_machine() {
        let (s, w, p, r) = world();
        let json = serde_json::to_string(&build_payload(&s, &w, &p, &r)).unwrap();
        for forbidden in [
            "alice",
            "hunter2",
            "sk-ant",
            "SECRETSECRET",
            "--token",
            "-Dpassword",
            "app.jar",
            ".jdks",
            "exePath",
            "commandLine",
            "4242", // PID
            "\"pid\"",
            "C:\\\\Users",
        ] {
            assert!(
                !json.contains(forbidden),
                "payload leaks {forbidden:?}: {json}"
            );
        }
    }

    #[test]
    fn payload_is_small_and_bounded() {
        let (s, w, mut p, r) = world();
        for i in 0..500 {
            p.push(sample(
                i,
                &format!("proc{i}.exe"),
                (i as u64 + 1) * 1024,
                0.0,
                Category::Other,
            ));
        }
        let pl = build_payload(&s, &w, &p, &r);
        assert_eq!(pl.top_processes.len(), TOP_PROCESSES);
        assert!(pl.findings.len() <= MAX_FINDINGS);
        assert!(serde_json::to_string(&pl).unwrap().len() < 12_000);
    }

    #[test]
    fn sanitizer_is_applied_to_free_text_fields() {
        let (s, w, mut p, mut r) = world();
        p[0].name = "C:\\Users\\alice\\evil.exe".into();
        r.headline.summary =
            "Check C:\\Users\\bob\\x with key sk-ant-api03-ZZZZZZZZZZZZZZZZZZZZ".into();
        r.findings[0] = r.headline.clone();
        let json = serde_json::to_string(&build_payload(&s, &w, &p, &r)).unwrap();
        assert!(
            !json.contains("bob") && !json.contains("alice") && !json.contains("ZZZZ"),
            "{json}"
        );
        assert!(json.contains("<user>"));
    }

    #[test]
    fn empty_world_does_not_panic() {
        let (mut s, _, _, r) = world();
        s.disks.clear();
        s.disk_health.clear();
        s.pressure = None;
        let pl = build_payload(&s, &[], &[], &r);
        assert!(
            pl.workloads.is_empty() && pl.top_processes.is_empty() && pl.system.disks.is_empty()
        );
        assert_eq!(pl.system.pressure, "");
    }
}

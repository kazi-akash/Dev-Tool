//! Workflow tests: several modules working together on real files, the way a user's session would.
//! scan -> diagnose -> recommend -> approve -> act -> history/AI payload.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use serde_json::json;

use crate::actions::engine::ActionEngine;
use crate::actions::model::{
    ActionError, CommandRunner, Confirmation, Ctx, Facts, ProcessKiller, RiskLevel,
};
use crate::ai::payload::build_payload;
use crate::developers::workloads::detect_workloads;
use crate::diagnosis::engine::diagnose;
use crate::diagnosis::model::{DiagnosisReport, Severity, Status};
use crate::diagnosis::rules::{Config, Input};
use crate::monitoring::disks::{DiskThresholds, build_report};
use crate::monitoring::metrics::{SystemSnapshot, disk_metrics, memory_metrics};
use crate::monitoring::pressure::{PressureEngine, PressureInputs};
use crate::processes::model::{Category, sample};
use crate::storage::scanner::{ScanEntry, ScanEnv, ScanOptions, ScanProgress, run_scan};

const GIB: u64 = 1024 * 1024 * 1024;

struct NoRunner;
impl CommandRunner for NoRunner {
    fn run(&self, program: &str, _args: &[&str]) -> Result<String, String> {
        panic!("no external program may run in this test: {program}");
    }
}
struct NoKiller;
impl ProcessKiller for NoKiller {
    fn kill(&self, pid: u32) -> Result<(), String> {
        panic!("no process may be killed in this test: {pid}");
    }
}

fn write(p: &Path, n: usize) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, vec![9u8; n]).unwrap();
}

/// A fake developer machine on disk: a Next.js project, a Temp folder, an Android SDK.
struct Machine {
    _root: tempfile::TempDir,
    env: ScanEnv,
    project: PathBuf,
    temp: PathBuf,
    sdk: PathBuf,
}

fn machine() -> Machine {
    let root = tempfile::tempdir().unwrap();
    let base = root.path().to_path_buf();
    let project = base.join("work").join("shop");
    let temp = base.join("Local").join("Temp");
    let sdk = base.join("Local").join("Android").join("Sdk");
    write(&project.join("package.json"), 20);
    write(&project.join("src").join("app.ts"), 100);
    write(
        &project.join("node_modules").join("react").join("index.js"),
        5_000,
    );
    write(&project.join(".next").join("cache").join("chunk"), 3_000);
    write(&temp.join("old.tmp"), 2_000);
    write(
        &sdk.join("platforms").join("android-35").join("android.jar"),
        8_000,
    );
    let env = ScanEnv {
        local_app_data: Some(base.join("Local")),
        temp: Some(temp.clone()),
        dev_roots: vec![base.join("work")],
        ..Default::default()
    };
    Machine {
        _root: root,
        env,
        project,
        temp,
        sdk,
    }
}

fn opts() -> ScanOptions {
    ScanOptions {
        large_file_bytes: 1 << 40,
        ..Default::default()
    }
}

/// Snapshot of a PC whose C: drive is nearly full.
fn low_disk_snapshot(free_gb: u64) -> SystemSnapshot {
    let memory = memory_metrics(16 * GIB, 9 * GIB);
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
    let d = disk_metrics("Win".into(), "C:\\".into(), 275 * GIB, free_gb * GIB);
    SystemSnapshot {
        timestamp_ms: 1,
        memory,
        cpu_percent: 12.0,
        disk_health: vec![build_report(&DiskThresholds::default(), &d, &[], 0.0)],
        disks: vec![d],
        pressure: Some(pressure),
        uptime_secs: 10,
        process_count: 5,
    }
}

fn run_diagnosis(snap: &SystemSnapshot, entries: &[ScanEntry]) -> DiagnosisReport {
    let workloads = detect_workloads(&[sample(1, "node.exe", GIB, 1.0, Category::Node)]);
    diagnose(
        &Input {
            memory: &snap.memory,
            pressure: snap.pressure.as_ref().unwrap(),
            workloads: &workloads,
            growth: &[],
            disks: &snap.disks,
            disk_health: &snap.disk_health,
            docker: None,
            storage: Some(entries),
        },
        &Config::default(),
        1,
    )
}

/// The scanner reports tiny fixture sizes; recommendations ignore items under 100 MB, so inflate them
/// the way a real multi-gigabyte machine would look.
fn inflate(mut entries: Vec<ScanEntry>) -> Vec<ScanEntry> {
    for e in &mut entries {
        e.size_bytes = 3 * GIB;
    }
    entries
}

fn ctx<'a>(facts: &'a Facts) -> Ctx<'a> {
    Ctx {
        facts,
        runner: &NoRunner,
        killer: &NoKiller,
    }
}

#[test]
fn full_session_scan_diagnose_approve_clean() {
    let m = machine();
    let result = run_scan(
        &m.env,
        &opts(),
        &AtomicBool::new(false),
        &Mutex::new(ScanProgress::default()),
    );
    assert!(result.complete);

    // The scanner found the right things and classified them without touching anything.
    let kinds = |p: &Path| {
        result
            .entries
            .iter()
            .find(|e| Path::new(&e.path) == p)
            .map(|e| (e.kind, e.safety))
    };
    use crate::storage::safety::{EntryKind, Safety};
    assert_eq!(
        kinds(&m.project.join("node_modules")),
        Some((EntryKind::NodeModules, Safety::RequiresReview))
    );
    assert_eq!(
        kinds(&m.project.join(".next")),
        Some((EntryKind::BuildOutput, Safety::SafeToReview))
    );
    assert_eq!(
        kinds(&m.temp),
        Some((EntryKind::TempCache, Safety::SafeToClean))
    );
    assert_eq!(
        kinds(&m.sdk),
        Some((EntryKind::AndroidSdk, Safety::DoNotTouch))
    );
    assert!(
        m.project.join("node_modules").exists(),
        "scanning is read-only"
    );

    // A nearly full C: drive produces a storage-pressure diagnosis that offers cleanup of the temp cache.
    let entries = inflate(result.entries.clone());
    let snap = low_disk_snapshot(6);
    let report = run_diagnosis(&snap, &entries);
    assert_eq!(report.status, Status::Problem);
    assert_eq!(report.headline.rule_id, "low-disk");
    assert!(report.headline.severity >= Severity::High);

    let rec = report
        .headline
        .recommendations
        .iter()
        .find(|r| {
            r.action
                .as_ref()
                .is_some_and(|a| a.params["path"] == m.temp.to_string_lossy().as_ref())
        })
        .expect("temp cache is offered");
    assert_eq!(rec.risk, crate::diagnosis::model::Risk::Low);
    // Nothing was offered for the SDK, and node_modules is manual-only.
    assert!(
        report
            .headline
            .recommendations
            .iter()
            .all(|r| !r.id.contains("Sdk"))
    );
    let nm = report
        .headline
        .recommendations
        .iter()
        .find(|r| r.id.contains("node_modules"))
        .unwrap();
    assert!(nm.action.is_none() && nm.manual_steps.is_some());

    // Approve the recommendation: the engine re-validates everything from fresh facts and cleans.
    let action = rec.action.as_ref().unwrap();
    let facts = Facts {
        scan_entries: entries.clone(),
        user_profile: Some(PathBuf::from("C:\\Users\\nobody")),
        ..Default::default()
    };
    let engine = ActionEngine::default();
    let preview = engine
        .preview(&action.action_id, &action.params, &ctx(&facts))
        .unwrap();
    assert_eq!(preview.risk, RiskLevel::Low);
    let done = engine
        .execute(
            &action.action_id,
            &action.params,
            Confirmation::default(),
            &ctx(&facts),
        )
        .unwrap();
    assert!(done.success);
    assert!(
        m.temp.exists() && std::fs::read_dir(&m.temp).unwrap().count() == 0,
        "temp emptied, folder kept"
    );
    assert!(
        m.project.join("node_modules").exists(),
        "other items untouched"
    );
    assert!(m.sdk.exists());
    assert_eq!(engine.history().len(), 1);

    // The AI payload for this very report leaks no local paths.
    let payload = build_payload(&snap, &detect_workloads(&[]), &[], &report);
    let json = serde_json::to_string(&payload).unwrap();
    assert!(
        !json.contains(&*m.temp.to_string_lossy().replace('\\', "\\\\"))
            && !json.contains("node_modules\\\\"),
        "{json}"
    );
}

#[test]
fn destructive_cleanup_of_node_modules_needs_both_confirmations_end_to_end() {
    let m = machine();
    let result = run_scan(
        &m.env,
        &opts(),
        &AtomicBool::new(false),
        &Mutex::new(ScanProgress::default()),
    );
    let facts = Facts {
        scan_entries: result.entries,
        user_profile: Some(PathBuf::from("C:\\Users\\nobody")),
        ..Default::default()
    };
    let engine = ActionEngine::default();
    let nm = m.project.join("node_modules");
    let params = json!({ "path": nm.to_string_lossy() });

    for c in [
        Confirmation::default(),
        Confirmation {
            confirmed: true,
            double_confirmed: false,
        },
        Confirmation {
            confirmed: false,
            double_confirmed: true,
        },
    ] {
        let r = engine.execute("cleanup.path", &params, c, &ctx(&facts));
        assert!(
            matches!(r, Err(ActionError::ConfirmationRequired(_))),
            "{c:?}"
        );
        assert!(nm.exists());
    }
    engine
        .execute(
            "cleanup.path",
            &params,
            Confirmation {
                confirmed: true,
                double_confirmed: true,
            },
            &ctx(&facts),
        )
        .unwrap();
    assert!(!nm.exists());
    assert!(
        m.project.join("package.json").exists() && m.project.join("src").join("app.ts").exists(),
        "only the dependency folder went"
    );
}

#[test]
fn a_forged_request_cannot_reach_files_outside_the_scan() {
    let m = machine();
    let result = run_scan(
        &m.env,
        &opts(),
        &AtomicBool::new(false),
        &Mutex::new(ScanProgress::default()),
    );
    let facts = Facts {
        scan_entries: result.entries,
        user_profile: Some(PathBuf::from("C:\\Users\\nobody")),
        ..Default::default()
    };
    let engine = ActionEngine::default();
    let src = m.project.join("src");
    let sdk_file = m
        .sdk
        .join("platforms")
        .join("android-35")
        .join("android.jar");
    for forged in [
        json!({ "path": src.to_string_lossy() }), // real folder, never scanned
        json!({ "path": sdk_file.to_string_lossy() }), // inside a do-not-touch tree
        json!({ "path": m.sdk.to_string_lossy() }), // the do-not-touch tree itself
        json!({ "path": format!("{}\\..\\..", m.temp.display()) }), // traversal
        json!({ "path": "C:\\Windows\\System32" }),
        json!({ "path": "C:\\" }),
        json!({ "path": "relative\\node_modules" }),
    ] {
        let r = engine.execute(
            "cleanup.path",
            &forged,
            Confirmation {
                confirmed: true,
                double_confirmed: true,
            },
            &ctx(&facts),
        );
        assert!(r.is_err(), "{forged} must be refused");
    }
    assert!(src.exists() && sdk_file.exists() && m.temp.exists());
    assert!(
        engine.history().is_empty(),
        "refused requests never reach execution"
    );
}

#[test]
fn healthy_machine_end_to_end_is_calm() {
    let m = machine();
    let result = run_scan(
        &m.env,
        &opts(),
        &AtomicBool::new(false),
        &Mutex::new(ScanProgress::default()),
    );
    let snap = low_disk_snapshot(200);
    let report = run_diagnosis(&snap, &result.entries);
    assert_eq!(report.status, Status::Healthy);
    assert_eq!(report.headline.rule_id, "healthy");
}

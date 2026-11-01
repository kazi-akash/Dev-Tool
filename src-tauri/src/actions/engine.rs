//! Action registry plus the permission rules that stand between a request and its side effects.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use super::builtin;
use super::model::{
    ActionStatus, Measurement, Reversibility, Verification, VerifyState,
    Action, ActionError, ActionSpec, Confirmation, Ctx, Outcome, Preview, RiskLevel,
};

const HISTORY_LIMIT: usize = 200;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRecord {
    pub id: u64,
    pub action_id: String,
    pub name: String,
    pub at_ms: u64,
    pub risk: RiskLevel,
    /// Ran from safe automation instead of a click.
    pub automated: bool,
    pub success: bool,
    pub summary: String,
    pub freed_bytes: Option<u64>,
    pub rolled_back: bool,
    /// DevPilot can undo this run with one click (rollback).
    pub reversible: bool,
    /// What actually happened, not what the spec hoped for.
    pub reversibility: Reversibility,
    pub status: ActionStatus,
    /// Result of the VERIFY step; `None` when the action failed before it.
    pub verification: Option<Verification>,
    /// Result of the MEASURE step: before/after RAM and disk.
    pub measurement: Option<Measurement>,
    #[serde(skip)]
    outcome: Option<Outcome>,
}

/// Everything one execution produced.
struct Run {
    result: Result<Outcome, ActionError>,
    verification: Option<Verification>,
    measurement: Option<Measurement>,
}

type Sink = Box<dyn Fn(&ActionRecord) + Send + Sync>;

pub struct ActionEngine {
    actions: HashMap<&'static str, Box<dyn Action>>,
    log: Mutex<VecDeque<ActionRecord>>,
    next_id: AtomicU64,
    /// Called for every recorded action (used to persist history).
    sink: OnceLock<Sink>,
    /// How long a failed verification is retried while the system settles (processes exit, containers stop).
    settle: Duration,
    /// Pause before the closing measurement so released memory shows up.
    measure_delay: Duration,
}

impl Default for ActionEngine {
    fn default() -> Self {
        Self::new(builtin::all())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl ActionEngine {
    pub fn new(actions: Vec<Box<dyn Action>>) -> Self {
        Self {
            actions: actions.into_iter().map(|a| (a.spec().id, a)).collect(),
            log: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
            sink: OnceLock::new(),
            settle: Duration::from_secs(3),
            measure_delay: Duration::from_millis(600),
        }
    }

    /// Override the verify/measure timing (tests use zero so they do not sleep).
    #[cfg(test)]
    pub fn with_timing(mut self, settle: Duration, measure_delay: Duration) -> Self {
        self.settle = settle;
        self.measure_delay = measure_delay;
        self
    }

    /// Register the persistence hook. Only the first call has an effect.
    pub fn set_sink(&self, sink: impl Fn(&ActionRecord) + Send + Sync + 'static) {
        let _ = self.sink.set(Box::new(sink));
    }

    fn get(&self, id: &str) -> Result<&dyn Action, ActionError> {
        self.actions
            .get(id)
            .map(|a| a.as_ref())
            .ok_or_else(|| ActionError::Unknown(id.to_string()))
    }

    pub fn list(&self) -> Vec<ActionSpec> {
        let mut v: Vec<ActionSpec> = self.actions.values().map(|a| *a.spec()).collect();
        v.sort_by_key(|s| s.id);
        v
    }

    pub fn preview(&self, id: &str, params: &Value, ctx: &Ctx) -> Result<Preview, ActionError> {
        self.get(id)?.preview(params, ctx)
    }

    /// EXECUTE, then VERIFY (retrying while the system settles), then MEASURE.
    fn run(&self, action: &dyn Action, params: &Value, ctx: &Ctx) -> Run {
        let path = action.measure_path(params, ctx);
        let before = ctx.probe.reading(path.as_deref());
        let result = action.execute(params, ctx);
        let Ok(outcome) = &result else {
            return Run {
                result,
                verification: None,
                measurement: None,
            };
        };
        let started = Instant::now();
        let verification = loop {
            let v = action.verify(params, outcome, ctx);
            // Only "not confirmed" is retried: it may just be slow (a process still exiting).
            if v.state != VerifyState::NotConfirmed || started.elapsed() >= self.settle {
                break v;
            }
            std::thread::sleep(Duration::from_millis(250).min(self.settle));
        };
        if !self.measure_delay.is_zero() {
            std::thread::sleep(self.measure_delay);
        }
        let after = ctx.probe.reading(path.as_deref());
        let measurement = Measurement::new(before, after, action.measure_note());
        Run {
            result,
            verification: Some(verification),
            measurement: Some(measurement),
        }
    }

    fn record(&self, spec: &ActionSpec, risk: RiskLevel, automated: bool, run: Run) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (status, success, summary, freed, outcome, reversibility) = match &run.result {
            Ok(o) => {
                let status = match run.verification.as_ref().map(|v| v.state) {
                    Some(VerifyState::Verified) => ActionStatus::Success,
                    Some(VerifyState::NotConfirmed) => ActionStatus::NotEffective,
                    _ => ActionStatus::Unverified,
                };
                (
                    status,
                    true,
                    o.summary.clone(),
                    o.freed_bytes,
                    Some(o.clone()),
                    o.reversibility,
                )
            }
            Err(e) => (
                ActionStatus::Failed,
                false,
                e.to_string(),
                None,
                None,
                spec.reversibility,
            ),
        };
        let rec = ActionRecord {
            id,
            action_id: spec.id.into(),
            name: spec.name.into(),
            at_ms: now_ms(),
            risk,
            automated,
            success,
            summary,
            freed_bytes: freed,
            rolled_back: false,
            // A rollback button is only honest when the spec has one and this run did not lose it.
            reversible: spec.reversible && reversibility != Reversibility::Irreversible,
            reversibility,
            status,
            verification: run.verification,
            measurement: run.measurement,
            outcome,
        };
        if let Some(sink) = self.sink.get() {
            sink(&rec);
        }
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        log.push_front(rec);
        log.truncate(HISTORY_LIMIT);
        id
    }

    /// Execute after the user approved. The action is re-previewed here, so validation and the
    /// risk level never depend on anything the caller (UI or AI) claims.
    pub fn execute(
        &self,
        id: &str,
        params: &Value,
        confirmation: Confirmation,
        ctx: &Ctx,
    ) -> Result<ActionRecord, ActionError> {
        let action = self.get(id)?;
        let preview = action.preview(params, ctx)?;

        if preview.requires_confirmation && !confirmation.confirmed {
            return Err(ActionError::ConfirmationRequired(format!(
                "{:?} risk actions need your confirmation.",
                preview.risk
            )));
        }
        if preview.requires_double_confirmation && !confirmation.double_confirmed {
            return Err(ActionError::ConfirmationRequired(
                "Destructive actions need a second confirmation.".into(),
            ));
        }

        let run = self.run(action, params, ctx);
        let failure = run.result.as_ref().err().cloned();
        let rec_id = self.record(action.spec(), preview.risk, false, run);
        match failure {
            Some(e) => Err(e),
            None => Ok(self.find(rec_id).expect("record was just inserted")),
        }
    }

    /// Safe automation: only actions flagged eligible, only at LOW risk or below, never confirmed
    /// by a click. Anything else is refused.
    pub fn execute_automated(
        &self,
        id: &str,
        params: &Value,
        ctx: &Ctx,
    ) -> Result<ActionRecord, ActionError> {
        let action = self.get(id)?;
        if !action.spec().automation_eligible {
            return Err(ActionError::Refused(format!(
                "'{id}' is not allowed to run automatically."
            )));
        }
        let preview = action.preview(params, ctx)?;
        if preview.risk > RiskLevel::Low {
            return Err(ActionError::Refused(
                "Automation only runs LOW risk actions.".into(),
            ));
        }
        let run = self.run(action, params, ctx);
        let failure = run.result.as_ref().err().cloned();
        let rec_id = self.record(action.spec(), preview.risk, true, run);
        match failure {
            Some(e) => Err(e),
            None => Ok(self.find(rec_id).expect("record was just inserted")),
        }
    }

    pub fn rollback(&self, record_id: u64, ctx: &Ctx) -> Result<String, ActionError> {
        let (action_id, outcome) = {
            let log = self.log.lock().unwrap_or_else(|e| e.into_inner());
            let rec = log
                .iter()
                .find(|r| r.id == record_id)
                .ok_or_else(|| ActionError::Invalid("no such history entry".into()))?;
            if rec.rolled_back {
                return Err(ActionError::Invalid(
                    "this action was already rolled back".into(),
                ));
            }
            if !rec.success {
                return Err(ActionError::Invalid(
                    "a failed action has nothing to roll back".into(),
                ));
            }
            (
                rec.action_id.clone(),
                rec.outcome
                    .clone()
                    .ok_or_else(|| ActionError::Invalid("nothing to roll back".into()))?,
            )
        };
        let result = self
            .get(&action_id)?
            .rollback(&outcome, ctx)
            .ok_or_else(|| ActionError::Refused("this action cannot be undone".into()))??;
        if let Some(rec) = self
            .log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter_mut()
            .find(|r| r.id == record_id)
        {
            rec.rolled_back = true;
        }
        Ok(result)
    }

    fn find(&self, id: u64) -> Option<ActionRecord> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|r| r.id == id)
            .cloned()
    }

    pub fn history(&self) -> Vec<ActionRecord> {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::actions::model::{
        CommandRunner, Facts, Probe, ProbeProcess, ProcessKiller, SystemReading, TrashReceipt,
        Trasher,
    };
    use crate::processes::model::{Category, sample};
    use crate::storage::safety::{EntryKind, Safety};
    use crate::storage::scanner::ScanEntry;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<String>>,
        fail: bool,
        /// `docker ps` keeps listing the container (the stop did not take effect).
        still_running: bool,
    }
    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            if self.fail {
                Err("boom".into())
            } else if args.first() == Some(&"ps") {
                Ok(if self.still_running {
                    "redis\n".into()
                } else {
                    String::new()
                })
            } else {
                Ok("ok".into())
            }
        }
    }

    #[derive(Default)]
    struct FakeKiller {
        killed: Mutex<Vec<u32>>,
    }
    impl ProcessKiller for FakeKiller {
        fn kill(&self, pid: u32) -> Result<(), String> {
            self.killed.lock().unwrap().push(pid);
            Ok(())
        }
    }

    /// Probe with scripted readings and process list.
    struct FakeProbe {
        readings: Mutex<std::collections::VecDeque<SystemReading>>,
        procs: Mutex<Option<Vec<ProbeProcess>>>,
    }
    impl Default for FakeProbe {
        fn default() -> Self {
            Self {
                readings: Mutex::default(),
                procs: Mutex::new(Some(vec![])),
            }
        }
    }
    impl Probe for FakeProbe {
        fn reading(&self, _: Option<&Path>) -> SystemReading {
            self.readings.lock().unwrap().pop_front().unwrap_or_default()
        }
        fn processes(&self) -> Option<Vec<ProbeProcess>> {
            self.procs.lock().unwrap().clone()
        }
    }

    /// Recycle Bin double: really removes the item so verification sees it gone.
    struct FakeTrasher {
        in_bin: bool,
        trashed: Mutex<Vec<PathBuf>>,
        restored: Mutex<Vec<PathBuf>>,
    }
    impl Default for FakeTrasher {
        fn default() -> Self {
            Self {
                in_bin: true,
                trashed: Mutex::default(),
                restored: Mutex::default(),
            }
        }
    }
    impl Trasher for FakeTrasher {
        fn trash(&self, p: &Path) -> Result<TrashReceipt, String> {
            self.trashed.lock().unwrap().push(p.to_path_buf());
            if p.is_dir() {
                std::fs::remove_dir_all(p).map_err(|e| e.to_string())?;
            } else {
                std::fs::remove_file(p).map_err(|e| e.to_string())?;
            }
            Ok(TrashReceipt { in_bin: self.in_bin })
        }
        fn restore(&self, p: &Path) -> Result<(), String> {
            self.restored.lock().unwrap().push(p.to_path_buf());
            Ok(())
        }
    }

    struct Rig {
        engine: ActionEngine,
        runner: FakeRunner,
        killer: FakeKiller,
        probe: FakeProbe,
        trasher: FakeTrasher,
        facts: Facts,
    }

    impl Rig {
        fn new(facts: Facts) -> Self {
            Rig {
                engine: ActionEngine::default().with_timing(Duration::ZERO, Duration::ZERO),
                runner: FakeRunner::default(),
                killer: FakeKiller::default(),
                probe: FakeProbe::default(),
                trasher: FakeTrasher::default(),
                facts,
            }
        }
        fn ctx(&self) -> Ctx<'_> {
            Ctx {
                facts: &self.facts,
                runner: &self.runner,
                killer: &self.killer,
                probe: &self.probe,
                trasher: &self.trasher,
            }
        }
        fn exec(&self, id: &str, p: Value, c: bool, d: bool) -> Result<ActionRecord, ActionError> {
            self.engine.execute(
                id,
                &p,
                Confirmation {
                    confirmed: c,
                    double_confirmed: d,
                },
                &self.ctx(),
            )
        }
        fn calls(&self) -> Vec<String> {
            self.runner.calls.lock().unwrap().clone()
        }
    }

    fn entry(path: &Path, kind: EntryKind) -> ScanEntry {
        ScanEntry {
            path: path.to_string_lossy().into_owned(),
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            is_dir: true,
            size_bytes: 1,
            file_count: 1,
            last_modified_secs: None,
            kind,
            kind_label: "x",
            // Deliberately wrong: the engine must recompute safety from `kind`, not trust this.
            safety: Safety::SafeToClean,
            reason: "r",
        }
    }

    fn profile() -> PathBuf {
        PathBuf::from("C:\\Users\\devpilot-test")
    }

    fn facts_with(entries: Vec<ScanEntry>) -> Facts {
        Facts {
            scan_entries: entries,
            user_profile: Some(profile()),
            ..Default::default()
        }
    }

    fn write(p: &Path, n: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![1u8; n]).unwrap();
    }

    // --- registry & permissions ----------------------------------------------------------

    #[test]
    fn every_action_has_complete_metadata() {
        let engine = ActionEngine::default();
        let specs = engine.list();
        assert_eq!(specs.len(), 6);
        for s in &specs {
            assert!(
                !s.id.is_empty()
                    && !s.name.is_empty()
                    && !s.description.is_empty()
                    && !s.estimated_benefit.is_empty(),
                "{}",
                s.id
            );
        }
        let ids: Vec<_> = specs.iter().map(|s| s.id).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
    }

    #[test]
    fn every_action_the_diagnosis_can_recommend_is_registered() {
        let ids: Vec<_> = ActionEngine::default()
            .list()
            .iter()
            .map(|s| s.id)
            .collect();
        for referenced in [
            "docker.stop_container",
            "wsl.shutdown",
            "gradle.stop",
            "process.terminate",
            "cleanup.path",
        ] {
            assert!(ids.contains(&referenced), "{referenced}");
        }
    }

    #[test]
    fn unknown_actions_are_rejected_even_with_confirmation() {
        let rig = Rig::new(Facts::default());
        for id in ["rm -rf", "powershell", "registry.write", ""] {
            assert!(
                matches!(
                    rig.exec(id, json!({}), true, true),
                    Err(ActionError::Unknown(_))
                ),
                "{id}"
            );
        }
        assert!(rig.calls().is_empty());
    }

    #[test]
    fn low_risk_runs_without_confirmation_and_calls_only_fixed_commands() {
        let rig = Rig::new(Facts::default());
        let rec = rig
            .exec(
                "docker.stop_container",
                json!({ "container": "redis" }),
                false,
                false,
            )
            .unwrap();
        assert!(rec.success && rec.reversible);
        assert_eq!(rig.calls()[0], "docker stop redis");
        assert_eq!(rec.status, ActionStatus::Success, "the stop was verified");
        assert_eq!(rec.reversibility, Reversibility::Reversible);
    }

    #[test]
    fn medium_risk_needs_confirmation() {
        let rig = Rig::new(Facts::default());
        let err = rig
            .exec("wsl.shutdown", json!({}), false, false)
            .unwrap_err();
        assert!(matches!(err, ActionError::ConfirmationRequired(_)));
        assert!(rig.calls().is_empty(), "nothing ran");
        rig.exec("wsl.shutdown", json!({}), true, false).unwrap();
        assert_eq!(rig.calls(), vec!["wsl --shutdown"]);
    }

    #[test]
    fn container_names_cannot_inject_flags_or_commands() {
        let rig = Rig::new(Facts::default());
        for bad in [
            "--all",
            "-f",
            "a; calc",
            "a && b",
            "$(whoami)",
            "x y",
            "..\\..",
            "",
        ] {
            let r = rig.exec(
                "docker.stop_container",
                json!({ "container": bad }),
                true,
                true,
            );
            assert!(r.is_err(), "{bad}");
        }
        assert!(
            rig.exec("docker.stop_container", json!({}), true, true)
                .is_err()
        );
        assert!(
            rig.exec(
                "docker.stop_container",
                json!({ "container": 5 }),
                true,
                true
            )
            .is_err()
        );
        assert!(rig.calls().is_empty());
    }

    #[test]
    fn failures_are_reported_and_logged() {
        let mut rig = Rig::new(Facts::default());
        rig.runner = FakeRunner {
            fail: true,
            ..Default::default()
        };
        let err = rig
            .exec(
                "docker.stop_container",
                json!({ "container": "x" }),
                true,
                false,
            )
            .unwrap_err();
        assert!(matches!(err, ActionError::Failed(_)));
        let h = rig.engine.history();
        assert_eq!(h.len(), 1);
        assert!(!h[0].success && h[0].summary.contains("boom"));
    }

    #[test]
    fn history_is_newest_first_and_capped() {
        let rig = Rig::new(Facts::default());
        for i in 0..(HISTORY_LIMIT + 10) {
            rig.exec(
                "docker.stop_container",
                json!({ "container": format!("c{i}") }),
                false,
                false,
            )
            .unwrap();
        }
        let h = rig.engine.history();
        assert_eq!(h.len(), HISTORY_LIMIT);
        assert!(h[0].id > h[1].id);
    }

    // --- rollback ------------------------------------------------------------------------------

    #[test]
    fn stopped_container_can_be_started_again_once() {
        let rig = Rig::new(Facts::default());
        let rec = rig
            .exec(
                "docker.stop_container",
                json!({ "container": "redis" }),
                false,
                false,
            )
            .unwrap();
        let msg = rig.engine.rollback(rec.id, &rig.ctx()).unwrap();
        assert!(msg.contains("Started container redis"));
        let calls = rig.calls();
        assert_eq!(calls.first().unwrap(), "docker stop redis");
        assert_eq!(calls.last().unwrap(), "docker start redis");
        assert!(rig.engine.history()[0].rolled_back);
        assert!(
            rig.engine.rollback(rec.id, &rig.ctx()).is_err(),
            "only once"
        );
    }

    #[test]
    fn irreversible_actions_refuse_rollback() {
        let rig = Rig::new(Facts::default());
        let rec = rig.exec("wsl.shutdown", json!({}), true, false).unwrap();
        assert!(matches!(
            rig.engine.rollback(rec.id, &rig.ctx()),
            Err(ActionError::Refused(_))
        ));
        assert!(rig.engine.rollback(9999, &rig.ctx()).is_err());
    }

    // --- process.terminate -------------------------------------------------------------------

    fn facts_with_processes() -> Facts {
        Facts {
            processes: vec![
                sample(100, "node.exe", 1 << 30, 1.0, Category::Node),
                sample(4, "System", 0, 0.0, Category::System),
                sample(
                    std::process::id(),
                    "devpilot.exe",
                    1 << 20,
                    0.0,
                    Category::Other,
                ),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn closing_a_process_needs_confirmation_and_checks_identity() {
        let rig = Rig::new(facts_with_processes());
        assert!(matches!(
            rig.exec(
                "process.terminate",
                json!({ "pid": 100, "name": "node.exe" }),
                false,
                false
            ),
            Err(ActionError::ConfirmationRequired(_))
        ));
        assert!(rig.killer.killed.lock().unwrap().is_empty());
        rig.exec(
            "process.terminate",
            json!({ "pid": 100, "name": "NODE.EXE" }),
            true,
            false,
        )
        .unwrap();
        assert_eq!(*rig.killer.killed.lock().unwrap(), vec![100]);
    }

    #[test]
    fn terminate_refuses_system_self_reused_pid_and_missing() {
        let rig = Rig::new(facts_with_processes());
        let refused = |p: Value| {
            matches!(
                rig.exec("process.terminate", p, true, true),
                Err(ActionError::Refused(_))
            )
        };
        assert!(refused(json!({ "pid": 4 })), "system process");
        assert!(refused(json!({ "pid": std::process::id() })), "itself");
        assert!(
            refused(json!({ "pid": 100, "name": "chrome.exe" })),
            "PID now belongs to something else"
        );
        assert!(matches!(
            rig.exec("process.terminate", json!({ "pid": 424242 }), true, true),
            Err(ActionError::Invalid(_))
        ));
        assert!(matches!(
            rig.exec("process.terminate", json!({}), true, true),
            Err(ActionError::Invalid(_))
        ));
        assert!(matches!(
            rig.exec("process.terminate", json!({ "pid": -1 }), true, true),
            Err(ActionError::Invalid(_))
        ));
        assert!(rig.killer.killed.lock().unwrap().is_empty());
    }

    // --- cleanup.path ----------------------------------------------------------------------------

    #[test]
    fn cleanup_only_touches_items_from_the_scan() {
        let t = tempfile::tempdir().unwrap();
        let victim = t.path().join("proj").join("node_modules");
        write(&victim.join("a.js"), 10);
        let rig = Rig::new(facts_with_processes());
        let r = rig.exec(
            "cleanup.path",
            json!({ "path": victim.to_string_lossy() }),
            true,
            true,
        );
        assert!(matches!(r, Err(ActionError::Refused(m)) if m.contains("scan")));
        assert!(victim.exists());
    }

    #[test]
    fn cleanup_rejects_traversal_relative_protected_and_do_not_touch() {
        let t = tempfile::tempdir().unwrap();
        let sdk = t.path().join("Android").join("Sdk");
        write(&sdk.join("x"), 1);
        let rig = Rig::new(facts_with_processes());
        // Even if "scanned", DoNotTouch kinds are refused and protected paths never reach the list.
        let mut facts = rig.facts.clone();
        facts.scan_entries = vec![
            entry(&sdk, EntryKind::AndroidSdk),
            entry(Path::new("C:\\Windows\\System32"), EntryKind::UserData),
            entry(
                Path::new("C:\\Users\\devpilot-test\\Documents"),
                EntryKind::UserData,
            ),
        ];
        let rig = Rig { facts, ..rig };
        for p in [
            sdk.to_string_lossy().into_owned(),
            "C:\\Windows\\System32".into(),
            "C:\\Users\\devpilot-test\\Documents".into(),
            "C:\\".into(),
            "node_modules".into(),
            "..\\..\\Windows".into(),
            format!("{}\\..\\..", t.path().display()),
            "\\\\server\\share\\x".into(),
        ] {
            let r = rig.exec("cleanup.path", json!({ "path": p }), true, true);
            assert!(matches!(r, Err(ActionError::Refused(_))), "{p}: {r:?}");
        }
        assert!(sdk.join("x").exists());
    }

    #[test]
    fn node_modules_goes_to_the_recycle_bin_by_default_and_needs_two_confirmations() {
        let t = tempfile::tempdir().unwrap();
        let nm = t.path().join("proj").join("node_modules");
        write(&nm.join("pkg").join("index.js"), 100);
        // The entry lies about its safety (SAFE_TO_CLEAN); the engine uses the kind.
        let rig = Rig::new(facts_with(vec![entry(&nm, EntryKind::NodeModules)]));
        let args = json!({ "path": nm.to_string_lossy() });

        let p = rig.engine.preview("cleanup.path", &args, &rig.ctx()).unwrap();
        assert_eq!(p.risk, RiskLevel::Destructive);
        assert!(p.requires_confirmation && p.requires_double_confirmation);
        assert_eq!(p.reversibility, Reversibility::Recoverable);
        assert!(p.reversible, "restore from the Recycle Bin is offered");
        assert_eq!(
            p.estimated_benefit_bytes, None,
            "no space is freed until the bin is emptied, so none is promised"
        );
        assert!(p.warnings.iter().any(|w| w.contains("exists nowhere else")));
        assert!(p.warnings.iter().any(|w| w.contains("emptied")));

        assert!(matches!(
            rig.exec("cleanup.path", args.clone(), false, false),
            Err(ActionError::ConfirmationRequired(_))
        ));
        assert!(matches!(
            rig.exec("cleanup.path", args.clone(), true, false),
            Err(ActionError::ConfirmationRequired(_))
        ));
        assert!(nm.exists(), "still there after incomplete confirmations");

        let rec = rig.exec("cleanup.path", args, true, true).unwrap();
        assert!(!nm.exists());
        assert_eq!(rig.trasher.trashed.lock().unwrap().len(), 1);
        assert_eq!(rec.freed_bytes, None, "moving to the bin frees nothing yet");
        assert_eq!(rec.reversibility, Reversibility::Recoverable);
        assert_eq!(rec.status, ActionStatus::Success);
        assert!(rec.reversible);
    }

    #[test]
    fn recycle_bin_move_can_be_rolled_back_once() {
        let t = tempfile::tempdir().unwrap();
        let nm = t.path().join("proj").join("node_modules");
        write(&nm.join("pkg").join("index.js"), 100);
        let rig = Rig::new(facts_with(vec![entry(&nm, EntryKind::NodeModules)]));
        let rec = rig
            .exec("cleanup.path", json!({ "path": nm.to_string_lossy() }), true, true)
            .unwrap();
        let msg = rig.engine.rollback(rec.id, &rig.ctx()).unwrap();
        assert!(msg.contains("Restored"));
        assert_eq!(rig.trasher.restored.lock().unwrap().len(), 1);
        assert!(rig.engine.rollback(rec.id, &rig.ctx()).is_err(), "only once");
    }

    #[test]
    fn if_windows_keeps_no_bin_copy_the_record_says_irreversible_and_offers_no_undo() {
        let t = tempfile::tempdir().unwrap();
        let nm = t.path().join("proj").join("node_modules");
        write(&nm.join("pkg").join("index.js"), 100);
        let mut rig = Rig::new(facts_with(vec![entry(&nm, EntryKind::NodeModules)]));
        rig.trasher.in_bin = false;
        let rec = rig
            .exec("cleanup.path", json!({ "path": nm.to_string_lossy() }), true, true)
            .unwrap();
        assert_eq!(rec.reversibility, Reversibility::Irreversible);
        assert!(!rec.reversible, "no rollback is claimed when no copy exists");
        assert!(rec.summary.contains("permanent"));
        assert_eq!(rec.freed_bytes, Some(100));
        assert!(rig.engine.rollback(rec.id, &rig.ctx()).is_err());
    }

    #[test]
    fn permanent_request_deletes_and_is_irreversible() {
        let t = tempfile::tempdir().unwrap();
        let nm = t.path().join("proj").join("node_modules");
        write(&nm.join("pkg").join("index.js"), 100);
        let rig = Rig::new(facts_with(vec![entry(&nm, EntryKind::NodeModules)]));
        let args = json!({ "path": nm.to_string_lossy(), "permanent": true });
        let p = rig.engine.preview("cleanup.path", &args, &rig.ctx()).unwrap();
        assert_eq!(p.reversibility, Reversibility::Irreversible);
        assert!(!p.reversible);
        assert_eq!(p.estimated_benefit_bytes, Some(100));
        assert!(p.warnings.iter().any(|w| w.contains("not moved to the Recycle Bin")));
        let rec = rig.exec("cleanup.path", args, true, true).unwrap();
        assert_eq!(rec.freed_bytes, Some(100));
        assert!(!nm.exists());
        assert!(rig.trasher.trashed.lock().unwrap().is_empty(), "the bin was bypassed on request");
        assert_eq!(rec.reversibility, Reversibility::Irreversible);
    }

    #[test]
    fn measurement_reports_before_after_and_a_system_wide_caveat() {
        let rig = Rig::new(Facts::default());
        {
            let mut q = rig.probe.readings.lock().unwrap();
            q.push_back(SystemReading {
                ram_used_bytes: Some(15 * GIB),
                ram_available_bytes: Some(GIB),
                disk_free_bytes: None,
            });
            q.push_back(SystemReading {
                ram_used_bytes: Some(13 * GIB),
                ram_available_bytes: Some(3 * GIB),
                disk_free_bytes: None,
            });
        }
        let rec = rig
            .exec("docker.stop_container", json!({ "container": "redis" }), false, false)
            .unwrap();
        let m = rec.measurement.as_ref().unwrap();
        assert_eq!(m.ram_recovered_bytes, Some(2 * GIB as i64));
        assert_eq!(m.before.ram_used_bytes, Some(15 * GIB));
        assert!(m.note.contains("system-wide"));
        assert!(m.note.contains("keeps memory"), "docker caveat present: {}", m.note);
    }

    #[test]
    fn memory_that_grew_is_reported_as_negative_not_clamped() {
        let rig = Rig::new(Facts::default());
        {
            let mut q = rig.probe.readings.lock().unwrap();
            q.push_back(SystemReading {
                ram_available_bytes: Some(3 * GIB),
                ..Default::default()
            });
            q.push_back(SystemReading {
                ram_available_bytes: Some(2 * GIB),
                ..Default::default()
            });
        }
        let rec = rig
            .exec("docker.stop_container", json!({ "container": "redis" }), false, false)
            .unwrap();
        assert_eq!(rec.measurement.unwrap().ram_recovered_bytes, Some(-(GIB as i64)));
    }

    #[test]
    fn unreadable_system_state_gives_unknown_not_zero() {
        let rig = Rig::new(Facts::default());
        let rec = rig
            .exec("docker.stop_container", json!({ "container": "redis" }), false, false)
            .unwrap();
        let m = rec.measurement.unwrap();
        assert_eq!(m.ram_recovered_bytes, None);
        assert_eq!(m.disk_recovered_bytes, None);
    }

    #[test]
    fn terminate_is_verified_by_the_pid_being_gone_and_is_irreversible() {
        let procs = vec![sample(77, "node.exe", GIB, 1.0, Category::Node)];
        let rig = Rig::new(Facts {
            processes: procs,
            ..Default::default()
        });
        let rec = rig
            .exec("process.terminate", json!({ "pid": 77, "name": "node.exe" }), true, false)
            .unwrap();
        assert_eq!(rec.status, ActionStatus::Success);
        assert_eq!(rec.reversibility, Reversibility::Irreversible);
        assert!(!rec.reversible);
        assert!(rec.verification.unwrap().detail.contains("no longer running"));
    }

    #[test]
    fn a_process_that_survives_is_not_effective_not_success() {
        let procs = vec![sample(77, "node.exe", GIB, 1.0, Category::Node)];
        let rig = Rig::new(Facts {
            processes: procs,
            ..Default::default()
        });
        *rig.probe.procs.lock().unwrap() = Some(vec![ProbeProcess {
            pid: 77,
            name: "node.exe".into(),
            cmd: String::new(),
        }]);
        let rec = rig
            .exec("process.terminate", json!({ "pid": 77, "name": "node.exe" }), true, false)
            .unwrap();
        assert!(rec.success, "the call itself did not fail");
        assert_eq!(rec.status, ActionStatus::NotEffective);
        assert_eq!(rec.verification.unwrap().state, VerifyState::NotConfirmed);
    }

    #[test]
    fn unreadable_process_list_means_unverified() {
        let procs = vec![sample(77, "node.exe", GIB, 1.0, Category::Node)];
        let rig = Rig::new(Facts {
            processes: procs,
            ..Default::default()
        });
        *rig.probe.procs.lock().unwrap() = None;
        let rec = rig
            .exec("process.terminate", json!({ "pid": 77, "name": "node.exe" }), true, false)
            .unwrap();
        assert_eq!(rec.status, ActionStatus::Unverified);
    }

    #[test]
    fn wsl_shutdown_is_verified_by_vmmem_exit_and_gradle_stop_by_daemons_gone() {
        let rig = Rig::new(Facts::default());
        let rec = rig.exec("wsl.shutdown", json!({}), true, false).unwrap();
        assert_eq!(rec.status, ActionStatus::Success);
        assert_eq!(rec.reversibility, Reversibility::Irreversible);

        *rig.probe.procs.lock().unwrap() = Some(vec![ProbeProcess {
            pid: 9,
            name: "java.exe".into(),
            cmd: "java org.gradle.launcher.daemon.bootstrap.gradledaemon 8.5".into(),
        }]);
        let rec = rig.exec("gradle.stop", json!({}), false, false).unwrap();
        assert_eq!(rec.status, ActionStatus::NotEffective);
        assert!(rec.verification.unwrap().detail.contains("same Gradle version"));
        assert_eq!(rec.reversibility, Reversibility::Recoverable);
    }

#[test]    fn a_container_that_keeps_running_is_not_effective() {        let mut rig = Rig::new(Facts::default());        rig.runner.still_running = true;        let rec = rig            .exec("docker.stop_container", json!({ "container": "redis" }), false, false)            .unwrap();        assert!(rec.success);        assert_eq!(rec.status, ActionStatus::NotEffective);        assert!(rec.verification.unwrap().detail.contains("still shows as running"));    }
    #[test]
    fn a_failed_action_has_no_verification_or_measurement() {
        let mut rig = Rig::new(Facts::default());
        rig.runner.fail = true;
        let r = rig.exec("docker.stop_container", json!({ "container": "redis" }), false, false);
        assert!(r.is_err());
        let rec = &rig.engine.history()[0];
        assert_eq!(rec.status, ActionStatus::Failed);
        assert!(rec.verification.is_none() && rec.measurement.is_none());
    }

    #[test]
    fn no_irreversible_spec_claims_rollback_and_classes_are_consistent() {
        for s in ActionEngine::default().list() {
            if s.reversibility == Reversibility::Irreversible {
                assert!(!s.reversible, "{} is irreversible but offers rollback", s.id);
            }
            if s.reversibility == Reversibility::Reversible {
                assert!(s.reversible, "{} claims REVERSIBLE without a rollback", s.id);
            }
        }
    }

    #[test]
    fn build_output_is_medium_and_temp_cache_is_low_and_keeps_its_folder() {
        let t = tempfile::tempdir().unwrap();
        let next = t.path().join("proj").join(".next");
        let temp = t.path().join("Temp");
        write(&next.join("cache").join("x"), 50);
        write(&temp.join("a.tmp"), 20);
        write(&temp.join("sub").join("b.tmp"), 30);
        let rig = Rig::new(facts_with(vec![
            entry(&next, EntryKind::BuildOutput),
            entry(&temp, EntryKind::TempCache),
        ]));

        let pv = |p: &Path| {
            rig.engine
                .preview(
                    "cleanup.path",
                    &json!({ "path": p.to_string_lossy() }),
                    &rig.ctx(),
                )
                .unwrap()
        };
        assert_eq!(pv(&next).risk, RiskLevel::Medium);
        assert_eq!(pv(&temp).risk, RiskLevel::Low);

        rig.exec(
            "cleanup.path",
            json!({ "path": temp.to_string_lossy() }),
            false,
            false,
        )
        .unwrap();
        assert!(temp.exists(), "the folder itself is kept");
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0);

        assert!(
            rig.exec(
                "cleanup.path",
                json!({ "path": next.to_string_lossy() }),
                false,
                false
            )
            .is_err()
        );
        rig.exec(
            "cleanup.path",
            json!({ "path": next.to_string_lossy() }),
            true,
            false,
        )
        .unwrap();
        assert!(!next.exists());
    }

    #[test]
    fn items_that_vanished_since_the_scan_are_reported() {
        let t = tempfile::tempdir().unwrap();
        let gone = t.path().join("gone").join("node_modules");
        let rig = Rig::new(facts_with(vec![entry(&gone, EntryKind::NodeModules)]));
        let r = rig.exec(
            "cleanup.path",
            json!({ "path": gone.to_string_lossy() }),
            true,
            true,
        );
        assert!(matches!(r, Err(ActionError::Invalid(m)) if m.contains("no longer exists")));
    }

    #[test]
    fn path_matching_is_case_insensitive_but_exact() {
        let t = tempfile::tempdir().unwrap();
        let nm = t.path().join("p").join("node_modules");
        write(&nm.join("a"), 1);
        let rig = Rig::new(facts_with(vec![entry(&nm, EntryKind::NodeModules)]));
        let upper = nm.to_string_lossy().to_uppercase();
        assert!(
            rig.engine
                .preview("cleanup.path", &json!({ "path": upper }), &rig.ctx())
                .is_ok()
        );
        // The parent directory was not scanned, so it cannot be deleted through this item.
        let parent = nm.parent().unwrap().to_string_lossy().into_owned();
        assert!(
            rig.engine
                .preview("cleanup.path", &json!({ "path": parent }), &rig.ctx())
                .is_err()
        );
    }

    // --- cleanup.temp & automation -----------------------------------------------------------------

    #[test]
    fn automation_runs_only_eligible_low_risk_actions() {
        let t = tempfile::tempdir().unwrap();
        let temp = t.path().join("Temp");
        write(&temp.join("fresh.tmp"), 10);
        let mut facts = facts_with(vec![]);
        facts.temp_dir = Some(temp.clone());
        let rig = Rig::new(facts);

        let rec = rig
            .engine
            .execute_automated("cleanup.temp", &json!({}), &rig.ctx())
            .unwrap();
        assert!(rec.automated && rec.success);
        assert!(
            temp.join("fresh.tmp").exists(),
            "files younger than 24 h are kept"
        );

        for id in [
            "docker.stop_container",
            "wsl.shutdown",
            "process.terminate",
            "cleanup.path",
            "gradle.stop",
        ] {
            let r = rig.engine.execute_automated(
                id,
                &json!({ "container": "x", "pid": 1, "path": "C:\\x" }),
                &rig.ctx(),
            );
            assert!(matches!(r, Err(ActionError::Refused(_))), "{id}: {r:?}");
        }
        assert!(rig.calls().is_empty());
    }

    #[test]
    fn temp_cleanup_refuses_protected_locations_and_missing_temp() {
        let mut facts = facts_with(vec![]);
        facts.temp_dir = Some(PathBuf::from("C:\\Windows\\Temp"));
        let rig = Rig::new(facts);
        assert!(matches!(
            rig.engine
                .execute_automated("cleanup.temp", &json!({}), &rig.ctx()),
            Err(ActionError::Refused(_))
        ));
        let rig = Rig::new(facts_with(vec![]));
        assert!(matches!(
            rig.engine
                .execute_automated("cleanup.temp", &json!({}), &rig.ctx()),
            Err(ActionError::Failed(_))
        ));
    }

    #[test]
    fn every_preview_carries_the_fields_the_confirm_dialog_needs() {
        let rig = Rig::new(facts_with_processes());
        for (id, params) in [
            ("docker.stop_container", json!({ "container": "a" })),
            ("wsl.shutdown", json!({})),
            ("gradle.stop", json!({})),
            ("process.terminate", json!({ "pid": 100 })),
        ] {
            let p = rig.engine.preview(id, &params, &rig.ctx()).unwrap();
            assert!(
                !p.title.is_empty() && !p.summary.is_empty() && !p.items.is_empty(),
                "{id}"
            );
            assert_eq!(p.requires_confirmation, p.risk >= RiskLevel::Medium, "{id}");
        }
    }
}

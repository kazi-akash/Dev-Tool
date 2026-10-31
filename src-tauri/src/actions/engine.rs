//! Action registry plus the permission rules that stand between a request and its side effects.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use super::builtin;
use super::model::{
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
    pub reversible: bool,
    #[serde(skip)]
    outcome: Option<Outcome>,
}

type Sink = Box<dyn Fn(&ActionRecord) + Send + Sync>;

pub struct ActionEngine {
    actions: HashMap<&'static str, Box<dyn Action>>,
    log: Mutex<VecDeque<ActionRecord>>,
    next_id: AtomicU64,
    /// Called for every recorded action (used to persist history).
    sink: OnceLock<Sink>,
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
        }
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

    fn record(
        &self,
        spec: &ActionSpec,
        risk: RiskLevel,
        automated: bool,
        result: &Result<Outcome, ActionError>,
    ) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (success, summary, freed, outcome) = match result {
            Ok(o) => (true, o.summary.clone(), o.freed_bytes, Some(o.clone())),
            Err(e) => (false, e.to_string(), None, None),
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
            reversible: spec.reversible,
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

        let result = action.execute(params, ctx);
        let rec_id = self.record(action.spec(), preview.risk, false, &result);
        result.map(|_| self.find(rec_id).expect("record was just inserted"))
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
        let result = action.execute(params, ctx);
        let rec_id = self.record(action.spec(), preview.risk, true, &result);
        result.map(|_| self.find(rec_id).expect("record was just inserted"))
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
    use crate::actions::model::{CommandRunner, Facts, ProcessKiller};
    use crate::processes::model::{Category, sample};
    use crate::storage::safety::{EntryKind, Safety};
    use crate::storage::scanner::ScanEntry;
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<String>>,
        fail: bool,
    }
    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{program} {}", args.join(" ")));
            if self.fail {
                Err("boom".into())
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

    struct Rig {
        engine: ActionEngine,
        runner: FakeRunner,
        killer: FakeKiller,
        facts: Facts,
    }

    impl Rig {
        fn new(facts: Facts) -> Self {
            Rig {
                engine: ActionEngine::default(),
                runner: FakeRunner::default(),
                killer: FakeKiller::default(),
                facts,
            }
        }
        fn ctx(&self) -> Ctx<'_> {
            Ctx {
                facts: &self.facts,
                runner: &self.runner,
                killer: &self.killer,
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
        assert_eq!(rig.calls(), vec!["docker stop redis"]);
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
        assert_eq!(rig.calls(), vec!["docker stop redis", "docker start redis"]);
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
    fn node_modules_is_destructive_and_needs_two_confirmations() {
        let t = tempfile::tempdir().unwrap();
        let nm = t.path().join("proj").join("node_modules");
        write(&nm.join("pkg").join("index.js"), 100);
        // The entry lies about its safety (SAFE_TO_CLEAN); the engine uses the kind.
        let rig = Rig::new(facts_with(vec![entry(&nm, EntryKind::NodeModules)]));

        let p = rig
            .engine
            .preview(
                "cleanup.path",
                &json!({ "path": nm.to_string_lossy() }),
                &rig.ctx(),
            )
            .unwrap();
        assert_eq!(p.risk, RiskLevel::Destructive);
        assert!(p.requires_confirmation && p.requires_double_confirmation && !p.reversible);
        assert_eq!(p.estimated_benefit_bytes, Some(100));
        assert!(p.warnings.iter().any(|w| w.contains("exists nowhere else")));
        assert!(p.warnings.iter().any(|w| w.contains("Recycle Bin")));

        let args = json!({ "path": nm.to_string_lossy() });
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
        assert_eq!(rec.freed_bytes, Some(100));
        assert!(!nm.exists());
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

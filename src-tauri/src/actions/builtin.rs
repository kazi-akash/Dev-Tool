//! The whitelisted actions. Anything not registered here cannot run, whoever asks (UI, AI, tray).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::fs_clean::{
    clean_contents, is_reparse_point, plan_clean_contents, remove_path, tree_size,
};
use super::model::{
    Action, ActionError, ActionSpec, Ctx, Outcome, Preview, PreviewItem, Reversibility, RiskLevel,
    Verification,
};
use crate::docker::is_valid_container_ref;
use crate::processes::model::Category;
use crate::storage::paths::{has_traversal, is_drive_absolute, is_protected, normalize};
use crate::storage::safety::{EntryKind, Safety, classify};
use crate::util::fmt_bytes;

fn make_preview(
    spec: &ActionSpec,
    title: String,
    summary: String,
    risk: RiskLevel,
    items: Vec<PreviewItem>,
    benefit: Option<u64>,
    warnings: Vec<String>,
) -> Preview {
    Preview {
        action_id: spec.id.into(),
        title,
        summary,
        risk,
        items,
        estimated_benefit_bytes: benefit,
        warnings,
        requires_confirmation: risk.requires_confirmation(),
        requires_double_confirmation: risk.requires_double_confirmation(),
        reversible: spec.reversible,
        reversibility: spec.reversibility,
    }
}

fn str_param<'a>(p: &'a Value, key: &str) -> Result<&'a str, ActionError> {
    p.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ActionError::Invalid(format!("missing text parameter '{key}'")))
}

fn outcome(
    spec: &ActionSpec,
    summary: String,
    freed: Option<u64>,
    details: Vec<String>,
    rollback: Option<Value>,
) -> Outcome {
    Outcome {
        action_id: spec.id.into(),
        summary,
        freed_bytes: freed,
        details,
        rollback,
        reversibility: spec.reversibility,
        context: None,
    }
}

// --- docker.stop_container ---------------------------------------------------------------------

pub struct StopContainer;
const STOP_CONTAINER: ActionSpec = ActionSpec {
    id: "docker.stop_container",
    name: "Stop Docker container",
    description: "Stops one running container (docker stop). Its data volumes are kept; it can be started again.",
    risk_level: RiskLevel::Low,
    estimated_benefit: "Frees the container's memory",
    automation_eligible: false,
    reversible: true,
    reversibility: Reversibility::Reversible,
};

impl Action for StopContainer {
    fn spec(&self) -> &'static ActionSpec {
        &STOP_CONTAINER
    }

    fn preview(&self, params: &Value, _ctx: &Ctx) -> Result<Preview, ActionError> {
        let name = str_param(params, "container")?;
        if !is_valid_container_ref(name) {
            return Err(ActionError::Invalid(
                "not a valid container name or id".into(),
            ));
        }
        Ok(make_preview(
            &STOP_CONTAINER,
            format!("Stop container {name}"),
            "Runs `docker stop` for this one container. Unsaved data inside the container's memory is lost.".into(),
            RiskLevel::Low,
            vec![PreviewItem { label: name.into(), detail: Some("docker stop".into()), bytes: None }],
            None,
            vec![],
        ))
    }

    fn execute(&self, params: &Value, ctx: &Ctx) -> Result<Outcome, ActionError> {
        let name = str_param(params, "container")?;
        if !is_valid_container_ref(name) {
            return Err(ActionError::Invalid(
                "not a valid container name or id".into(),
            ));
        }
        ctx.runner
            .run("docker", &["stop", name])
            .map_err(ActionError::Failed)?;
        Ok(outcome(
            &STOP_CONTAINER,
            format!("Stopped container {name}."),
            None,
            vec![],
            Some(json!({ "container": name })),
        ))
    }

    fn verify(&self, params: &Value, _o: &Outcome, ctx: &Ctx) -> Verification {
        let Ok(name) = str_param(params, "container") else {
            return Verification::unverifiable("No container name to check.");
        };
        let filter = format!("name=^{name}$");
        match ctx
            .runner
            .run("docker", &["ps", "--filter", &filter, "--format", "{{.Names}}"])
        {
            Ok(out) if out.trim().is_empty() => {
                Verification::verified(format!("{name} is no longer in the running container list."))
            }
            Ok(_) => Verification::not_confirmed(format!("{name} still shows as running.")),
            Err(e) => Verification::unverifiable(format!("Docker could not be queried: {e}")),
        }
    }

    fn measure_note(&self) -> Option<&'static str> {
        Some("Docker Desktop's WSL2 VM often keeps memory after a container stops, so RAM may not drop until the VM shrinks or WSL is shut down.")
    }

    fn rollback(&self, o: &Outcome, ctx: &Ctx) -> Option<Result<String, ActionError>> {
        let name = o.rollback.as_ref()?.get("container")?.as_str()?;
        if !is_valid_container_ref(name) {
            return Some(Err(ActionError::Invalid(
                "not a valid container name or id".into(),
            )));
        }
        Some(
            ctx.runner
                .run("docker", &["start", name])
                .map(|_| format!("Started container {name} again."))
                .map_err(ActionError::Failed),
        )
    }
}

// --- wsl.shutdown ------------------------------------------------------------------------------

pub struct WslShutdown;
const WSL_SHUTDOWN: ActionSpec = ActionSpec {
    id: "wsl.shutdown",
    name: "Shut down WSL",
    description: "Runs `wsl --shutdown`, stopping the WSL2 VM and everything in it, and returning its memory to Windows.",
    risk_level: RiskLevel::Medium,
    estimated_benefit: "Returns the WSL2 VM's memory",
    automation_eligible: false,
    reversible: false,
    reversibility: Reversibility::Irreversible,
};

impl Action for WslShutdown {
    fn spec(&self) -> &'static ActionSpec {
        &WSL_SHUTDOWN
    }

    fn preview(&self, _params: &Value, _ctx: &Ctx) -> Result<Preview, ActionError> {
        Ok(make_preview(
            &WSL_SHUTDOWN,
            "Shut down WSL".into(),
            "Stops every WSL distribution and the Docker Desktop engine. WSL starts again automatically on next use.".into(),
            RiskLevel::Medium,
            vec![PreviewItem { label: "wsl --shutdown".into(), detail: None, bytes: None }],
            None,
            vec!["Running terminals, dev servers and containers inside WSL will stop and unsaved work in them is lost.".into()],
        ))
    }

    fn verify(&self, _p: &Value, _o: &Outcome, ctx: &Ctx) -> Verification {
        match ctx.probe.processes() {
            None => Verification::unverifiable("The process list could not be read."),
            Some(ps) => {
                let vm = ps
                    .iter()
                    .filter(|p| p.name.to_lowercase().starts_with("vmmem"))
                    .count();
                if vm == 0 {
                    Verification::verified("The WSL2 VM process (vmmem) is gone.")
                } else {
                    Verification::not_confirmed("The WSL2 VM process (vmmem) is still running.")
                }
            }
        }
    }

    fn execute(&self, _params: &Value, ctx: &Ctx) -> Result<Outcome, ActionError> {
        ctx.runner
            .run("wsl", &["--shutdown"])
            .map_err(ActionError::Failed)?;
        Ok(outcome(
            &WSL_SHUTDOWN,
            "WSL was shut down.".into(),
            None,
            vec![],
            None,
        ))
    }
}

// --- gradle.stop -------------------------------------------------------------------------------

pub struct GradleStop;
const GRADLE_STOP: ActionSpec = ActionSpec {
    id: "gradle.stop",
    name: "Stop Gradle daemons",
    description: "Runs `gradle --stop`, which asks idle Gradle daemons to exit. The next build starts a new one.",
    risk_level: RiskLevel::Low,
    estimated_benefit: "Frees the Gradle daemons' JVM memory",
    automation_eligible: false,
    reversible: false,
    reversibility: Reversibility::Recoverable,
};

impl Action for GradleStop {
    fn spec(&self) -> &'static ActionSpec {
        &GRADLE_STOP
    }

    fn preview(&self, _params: &Value, _ctx: &Ctx) -> Result<Preview, ActionError> {
        Ok(make_preview(
            &GRADLE_STOP,
            "Stop Gradle daemons".into(),
            "Asks Gradle to stop its daemons. The next build is a little slower while a daemon restarts.".into(),
            RiskLevel::Low,
            vec![PreviewItem { label: "gradle --stop".into(), detail: None, bytes: None }],
            None,
            vec!["A build that is running right now will be interrupted.".into()],
        ))
    }

    fn verify(&self, _p: &Value, _o: &Outcome, ctx: &Ctx) -> Verification {
        match ctx.probe.processes() {
            None => Verification::unverifiable("The process list could not be read."),
            Some(ps) => {
                let n = ps.iter().filter(|p| p.cmd.contains("gradledaemon")).count();
                if n == 0 {
                    Verification::verified("No Gradle daemon is running any more.")
                } else {
                    Verification::not_confirmed(format!(
                        "{n} Gradle daemon(s) still run. `gradle --stop` only stops daemons of the same Gradle version; busy daemons are not stopped."
                    ))
                }
            }
        }
    }

    fn execute(&self, _params: &Value, ctx: &Ctx) -> Result<Outcome, ActionError> {
        // `gradle` is usually a .bat on Windows, which needs the command interpreter. The arguments are fixed.
        let out = ctx
            .runner
            .run("cmd", &["/C", "gradle", "--stop"])
            .map_err(|e| {
                ActionError::Failed(format!(
                    "{e}. If Gradle is not on PATH, run `./gradlew --stop` in your project."
                ))
            })?;
        let details: Vec<String> = out.lines().take(5).map(str::to_string).collect();
        Ok(outcome(
            &GRADLE_STOP,
            "Gradle daemons were asked to stop.".into(),
            None,
            details,
            None,
        ))
    }
}

// --- process.terminate -------------------------------------------------------------------------

pub struct TerminateProcess;
const TERMINATE_PROCESS: ActionSpec = ActionSpec {
    id: "process.terminate",
    name: "Close a process",
    description: "Ends one specific process chosen from the live process list. Never offered for system processes.",
    risk_level: RiskLevel::Medium,
    estimated_benefit: "Frees that process's memory",
    automation_eligible: false,
    reversible: false,
    reversibility: Reversibility::Irreversible,
};

fn find_target<'a>(
    params: &Value,
    ctx: &'a Ctx,
) -> Result<&'a crate::processes::model::ProcessInfo, ActionError> {
    let pid = params
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok())
        .ok_or_else(|| ActionError::Invalid("missing numeric parameter 'pid'".into()))?;
    if pid == std::process::id() {
        return Err(ActionError::Refused(
            "DevPilot will not close itself.".into(),
        ));
    }
    let p = ctx
        .facts
        .processes
        .iter()
        .find(|p| p.pid == pid)
        .ok_or_else(|| ActionError::Invalid("that process no longer exists".into()))?;
    // Guards against PID reuse: the name seen when the recommendation was made must still match.
    if let Some(expected) = params.get("name").and_then(Value::as_str)
        && !expected.eq_ignore_ascii_case(&p.name)
    {
        return Err(ActionError::Refused(format!(
            "PID {pid} is now {}, not {expected}. Refresh and try again.",
            p.name
        )));
    }
    if p.category == Category::System {
        return Err(ActionError::Refused(
            "System processes are never closed by DevPilot.".into(),
        ));
    }
    Ok(p)
}

impl Action for TerminateProcess {
    fn spec(&self) -> &'static ActionSpec {
        &TERMINATE_PROCESS
    }

    fn preview(&self, params: &Value, ctx: &Ctx) -> Result<Preview, ActionError> {
        let p = find_target(params, ctx)?;
        Ok(make_preview(
            &TERMINATE_PROCESS,
            format!("Close {} (PID {})", p.name, p.pid),
            format!(
                "Ends this process immediately and frees about {}.",
                fmt_bytes(p.memory_bytes)
            ),
            RiskLevel::Medium,
            vec![PreviewItem {
                label: p.name.clone(),
                detail: p.exe_path.clone(),
                bytes: Some(p.memory_bytes),
            }],
            Some(p.memory_bytes),
            vec![
                "The program is ended without a chance to save. Unsaved work in it is lost.".into(),
            ],
        ))
    }

    fn verify(&self, params: &Value, _o: &Outcome, ctx: &Ctx) -> Verification {
        let Some(pid) = params.get("pid").and_then(Value::as_u64) else {
            return Verification::unverifiable("No process id to check.");
        };
        match ctx.probe.processes() {
            None => Verification::unverifiable("The process list could not be read."),
            Some(ps) if ps.iter().any(|p| u64::from(p.pid) == pid) => {
                Verification::not_confirmed(format!("PID {pid} is still running."))
            }
            Some(_) => Verification::verified(format!("PID {pid} is no longer running.")),
        }
    }

    fn execute(&self, params: &Value, ctx: &Ctx) -> Result<Outcome, ActionError> {
        let p = find_target(params, ctx)?;
        ctx.killer.kill(p.pid).map_err(ActionError::Failed)?;
        Ok(outcome(
            &TERMINATE_PROCESS,
            format!("Closed {} (PID {}).", p.name, p.pid),
            None,
            vec![],
            None,
        ))
    }
}

// --- cleanup.path ------------------------------------------------------------------------------

pub struct CleanupPath;
const CLEANUP_PATH: ActionSpec = ActionSpec {
    id: "cleanup.path",
    name: "Delete a scanned item",
    description: "Removes one item found by the storage scanner: moved to the Recycle Bin when it fits, otherwise deleted permanently (the preview says which). Refused for protected and do-not-touch locations.",
    risk_level: RiskLevel::Destructive,
    estimated_benefit: "Frees the item's disk space",
    automation_eligible: false,
    reversible: true,
    reversibility: Reversibility::Recoverable,
};

struct CleanupTarget {
    path: PathBuf,
    kind: EntryKind,
    size: u64,
    risk: RiskLevel,
    safety: Safety,
    reason: &'static str,
}

/// Every check that decides whether a path may be deleted. Used by both preview and execute.
fn validate_cleanup(params: &Value, ctx: &Ctx) -> Result<CleanupTarget, ActionError> {
    let raw = str_param(params, "path")?;
    let path = PathBuf::from(raw);

    if has_traversal(&path) {
        return Err(ActionError::Refused(
            "Paths containing '..' are not accepted.".into(),
        ));
    }
    if !is_drive_absolute(&path) {
        return Err(ActionError::Refused(
            "Only absolute drive paths are accepted.".into(),
        ));
    }
    if is_protected(&path, ctx.facts.user_profile.as_deref()) {
        return Err(ActionError::Refused(
            "This location is protected (system or personal folder).".into(),
        ));
    }
    // Whitelist: only what the last scan actually found.
    let wanted = normalize(&path);
    let entry = ctx
        .facts
        .scan_entries
        .iter()
        .find(|e| normalize(Path::new(&e.path)) == wanted)
        .ok_or_else(|| {
            ActionError::Refused(
                "This item was not found by the latest storage scan. Run a scan first.".into(),
            )
        })?;

    // Safety is recomputed from the kind, not read from the (client-visible) entry.
    let class = classify(entry.kind);
    let risk = match class.safety {
        Safety::SafeToClean => RiskLevel::Low,
        Safety::SafeToReview => RiskLevel::Medium,
        Safety::RequiresReview => RiskLevel::Destructive,
        Safety::DoNotTouch => {
            return Err(ActionError::Refused(format!(
                "This is marked do-not-touch. {}",
                class.reason
            )));
        }
    };
    let md = std::fs::symlink_metadata(&path)
        .map_err(|_| ActionError::Invalid("The item no longer exists. Run a new scan.".into()))?;
    if is_reparse_point(&path) {
        return Err(ActionError::Refused(
            "Links and junctions are never deleted.".into(),
        ));
    }
    let size = if md.is_dir() {
        tree_size(&path)
    } else {
        md.len()
    };
    Ok(CleanupTarget {
        path,
        kind: entry.kind,
        size,
        risk,
        safety: class.safety,
        reason: class.reason,
    })
}

/// Caches keep their folder (it may be in use) and lose only the contents.
fn keeps_folder(kind: EntryKind) -> bool {
    matches!(kind, EntryKind::TempCache | EntryKind::PackageCache)
}

/// Items larger than this are not sent to the Recycle Bin by default: Windows may refuse or
/// permanently delete items that do not fit, so the preview says "permanent" instead of promising
/// a recovery it cannot guarantee.
const RECYCLE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// How a cleanup will remove the item. Decided in one place so preview and execute agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeleteMode {
    /// Move the item to the Recycle Bin.
    Recycle,
    /// Delete the item permanently.
    Permanent { too_large: bool },
    /// Cache folders keep their folder and lose their contents permanently.
    Contents,
}

fn delete_mode(t: &CleanupTarget, params: &Value) -> DeleteMode {
    if keeps_folder(t.kind) {
        DeleteMode::Contents
    } else if params
        .get("permanent")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        DeleteMode::Permanent { too_large: false }
    } else if t.size > RECYCLE_MAX_BYTES {
        DeleteMode::Permanent { too_large: true }
    } else {
        DeleteMode::Recycle
    }
}

fn mode_reversibility(mode: DeleteMode) -> Reversibility {
    match mode {
        DeleteMode::Recycle => Reversibility::Recoverable,
        _ => Reversibility::Irreversible,
    }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl Action for CleanupPath {
    fn spec(&self) -> &'static ActionSpec {
        &CLEANUP_PATH
    }

    fn preview(&self, params: &Value, ctx: &Ctx) -> Result<Preview, ActionError> {
        let t = validate_cleanup(params, ctx)?;
        let mode = delete_mode(&t, params);
        let mut warnings = match mode {
            DeleteMode::Recycle => vec![
                "Moved to the Recycle Bin, not deleted. Disk space is released only when the Recycle Bin is emptied.".to_string(),
            ],
            DeleteMode::Permanent { too_large: true } => vec![format!(
                "Larger than {}, so it is deleted permanently: it may not fit in the Recycle Bin.",
                fmt_bytes(RECYCLE_MAX_BYTES)
            )],
            DeleteMode::Permanent { too_large: false } => {
                vec!["Deleted permanently. It is not moved to the Recycle Bin.".to_string()]
            }
            DeleteMode::Contents => vec![
                "Contents are deleted permanently (not sent to the Recycle Bin). Tools recreate caches when they need them.".to_string(),
            ],
        };
        if t.safety == Safety::RequiresReview {
            warnings.push(
                "This may contain work that exists nowhere else. Check it before you confirm."
                    .into(),
            );
        }
        if mode == DeleteMode::Contents {
            warnings.push("The folder itself is kept; files that are in use are skipped.".into());
        }
        let verb = if mode == DeleteMode::Recycle {
            "Move to Recycle Bin:"
        } else {
            "Delete"
        };
        let mut p = make_preview(
            &CLEANUP_PATH,
            format!("{verb} {}", file_label(&t.path)),
            format!("{} {}", fmt_bytes(t.size), t.reason),
            t.risk,
            vec![PreviewItem {
                label: t.path.to_string_lossy().into_owned(),
                detail: Some(format!("{:?}", t.kind)),
                bytes: Some(t.size),
            }],
            // Nothing is freed until the bin is emptied, so no immediate benefit is promised.
            (mode != DeleteMode::Recycle).then_some(t.size),
            warnings,
        );
        p.reversible = mode == DeleteMode::Recycle;
        p.reversibility = mode_reversibility(mode);
        Ok(p)
    }

    fn execute(&self, params: &Value, ctx: &Ctx) -> Result<Outcome, ActionError> {
        let t = validate_cleanup(params, ctx)?;
        let mode = delete_mode(&t, params);
        let context = json!({ "mode": format!("{mode:?}"), "sizeBefore": t.size });
        match mode {
            DeleteMode::Contents => {
                let r = clean_contents(&t.path, None, SystemTime::now());
                let mut details = vec![format!("{} items removed", r.removed)];
                if r.failed > 0 {
                    details.push(format!(
                        "{} items skipped (in use or protected by Windows)",
                        r.failed
                    ));
                }
                let mut o = outcome(
                    &CLEANUP_PATH,
                    format!("Freed {}.", fmt_bytes(r.freed_bytes)),
                    Some(r.freed_bytes),
                    details,
                    None,
                );
                o.reversibility = Reversibility::Irreversible;
                o.context =
                    Some(json!({ "mode": "Contents", "sizeBefore": t.size, "failed": r.failed }));
                Ok(o)
            }
            DeleteMode::Recycle => {
                let receipt = ctx.trasher.trash(&t.path).map_err(|e| {
                    ActionError::Failed(format!("Could not move to the Recycle Bin: {e}"))
                })?;
                if receipt.in_bin {
                    let mut o = outcome(
                        &CLEANUP_PATH,
                        format!(
                            "Moved {} ({}) to the Recycle Bin. The space is released when the bin is emptied.",
                            file_label(&t.path),
                            fmt_bytes(t.size)
                        ),
                        None,
                        vec![],
                        Some(json!({ "restore": t.path.to_string_lossy() })),
                    );
                    o.reversibility = Reversibility::Recoverable;
                    o.context = Some(context);
                    Ok(o)
                } else {
                    // Windows did not keep a copy: say so instead of promising a recovery.
                    let mut o = outcome(
                        &CLEANUP_PATH,
                        format!(
                            "Removed {} ({}). Windows did not keep a Recycle Bin copy, so this is permanent.",
                            file_label(&t.path),
                            fmt_bytes(t.size)
                        ),
                        Some(t.size),
                        vec![],
                        None,
                    );
                    o.reversibility = Reversibility::Irreversible;
                    o.context = Some(context);
                    Ok(o)
                }
            }
            DeleteMode::Permanent { .. } => {
                let freed = remove_path(&t.path)
                    .map_err(|e| ActionError::Failed(format!("Could not delete: {e}")))?;
                let mut o = outcome(
                    &CLEANUP_PATH,
                    format!("Deleted and freed {}.", fmt_bytes(freed)),
                    Some(freed),
                    vec![],
                    None,
                );
                o.reversibility = Reversibility::Irreversible;
                o.context = Some(context);
                Ok(o)
            }
        }
    }

    fn verify(&self, params: &Value, o: &Outcome, _ctx: &Ctx) -> Verification {
        let Ok(raw) = str_param(params, "path") else {
            return Verification::unverifiable("No path to check.");
        };
        let path = Path::new(raw);
        let ctx_val = |k: &str| o.context.as_ref().and_then(|c| c.get(k));
        if ctx_val("mode").and_then(Value::as_str) == Some("Contents") {
            let before = ctx_val("sizeBefore").and_then(Value::as_u64).unwrap_or(0);
            let now = tree_size(path);
            return if now < before || before == 0 {
                Verification::verified(format!(
                    "The folder now holds {} (was {}).",
                    fmt_bytes(now),
                    fmt_bytes(before)
                ))
            } else {
                Verification::not_confirmed(
                    "The folder did not shrink (its files may all be in use).",
                )
            };
        }
        if path.exists() {
            Verification::not_confirmed("The item is still at its original location.")
        } else if o.reversibility == Reversibility::Recoverable {
            Verification::verified(
                "The item is gone from its location and was found in the Recycle Bin.",
            )
        } else {
            Verification::verified("The item is gone from its location.")
        }
    }

    fn measure_path(&self, params: &Value, _ctx: &Ctx) -> Option<PathBuf> {
        str_param(params, "path").ok().map(PathBuf::from)
    }

    fn measure_note(&self) -> Option<&'static str> {
        Some("Items moved to the Recycle Bin free no disk space until the bin is emptied.")
    }

    fn rollback(&self, o: &Outcome, ctx: &Ctx) -> Option<Result<String, ActionError>> {
        let original = Path::new(o.rollback.as_ref()?.get("restore")?.as_str()?);
        // Never restore into a place a cleanup could not have touched.
        if has_traversal(original) || !is_drive_absolute(original) {
            return Some(Err(ActionError::Refused(
                "Not a valid restore location.".into(),
            )));
        }
        if original.exists() {
            return Some(Err(ActionError::Failed(
                "Something already exists at the original location; restore was not attempted."
                    .into(),
            )));
        }
        Some(
            ctx.trasher
                .restore(original)
                .map(|_| format!("Restored {} from the Recycle Bin.", original.display()))
                .map_err(ActionError::Failed),
        )
    }
}

// --- cleanup.temp (safe automation) ------------------------------------------------------------

pub struct CleanupTemp;
const CLEANUP_TEMP: ActionSpec = ActionSpec {
    id: "cleanup.temp",
    name: "Clean old temporary files",
    description: "Deletes files and folders in your user Temp folder that have not changed for 24 hours.",
    risk_level: RiskLevel::Low,
    estimated_benefit: "Frees old temporary files",
    automation_eligible: true,
    reversible: false,
    reversibility: Reversibility::Irreversible,
};

const TEMP_MIN_AGE: Duration = Duration::from_secs(24 * 3600);

fn temp_dir<'a>(ctx: &Ctx<'a>) -> Result<&'a Path, ActionError> {
    let t =
        ctx.facts.temp_dir.as_deref().ok_or_else(|| {
            ActionError::Failed("The Temp folder could not be determined.".into())
        })?;
    if is_protected(t, ctx.facts.user_profile.as_deref()) || normalize(t).contains("\\windows\\") {
        return Err(ActionError::Refused(
            "The Temp folder is in a protected location.".into(),
        ));
    }
    Ok(t)
}

impl Action for CleanupTemp {
    fn spec(&self) -> &'static ActionSpec {
        &CLEANUP_TEMP
    }

    fn preview(&self, _params: &Value, ctx: &Ctx) -> Result<Preview, ActionError> {
        let t = temp_dir(ctx)?;
        let plan = plan_clean_contents(t, Some(TEMP_MIN_AGE), SystemTime::now());
        Ok(make_preview(
            &CLEANUP_TEMP,
            "Clean old temporary files".into(),
            format!(
                "{} items ({}) in {} are older than 24 hours.",
                plan.removed,
                fmt_bytes(plan.freed_bytes),
                t.display()
            ),
            RiskLevel::Low,
            vec![PreviewItem {
                label: t.to_string_lossy().into_owned(),
                detail: Some("items older than 24 h".into()),
                bytes: Some(plan.freed_bytes),
            }],
            Some(plan.freed_bytes),
            vec!["Files in use are skipped.".into()],
        ))
    }

    fn verify(&self, _params: &Value, o: &Outcome, ctx: &Ctx) -> Verification {
        let Ok(t) = temp_dir(ctx) else {
            return Verification::unverifiable("The Temp folder could not be checked.");
        };
        let failed = o
            .context
            .as_ref()
            .and_then(|c| c.get("failed"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        // Anything still eligible beyond the items Windows refused to delete means work was missed.
        let left = plan_clean_contents(t, Some(TEMP_MIN_AGE), SystemTime::now()).removed;
        if left <= failed {
            Verification::verified(format!(
                "No old temporary items remain except {left} that are in use."
            ))
        } else {
            Verification::not_confirmed(format!(
                "{left} old items are still there ({failed} were reported in use)."
            ))
        }
    }

    fn measure_path(&self, _params: &Value, ctx: &Ctx) -> Option<PathBuf> {
        ctx.facts.temp_dir.clone()
    }

    fn execute(&self, _params: &Value, ctx: &Ctx) -> Result<Outcome, ActionError> {
        let t = temp_dir(ctx)?;
        let r = clean_contents(t, Some(TEMP_MIN_AGE), SystemTime::now());
        let mut details = vec![format!("{} items removed", r.removed)];
        if r.failed > 0 {
            details.push(format!("{} items skipped (in use)", r.failed));
        }
        let mut o = outcome(
            &CLEANUP_TEMP,
            format!("Freed {}.", fmt_bytes(r.freed_bytes)),
            Some(r.freed_bytes),
            details,
            None,
        );
        o.context = Some(json!({ "failed": r.failed }));
        Ok(o)
    }
}

pub fn all() -> Vec<Box<dyn Action>> {
    vec![
        Box::new(StopContainer),
        Box::new(WslShutdown),
        Box::new(GradleStop),
        Box::new(TerminateProcess),
        Box::new(CleanupPath),
        Box::new(CleanupTemp),
    ]
}

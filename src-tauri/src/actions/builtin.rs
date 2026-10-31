//! The whitelisted actions. Anything not registered here cannot run, whoever asks (UI, AI, tray).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::fs_clean::{
    clean_contents, is_reparse_point, plan_clean_contents, remove_path, tree_size,
};
use super::model::{
    Action, ActionError, ActionSpec, Ctx, Outcome, Preview, PreviewItem, RiskLevel,
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
    description: "Permanently deletes one item found by the storage scanner. Refused for protected and do-not-touch locations.",
    risk_level: RiskLevel::Destructive,
    estimated_benefit: "Frees the item's disk space",
    automation_eligible: false,
    reversible: false,
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

impl Action for CleanupPath {
    fn spec(&self) -> &'static ActionSpec {
        &CLEANUP_PATH
    }

    fn preview(&self, params: &Value, ctx: &Ctx) -> Result<Preview, ActionError> {
        let t = validate_cleanup(params, ctx)?;
        let mut warnings =
            vec!["Deleted permanently. It is not moved to the Recycle Bin.".to_string()];
        if t.safety == Safety::RequiresReview {
            warnings.push(
                "This may contain work that exists nowhere else. Check it before you confirm."
                    .into(),
            );
        }
        if keeps_folder(t.kind) {
            warnings.push("The folder itself is kept; files that are in use are skipped.".into());
        }
        Ok(make_preview(
            &CLEANUP_PATH,
            format!(
                "Delete {}",
                t.path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            ),
            format!("{} {}", fmt_bytes(t.size), t.reason),
            t.risk,
            vec![PreviewItem {
                label: t.path.to_string_lossy().into_owned(),
                detail: Some(format!("{:?}", t.kind)),
                bytes: Some(t.size),
            }],
            Some(t.size),
            warnings,
        ))
    }

    fn execute(&self, params: &Value, ctx: &Ctx) -> Result<Outcome, ActionError> {
        let t = validate_cleanup(params, ctx)?;
        if keeps_folder(t.kind) {
            let r = clean_contents(&t.path, None, SystemTime::now());
            let mut details = vec![format!("{} items removed", r.removed)];
            if r.failed > 0 {
                details.push(format!(
                    "{} items skipped (in use or protected by Windows)",
                    r.failed
                ));
            }
            return Ok(outcome(
                &CLEANUP_PATH,
                format!("Freed {}.", fmt_bytes(r.freed_bytes)),
                Some(r.freed_bytes),
                details,
                None,
            ));
        }
        let freed = remove_path(&t.path)
            .map_err(|e| ActionError::Failed(format!("Could not delete: {e}")))?;
        Ok(outcome(
            &CLEANUP_PATH,
            format!("Deleted and freed {}.", fmt_bytes(freed)),
            Some(freed),
            vec![],
            None,
        ))
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

    fn execute(&self, _params: &Value, ctx: &Ctx) -> Result<Outcome, ActionError> {
        let t = temp_dir(ctx)?;
        let r = clean_contents(t, Some(TEMP_MIN_AGE), SystemTime::now());
        let mut details = vec![format!("{} items removed", r.removed)];
        if r.failed > 0 {
            details.push(format!("{} items skipped (in use)", r.failed));
        }
        Ok(outcome(
            &CLEANUP_TEMP,
            format!("Freed {}.", fmt_bytes(r.freed_bytes)),
            Some(r.freed_bytes),
            details,
            None,
        ))
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

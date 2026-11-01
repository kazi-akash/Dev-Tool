//! Action engine types. Every corrective action is described by an `ActionSpec` and implements
//! `Action` (preview, execute, optional rollback). Nothing runs without going through the engine's
//! permission checks.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::processes::model::ProcessInfo;
use crate::storage::scanner::ScanEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskLevel {
    /// Read-only or purely informational. No built-in action is this harmless yet.
    #[allow(dead_code)]
    Safe,
    Low,
    Medium,
    /// Reserved for actions that change system configuration. None exist by design.
    #[allow(dead_code)]
    High,
    Destructive,
}

impl RiskLevel {
    /// MEDIUM and above need an explicit confirmation.
    pub fn requires_confirmation(self) -> bool {
        self >= RiskLevel::Medium
    }

    /// DESTRUCTIVE needs a second, separate confirmation.
    pub fn requires_double_confirmation(self) -> bool {
        self == RiskLevel::Destructive
    }
}


/// How far an action can be taken back. Never claim more than was verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Reversibility {
    /// DevPilot can undo it with one click (rollback).
    Reversible,
    /// No rollback, but the user can get it back with standard means: Recycle Bin restore, or the
    /// service simply restarts on next use.
    Recoverable,
    /// Permanent. Data or state that was lost is gone.
    Irreversible,
}

/// Whether the effect of an action could be confirmed afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VerifyState {
    /// The expected end state was observed.
    Verified,
    /// The check ran and the expected end state was not there.
    NotConfirmed,
    /// No reliable check exists for this action.
    Unverifiable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    pub state: VerifyState,
    pub detail: String,
}

impl Verification {
    pub fn verified(detail: impl Into<String>) -> Self {
        Self {
            state: VerifyState::Verified,
            detail: detail.into(),
        }
    }
    pub fn not_confirmed(detail: impl Into<String>) -> Self {
        Self {
            state: VerifyState::NotConfirmed,
            detail: detail.into(),
        }
    }
    pub fn unverifiable(detail: impl Into<String>) -> Self {
        Self {
            state: VerifyState::Unverifiable,
            detail: detail.into(),
        }
    }
}

/// Final classification of one run, shown to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActionStatus {
    /// Ran and the expected end state was verified.
    Success,
    /// Ran, but no check could confirm the effect.
    Unverified,
    /// Ran without error, but the check found the expected end state missing.
    NotEffective,
    Failed,
}

/// A point-in-time reading of the machine used to measure what an action changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemReading {
    pub ram_used_bytes: Option<u64>,
    pub ram_available_bytes: Option<u64>,
    /// Free space of the volume holding the measured path, when the action has one.
    pub disk_free_bytes: Option<u64>,
}

/// Before/after readings. Differences are system-wide: other programs change memory too, so the
/// numbers are reported as measured, never as a guaranteed result of the action alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Measurement {
    pub before: SystemReading,
    pub after: SystemReading,
    /// `available_after - available_before`; negative when memory use grew.
    pub ram_recovered_bytes: Option<i64>,
    /// `disk_free_after - disk_free_before`.
    pub disk_recovered_bytes: Option<i64>,
    pub note: String,
}

impl Measurement {
    pub fn new(before: SystemReading, after: SystemReading, action_note: Option<&str>) -> Self {
        let diff = |b: Option<u64>, a: Option<u64>| Some(a? as i64 - b? as i64);
        let mut note = String::from(
            "Measured system-wide before and after; other programs also change memory and disk use, so this is not guaranteed to come from this action alone.",
        );
        if let Some(n) = action_note {
            note.push(' ');
            note.push_str(n);
        }
        Self {
            ram_recovered_bytes: diff(before.ram_available_bytes, after.ram_available_bytes),
            disk_recovered_bytes: diff(before.disk_free_bytes, after.disk_free_bytes),
            before,
            after,
            note,
        }
    }
}

/// A process as seen by the verification step (fresh from the OS, not the 2-second-old cache).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeProcess {
    pub pid: u32,
    pub name: String,
    /// Lower-cased command line, when readable.
    pub cmd: String,
}

/// Fresh facts for verification and measurement. Abstracted so tests are deterministic.
pub trait Probe: Send + Sync {
    fn reading(&self, measure_path: Option<&Path>) -> SystemReading;
    fn processes(&self) -> Option<Vec<ProbeProcess>>;
}

/// Result of moving something to the Recycle Bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrashReceipt {
    /// The item was found in the Recycle Bin afterwards. `false` means Windows may have deleted it
    /// permanently (bin disabled for the volume or item too large).
    pub in_bin: bool,
}

/// Recycle Bin access. Abstracted so tests never touch the real bin.
pub trait Trasher: Send + Sync {
    fn trash(&self, path: &Path) -> Result<TrashReceipt, String>;
    /// Put back the item that was at `original`.
    fn restore(&self, original: &Path) -> Result<(), String>;
}

/// A probe that knows nothing (verification and measurement degrade to "unavailable").
#[cfg(test)]
pub struct NullProbe;
#[cfg(test)]
impl Probe for NullProbe {
    fn reading(&self, _: Option<&Path>) -> SystemReading {
        SystemReading::default()
    }
    fn processes(&self) -> Option<Vec<ProbeProcess>> {
        None
    }
}

/// A Recycle Bin that refuses everything.
#[cfg(test)]
pub struct NullTrasher;
#[cfg(test)]
impl Trasher for NullTrasher {
    fn trash(&self, _: &Path) -> Result<TrashReceipt, String> {
        Err("the Recycle Bin is not available".into())
    }
    fn restore(&self, _: &Path) -> Result<(), String> {
        Err("the Recycle Bin is not available".into())
    }
}
/// Static description of an action.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// Highest risk this action can have; the actual risk of one invocation comes from its preview.
    pub risk_level: RiskLevel,
    pub estimated_benefit: &'static str,
    /// May run without a click when the user enabled safe automation (risk must also be LOW or less).
    pub automation_eligible: bool,
    /// Has a meaningful `rollback`.
    pub reversible: bool,
    /// Best-case class of this action. One invocation may be worse (see `Preview::reversibility`).
    pub reversibility: Reversibility,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    pub label: String,
    pub detail: Option<String>,
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub action_id: String,
    pub title: String,
    pub summary: String,
    /// Risk of *this* invocation.
    pub risk: RiskLevel,
    pub items: Vec<PreviewItem>,
    pub estimated_benefit_bytes: Option<u64>,
    pub warnings: Vec<String>,
    pub requires_confirmation: bool,
    pub requires_double_confirmation: bool,
    pub reversible: bool,
    /// Class of *this* invocation: what happens if the user changes their mind.
    pub reversibility: Reversibility,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub action_id: String,
    pub summary: String,
    pub freed_bytes: Option<u64>,
    pub details: Vec<String>,
    /// Opaque data the action needs to undo itself.
    pub rollback: Option<Value>,
    /// What actually happened, which can be worse than the preview promised (for example when the
    /// Recycle Bin kept no copy). The record and the UI use this, not the spec.
    pub reversibility: Reversibility,
    /// Facts the verify step needs (not shown to the user).
    #[serde(skip)]
    pub context: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "code", content = "message", rename_all = "camelCase")]
pub enum ActionError {
    /// No such action id.
    Unknown(String),
    /// Bad or missing parameters.
    Invalid(String),
    /// Understood but not allowed (protected path, system process, not in scan...).
    Refused(String),
    /// The risk level needs a confirmation that was not given.
    ConfirmationRequired(String),
    /// Allowed, ran, and failed.
    Failed(String),
}

impl std::fmt::Display for ActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, msg) = match self {
            ActionError::Unknown(m) => ("unknown action", m),
            ActionError::Invalid(m) => ("invalid request", m),
            ActionError::Refused(m) => ("refused", m),
            ActionError::ConfirmationRequired(m) => ("confirmation required", m),
            ActionError::Failed(m) => ("failed", m),
        };
        write!(f, "{kind}: {msg}")
    }
}

/// Runs external programs. Abstracted so tests never touch Docker, WSL or Gradle.
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String>;
}

/// Terminates a process by PID. Abstracted for the same reason.
pub trait ProcessKiller: Send + Sync {
    fn kill(&self, pid: u32) -> Result<(), String>;
}

/// Machine facts actions may need. Assembled fresh for every request.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub processes: Vec<ProcessInfo>,
    /// Entries of the latest storage scan. Cleanup may only touch what is listed here.
    pub scan_entries: Vec<ScanEntry>,
    pub temp_dir: Option<PathBuf>,
    pub user_profile: Option<PathBuf>,
}

pub struct Ctx<'a> {
    pub facts: &'a Facts,
    pub runner: &'a dyn CommandRunner,
    pub killer: &'a dyn ProcessKiller,
    pub probe: &'a dyn Probe,
    pub trasher: &'a dyn Trasher,
}

pub trait Action: Send + Sync {
    fn spec(&self) -> &'static ActionSpec;
    /// Validate the parameters against current facts and describe exactly what would happen.
    /// Must not change anything.
    fn preview(&self, params: &Value, ctx: &Ctx) -> Result<Preview, ActionError>;
    /// Do it. The engine calls `preview` first and enforces confirmations; implementations must
    /// still treat `params` as untrusted.
    fn execute(&self, params: &Value, ctx: &Ctx) -> Result<Outcome, ActionError>;
    /// Undo a previous execution, when that is possible. `None` means not reversible.
    /// Check that the expected end state is real. Called after `execute`, repeatedly while the
    /// system settles. The default says plainly that nothing was checked.
    fn verify(&self, _params: &Value, _outcome: &Outcome, _ctx: &Ctx) -> Verification {
        Verification::unverifiable("No automatic check exists for this action.")
    }
    /// A path whose volume's free space should be measured around the action.
    fn measure_path(&self, _params: &Value, _ctx: &Ctx) -> Option<PathBuf> {
        None
    }
    /// Extra caveat for the measurement (for example Docker keeping memory after a stop).
    fn measure_note(&self) -> Option<&'static str> {
        None
    }
    fn rollback(&self, _outcome: &Outcome, _ctx: &Ctx) -> Option<Result<String, ActionError>> {
        None
    }
}

/// Which confirmations the user gave for one execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Confirmation {
    pub confirmed: bool,
    pub double_confirmed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn risk_ordering_and_confirmation_rules() {
        assert!(RiskLevel::Safe < RiskLevel::Low && RiskLevel::High < RiskLevel::Destructive);
        for (r, c, d) in [
            (RiskLevel::Safe, false, false),
            (RiskLevel::Low, false, false),
            (RiskLevel::Medium, true, false),
            (RiskLevel::High, true, false),
            (RiskLevel::Destructive, true, true),
        ] {
            assert_eq!(
                (r.requires_confirmation(), r.requires_double_confirmation()),
                (c, d),
                "{r:?}"
            );
        }
    }

    #[test]
    fn errors_serialize_with_code_and_message() {
        let v = serde_json::to_value(ActionError::Refused("nope".into())).unwrap();
        assert_eq!(v["code"], "refused");
        assert_eq!(v["message"], "nope");
        assert_eq!(
            serde_json::to_string(&RiskLevel::Destructive).unwrap(),
            "\"DESTRUCTIVE\""
        );
    }
}

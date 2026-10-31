//! Action engine types. Every corrective action is described by an `ActionSpec` and implements
//! `Action` (preview, execute, optional rollback). Nothing runs without going through the engine's
//! permission checks.

use std::path::PathBuf;

use serde::Serialize;
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

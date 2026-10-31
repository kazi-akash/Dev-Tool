//! Diagnosis data model (JSON camelCase, shared with the UI and, sanitized, the AI gateway).

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Info,
    Warning,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contributor {
    pub name: String,
    pub memory_bytes: u64,
    /// Share of total installed RAM, 0-100.
    pub percent: f32,
    pub process_count: usize,
    /// Largest process, for example `java.exe`.
    pub detail: Option<String>,
}

/// Reference to an action the user can approve. The action engine re-validates everything.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRef {
    pub action_id: String,
    pub params: Value,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub id: String,
    pub problem: String,
    pub explanation: String,
    /// Human-readable, for example "About 1.8 GB of RAM".
    pub expected_benefit: String,
    pub expected_benefit_bytes: Option<u64>,
    pub risk: Risk,
    /// One-click action, when a safe one exists.
    pub action: Option<ActionRef>,
    /// What to do by hand when there is no action.
    pub manual_steps: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnosis {
    /// Stable rule id, for example `memory-pressure`.
    pub rule_id: String,
    pub severity: Severity,
    pub title: String,
    /// Explains *why*, with numbers.
    pub summary: String,
    pub primary_cause: Option<String>,
    pub contributors: Vec<Contributor>,
    /// 0..1
    pub confidence: f32,
    pub evidence: Vec<String>,
    pub recommendations: Vec<Recommendation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Healthy,
    Attention,
    Problem,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosisReport {
    pub generated_at_ms: u64,
    pub status: Status,
    /// Most important finding (first of `findings`).
    pub headline: Diagnosis,
    pub findings: Vec<Diagnosis>,
}

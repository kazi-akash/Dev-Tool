//! Process data model shared with the UI (camelCase JSON) and later engines (workloads, diagnosis).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Category {
    System,
    Browser,
    Development,
    Docker,
    Node,
    Php,
    Database,
    Java,
    Android,
    Flutter,
    Ide,
    Other,
}

/// What a classification was based on. Name-only matches are the weakest evidence;
/// downstream rules should not treat them as proof (e.g. a renamed `node.exe`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Evidence {
    /// Executable location (e.g. under `C:\Windows`, `Microsoft VS Code`).
    Path,
    /// Command line content (e.g. `artisan`, `GradleDaemon`).
    CommandLine,
    /// Executable file name only.
    Name,
    /// Nothing matched.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Classification {
    pub category: Category,
    pub evidence: Evidence,
}

impl Classification {
    pub const OTHER: Self = Self {
        category: Category::Other,
        evidence: Evidence::None,
    };
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    /// Full executable path when the OS allows reading it.
    pub exe_path: Option<String>,
    /// Redacted and truncated; see `redact`.
    pub command_line: Option<String>,
    pub parent_pid: Option<u32>,
    /// Process start, Unix seconds.
    pub start_time: u64,
    /// Share of total system CPU (0-100), not per-core.
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub category: Category,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortBy {
    Memory,
    Cpu,
    Name,
}

pub const DEFAULT_LIMIT: usize = 100;
pub const MAX_LIMIT: usize = 500;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessQuery {
    pub limit: Option<usize>,
    pub sort_by: Option<SortBy>,
    pub category: Option<Category>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessList {
    /// All processes currently known.
    pub total: usize,
    /// Processes matching the filter (before `limit`).
    pub matched: usize,
    pub processes: Vec<ProcessInfo>,
}

/// Apply filter, sort and limit. Pure; the UI never receives the full unbounded list.
pub fn query_processes(all: &[ProcessInfo], q: &ProcessQuery) -> ProcessList {
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let sort_by = q.sort_by.unwrap_or(SortBy::Memory);

    let mut matched: Vec<&ProcessInfo> = all
        .iter()
        .filter(|p| q.category.is_none_or(|c| p.category == c))
        .collect();
    let matched_count = matched.len();

    match sort_by {
        SortBy::Memory => matched.sort_by_key(|p| std::cmp::Reverse(p.memory_bytes)),
        SortBy::Cpu => matched.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent)),
        SortBy::Name => matched.sort_by_key(|p| p.name.to_lowercase()),
    }

    ProcessList {
        total: all.len(),
        matched: matched_count,
        processes: matched.into_iter().take(limit).cloned().collect(),
    }
}

#[cfg(test)]
pub(crate) fn sample(pid: u32, name: &str, mem: u64, cpu: f32, category: Category) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.into(),
        exe_path: None,
        command_line: None,
        parent_pid: None,
        start_time: 0,
        cpu_percent: cpu,
        memory_bytes: mem,
        category,
        evidence: Evidence::Name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<ProcessInfo> {
        vec![
            sample(1, "b.exe", 100, 5.0, Category::Node),
            sample(2, "A.exe", 300, 1.0, Category::Docker),
            sample(3, "c.exe", 200, 9.0, Category::Node),
        ]
    }

    fn q(
        limit: Option<usize>,
        sort_by: Option<SortBy>,
        category: Option<Category>,
    ) -> ProcessQuery {
        ProcessQuery {
            limit,
            sort_by,
            category,
        }
    }

    #[test]
    fn default_sorts_by_memory_desc() {
        let r = query_processes(&fixture(), &q(None, None, None));
        let pids: Vec<u32> = r.processes.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![2, 3, 1]);
        assert_eq!((r.total, r.matched), (3, 3));
    }

    #[test]
    fn sorts_by_cpu_and_name_case_insensitive() {
        let by_cpu = query_processes(&fixture(), &q(None, Some(SortBy::Cpu), None));
        assert_eq!(by_cpu.processes[0].pid, 3);
        let by_name = query_processes(&fixture(), &q(None, Some(SortBy::Name), None));
        assert_eq!(by_name.processes[0].name, "A.exe");
    }

    #[test]
    fn filters_by_category_and_reports_matched() {
        let r = query_processes(&fixture(), &q(None, None, Some(Category::Node)));
        assert_eq!(r.matched, 2);
        assert_eq!(r.total, 3);
        assert!(r.processes.iter().all(|p| p.category == Category::Node));
    }

    #[test]
    fn limit_truncates_but_matched_is_pre_limit() {
        let r = query_processes(&fixture(), &q(Some(1), None, None));
        assert_eq!(r.processes.len(), 1);
        assert_eq!(r.matched, 3);
    }

    #[test]
    fn limit_is_clamped() {
        let r = query_processes(&fixture(), &q(Some(0), None, None));
        assert_eq!(r.processes.len(), 1);
        let many: Vec<ProcessInfo> = (0..MAX_LIMIT as u32 + 50)
            .map(|i| sample(i, "x.exe", 1, 0.0, Category::Other))
            .collect();
        let r = query_processes(&many, &q(Some(usize::MAX), None, None));
        assert_eq!(r.processes.len(), MAX_LIMIT);
    }

    #[test]
    fn category_serializes_screaming_snake() {
        assert_eq!(serde_json::to_string(&Category::Ide).unwrap(), "\"IDE\"");
        assert_eq!(serde_json::to_string(&Category::Php).unwrap(), "\"PHP\"");
        let c: Category = serde_json::from_str("\"DOCKER\"").unwrap();
        assert_eq!(c, Category::Docker);
    }
}

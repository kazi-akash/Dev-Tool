//! Turns sysinfo's live process table into `ProcessInfo`, caching the expensive static parts
//! (path, redacted command line, classification) per process instance.

use std::collections::{HashMap, HashSet};

use sysinfo::{Pid, System};

use super::classify::classify;
use super::model::{Category, Evidence, ProcessInfo};
use super::redact::redact_command_line;

/// sysinfo reports per-process CPU as a share of one core (can exceed 100 on multi-core).
/// Normalize to a share of total system capacity, like Task Manager.
pub fn normalize_cpu(raw: f32, cores: usize) -> f32 {
    if !raw.is_finite() || raw <= 0.0 {
        return 0.0;
    }
    (raw / cores.max(1) as f32).clamp(0.0, 100.0)
}

/// Static facts about one process instance. Keyed by (pid, start_time) so a reused PID is not
/// mistaken for the old process.
struct StaticInfo {
    name: String,
    exe_path: Option<String>,
    command_line: Option<String>,
    category: Category,
    evidence: Evidence,
}

#[derive(Default)]
pub struct ProcessCollector {
    cache: HashMap<(u32, u64), StaticInfo>,
}

impl ProcessCollector {
    /// Build the current list. Entries for processes that disappeared are dropped from the cache.
    pub fn collect(&mut self, sys: &System) -> Vec<ProcessInfo> {
        let cores = sys.cpus().len().max(1);
        let mut live: HashSet<(u32, u64)> = HashSet::with_capacity(sys.processes().len());
        let mut out = Vec::with_capacity(sys.processes().len());

        for (pid, proc_) in sys.processes() {
            let pid_u32 = pid.as_u32();
            let start = proc_.start_time();
            let key = (pid_u32, start);
            live.insert(key);

            let info = self.cache.entry(key).or_insert_with(|| {
                let name = proc_.name().to_string_lossy().into_owned();
                let exe_path = proc_.exe().map(|p| p.to_string_lossy().into_owned());
                let args: Vec<String> = proc_
                    .cmd()
                    .iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
                // Classify on the raw command line (matching only), but never store it raw.
                let class = classify(&name, exe_path.as_deref(), &args.join(" "));
                StaticInfo {
                    name,
                    exe_path,
                    command_line: redact_command_line(&args),
                    category: class.category,
                    evidence: class.evidence,
                }
            });

            out.push(ProcessInfo {
                pid: pid_u32,
                name: info.name.clone(),
                exe_path: info.exe_path.clone(),
                command_line: info.command_line.clone(),
                parent_pid: proc_.parent().map(Pid::as_u32),
                start_time: start,
                cpu_percent: normalize_cpu(proc_.cpu_usage(), cores),
                memory_bytes: proc_.memory(),
                category: info.category,
                evidence: info.evidence,
            });
        }

        self.cache.retain(|k, _| live.contains(k));
        // `live` is only needed for the retain above.
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against the real OS process table: the test binary itself must show up with sane data.
    #[test]
    fn collects_real_processes_including_self() {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, UpdateKind};

        let mut sys = System::new().expect("system info available");
        sys.refresh_cpu_usage();
        let kind = ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet);
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        sys.refresh_cpu_usage();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);

        let mut collector = ProcessCollector::default();
        let list = collector.collect(&sys);
        assert!(
            list.len() > 10,
            "expected many processes, got {}",
            list.len()
        );

        let me = list
            .iter()
            .find(|p| p.pid == std::process::id())
            .expect("own process present");
        assert!(me.memory_bytes > 0);
        assert!(me.exe_path.is_some());
        assert!(me.start_time > 0);
        assert!(me.parent_pid.is_some());
        assert!((0.0..=100.0).contains(&me.cpu_percent));
        assert!(list.iter().all(|p| (0.0..=100.0).contains(&p.cpu_percent)));

        // Second pass reuses the cache and yields the same classification.
        let again = collector.collect(&sys);
        let me2 = again.iter().find(|p| p.pid == me.pid).unwrap();
        assert_eq!((me.category, me.evidence), (me2.category, me2.evidence));
    }

    #[test]
    fn cpu_is_normalized_by_core_count() {
        assert_eq!(normalize_cpu(400.0, 8), 50.0);
        assert_eq!(normalize_cpu(100.0, 1), 100.0);
    }

    #[test]
    fn cpu_is_clamped_and_nan_safe() {
        assert_eq!(normalize_cpu(2000.0, 4), 100.0);
        assert_eq!(normalize_cpu(f32::NAN, 4), 0.0);
        assert_eq!(normalize_cpu(-1.0, 4), 0.0);
        assert_eq!(normalize_cpu(50.0, 0), 50.0);
    }
}

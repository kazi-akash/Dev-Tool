//! Production implementations of the engine's side-effect traits.

use std::path::Path;
use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use super::model::{
    CommandRunner, Probe, ProbeProcess, ProcessKiller, SystemReading, TrashReceipt, Trasher,
};
use crate::docker::run_program;

pub struct RealRunner;

impl CommandRunner for RealRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        // Only programs the built-in actions use; this is not a general shell.
        if !matches!(program, "docker" | "wsl" | "cmd") {
            return Err(format!("program '{program}' is not allowed"));
        }
        run_program(program, args, Duration::from_secs(60))
    }
}

pub struct RealKiller;

impl ProcessKiller for RealKiller {
    fn kill(&self, pid: u32) -> Result<(), String> {
        let mut sys = System::new().map_err(|e| e.to_string())?;
        let pid = Pid::from_u32(pid);
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        let proc_ = sys.process(pid).ok_or("the process has already exited")?;
        if proc_.kill() {
            Ok(())
        } else {
            Err("Windows refused to end the process (it may need administrator rights)".into())
        }
    }
}

/// Fresh OS readings for verification and measurement. Each call builds its own `System`, so it
/// never depends on the 2-second-old monitor cache.
pub struct RealProbe;

impl Probe for RealProbe {
    fn reading(&self, measure_path: Option<&Path>) -> SystemReading {
        let mut reading = SystemReading::default();
        if let Ok(mut sys) = System::new() {
            sys.refresh_memory();
            let total = sys.total_memory();
            let available = sys.available_memory().min(total);
            if total > 0 {
                reading.ram_available_bytes = Some(available);
                reading.ram_used_bytes = Some(total - available);
            }
        }
        if let (Some(path), Ok(disks)) = (measure_path, sysinfo::Disks::new_with_refreshed_list()) {
            reading.disk_free_bytes = free_space_for(path, &disks);
        }
        reading
    }

    fn processes(&self) -> Option<Vec<ProbeProcess>> {
        let mut sys = System::new().ok()?;
        sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cmd(UpdateKind::Always),
        );
        Some(
            sys.processes()
                .iter()
                .map(|(pid, p)| ProbeProcess {
                    pid: pid.as_u32(),
                    name: p.name().to_string_lossy().into_owned(),
                    cmd: p
                        .cmd()
                        .iter()
                        .map(|a| a.to_string_lossy())
                        .collect::<Vec<_>>()
                        .join(" ")
                        .to_lowercase(),
                })
                .collect(),
        )
    }
}

/// Free bytes of the volume with the longest mount point that is a prefix of `path`.
fn free_space_for(path: &Path, disks: &sysinfo::Disks) -> Option<u64> {
    let p = path.to_string_lossy().to_lowercase().replace('/', "\\");
    disks
        .iter()
        .filter(|d| {
            let m = d
                .mount_point()
                .to_string_lossy()
                .to_lowercase()
                .replace('/', "\\");
            !m.is_empty() && p.starts_with(&m)
        })
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

/// The Windows Recycle Bin (via the `trash` crate; its own unsafe code is not ours).
pub struct RealTrasher;

#[cfg(windows)]
fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().to_lowercase().replace('/', "\\");
    norm(a).trim_end_matches('\\') == norm(b).trim_end_matches('\\')
}

#[cfg(windows)]
impl Trasher for RealTrasher {
    fn trash(&self, path: &Path) -> Result<TrashReceipt, String> {
        trash::delete(path).map_err(|e| e.to_string())?;
        // Windows may have bypassed the bin (disabled for the volume, item too large). Look for the
        // item instead of assuming; "not found" is reported as such.
        let in_bin = trash::os_limited::list()
            .map(|items| items.iter().any(|i| same_path(&i.original_path(), path)))
            .unwrap_or(false);
        Ok(TrashReceipt { in_bin })
    }

    fn restore(&self, original: &Path) -> Result<(), String> {
        let items: Vec<_> = trash::os_limited::list()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|i| same_path(&i.original_path(), original))
            .collect();
        if items.is_empty() {
            return Err(
                "The item is no longer in the Recycle Bin (it may have been emptied or restored)."
                    .into(),
            );
        }
        trash::os_limited::restore_all(items).map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
impl Trasher for RealTrasher {
    fn trash(&self, _: &Path) -> Result<TrashReceipt, String> {
        Err("the Recycle Bin is only available on Windows".into())
    }
    fn restore(&self, _: &Path) -> Result<(), String> {
        Err("the Recycle Bin is only available on Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_rejects_unlisted_programs() {
        assert!(
            RealRunner
                .run("powershell", &["-c", "calc"])
                .unwrap_err()
                .contains("not allowed")
        );
        assert!(
            RealRunner
                .run("cmd.exe", &["/C", "dir"])
                .unwrap_err()
                .contains("not allowed")
        );
        assert!(RealRunner.run("..\\evil", &[]).is_err());
    }

    #[test]
    fn killing_a_missing_process_is_an_error_not_a_panic() {
        assert!(RealKiller.kill(u32::MAX - 1).is_err());
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;

    #[test]
    fn real_probe_reads_memory_processes_and_disk() {
        let p = RealProbe;
        let r = p.reading(Some(&std::env::temp_dir()));
        assert!(r.ram_used_bytes.is_some() && r.ram_available_bytes.is_some());
        assert!(r.disk_free_bytes.is_some_and(|b| b > 0), "{r:?}");
        let ps = p.processes().expect("process list");
        assert!(ps.iter().any(|x| x.pid == std::process::id()));
    }

    #[test]
    fn disk_lookup_prefers_the_longest_mount_prefix() {
        // A path on no known volume yields nothing instead of a wrong drive.
        let disks = sysinfo::Disks::new_with_refreshed_list().unwrap();
        assert_eq!(free_space_for(Path::new("Q:\\nowhere\\x"), &disks), None);
    }
}

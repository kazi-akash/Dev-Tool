//! Production implementations of the engine's side-effect traits.

use std::time::Duration;

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use super::model::{CommandRunner, ProcessKiller};
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

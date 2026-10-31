//! Docker monitor. Talks to the `docker` CLI on demand (never per tick) and degrades to
//! `available: false` when Docker is missing, stopped, or slow.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

const CACHE_TTL: Duration = Duration::from_secs(10);
const CLI_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub running: bool,
    pub memory_bytes: Option<u64>,
    pub cpu_percent: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerStatus {
    pub available: bool,
    /// Why Docker data is missing (not installed, daemon stopped, timeout).
    pub error: Option<String>,
    pub containers: Vec<Container>,
    pub total_memory_bytes: u64,
}

impl DockerStatus {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            error: Some(reason.into()),
            containers: vec![],
            total_memory_bytes: 0,
        }
    }
}

/// "52.3MiB" / "1.5GiB" / "512kB" / "0B" -> bytes. Docker mixes SI and IEC suffixes.
pub fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = s.split_at(split);
    let value: f64 = num.parse().ok()?;
    let mult: f64 = match unit.trim().to_ascii_lowercase().as_str() {
        "b" => 1.0,
        "kb" => 1e3,
        "kib" => 1024.0,
        "mb" => 1e6,
        "mib" => 1024.0 * 1024.0,
        "gb" => 1e9,
        "gib" => 1024.0 * 1024.0 * 1024.0,
        "tb" => 1e12,
        "tib" => 1024.0_f64.powi(4),
        _ => return None,
    };
    Some((value * mult) as u64)
}

/// `"52.3MiB / 7.6GiB"` -> bytes used.
pub fn parse_mem_usage(s: &str) -> Option<u64> {
    parse_size(s.split('/').next()?)
}

fn str_field<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Parse `docker ps --format "{{json .}}"` output (one JSON object per line). Bad lines are skipped.
pub fn parse_ps(output: &str) -> Vec<Container> {
    output
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .map(|v| {
            let state = str_field(&v, "State");
            let status = str_field(&v, "Status");
            Container {
                id: str_field(&v, "ID").to_string(),
                name: str_field(&v, "Names").to_string(),
                image: str_field(&v, "Image").to_string(),
                status: status.to_string(),
                running: state == "running" || (state.is_empty() && status.starts_with("Up")),
                memory_bytes: None,
                cpu_percent: None,
            }
        })
        .filter(|c| !c.id.is_empty())
        .collect()
}

/// Merge `docker stats --no-stream --format "{{json .}}"` into containers (matched by id prefix or name).
pub fn merge_stats(containers: &mut [Container], output: &str) {
    for v in output
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
    {
        let id = str_field(&v, "ID");
        let id = if id.is_empty() {
            str_field(&v, "Container")
        } else {
            id
        };
        let name = str_field(&v, "Name");
        let Some(c) = containers.iter_mut().find(|c| {
            (!id.is_empty() && (c.id.starts_with(id) || id.starts_with(&c.id))) || c.name == name
        }) else {
            continue;
        };
        c.memory_bytes = parse_mem_usage(str_field(&v, "MemUsage"));
        c.cpu_percent = str_field(&v, "CPUPerc").trim_end_matches('%').parse().ok();
    }
}

/// Container name or id as accepted by Docker. Rejects anything that could be a flag or contain shell syntax.
#[allow(dead_code)] // consumed by the stop-container action (Phase 12)
pub fn is_valid_container_ref(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    s.len() <= 128
        && first.is_ascii_alphanumeric()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Run `docker` with a hard timeout and no console window. Returns stdout.
pub fn run_docker(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("docker");
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Docker CLI not found".to_string()
        } else {
            format!("cannot start docker: {e}")
        }
    })?;

    let deadline = Instant::now() + CLI_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut s) = child.stdout.take() {
                    let _ = s.read_to_string(&mut out);
                }
                if status.success() {
                    return Ok(out);
                }
                let mut err = String::new();
                if let Some(mut s) = child.stderr.take() {
                    let _ = s.read_to_string(&mut err);
                }
                let first = err.lines().next().unwrap_or("docker returned an error");
                return Err(first.chars().take(160).collect());
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("docker did not respond in time".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("docker wait failed: {e}")),
        }
    }
}

fn query_docker() -> DockerStatus {
    let ps = match run_docker(&["ps", "--format", "{{json .}}"]) {
        Ok(o) => o,
        Err(e) => return DockerStatus::unavailable(e),
    };
    let mut containers = parse_ps(&ps);
    if containers.iter().any(|c| c.running) {
        // Stats are best-effort: containers still list without them.
        if let Ok(stats) = run_docker(&["stats", "--no-stream", "--format", "{{json .}}"]) {
            merge_stats(&mut containers, &stats);
        }
    }
    let total = containers.iter().filter_map(|c| c.memory_bytes).sum();
    DockerStatus {
        available: true,
        error: None,
        containers,
        total_memory_bytes: total,
    }
}

/// Cached on-demand Docker status.
#[derive(Default)]
pub struct DockerMonitor {
    cache: Mutex<Option<(Instant, DockerStatus)>>,
}

impl DockerMonitor {
    /// Blocking. Call from a worker thread, not the UI/async executor directly.
    pub fn status(&self, force: bool) -> DockerStatus {
        {
            let guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((at, s)) = guard.as_ref()
                && !force
                && at.elapsed() < CACHE_TTL
            {
                return s.clone();
            }
        }
        let fresh = query_docker();
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((Instant::now(), fresh.clone()));
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sizes_with_si_and_iec_units() {
        assert_eq!(parse_size("0B"), Some(0));
        assert_eq!(parse_size("512kB"), Some(512_000));
        assert_eq!(parse_size("1KiB"), Some(1024));
        assert_eq!(parse_size("52.3MiB"), Some((52.3 * 1024.0 * 1024.0) as u64));
        assert_eq!(parse_size("1.5GiB"), Some(1610612736));
        assert_eq!(parse_size("2GB"), Some(2_000_000_000));
    }

    #[test]
    fn rejects_bad_sizes() {
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size("12"), None);
        assert_eq!(parse_size("5 parsecs"), None);
    }

    #[test]
    fn mem_usage_takes_used_part() {
        assert_eq!(parse_mem_usage("100MiB / 7.6GiB"), Some(100 * 1024 * 1024));
        assert_eq!(parse_mem_usage("garbage"), None);
    }

    const PS: &str = r#"{"ID":"abc123def456","Names":"redis","Image":"redis:7","Status":"Up 2 hours","State":"running"}
{"ID":"fff000aaa111","Names":"old-api","Image":"api:1","Status":"Exited (0) 3 days ago","State":"exited"}
not json at all
{"Names":"no-id"}"#;

    #[test]
    fn parses_ps_and_skips_garbage() {
        let c = parse_ps(PS);
        assert_eq!(c.len(), 2);
        assert!(c[0].running && !c[1].running);
        assert_eq!(c[0].name, "redis");
        assert_eq!(c[0].image, "redis:7");
    }

    #[test]
    fn running_inferred_from_status_when_state_missing() {
        let c = parse_ps(r#"{"ID":"a1","Names":"x","Image":"i","Status":"Up 3 minutes"}"#);
        assert!(c[0].running);
    }

    #[test]
    fn merges_stats_by_id_prefix_and_name() {
        let mut c = parse_ps(PS);
        merge_stats(
            &mut c,
            r#"{"ID":"abc123def456","Name":"redis","CPUPerc":"1.25%","MemUsage":"20MiB / 7.6GiB"}
{"Container":"zzz","Name":"unknown","CPUPerc":"0%","MemUsage":"1MiB / 1GiB"}"#,
        );
        assert_eq!(c[0].memory_bytes, Some(20 * 1024 * 1024));
        assert_eq!(c[0].cpu_percent, Some(1.25));
        assert_eq!(c[1].memory_bytes, None);
    }

    #[test]
    fn container_ref_validation_blocks_flags_and_shell_syntax() {
        assert!(is_valid_container_ref("redis"));
        assert!(is_valid_container_ref("my-app_1.web"));
        assert!(is_valid_container_ref("abc123def456"));
        for bad in [
            "",
            "-rf",
            "--all",
            "a b",
            "a;b",
            "a&b",
            "a|b",
            "$(x)",
            "a/b",
            "a`b",
            "..\\x",
            &"a".repeat(129),
        ] {
            assert!(!is_valid_container_ref(bad), "{bad}");
        }
    }

    #[test]
    fn unavailable_status_shape() {
        let s = DockerStatus::unavailable("Docker CLI not found");
        assert!(!s.available);
        assert_eq!(s.error.as_deref(), Some("Docker CLI not found"));
        assert!(s.containers.is_empty());
    }

    #[test]
    fn missing_cli_degrades_instead_of_failing() {
        // Whatever the machine has installed, this must return a status, never panic.
        let s = DockerMonitor::default().status(true);
        assert_eq!(s.available, s.error.is_none());
    }
}

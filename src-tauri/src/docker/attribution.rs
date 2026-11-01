//! Docker/WSL2 memory attribution. Pure: workload members and Docker status are passed in.
//!
//! Two different measurements exist and must never be added together:
//!
//! * **Host memory**: what Windows charges to the `vmmem`/`vmmemWSL` process and the Docker Desktop
//!   processes. This is what actually reduces free RAM.
//! * **Container-reported memory**: what `docker stats` says each container uses *inside* the WSL2
//!   VM. It is a subset of the VM's host memory, excludes page cache the VM still holds, and is
//!   sampled at a different moment.
//!
//! The breakdown shows both side by side, derives the unattributed part of the VM only when the
//! numbers are consistent, and says plainly when attribution is impossible. Nothing is invented.

use serde::Serialize;

use super::{Container, DockerStatus};
use crate::developers::workloads::{Member, Workload, WorkloadKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AttributionLevel {
    /// Every running container reported memory and the numbers are consistent.
    Attributed,
    /// Some containers have no memory figure, or the numbers disagree.
    Partial,
    /// No Docker data: only host-process memory is known.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerBreakdown {
    /// Memory Windows charges to Docker/WSL2 processes. This is the figure that counts against RAM.
    pub host_total_bytes: u64,
    /// The WSL2 VM process (`vmmem`/`vmmemWSL`), when present.
    pub wsl_vm_bytes: Option<u64>,
    /// Docker Desktop, backend, services and CLI on the host.
    pub desktop_backend_bytes: u64,
    /// Running containers, largest memory first (unknown memory last).
    pub containers: Vec<Container>,
    /// Sum of container-reported memory. Lives inside the VM; not added to `host_total_bytes`.
    pub containers_reported_bytes: Option<u64>,
    /// VM memory not explained by containers (other distros, page cache, kernel). `None` when it
    /// cannot be computed honestly.
    pub unattributed_vm_bytes: Option<u64>,
    pub level: AttributionLevel,
    pub notes: Vec<String>,
}

fn is_vm(m: &Member) -> bool {
    m.label.starts_with("WSL2 VM")
}

pub fn breakdown(workloads: &[Workload], docker: &DockerStatus) -> Option<DockerBreakdown> {
    let w = workloads
        .iter()
        .find(|w| matches!(w.kind, WorkloadKind::Docker))?;
    let vm: u64 = w
        .members
        .iter()
        .filter(|m| is_vm(m))
        .map(|m| m.memory_bytes)
        .sum();
    let has_vm = w.members.iter().any(is_vm);
    let wsl_vm_bytes = has_vm.then_some(vm);
    let desktop = w.memory_bytes.saturating_sub(vm);
    let mut notes = Vec::new();

    let mut running: Vec<Container> = docker
        .containers
        .iter()
        .filter(|c| c.running)
        .cloned()
        .collect();
    running.sort_by(|a, b| {
        b.memory_bytes
            .cmp(&a.memory_bytes)
            .then(a.name.cmp(&b.name))
    });

    if !docker.available {
        notes.push(format!(
            "Container details are unavailable ({}). Only host-process memory is shown; it cannot be split by container.",
            docker.error.as_deref().unwrap_or("Docker did not respond")
        ));
        return Some(DockerBreakdown {
            host_total_bytes: w.memory_bytes,
            wsl_vm_bytes,
            desktop_backend_bytes: desktop,
            containers: vec![],
            containers_reported_bytes: None,
            unattributed_vm_bytes: None,
            level: AttributionLevel::Unavailable,
            notes,
        });
    }

    let known: Vec<u64> = running.iter().filter_map(|c| c.memory_bytes).collect();
    let missing = running.len() - known.len();
    let reported: Option<u64> = (!known.is_empty()).then(|| known.iter().sum());
    let mut level = AttributionLevel::Attributed;

    if missing > 0 {
        level = AttributionLevel::Partial;
        notes.push(format!(
            "{missing} running container(s) reported no memory figure; they are listed without one, not as zero."
        ));
    }
    if running.is_empty() {
        notes.push("No containers are running. Remaining Docker/WSL2 memory is the VM and Docker Desktop themselves.".into());
    }

    let unattributed = match (wsl_vm_bytes, reported) {
        (Some(vm), Some(r)) if r <= vm && missing == 0 => Some(vm - r),
        (Some(vm), Some(r)) if r > vm => {
            level = AttributionLevel::Partial;
            notes.push("Containers report more memory than the VM holds on the host; the two samples were taken at different moments, so no remainder is computed.".into());
            None
        }
        (None, Some(_)) => {
            notes.push("No WSL2 VM process was found (Docker may use Hyper-V or another backend), so container memory cannot be compared to host memory.".into());
            level = AttributionLevel::Partial;
            None
        }
        _ => None,
    };
    if unattributed.is_some() {
        notes.push("Container-reported memory is inside the WSL2 VM. The remainder is the VM's own memory, caches and non-container WSL distros; Windows may hold it after containers release it.".into());
    }

    Some(DockerBreakdown {
        host_total_bytes: w.memory_bytes,
        wsl_vm_bytes,
        desktop_backend_bytes: desktop,
        containers: running,
        containers_reported_bytes: reported,
        unattributed_vm_bytes: unattributed,
        level,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;

    fn workload(vm: Option<u64>, desktop: u64) -> Workload {
        let mut members = vec![Member {
            label: "Docker Desktop".into(),
            process_count: 2,
            memory_bytes: desktop,
        }];
        if let Some(v) = vm {
            members.push(Member {
                label: "WSL2 VM (vmmem)".into(),
                process_count: 1,
                memory_bytes: v,
            });
        }
        Workload {
            kind: WorkloadKind::Docker,
            name: "Docker / WSL2".into(),
            memory_bytes: desktop + vm.unwrap_or(0),
            cpu_percent: 1.0,
            process_count: 3,
            members,
            top_processes: vec![],
            signals: vec![],
        }
    }

    fn c(name: &str, mem_mb: Option<u64>, running: bool) -> Container {
        Container {
            id: format!("id-{name}"),
            name: name.into(),
            image: "img:1".into(),
            status: "Up".into(),
            running,
            memory_bytes: mem_mb.map(|m| m * MB),
            cpu_percent: Some(1.0),
            memory_limit_bytes: None,
        }
    }

    fn status(containers: Vec<Container>) -> DockerStatus {
        DockerStatus {
            available: true,
            error: None,
            total_memory_bytes: containers.iter().filter_map(|c| c.memory_bytes).sum(),
            containers,
        }
    }

    #[test]
    fn host_total_is_not_double_counted_with_containers() {
        let w = workload(Some(4 * GB + 800 * MB), GB);
        let d = status(vec![c("api", Some(1400), true), c("mysql", Some(1100), true)]);
        let b = breakdown(std::slice::from_ref(&w), &d).unwrap();
        assert_eq!(b.host_total_bytes, w.memory_bytes, "host figure only");
        assert_eq!(b.containers_reported_bytes, Some(2500 * MB));
        assert_eq!(b.unattributed_vm_bytes, Some(4 * GB + 800 * MB - 2500 * MB));
        assert_eq!(b.level, AttributionLevel::Attributed);
        assert_eq!(b.desktop_backend_bytes, GB);
    }

    #[test]
    fn containers_sorted_largest_first_stopped_excluded() {
        let w = workload(Some(4 * GB), GB);
        let d = status(vec![
            c("redis", Some(380), true),
            c("api", Some(1400), true),
            c("old", None, false),
            c("worker", Some(620), true),
        ]);
        let b = breakdown(&[w], &d).unwrap();
        let names: Vec<&str> = b.containers.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["api", "worker", "redis"]);
    }

    #[test]
    fn missing_container_memory_is_partial_and_never_zero() {
        let w = workload(Some(4 * GB), GB);
        let d = status(vec![c("api", Some(1400), true), c("odd", None, true)]);
        let b = breakdown(&[w], &d).unwrap();
        assert_eq!(b.level, AttributionLevel::Partial);
        assert_eq!(b.unattributed_vm_bytes, None, "no remainder with an unknown part");
        assert_eq!(b.containers.last().unwrap().memory_bytes, None);
        assert!(b.notes.iter().any(|n| n.contains("not as zero")));
    }

    #[test]
    fn containers_larger_than_vm_do_not_produce_a_remainder() {
        let w = workload(Some(GB), GB);
        let d = status(vec![c("api", Some(3000), true)]);
        let b = breakdown(&[w], &d).unwrap();
        assert_eq!(b.unattributed_vm_bytes, None);
        assert_eq!(b.level, AttributionLevel::Partial);
        assert!(b.notes.iter().any(|n| n.contains("different moments")));
    }

    #[test]
    fn docker_unavailable_says_so_and_shows_host_only() {
        let w = workload(Some(4 * GB), GB);
        let d = DockerStatus {
            available: false,
            error: Some("Docker CLI not found".into()),
            containers: vec![],
            total_memory_bytes: 0,
        };
        let b = breakdown(&[w], &d).unwrap();
        assert_eq!(b.level, AttributionLevel::Unavailable);
        assert!(b.containers.is_empty());
        assert!(b.notes[0].contains("Docker CLI not found"));
        assert_eq!(b.host_total_bytes, 5 * GB);
    }

    #[test]
    fn no_vm_process_means_no_comparison() {
        let w = workload(None, GB);
        let d = status(vec![c("api", Some(500), true)]);
        let b = breakdown(&[w], &d).unwrap();
        assert_eq!(b.wsl_vm_bytes, None);
        assert_eq!(b.unattributed_vm_bytes, None);
        assert!(b.notes.iter().any(|n| n.contains("No WSL2 VM process")));
    }

    #[test]
    fn no_docker_workload_means_no_breakdown() {
        assert!(breakdown(&[], &status(vec![])).is_none());
    }

    #[test]
    fn no_running_containers_explains_remaining_memory() {
        let w = workload(Some(2 * GB), GB);
        let b = breakdown(&[w], &status(vec![c("old", None, false)])).unwrap();
        assert!(b.containers.is_empty());
        assert_eq!(b.containers_reported_bytes, None);
        assert!(b.notes.iter().any(|n| n.contains("No containers are running")));
    }
}

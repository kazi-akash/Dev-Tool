//! Groups raw processes into developer workloads (Docker/WSL2, Android/Gradle, Next.js, Laragon...).
//! Pure function of the process list. Each process joins at most one workload, chosen by priority.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::processes::model::{Category, ProcessInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkloadKind {
    Docker,
    Android,
    Flutter,
    Laragon,
    Laravel,
    Node,
    Java,
    Database,
    Ide,
    Browser,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub label: String,
    pub process_count: usize,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopProcess {
    pub pid: u32,
    pub name: String,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workload {
    pub kind: WorkloadKind,
    pub name: String,
    pub memory_bytes: u64,
    /// Sum of member CPU shares (of total system capacity), capped at 100.
    pub cpu_percent: f32,
    pub process_count: usize,
    pub members: Vec<Member>,
    pub top_processes: Vec<TopProcess>,
    /// Facts other engines can rely on, e.g. `gradle` (a Gradle daemon is running), `nextjs`, `wsl-vm`.
    pub signals: Vec<&'static str>,
}

impl Workload {
    pub fn has_signal(&self, s: &str) -> bool {
        self.signals.contains(&s)
    }
}

fn has_tag(p: &ProcessInfo, tag: &str) -> bool {
    p.tags.contains(&tag)
}

fn kind_for(p: &ProcessInfo) -> Option<WorkloadKind> {
    use WorkloadKind as K;
    if p.category == Category::Docker {
        return Some(K::Docker);
    }
    if p.category == Category::Android || (has_tag(p, "gradle") && has_tag(p, "android")) {
        return Some(K::Android);
    }
    if p.category == Category::Flutter {
        return Some(K::Flutter);
    }
    // The IDE itself is never part of a project stack, even if installed under a stack folder.
    if p.category == Category::Ide {
        return Some(K::Ide);
    }
    if has_tag(p, "laragon") {
        return Some(K::Laragon);
    }
    if p.category == Category::Php || has_tag(p, "artisan") {
        return Some(K::Laravel);
    }
    match p.category {
        Category::Node => Some(K::Node),
        Category::Java => Some(K::Java),
        Category::Database => Some(K::Database),
        Category::Browser => Some(K::Browser),
        _ => None,
    }
}

fn base_name(p: &ProcessInfo) -> String {
    let n = p.name.to_lowercase();
    n.strip_suffix(".exe").unwrap_or(&n).to_string()
}

fn member_label(kind: WorkloadKind, p: &ProcessInfo) -> String {
    use WorkloadKind as K;
    let n = base_name(p);
    let label = match kind {
        K::Docker => {
            if n.contains("docker desktop") {
                "Docker Desktop"
            } else if n == "com.docker.backend" {
                "Docker backend"
            } else if n.starts_with("com.docker") {
                "Docker services"
            } else if n.starts_with("vmmem") {
                "WSL2 VM (vmmem)"
            } else if n.starts_with("wsl") {
                "WSL"
            } else if n == "docker" || n == "dockerd" {
                "Docker engine / CLI"
            } else {
                return p.name.clone();
            }
        }
        K::Android => {
            if has_tag(p, "gradle") {
                "Gradle daemon"
            } else if n == "adb" {
                "adb"
            } else if n == "emulator" || n.starts_with("qemu-system") {
                "Android emulator"
            } else {
                return p.name.clone();
            }
        }
        K::Laragon => match p.category {
            Category::Database => "Database",
            Category::Php => "PHP",
            Category::Node => "Node.js",
            _ if n == "nginx" || n == "httpd" || n == "apache" => "Web server",
            _ if n == "laragon" => "Laragon",
            _ => return p.name.clone(),
        },
        K::Laravel => {
            if has_tag(p, "artisan") {
                "artisan"
            } else {
                "PHP"
            }
        }
        K::Node => {
            if has_tag(p, "nextjs") {
                "Next.js"
            } else if has_tag(p, "vite") {
                "Vite"
            } else if has_tag(p, "package-manager") {
                "Package manager"
            } else {
                "Node.js scripts"
            }
        }
        K::Java => {
            if has_tag(p, "gradle") {
                "Gradle daemon"
            } else {
                "JVM"
            }
        }
        K::Ide => {
            let path = p.exe_path.as_deref().unwrap_or("").to_lowercase();
            if path.contains("vs code") || n == "code" {
                "VS Code"
            } else if n == "kiro" {
                "Kiro"
            } else if n.contains("studio64") || path.contains("android studio") {
                "Android Studio"
            } else if path.contains("jetbrains") || n.contains("idea") {
                "JetBrains"
            } else if n == "cursor" {
                "Cursor"
            } else {
                return p.name.clone();
            }
        }
        K::Browser => {
            if n.contains("webview2") {
                "WebView2"
            } else if n.contains("chrome") {
                "Chrome"
            } else if n.contains("msedge") {
                "Edge"
            } else if n.contains("brave") {
                "Brave"
            } else if n.contains("firefox") {
                "Firefox"
            } else {
                return p.name.clone();
            }
        }
        K::Flutter | K::Database => return p.name.clone(),
    };
    label.to_string()
}

fn workload_name(kind: WorkloadKind, signals: &[&'static str]) -> &'static str {
    use WorkloadKind as K;
    match kind {
        K::Docker => "Docker / WSL2",
        K::Android => "Android / Gradle",
        K::Flutter => "Flutter",
        K::Laragon => "Laragon stack",
        K::Laravel => "Laravel / PHP",
        K::Node if signals.contains(&"nextjs") => "Next.js / Node.js",
        K::Node => "Node.js",
        K::Java => "Java / Gradle",
        K::Database => "Databases",
        K::Ide => "IDEs",
        K::Browser => "Browsers",
    }
}

fn signals_for(kind: WorkloadKind, procs: &[&ProcessInfo]) -> Vec<&'static str> {
    use WorkloadKind as K;
    let mut s = Vec::new();
    let any = |tag: &str| procs.iter().any(|p| has_tag(p, tag));
    if matches!(kind, K::Android | K::Java) && any("gradle") {
        s.push("gradle");
    }
    if kind == K::Node && any("nextjs") {
        s.push("nextjs");
    }
    if kind == K::Node && any("vite") {
        s.push("vite");
    }
    if kind == K::Docker && procs.iter().any(|p| base_name(p).starts_with("vmmem")) {
        s.push("wsl-vm");
    }
    s
}

/// Detect workloads, largest memory first.
pub fn detect_workloads(processes: &[ProcessInfo]) -> Vec<Workload> {
    let mut groups: BTreeMap<u8, (WorkloadKind, Vec<&ProcessInfo>)> = BTreeMap::new();
    for p in processes {
        if let Some(kind) = kind_for(p) {
            groups
                .entry(kind as u8)
                .or_insert_with(|| (kind, Vec::new()))
                .1
                .push(p);
        }
    }

    let mut out: Vec<Workload> = groups
        .into_values()
        .map(|(kind, procs)| {
            let mut members: BTreeMap<String, Member> = BTreeMap::new();
            for p in &procs {
                let label = member_label(kind, p);
                let m = members.entry(label.clone()).or_insert(Member {
                    label,
                    process_count: 0,
                    memory_bytes: 0,
                });
                m.process_count += 1;
                m.memory_bytes += p.memory_bytes;
            }
            let mut members: Vec<Member> = members.into_values().collect();
            members.sort_by_key(|m| std::cmp::Reverse(m.memory_bytes));

            let mut sorted = procs.clone();
            sorted.sort_by_key(|p| std::cmp::Reverse(p.memory_bytes));
            let signals = signals_for(kind, &procs);

            Workload {
                kind,
                name: workload_name(kind, &signals).to_string(),
                memory_bytes: procs.iter().map(|p| p.memory_bytes).sum(),
                cpu_percent: procs.iter().map(|p| p.cpu_percent).sum::<f32>().min(100.0),
                process_count: procs.len(),
                members,
                top_processes: sorted
                    .iter()
                    .take(3)
                    .map(|p| TopProcess {
                        pid: p.pid,
                        name: p.name.clone(),
                        memory_bytes: p.memory_bytes,
                    })
                    .collect(),
                signals,
            }
        })
        .collect();

    out.sort_by_key(|w| std::cmp::Reverse(w.memory_bytes));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processes::model::sample;

    const MB: u64 = 1024 * 1024;

    fn p(pid: u32, name: &str, mb: u64, cat: Category, tags: &[&'static str]) -> ProcessInfo {
        let mut x = sample(pid, name, mb * MB, 1.0, cat);
        x.tags = tags.to_vec();
        x
    }

    fn find(ws: &[Workload], k: WorkloadKind) -> &Workload {
        ws.iter()
            .find(|w| w.kind == k)
            .unwrap_or_else(|| panic!("missing {k:?}"))
    }

    #[test]
    fn docker_groups_desktop_backend_and_wsl_vm() {
        let ws = detect_workloads(&[
            p(1, "Docker Desktop.exe", 300, Category::Docker, &[]),
            p(2, "com.docker.backend.exe", 100, Category::Docker, &[]),
            p(3, "vmmemWSL", 4000, Category::Docker, &[]),
            p(4, "node.exe", 50, Category::Node, &[]),
        ]);
        let d = find(&ws, WorkloadKind::Docker);
        assert_eq!(d.memory_bytes, 4400 * MB);
        assert_eq!(d.process_count, 3);
        assert_eq!(d.members[0].label, "WSL2 VM (vmmem)");
        assert!(d.has_signal("wsl-vm"));
        assert_eq!(d.name, "Docker / WSL2");
    }

    #[test]
    fn android_gradle_needs_android_evidence() {
        let ws = detect_workloads(&[
            p(1, "java.exe", 3000, Category::Java, &["gradle", "android"]),
            p(2, "adb.exe", 20, Category::Android, &[]),
            p(3, "java.exe", 800, Category::Java, &["gradle"]),
        ]);
        let a = find(&ws, WorkloadKind::Android);
        assert_eq!(a.process_count, 2);
        assert!(a.has_signal("gradle"));
        // Plain Gradle stays in the Java workload, not Android.
        let j = find(&ws, WorkloadKind::Java);
        assert_eq!(j.memory_bytes, 800 * MB);
        assert_eq!(j.name, "Java / Gradle");
    }

    #[test]
    fn next_and_vite_members_and_name() {
        let ws = detect_workloads(&[
            p(1, "node.exe", 1500, Category::Node, &["nextjs"]),
            p(2, "node.exe", 500, Category::Node, &["vite"]),
            p(3, "node.exe", 100, Category::Node, &[]),
            p(4, "pnpm.exe", 40, Category::Node, &["package-manager"]),
        ]);
        let n = find(&ws, WorkloadKind::Node);
        assert_eq!(n.name, "Next.js / Node.js");
        let labels: Vec<&str> = n.members.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["Next.js", "Vite", "Node.js scripts", "Package manager"]
        );
    }

    #[test]
    fn laragon_stack_collects_db_web_and_php() {
        let ws = detect_workloads(&[
            p(1, "mysqld.exe", 400, Category::Database, &["laragon"]),
            p(2, "nginx.exe", 30, Category::Development, &["laragon"]),
            p(3, "php-cgi.exe", 80, Category::Php, &["laragon"]),
            p(4, "mysqld.exe", 900, Category::Database, &[]),
        ]);
        let l = find(&ws, WorkloadKind::Laragon);
        assert_eq!(l.process_count, 3);
        let labels: Vec<&str> = l.members.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, vec!["Database", "PHP", "Web server"]);
        // A MySQL outside Laragon is its own Database workload.
        assert_eq!(find(&ws, WorkloadKind::Database).memory_bytes, 900 * MB);
    }

    #[test]
    fn laravel_artisan_outside_laragon() {
        let ws = detect_workloads(&[p(1, "php.exe", 60, Category::Php, &["artisan"])]);
        let l = find(&ws, WorkloadKind::Laravel);
        assert_eq!(l.members[0].label, "artisan");
    }

    #[test]
    fn ide_is_never_pulled_into_a_stack() {
        let ws = detect_workloads(&[p(1, "Code.exe", 500, Category::Ide, &["laragon"])]);
        assert_eq!(ws.len(), 1);
        assert_eq!(ws[0].kind, WorkloadKind::Ide);
        assert_eq!(ws[0].members[0].label, "VS Code");
    }

    #[test]
    fn unrelated_processes_form_no_workload() {
        assert!(
            detect_workloads(&[
                p(1, "notepad.exe", 10, Category::Other, &[]),
                p(2, "svchost.exe", 10, Category::System, &[]),
            ])
            .is_empty()
        );
        assert!(detect_workloads(&[]).is_empty());
    }

    #[test]
    fn sorted_by_memory_with_top_processes_and_capped_cpu() {
        let mut a = p(1, "chrome.exe", 100, Category::Browser, &[]);
        a.cpu_percent = 80.0;
        let mut b = p(2, "chrome.exe", 900, Category::Browser, &[]);
        b.cpu_percent = 70.0;
        let ws = detect_workloads(&[
            a,
            b,
            p(3, "vmmem", 5000, Category::Docker, &[]),
            p(4, "chrome.exe", 50, Category::Browser, &[]),
            p(5, "chrome.exe", 40, Category::Browser, &[]),
        ]);
        assert_eq!(ws[0].kind, WorkloadKind::Docker);
        let br = find(&ws, WorkloadKind::Browser);
        assert_eq!(br.cpu_percent, 100.0);
        assert_eq!(br.top_processes.len(), 3);
        assert_eq!(br.top_processes[0].pid, 2);
    }

    #[test]
    fn serializes_camel_case() {
        let ws = detect_workloads(&[p(1, "vmmem", 1, Category::Docker, &[])]);
        let j = serde_json::to_string(&ws[0]).unwrap();
        assert!(j.contains("\"kind\":\"docker\""));
        assert!(j.contains("\"processCount\":1"));
        assert!(j.contains("\"topProcesses\""));
    }
}

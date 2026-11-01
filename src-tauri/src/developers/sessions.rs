//! Developer build/session tracking and incident correlation. Pure: processes and time are passed in.
//!
//! A *session* is a known developer activity (Gradle/Android build, Flutter build, Next.js build,
//! npm/pnpm build, Docker compose startup, Android emulator startup) observed in the process list.
//! The tracker records RAM before/peak, CPU and disk peaks, new processes and workload memory growth.
//!
//! Correlation with an incident is evidence-based and worded carefully: overlapping in time is never
//! enough to claim causation, so the strongest statement is "Strong correlation detected".

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::monitoring::incident::NamedBytes;
use crate::processes::model::{Category, ProcessInfo};

const GIB: u64 = 1024 * 1024 * 1024;
/// A detected activity must be absent this long before its session ends (builds spawn/die in bursts).
const END_GRACE_SECS: f64 = 10.0;
/// Shorter sessions are noise (a CLI help command, a failed start).
const MIN_SESSION_SECS: f64 = 5.0;
/// "Before" is the RAM level from about this long before the session started.
const BEFORE_SECS: f64 = 15.0;
const RING_SECS: f64 = 60.0;
/// A Gradle daemon only counts as building when it is doing real work (share of total CPU).
const GRADLE_DAEMON_BUSY_CPU: f32 = 3.0;
const MAX_NEW_PIDS: usize = 20_000;
/// How long after a session ended an incident still counts as overlapping.
const OVERLAP_SLACK_MS: u64 = 30_000;
const TOP_GROWTH: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ActivityKind {
    GradleBuild,
    AndroidBuild,
    FlutterBuild,
    NextBuild,
    PackageBuild,
    DockerCompose,
    AndroidEmulator,
}

impl ActivityKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::GradleBuild => "Gradle build",
            Self::AndroidBuild => "Android build",
            Self::FlutterBuild => "Flutter build",
            Self::NextBuild => "Next.js build",
            Self::PackageBuild => "npm/pnpm build",
            Self::DockerCompose => "Docker compose startup",
            Self::AndroidEmulator => "Android emulator startup",
        }
    }

    /// Startup activities leave a process running afterwards, so they are time-boxed.
    fn max_secs(self) -> f64 {
        match self {
            Self::DockerCompose => 180.0,
            Self::AndroidEmulator => 120.0,
            _ => 3_600.0,
        }
    }
}

fn norm(cmd: &str) -> String {
    cmd.to_lowercase().replace('/', "\\")
}

/// Cheap prefilter so the per-tick scan only lowercases command lines that could matter.
fn candidate(p: &ProcessInfo) -> bool {
    p.tags
        .iter()
        .any(|t| matches!(*t, "gradle" | "nextjs" | "package-manager"))
        || matches!(
            p.category,
            Category::Flutter | Category::Docker | Category::Android
        )
        || {
            let n = p.name.to_lowercase();
            n.starts_with("qemu-system")
                || n.starts_with("emulator")
                || n.starts_with("docker")
                || n.starts_with("flutter")
                || n.starts_with("dart")
        }
}

/// Which known activities are visible in this process list right now.
pub fn detect(processes: &[ProcessInfo]) -> HashSet<ActivityKind> {
    let mut found = HashSet::new();
    let mut gradle = false;
    let mut android = false;
    for p in processes.iter().filter(|p| candidate(p)) {
        let cmd = norm(p.command_line.as_deref().unwrap_or(""));
        let padded = format!(" {cmd} ");
        let name = p.name.to_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name).to_string();

        if p.tags.contains(&"gradle") {
            let client =
                cmd.contains("gradlewrappermain") || cmd.contains("gradle.launcher.gradlemain");
            let busy_daemon =
                cmd.contains("gradledaemon") && p.cpu_percent >= GRADLE_DAEMON_BUSY_CPU;
            if client || busy_daemon {
                gradle = true;
            }
            if p.tags.contains(&"android") {
                android = true;
            }
        }
        if cmd.contains("flutter_tools") && padded.contains(" build ") {
            found.insert(ActivityKind::FlutterBuild);
        }
        if p.tags.contains(&"nextjs") && cmd.contains("next build") {
            found.insert(ActivityKind::NextBuild);
        }
        if p.tags.contains(&"package-manager")
            && (padded.contains(" run build ") || padded.contains(" build "))
            && !cmd.contains("next build")
        {
            found.insert(ActivityKind::PackageBuild);
        }
        if (name == "docker" || name == "docker-compose")
            && (padded.contains(" compose up ")
                || padded.contains(" compose  up ")
                || (name == "docker-compose" && padded.contains(" up ")))
        {
            found.insert(ActivityKind::DockerCompose);
        }
        if name.starts_with("qemu-system") || (name == "emulator" && cmd.contains("-avd")) {
            found.insert(ActivityKind::AndroidEmulator);
        }
    }
    // A Next.js build runs under a package manager; report the more specific activity only.
    if found.contains(&ActivityKind::NextBuild) {
        found.remove(&ActivityKind::PackageBuild);
    }
    if gradle {
        found.insert(if android {
            ActivityKind::AndroidBuild
        } else {
            ActivityKind::GradleBuild
        });
    }
    found
}

/// What the tracker needs from one tick.
pub struct Tick<'a> {
    pub ts_ms: u64,
    pub ram_used_bytes: u64,
    pub ram_percent: f32,
    pub cpu_percent: f32,
    pub disk_active_percent: Option<f32>,
    pub workloads: &'a [NamedBytes],
    pub docker_bytes: Option<u64>,
    pub pids: &'a [u32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CorrelationLevel {
    /// Timing and memory growth agree.
    Strong,
    /// Overlapped in time only.
    Possible,
    /// No incident overlapped this session.
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Correlation {
    pub level: CorrelationLevel,
    pub incident_id: Option<i64>,
    pub statement: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    /// Database id; 0 until stored.
    pub id: i64,
    pub kind: ActivityKind,
    pub label: String,
    pub start_ms: u64,
    pub end_ms: u64,
    pub duration_secs: u64,
    pub ram_before_bytes: u64,
    pub ram_peak_bytes: u64,
    pub ram_peak_percent: f32,
    pub cpu_peak_percent: f32,
    pub disk_peak_percent: Option<f32>,
    pub processes_created: usize,
    /// Largest workload memory increases during the session.
    pub workload_growth: Vec<NamedBytes>,
    /// Docker/WSL2 memory change; negative when it shrank. Host-process memory, not container-reported.
    pub docker_change_bytes: i64,
    pub result: String,
    /// Filled at query time from the stored incidents; never persisted.
    #[serde(default, skip_deserializing)]
    pub correlation: Option<Correlation>,
}

struct Active {
    kind: ActivityKind,
    start_secs: f64,
    start_ms: u64,
    last_seen_secs: f64,
    ram_before: u64,
    ram_peak: u64,
    ram_peak_percent: f32,
    cpu_peak: f32,
    disk_peak: Option<f32>,
    start_pids: HashSet<u32>,
    new_pids: HashSet<u32>,
    workload_start: HashMap<String, u64>,
    workload_peak: HashMap<String, u64>,
    docker_start: Option<u64>,
    docker_peak: Option<u64>,
    last_ms: u64,
    /// Time of the last tick the activity was actually visible (the session ends here, not after the grace period).
    last_seen_ms: u64,
}

#[derive(Default)]
pub struct SessionTracker {
    active: HashMap<ActivityKind, Active>,
    /// Startup activities whose process keeps running; blocked until it disappears once.
    blocked: HashSet<ActivityKind>,
    ring: std::collections::VecDeque<(f64, u64)>,
}

impl SessionTracker {
    /// Labels of the sessions running now (for incident samples).
    pub fn active_labels(&self) -> Vec<String> {
        let mut v: Vec<String> = self.active.keys().map(|k| k.label().to_string()).collect();
        v.sort();
        v
    }

    pub fn observe(
        &mut self,
        now_secs: f64,
        tick: &Tick,
        detected: &HashSet<ActivityKind>,
    ) -> Vec<SessionSummary> {
        self.ring.push_back((now_secs, tick.ram_used_bytes));
        while self
            .ring
            .front()
            .is_some_and(|(t, _)| now_secs - t > RING_SECS)
        {
            self.ring.pop_front();
        }
        // A blocked startup activity becomes eligible again once its process is gone.
        self.blocked.retain(|k| detected.contains(k));

        for kind in detected {
            if self.blocked.contains(kind) {
                continue;
            }
            if !self.active.contains_key(kind) {
                let before = self
                    .ring
                    .iter()
                    .find(|(t, _)| now_secs - t <= BEFORE_SECS)
                    .map_or(tick.ram_used_bytes, |(_, b)| *b);
                self.active.insert(
                    *kind,
                    Active {
                        kind: *kind,
                        start_secs: now_secs,
                        start_ms: tick.ts_ms,
                        last_seen_secs: now_secs,
                        ram_before: before,
                        ram_peak: tick.ram_used_bytes,
                        ram_peak_percent: tick.ram_percent,
                        cpu_peak: tick.cpu_percent,
                        disk_peak: tick.disk_active_percent,
                        start_pids: tick.pids.iter().copied().collect(),
                        new_pids: HashSet::new(),
                        workload_start: tick
                            .workloads
                            .iter()
                            .map(|w| (w.name.clone(), w.bytes))
                            .collect(),
                        workload_peak: HashMap::new(),
                        docker_start: tick.docker_bytes,
                        docker_peak: tick.docker_bytes,
                        last_ms: tick.ts_ms,
                        last_seen_ms: tick.ts_ms,
                    },
                );
            }
            if let Some(a) = self.active.get_mut(kind) {
                a.last_seen_secs = now_secs;
                a.last_seen_ms = tick.ts_ms;
            }
        }

        let mut finished = Vec::new();
        let mut ended: Vec<ActivityKind> = Vec::new();
        for (kind, a) in self.active.iter_mut() {
            a.last_ms = tick.ts_ms;
            a.ram_peak = a.ram_peak.max(tick.ram_used_bytes);
            a.ram_peak_percent = a.ram_peak_percent.max(tick.ram_percent);
            a.cpu_peak = a.cpu_peak.max(tick.cpu_percent);
            a.disk_peak = match (a.disk_peak, tick.disk_active_percent) {
                (Some(x), Some(y)) => Some(x.max(y)),
                (x, y) => x.or(y),
            };
            for p in tick.pids {
                if !a.start_pids.contains(p) && a.new_pids.len() < MAX_NEW_PIDS {
                    a.new_pids.insert(*p);
                }
            }
            for w in tick.workloads {
                let e = a.workload_peak.entry(w.name.clone()).or_insert(0);
                *e = (*e).max(w.bytes);
            }
            a.docker_peak = match (a.docker_peak, tick.docker_bytes) {
                (Some(x), Some(y)) => Some(x.max(y)),
                (x, y) => x.or(y),
            };
            let gone = now_secs - a.last_seen_secs >= END_GRACE_SECS;
            let timed_out = now_secs - a.start_secs >= kind.max_secs();
            if gone || timed_out {
                ended.push(*kind);
                if timed_out && !gone {
                    self.blocked.insert(*kind);
                }
            }
        }
        for kind in ended {
            if let Some(a) = self.active.remove(&kind)
                && a.last_seen_secs - a.start_secs >= MIN_SESSION_SECS
            {
                finished.push(summarize(a, tick.docker_bytes));
            }
        }
        finished
    }
}

fn result_label(peak_percent: f32) -> &'static str {
    if peak_percent >= 92.0 {
        "CRITICAL MEMORY PRESSURE"
    } else if peak_percent >= 85.0 {
        "HIGH MEMORY PRESSURE"
    } else if peak_percent >= 70.0 {
        "MODERATE MEMORY PRESSURE"
    } else {
        "NORMAL"
    }
}

fn summarize(a: Active, docker_now: Option<u64>) -> SessionSummary {
    let mut growth: Vec<NamedBytes> = a
        .workload_peak
        .iter()
        .filter_map(|(name, peak)| {
            let start = a.workload_start.get(name).copied().unwrap_or(0);
            (*peak > start).then(|| NamedBytes {
                name: name.clone(),
                bytes: peak - start,
            })
        })
        .collect();
    growth.sort_by(|x, y| y.bytes.cmp(&x.bytes).then(x.name.cmp(&y.name)));
    growth.truncate(TOP_GROWTH);
    // Docker/WSL change: peak against start, both host-process memory.
    let docker_change = match (a.docker_start, a.docker_peak.or(docker_now)) {
        (Some(s), Some(p)) => p as i64 - s as i64,
        _ => 0,
    };
    let duration_secs = a.last_seen_ms.saturating_sub(a.start_ms) / 1000;
    SessionSummary {
        id: 0,
        kind: a.kind,
        label: a.kind.label().to_string(),
        start_ms: a.start_ms,
        end_ms: a.last_seen_ms,
        duration_secs,
        ram_before_bytes: a.ram_before,
        ram_peak_bytes: a.ram_peak,
        ram_peak_percent: a.ram_peak_percent,
        cpu_peak_percent: a.cpu_peak,
        disk_peak_percent: a.disk_peak,
        processes_created: a.new_pids.len(),
        workload_growth: growth,
        docker_change_bytes: docker_change,
        result: result_label(a.ram_peak_percent).to_string(),
        correlation: None,
    }
}

/// Window of a recorded or running incident.
#[derive(Debug, Clone, Copy)]
pub struct IncidentWindow {
    pub id: i64,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Judge how a session relates to incidents. Never claims causation.
///
/// STRONG needs all of: time overlap, RAM that grew by at least 1 GiB during the session, and
/// workload growth that explains at least half of that RAM growth. Overlap alone is only POSSIBLE.
pub fn correlate(s: &SessionSummary, incidents: &[IncidentWindow]) -> Correlation {
    let overlapping = incidents
        .iter()
        .filter(|i| i.start_ms <= s.end_ms + OVERLAP_SLACK_MS && i.end_ms >= s.start_ms)
        .max_by_key(|i| i.end_ms - i.start_ms);
    let Some(inc) = overlapping else {
        return Correlation {
            level: CorrelationLevel::None,
            incident_id: None,
            statement: format!(
                "No performance incident overlapped this {}.",
                s.label.to_lowercase()
            ),
        };
    };
    let ram_growth = s.ram_peak_bytes.saturating_sub(s.ram_before_bytes);
    let explained: u64 = s.workload_growth.iter().map(|w| w.bytes).sum();
    let strong = ram_growth >= GIB && explained * 2 >= ram_growth;
    if strong {
        let top = s
            .workload_growth
            .first()
            .map(|w| format!(" {} grew by {}.", w.name, crate::util::fmt_bytes(w.bytes)))
            .unwrap_or_default();
        Correlation {
            level: CorrelationLevel::Strong,
            incident_id: Some(inc.id),
            statement: format!(
                "Strong correlation detected. This {} overlapped incident #{:04}, RAM grew by {} during the session.{top} Timing and memory growth agree, but that does not by itself prove the {} caused the incident.",
                s.label.to_lowercase(),
                inc.id,
                crate::util::fmt_bytes(ram_growth),
                s.label.to_lowercase(),
            ),
        }
    } else {
        Correlation {
            level: CorrelationLevel::Possible,
            incident_id: Some(inc.id),
            statement: format!(
                "Possible correlation: this {} overlapped incident #{:04} in time, but the measured memory growth does not clearly come from it.",
                s.label.to_lowercase(),
                inc.id
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processes::model::{Category, Evidence};

    fn proc(
        name: &str,
        cmd: &str,
        cpu: f32,
        tags: Vec<&'static str>,
        cat: Category,
    ) -> ProcessInfo {
        ProcessInfo {
            pid: 1,
            name: name.into(),
            exe_path: None,
            command_line: Some(cmd.into()),
            parent_pid: None,
            start_time: 0,
            cpu_percent: cpu,
            memory_bytes: GIB,
            category: cat,
            evidence: Evidence::CommandLine,
            tags,
        }
    }

    #[test]
    fn detects_gradle_client_and_busy_daemon_but_not_idle_daemon() {
        let client = proc(
            "java.exe",
            "java -classpath gradle-wrapper.jar org.gradle.wrapper.GradleWrapperMain assembleDebug",
            1.0,
            vec!["gradle"],
            Category::Java,
        );
        assert!(detect(&[client]).contains(&ActivityKind::GradleBuild));
        let busy = proc(
            "java.exe",
            "java org.gradle.launcher.daemon.bootstrap.GradleDaemon 8.5",
            12.0,
            vec!["gradle"],
            Category::Java,
        );
        assert!(detect(&[busy]).contains(&ActivityKind::GradleBuild));
        let idle = proc(
            "java.exe",
            "java org.gradle.launcher.daemon.bootstrap.GradleDaemon 8.5",
            0.2,
            vec!["gradle"],
            Category::Java,
        );
        assert!(detect(&[idle]).is_empty(), "an idle daemon is not a build");
    }

    #[test]
    fn gradle_with_android_evidence_is_an_android_build() {
        let p = proc(
            "java.exe",
            "java org.gradle.wrapper.GradleWrapperMain assembleDebug -Pandroid",
            5.0,
            vec!["gradle", "android"],
            Category::Android,
        );
        let d = detect(&[p]);
        assert!(d.contains(&ActivityKind::AndroidBuild));
        assert!(!d.contains(&ActivityKind::GradleBuild));
    }

    #[test]
    fn detects_flutter_next_package_docker_emulator() {
        let flutter = proc(
            "dart.exe",
            "dart flutter_tools.snapshot build apk --release",
            5.0,
            vec![],
            Category::Flutter,
        );
        assert!(detect(&[flutter]).contains(&ActivityKind::FlutterBuild));
        let flutter_run = proc(
            "dart.exe",
            "dart flutter_tools.snapshot run",
            5.0,
            vec![],
            Category::Flutter,
        );
        assert!(
            detect(&[flutter_run]).is_empty(),
            "flutter run is not a build"
        );
        let next = proc(
            "node.exe",
            "node C:\\app\\node_modules\\next\\dist\\bin\\next build",
            5.0,
            vec!["nextjs"],
            Category::Node,
        );
        assert!(detect(&[next]).contains(&ActivityKind::NextBuild));
        let pnpm = proc(
            "node.exe",
            "node pnpm.cjs run build",
            5.0,
            vec!["package-manager"],
            Category::Node,
        );
        assert!(detect(&[pnpm]).contains(&ActivityKind::PackageBuild));
        let compose = proc(
            "docker.exe",
            "docker compose up -d",
            1.0,
            vec![],
            Category::Docker,
        );
        assert!(detect(&[compose]).contains(&ActivityKind::DockerCompose));
        let ps = proc("docker.exe", "docker ps", 1.0, vec![], Category::Docker);
        assert!(detect(&[ps]).is_empty());
        let emu = proc(
            "qemu-system-x86_64.exe",
            "qemu-system-x86_64 -avd Pixel_7",
            20.0,
            vec![],
            Category::Android,
        );
        assert!(detect(&[emu]).contains(&ActivityKind::AndroidEmulator));
    }

    #[test]
    fn next_build_under_pnpm_is_one_session_not_two() {
        let pnpm = proc(
            "node.exe",
            "node pnpm.cjs run build",
            1.0,
            vec!["package-manager"],
            Category::Node,
        );
        let next = proc(
            "node.exe",
            "node next\\dist\\bin\\next build",
            5.0,
            vec!["nextjs"],
            Category::Node,
        );
        let d = detect(&[pnpm, next]);
        assert_eq!(d.len(), 1);
        assert!(d.contains(&ActivityKind::NextBuild));
    }

    fn tick<'a>(t: u64, used_gb: f64, pct: f32, w: &'a [NamedBytes], pids: &'a [u32]) -> Tick<'a> {
        Tick {
            ts_ms: t * 1000,
            ram_used_bytes: (used_gb * GIB as f64) as u64,
            ram_percent: pct,
            cpu_percent: 30.0,
            disk_active_percent: Some(40.0),
            workloads: w,
            docker_bytes: Some(2 * GIB),
            pids,
        }
    }

    fn one(kind: ActivityKind) -> HashSet<ActivityKind> {
        HashSet::from([kind])
    }

    #[test]
    fn records_before_peak_growth_and_new_processes() {
        let mut t = SessionTracker::default();
        let none = HashSet::new();
        let gradle = |g: f64| {
            vec![NamedBytes {
                name: "Android/Gradle".into(),
                bytes: (g * GIB as f64) as u64,
            }]
        };
        for s in 0..20 {
            t.observe(s as f64, &tick(s, 8.2, 51.0, &[], &[1, 2, 3]), &none);
        }
        let k = ActivityKind::AndroidBuild;
        let w0 = gradle(1.0);
        t.observe(20.0, &tick(20, 8.4, 52.0, &w0, &[1, 2, 3]), &one(k));
        let w1 = gradle(5.3);
        for s in 21..60 {
            t.observe(
                s as f64,
                &tick(s, 15.4, 96.0, &w1, &[1, 2, 3, 100, 101, 102]),
                &one(k),
            );
        }
        assert_eq!(t.active_labels(), vec!["Android build"]);
        let mut done = vec![];
        for s in 60..80 {
            done.extend(t.observe(s as f64, &tick(s, 9.0, 56.0, &w1, &[1, 2, 3]), &none));
        }
        assert_eq!(done.len(), 1);
        let s = &done[0];
        assert_eq!(s.kind, ActivityKind::AndroidBuild);
        assert!((s.ram_before_bytes as f64 / GIB as f64 - 8.2).abs() < 0.3);
        assert!((s.ram_peak_bytes as f64 / GIB as f64 - 15.4).abs() < 0.01);
        assert_eq!(s.processes_created, 3);
        assert!((s.workload_growth[0].bytes as f64 / GIB as f64 - 4.3).abs() < 0.01);
        assert_eq!(s.result, "CRITICAL MEMORY PRESSURE");
        assert_eq!(s.disk_peak_percent, Some(40.0));
    }

    #[test]
    fn short_blips_and_brief_gaps_are_handled() {
        let mut t = SessionTracker::default();
        let none = HashSet::new();
        let k = ActivityKind::NextBuild;
        // 2 s blip: dropped.
        t.observe(0.0, &tick(0, 5.0, 30.0, &[], &[]), &one(k));
        t.observe(1.0, &tick(1, 5.0, 30.0, &[], &[]), &one(k));
        let mut done = vec![];
        for s in 2..20 {
            done.extend(t.observe(s as f64, &tick(s, 5.0, 30.0, &[], &[]), &none));
        }
        assert!(done.is_empty());
        // Gap under the grace period keeps one session.
        for s in 30..40 {
            t.observe(s as f64, &tick(s, 5.0, 30.0, &[], &[]), &one(k));
        }
        for s in 40..46 {
            assert!(
                t.observe(s as f64, &tick(s, 5.0, 30.0, &[], &[]), &none)
                    .is_empty()
            );
        }
        for s in 46..50 {
            t.observe(s as f64, &tick(s, 5.0, 30.0, &[], &[]), &one(k));
        }
        for s in 50..70 {
            done.extend(t.observe(s as f64, &tick(s, 5.0, 30.0, &[], &[]), &none));
        }
        assert_eq!(done.len(), 1, "one session across a 6 s gap");
    }

    #[test]
    fn running_emulator_is_time_boxed_and_does_not_restart() {
        let mut t = SessionTracker::default();
        let k = ActivityKind::AndroidEmulator;
        let mut done = vec![];
        for s in 0..600 {
            done.extend(t.observe(s as f64, &tick(s, 9.0, 55.0, &[], &[]), &one(k)));
        }
        assert_eq!(done.len(), 1, "startup is one 120 s session, not repeated");
        assert!(done[0].duration_secs <= 125);
        // Emulator closes, then starts again later: a new session is allowed.
        for s in 600..620 {
            t.observe(s as f64, &tick(s, 9.0, 55.0, &[], &[]), &HashSet::new());
        }
        t.observe(620.0, &tick(620, 9.0, 55.0, &[], &[]), &one(k));
        assert_eq!(t.active_labels(), vec!["Android emulator startup"]);
    }

    fn summary(ram_before_gb: u64, peak_gb: u64, grew_gb: u64) -> SessionSummary {
        SessionSummary {
            id: 1,
            kind: ActivityKind::AndroidBuild,
            label: "Android build".into(),
            start_ms: 100_000,
            end_ms: 160_000,
            duration_secs: 60,
            ram_before_bytes: ram_before_gb * GIB,
            ram_peak_bytes: peak_gb * GIB,
            ram_peak_percent: 96.0,
            cpu_peak_percent: 50.0,
            disk_peak_percent: None,
            processes_created: 10,
            workload_growth: if grew_gb > 0 {
                vec![NamedBytes {
                    name: "Android/Gradle".into(),
                    bytes: grew_gb * GIB,
                }]
            } else {
                vec![]
            },
            docker_change_bytes: 0,
            result: "CRITICAL MEMORY PRESSURE".into(),
            correlation: None,
        }
    }

    const INC: IncidentWindow = IncidentWindow {
        id: 42,
        start_ms: 120_000,
        end_ms: 200_000,
    };

    #[test]
    fn strong_needs_overlap_ram_growth_and_explained_growth() {
        let c = correlate(&summary(8, 15, 5), &[INC]);
        assert_eq!(c.level, CorrelationLevel::Strong);
        assert!(c.statement.starts_with("Strong correlation detected."));
        assert!(c.statement.contains("does not by itself prove"));
        assert_eq!(c.incident_id, Some(42));
    }

    #[test]
    fn overlap_without_explained_growth_is_only_possible() {
        let c = correlate(&summary(8, 15, 1), &[INC]);
        assert_eq!(c.level, CorrelationLevel::Possible);
        assert!(!c.statement.to_lowercase().contains("caused"));
        let c = correlate(&summary(8, 8, 0), &[INC]);
        assert_eq!(c.level, CorrelationLevel::Possible);
    }

    #[test]
    fn no_overlap_means_no_correlation_claim() {
        let far = IncidentWindow {
            id: 1,
            start_ms: 1_000_000,
            end_ms: 1_100_000,
        };
        let c = correlate(&summary(8, 15, 5), &[far]);
        assert_eq!(c.level, CorrelationLevel::None);
        assert_eq!(c.incident_id, None);
        assert_eq!(
            correlate(&summary(8, 15, 5), &[]).level,
            CorrelationLevel::None
        );
    }

    #[test]
    fn incident_just_after_the_session_still_overlaps_within_slack() {
        let after = IncidentWindow {
            id: 7,
            start_ms: 180_000,
            end_ms: 230_000,
        };
        assert_ne!(
            correlate(&summary(8, 15, 5), &[after]).level,
            CorrelationLevel::None
        );
        let later = IncidentWindow {
            id: 8,
            start_ms: 400_000,
            end_ms: 450_000,
        };
        assert_eq!(
            correlate(&summary(8, 15, 5), &[later]).level,
            CorrelationLevel::None
        );
    }

    #[test]
    fn result_labels_follow_peak() {
        assert_eq!(result_label(50.0), "NORMAL");
        assert_eq!(result_label(86.0), "HIGH MEMORY PRESSURE");
        assert_eq!(result_label(93.0), "CRITICAL MEMORY PRESSURE");
    }
}

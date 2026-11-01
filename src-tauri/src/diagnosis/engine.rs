//! Runs every rule, ranks the findings, and builds the report the UI and AI gateway consume.

use super::model::{DiagnosisReport, Severity, Status};
use super::rules::{Config, Input, RULES, healthy};

pub fn diagnose(input: &Input, config: &Config, now_ms: u64) -> DiagnosisReport {
    let mut ranked: Vec<(usize, _)> = RULES
        .iter()
        .enumerate()
        .filter_map(|(order, rule)| rule(input, config).map(|d| (order, d)))
        .collect();
    if ranked.is_empty() {
        ranked.push((usize::MAX, healthy(input)));
    }
    // Most severe first; equal severity keeps rule order (memory before docker before disk...).
    ranked.sort_by(|a, b| b.1.severity.cmp(&a.1.severity).then(a.0.cmp(&b.0)));

    let findings: Vec<_> = ranked.into_iter().map(|(_, d)| d).collect();
    let headline = findings[0].clone();
    let status = match headline.severity {
        Severity::Critical | Severity::High => Status::Problem,
        Severity::Warning => Status::Attention,
        Severity::Info => Status::Healthy,
    };
    DiagnosisReport {
        generated_at_ms: now_ms,
        status,
        headline,
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::developers::workloads::{Workload, detect_workloads};
    use crate::diagnosis::model::Risk;
    use crate::docker::{Container, DockerStatus};
    use crate::monitoring::disks::{DiskReport, DiskThresholds, build_report};
    use crate::monitoring::growth::GrowthFinding;
    use crate::monitoring::metrics::{DiskMetrics, MemoryMetrics, disk_metrics, memory_metrics};
    use crate::monitoring::pressure::{PressureEngine, PressureInputs, PressureReport};
    use crate::processes::model::{Category, ProcessInfo, sample};
    use crate::storage::safety::{EntryKind, Safety};
    use crate::storage::scanner::ScanEntry;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn gib(x: f64) -> u64 {
        (x * GIB as f64) as u64
    }

    struct World {
        memory: MemoryMetrics,
        pressure: PressureReport,
        workloads: Vec<Workload>,
        growth: Vec<GrowthFinding>,
        disks: Vec<DiskMetrics>,
        health: Vec<DiskReport>,
        docker: Option<DockerStatus>,
        storage: Option<Vec<ScanEntry>>,
        telemetry: Option<crate::monitoring::telemetry::MemoryTelemetry>,
    }

    impl World {
        /// 16 GB machine at `percent` RAM usage, one healthy 500 GB C: drive.
        fn new(percent: f32, procs: Vec<ProcessInfo>) -> Self {
            let total = 16 * GIB;
            let used = (total as f64 * percent as f64 / 100.0) as u64;
            let memory = memory_metrics(total, total - used);
            let pressure = PressureEngine::default().update(
                PressureInputs {
                    percent: memory.percent,
                    available_bytes: memory.available_bytes,
                    total_bytes: total,
                    commit_percent: None,
                    pagefile_percent: None,
                },
                0.0,
            );
            let mut w = World {
                memory,
                pressure,
                workloads: detect_workloads(&procs),
                growth: vec![],
                disks: vec![],
                health: vec![],
                docker: None,
                storage: None,
                telemetry: None,
            };
            w.set_disk("C:\\", 500.0, 250.0);
            w
        }

        fn set_disk(&mut self, mount: &str, total_gb: f64, free_gb: f64) {
            let d = disk_metrics("d".into(), mount.into(), gib(total_gb), gib(free_gb));
            let r = build_report(&DiskThresholds::default(), &d, &[], 0.0);
            self.disks = vec![d];
            self.health = vec![r];
        }

        fn run(&self) -> DiagnosisReport {
            let input = Input {
                memory: &self.memory,
                pressure: &self.pressure,
                workloads: &self.workloads,
                growth: &self.growth,
                disks: &self.disks,
                disk_health: &self.health,
                docker: self.docker.as_ref(),
                storage: self.storage.as_deref(),
                telemetry: self.telemetry.as_ref(),
            };
            diagnose(&input, &Config::default(), 123)
        }
    }

    fn proc(pid: u32, name: &str, gb: f64, cat: Category, tags: &[&'static str]) -> ProcessInfo {
        let mut p = sample(pid, name, gib(gb), 1.0, cat);
        p.tags = tags.to_vec();
        p
    }

    fn entry(path: &str, gb: f64, kind: EntryKind, safety: Safety) -> ScanEntry {
        ScanEntry {
            path: path.into(),
            name: path.rsplit('\\').next().unwrap().into(),
            is_dir: true,
            size_bytes: gib(gb),
            file_count: 10,
            last_modified_secs: None,
            kind,
            kind_label: "x",
            safety,
            reason: "Reason.",
        }
    }

    fn container(id: &str, name: &str, mem_mb: u64, cpu: f32) -> Container {
        Container {
            id: id.into(),
            name: name.into(),
            image: "img".into(),
            status: "Up".into(),
            running: true,
            memory_bytes: Some(mem_mb * 1024 * 1024),
            cpu_percent: Some(cpu),
        }
    }

    fn docker_ok(containers: Vec<Container>) -> DockerStatus {
        DockerStatus {
            available: true,
            error: None,
            total_memory_bytes: 0,
            containers,
        }
    }

    fn rule<'a>(
        r: &'a DiagnosisReport,
        id: &str,
    ) -> Option<&'a crate::diagnosis::model::Diagnosis> {
        r.findings.iter().find(|d| d.rule_id == id)
    }

    // --- spec scenarios -----------------------------------------------------------------

    #[test]
    fn spec_example_96_percent_with_gradle_and_docker_explains_why() {
        let w = World::new(
            96.0,
            vec![
                proc(1, "java.exe", 4.1, Category::Java, &["gradle", "android"]),
                proc(2, "vmmemWSL", 4.8, Category::Docker, &[]),
                proc(3, "chrome.exe", 0.3, Category::Browser, &[]),
            ],
        );
        let r = w.run();
        assert_eq!(r.status, Status::Problem);
        assert_eq!(r.headline.rule_id, "memory-pressure");
        assert_eq!(r.headline.severity, Severity::Critical);
        let s = &r.headline.summary;
        assert!(s.starts_with("RAM is at 96%."), "{s}");
        assert!(
            s.contains("Docker/WSL2") && s.contains("Android Gradle build"),
            "{s}"
        );
        assert!(s.contains("approximately 8.9 GB"), "{s}");
        assert!(s.contains("primary contributors"), "{s}");
        assert_eq!(
            r.headline.contributors.len(),
            3.min(r.headline.contributors.len())
        );
        assert!(
            r.headline.confidence > 0.8 && r.headline.confidence <= 0.95,
            "{}",
            r.headline.confidence
        );
        assert_eq!(r.headline.primary_cause.as_deref(), Some("Docker/WSL2"));
        let pcts: Vec<u32> = r
            .headline
            .contributors
            .iter()
            .map(|c| c.percent.round() as u32)
            .collect();
        assert_eq!(pcts[0], 30); // 4.8 / 16
        assert_eq!(pcts[1], 26); // 4.1 / 16
    }

    #[test]
    fn healthy_machine_gets_a_healthy_report() {
        let r = World::new(
            40.0,
            vec![proc(1, "chrome.exe", 1.0, Category::Browser, &[])],
        )
        .run();
        assert_eq!(r.status, Status::Healthy);
        assert_eq!(r.headline.rule_id, "healthy");
        assert_eq!(r.findings.len(), 1);
        assert!(r.headline.summary.contains("RAM is at 40%"));
    }

    fn telemetry_for(
        w: &World,
        commit: f32,
        faults: f32,
        disk: f32,
    ) -> crate::monitoring::telemetry::MemoryTelemetry {
        use crate::monitoring::telemetry::{TelemetryEngine, TelemetryInputs};
        use crate::windows::perf::PerfSample;
        let perf = PerfSample {
            hard_faults_per_sec: Some(faults),
            pagefile_usage_percent: Some(80.0),
            disk_active_percent: Some(disk),
        };
        let commit_info = crate::windows::memory::CommitInfo {
            commit_total: (commit as f64 / 100.0 * (24 * GIB) as f64) as u64,
            commit_limit: 24 * GIB,
            physical_total: 16 * GIB,
        };
        TelemetryEngine::default().update(TelemetryInputs {
            total_bytes: 16 * GIB,
            available_bytes: w.memory.available_bytes,
            commit: Some(commit_info),
            perf,
            pressure: &w.pressure,
        })
    }

    #[test]
    fn pressure_explains_paging_mechanism_when_telemetry_shows_it() {
        let mut w = World::new(96.0, vec![]);
        w.telemetry = Some(telemetry_for(&w, 98.0, 900.0, 100.0));
        let d = w.run().headline;
        assert_eq!(d.rule_id, "memory-pressure");
        assert!(d.summary.contains("heavy paging"), "{}", d.summary);
        assert!(d.evidence.iter().any(|e| e.contains("hard faults")));
        assert!(!d.summary.contains("definitely"));
    }

    #[test]
    fn pressure_without_telemetry_keeps_plain_wording() {
        let w = World::new(96.0, vec![]);
        let d = w.run().headline;
        assert!(d.summary.starts_with("RAM is at 96%."), "{}", d.summary);
    }

    #[test]
    fn unconcentrated_pressure_says_spread_out() {
        // Three named workloads (6 GB, 48% of used RAM) plus lots of unrelated processes.
        let mut procs = vec![
            proc(1, "vmmem", 2.0, Category::Docker, &[]),
            proc(2, "chrome.exe", 2.0, Category::Browser, &[]),
            proc(3, "Code.exe", 2.0, Category::Ide, &[]),
        ];
        procs.extend((10..17).map(|i| proc(i, "random.exe", 0.9, Category::Other, &[])));
        let r = World::new(78.0, procs).run();
        let d = &r.headline;
        assert_eq!(d.rule_id, "memory-pressure");
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(r.status, Status::Attention);
        assert!(
            d.summary.contains("spread across many processes"),
            "{}",
            d.summary
        );
    }

    #[test]
    fn contributors_are_capped_at_three_and_ignore_tiny_workloads() {
        let r = World::new(
            90.0,
            vec![
                proc(1, "vmmem", 3.0, Category::Docker, &[]),
                proc(2, "chrome.exe", 2.5, Category::Browser, &[]),
                proc(3, "Code.exe", 2.0, Category::Ide, &[]),
                proc(4, "node.exe", 1.5, Category::Node, &[]),
                proc(5, "adb.exe", 0.1, Category::Android, &[]),
            ],
        )
        .run();
        let c = &r.headline.contributors;
        assert_eq!(c.len(), 3);
        assert!(c.iter().all(|x| x.percent >= 3.0));
    }

    #[test]
    fn pressure_with_no_identifiable_workloads_is_low_confidence() {
        let r = World::new(88.0, vec![proc(1, "random.exe", 6.0, Category::Other, &[])]).run();
        assert_eq!(r.headline.rule_id, "memory-pressure");
        assert!(r.headline.confidence <= 0.45);
        assert!(r.headline.contributors.is_empty());
    }

    // --- docker -------------------------------------------------------------------------

    #[test]
    fn docker_rule_needs_both_size_and_ram_pressure() {
        let big = || vec![proc(1, "vmmemWSL", 5.0, Category::Docker, &[])];
        assert!(rule(&World::new(90.0, big()).run(), "docker-wsl").is_some());
        assert!(
            rule(&World::new(80.0, big()).run(), "docker-wsl").is_none(),
            "RAM not above 85%"
        );
        assert!(
            rule(
                &World::new(90.0, vec![proc(1, "vmmem", 3.0, Category::Docker, &[])]).run(),
                "docker-wsl"
            )
            .is_none(),
            "Docker not above 4 GB"
        );
    }

    #[test]
    fn idle_containers_get_stop_actions_busy_ones_do_not() {
        let mut w = World::new(90.0, vec![proc(1, "vmmemWSL", 5.0, Category::Docker, &[])]);
        w.docker = Some(docker_ok(vec![
            container("aaa", "redis-idle", 900, 0.2),
            container("bbb", "api-busy", 1200, 55.0),
        ]));
        let r = w.run();
        let d = rule(&r, "docker-wsl").unwrap();
        let stops: Vec<_> = d
            .recommendations
            .iter()
            .filter_map(|r| r.action.as_ref())
            .filter(|a| a.action_id == "docker.stop_container")
            .collect();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].params["container"], "redis-idle");
        assert!(d.recommendations.iter().all(|r| r.id != "docker-stop-bbb"));
        // Containers are running, so shutting down the whole VM is not offered.
        assert!(d.recommendations.iter().all(|r| r.id != "wsl-shutdown"));
    }

    #[test]
    fn wsl_vm_holding_memory_with_no_containers_offers_shutdown() {
        let mut w = World::new(90.0, vec![proc(1, "vmmemWSL", 5.0, Category::Docker, &[])]);
        w.docker = Some(docker_ok(vec![]));
        let r = w.run();
        let rec = rule(&r, "docker-wsl")
            .unwrap()
            .recommendations
            .iter()
            .find(|r| r.id == "wsl-shutdown")
            .unwrap();
        assert_eq!(rec.action.as_ref().unwrap().action_id, "wsl.shutdown");
        assert_eq!(rec.risk, Risk::Medium);
        assert!(rec.expected_benefit.contains("5.0 GB"));
    }

    #[test]
    fn docker_unavailable_still_diagnoses_and_says_why_details_are_missing() {
        let mut w = World::new(90.0, vec![proc(1, "vmmemWSL", 5.0, Category::Docker, &[])]);
        w.docker = Some(DockerStatus {
            available: false,
            error: Some("daemon not running".into()),
            containers: vec![],
            total_memory_bytes: 0,
        });
        let r = w.run();
        let d = rule(&r, "docker-wsl").unwrap();
        assert!(d.evidence.iter().any(|e| e.contains("daemon not running")));
        assert!(d.recommendations.iter().any(|r| r.id == "wsl-memory-limit"
            && r.action.is_none()
            && r.manual_steps.as_deref().unwrap().contains(".wslconfig")));
    }

    // --- android / node -------------------------------------------------------------------

    #[test]
    fn android_gradle_over_3gb_offers_stop_daemons() {
        let w = World::new(
            88.0,
            vec![proc(
                1,
                "java.exe",
                3.5,
                Category::Java,
                &["gradle", "android"],
            )],
        );
        let r = w.run();
        let d = rule(&r, "android-gradle").unwrap();
        assert!(d.title.starts_with("Android Gradle build"));
        assert_eq!(
            d.recommendations[0].action.as_ref().unwrap().action_id,
            "gradle.stop"
        );
        assert_eq!(d.severity, Severity::High);
    }

    #[test]
    fn plain_gradle_is_not_called_android() {
        let r = World::new(
            88.0,
            vec![proc(1, "java.exe", 3.5, Category::Java, &["gradle"])],
        )
        .run();
        let d = rule(&r, "android-gradle").unwrap();
        assert!(d.title.starts_with("Gradle daemons"));
        assert!(!d.title.contains("Android"));
    }

    #[test]
    fn gradle_rule_conditions() {
        let small = World::new(
            88.0,
            vec![proc(
                1,
                "java.exe",
                2.0,
                Category::Java,
                &["gradle", "android"],
            )],
        )
        .run();
        assert!(rule(&small, "android-gradle").is_none());
        let calm = World::new(
            50.0,
            vec![proc(
                1,
                "java.exe",
                6.0,
                Category::Java,
                &["gradle", "android"],
            )],
        )
        .run();
        assert!(rule(&calm, "android-gradle").is_none());
        let no_gradle = World::new(88.0, vec![proc(1, "java.exe", 6.0, Category::Java, &[])]).run();
        assert!(rule(&no_gradle, "android-gradle").is_none());
    }

    #[test]
    fn next_dev_servers_named_specifically() {
        let w = World::new(
            80.0,
            vec![
                proc(1, "node.exe", 1.5, Category::Node, &["nextjs"]),
                proc(2, "node.exe", 1.2, Category::Node, &["nextjs"]),
            ],
        );
        let r = w.run();
        let d = rule(&r, "node-dev").unwrap();
        assert!(d.title.starts_with("Next.js dev servers"));
        assert_eq!(d.primary_cause.as_deref(), Some("Next.js dev server"));
    }

    // --- growth ---------------------------------------------------------------------------

    fn growth(pid: u32) -> GrowthFinding {
        GrowthFinding {
            pid,
            name: "node.exe".into(),
            start_bytes: gib(1.2),
            current_bytes: gib(3.8),
            growth_bytes: gib(2.6),
            growth_percent: 216.0,
            bytes_per_min: 90.0e6,
            window_minutes: 30.0,
            confidence: 0.91,
            summary: "node.exe (PID 7) grew from 1.2 GB to 3.8 GB over 30 min. Possible memory growth/leak.".into(),
        }
    }

    #[test]
    fn growth_is_reported_as_possible_with_confidence_and_a_caveat() {
        let mut w = World::new(60.0, vec![]);
        w.growth = vec![growth(7)];
        let r = w.run();
        let d = rule(&r, "memory-growth").unwrap();
        assert_eq!(d.title, "Possible memory growth/leak");
        assert!(d.summary.contains("Confidence 91%"), "{}", d.summary);
        assert!(d.summary.contains("only a leak if it never levels off"));
        assert_eq!(d.severity, Severity::Warning);
        let a = d.recommendations[0].action.as_ref().unwrap();
        assert_eq!(
            (a.action_id.as_str(), a.params["pid"].as_u64()),
            ("process.terminate", Some(7))
        );
        assert_eq!(d.recommendations[0].risk, Risk::Medium);
    }

    #[test]
    fn growth_escalates_when_ram_is_also_high() {
        let mut w = World::new(90.0, vec![]);
        w.growth = vec![growth(7)];
        assert_eq!(
            rule(&w.run(), "memory-growth").unwrap().severity,
            Severity::High
        );
    }

    // --- disk -----------------------------------------------------------------------------

    #[test]
    fn low_c_drive_is_flagged_with_numbers() {
        let mut w = World::new(40.0, vec![]);
        w.set_disk("C:\\", 275.0, 6.0);
        let r = w.run();
        assert_eq!(r.headline.rule_id, "low-disk");
        assert!(r.headline.severity >= Severity::High);
        assert!(r.headline.title.contains("C:\\"));
        assert!(
            r.headline.summary.contains("6.0 GB free"),
            "{}",
            r.headline.summary
        );
    }

    #[test]
    fn under_10gb_free_is_flagged_even_if_percentage_looks_fine() {
        let mut w = World::new(40.0, vec![]);
        w.set_disk("D:\\", 2000.0, 9.0); // 0.45% => critical anyway
        assert_eq!(w.run().headline.rule_id, "low-disk");
        let mut ok = World::new(40.0, vec![]);
        ok.set_disk("D:\\", 2000.0, 600.0);
        assert_eq!(ok.run().headline.rule_id, "healthy");
    }

    #[test]
    fn disk_prediction_text_is_included() {
        let mut w = World::new(40.0, vec![]);
        w.set_disk("C:\\", 200.0, 15.0);
        w.health[0].hours_to_full = Some(1.5);
        let s = w.run().headline.summary;
        assert!(s.contains("full in about 1.5 hours"), "{s}");
    }

    #[test]
    fn disk_recommendations_follow_safety_classes() {
        let mut w = World::new(40.0, vec![]);
        w.set_disk("C:\\", 275.0, 6.0);
        w.storage = Some(vec![
            entry(
                "C:\\Users\\a\\AppData\\Local\\Temp",
                3.0,
                EntryKind::TempCache,
                Safety::SafeToClean,
            ),
            entry(
                "C:\\Users\\a\\proj\\.next",
                2.0,
                EntryKind::BuildOutput,
                Safety::SafeToReview,
            ),
            entry(
                "C:\\Users\\a\\proj\\node_modules",
                1.5,
                EntryKind::NodeModules,
                Safety::RequiresReview,
            ),
            entry(
                "C:\\Users\\a\\AppData\\Local\\Android\\Sdk",
                20.0,
                EntryKind::AndroidSdk,
                Safety::DoNotTouch,
            ),
            entry(
                "D:\\other\\cache",
                9.0,
                EntryKind::PackageCache,
                Safety::SafeToClean,
            ),
        ]);
        let r = w.run();
        let d = rule(&r, "low-disk").unwrap();
        let by = |needle: &str| {
            d.recommendations
                .iter()
                .find(|r| r.id.contains(needle))
                .unwrap()
        };

        let temp = by("Temp");
        assert_eq!(
            (temp.risk, temp.action.as_ref().unwrap().action_id.as_str()),
            (Risk::Low, "cleanup.path")
        );
        assert_eq!(by(".next").risk, Risk::Medium);
        let nm = by("node_modules");
        assert!(
            nm.action.is_none() && nm.manual_steps.is_some(),
            "needs manual review"
        );
        assert!(
            d.recommendations.iter().all(|r| !r.id.contains("Sdk")),
            "DO_NOT_TOUCH never offered"
        );
        assert!(
            d.recommendations.iter().all(|r| !r.id.contains("D:\\")),
            "other drives are not offered for C:"
        );
        assert!(d.summary.contains("could recover about"), "{}", d.summary);
    }

    #[test]
    fn no_scan_yet_asks_for_one() {
        let mut w = World::new(40.0, vec![]);
        w.set_disk("C:\\", 275.0, 6.0);
        let d = w.run().headline;
        assert_eq!(d.recommendations[0].id, "storage-scan");
        assert!(
            d.recommendations[0]
                .manual_steps
                .as_deref()
                .unwrap()
                .contains("Scan")
        );
    }

    #[test]
    fn disposable_caches_are_an_info_finding_when_big_enough() {
        let mut w = World::new(40.0, vec![]);
        w.storage = Some(vec![
            entry(
                "C:\\t\\Temp",
                1.5,
                EntryKind::TempCache,
                Safety::SafeToClean,
            ),
            entry(
                "C:\\t\\npm-cache",
                1.5,
                EntryKind::PackageCache,
                Safety::SafeToClean,
            ),
        ]);
        let r = w.run();
        let d = rule(&r, "cleanable-caches").unwrap();
        assert_eq!(d.severity, Severity::Info);
        assert!(d.recommendations.iter().all(|r| r.risk == Risk::Low));
        assert_eq!(
            r.status,
            Status::Healthy,
            "info findings do not make the system 'unhealthy'"
        );

        w.storage = Some(vec![entry(
            "C:\\t\\Temp",
            0.5,
            EntryKind::TempCache,
            Safety::SafeToClean,
        )]);
        assert!(rule(&w.run(), "cleanable-caches").is_none());
    }

    // --- ranking & serialization --------------------------------------------------------------

    #[test]
    fn most_severe_finding_leads_and_ties_keep_rule_order() {
        let mut w = World::new(
            78.0,
            vec![proc(1, "chrome.exe", 5.0, Category::Browser, &[])],
        );
        w.set_disk("C:\\", 275.0, 4.0);
        let r = w.run();
        assert_eq!(r.headline.rule_id, "low-disk");
        assert_eq!(r.findings[0].rule_id, "low-disk");
        assert_eq!(r.findings[1].rule_id, "memory-pressure");
        let sev: Vec<_> = r.findings.iter().map(|d| d.severity).collect();
        assert!(sev.windows(2).all(|p| p[0] >= p[1]));
    }

    #[test]
    fn report_serializes_with_spec_shape() {
        let r = World::new(
            96.0,
            vec![
                proc(1, "java.exe", 4.1, Category::Java, &["gradle", "android"]),
                proc(2, "vmmem", 4.8, Category::Docker, &[]),
            ],
        )
        .run();
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["headline"]["severity"], "critical");
        assert_eq!(v["status"], "problem");
        assert!(v["headline"]["primaryCause"].is_string());
        assert!(v["headline"]["contributors"][0]["memoryBytes"].is_number());
        assert!(v["headline"]["contributors"][0]["percent"].is_number());
        assert!(v["generatedAtMs"] == 123);
        assert!(v["findings"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn every_recommendation_has_problem_explanation_benefit_risk_and_a_way_to_act() {
        let mut w = World::new(
            92.5,
            vec![
                proc(1, "vmmemWSL", 5.0, Category::Docker, &[]),
                proc(2, "java.exe", 3.5, Category::Java, &["gradle", "android"]),
                proc(3, "node.exe", 2.5, Category::Node, &["nextjs"]),
            ],
        );
        w.growth = vec![growth(9)];
        w.set_disk("C:\\", 275.0, 6.0);
        w.docker = Some(docker_ok(vec![container("a", "idle", 500, 0.1)]));
        let r = w.run();
        let mut n = 0;
        for d in &r.findings {
            for rec in &d.recommendations {
                n += 1;
                assert!(
                    !rec.problem.is_empty()
                        && !rec.explanation.is_empty()
                        && !rec.expected_benefit.is_empty(),
                    "{}",
                    rec.id
                );
                assert!(
                    rec.action.is_some() || rec.manual_steps.is_some(),
                    "{} has no way to act",
                    rec.id
                );
            }
        }
        assert!(n >= 6, "{n}");
    }
}

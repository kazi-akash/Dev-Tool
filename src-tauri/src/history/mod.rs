//! Background history recorder. Samples arrive from the monitor, are buffered, and written in
//! batches; rollups and event pruning run on a timer. Anything that fails is dropped quietly: the
//! live app never depends on history.

pub mod aggregate;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::database::{Database, StoredAction, StoredDiagnosis};
use crate::developers::workloads::Workload;
use crate::monitoring::metrics::SystemSnapshot;
use crate::monitoring::incident::IncidentSummary;
use aggregate::{DiskPoint, Row, TopPoint};

const FLUSH_EVERY: Duration = Duration::from_secs(10);
const ROLLUP_EVERY: Duration = Duration::from_secs(60);
/// Bound the in-memory buffer if the database stalls.
const MAX_BUFFER: usize = 2_000;

enum Msg {
    Sample(Row),
    Diagnosis(StoredDiagnosis),
    Action(StoredAction),
    Incident(IncidentSummary),
}

#[derive(Clone)]
pub struct HistoryHandle {
    tx: UnboundedSender<Msg>,
    db: Database,
}

impl HistoryHandle {
    pub fn sample(&self, row: Row) {
        let _ = self.tx.send(Msg::Sample(row));
    }

    pub fn diagnosis(&self, d: StoredDiagnosis) {
        let _ = self.tx.send(Msg::Diagnosis(d));
    }

    pub fn incident(&self, s: IncidentSummary) {
        let _ = self.tx.send(Msg::Incident(s));
    }

    pub fn action(&self, a: StoredAction) {
        let _ = self.tx.send(Msg::Action(a));
    }

    pub fn db(&self) -> &Database {
        &self.db
    }
}

fn text<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Row for the diagnosis history: only the headline, never the full report.
pub fn stored_diagnosis(r: &crate::diagnosis::model::DiagnosisReport) -> StoredDiagnosis {
    let h = &r.headline;
    StoredDiagnosis {
        id: 0,
        ts_ms: r.generated_at_ms as i64,
        status: text(&r.status),
        severity: text(&h.severity),
        rule_id: h.rule_id.clone(),
        title: h.title.clone(),
        summary: h.summary.clone(),
        primary_cause: h.primary_cause.clone(),
        confidence: h.confidence as f64,
    }
}

pub fn stored_action(a: &crate::actions::engine::ActionRecord) -> StoredAction {
    StoredAction {
        id: 0,
        ts_ms: a.at_ms as i64,
        action_id: a.action_id.clone(),
        name: a.name.clone(),
        risk: text(&a.risk),
        automated: a.automated,
        success: a.success,
        summary: a.summary.clone(),
        freed_bytes: a.freed_bytes.map(|b| b as i64),
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// One history sample from a live snapshot. Keeps only what the History page draws: totals, drive
/// free space, and the five largest workloads (names, not processes).
pub fn sample_row(snapshot: &SystemSnapshot, workloads: &[Workload]) -> Row {
    Row::raw(
        snapshot.timestamp_ms as i64,
        snapshot.memory.percent as f64,
        snapshot.cpu_percent as f64,
        snapshot.memory.used_bytes,
        snapshot.memory.total_bytes,
        snapshot
            .disks
            .iter()
            .map(|d| DiskPoint {
                mount: d.mount_point.clone(),
                free_bytes: d.free_bytes,
                total_bytes: d.total_bytes,
            })
            .collect(),
        workloads
            .iter()
            .take(5)
            .map(|w| TopPoint {
                name: w.name.clone(),
                memory_bytes: w.memory_bytes,
            })
            .collect(),
    )
}

/// Start the recorder on the Tauri async runtime.
pub fn start(db: Database) -> HistoryHandle {
    let (tx, mut rx) = unbounded_channel::<Msg>();
    let handle = HistoryHandle { tx, db: db.clone() };

    tauri::async_runtime::spawn(async move {
        let mut buffer: Vec<Row> = Vec::new();
        let mut flush = tokio::time::interval(FLUSH_EVERY);
        let mut roll = tokio::time::interval(ROLLUP_EVERY);
        loop {
            tokio::select! {
                msg = rx.recv() => match msg {
                    Some(Msg::Sample(r)) => {
                        if buffer.len() >= MAX_BUFFER {
                            buffer.remove(0);
                        }
                        buffer.push(r);
                    }
                    Some(Msg::Diagnosis(d)) => { let _ = db.record_diagnosis(&d).await; }
                    Some(Msg::Action(a)) => { let _ = db.insert_action(&a).await; }
                    Some(Msg::Incident(s)) => { let _ = db.insert_incident(&s).await; }
                    None => {
                        let _ = db.insert_raw(&buffer).await;
                        return;
                    }
                },
                _ = flush.tick() => {
                    if db.insert_raw(&buffer).await.is_ok() {
                        buffer.clear();
                    }
                }
                _ = roll.tick() => {
                    let _ = db.rollup(now_ms()).await;
                }
            }
        }
    });
    handle
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::developers::workloads::detect_workloads;
    use crate::monitoring::disks::{DiskThresholds, build_report};
    use crate::monitoring::metrics::{disk_metrics, memory_metrics};
    use crate::processes::model::{Category, sample};

    const GIB: u64 = 1024 * 1024 * 1024;

    fn snap() -> SystemSnapshot {
        let d = disk_metrics("Win".into(), "C:\\".into(), 100 * GIB, 40 * GIB);
        SystemSnapshot {
            timestamp_ms: 1_700_000_000_000,
            memory: memory_metrics(16 * GIB, 4 * GIB),
            cpu_percent: 37.5,
            disk_health: vec![build_report(&DiskThresholds::default(), &d, &[], 0.0)],
            disks: vec![d],
            pressure: None,
            telemetry: None,
            uptime_secs: 1,
            process_count: 1,
        }
    }

    #[test]
    fn sample_row_keeps_totals_drives_and_top_five_workloads() {
        let mut procs = vec![
            sample(1, "vmmem", 5 * GIB, 0.0, Category::Docker),
            sample(2, "chrome.exe", 2 * GIB, 0.0, Category::Browser),
            sample(3, "Code.exe", GIB, 0.0, Category::Ide),
            sample(4, "node.exe", GIB / 2, 0.0, Category::Node),
            sample(5, "java.exe", GIB / 4, 0.0, Category::Java),
            sample(6, "mysqld.exe", GIB / 8, 0.0, Category::Database),
        ];
        procs[1].memory_bytes += 1;
        let r = sample_row(&snap(), &detect_workloads(&procs));
        assert_eq!(r.ts_ms, 1_700_000_000_000);
        assert_eq!(r.samples, 1);
        assert_eq!(r.ram_avg, 75.0);
        assert_eq!(r.cpu_avg, 37.5);
        assert_eq!(r.ram_total, 16 * GIB);
        assert_eq!(r.disks[0].mount, "C:\\");
        assert_eq!(r.disks[0].free_bytes, 40 * GIB);
        assert_eq!(r.top.len(), 5);
        assert_eq!(r.top[0].name, "Docker / WSL2");
    }

    #[tokio::test]
    async fn recorder_writes_samples_and_events_and_survives_a_closed_channel() {
        let db = Database::memory().await.unwrap();
        // `start` uses tauri's runtime, which is the same tokio runtime family; drive it with a manual loop here.
        let (tx, mut rx) = unbounded_channel::<Msg>();
        let handle = HistoryHandle { tx, db: db.clone() };
        handle.sample(Row::raw(1000, 50.0, 5.0, 1, 2, vec![], vec![]));
        handle.action(StoredAction {
            id: 0,
            ts_ms: 1,
            action_id: "x".into(),
            name: "n".into(),
            risk: "LOW".into(),
            automated: false,
            success: true,
            summary: "s".into(),
            freed_bytes: None,
        });
        drop(handle);
        let mut buf = vec![];
        while let Some(m) = rx.recv().await {
            match m {
                Msg::Sample(r) => buf.push(r),
                Msg::Action(a) => db.insert_action(&a).await.unwrap(),
                Msg::Diagnosis(_) | Msg::Incident(_) => {}
            }
        }
        db.insert_raw(&buf).await.unwrap();
        assert_eq!(db.counts().await.unwrap().0, 1);
        assert_eq!(db.recent_actions(5).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn sending_after_the_recorder_stopped_is_harmless() {
        let db = Database::memory().await.unwrap();
        let (tx, rx) = unbounded_channel::<Msg>();
        drop(rx);
        let h = HistoryHandle { tx, db };
        h.sample(Row::raw(1, 1.0, 1.0, 1, 1, vec![], vec![])); // must not panic
    }
}

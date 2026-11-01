//! SQLite repository (SQLx). Every function returns a plain `String` error: callers log and carry
//! on, because history is a convenience and must never take the app down.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row as _, SqlitePool};

use crate::monitoring::incident::IncidentSummary;
use crate::history::aggregate::{
    BUCKET_1H, BUCKET_1M, BUCKET_5M, DiskPoint, EVENTS_KEEP_MS, HOUR_KEEP_MS, MIN1_KEEP_MS,
    MIN5_KEEP_MS, RAW_KEEP_MS, Row, TopPoint, bucket_for_range, cutoff, merge,
};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredDiagnosis {
    pub id: i64,
    pub ts_ms: i64,
    pub status: String,
    pub severity: String,
    pub rule_id: String,
    pub title: String,
    pub summary: String,
    pub primary_cause: Option<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredAction {
    pub id: i64,
    pub ts_ms: i64,
    pub action_id: String,
    pub name: String,
    pub risk: String,
    pub automated: bool,
    pub success: bool,
    pub summary: String,
    pub freed_bytes: Option<i64>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RollupReport {
    pub raw_to_1m: usize,
    pub m1_to_5m: usize,
    pub m5_to_1h: usize,
}

impl Database {
    /// Open (creating if needed) the database file and apply migrations.
    pub async fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create data folder: {e}"))?;
        }
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(opts)
            .await
            .map_err(err)?;
        Self::migrate(pool).await
    }

    /// Private in-memory database (tests).
    #[cfg(test)]
    pub async fn memory() -> Result<Self, String> {
        use std::str::FromStr;
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").map_err(err)?;
        // Each in-memory connection is its own database, so keep exactly one.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(err)?;
        Self::migrate(pool).await
    }

    async fn migrate(pool: SqlitePool) -> Result<Self, String> {
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(err)?;
        Ok(Self { pool })
    }

    // --- metrics ---------------------------------------------------------------------------

    pub async fn insert_raw(&self, rows: &[Row]) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await.map_err(err)?;
        for r in rows {
            sqlx::query(
                "INSERT OR REPLACE INTO metrics_raw (ts_ms, ram_percent, cpu_percent, ram_used, ram_total, disks_json, top_json)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(r.ts_ms)
            .bind(r.ram_avg)
            .bind(r.cpu_avg)
            .bind(r.ram_used_avg as i64)
            .bind(r.ram_total as i64)
            .bind(serde_json::to_string(&r.disks).map_err(err)?)
            .bind(serde_json::to_string(&r.top).map_err(err)?)
            .execute(&mut *tx)
            .await
            .map_err(err)?;
        }
        tx.commit().await.map_err(err)
    }

    fn parse_disks(s: &str) -> Vec<DiskPoint> {
        serde_json::from_str(s).unwrap_or_default()
    }

    fn parse_top(s: &str) -> Vec<TopPoint> {
        serde_json::from_str(s).unwrap_or_default()
    }

    async fn raw_before(&self, ts: i64) -> Result<Vec<Row>, String> {
        let rows = sqlx::query("SELECT * FROM metrics_raw WHERE ts_ms < ? ORDER BY ts_ms")
            .bind(ts)
            .fetch_all(&self.pool)
            .await
            .map_err(err)?;
        Ok(rows.iter().map(raw_row).collect())
    }

    async fn agg_before(&self, bucket: i64, ts: i64) -> Result<Vec<Row>, String> {
        let rows = sqlx::query(
            "SELECT * FROM metrics_agg WHERE bucket_secs = ? AND ts_ms < ? ORDER BY ts_ms",
        )
        .bind(bucket)
        .bind(ts)
        .fetch_all(&self.pool)
        .await
        .map_err(err)?;
        Ok(rows.iter().map(agg_row).collect())
    }

    /// Roll old rows into coarser buckets (raw -> 1 min -> 5 min -> 1 h) and drop what is expired.
    /// Each step runs in one transaction, so a failure cannot lose or duplicate data.
    pub async fn rollup(&self, now_ms: i64) -> Result<RollupReport, String> {
        let mut report = RollupReport::default();

        let c = cutoff(now_ms, RAW_KEEP_MS, BUCKET_1M);
        let raw = self.raw_before(c).await?;
        if !raw.is_empty() {
            let merged = merge(&raw, BUCKET_1M);
            let mut tx = self.pool.begin().await.map_err(err)?;
            for m in &merged {
                insert_agg(&mut tx, BUCKET_1M, m).await?;
            }
            sqlx::query("DELETE FROM metrics_raw WHERE ts_ms < ?")
                .bind(c)
                .execute(&mut *tx)
                .await
                .map_err(err)?;
            tx.commit().await.map_err(err)?;
            report.raw_to_1m = merged.len();
        }

        for (from, to, keep, slot) in [
            (BUCKET_1M, BUCKET_5M, MIN1_KEEP_MS, 0),
            (BUCKET_5M, BUCKET_1H, MIN5_KEEP_MS, 1),
        ] {
            let c = cutoff(now_ms, keep, to);
            let rows = self.agg_before(from, c).await?;
            if rows.is_empty() {
                continue;
            }
            let merged = merge(&rows, to);
            let mut tx = self.pool.begin().await.map_err(err)?;
            for m in &merged {
                insert_agg(&mut tx, to, m).await?;
            }
            sqlx::query("DELETE FROM metrics_agg WHERE bucket_secs = ? AND ts_ms < ?")
                .bind(from)
                .bind(c)
                .execute(&mut *tx)
                .await
                .map_err(err)?;
            tx.commit().await.map_err(err)?;
            if slot == 0 {
                report.m1_to_5m = merged.len();
            } else {
                report.m5_to_1h = merged.len();
            }
        }

        sqlx::query("DELETE FROM metrics_agg WHERE bucket_secs = ? AND ts_ms < ?")
            .bind(BUCKET_1H)
            .bind(now_ms - HOUR_KEEP_MS)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        let events_cut = now_ms - EVENTS_KEEP_MS;
        sqlx::query("DELETE FROM diagnoses WHERE ts_ms < ?")
            .bind(events_cut)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        sqlx::query("DELETE FROM incidents WHERE start_ms < ?")
            .bind(events_cut)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        sqlx::query("DELETE FROM action_log WHERE ts_ms < ?")
            .bind(events_cut)
            .execute(&self.pool)
            .await
            .map_err(err)?;
        Ok(report)
    }

    /// Points for a chart covering the last `range_ms`, at a resolution suited to the range.
    pub async fn query_history(&self, now_ms: i64, range_ms: i64) -> Result<Vec<Row>, String> {
        let from = now_ms - range_ms;
        let raw = sqlx::query("SELECT * FROM metrics_raw WHERE ts_ms >= ? ORDER BY ts_ms")
            .bind(from)
            .fetch_all(&self.pool)
            .await
            .map_err(err)?;
        let agg = sqlx::query("SELECT * FROM metrics_agg WHERE ts_ms >= ? ORDER BY ts_ms")
            .bind(from)
            .fetch_all(&self.pool)
            .await
            .map_err(err)?;
        let mut rows: Vec<Row> = agg
            .iter()
            .map(agg_row)
            .chain(raw.iter().map(raw_row))
            .collect();
        match bucket_for_range(range_ms) {
            Some(b) => Ok(merge(&rows, b)),
            None => {
                rows.sort_by_key(|r| r.ts_ms);
                Ok(rows)
            }
        }
    }

    #[cfg(test)]
    pub async fn counts(&self) -> Result<(i64, i64), String> {
        let raw: i64 = sqlx::query("SELECT COUNT(*) AS n FROM metrics_raw")
            .fetch_one(&self.pool)
            .await
            .map_err(err)?
            .get("n");
        let agg: i64 = sqlx::query("SELECT COUNT(*) AS n FROM metrics_agg")
            .fetch_one(&self.pool)
            .await
            .map_err(err)?
            .get("n");
        Ok((raw, agg))
    }

    // --- diagnoses -----------------------------------------------------------------------------

    /// Record a diagnosis unless it repeats the previous one (same rule, severity, status) within 30 minutes.
    /// Returns whether a row was written.
    pub async fn record_diagnosis(&self, d: &StoredDiagnosis) -> Result<bool, String> {
        if let Some(last) = self.recent_diagnoses(1).await?.first()
            && last.rule_id == d.rule_id
            && last.severity == d.severity
            && last.status == d.status
            && d.ts_ms - last.ts_ms < 30 * 60 * 1000
        {
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO diagnoses (ts_ms, status, severity, rule_id, title, summary, primary_cause, confidence)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(d.ts_ms)
        .bind(&d.status)
        .bind(&d.severity)
        .bind(&d.rule_id)
        .bind(&d.title)
        .bind(&d.summary)
        .bind(&d.primary_cause)
        .bind(d.confidence)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(true)
    }

    pub async fn recent_diagnoses(&self, limit: i64) -> Result<Vec<StoredDiagnosis>, String> {
        let rows = sqlx::query("SELECT * FROM diagnoses ORDER BY ts_ms DESC, id DESC LIMIT ?")
            .bind(limit.clamp(1, 500))
            .fetch_all(&self.pool)
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .map(|r| StoredDiagnosis {
                id: r.get("id"),
                ts_ms: r.get("ts_ms"),
                status: r.get("status"),
                severity: r.get("severity"),
                rule_id: r.get("rule_id"),
                title: r.get("title"),
                summary: r.get("summary"),
                primary_cause: r.get("primary_cause"),
                confidence: r.get("confidence"),
            })
            .collect())
    }


    // --- incidents -------------------------------------------------------------------------------

    /// Store an incident summary; returns its id. The full summary is kept as JSON, the columns
    /// exist for listing and pruning.
    pub async fn insert_incident(&self, s: &IncidentSummary) -> Result<i64, String> {
        let json = serde_json::to_string(s).map_err(err)?;
        let mechanism = serde_json::to_value(s.mechanism)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let res = sqlx::query(
            "INSERT INTO incidents (start_ms, end_ms, duration_secs, peak_ram_percent, mechanism, confidence, diagnosis, summary_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(s.start_ms as i64)
        .bind(s.end_ms as i64)
        .bind(s.duration_secs as i64)
        .bind(s.peak_ram_percent as f64)
        .bind(mechanism)
        .bind(s.confidence as f64)
        .bind(&s.diagnosis)
        .bind(json)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(res.last_insert_rowid())
    }

    /// Newest first. Rows whose JSON no longer parses (older schema) are skipped, not fatal.
    pub async fn recent_incidents(&self, limit: i64) -> Result<Vec<IncidentSummary>, String> {
        let rows = sqlx::query("SELECT id, summary_json FROM incidents ORDER BY start_ms DESC, id DESC LIMIT ?")
            .bind(limit.clamp(1, 200))
            .fetch_all(&self.pool)
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let mut s: IncidentSummary =
                    serde_json::from_str(&r.get::<String, _>("summary_json")).ok()?;
                s.id = r.get("id");
                Some(s)
            })
            .collect())
    }
    // --- actions ---------------------------------------------------------------------------------

    pub async fn insert_action(&self, a: &StoredAction) -> Result<(), String> {
        sqlx::query(
            "INSERT INTO action_log (ts_ms, action_id, name, risk, automated, success, summary, freed_bytes)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(a.ts_ms)
        .bind(&a.action_id)
        .bind(&a.name)
        .bind(&a.risk)
        .bind(a.automated)
        .bind(a.success)
        .bind(&a.summary)
        .bind(a.freed_bytes)
        .execute(&self.pool)
        .await
        .map_err(err)?;
        Ok(())
    }

    pub async fn recent_actions(&self, limit: i64) -> Result<Vec<StoredAction>, String> {
        let rows = sqlx::query("SELECT * FROM action_log ORDER BY ts_ms DESC, id DESC LIMIT ?")
            .bind(limit.clamp(1, 500))
            .fetch_all(&self.pool)
            .await
            .map_err(err)?;
        Ok(rows
            .iter()
            .map(|r| StoredAction {
                id: r.get("id"),
                ts_ms: r.get("ts_ms"),
                action_id: r.get("action_id"),
                name: r.get("name"),
                risk: r.get("risk"),
                automated: r.get("automated"),
                success: r.get("success"),
                summary: r.get("summary"),
                freed_bytes: r.get("freed_bytes"),
            })
            .collect())
    }
}

async fn insert_agg(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    bucket: i64,
    r: &Row,
) -> Result<(), String> {
    sqlx::query(
        "INSERT OR REPLACE INTO metrics_agg
         (bucket_secs, ts_ms, samples, ram_avg, ram_max, cpu_avg, cpu_max, ram_used_avg, ram_total, disks_json, top_json)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(bucket)
    .bind(r.ts_ms)
    .bind(r.samples as i64)
    .bind(r.ram_avg)
    .bind(r.ram_max)
    .bind(r.cpu_avg)
    .bind(r.cpu_max)
    .bind(r.ram_used_avg as i64)
    .bind(r.ram_total as i64)
    .bind(serde_json::to_string(&r.disks).map_err(err)?)
    .bind(serde_json::to_string(&r.top).map_err(err)?)
    .execute(&mut **tx)
    .await
    .map_err(err)?;
    Ok(())
}

fn raw_row(r: &sqlx::sqlite::SqliteRow) -> Row {
    let ram: f64 = r.get("ram_percent");
    let cpu: f64 = r.get("cpu_percent");
    Row::raw(
        r.get("ts_ms"),
        ram,
        cpu,
        r.get::<i64, _>("ram_used") as u64,
        r.get::<i64, _>("ram_total") as u64,
        Database::parse_disks(r.get::<String, _>("disks_json").as_str()),
        Database::parse_top(r.get::<String, _>("top_json").as_str()),
    )
}

fn agg_row(r: &sqlx::sqlite::SqliteRow) -> Row {
    Row {
        ts_ms: r.get("ts_ms"),
        samples: r.get::<i64, _>("samples") as u32,
        ram_avg: r.get("ram_avg"),
        ram_max: r.get("ram_max"),
        cpu_avg: r.get("cpu_avg"),
        cpu_max: r.get("cpu_max"),
        ram_used_avg: r.get::<i64, _>("ram_used_avg") as u64,
        ram_total: r.get::<i64, _>("ram_total") as u64,
        disks: Database::parse_disks(r.get::<String, _>("disks_json").as_str()),
        top: Database::parse_top(r.get::<String, _>("top_json").as_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;

    fn raw(ts: i64, ram: f64) -> Row {
        Row::raw(
            ts,
            ram,
            ram / 2.0,
            (ram * 10.0) as u64,
            16_000,
            vec![DiskPoint {
                mount: "C:\\".into(),
                free_bytes: 100,
                total_bytes: 1000,
            }],
            vec![TopPoint {
                name: "Docker / WSL2".into(),
                memory_bytes: 5,
            }],
        )
    }

    fn diag(ts: i64, rule: &str, sev: &str) -> StoredDiagnosis {
        StoredDiagnosis {
            id: 0,
            ts_ms: ts,
            status: "problem".into(),
            severity: sev.into(),
            rule_id: rule.into(),
            title: "t".into(),
            summary: "s".into(),
            primary_cause: Some("Docker/WSL2".into()),
            confidence: 0.9,
        }
    }

    #[tokio::test]
    async fn migrations_apply_and_start_empty() {
        let db = Database::memory().await.unwrap();
        assert_eq!(db.counts().await.unwrap(), (0, 0));
        assert!(db.recent_diagnoses(10).await.unwrap().is_empty());
        assert!(db.recent_actions(10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn raw_rows_round_trip_with_detail() {
        let db = Database::memory().await.unwrap();
        db.insert_raw(&[raw(1000, 50.0), raw(3000, 60.0)])
            .await
            .unwrap();
        let got = db.query_history(10_000, 60_000).await.unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[1].ram_avg, 60.0);
        assert_eq!(got[0].disks[0].mount, "C:\\");
        assert_eq!(got[0].top[0].name, "Docker / WSL2");
        // Same timestamp replaces instead of failing.
        db.insert_raw(&[raw(1000, 99.0)]).await.unwrap();
        assert_eq!(db.counts().await.unwrap().0, 2);
        db.insert_raw(&[]).await.unwrap();
    }

    #[tokio::test]
    async fn old_raw_rows_roll_into_minute_buckets_and_recent_ones_stay() {
        let db = Database::memory().await.unwrap();
        let now = 10 * HOUR;
        // 100 samples ending 2 hours ago (old) plus 30 recent ones.
        let old: Vec<Row> = (0..100)
            .map(|i| raw(now - 2 * HOUR + i * 2_000, 50.0 + (i % 10) as f64))
            .collect();
        let recent: Vec<Row> = (0..30)
            .map(|i| raw(now - 5 * MIN + i * 2_000, 70.0))
            .collect();
        db.insert_raw(&old).await.unwrap();
        db.insert_raw(&recent).await.unwrap();

        let r = db.rollup(now).await.unwrap();
        assert!(r.raw_to_1m >= 3 && r.raw_to_1m <= 5, "{r:?}");
        let (raw_left, agg) = db.counts().await.unwrap();
        assert_eq!(raw_left, 30, "recent raw samples are kept");
        assert_eq!(agg as usize, r.raw_to_1m);

        // Nothing is lost: total sample weight of the buckets equals what went in.
        let rows = db.query_history(now, 3 * HOUR).await.unwrap();
        let samples: u32 = rows.iter().map(|r| r.samples).sum();
        assert_eq!(samples, 130);
    }

    #[tokio::test]
    async fn rollup_is_idempotent() {
        let db = Database::memory().await.unwrap();
        let now = 5 * HOUR;
        let rows: Vec<Row> = (0..200)
            .map(|i| raw(now - 3 * HOUR + i * 2_000, 55.0))
            .collect();
        db.insert_raw(&rows).await.unwrap();
        db.rollup(now).await.unwrap();
        let before = db.counts().await.unwrap();
        let second = db.rollup(now).await.unwrap();
        assert_eq!(second, RollupReport::default());
        assert_eq!(db.counts().await.unwrap(), before);
    }

    #[tokio::test]
    async fn tiers_cascade_and_ancient_data_expires() {
        let db = Database::memory().await.unwrap();
        let now = 400 * DAY;
        // One sample at each age.
        db.insert_raw(&[raw(now - 2 * HOUR, 40.0)]).await.unwrap();
        db.rollup(now).await.unwrap();
        assert_eq!(db.counts().await.unwrap(), (0, 1));

        // Pretend a day and a bit passes: the 1-minute row becomes a 5-minute row.
        let r = db.rollup(now + 2 * DAY).await.unwrap();
        assert_eq!(r.m1_to_5m, 1);
        // Then 8 more days: it becomes a 1-hour row.
        let r = db.rollup(now + 12 * DAY).await.unwrap();
        assert_eq!(r.m5_to_1h, 1);
        assert_eq!(db.counts().await.unwrap(), (0, 1));
        // Then it is older than 180 days and disappears.
        db.rollup(now + 200 * DAY).await.unwrap();
        assert_eq!(db.counts().await.unwrap(), (0, 0));
    }

    #[tokio::test]
    async fn long_range_queries_return_bounded_point_counts() {
        let db = Database::memory().await.unwrap();
        let now = 3 * DAY;
        // 2 days of samples every 30 s.
        let rows: Vec<Row> = (0..(2 * DAY / 30_000))
            .map(|i| raw(now - 2 * DAY + i * 30_000, 50.0))
            .collect();
        db.insert_raw(&rows).await.unwrap();
        db.rollup(now).await.unwrap();
        let day = db.query_history(now, DAY).await.unwrap();
        assert!(day.len() <= 1500, "{}", day.len());
        let week = db.query_history(now, 7 * DAY).await.unwrap();
        assert!(week.len() <= 700, "{}", week.len());
        assert!(week.windows(2).all(|w| w[0].ts_ms < w[1].ts_ms));
    }

    #[tokio::test]
    async fn diagnoses_are_deduplicated_but_changes_are_recorded() {
        let db = Database::memory().await.unwrap();
        assert!(
            db.record_diagnosis(&diag(1000, "memory-pressure", "high"))
                .await
                .unwrap()
        );
        assert!(
            !db.record_diagnosis(&diag(60_000, "memory-pressure", "high"))
                .await
                .unwrap(),
            "same problem, soon after"
        );
        assert!(
            db.record_diagnosis(&diag(120_000, "memory-pressure", "critical"))
                .await
                .unwrap(),
            "severity changed"
        );
        assert!(
            db.record_diagnosis(&diag(180_000, "low-disk", "critical"))
                .await
                .unwrap(),
            "different rule"
        );
        assert!(
            db.record_diagnosis(&diag(180_000 + 31 * MIN, "low-disk", "critical"))
                .await
                .unwrap(),
            "still there after 30 min"
        );
        let all = db.recent_diagnoses(10).await.unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].rule_id, "low-disk");
        assert_eq!(all[0].primary_cause.as_deref(), Some("Docker/WSL2"));
    }

    #[tokio::test]
    async fn actions_round_trip_newest_first() {
        let db = Database::memory().await.unwrap();
        for i in 0..3 {
            db.insert_action(&StoredAction {
                id: 0,
                ts_ms: 1000 + i,
                action_id: "docker.stop_container".into(),
                name: "Stop".into(),
                risk: "LOW".into(),
                automated: i == 1,
                success: i != 2,
                summary: format!("n{i}"),
                freed_bytes: (i == 0).then_some(42),
            })
            .await
            .unwrap();
        }
        let a = db.recent_actions(2).await.unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].summary, "n2");
        assert!(!a[0].success && a[1].automated);
        assert_eq!(
            db.recent_actions(10).await.unwrap()[2].freed_bytes,
            Some(42)
        );
    }

    #[tokio::test]
    async fn old_events_are_pruned() {
        let db = Database::memory().await.unwrap();
        let now = 200 * DAY;
        db.record_diagnosis(&diag(now - 100 * DAY, "x", "high"))
            .await
            .unwrap();
        db.record_diagnosis(&diag(now - DAY, "y", "high"))
            .await
            .unwrap();
        db.rollup(now).await.unwrap();
        let left = db.recent_diagnoses(10).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].rule_id, "y");
    }

    #[tokio::test]
    async fn opening_a_real_file_works_and_reopening_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("devpilot.db");
        {
            let db = Database::open(&path).await.unwrap();
            db.insert_raw(&[raw(1000, 50.0)]).await.unwrap();
        }
        let db = Database::open(&path).await.unwrap();
        assert_eq!(db.counts().await.unwrap().0, 1);
    }

    #[tokio::test]
    async fn an_unusable_path_is_an_error_not_a_panic() {
        // A file where a directory is required.
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file.txt");
        std::fs::write(&blocker, b"x").unwrap();
        assert!(Database::open(&blocker.join("devpilot.db")).await.is_err());
    }
}

#[cfg(test)]
mod incident_tests {
    use super::*;
    use crate::monitoring::incident::summarize;

    #[tokio::test]
    async fn incident_round_trips_and_lists_newest_first() {
        let db = Database::memory().await.unwrap();
        let mk = |ts: u64| {
            let mut s = summarize(vec![], vec!["RAM above the critical threshold"]);
            s.start_ms = ts;
            s.end_ms = ts + 1000;
            s.diagnosis = format!("incident at {ts}");
            s
        };
        let id1 = db.insert_incident(&mk(1_000)).await.unwrap();
        let id2 = db.insert_incident(&mk(5_000)).await.unwrap();
        assert!(id2 > id1);
        let list = db.recent_incidents(10).await.unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].start_ms, 5_000);
        assert_eq!(list[0].id, id2);
        assert_eq!(list[1].diagnosis, "incident at 1000");
        assert_eq!(list[1].triggers, vec!["RAM above the critical threshold"]);
    }

    #[tokio::test]
    async fn old_incidents_are_pruned_by_rollup() {
        let db = Database::memory().await.unwrap();
        let mut s = summarize(vec![], vec![]);
        s.start_ms = 1;
        db.insert_incident(&s).await.unwrap();
        db.rollup(EVENTS_KEEP_MS + 10_000_000).await.unwrap();
        assert!(db.recent_incidents(10).await.unwrap().is_empty());
    }
}

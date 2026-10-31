//! Time-bucket aggregation for history. Pure, so rollups are tested without a database.
//!
//! A `Row` is either one raw sample (`samples == 1`) or a bucket of many. Merging weights averages
//! by sample count, keeps the maximums, and takes the drive and workload detail from the sample
//! where RAM peaked (the moment most worth explaining).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const RAW_KEEP_MS: i64 = 30 * 60 * 1000;
pub const MIN1_KEEP_MS: i64 = 24 * 3600 * 1000;
pub const MIN5_KEEP_MS: i64 = 7 * 24 * 3600 * 1000;
pub const HOUR_KEEP_MS: i64 = 180 * 24 * 3600 * 1000;
pub const EVENTS_KEEP_MS: i64 = 90 * 24 * 3600 * 1000;

pub const BUCKET_1M: i64 = 60;
pub const BUCKET_5M: i64 = 300;
pub const BUCKET_1H: i64 = 3600;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskPoint {
    pub mount: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopPoint {
    pub name: String,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub ts_ms: i64,
    pub samples: u32,
    pub ram_avg: f64,
    pub ram_max: f64,
    pub cpu_avg: f64,
    pub cpu_max: f64,
    pub ram_used_avg: u64,
    pub ram_total: u64,
    pub disks: Vec<DiskPoint>,
    pub top: Vec<TopPoint>,
}

impl Row {
    pub fn raw(
        ts_ms: i64,
        ram: f64,
        cpu: f64,
        used: u64,
        total: u64,
        disks: Vec<DiskPoint>,
        top: Vec<TopPoint>,
    ) -> Self {
        Row {
            ts_ms,
            samples: 1,
            ram_avg: ram,
            ram_max: ram,
            cpu_avg: cpu,
            cpu_max: cpu,
            ram_used_avg: used,
            ram_total: total,
            disks,
            top,
        }
    }
}

/// Merge rows into buckets of `bucket_secs`. Output is ordered by time.
pub fn merge(rows: &[Row], bucket_secs: i64) -> Vec<Row> {
    let bucket_ms = bucket_secs.max(1) * 1000;
    let mut groups: BTreeMap<i64, Vec<&Row>> = BTreeMap::new();
    for r in rows {
        groups
            .entry(r.ts_ms.div_euclid(bucket_ms) * bucket_ms)
            .or_default()
            .push(r);
    }
    groups
        .into_iter()
        .map(|(start, g)| {
            let n: u64 = g.iter().map(|r| r.samples as u64).sum::<u64>().max(1);
            let w = |f: &dyn Fn(&Row) -> f64| {
                g.iter().map(|r| f(r) * r.samples as f64).sum::<f64>() / n as f64
            };
            // The peak sample supplies the per-drive and per-workload detail.
            let peak = g
                .iter()
                .max_by(|a, b| a.ram_max.total_cmp(&b.ram_max))
                .expect("non-empty group");
            Row {
                ts_ms: start,
                samples: n.min(u32::MAX as u64) as u32,
                ram_avg: w(&|r| r.ram_avg),
                ram_max: g.iter().map(|r| r.ram_max).fold(f64::MIN, f64::max),
                cpu_avg: w(&|r| r.cpu_avg),
                cpu_max: g.iter().map(|r| r.cpu_max).fold(f64::MIN, f64::max),
                ram_used_avg: w(&|r| r.ram_used_avg as f64).round() as u64,
                ram_total: g.iter().map(|r| r.ram_total).max().unwrap_or(0),
                disks: peak.disks.clone(),
                top: peak.top.clone(),
            }
        })
        .collect()
}

/// Largest bucket boundary at or before `now - keep`: only whole buckets are ever rolled up.
pub fn cutoff(now_ms: i64, keep_ms: i64, bucket_secs: i64) -> i64 {
    let b = bucket_secs * 1000;
    (now_ms - keep_ms).div_euclid(b) * b
}

/// Which bucket size a chart over `range_ms` should use (about 100-400 points).
pub fn bucket_for_range(range_ms: i64) -> Option<i64> {
    match range_ms {
        r if r <= 45 * 60 * 1000 => None, // raw resolution is fine
        r if r <= 36 * 3600 * 1000 => Some(BUCKET_1M),
        r if r <= 8 * 24 * 3600 * 1000 => Some(BUCKET_5M),
        _ => Some(BUCKET_1H),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(ts: i64, ram: f64, cpu: f64) -> Row {
        Row::raw(ts, ram, cpu, (ram * 100.0) as u64, 16_000, vec![], vec![])
    }

    #[test]
    fn merges_by_bucket_with_weighted_average_and_max() {
        let rows = vec![
            r(0, 50.0, 10.0),
            r(2_000, 70.0, 30.0),
            r(58_000, 60.0, 20.0),
            r(60_000, 90.0, 5.0),
        ];
        let m = merge(&rows, 60);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].ts_ms, m[0].samples), (0, 3));
        assert!((m[0].ram_avg - 60.0).abs() < 1e-9);
        assert_eq!(m[0].ram_max, 70.0);
        assert_eq!(m[0].cpu_max, 30.0);
        assert!((m[0].cpu_avg - 20.0).abs() < 1e-9);
        assert_eq!((m[1].ts_ms, m[1].samples, m[1].ram_max), (60_000, 1, 90.0));
    }

    #[test]
    fn merging_merged_rows_is_weighted_by_sample_count() {
        // 1 sample at 100 and 3 samples at 20 average to 40, not 60.
        let a = Row {
            samples: 1,
            ..r(0, 100.0, 0.0)
        };
        let b = Row {
            samples: 3,
            ..r(1_000, 20.0, 0.0)
        };
        let m = merge(&[a, b], 300);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].samples, 4);
        assert!((m[0].ram_avg - 40.0).abs() < 1e-9);
        assert_eq!(m[0].ram_max, 100.0);
    }

    #[test]
    fn rollup_is_equivalent_to_direct_aggregation() {
        let rows: Vec<Row> = (0..600)
            .map(|i| r(i * 2_000, 40.0 + (i % 17) as f64, (i % 9) as f64))
            .collect();
        let direct = merge(&rows, 300);
        let via_minutes = merge(&merge(&rows, 60), 300);
        assert_eq!(direct.len(), via_minutes.len());
        for (a, b) in direct.iter().zip(&via_minutes) {
            assert_eq!((a.ts_ms, a.samples), (b.ts_ms, b.samples));
            assert!((a.ram_avg - b.ram_avg).abs() < 1e-9 && a.ram_max == b.ram_max);
            assert!((a.cpu_avg - b.cpu_avg).abs() < 1e-9);
        }
    }

    #[test]
    fn detail_comes_from_the_peak_sample() {
        let disks = |free: u64| {
            vec![DiskPoint {
                mount: "C:\\".into(),
                free_bytes: free,
                total_bytes: 100,
            }]
        };
        let top = |n: &str| {
            vec![TopPoint {
                name: n.into(),
                memory_bytes: 1,
            }]
        };
        let rows = vec![
            Row::raw(0, 50.0, 0.0, 1, 2, disks(10), top("calm")),
            Row::raw(1_000, 95.0, 0.0, 1, 2, disks(5), top("peak")),
            Row::raw(2_000, 60.0, 0.0, 1, 2, disks(9), top("later")),
        ];
        let m = merge(&rows, 60);
        assert_eq!(m[0].top[0].name, "peak");
        assert_eq!(m[0].disks[0].free_bytes, 5);
    }

    #[test]
    fn unsorted_input_and_empty_input() {
        let rows = vec![r(120_000, 1.0, 1.0), r(0, 2.0, 2.0)];
        let m = merge(&rows, 60);
        assert_eq!(
            m.iter().map(|x| x.ts_ms).collect::<Vec<_>>(),
            vec![0, 120_000]
        );
        assert!(merge(&[], 60).is_empty());
    }

    #[test]
    fn negative_timestamps_do_not_panic() {
        let m = merge(&[r(-1_000, 1.0, 1.0)], 60);
        assert_eq!(m[0].ts_ms, -60_000);
    }

    #[test]
    fn cutoff_aligns_to_whole_buckets() {
        assert_eq!(
            cutoff(10_000_000, 1_800_000, 60),
            8_160_000 / 60_000 * 60_000
        );
        assert_eq!(cutoff(3_600_000 * 5 + 1234, 0, 3600), 3_600_000 * 5);
        assert_eq!(
            cutoff(100, 1000, 60),
            -60_000,
            "stays aligned even before the epoch"
        );
    }

    #[test]
    fn range_to_bucket_choice() {
        assert_eq!(bucket_for_range(15 * 60 * 1000), None);
        assert_eq!(bucket_for_range(6 * 3600 * 1000), Some(BUCKET_1M));
        assert_eq!(bucket_for_range(24 * 3600 * 1000), Some(BUCKET_1M));
        assert_eq!(bucket_for_range(7 * 24 * 3600 * 1000), Some(BUCKET_5M));
        assert_eq!(bucket_for_range(30 * 24 * 3600 * 1000), Some(BUCKET_1H));
    }
}

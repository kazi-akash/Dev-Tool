//! Windows commit-charge information (not available through sysinfo).

/// Commit/physical memory totals in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommitInfo {
    pub commit_total: u64,
    pub commit_limit: u64,
    pub physical_total: u64,
}

impl CommitInfo {
    /// Commit charge as a percentage of the commit limit (RAM + pagefile).
    pub fn commit_percent(&self) -> f32 {
        pct(self.commit_total, self.commit_limit)
    }

    /// How much of the pagefile-backed commit headroom is consumed.
    ///
    /// Estimate: commit above physical RAM must be backed by the pagefile, so
    /// `(commit_total - RAM) / (commit_limit - RAM)`. It reads 0 while everything fits in RAM and
    /// approaches 100 as the system nears its commit limit. It is not the pagefile's resident size.
    pub fn pagefile_overflow_percent(&self) -> f32 {
        let capacity = self.commit_limit.saturating_sub(self.physical_total);
        let used = self.commit_total.saturating_sub(self.physical_total);
        pct(used, capacity)
    }
}

fn pct(used: u64, total: u64) -> f32 {
    if total == 0 {
        return 0.0;
    }
    ((used as f64 / total as f64) * 100.0).clamp(0.0, 100.0) as f32
}

/// Read commit info. `None` if the Windows call fails; callers degrade to RAM-only pressure.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn read_commit_info() -> Option<CommitInfo> {
    use ::windows::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};

    let mut info = PERFORMANCE_INFORMATION::default();
    // SAFETY: `info` is a valid, writable PERFORMANCE_INFORMATION and `cb` is its exact size.
    unsafe {
        GetPerformanceInfo(
            &mut info,
            std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32,
        )
        .ok()?;
    }
    let page = info.PageSize as u64;
    Some(CommitInfo {
        commit_total: info.CommitTotal as u64 * page,
        commit_limit: info.CommitLimit as u64 * page,
        physical_total: info.PhysicalTotal as u64 * page,
    })
}

#[cfg(not(windows))]
pub fn read_commit_info() -> Option<CommitInfo> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn overflow_is_zero_while_commit_fits_in_ram() {
        let c = CommitInfo {
            commit_total: 10 * GB,
            commit_limit: 24 * GB,
            physical_total: 16 * GB,
        };
        assert_eq!(c.pagefile_overflow_percent(), 0.0);
        assert!((c.commit_percent() - 41.666).abs() < 0.1);
    }

    #[test]
    fn overflow_grows_past_ram() {
        let c = CommitInfo {
            commit_total: 20 * GB,
            commit_limit: 24 * GB,
            physical_total: 16 * GB,
        };
        assert_eq!(c.pagefile_overflow_percent(), 50.0);
    }

    #[test]
    fn no_pagefile_means_no_division_by_zero() {
        let c = CommitInfo {
            commit_total: 10 * GB,
            commit_limit: 16 * GB,
            physical_total: 16 * GB,
        };
        assert_eq!(c.pagefile_overflow_percent(), 0.0);
    }

    #[test]
    fn reads_real_values_on_windows() {
        let info = read_commit_info();
        if cfg!(windows) {
            let c = info.expect("GetPerformanceInfo failed on Windows");
            assert!(c.physical_total > 0);
            assert!(c.commit_limit >= c.physical_total);
            assert!(c.commit_total > 0);
        } else {
            assert!(info.is_none());
        }
    }
}

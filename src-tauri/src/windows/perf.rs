//! Windows performance counters (PDH): hard-fault rate, pagefile usage and disk active time.
//!
//! These are not available through `sysinfo`. Counters are added by their *English* path
//! (`PdhAddEnglishCounterW`) so they work on every display language. Rate counters need two
//! collections, so the first sample reports `None` for them. Any failure degrades to `None`;
//! callers must treat every field as optional. This module holds the second audited `unsafe` area.

/// One reading. `None` means "not measurable right now", never "zero".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PerfSample {
    /// Pages read from disk to resolve hard faults per second (`\Memory\Pages Input/sec`).
    pub hard_faults_per_sec: Option<f32>,
    /// Share of the pagefile in use, 0-100 (`\Paging File(_Total)\% Usage`).
    pub pagefile_usage_percent: Option<f32>,
    /// Share of time the disks were busy, 0-100 (100 - `\PhysicalDisk(_Total)\% Idle Time`).
    pub disk_active_percent: Option<f32>,
}

/// Disk active time from the idle-time counter, clamped to 0-100.
pub fn active_from_idle(idle_percent: f64) -> Option<f32> {
    if !idle_percent.is_finite() {
        return None;
    }
    Some((100.0 - idle_percent).clamp(0.0, 100.0) as f32)
}

/// Sanitize a counter value: finite and non-negative.
pub fn sane(value: f64) -> Option<f32> {
    (value.is_finite() && value >= 0.0).then_some(value.min(f32::MAX as f64) as f32)
}

#[cfg(windows)]
pub use imp::PerfCounters;

#[cfg(not(windows))]
#[derive(Default)]
pub struct PerfCounters;

#[cfg(not(windows))]
impl PerfCounters {
    pub fn new() -> Self {
        Self
    }
    pub fn sample(&mut self) -> PerfSample {
        PerfSample::default()
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use ::windows::Win32::System::Performance::{
        PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PdhAddEnglishCounterW,
        PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterValue, PdhOpenQueryW,
    };
    use ::windows::core::{PCWSTR, w};

    use super::{PerfSample, active_from_idle, sane};

    /// The PDH query handle is owned by exactly one thread (the sampler); it is never shared.
    pub struct PerfCounters {
        query: Option<PDH_HQUERY>,
        faults: Option<PDH_HCOUNTER>,
        pagefile: Option<PDH_HCOUNTER>,
        disk_idle: Option<PDH_HCOUNTER>,
    }

    impl Default for PerfCounters {
        fn default() -> Self {
            Self::new()
        }
    }

    fn add(query: PDH_HQUERY, path: PCWSTR) -> Option<PDH_HCOUNTER> {
        let mut counter = PDH_HCOUNTER::default();
        // SAFETY: `query` is a live handle from PdhOpenQueryW, `path` is a NUL-terminated static
        // wide string and `counter` is a valid out pointer. Non-zero return means failure.
        let status = unsafe { PdhAddEnglishCounterW(query, path, 0, &mut counter) };
        (status == 0 && !counter.is_invalid()).then_some(counter)
    }

    fn read(counter: PDH_HCOUNTER) -> Option<f64> {
        let mut value = PDH_FMT_COUNTERVALUE::default();
        // SAFETY: `counter` belongs to a live query; `value` is a valid, writable out struct.
        let status =
            unsafe { PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE, None, &mut value) };
        // CStatus 0 / 1 are PDH_CSTATUS_VALID_DATA / NEW_DATA.
        if status != 0 || value.CStatus > 1 {
            return None;
        }
        // SAFETY: PDH_FMT_DOUBLE was requested and the status is valid, so `doubleValue` is set.
        Some(unsafe { value.Anonymous.doubleValue })
    }

    impl PerfCounters {
        pub fn new() -> Self {
            let mut query = PDH_HQUERY::default();
            // SAFETY: null data source means real-time data; `query` is a valid out pointer.
            let status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut query) };
            if status != 0 || query.is_invalid() {
                return Self {
                    query: None,
                    faults: None,
                    pagefile: None,
                    disk_idle: None,
                };
            }
            let me = Self {
                query: Some(query),
                faults: add(query, w!("\\Memory\\Pages Input/sec")),
                pagefile: add(query, w!("\\Paging File(_Total)\\% Usage")),
                disk_idle: add(query, w!("\\PhysicalDisk(_Total)\\% Idle Time")),
            };
            // Prime the rate counters; the next collection yields the first real values.
            // SAFETY: `query` is live.
            unsafe { PdhCollectQueryData(query) };
            me
        }

        pub fn sample(&mut self) -> PerfSample {
            let Some(query) = self.query else {
                return PerfSample::default();
            };
            // SAFETY: `query` is live and owned by this struct.
            if unsafe { PdhCollectQueryData(query) } != 0 {
                return PerfSample::default();
            }
            PerfSample {
                hard_faults_per_sec: self.faults.and_then(read).and_then(sane),
                pagefile_usage_percent: self
                    .pagefile
                    .and_then(read)
                    .and_then(sane)
                    .map(|v| v.min(100.0)),
                disk_active_percent: self.disk_idle.and_then(read).and_then(active_from_idle),
            }
        }
    }

    impl Drop for PerfCounters {
        fn drop(&mut self) {
            if let Some(q) = self.query.take() {
                // SAFETY: closing the handle opened in `new`; counters die with the query.
                unsafe { PdhCloseQuery(q) };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_maps_to_active() {
        assert_eq!(active_from_idle(100.0), Some(0.0));
        assert_eq!(active_from_idle(0.0), Some(100.0));
        assert_eq!(active_from_idle(-5.0), Some(100.0));
        assert_eq!(active_from_idle(140.0), Some(0.0));
        assert_eq!(active_from_idle(f64::NAN), None);
    }

    #[test]
    fn sane_rejects_nan_and_negative() {
        assert_eq!(sane(f64::NAN), None);
        assert_eq!(sane(-1.0), None);
        assert_eq!(sane(12.5), Some(12.5));
    }

    #[test]
    fn real_counters_do_not_crash_and_stay_in_range() {
        let mut p = PerfCounters::new();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let s = p.sample();
        if let Some(d) = s.disk_active_percent {
            assert!((0.0..=100.0).contains(&d));
        }
        if let Some(f) = s.hard_faults_per_sec {
            assert!(f >= 0.0);
        }
        if cfg!(windows) {
            // Pages Input/sec exists on every Windows install.
            assert!(s.hard_faults_per_sec.is_some(), "{s:?}");
        }
    }
}

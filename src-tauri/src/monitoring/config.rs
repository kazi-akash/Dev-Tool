//! Monitor interval rules, shared by the service and the IPC command that changes it.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub const DEFAULT_INTERVAL_SECS: u64 = 2;
pub const MIN_INTERVAL_SECS: u64 = 1;
pub const MAX_INTERVAL_SECS: u64 = 60;

/// Interval in seconds, adjustable at runtime. Always within MIN..=MAX.
#[derive(Debug)]
pub struct MonitorInterval(AtomicU64);

impl Default for MonitorInterval {
    fn default() -> Self {
        Self(AtomicU64::new(DEFAULT_INTERVAL_SECS))
    }
}

impl MonitorInterval {
    pub fn get(&self) -> Duration {
        Duration::from_secs(self.0.load(Ordering::Relaxed))
    }

    /// Validates and stores. Rejects out-of-range values instead of silently clamping.
    pub fn set_secs(&self, secs: u64) -> Result<(), String> {
        if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&secs) {
            return Err(format!(
                "interval must be between {MIN_INTERVAL_SECS} and {MAX_INTERVAL_SECS} seconds"
            ));
        }
        self.0.store(secs, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_two_seconds() {
        assert_eq!(MonitorInterval::default().get(), Duration::from_secs(2));
    }

    #[test]
    fn accepts_in_range() {
        let i = MonitorInterval::default();
        assert!(i.set_secs(5).is_ok());
        assert_eq!(i.get(), Duration::from_secs(5));
    }

    #[test]
    fn rejects_out_of_range_and_keeps_old_value() {
        let i = MonitorInterval::default();
        assert!(i.set_secs(0).is_err());
        assert!(i.set_secs(61).is_err());
        assert_eq!(i.get(), Duration::from_secs(2));
    }
}

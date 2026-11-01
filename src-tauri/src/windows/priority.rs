//! Lowers the calling thread to background priority: less CPU, and, more importantly for a disk
//! scan, **low I/O and memory priority** (`THREAD_MODE_BACKGROUND_BEGIN`). Used by the storage scan
//! so it never competes with the programs the user is actually working in.
//!
//! This is the third audited `unsafe` area (with `memory.rs` and `perf.rs`): two Windows calls with
//! no pointers, acting only on the current thread.

/// Put the current thread in background mode. Returns whether Windows accepted it; failure is
/// harmless (the scan is merely a little less polite).
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn enter_background_mode() -> bool {
    use ::windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
    };
    // SAFETY: `GetCurrentThread` returns a pseudo-handle for the calling thread, valid without
    // closing; `SetThreadPriority` takes no pointers and only changes this thread's priority.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN).is_ok() }
}

#[cfg(not(windows))]
pub fn enter_background_mode() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_mode_is_accepted_on_a_worker_thread_and_only_affects_it() {
        // A separate thread: the test harness thread must keep its priority.
        let accepted = std::thread::spawn(enter_background_mode).join().unwrap();
        assert_eq!(accepted, cfg!(windows));
    }
}

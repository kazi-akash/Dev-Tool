use std::sync::RwLock;

use super::model::{ProcessInfo, ProcessList, ProcessQuery, query_processes};

/// Latest full process list, written by the sampler and read by IPC commands and later engines.
#[derive(Default)]
pub struct ProcessStore {
    latest: RwLock<Vec<ProcessInfo>>,
}

impl ProcessStore {
    pub fn replace(&self, processes: Vec<ProcessInfo>) {
        // Poisoned lock only means a writer panicked; the data is still a valid list.
        *self.latest.write().unwrap_or_else(|e| e.into_inner()) = processes;
    }

    pub fn query(&self, q: &ProcessQuery) -> ProcessList {
        let guard = self.latest.read().unwrap_or_else(|e| e.into_inner());
        query_processes(&guard, q)
    }

    /// Full copy for in-process engines (workload detection, diagnosis).
    #[allow(dead_code)] // first consumer arrives in Phase 4
    pub fn snapshot(&self) -> Vec<ProcessInfo> {
        self.latest
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processes::model::{Category, sample};

    fn q() -> ProcessQuery {
        ProcessQuery {
            limit: None,
            sort_by: None,
            category: None,
        }
    }

    #[test]
    fn empty_store_returns_empty_list() {
        let r = ProcessStore::default().query(&q());
        assert_eq!((r.total, r.matched, r.processes.len()), (0, 0, 0));
    }

    #[test]
    fn replace_then_query() {
        let s = ProcessStore::default();
        s.replace(vec![
            sample(1, "a", 10, 0.0, Category::Node),
            sample(2, "b", 20, 0.0, Category::Node),
        ]);
        let r = s.query(&q());
        assert_eq!(r.total, 2);
        assert_eq!(r.processes[0].pid, 2);
        s.replace(vec![]);
        assert_eq!(s.query(&q()).total, 0);
    }

    #[test]
    fn survives_poisoned_lock() {
        use std::sync::Arc;
        let s = Arc::new(ProcessStore::default());
        s.replace(vec![sample(1, "a", 1, 0.0, Category::Other)]);
        let s2 = s.clone();
        let _ = std::thread::spawn(move || {
            let _guard = s2.latest.write().unwrap();
            panic!("poison");
        })
        .join();
        assert_eq!(s.query(&q()).total, 1);
        s.replace(vec![]);
        assert_eq!(s.query(&q()).total, 0);
    }
}

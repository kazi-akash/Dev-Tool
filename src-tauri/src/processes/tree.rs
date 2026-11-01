//! Parent/child process tree. Pure and bounded: the UI never receives the whole process table.
//!
//! Windows reuses PIDs, so a recorded parent PID may now belong to an unrelated, newer process. A
//! parent is only accepted when it started no later than its child; otherwise the child is a root.
//! Cycles (possible after PID reuse) are cut. Memory is each process's own figure; a subtree total
//! is the plain sum of its members, which counts shared pages once per process, like Task Manager's
//! "Working set" column does.

use std::collections::HashMap;

use serde::Serialize;

use super::model::{Category, ProcessInfo};

/// Subtrees smaller than this are left out (they are noise in a tree meant to explain memory).
pub const MIN_SUBTREE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_ROOTS: usize = 40;
pub const MAX_NODES: usize = 400;
pub const MAX_DEPTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub pid: u32,
    pub name: String,
    pub category: Category,
    pub cpu_percent: f32,
    /// This process only.
    pub memory_bytes: u64,
    /// This process plus every descendant.
    pub subtree_bytes: u64,
    /// Descendants not shown because of the depth or node limit, or because they were tiny.
    pub hidden_children: usize,
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessTree {
    pub roots: Vec<TreeNode>,
    /// Processes in the table / processes represented by a node.
    pub total_processes: usize,
    pub shown_nodes: usize,
    /// More roots or nodes existed than the limits allow.
    pub truncated: bool,
}

/// Index of each process's accepted parent, or `None` for roots.
fn parent_links(procs: &[ProcessInfo]) -> Vec<Option<usize>> {
    let by_pid: HashMap<u32, usize> = procs.iter().enumerate().map(|(i, p)| (p.pid, i)).collect();
    procs
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let parent = *by_pid.get(&p.parent_pid?)?;
            if parent == i {
                return None;
            }
            // PID reuse: a parent cannot be younger than its child. Unknown start times (0) are accepted.
            let (pp, cp) = (procs[parent].start_time, p.start_time);
            (pp == 0 || cp == 0 || pp <= cp).then_some(parent)
        })
        .collect()
}

pub fn build_tree(procs: &[ProcessInfo]) -> ProcessTree {
    let links = parent_links(procs);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); procs.len()];
    let mut roots: Vec<usize> = Vec::new();
    for (i, parent) in links.iter().enumerate() {
        match parent {
            Some(p) => children[*p].push(i),
            None => roots.push(i),
        }
    }

    // Subtree sums, iteratively and cycle-safe: a process visited twice is not counted again.
    let mut subtree = vec![0u64; procs.len()];
    let mut counted = vec![false; procs.len()];
    fn sum(
        i: usize,
        procs: &[ProcessInfo],
        children: &[Vec<usize>],
        subtree: &mut [u64],
        counted: &mut [bool],
    ) -> u64 {
        if counted[i] {
            return 0;
        }
        counted[i] = true;
        let mut total = procs[i].memory_bytes;
        for &c in &children[i] {
            total += sum(c, procs, children, subtree, counted);
        }
        subtree[i] = total;
        total
    }
    // Roots first; anything left over sits in a cycle and is attached below as a root of its own.
    for &r in &roots {
        sum(r, procs, &children, &mut subtree, &mut counted);
    }
    for i in 0..procs.len() {
        if !counted[i] {
            // A cycle after PID reuse: cut it here and treat this process as a root.
            roots.push(i);
            children[i].retain(|c| !counted[*c]);
            sum(i, procs, &children, &mut subtree, &mut counted);
        }
    }

    roots.retain(|&r| subtree[r] >= MIN_SUBTREE_BYTES);
    roots.sort_by_key(|&r| std::cmp::Reverse(subtree[r]));
    let mut truncated = roots.len() > MAX_ROOTS;
    roots.truncate(MAX_ROOTS);

    let mut budget = MAX_NODES;
    let mut shown = 0usize;
    let mut out = Vec::new();
    for r in roots {
        if budget == 0 {
            truncated = true;
            break;
        }
        out.push(node(
            r,
            0,
            procs,
            &children,
            &subtree,
            &mut budget,
            &mut shown,
            &mut truncated,
        ));
    }
    ProcessTree {
        roots: out,
        total_processes: procs.len(),
        shown_nodes: shown,
        truncated,
    }
}

#[allow(clippy::too_many_arguments)]
fn node(
    i: usize,
    depth: usize,
    procs: &[ProcessInfo],
    children: &[Vec<usize>],
    subtree: &[u64],
    budget: &mut usize,
    shown: &mut usize,
    truncated: &mut bool,
) -> TreeNode {
    *budget = budget.saturating_sub(1);
    *shown += 1;
    let mut kids: Vec<usize> = children[i]
        .iter()
        .copied()
        .filter(|&c| subtree[c] >= MIN_SUBTREE_BYTES)
        .collect();
    kids.sort_by_key(|&c| std::cmp::Reverse(subtree[c]));
    let mut hidden = children[i].len() - kids.len();
    let mut shown_kids = Vec::new();
    for (n, c) in kids.iter().enumerate() {
        if depth + 1 >= MAX_DEPTH || *budget == 0 {
            hidden += kids.len() - n;
            *truncated = true;
            break;
        }
        shown_kids.push(node(
            *c,
            depth + 1,
            procs,
            children,
            subtree,
            budget,
            shown,
            truncated,
        ));
    }
    let p = &procs[i];
    TreeNode {
        pid: p.pid,
        name: p.name.clone(),
        category: p.category,
        cpu_percent: p.cpu_percent,
        memory_bytes: p.memory_bytes,
        subtree_bytes: subtree[i],
        hidden_children: hidden,
        children: shown_kids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processes::model::sample;

    const MIB: u64 = 1024 * 1024;

    fn p(pid: u32, parent: Option<u32>, start: u64, mem_mb: u64) -> ProcessInfo {
        let mut x = sample(
            pid,
            &format!("p{pid}.exe"),
            mem_mb * MIB,
            1.0,
            Category::Node,
        );
        x.parent_pid = parent;
        x.start_time = start;
        x
    }

    fn find(nodes: &[TreeNode], pid: u32) -> Option<&TreeNode> {
        nodes.iter().find_map(|n| {
            if n.pid == pid {
                Some(n)
            } else {
                find(&n.children, pid)
            }
        })
    }

    #[test]
    fn groups_children_under_parents_and_sums_subtrees() {
        let t = build_tree(&[
            p(1, None, 10, 100),
            p(2, Some(1), 20, 300),
            p(3, Some(1), 30, 50),
            p(4, Some(2), 40, 200),
        ]);
        assert_eq!(t.roots.len(), 1);
        let root = &t.roots[0];
        assert_eq!(
            (root.pid, root.memory_bytes, root.subtree_bytes),
            (1, 100 * MIB, 650 * MIB)
        );
        // Largest subtree first.
        assert_eq!(
            root.children.iter().map(|c| c.pid).collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(find(&t.roots, 4).unwrap().subtree_bytes, 200 * MIB);
        assert_eq!(t.shown_nodes, 4);
        assert!(!t.truncated);
    }

    #[test]
    fn a_reused_parent_pid_does_not_adopt_older_processes() {
        // PID 1 started after PID 2, so it cannot be PID 2's parent: 2 is a root.
        let t = build_tree(&[p(1, None, 500, 100), p(2, Some(1), 100, 100)]);
        assert_eq!(t.roots.len(), 2);
        assert!(t.roots.iter().all(|r| r.children.is_empty()));
    }

    #[test]
    fn missing_parents_make_roots_and_self_parents_are_cut() {
        let t = build_tree(&[p(5, Some(999), 1, 100), p(6, Some(6), 1, 100)]);
        assert_eq!(t.roots.len(), 2);
    }

    #[test]
    fn cycles_terminate_and_nothing_is_counted_twice() {
        let t = build_tree(&[p(1, Some(2), 0, 100), p(2, Some(1), 0, 100)]);
        let total: u64 = t.roots.iter().map(|r| r.subtree_bytes).sum();
        assert_eq!(total, 200 * MIB, "each process appears once");
        assert!(t.shown_nodes >= 1);
    }

    #[test]
    fn tiny_subtrees_are_hidden_but_counted_as_hidden_children() {
        let t = build_tree(&[p(1, None, 1, 500), p(2, Some(1), 2, 1), p(3, Some(1), 3, 2)]);
        let root = &t.roots[0];
        assert!(root.children.is_empty());
        assert_eq!(root.hidden_children, 2);
        assert_eq!(
            root.subtree_bytes,
            503 * MIB,
            "hidden members still count toward the total"
        );
    }

    #[test]
    fn a_tiny_root_is_left_out_entirely() {
        let t = build_tree(&[p(1, None, 1, 5)]);
        assert!(t.roots.is_empty());
        assert_eq!(t.total_processes, 1);
    }

    #[test]
    fn depth_and_node_limits_bound_the_output() {
        // A chain deeper than MAX_DEPTH.
        let chain: Vec<ProcessInfo> = (1..=20)
            .map(|i| p(i, if i == 1 { None } else { Some(i - 1) }, i as u64, 50))
            .collect();
        let t = build_tree(&chain);
        fn depth(n: &TreeNode) -> usize {
            1 + n.children.iter().map(depth).max().unwrap_or(0)
        }
        assert!(depth(&t.roots[0]) <= MAX_DEPTH);
        assert!(t.truncated);

        // Many roots.
        let many: Vec<ProcessInfo> = (1..=300).map(|i| p(i, None, 1, 30)).collect();
        let t = build_tree(&many);
        assert_eq!(t.roots.len(), MAX_ROOTS);
        assert!(t.truncated);
        assert!(t.shown_nodes <= MAX_NODES);
    }

    #[test]
    fn roots_are_ordered_by_subtree_memory() {
        let t = build_tree(&[
            p(1, None, 1, 100),
            p(2, None, 1, 900),
            p(3, Some(1), 2, 900),
        ]);
        assert_eq!(
            t.roots.iter().map(|r| r.pid).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn serializes_camel_case() {
        let j = serde_json::to_string(&build_tree(&[p(1, None, 1, 100)])).unwrap();
        assert!(
            j.contains("subtreeBytes")
                && j.contains("hiddenChildren")
                && j.contains("totalProcesses"),
            "{j}"
        );
    }
}

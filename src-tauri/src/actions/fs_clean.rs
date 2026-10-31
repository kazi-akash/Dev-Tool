//! Deletion helpers. Never follow reparse points (symlinks, junctions); report what was freed.

use std::fs;
use std::io;
use std::path::Path;
use std::time::{Duration, SystemTime};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CleanReport {
    pub freed_bytes: u64,
    pub removed: usize,
    pub failed: usize,
}

#[cfg(windows)]
fn is_link(md: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    md.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT
}

#[cfg(not(windows))]
fn is_link(md: &fs::Metadata) -> bool {
    md.file_type().is_symlink()
}

/// Size of a file or tree, not following links.
pub fn tree_size(path: &Path) -> u64 {
    let Ok(md) = fs::symlink_metadata(path) else {
        return 0;
    };
    if is_link(&md) {
        return 0;
    }
    if !md.is_dir() {
        return md.len();
    }
    fs::read_dir(path)
        .map(|rd| rd.flatten().map(|e| tree_size(&e.path())).sum())
        .unwrap_or(0)
}

/// True if `path` is itself a symlink/junction (deleting it would be ambiguous; callers refuse).
pub fn is_reparse_point(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| is_link(&m))
        .unwrap_or(false)
}

/// Remove one file or directory tree. A link is removed as a link, its target is untouched.
pub fn remove_path(path: &Path) -> io::Result<u64> {
    let md = fs::symlink_metadata(path)?;
    let size = tree_size(path);
    if is_link(&md) {
        // A directory symlink/junction is removed with `remove_dir` (the target is untouched);
        // file links use `remove_file`. `symlink_metadata().is_dir()` is false for links, so try both.
        fs::remove_dir(path).or_else(|_| fs::remove_file(path))?;
        return Ok(0);
    }
    if md.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(size)
}

fn age_of(md: &fs::Metadata, now: SystemTime) -> Option<Duration> {
    now.duration_since(md.modified().ok()?).ok()
}

/// Remove the *children* of `dir` (the folder itself stays; it may be in use). With `min_age`,
/// children modified more recently than that are kept. Locked or protected children are skipped.
pub fn clean_contents(dir: &Path, min_age: Option<Duration>, now: SystemTime) -> CleanReport {
    let mut report = CleanReport::default();
    let Ok(rd) = fs::read_dir(dir) else {
        return report;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(md) = fs::symlink_metadata(&path) else {
            report.failed += 1;
            continue;
        };
        if let Some(min) = min_age
            && age_of(&md, now).is_none_or(|a| a < min)
        {
            continue;
        }
        match remove_path(&path) {
            Ok(freed) => {
                report.freed_bytes += freed;
                report.removed += 1;
            }
            Err(_) => report.failed += 1,
        }
    }
    report
}

/// What `clean_contents` would remove, without removing it.
pub fn plan_clean_contents(dir: &Path, min_age: Option<Duration>, now: SystemTime) -> CleanReport {
    let mut report = CleanReport::default();
    let Ok(rd) = fs::read_dir(dir) else {
        return report;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(md) = fs::symlink_metadata(&path) else {
            continue;
        };
        if let Some(min) = min_age
            && age_of(&md, now).is_none_or(|a| a < min)
        {
            continue;
        }
        report.freed_bytes += tree_size(&path);
        report.removed += 1;
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(p: &Path, n: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::File::create(p)
            .unwrap()
            .write_all(&vec![1u8; n])
            .unwrap();
    }

    #[test]
    fn tree_size_sums_nested_files() {
        let t = tempfile::tempdir().unwrap();
        write(&t.path().join("a/b/c.bin"), 100);
        write(&t.path().join("a/d.bin"), 50);
        assert_eq!(tree_size(&t.path().join("a")), 150);
        assert_eq!(tree_size(&t.path().join("missing")), 0);
    }

    #[test]
    fn remove_path_deletes_trees_and_reports_size() {
        let t = tempfile::tempdir().unwrap();
        write(&t.path().join("x/y/z.bin"), 300);
        assert_eq!(remove_path(&t.path().join("x")).unwrap(), 300);
        assert!(!t.path().join("x").exists());
        write(&t.path().join("f.bin"), 7);
        assert_eq!(remove_path(&t.path().join("f.bin")).unwrap(), 7);
        assert!(remove_path(&t.path().join("gone")).is_err());
    }

    #[test]
    fn clean_contents_keeps_the_folder_and_removes_children() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("Temp");
        write(&dir.join("a.tmp"), 10);
        write(&dir.join("sub/b.tmp"), 20);
        let r = clean_contents(&dir, None, SystemTime::now());
        assert_eq!((r.freed_bytes, r.removed, r.failed), (30, 2, 0));
        assert!(dir.exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
    }

    #[test]
    fn min_age_keeps_recent_files() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("Temp");
        write(&dir.join("fresh.tmp"), 10);
        // "now" is 1 hour after the file was written; require 24 h.
        let later = SystemTime::now() + Duration::from_secs(3600);
        let kept = clean_contents(&dir, Some(Duration::from_secs(24 * 3600)), later);
        assert_eq!(kept.removed, 0);
        assert!(dir.join("fresh.tmp").exists());
        // 2 days later it qualifies.
        let much_later = SystemTime::now() + Duration::from_secs(2 * 24 * 3600);
        let plan = plan_clean_contents(&dir, Some(Duration::from_secs(24 * 3600)), much_later);
        assert_eq!((plan.removed, plan.freed_bytes), (1, 10));
        assert!(dir.join("fresh.tmp").exists(), "planning never deletes");
        assert_eq!(
            clean_contents(&dir, Some(Duration::from_secs(24 * 3600)), much_later).removed,
            1
        );
    }

    #[test]
    fn missing_directory_is_a_noop() {
        let r = clean_contents(
            Path::new("Z:\\definitely\\not\\here"),
            None,
            SystemTime::now(),
        );
        assert_eq!(r, CleanReport::default());
    }

    #[test]
    fn directory_links_are_never_followed() {
        let t = tempfile::tempdir().unwrap();
        let target = t.path().join("precious");
        write(&target.join("keep.txt"), 5);
        let dir = t.path().join("Temp");
        fs::create_dir_all(&dir).unwrap();
        let link = dir.join("link");
        // Creating symlinks may need privileges; junction creation via mklink is unavailable here.
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&target, &link).is_ok();
        #[cfg(not(windows))]
        let made = std::os::unix::fs::symlink(&target, &link).is_ok();
        if !made {
            eprintln!("skipping: cannot create symlinks without privileges");
            return;
        }
        assert!(is_reparse_point(&link));
        let r = clean_contents(&dir, None, SystemTime::now());
        assert_eq!(r.failed, 0);
        assert!(target.join("keep.txt").exists(), "link target must survive");
    }
}

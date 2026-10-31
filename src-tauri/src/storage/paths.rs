//! Path safety. Used by the scanner (what it may enter) and the action engine (what it may delete).
//! Everything here is a pure function of the path text, so it is cheap to test exhaustively.

use std::path::{Component, Path};

/// Lowercase with `\` separators and no trailing separator (drive roots keep `x:\`).
pub fn normalize(p: &Path) -> String {
    let s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    // Strip the verbatim prefix produced by canonicalize on Windows.
    let s = s.strip_prefix("\\\\?\\").unwrap_or(&s).to_string();
    if s.len() > 3 {
        s.trim_end_matches('\\').to_string()
    } else {
        s
    }
}

/// True if the path contains `..` components.
pub fn has_traversal(p: &Path) -> bool {
    p.components().any(|c| matches!(c, Component::ParentDir))
}

/// `x:\...` absolute path on a drive letter. UNC and relative paths are rejected.
pub fn is_drive_absolute(p: &Path) -> bool {
    let n = normalize(p);
    let b = n.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

pub fn is_drive_root(p: &Path) -> bool {
    let n = normalize(p);
    n.len() == 3 && is_drive_absolute(p)
}

/// Directories directly under a drive that must never be scanned into or modified.
const PROTECTED_TOP_LEVEL: &[&str] = &[
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "system volume information",
    "$recycle.bin",
    "recovery",
    "boot",
    "efi",
    "$windows.~bt",
    "$windows.~ws",
    "windows.old",
    "msocache",
    "config.msi",
];

const USER_ROOT_DIRS: &[&str] = &[
    "desktop",
    "documents",
    "downloads",
    "pictures",
    "music",
    "videos",
    "onedrive",
    "appdata",
    "contacts",
    "favorites",
    "links",
    "saved games",
    "searches",
];

/// Protected = system areas, drive roots, and the structural folders of the user profile.
/// A *file or subfolder inside* a user folder (for example `Downloads\big.iso`) is not protected here;
/// whether it may be touched is decided by classification and explicit user choice.
pub fn is_protected(p: &Path, user_profile: Option<&Path>) -> bool {
    if !is_drive_absolute(p) || has_traversal(p) {
        return true;
    }
    let n = normalize(p);
    if is_drive_root(p) {
        return true;
    }
    let after_drive = &n[3..];
    let first = after_drive.split('\\').next().unwrap_or("");
    if PROTECTED_TOP_LEVEL.contains(&first) {
        return true;
    }
    // "x:\users" itself and other profiles' structure.
    if after_drive == "users" {
        return true;
    }
    if let Some(profile) = user_profile {
        let prof = normalize(profile);
        if n == prof {
            return true;
        }
        if let Some(rest) = n.strip_prefix(&format!("{prof}\\")) {
            let mut parts = rest.split('\\');
            let a = parts.next().unwrap_or("");
            let b = parts.next();
            if b.is_none() && USER_ROOT_DIRS.contains(&a) {
                return true;
            }
            // appdata\local, appdata\roaming, appdata\locallow themselves
            if a == "appdata"
                && parts.next().is_none()
                && matches!(b, Some("local" | "roaming" | "locallow"))
            {
                return true;
            }
        }
    } else if after_drive.starts_with("users\\") {
        let depth = after_drive.split('\\').count();
        // users\<name> and users\<name>\<folder> without a known profile: be conservative.
        if depth <= 3 {
            return true;
        }
    }
    false
}

/// `Some(path)` is `inside` `root` (strictly below it, component-wise, case-insensitive).
#[allow(dead_code)] // used by the delete actions (Phase 12)
pub fn is_strictly_inside(path: &Path, root: &Path) -> bool {
    let (p, r) = (normalize(path), normalize(root));
    p.len() > r.len() && p.starts_with(&r) && p.as_bytes()[r.len()] == b'\\'
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn pp(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    const PROFILE: &str = "C:\\Users\\dev";

    fn protected(s: &str) -> bool {
        is_protected(&pp(s), Some(&pp(PROFILE)))
    }

    #[test]
    fn normalize_handles_case_slashes_and_verbatim() {
        assert_eq!(normalize(&pp("C:/Users/Dev/")), "c:\\users\\dev");
        assert_eq!(normalize(&pp("\\\\?\\C:\\Temp")), "c:\\temp");
        assert_eq!(normalize(&pp("C:\\")), "c:\\");
    }

    #[test]
    fn traversal_detected() {
        assert!(has_traversal(&pp("C:\\Users\\dev\\..\\..\\Windows")));
        assert!(!has_traversal(&pp("C:\\Users\\dev\\proj")));
    }

    #[test]
    fn drive_absolute_rules() {
        assert!(is_drive_absolute(&pp("S:\\proj")));
        assert!(!is_drive_absolute(&pp("proj\\node_modules")));
        assert!(!is_drive_absolute(&pp("\\\\server\\share\\x")));
        assert!(!is_drive_absolute(&pp("C:proj")));
        assert!(is_drive_root(&pp("D:\\")));
        assert!(!is_drive_root(&pp("D:\\x")));
    }

    #[test]
    fn system_areas_are_protected() {
        for p in [
            "C:\\",
            "C:\\Windows",
            "C:\\Windows\\System32\\drivers",
            "c:\\WINDOWS\\Temp",
            "C:\\Program Files\\Docker",
            "C:\\Program Files (x86)\\X",
            "C:\\ProgramData\\Microsoft",
            "D:\\System Volume Information",
            "D:\\$Recycle.Bin\\S-1",
            "C:\\Users",
        ] {
            assert!(protected(p), "{p}");
        }
    }

    #[test]
    fn user_structure_is_protected_but_contents_are_not() {
        for p in [
            "C:\\Users\\dev",
            "C:\\Users\\dev\\Documents",
            "C:\\Users\\dev\\Desktop",
            "C:\\Users\\dev\\Downloads",
            "C:\\Users\\dev\\AppData",
            "C:\\Users\\dev\\AppData\\Local",
            "C:\\Users\\dev\\AppData\\Roaming",
        ] {
            assert!(protected(p), "{p}");
        }
        for p in [
            "C:\\Users\\dev\\Downloads\\big.iso",
            "C:\\Users\\dev\\AppData\\Local\\Temp",
            "C:\\Users\\dev\\AppData\\Local\\npm-cache",
            "C:\\Users\\dev\\projects\\app\\node_modules",
            "S:\\Client Projects\\app\\node_modules",
            "D:\\work",
        ] {
            assert!(!protected(p), "{p}");
        }
    }

    #[test]
    fn relative_unc_and_traversal_are_always_protected() {
        assert!(protected("node_modules"));
        assert!(protected("\\\\server\\share\\a"));
        assert!(protected("C:\\Users\\dev\\proj\\..\\..\\..\\Windows"));
    }

    #[test]
    fn without_known_profile_user_roots_are_conservative() {
        assert!(is_protected(&pp("C:\\Users\\alice\\Documents"), None));
        assert!(!is_protected(
            &pp("C:\\Users\\alice\\proj\\node_modules"),
            None
        ));
    }

    #[test]
    fn strictly_inside() {
        assert!(is_strictly_inside(&pp("C:\\a\\b\\c"), &pp("c:\\A\\B")));
        assert!(!is_strictly_inside(&pp("C:\\a\\b"), &pp("C:\\a\\b")));
        assert!(
            !is_strictly_inside(&pp("C:\\a\\bc"), &pp("C:\\a\\b")),
            "prefix is not a parent"
        );
    }
}

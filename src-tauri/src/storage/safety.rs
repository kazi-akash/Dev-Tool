//! Cleanup safety classification. Never decided by file extension alone.

use serde::Serialize;

/// What kind of thing a scanned entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    TempCache,
    PackageCache,
    GradleCache,
    AndroidSdk,
    AndroidAvd,
    FlutterCache,
    DockerData,
    WslDisk,
    NodeModules,
    BuildOutput,
    Apk,
    Archive,
    DiskImage,
    Video,
    Installer,
    VirtualDisk,
    UserData,
    LargeFile,
    /// Reserved for entries no rule can place; always `RequiresReview`.
    #[allow(dead_code)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Safety {
    /// Safe to inspect and, after a look, remove with low risk (re-creatable).
    SafeToReview,
    /// Known disposable cache; eligible for opt-in safe automation.
    SafeToClean,
    /// Might contain irreplaceable data or active work; the user must decide.
    RequiresReview,
    /// Never offered for cleanup.
    DoNotTouch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Classification {
    pub safety: Safety,
    pub reason: &'static str,
}

/// Safety follows from what the entry *is* (location + role), not its file extension.
pub fn classify(kind: EntryKind) -> Classification {
    use EntryKind::*;
    use Safety::*;
    let (safety, reason) = match kind {
        TempCache => (SafeToClean, "Temporary files are recreated on demand."),
        PackageCache => (
            SafeToClean,
            "Package manager cache; packages are re-downloaded when needed.",
        ),
        GradleCache => (
            SafeToReview,
            "Gradle caches are re-downloaded, but the next build will be slower.",
        ),
        FlutterCache => (SafeToReview, "Pub cache is re-downloaded when needed."),
        BuildOutput => (
            SafeToReview,
            "Generated build output; a rebuild recreates it. Check it is not committed.",
        ),
        NodeModules => (
            RequiresReview,
            "Reinstallable with the package manager, but this project may be in use.",
        ),
        Apk => (
            RequiresReview,
            "Built APKs may be the only copy of a release.",
        ),
        Archive => (
            RequiresReview,
            "Large archive; may contain the only copy of data.",
        ),
        DiskImage => (
            RequiresReview,
            "Disk image; may be needed to reinstall software.",
        ),
        Video => (
            RequiresReview,
            "Personal media; DevPilot cannot know if it is replaceable.",
        ),
        Installer => (
            RequiresReview,
            "Installer; re-downloadable only if you know the source.",
        ),
        VirtualDisk => (RequiresReview, "Virtual disk; may hold a whole machine."),
        LargeFile | Unknown => (
            RequiresReview,
            "Unrecognized content; review before deciding.",
        ),
        AndroidSdk => (
            DoNotTouch,
            "Deleting the SDK breaks Android builds. Use the SDK Manager to remove packages.",
        ),
        AndroidAvd => (
            DoNotTouch,
            "Emulator images; manage them in Android Studio's Device Manager.",
        ),
        DockerData => (
            DoNotTouch,
            "Docker's disk image. Use `docker system prune` from Docker instead.",
        ),
        WslDisk => (
            DoNotTouch,
            "A WSL distribution's disk. Deleting it destroys that distribution.",
        ),
        UserData => (
            DoNotTouch,
            "Your personal documents are never offered for cleanup.",
        ),
    };
    Classification { safety, reason }
}

/// Role of a large file inferred from extension, used only to label it. Anything found this way is
/// at best `RequiresReview`: an extension cannot prove a file is disposable.
pub fn kind_from_extension(ext: &str) -> EntryKind {
    match ext.to_ascii_lowercase().as_str() {
        "apk" | "aab" => EntryKind::Apk,
        "iso" | "img" => EntryKind::DiskImage,
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz" => EntryKind::Archive,
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" => EntryKind::Video,
        "vhd" | "vhdx" | "vmdk" | "vdi" | "qcow2" => EntryKind::VirtualDisk,
        _ => EntryKind::LargeFile,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EntryKind::*;

    #[test]
    fn spec_examples() {
        assert_eq!(classify(TempCache).safety, Safety::SafeToClean);
        assert_eq!(classify(NodeModules).safety, Safety::RequiresReview);
        assert_eq!(classify(UserData).safety, Safety::DoNotTouch);
        assert_eq!(classify(Unknown).safety, Safety::RequiresReview);
    }

    #[test]
    fn only_known_disposable_caches_are_auto_clean_eligible() {
        let all = [
            TempCache,
            PackageCache,
            GradleCache,
            AndroidSdk,
            AndroidAvd,
            FlutterCache,
            DockerData,
            WslDisk,
            NodeModules,
            BuildOutput,
            Apk,
            Archive,
            DiskImage,
            Video,
            Installer,
            VirtualDisk,
            UserData,
            LargeFile,
            Unknown,
        ];
        let clean: Vec<_> = all
            .iter()
            .filter(|k| classify(**k).safety == Safety::SafeToClean)
            .collect();
        assert_eq!(clean, vec![&TempCache, &PackageCache]);
    }

    #[test]
    fn dangerous_things_are_do_not_touch() {
        for k in [AndroidSdk, AndroidAvd, DockerData, WslDisk, UserData] {
            assert_eq!(classify(k).safety, Safety::DoNotTouch, "{k:?}");
        }
    }

    #[test]
    fn every_kind_has_a_reason() {
        for k in [
            TempCache,
            PackageCache,
            NodeModules,
            Unknown,
            DockerData,
            Video,
        ] {
            assert!(!classify(k).reason.is_empty());
        }
    }

    #[test]
    fn extension_never_yields_clean_or_review_safe() {
        for ext in [
            "apk", "iso", "zip", "mp4", "vhdx", "tmp", "log", "bak", "cache", "xyz", "",
        ] {
            let c = classify(kind_from_extension(ext));
            assert_eq!(c.safety, Safety::RequiresReview, "{ext}");
        }
    }

    #[test]
    fn extension_labels() {
        assert_eq!(kind_from_extension("APK"), Apk);
        assert_eq!(kind_from_extension("iso"), DiskImage);
        assert_eq!(kind_from_extension("7z"), Archive);
        assert_eq!(kind_from_extension("MKV"), Video);
        assert_eq!(kind_from_extension("vhdx"), VirtualDisk);
        assert_eq!(kind_from_extension("dat"), LargeFile);
    }

    #[test]
    fn serialization() {
        assert_eq!(
            serde_json::to_string(&Safety::SafeToClean).unwrap(),
            "\"SAFE_TO_CLEAN\""
        );
        assert_eq!(
            serde_json::to_string(&Safety::DoNotTouch).unwrap(),
            "\"DO_NOT_TOUCH\""
        );
        assert_eq!(
            serde_json::to_string(&NodeModules).unwrap(),
            "\"nodeModules\""
        );
    }
}

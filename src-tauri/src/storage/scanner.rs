//! Read-only storage scanner. It measures and classifies; it never modifies anything.
//!
//! Scope: known developer/cache locations, large files in user folders, and project artifacts
//! (`node_modules`, build output, old APKs) under development roots. Protected system areas,
//! reparse points (symlinks, junctions, OneDrive placeholders) are never entered.

use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::governor::{CHECK_EVERY, Gate, GovernorHandle, PAUSE_POLL, RUN_SLEEP, THROTTLE_SLEEP};
use super::paths::{has_traversal, is_protected};
use super::safety::{EntryKind, Safety, classify, kind_from_extension};

const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub large_file_bytes: u64,
    pub max_depth: usize,
    pub time_budget: Duration,
    pub apk_min_age_days: u64,
    pub apk_min_bytes: u64,
    pub max_visited: usize,
    /// Asked every few hundred entries whether to run, slow down or pause. `None` (tests) never waits.
    pub governor: Option<GovernorHandle>,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            large_file_bytes: 500 * MIB,
            max_depth: 7,
            time_budget: Duration::from_secs(180),
            apk_min_age_days: 30,
            apk_min_bytes: 5 * MIB,
            max_visited: 8_000_000,
            governor: None,
        }
    }
}

/// Where to look. Built from the real environment, or from temp dirs in tests.
#[derive(Debug, Clone, Default)]
pub struct ScanEnv {
    pub user_profile: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    /// `%APPDATA%` (Roaming). Only the optional deep scan reads it.
    pub roaming_app_data: Option<PathBuf>,
    pub temp: Option<PathBuf>,
    /// Folders scanned for project artifacts.
    pub dev_roots: Vec<PathBuf>,
    /// Folders scanned for large personal files.
    pub user_folders: Vec<PathBuf>,
}

impl ScanEnv {
    pub fn from_system() -> Self {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let user_profile = var("USERPROFILE");
        let mut dev_roots = Vec::new();
        let mut user_folders = Vec::new();
        if let Some(p) = &user_profile {
            for d in [
                "source",
                "repos",
                "projects",
                "dev",
                "code",
                "work",
                "workspace",
                "Documents",
                "Desktop",
            ] {
                dev_roots.push(p.join(d));
            }
            for d in ["Downloads", "Desktop", "Documents", "Videos"] {
                user_folders.push(p.join(d));
            }
        }
        // Other fixed drives are commonly used for projects (for example `S:\Client Projects`).
        let system_drive = std::env::var("SystemDrive")
            .unwrap_or_else(|_| "C:".into())
            .to_lowercase();
        for letter in b'A'..=b'Z' {
            let root = format!("{}:\\", letter as char);
            if root.to_lowercase().starts_with(&system_drive) {
                continue;
            }
            if Path::new(&root).is_dir() {
                dev_roots.push(PathBuf::from(root));
            }
        }
        Self {
            user_profile,
            local_app_data: var("LOCALAPPDATA"),
            roaming_app_data: var("APPDATA"),
            temp: var("TEMP").or_else(|| var("TMP")),
            dev_roots,
            user_folders,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanEntry {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub file_count: u64,
    /// Unix seconds of the newest file inside (directories) or the file itself.
    pub last_modified_secs: Option<u64>,
    pub kind: EntryKind,
    pub kind_label: &'static str,
    pub safety: Safety,
    pub reason: &'static str,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub started_at_ms: u64,
    pub duration_ms: u64,
    /// False when cancelled or cut short by the time/entry budget.
    pub complete: bool,
    /// Entries or folders that could not be read (permissions, races).
    pub unreadable: usize,
    pub notes: Vec<String>,
    pub entries: Vec<ScanEntry>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub phase: String,
    pub current_path: String,
    pub entries_found: usize,
    pub items_visited: usize,
}

pub fn kind_label(kind: EntryKind) -> &'static str {
    use EntryKind::*;
    match kind {
        TempCache => "Temporary files",
        PackageCache => "Package manager cache",
        GradleCache => "Gradle cache",
        AndroidSdk => "Android SDK",
        AndroidAvd => "Android emulator images",
        FlutterCache => "Flutter/Dart cache",
        DockerData => "Docker data",
        WslDisk => "WSL disk",
        NodeModules => "Developer dependency",
        BuildOutput => "Build output",
        Apk => "APK",
        Archive => "Large archive",
        DiskImage => "Disk image",
        Video => "Video",
        Installer => "Installer",
        VirtualDisk => "Virtual disk",
        UserData => "Personal folder",
        LargeFile => "Large file",
        Unknown => "Unknown",
        AppData => "Application data",
    }
}

struct Ctx<'a> {
    cancel: &'a AtomicBool,
    deadline: Instant,
    max_visited: usize,
    progress: &'a Mutex<ScanProgress>,
    visited: AtomicUsize,
    unreadable: AtomicUsize,
    stopped: AtomicBool,
    governor: Option<GovernorHandle>,
    /// Time spent paused; it does not count against the time budget.
    paused_ms: AtomicU64,
}

impl Ctx<'_> {
    /// True when the scan must stop (cancelled, over budget, or too many entries).
    fn should_stop(&self) -> bool {
        if self.stopped.load(Ordering::Relaxed) {
            return true;
        }
        let v = self.visited.load(Ordering::Relaxed);
        if self.cancel.load(Ordering::Relaxed)
            || Instant::now()
                >= self.deadline + Duration::from_millis(self.paused_ms.load(Ordering::Relaxed))
            || v >= self.max_visited
        {
            self.stopped.store(true, Ordering::Relaxed);
            return true;
        }
        false
    }

    fn tick(&self, path: &Path) {
        let v = self.visited.fetch_add(1, Ordering::Relaxed) + 1;
        if v.is_multiple_of(2048)
            && let Ok(mut p) = self.progress.lock()
        {
            p.items_visited = v;
            p.current_path = path.to_string_lossy().into_owned();
        }
        if v.is_multiple_of(CHECK_EVERY) {
            self.govern();
        }
    }

    /// Slow down, or wait, according to the governor. Returns early when the scan is cancelled.
    fn govern(&self) {
        let Some(g) = &self.governor else {
            return;
        };
        match g.check() {
            Gate::Run => std::thread::sleep(RUN_SLEEP),
            Gate::Throttle(_) => std::thread::sleep(THROTTLE_SLEEP),
            Gate::Pause(reason) => {
                let began = Instant::now();
                let previous = self
                    .progress
                    .lock()
                    .map(|p| p.phase.clone())
                    .unwrap_or_default();
                while !self.cancel.load(Ordering::Relaxed) {
                    if let Ok(mut p) = self.progress.lock() {
                        p.phase = format!("Paused: {reason}");
                    }
                    std::thread::sleep(PAUSE_POLL);
                    if !matches!(g.check(), Gate::Pause(_)) {
                        break;
                    }
                }
                if self.cancel.load(Ordering::Relaxed) {
                    // Cancelled while waiting: stop now instead of finishing the walk.
                    self.stopped.store(true, Ordering::Relaxed);
                }
                self.paused_ms
                    .fetch_add(began.elapsed().as_millis() as u64, Ordering::Relaxed);
                if let Ok(mut p) = self.progress.lock() {
                    p.phase = previous;
                }
            }
        }
    }

    fn unreadable(&self) {
        self.unreadable.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(windows)]
fn skip_entry(md: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const REPARSE_POINT: u32 = 0x400;
    const OFFLINE: u32 = 0x1000;
    const RECALL_ON_OPEN: u32 = 0x40000;
    const RECALL_ON_DATA_ACCESS: u32 = 0x400000;
    md.file_attributes() & (REPARSE_POINT | OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS) != 0
}

#[cfg(not(windows))]
fn skip_entry(md: &Metadata) -> bool {
    md.file_type().is_symlink()
}

fn mtime_secs(md: &Metadata) -> Option<u64> {
    md.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

#[derive(Default, Clone, Copy)]
struct Sizing {
    bytes: u64,
    files: u64,
    newest: Option<u64>,
}

/// Sum a directory tree without following reparse points. Stops early when the context says so.
fn dir_size(root: &Path, ctx: &Ctx) -> Sizing {
    let mut total = Sizing::default();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if ctx.should_stop() {
            break;
        }
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => {
                ctx.unreadable();
                continue;
            }
        };
        for entry in rd.flatten() {
            ctx.tick(&entry.path());
            let Ok(md) = fs::symlink_metadata(entry.path()) else {
                ctx.unreadable();
                continue;
            };
            if skip_entry(&md) {
                continue;
            }
            if md.is_dir() {
                stack.push(entry.path());
            } else {
                total.bytes += md.len();
                total.files += 1;
                let m = mtime_secs(&md);
                total.newest = total.newest.max(m);
            }
        }
    }
    total
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.to_string_lossy().into_owned())
}

fn make_entry(path: &Path, is_dir: bool, s: Sizing, kind: EntryKind) -> ScanEntry {
    let c = classify(kind);
    ScanEntry {
        path: path.to_string_lossy().into_owned(),
        name: name_of(path),
        is_dir,
        size_bytes: s.bytes,
        file_count: s.files,
        last_modified_secs: s.newest,
        kind,
        kind_label: kind_label(kind),
        safety: c.safety,
        reason: c.reason,
    }
}

/// Fixed locations worth reporting. Only paths that exist are returned.
pub fn known_locations(env: &ScanEnv) -> Vec<(PathBuf, EntryKind)> {
    let mut v: Vec<(PathBuf, EntryKind)> = Vec::new();
    if let Some(t) = &env.temp {
        v.push((t.clone(), EntryKind::TempCache));
    }
    if let Some(l) = &env.local_app_data {
        for (rel, kind) in [
            ("npm-cache", EntryKind::PackageCache),
            ("pnpm\\store", EntryKind::PackageCache),
            ("pnpm-cache", EntryKind::PackageCache),
            ("Yarn\\Cache", EntryKind::PackageCache),
            ("Composer", EntryKind::PackageCache),
            ("pip\\cache", EntryKind::PackageCache),
            ("Pub\\Cache", EntryKind::FlutterCache),
            ("Android\\Sdk", EntryKind::AndroidSdk),
            ("Docker", EntryKind::DockerData),
        ] {
            v.push((l.join(rel), kind));
        }
        // WSL distributions (older Store layout and newer `wsl\{guid}` layout).
        for base in ["Packages", "wsl"] {
            if let Ok(rd) = fs::read_dir(l.join(base)) {
                for e in rd.flatten() {
                    for rel in ["LocalState\\ext4.vhdx", "ext4.vhdx"] {
                        v.push((e.path().join(rel), EntryKind::WslDisk));
                    }
                }
            }
        }
    }
    if let Some(p) = &env.user_profile {
        for (rel, kind) in [
            (".gradle\\caches", EntryKind::GradleCache),
            (".gradle\\wrapper\\dists", EntryKind::GradleCache),
            (".android\\avd", EntryKind::AndroidAvd),
            (".nuget\\packages", EntryKind::PackageCache),
            (".cargo\\registry", EntryKind::PackageCache),
            (".npm\\_cacache", EntryKind::PackageCache),
        ] {
            v.push((p.join(rel), kind));
        }
    }
    v.retain(|(p, _)| p.exists() && !has_traversal(p));
    v
}

/// Is `dir` (named `name`) a generated build-output folder, judged by a sibling marker file
/// so that an unrelated folder called `build` or `dist` is not mislabeled.
pub fn is_build_output(parent: &Path, name: &str) -> bool {
    let has = |m: &str| parent.join(m).exists();
    match name.to_ascii_lowercase().as_str() {
        ".next" | ".nuxt" | ".svelte-kit" | ".turbo" => has("package.json"),
        "dist" | "out" => has("package.json") || has("tsconfig.json"),
        "build" => {
            has("package.json")
                || has("build.gradle")
                || has("build.gradle.kts")
                || has("pubspec.yaml")
                || has("CMakeLists.txt")
        }
        "target" => has("Cargo.toml") || has("pom.xml"),
        ".dart_tool" => has("pubspec.yaml"),
        _ => false,
    }
}

fn is_old_apk(md: &Metadata, opts: &ScanOptions) -> bool {
    let Some(m) = mtime_secs(md) else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    md.len() >= opts.apk_min_bytes && now.saturating_sub(m) >= opts.apk_min_age_days * 86_400
}

/// Directory names never descended into during discovery.
fn skip_dir_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        ".git" | ".hg" | ".svn" | "$recycle.bin" | "system volume information"
    )
}

struct Walker<'a> {
    env: &'a ScanEnv,
    opts: &'a ScanOptions,
    ctx: &'a Ctx<'a>,
    out: &'a mut Vec<ScanEntry>,
}

impl Walker<'_> {
    fn note(&self, phase: &str) {
        if let Ok(mut p) = self.ctx.progress.lock() {
            p.phase = phase.into();
            p.entries_found = self.out.len();
        }
    }

    fn protected(&self, p: &Path) -> bool {
        is_protected(p, self.env.user_profile.as_deref())
    }

    /// Discover project artifacts and large files under `root`.
    /// `artifacts`: look for node_modules/build output/APKs. `large_files`: report big files.
    fn walk(&mut self, root: &Path, artifacts: bool, large_files: bool) {
        let mut stack = vec![(root.to_path_buf(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if self.ctx.should_stop() {
                return;
            }
            let rd = match fs::read_dir(&dir) {
                Ok(rd) => rd,
                Err(_) => {
                    self.ctx.unreadable();
                    continue;
                }
            };
            for entry in rd.flatten() {
                let path = entry.path();
                self.ctx.tick(&path);
                let Ok(md) = fs::symlink_metadata(&path) else {
                    self.ctx.unreadable();
                    continue;
                };
                if skip_entry(&md) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();

                if md.is_dir() {
                    if skip_dir_name(&name) || self.protected(&path) {
                        continue;
                    }
                    if artifacts && name.eq_ignore_ascii_case("node_modules") {
                        let s = dir_size(&path, self.ctx);
                        self.out
                            .push(make_entry(&path, true, s, EntryKind::NodeModules));
                        continue; // never look inside, nested node_modules are part of this one
                    }
                    if artifacts && is_build_output(&dir, &name) {
                        let s = dir_size(&path, self.ctx);
                        self.out
                            .push(make_entry(&path, true, s, EntryKind::BuildOutput));
                        continue;
                    }
                    if depth < self.opts.max_depth {
                        stack.push((path, depth + 1));
                    }
                } else {
                    let ext = path
                        .extension()
                        .map(|e| e.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let mut kind = kind_from_extension(&ext);
                    // An .exe/.msi is only an "installer" when it sits loose in a personal folder
                    // (not when it is a build artifact inside a project).
                    if !artifacts
                        && matches!(ext.to_ascii_lowercase().as_str(), "exe" | "msi" | "msix")
                    {
                        kind = EntryKind::Installer;
                    }
                    let big = large_files && md.len() >= self.opts.large_file_bytes;
                    let old_apk = artifacts && kind == EntryKind::Apk && is_old_apk(&md, self.opts);
                    if big || old_apk {
                        let s = Sizing {
                            bytes: md.len(),
                            files: 1,
                            newest: mtime_secs(&md),
                        };
                        self.out.push(make_entry(&path, false, s, kind));
                    }
                }
            }
        }
    }
}

/// Run a full scan. Blocking; call from a worker thread.
pub fn run_scan(
    env: &ScanEnv,
    opts: &ScanOptions,
    cancel: &AtomicBool,
    progress: &Mutex<ScanProgress>,
) -> ScanResult {
    let start = Instant::now();
    let started_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let ctx = Ctx {
        cancel,
        deadline: start + opts.time_budget,
        max_visited: opts.max_visited,
        progress,
        visited: AtomicUsize::new(0),
        unreadable: AtomicUsize::new(0),
        stopped: AtomicBool::new(false),
        governor: opts.governor.clone(),
        paused_ms: AtomicU64::new(0),
    };
    let mut entries: Vec<ScanEntry> = Vec::new();
    let mut notes = Vec::new();

    // 1. Known locations.
    if let Ok(mut p) = progress.lock() {
        *p = ScanProgress {
            phase: "Known developer locations".into(),
            ..Default::default()
        };
    }
    for (path, kind) in known_locations(env) {
        if ctx.should_stop() {
            break;
        }
        let md = fs::symlink_metadata(&path);
        let is_file = md.as_ref().map(|m| m.is_file()).unwrap_or(false);
        let s = if is_file {
            md.as_ref()
                .map(|m| Sizing {
                    bytes: m.len(),
                    files: 1,
                    newest: mtime_secs(m),
                })
                .unwrap_or_default()
        } else {
            dir_size(&path, &ctx)
        };
        if s.bytes > 0 {
            entries.push(make_entry(&path, !is_file, s, kind));
        }
    }

    // 2. User folders: report their size and large files inside.
    if let Ok(mut p) = progress.lock() {
        p.phase = "Large files in your folders".into();
        p.entries_found = entries.len();
    }
    for folder in &env.user_folders {
        if ctx.should_stop() || !folder.is_dir() {
            continue;
        }
        let s = dir_size(folder, &ctx);
        if s.bytes > 0 {
            entries.push(make_entry(folder, true, s, EntryKind::UserData));
        }
        let mut w = Walker {
            env,
            opts,
            ctx: &ctx,
            out: &mut entries,
        };
        w.walk(folder, false, true);
    }

    // 3. Development roots: node_modules, build output, old APKs, plus large files.
    if let Ok(mut p) = progress.lock() {
        p.phase = "Project artifacts".into();
        p.entries_found = entries.len();
    }
    for root in &env.dev_roots {
        if ctx.should_stop() || !root.is_dir() {
            continue;
        }
        if let Ok(mut p) = progress.lock() {
            p.current_path = root.to_string_lossy().into_owned();
        }
        let mut w = Walker {
            env,
            opts,
            ctx: &ctx,
            out: &mut entries,
        };
        w.note("Project artifacts");
        w.walk(root, true, true);
    }

    // The same path can be reached from overlapping roots (e.g. Documents is both a user folder and a dev root).
    entries.sort_by_key(|e| e.path.to_lowercase());
    entries.dedup_by(|a, b| a.path.eq_ignore_ascii_case(&b.path));
    entries.sort_by_key(|e| std::cmp::Reverse(e.size_bytes));

    let complete = !ctx.stopped.load(Ordering::Relaxed);
    if !complete {
        notes.push(if cancel.load(Ordering::Relaxed) {
            "Scan was cancelled; results are partial.".to_string()
        } else {
            "Scan stopped at its time or size limit; results are partial.".to_string()
        });
    }
    let unreadable = ctx.unreadable.load(Ordering::Relaxed);
    if unreadable > 0 {
        notes.push(format!(
            "{unreadable} items could not be read (permissions or changed during the scan)."
        ));
    }
    if let Ok(mut p) = progress.lock() {
        p.phase = if complete {
            "Done".into()
        } else {
            "Stopped".into()
        };
        p.entries_found = entries.len();
        p.items_visited = ctx.visited.load(Ordering::Relaxed);
    }

    ScanResult {
        started_at_ms,
        duration_ms: start.elapsed().as_millis() as u64,
        complete,
        unreadable,
        notes,
        entries,
    }
}

// --- optional deep AppData scan ------------------------------------------------------------------

/// Per-app folders reported by the deep scan must be at least this big.
const DEEP_MIN_BYTES: u64 = 100 * MIB;
/// A deep scan may run much longer than the normal one; it is throttled and pauses under pressure.
pub const DEEP_TIME_BUDGET: Duration = Duration::from_secs(15 * 60);
/// Rest between two top-level folders, on top of the governor's own pauses.
const DEEP_FOLDER_REST: Duration = Duration::from_millis(25);
/// How many app folders are reported.
const DEEP_MAX_ENTRIES: usize = 60;

/// AppData folders that hold Windows or system-owned state, not app data. They are never measured,
/// let alone listed: sizing them says nothing a user could act on, and some are sensitive.
const DEEP_SKIP: &[&str] = &[
    "microsoft",
    "packages",
    "temp",
    "connecteddevicesplatform",
    "publishers",
    "d3dscache",
    "comms",
    "programs",
    "placeholdertilelogofolder",
    "virtualstore",
    "windowsapps",
];

fn deep_skip(name: &str) -> bool {
    DEEP_SKIP.contains(&name.to_lowercase().as_str())
}

/// The three AppData roots (Local, Roaming, LocalLow) that exist.
pub fn deep_roots(env: &ScanEnv) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(l) = &env.local_app_data {
        roots.push(l.clone());
        if let Some(parent) = l.parent() {
            roots.push(parent.join("LocalLow"));
        }
    }
    if let Some(r) = &env.roaming_app_data {
        roots.push(r.clone());
    }
    roots.retain(|p| p.is_dir());
    roots.dedup();
    roots
}

/// Measure per-app folders under AppData. Only ever started by an explicit user request. Report-only:
/// entries are `AppData`, classified DO_NOT_TOUCH, so the cleanup action refuses them.
pub fn run_deep_appdata_scan(
    env: &ScanEnv,
    opts: &ScanOptions,
    cancel: &AtomicBool,
    progress: &Mutex<ScanProgress>,
) -> ScanResult {
    let start = Instant::now();
    let started_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let ctx = Ctx {
        cancel,
        deadline: start + opts.time_budget,
        max_visited: opts.max_visited,
        progress,
        visited: AtomicUsize::new(0),
        unreadable: AtomicUsize::new(0),
        stopped: AtomicBool::new(false),
        governor: opts.governor.clone(),
        paused_ms: AtomicU64::new(0),
    };
    let mut entries: Vec<ScanEntry> = Vec::new();
    let mut notes = vec![
        "Deep scan: only per-app folders over 100 MB are listed. Windows and system folders are skipped.".to_string(),
    ];

    'roots: for root in deep_roots(env) {
        let Ok(rd) = fs::read_dir(&root) else {
            ctx.unreadable();
            continue;
        };
        for entry in rd.flatten() {
            if ctx.should_stop() {
                break 'roots;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(md) = fs::symlink_metadata(&path) else {
                ctx.unreadable();
                continue;
            };
            // Files, links and junctions are not app folders; system-owned folders are skipped.
            if !md.is_dir()
                || skip_entry(&md)
                || deep_skip(&name)
                || is_protected(&path, env.user_profile.as_deref())
            {
                continue;
            }
            if let Ok(mut p) = progress.lock() {
                p.phase = "Deep scan of AppData".into();
                p.current_path = path.to_string_lossy().into_owned();
            }
            let sizing = dir_size(&path, &ctx);
            if sizing.bytes >= DEEP_MIN_BYTES {
                entries.push(make_entry(&path, true, sizing, EntryKind::AppData));
                if let Ok(mut p) = progress.lock() {
                    p.entries_found = entries.len();
                }
            }
            std::thread::sleep(DEEP_FOLDER_REST);
        }
    }

    entries.sort_by_key(|e| std::cmp::Reverse(e.size_bytes));
    entries.truncate(DEEP_MAX_ENTRIES);

    let complete = !ctx.stopped.load(Ordering::Relaxed);
    if !complete {
        notes.push(if cancel.load(Ordering::Relaxed) {
            "Deep scan was cancelled; results are partial.".to_string()
        } else {
            "Deep scan stopped at its time or size limit; results are partial.".to_string()
        });
    }
    let unreadable = ctx.unreadable.load(Ordering::Relaxed);
    if unreadable > 0 {
        notes.push(format!("{unreadable} items could not be read."));
    }
    if let Ok(mut p) = progress.lock() {
        p.phase = if complete {
            "Done".into()
        } else {
            "Stopped".into()
        };
        p.entries_found = entries.len();
        p.items_visited = ctx.visited.load(Ordering::Relaxed);
    }
    ScanResult {
        started_at_ms,
        duration_ms: start.elapsed().as_millis() as u64,
        complete,
        unreadable,
        notes,
        entries,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(path)
            .unwrap()
            .write_all(&vec![7u8; bytes])
            .unwrap();
    }

    fn scan(env: &ScanEnv, opts: &ScanOptions) -> ScanResult {
        run_scan(
            env,
            opts,
            &AtomicBool::new(false),
            &Mutex::new(ScanProgress::default()),
        )
    }

    fn small_opts() -> ScanOptions {
        ScanOptions {
            large_file_bytes: 1000,
            apk_min_bytes: 100,
            apk_min_age_days: 0,
            ..Default::default()
        }
    }

    // --- governor: pause, resume, throttle ----------------------------------------------------

    /// A project with a big node_modules, plus enough loose files that the walk passes several
    /// governor checks (one every `CHECK_EVERY` entries).
    fn busy_tree(root: &Path) -> ScanEnv {
        let proj = root.join("p");
        write(&proj.join("package.json"), 2);
        for i in 0..40 {
            write(
                &proj
                    .join("node_modules")
                    .join(format!("pkg{i}"))
                    .join("index.js"),
                50,
            );
        }
        for i in 0..900 {
            write(&proj.join("src").join(format!("f{i}.ts")), 1);
        }
        ScanEnv {
            dev_roots: vec![root.to_path_buf()],
            ..Default::default()
        }
    }

    #[test]
    fn a_paused_scan_waits_then_finishes_and_pause_time_is_not_charged_to_the_budget() {
        use crate::storage::governor::{Gate, GovernorHandle};
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool as Flag;

        let tmp = tempfile::tempdir().unwrap();
        let env = busy_tree(tmp.path());
        let pause = Arc::new(Flag::new(true));
        let p = pause.clone();
        let opts = ScanOptions {
            // A budget far shorter than the pause below.
            time_budget: Duration::from_millis(1500),
            governor: Some(GovernorHandle::new(move || {
                if p.load(Ordering::SeqCst) {
                    Gate::Pause("memory pressure is critical".into())
                } else {
                    Gate::Run
                }
            })),
            ..small_opts()
        };
        let progress = Arc::new(Mutex::new(ScanProgress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let (e, o, pr, c) = (env.clone(), opts.clone(), progress.clone(), cancel.clone());
        let handle = std::thread::spawn(move || run_scan(&e, &o, &c, &pr));

        // While critical pressure lasts the scan reports why it is waiting.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let phase = progress.lock().unwrap().phase.clone();
            if phase.starts_with("Paused:") {
                assert!(phase.contains("memory pressure is critical"), "{phase}");
                break;
            }
            assert!(Instant::now() < deadline, "never paused; phase = {phase}");
            std::thread::sleep(Duration::from_millis(20));
        }
        // Stay paused longer than the whole time budget, then let it continue.
        std::thread::sleep(Duration::from_millis(2200));
        assert!(!handle.is_finished(), "a paused scan makes no progress");
        pause.store(false, Ordering::SeqCst);
        let r = handle.join().unwrap();
        assert!(
            r.complete,
            "pause time must not use up the budget: {:?}",
            r.notes
        );
        assert!(find(&r, "node_modules").is_some());
        assert_ne!(
            progress.lock().unwrap().phase,
            "Paused: memory pressure is critical"
        );
    }

    #[test]
    fn cancelling_a_paused_scan_ends_it_promptly_with_a_partial_result() {
        use crate::storage::governor::{Gate, GovernorHandle};
        use std::sync::Arc;

        let tmp = tempfile::tempdir().unwrap();
        let env = busy_tree(tmp.path());
        let opts = ScanOptions {
            governor: Some(GovernorHandle::new(|| {
                Gate::Pause("a flutter build is running".into())
            })),
            ..small_opts()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(ScanProgress::default()));
        let (e, o, pr, c) = (env, opts, progress.clone(), cancel.clone());
        let handle = std::thread::spawn(move || run_scan(&e, &o, &c, &pr));
        std::thread::sleep(Duration::from_millis(700));
        let began = Instant::now();
        cancel.store(true, Ordering::SeqCst);
        let r = handle.join().unwrap();
        assert!(
            began.elapsed() < Duration::from_secs(3),
            "cancel must not wait for the pause to end"
        );
        assert!(!r.complete);
        assert!(r.notes.iter().any(|n| n.contains("cancelled")));
    }

    #[test]
    fn throttling_slows_the_scan_but_does_not_change_what_it_finds() {
        use crate::storage::governor::{Gate, GovernorHandle};

        let tmp = tempfile::tempdir().unwrap();
        let env = busy_tree(tmp.path());
        let free = scan(&env, &small_opts());
        let throttled_opts = ScanOptions {
            governor: Some(GovernorHandle::new(|| {
                Gate::Throttle("memory pressure is high".into())
            })),
            ..small_opts()
        };
        let began = Instant::now();
        let slow = scan(&env, &throttled_opts);
        // ~1000 entries / 256 per check = a few checks of 15 ms each: measurable, never wrong.
        assert!(
            began.elapsed() >= Duration::from_millis(30),
            "{:?}",
            began.elapsed()
        );
        assert!(slow.complete);
        let paths = |r: &ScanResult| {
            r.entries
                .iter()
                .map(|e| (e.path.clone(), e.size_bytes))
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(&free), paths(&slow));
    }

    #[test]
    fn without_a_governor_nothing_waits() {
        let tmp = tempfile::tempdir().unwrap();
        let env = busy_tree(tmp.path());
        let began = Instant::now();
        assert!(scan(&env, &small_opts()).complete);
        assert!(began.elapsed() < Duration::from_secs(2));
    }

    // --- deep AppData scan ---------------------------------------------------------------------

    fn big_file(path: &Path, mib: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::File::create(path).unwrap().set_len(mib * MIB).unwrap();
    }

    /// `<tmp>\AppData\{Local,Roaming,LocalLow}` with a few app folders.
    fn appdata(root: &Path) -> ScanEnv {
        let ad = root.join("AppData");
        big_file(&ad.join("Local").join("Slack").join("cache.bin"), 150);
        big_file(&ad.join("Local").join("TinyApp").join("x.bin"), 1);
        big_file(
            &ad.join("Local")
                .join("Microsoft")
                .join("Windows")
                .join("big.bin"),
            300,
        );
        big_file(
            &ad.join("Local").join("Packages").join("p").join("big.bin"),
            300,
        );
        big_file(
            &ad.join("Roaming").join("Code").join("User").join("db.bin"),
            120,
        );
        big_file(&ad.join("LocalLow").join("Game").join("save.bin"), 110);
        big_file(&ad.join("Local").join("loose-file.bin"), 200);
        ScanEnv {
            user_profile: Some(root.to_path_buf()),
            local_app_data: Some(ad.join("Local")),
            roaming_app_data: Some(ad.join("Roaming")),
            ..Default::default()
        }
    }

    fn deep(env: &ScanEnv, opts: &ScanOptions) -> ScanResult {
        run_deep_appdata_scan(
            env,
            opts,
            &AtomicBool::new(false),
            &Mutex::new(ScanProgress::default()),
        )
    }

    #[test]
    fn deep_scan_lists_big_app_folders_from_all_three_roots_and_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        let env = appdata(tmp.path());
        let r = deep(&env, &ScanOptions::default());
        let names: Vec<&str> = r.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Slack", "Code", "Game"],
            "largest first, small and loose files left out"
        );
        assert!(r.complete);
        for e in &r.entries {
            assert_eq!(e.kind, EntryKind::AppData);
            assert_eq!(
                e.safety,
                Safety::DoNotTouch,
                "{} must be report-only",
                e.name
            );
        }
    }

    #[test]
    fn deep_scan_never_measures_windows_owned_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let env = appdata(tmp.path());
        let r = deep(&env, &ScanOptions::default());
        for skipped in ["Microsoft", "Packages", "Temp"] {
            assert!(r.entries.iter().all(|e| e.name != skipped), "{skipped}");
        }
        assert!(
            deep_skip("MICROSOFT") && deep_skip("packages"),
            "case-insensitive"
        );
        assert!(!deep_skip("Slack"));
    }

    #[test]
    fn deep_scan_is_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        let env = appdata(tmp.path());
        let before = walk_all(&tmp.path().join("AppData"));
        deep(&env, &ScanOptions::default());
        assert_eq!(before, walk_all(&tmp.path().join("AppData")));
    }

    #[test]
    fn deep_scan_without_appdata_is_empty_not_an_error() {
        let r = deep(&ScanEnv::default(), &ScanOptions::default());
        assert!(r.entries.is_empty() && r.complete);
    }

    #[test]
    fn a_paused_deep_scan_can_be_cancelled() {
        use crate::storage::governor::{Gate, GovernorHandle};
        use std::sync::Arc;

        let tmp = tempfile::tempdir().unwrap();
        // Enough entries in one folder to reach a governor check.
        let env = appdata(tmp.path());
        for i in 0..600 {
            write(
                &tmp.path()
                    .join("AppData")
                    .join("Local")
                    .join("Slack")
                    .join(format!("f{i}")),
                1,
            );
        }
        let opts = ScanOptions {
            governor: Some(GovernorHandle::new(|| {
                Gate::Pause("memory pressure is critical".into())
            })),
            ..ScanOptions::default()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(ScanProgress::default()));
        let (e, o, c, p) = (env, opts, cancel.clone(), progress.clone());
        let handle = std::thread::spawn(move || run_deep_appdata_scan(&e, &o, &c, &p));
        std::thread::sleep(Duration::from_millis(800));
        assert!(progress.lock().unwrap().phase.starts_with("Paused:"));
        cancel.store(true, Ordering::SeqCst);
        let r = handle.join().unwrap();
        assert!(!r.complete);
    }

    fn find<'a>(r: &'a ScanResult, suffix: &str) -> Option<&'a ScanEntry> {
        r.entries
            .iter()
            .find(|e| e.path.replace('/', "\\").ends_with(suffix))
    }

    #[test]
    fn finds_node_modules_with_size_count_and_review_safety() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("projects");
        write(&root.join("app\\package.json"), 10);
        write(&root.join("app\\node_modules\\a\\index.js"), 400);
        write(
            &root.join("app\\node_modules\\a\\node_modules\\b\\x.js"),
            600,
        );
        let env = ScanEnv {
            dev_roots: vec![root.clone()],
            ..Default::default()
        };
        let r = scan(&env, &small_opts());

        let nm: Vec<_> = r
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::NodeModules)
            .collect();
        assert_eq!(
            nm.len(),
            1,
            "nested node_modules must not be reported separately"
        );
        assert_eq!(nm[0].size_bytes, 1000);
        assert_eq!(nm[0].file_count, 2);
        assert_eq!(nm[0].safety, Safety::RequiresReview);
        assert_eq!(nm[0].kind_label, "Developer dependency");
        assert!(nm[0].last_modified_secs.is_some());
        assert!(r.complete);
    }

    #[test]
    fn build_output_needs_a_project_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        write(&root.join("web\\package.json"), 5);
        write(&root.join("web\\dist\\bundle.js"), 50);
        write(&root.join("web\\.next\\cache\\x"), 70);
        write(&root.join("rust\\Cargo.toml"), 5);
        write(&root.join("rust\\target\\debug\\app.exe"), 90);
        write(&root.join("random\\dist\\notes.txt"), 33); // no marker
        write(&root.join("random\\build\\notes.txt"), 33);
        let env = ScanEnv {
            dev_roots: vec![root],
            ..Default::default()
        };
        let r = scan(&env, &small_opts());

        assert!(
            find(&r, "web\\dist").is_some_and(
                |e| e.kind == EntryKind::BuildOutput && e.safety == Safety::SafeToReview
            )
        );
        assert!(find(&r, "web\\.next").is_some());
        assert!(find(&r, "rust\\target").is_some());
        assert!(find(&r, "random\\dist").is_none());
        assert!(find(&r, "random\\build").is_none());
    }

    #[test]
    fn large_files_are_labeled_by_role_but_only_require_review() {
        let tmp = tempfile::tempdir().unwrap();
        let dl = tmp.path().join("Downloads");
        write(&dl.join("os.iso"), 2000);
        write(&dl.join("movie.mp4"), 3000);
        write(&dl.join("data.bin"), 2500);
        write(&dl.join("small.txt"), 10);
        let env = ScanEnv {
            user_folders: vec![dl],
            ..Default::default()
        };
        let r = scan(&env, &small_opts());

        let kinds: Vec<_> = r
            .entries
            .iter()
            .filter(|e| !e.is_dir)
            .map(|e| (e.name.as_str(), e.kind))
            .collect();
        assert!(kinds.contains(&("os.iso", EntryKind::DiskImage)));
        assert!(kinds.contains(&("movie.mp4", EntryKind::Video)));
        assert!(kinds.contains(&("data.bin", EntryKind::LargeFile)));
        assert!(!kinds.iter().any(|(n, _)| *n == "small.txt"));
        assert!(
            r.entries
                .iter()
                .filter(|e| !e.is_dir)
                .all(|e| e.safety == Safety::RequiresReview)
        );
    }

    #[test]
    fn loose_installers_in_personal_folders_only() {
        let tmp = tempfile::tempdir().unwrap();
        let dl = tmp.path().join("Downloads");
        let proj = tmp.path().join("proj");
        write(&dl.join("setup.exe"), 2000);
        write(&proj.join("app.exe"), 2000);
        let env = ScanEnv {
            user_folders: vec![dl],
            dev_roots: vec![proj],
            ..Default::default()
        };
        let r = scan(&env, &small_opts());
        assert_eq!(
            find(&r, "setup.exe").map(|e| e.kind),
            Some(EntryKind::Installer)
        );
        assert_eq!(
            find(&r, "app.exe").map(|e| e.kind),
            Some(EntryKind::LargeFile)
        );
    }

    #[test]
    fn user_folder_itself_is_reported_as_do_not_touch() {
        let tmp = tempfile::tempdir().unwrap();
        let docs = tmp.path().join("Documents");
        write(&docs.join("thesis.docx"), 300);
        let env = ScanEnv {
            user_folders: vec![docs],
            ..Default::default()
        };
        let r = scan(&env, &small_opts());
        let e = find(&r, "Documents").unwrap();
        assert_eq!(
            (e.kind, e.safety, e.size_bytes),
            (EntryKind::UserData, Safety::DoNotTouch, 300)
        );
    }

    #[test]
    fn old_apks_reported_only_when_big_and_old_enough() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        write(&root.join("app-release.apk"), 500);
        write(&root.join("tiny.apk"), 10);
        let env = ScanEnv {
            dev_roots: vec![root.clone()],
            ..Default::default()
        };
        let r = scan(&env, &small_opts());
        assert!(find(&r, "app-release.apk").is_some_and(|e| e.kind == EntryKind::Apk));
        assert!(find(&r, "tiny.apk").is_none());

        // Same file, but the age gate is 30 days and the file is brand new.
        let strict = ScanOptions {
            apk_min_age_days: 30,
            ..small_opts()
        };
        assert!(find(&scan(&env, &strict), "app-release.apk").is_none());
    }

    #[test]
    fn known_locations_classified_and_missing_ones_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let local = tmp.path().join("Local");
        let temp = local.join("Temp");
        write(&temp.join("a.tmp"), 100);
        write(&local.join("npm-cache\\_cacache\\x"), 200);
        write(&local.join("Android\\Sdk\\platforms\\x"), 300);
        write(&local.join("Docker\\wsl\\disk\\docker_data.vhdx"), 400);
        let env = ScanEnv {
            local_app_data: Some(local),
            temp: Some(temp),
            ..Default::default()
        };
        let r = scan(&env, &small_opts());

        let safety = |suffix: &str| find(&r, suffix).map(|e| e.safety);
        assert_eq!(safety("Temp"), Some(Safety::SafeToClean));
        assert_eq!(safety("npm-cache"), Some(Safety::SafeToClean));
        assert_eq!(safety("Sdk"), Some(Safety::DoNotTouch));
        assert_eq!(safety("Docker"), Some(Safety::DoNotTouch));
        assert!(
            find(&r, "pnpm-cache").is_none(),
            "missing locations are not reported"
        );
    }

    #[test]
    fn protected_directories_are_not_entered() {
        let tmp = tempfile::tempdir().unwrap();
        // A folder literally named like a protected top-level dir, but not at a drive root, is fine;
        // the real guard is exercised through `is_protected` on drive-level names in paths.rs.
        let root = tmp.path().join("p");
        write(&root.join(".git\\objects\\big"), 5000);
        write(&root.join("src\\app.js"), 10);
        let env = ScanEnv {
            dev_roots: vec![root],
            ..Default::default()
        };
        let r = scan(&env, &small_opts());
        assert!(
            r.entries.iter().all(|e| !e.path.contains(".git")),
            ".git is never entered"
        );
    }

    #[test]
    fn cancelled_scan_returns_partial_results_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        for i in 0..20 {
            write(&root.join(format!("app{i}\\package.json")), 5);
            write(&root.join(format!("app{i}\\node_modules\\x\\y.js")), 50);
        }
        let env = ScanEnv {
            dev_roots: vec![root],
            ..Default::default()
        };
        let cancel = AtomicBool::new(true);
        let r = run_scan(
            &env,
            &small_opts(),
            &cancel,
            &Mutex::new(ScanProgress::default()),
        );
        assert!(!r.complete);
        assert!(r.notes.iter().any(|n| n.contains("cancelled")));
        assert!(r.entries.is_empty());
    }

    #[test]
    fn tiny_budget_stops_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        for i in 0..50 {
            write(&root.join(format!("d{i}\\f.txt")), 5);
        }
        let env = ScanEnv {
            dev_roots: vec![root],
            ..Default::default()
        };
        let opts = ScanOptions {
            max_visited: 10,
            ..small_opts()
        };
        let r = scan(&env, &opts);
        assert!(!r.complete);
        assert!(r.notes.iter().any(|n| n.contains("limit")));
    }

    #[test]
    fn overlapping_roots_do_not_duplicate_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let docs = tmp.path().join("Documents");
        write(&docs.join("proj\\package.json"), 5);
        write(&docs.join("proj\\node_modules\\a\\b.js"), 50);
        let env = ScanEnv {
            dev_roots: vec![docs.clone(), docs.clone()],
            user_folders: vec![docs],
            ..Default::default()
        };
        let r = scan(&env, &small_opts());
        assert_eq!(
            r.entries
                .iter()
                .filter(|e| e.kind == EntryKind::NodeModules)
                .count(),
            1
        );
    }

    #[test]
    fn entries_sorted_by_size_descending_and_scan_is_read_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        write(&root.join("a\\package.json"), 5);
        write(&root.join("a\\node_modules\\x"), 100);
        write(&root.join("b\\package.json"), 5);
        write(&root.join("b\\node_modules\\x"), 900);
        let before: Vec<_> = walk_all(&root);
        let env = ScanEnv {
            dev_roots: vec![root.clone()],
            ..Default::default()
        };
        let r = scan(&env, &small_opts());
        assert!(
            r.entries
                .windows(2)
                .all(|w| w[0].size_bytes >= w[1].size_bytes)
        );
        assert_eq!(
            before,
            walk_all(&root),
            "scanner must not change the file system"
        );
    }

    fn walk_all(root: &Path) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let md = e.metadata().unwrap();
                out.push((e.path().to_string_lossy().into_owned(), md.len()));
                if md.is_dir() {
                    stack.push(e.path());
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn build_output_markers() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("package.json"), 1);
        assert!(is_build_output(tmp.path(), "dist"));
        assert!(is_build_output(tmp.path(), "BUILD"));
        assert!(is_build_output(tmp.path(), ".next"));
        assert!(!is_build_output(tmp.path(), "target"));
        assert!(!is_build_output(tmp.path(), "src"));
    }

    #[test]
    fn reports_progress() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("p");
        write(&root.join("a\\package.json"), 5);
        write(&root.join("a\\node_modules\\x"), 100);
        let env = ScanEnv {
            dev_roots: vec![root],
            ..Default::default()
        };
        let progress = Mutex::new(ScanProgress::default());
        let r = run_scan(&env, &small_opts(), &AtomicBool::new(false), &progress);
        let p = progress.lock().unwrap();
        assert_eq!(p.phase, "Done");
        assert_eq!(p.entries_found, r.entries.len());
    }
}

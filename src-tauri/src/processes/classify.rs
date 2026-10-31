//! Process classification. Pure function of (name, exe path, command line).
//!
//! Rules are tried in priority order. Within a rule, path and command-line evidence is preferred
//! over a bare name match, and the evidence used is reported so later engines can weigh it.
//! Example: a `java.exe` is JAVA by name only; with `org.gradle` and `android` in its command line
//! it becomes ANDROID with command-line evidence.

use super::model::{Category, Classification, Evidence};

struct Input {
    /// Lowercase file name without `.exe`.
    name: String,
    /// Lowercase path with `\` separators, if readable.
    path: Option<String>,
    /// Lowercase joined command line (unredacted: used only for matching, never stored).
    cmd: String,
}

impl Input {
    fn path_has(&self, needle: &str) -> bool {
        self.path.as_deref().is_some_and(|p| p.contains(needle))
    }

    fn cmd_has(&self, needle: &str) -> bool {
        self.cmd.contains(needle)
    }

    fn name_is(&self, names: &[&str]) -> bool {
        names.contains(&self.name.as_str())
    }

    fn name_starts(&self, prefixes: &[&str]) -> bool {
        prefixes.iter().any(|p| self.name.starts_with(p))
    }
}

fn hit(category: Category, evidence: Evidence) -> Option<Classification> {
    Some(Classification { category, evidence })
}

/// Best evidence for a rule: path first, then command line, then bare name.
fn by_evidence(
    i: &Input,
    category: Category,
    path_needles: &[&str],
    cmd_needles: &[&str],
    names: &[&str],
) -> Option<Classification> {
    if path_needles.iter().any(|n| i.path_has(n)) {
        return hit(category, Evidence::Path);
    }
    if cmd_needles.iter().any(|n| i.cmd_has(n)) {
        return hit(category, Evidence::CommandLine);
    }
    if i.name_is(names) {
        return hit(category, Evidence::Name);
    }
    None
}

fn docker(i: &Input) -> Option<Classification> {
    if i.path_has("\\program files\\docker\\") || i.path_has("\\docker desktop\\") {
        return hit(Category::Docker, Evidence::Path);
    }
    if i.name_starts(&["docker", "com.docker."])
        || i.name_is(&[
            "vmmem",
            "vmmemwsl",
            "vmcompute",
            "wsl",
            "wslhost",
            "wslservice",
            "wslrelay",
        ])
    {
        return hit(Category::Docker, Evidence::Name);
    }
    None
}

fn android(i: &Input) -> Option<Classification> {
    let gradle = i.cmd_has("gradledaemon")
        || i.cmd_has("gradle-launcher")
        || i.cmd_has("org.gradle")
        || i.cmd_has("gradlewrappermain");
    if gradle && (i.cmd_has("android") || i.path_has("android")) {
        return hit(Category::Android, Evidence::CommandLine);
    }
    if i.path_has("\\platform-tools\\")
        || i.path_has("\\android\\sdk\\")
        || i.path_has("\\android\\emulator\\")
    {
        return hit(Category::Android, Evidence::Path);
    }
    if i.name_starts(&["qemu-system"]) && i.cmd_has("-avd") {
        return hit(Category::Android, Evidence::CommandLine);
    }
    if i.name_is(&["adb", "emulator"]) {
        return hit(Category::Android, Evidence::Name);
    }
    None
}

fn flutter(i: &Input) -> Option<Classification> {
    by_evidence(
        i,
        Category::Flutter,
        &["\\flutter\\bin\\", "\\flutter\\"],
        &["flutter_tools", "\\flutter\\bin"],
        &["flutter", "dart", "flutter_tester", "flutter-tester"],
    )
}

fn ide(i: &Input) -> Option<Classification> {
    by_evidence(
        i,
        Category::Ide,
        &[
            "\\microsoft vs code",
            "\\kiro\\",
            "\\android studio\\",
            "\\jetbrains\\",
            "\\cursor\\",
            "\\visual studio\\",
        ],
        &[],
        &[
            "code",
            "code - insiders",
            "kiro",
            "studio64",
            "idea64",
            "pycharm64",
            "webstorm64",
            "phpstorm64",
            "rider64",
            "clion64",
            "devenv",
            "cursor",
        ],
    )
}

fn php(i: &Input) -> Option<Classification> {
    by_evidence(
        i,
        Category::Php,
        &["\\bin\\php\\", "\\php\\php"],
        &["artisan"],
        &["php", "php-cgi", "php-fpm", "php-win"],
    )
}

fn database(i: &Input) -> Option<Classification> {
    const NAMES: &[&str] = &[
        "mysqld",
        "mysqld-nt",
        "mariadbd",
        "postgres",
        "mongod",
        "redis-server",
        "sqlservr",
        "memcached",
    ];
    if !i.name_is(NAMES) {
        return None;
    }
    let dir_evidence = [
        "\\mysql\\",
        "\\mariadb\\",
        "\\postgresql\\",
        "\\mongodb\\",
        "\\redis\\",
    ]
    .iter()
    .any(|d| i.path_has(d));
    hit(
        Category::Database,
        if dir_evidence {
            Evidence::Path
        } else {
            Evidence::Name
        },
    )
}

fn node(i: &Input) -> Option<Classification> {
    if i.path_has("\\nodejs\\") || i.path_has("\\node-v") {
        return hit(Category::Node, Evidence::Path);
    }
    if i.name_is(&["node", "npm", "npx", "pnpm", "yarn", "bun"]) {
        // Next.js/Vite/etc. in the command line strengthens the case but keeps the category.
        let framework = ["next", "vite", "webpack", "nuxt", "astro"]
            .iter()
            .any(|f| i.cmd_has(f));
        return hit(
            Category::Node,
            if framework {
                Evidence::CommandLine
            } else {
                Evidence::Name
            },
        );
    }
    None
}

fn java(i: &Input) -> Option<Classification> {
    if i.name_is(&["java", "javaw", "javaws"])
        && (i.path_has("\\jdk") || i.path_has("\\jbr\\") || i.path_has("\\java\\"))
    {
        return hit(Category::Java, Evidence::Path);
    }
    if i.cmd_has("org.gradle") || i.cmd_has("gradledaemon") {
        return hit(Category::Java, Evidence::CommandLine);
    }
    if i.name_is(&[
        "java",
        "javaw",
        "javaws",
        "gradle",
        "gradlew",
        "mvn",
        "mvnw",
        "kotlin-daemon",
    ]) {
        return hit(Category::Java, Evidence::Name);
    }
    None
}

fn browser(i: &Input) -> Option<Classification> {
    by_evidence(
        i,
        Category::Browser,
        &[
            "\\google\\chrome\\",
            "\\microsoft\\edge\\",
            "\\edgewebview\\",
            "\\mozilla firefox\\",
            "\\bravesoftware\\",
            "\\opera",
            "\\vivaldi\\",
        ],
        &[],
        &[
            "chrome",
            "msedge",
            "msedgewebview2",
            "firefox",
            "brave",
            "opera",
            "vivaldi",
            "iexplore",
            "arc",
        ],
    )
}

fn development(i: &Input) -> Option<Classification> {
    if i.name_is(&[
        "git", "cargo", "rustc", "rustup", "python", "pythonw", "py", "go", "dotnet", "msbuild",
        "composer", "nginx", "httpd", "apache", "caddy", "laragon", "tauri", "cmake", "ninja",
        "gcc", "clang",
    ]) {
        return hit(Category::Development, Evidence::Name);
    }
    // Anything else shipped inside Laragon's tree (nginx, apache, helpers).
    if i.path_has("\\laragon\\") {
        return hit(Category::Development, Evidence::Path);
    }
    None
}

fn system(i: &Input) -> Option<Classification> {
    if let Some(p) = i.path.as_deref() {
        // "x:\windows\..." on any drive letter.
        let after_drive = p.get(1..).filter(|r| r.starts_with(":\\")).map(|r| &r[2..]);
        if after_drive.is_some_and(|r| r.starts_with("windows\\"))
            || p.contains("\\windows defender\\")
        {
            return hit(Category::System, Evidence::Path);
        }
    }
    if i.name_is(&[
        "system",
        "system idle process",
        "registry",
        "memory compression",
        "secure system",
        "smss",
        "csrss",
        "wininit",
        "services",
        "lsass",
        "svchost",
        "dwm",
        "winlogon",
        "fontdrvhost",
        "msmpeng",
    ]) {
        return hit(Category::System, Evidence::Name);
    }
    None
}

/// Classify a process. `exe_path` and `cmdline` may be unavailable (protected processes).
pub fn classify(name: &str, exe_path: Option<&str>, cmdline: &str) -> Classification {
    let lower_name = name.to_lowercase();
    let input = Input {
        name: lower_name
            .strip_suffix(".exe")
            .unwrap_or(&lower_name)
            .to_string(),
        path: exe_path.map(|p| p.to_lowercase().replace('/', "\\")),
        cmd: cmdline.to_lowercase().replace('/', "\\"),
    };

    let rules: [fn(&Input) -> Option<Classification>; 11] = [
        docker,
        android,
        flutter,
        ide,
        php,
        database,
        node,
        java,
        browser,
        development,
        system,
    ];
    rules
        .iter()
        .find_map(|rule| rule(&input))
        .unwrap_or(Classification::OTHER)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, path: Option<&str>, cmd: &str) -> (Category, Evidence) {
        let r = classify(name, path, cmd);
        (r.category, r.evidence)
    }

    #[test]
    fn docker_family() {
        assert_eq!(
            c("Docker Desktop.exe", None, ""),
            (Category::Docker, Evidence::Name)
        );
        assert_eq!(
            c("com.docker.backend.exe", None, ""),
            (Category::Docker, Evidence::Name)
        );
        assert_eq!(c("vmmem", None, ""), (Category::Docker, Evidence::Name));
        assert_eq!(c("vmmemWSL", None, ""), (Category::Docker, Evidence::Name));
        assert_eq!(
            c(
                "backend.exe",
                Some("C:\\Program Files\\Docker\\Docker\\resources\\backend.exe"),
                ""
            ),
            (Category::Docker, Evidence::Path)
        );
    }

    #[test]
    fn node_by_name_vs_framework_vs_path() {
        assert_eq!(
            c("node.exe", None, "node server.js"),
            (Category::Node, Evidence::Name)
        );
        assert_eq!(
            c("node.exe", None, "node node_modules/next/dist/bin/next dev"),
            (Category::Node, Evidence::CommandLine)
        );
        assert_eq!(
            c(
                "node.exe",
                Some("C:\\laragon\\bin\\nodejs\\node-v22\\node.exe"),
                ""
            ),
            (Category::Node, Evidence::Path)
        );
    }

    #[test]
    fn node_inside_vscode_is_ide_not_node() {
        let r = c(
            "node.exe",
            Some(
                "C:\\Users\\a\\AppData\\Local\\Programs\\Microsoft VS Code\\resources\\app\\node.exe",
            ),
            "",
        );
        assert_eq!(r, (Category::Ide, Evidence::Path));
    }

    #[test]
    fn java_plain_vs_gradle_vs_android_gradle() {
        assert_eq!(
            c("java.exe", None, "java -jar app.jar"),
            (Category::Java, Evidence::Name)
        );
        assert_eq!(
            c(
                "java.exe",
                None,
                "java org.gradle.launcher.daemon.bootstrap.GradleDaemon 8.5"
            ),
            (Category::Java, Evidence::CommandLine)
        );
        assert_eq!(
            c(
                "java.exe",
                None,
                "java -Dorg.gradle.appname=gradlew org.gradle.launcher.daemon.bootstrap.GradleDaemon C:\\Users\\a\\AppData\\Local\\Android\\Sdk"
            ),
            (Category::Android, Evidence::CommandLine)
        );
    }

    #[test]
    fn android_tools() {
        assert_eq!(c("adb.exe", None, ""), (Category::Android, Evidence::Name));
        assert_eq!(
            c(
                "adb.exe",
                Some("C:\\Users\\a\\AppData\\Local\\Android\\Sdk\\platform-tools\\adb.exe"),
                ""
            ),
            (Category::Android, Evidence::Path)
        );
        assert_eq!(
            c(
                "qemu-system-x86_64.exe",
                None,
                "qemu-system-x86_64 -avd Pixel_7"
            ),
            (Category::Android, Evidence::CommandLine)
        );
        // Plain qemu without an AVD is not Android.
        assert_eq!(
            c("qemu-system-x86_64.exe", None, "qemu-system-x86_64 -m 2G").0,
            Category::Other
        );
    }

    #[test]
    fn android_studio_is_ide_but_its_gradle_daemon_is_android() {
        assert_eq!(
            c(
                "studio64.exe",
                Some("C:\\Program Files\\Android\\Android Studio\\bin\\studio64.exe"),
                ""
            ),
            (Category::Ide, Evidence::Path)
        );
        let r = c(
            "java.exe",
            Some("C:\\Program Files\\Android\\Android Studio\\jbr\\bin\\java.exe"),
            "java org.gradle.launcher.daemon.bootstrap.GradleDaemon android",
        );
        assert_eq!(r.0, Category::Android);
    }

    #[test]
    fn flutter_family() {
        assert_eq!(
            c("flutter_tester.exe", None, ""),
            (Category::Flutter, Evidence::Name)
        );
        assert_eq!(
            c(
                "dart.exe",
                Some("C:\\src\\flutter\\bin\\cache\\dart-sdk\\bin\\dart.exe"),
                ""
            ),
            (Category::Flutter, Evidence::Path)
        );
    }

    #[test]
    fn php_and_artisan() {
        assert_eq!(
            c("php.exe", None, "php -v"),
            (Category::Php, Evidence::Name)
        );
        assert_eq!(
            c(
                "php.exe",
                Some("C:\\laragon\\bin\\php\\php-8.3\\php.exe"),
                "php artisan serve"
            ),
            (Category::Php, Evidence::Path)
        );
        assert_eq!(
            c("something.exe", None, "x artisan queue:work"),
            (Category::Php, Evidence::CommandLine)
        );
    }

    #[test]
    fn databases() {
        assert_eq!(
            c("mysqld.exe", None, ""),
            (Category::Database, Evidence::Name)
        );
        assert_eq!(
            c(
                "mysqld.exe",
                Some("C:\\laragon\\bin\\mysql\\mysql-8\\bin\\mysqld.exe"),
                ""
            ),
            (Category::Database, Evidence::Path)
        );
        assert_eq!(c("redis-server.exe", None, "").0, Category::Database);
    }

    #[test]
    fn ides() {
        assert_eq!(
            c("Code.exe", None, "--type=renderer"),
            (Category::Ide, Evidence::Name)
        );
        assert_eq!(c("Kiro.exe", None, ""), (Category::Ide, Evidence::Name));
        assert_eq!(
            c(
                "Code.exe",
                Some("C:\\Users\\a\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe"),
                ""
            ),
            (Category::Ide, Evidence::Path)
        );
    }

    #[test]
    fn browsers() {
        assert_eq!(c("chrome.exe", None, "").0, Category::Browser);
        assert_eq!(c("msedge.exe", None, "").0, Category::Browser);
        assert_eq!(
            c(
                "firefox.exe",
                Some("C:\\Program Files\\Mozilla Firefox\\firefox.exe"),
                ""
            ),
            (Category::Browser, Evidence::Path)
        );
    }

    #[test]
    fn system_by_path_and_name() {
        assert_eq!(
            c(
                "svchost.exe",
                Some("C:\\Windows\\System32\\svchost.exe"),
                ""
            ),
            (Category::System, Evidence::Path)
        );
        assert_eq!(
            c(
                "conhost.exe",
                Some("D:\\Windows\\System32\\conhost.exe"),
                ""
            ),
            (Category::System, Evidence::Path)
        );
        assert_eq!(c("System", None, ""), (Category::System, Evidence::Name));
        assert_eq!(
            c("MsMpEng.exe", None, ""),
            (Category::System, Evidence::Name)
        );
    }

    #[test]
    fn laragon_tree_is_development() {
        assert_eq!(
            c("nginx.exe", None, ""),
            (Category::Development, Evidence::Name)
        );
        assert_eq!(
            c(
                "whatever.exe",
                Some("C:\\laragon\\bin\\x\\whatever.exe"),
                ""
            ),
            (Category::Development, Evidence::Path)
        );
    }

    #[test]
    fn unknown_is_other_with_no_evidence() {
        assert_eq!(
            c("randomapp.exe", None, ""),
            (Category::Other, Evidence::None)
        );
        assert_eq!(c("", None, ""), (Category::Other, Evidence::None));
    }

    #[test]
    fn name_alone_does_not_override_better_evidence() {
        // Named like a system process but living in a user folder: not SYSTEM by path,
        // and "svchost" by name is only name-level evidence.
        let r = classify(
            "svchost.exe",
            Some("C:\\Users\\a\\Downloads\\svchost.exe"),
            "",
        );
        assert_eq!(r.evidence, Evidence::Name);
    }

    #[test]
    fn matching_is_case_and_slash_insensitive() {
        assert_eq!(
            c(
                "NODE.EXE",
                Some("C:/Laragon/bin/NodeJS/node-v22/node.exe"),
                ""
            )
            .1,
            Evidence::Path
        );
    }
}

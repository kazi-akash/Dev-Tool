//! "Start DevPilot with Windows": one value in the current user's `Run` key.
//!
//! * Only the user can turn it on: the sole writer is the `set_start_with_windows` command, called
//!   from the Settings switch. Nothing at startup or install time writes it (a test enforces this).
//! * Per user (HKCU): no administrator rights, no machine-wide change.
//! * The registry is the source of truth, not a copy in localStorage, so the switch always shows
//!   what Windows will really do. A value that points at a different copy of DevPilot is reported
//!   as such and fixed by switching it off and on.
//! * The launch command carries `--minimized`: tray and monitoring start, the main window does not.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Command-line flag that starts DevPilot in the tray without opening the window.
pub const MINIMIZED_FLAG: &str = "--minimized";
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "DevPilot";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartStatus {
    pub enabled: bool,
    /// The registered command starts *this* copy of DevPilot. `false` after the app was moved,
    /// reinstalled elsewhere, or when another copy registered itself.
    pub points_to_this_app: bool,
    /// What Windows will run, for display.
    pub command: Option<String>,
}

/// `"C:\Path With Spaces\devpilot.exe" --minimized`
pub fn run_command(exe: &Path) -> String {
    format!("\"{}\" {MINIMIZED_FLAG}", exe.display())
}

/// The executable named by a stored command (quoted, or up to the first space when unquoted).
pub fn parse_exe(command: &str) -> Option<PathBuf> {
    let c = command.trim();
    let exe = match c.strip_prefix('"') {
        Some(rest) => rest.split('"').next()?,
        None => c.split_whitespace().next()?,
    };
    (!exe.is_empty()).then(|| PathBuf::from(exe))
}

fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| p.to_string_lossy().to_lowercase().replace('/', "\\");
    n(a) == n(b)
}

/// True when this process was started by the autostart entry (or asked to stay in the tray).
pub fn started_minimized<I: IntoIterator<Item = String>>(args: I) -> bool {
    args.into_iter().any(|a| a == MINIMIZED_FLAG)
}

pub fn status_from(stored: Option<String>, this_exe: &Path) -> AutostartStatus {
    let points_to_this_app = stored
        .as_deref()
        .and_then(parse_exe)
        .is_some_and(|e| same_path(&e, this_exe));
    AutostartStatus {
        enabled: stored.is_some(),
        points_to_this_app,
        command: stored,
    }
}

/// Where the value lives. Production uses the real `Run` key; tests use a throwaway key.
pub struct Autostart {
    subkey: String,
    exe: PathBuf,
}

impl Autostart {
    pub fn for_this_app() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| format!("cannot find this program: {e}"))?;
        Ok(Self {
            subkey: RUN_KEY.into(),
            exe,
        })
    }

    #[cfg(test)]
    fn with_key(subkey: &str, exe: PathBuf) -> Self {
        Self {
            subkey: subkey.into(),
            exe,
        }
    }
}

#[cfg(windows)]
impl Autostart {
    fn key(&self, write: bool) -> std::io::Result<winreg::RegKey> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        if write {
            hkcu.create_subkey(&self.subkey).map(|(k, _)| k)
        } else {
            hkcu.open_subkey_with_flags(&self.subkey, KEY_READ | KEY_SET_VALUE)
                .or_else(|_| hkcu.open_subkey_with_flags(&self.subkey, KEY_READ))
        }
    }

    pub fn status(&self) -> AutostartStatus {
        let stored: Option<String> = self
            .key(false)
            .ok()
            .and_then(|k| k.get_value(VALUE_NAME).ok());
        status_from(stored, &self.exe)
    }

    /// Turn the entry on or off. Switching on always (re)writes the command for this copy.
    pub fn set_enabled(&self, on: bool) -> Result<(), String> {
        let k = self
            .key(true)
            .map_err(|e| format!("cannot open the Windows startup list: {e}"))?;
        if on {
            k.set_value(VALUE_NAME, &run_command(&self.exe))
                .map_err(|e| format!("cannot add DevPilot to Windows startup: {e}"))
        } else {
            match k.delete_value(VALUE_NAME) {
                Ok(()) => Ok(()),
                // Already absent is the state the user asked for.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("cannot remove DevPilot from Windows startup: {e}")),
            }
        }
    }
}

#[cfg(not(windows))]
impl Autostart {
    pub fn status(&self) -> AutostartStatus {
        status_from(None, &self.exe)
    }
    pub fn set_enabled(&self, _on: bool) -> Result<(), String> {
        Err("Start with Windows is only available on Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_quotes_the_path_and_carries_the_minimized_flag() {
        let c = run_command(Path::new("C:\\Program Files\\DevPilot\\devpilot.exe"));
        assert_eq!(
            c,
            "\"C:\\Program Files\\DevPilot\\devpilot.exe\" --minimized"
        );
    }

    #[test]
    fn parse_exe_handles_quoted_and_unquoted_commands() {
        assert_eq!(
            parse_exe("\"C:\\A B\\devpilot.exe\" --minimized"),
            Some(PathBuf::from("C:\\A B\\devpilot.exe"))
        );
        assert_eq!(
            parse_exe("C:\\x\\devpilot.exe --minimized"),
            Some(PathBuf::from("C:\\x\\devpilot.exe"))
        );
        assert_eq!(parse_exe(""), None);
        assert_eq!(parse_exe("\"\""), None);
    }

    #[test]
    fn status_distinguishes_off_this_app_and_another_copy() {
        let me = Path::new("C:\\DevPilot\\devpilot.exe");
        assert_eq!(
            status_from(None, me),
            AutostartStatus {
                enabled: false,
                points_to_this_app: false,
                command: None
            }
        );
        let mine = status_from(Some(run_command(me)), me);
        assert!(mine.enabled && mine.points_to_this_app);
        let other = status_from(Some(run_command(Path::new("D:\\old\\devpilot.exe"))), me);
        assert!(other.enabled && !other.points_to_this_app);
        // Windows paths are case-insensitive.
        assert!(
            status_from(
                Some(run_command(Path::new("c:\\devpilot\\DEVPILOT.EXE"))),
                me
            )
            .points_to_this_app
        );
    }

    #[test]
    fn minimized_flag_detection_is_exact() {
        assert!(started_minimized([
            "devpilot.exe".into(),
            "--minimized".into()
        ]));
        assert!(!started_minimized(["devpilot.exe".into()]));
        assert!(!started_minimized([
            "--minimizedx".into(),
            "minimized".into()
        ]));
    }

    /// Real registry, throwaway key under HKCU: proves the write, read, repoint and delete paths.
    #[cfg(windows)]
    #[test]
    fn round_trips_in_the_real_registry() {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let subkey = format!(
            "Software\\DevPilotTest\\DevPilotAutostart{}",
            std::process::id()
        );
        let exe = PathBuf::from("C:\\Test\\devpilot.exe");
        let a = Autostart::with_key(&subkey, exe.clone());
        assert!(!a.status().enabled);
        a.set_enabled(true).unwrap();
        let s = a.status();
        assert!(s.enabled && s.points_to_this_app);
        assert_eq!(
            s.command.as_deref(),
            Some("\"C:\\Test\\devpilot.exe\" --minimized")
        );

        // Another copy takes over; switching on again repoints it to this one.
        let other = Autostart::with_key(&subkey, PathBuf::from("D:\\Other\\devpilot.exe"));
        other.set_enabled(true).unwrap();
        assert!(!a.status().points_to_this_app);
        a.set_enabled(true).unwrap();
        assert!(a.status().points_to_this_app);

        a.set_enabled(false).unwrap();
        assert!(!a.status().enabled);
        a.set_enabled(false).unwrap(); // already off is fine
        let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all("Software\\DevPilotTest");
    }

    #[test]
    fn nothing_but_the_settings_command_can_enable_it() {
        let others = [
            ("lib.rs", include_str!("../lib.rs")),
            ("windows/tray.rs", include_str!("tray.rs")),
            ("windows/window.rs", include_str!("window.rs")),
            (
                "monitoring/service.rs",
                include_str!("../monitoring/service.rs"),
            ),
            ("prefs.rs", include_str!("../prefs.rs")),
            ("commands/window.rs", include_str!("../commands/window.rs")),
            (
                "commands/actions.rs",
                include_str!("../commands/actions.rs"),
            ),
        ];
        for (name, src) in others {
            let prod = src.split("#[cfg(test)]").next().unwrap();
            assert!(
                !prod.contains("Autostart"),
                "{name} must not touch the startup entry"
            );
        }
        // The single writer: the user-triggered command.
        let sys = include_str!("../commands/system.rs");
        let sys = sys.split("#[cfg(test)]").next().unwrap();
        assert_eq!(
            sys.matches("Autostart::for_this_app()").count(),
            2,
            "get + set only"
        );
        assert_eq!(sys.matches("a.set_enabled(enabled)").count(), 1);
    }
}

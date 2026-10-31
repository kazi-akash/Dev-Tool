//! Command lines often carry secrets (`--token abc`, `PASSWORD=...`). Redact before storing or sending anywhere.

const MAX_LEN: usize = 300;
const REDACTED: &str = "***";

/// Substrings that mark an argument name as sensitive (matched case-insensitively).
const SENSITIVE_KEYS: &[&str] = &[
    "password",
    "passwd",
    "pwd",
    "secret",
    "token",
    "apikey",
    "api-key",
    "api_key",
    "auth",
    "credential",
    "private-key",
    "private_key",
    "access-key",
    "access_key",
    "bearer",
];

fn is_sensitive_key(key: &str) -> bool {
    let k = key.trim_start_matches('-').to_ascii_lowercase();
    SENSITIVE_KEYS.iter().any(|s| k.contains(s))
}

/// Redact one argument given the previous one (for `--token value` style pairs).
/// Returns the text to emit and whether the *next* argument must be redacted too.
fn redact_arg(arg: &str, redact_this: bool) -> (String, bool) {
    if redact_this {
        return (REDACTED.to_string(), false);
    }
    // KEY=value (also --key=value)
    if let Some((key, _)) = arg.split_once('=') {
        if is_sensitive_key(key) {
            return (format!("{key}={REDACTED}"), false);
        }
        return (arg.to_string(), false);
    }
    // --token <next>
    if arg.starts_with('-') && is_sensitive_key(arg) {
        return (arg.to_string(), true);
    }
    // Bare credentials embedded in URLs: scheme://user:pass@host
    if let Some(scheme_end) = arg.find("://") {
        let rest = &arg[scheme_end + 3..];
        if let Some(at) = rest.find('@') {
            let creds = &rest[..at];
            if !rest[..at].contains('/') && creds.contains(':') {
                return (
                    format!("{}{REDACTED}@{}", &arg[..scheme_end + 3], &rest[at + 1..]),
                    false,
                );
            }
        }
    }
    (arg.to_string(), false)
}

/// Join arguments into one display string with secrets masked and length capped.
pub fn redact_command_line(args: &[String]) -> Option<String> {
    if args.is_empty() {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(args.len());
    let mut redact_next = false;
    for arg in args {
        let (text, next) = redact_arg(arg, redact_next);
        redact_next = next;
        out.push(text);
    }
    let joined = out.join(" ");
    if joined.chars().count() <= MAX_LEN {
        return Some(joined);
    }
    let truncated: String = joined.chars().take(MAX_LEN).collect();
    Some(format!("{truncated}..."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(args: &[&str]) -> String {
        let v: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        redact_command_line(&v).unwrap()
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(redact_command_line(&[]), None);
    }

    #[test]
    fn plain_args_unchanged() {
        assert_eq!(
            r(&["node", "server.js", "--port", "3000"]),
            "node server.js --port 3000"
        );
    }

    #[test]
    fn flag_then_value_is_masked() {
        assert_eq!(
            r(&["app", "--token", "abc123", "--verbose"]),
            "app --token *** --verbose"
        );
        assert_eq!(r(&["app", "--api-key", "k"]), "app --api-key ***");
    }

    #[test]
    fn key_equals_value_is_masked() {
        assert_eq!(r(&["app", "--password=hunter2"]), "app --password=***");
        assert_eq!(r(&["php", "DB_PASSWORD=x"]), "php DB_PASSWORD=***");
    }

    #[test]
    fn non_sensitive_key_value_kept() {
        assert_eq!(
            r(&["app", "--port=3000", "NODE_ENV=dev"]),
            "app --port=3000 NODE_ENV=dev"
        );
    }

    #[test]
    fn url_credentials_are_masked() {
        assert_eq!(
            r(&["git", "clone", "https://user:pass@github.com/x/y"]),
            "git clone https://***@github.com/x/y"
        );
    }

    #[test]
    fn url_without_credentials_kept() {
        assert_eq!(
            r(&["curl", "https://example.com/a@b"]),
            "curl https://example.com/a@b"
        );
    }

    #[test]
    fn long_command_lines_are_truncated() {
        let long = "x".repeat(1000);
        let out = r(&["app", &long]);
        assert!(out.chars().count() <= MAX_LEN + 3);
        assert!(out.ends_with("..."));
    }

    #[test]
    fn secret_value_never_survives() {
        let out = r(&[
            "tool",
            "-authorization",
            "Bearer SECRETVALUE",
            "--secret=SECRETVALUE",
        ]);
        assert!(!out.contains("SECRETVALUE"), "{out}");
    }
}

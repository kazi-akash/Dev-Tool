//! Text sanitizer for anything that may leave the machine. Conservative: when in doubt, redact.

const REDACTED: &str = "[redacted]";
const MAX_TEXT: usize = 600;

/// Known secret prefixes (API keys and tokens of common services).
const SECRET_PREFIXES: &[&str] = &[
    "sk-",
    "sk_",
    "ghp_",
    "gho_",
    "ghs_",
    "github_pat_",
    "xox",
    "akia",
    "asia",
    "aiza",
    "glpat-",
    "npm_",
    "eyj",
];

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '=' | '.')
}

/// Long, mixed letter/digit strings look like keys, hashes or tokens.
fn looks_like_secret(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    if SECRET_PREFIXES.iter().any(|p| lower.starts_with(p)) && word.len() >= 12 {
        return true;
    }
    if word.len() >= 24 {
        let has_digit = word.chars().any(|c| c.is_ascii_digit());
        let has_alpha = word.chars().any(|c| c.is_ascii_alphabetic());
        return has_digit && has_alpha;
    }
    false
}

/// Replace `C:\Users\<name>\...` with `C:\Users\<user>\...` (any drive, either slash).
fn mask_user_names(s: &str) -> String {
    let bytes = s.as_bytes();
    let lower = s.to_ascii_lowercase();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let rest = &lower[i..];
        let hit = ["\\users\\", "/users/"]
            .iter()
            .find(|p| rest.starts_with(**p));
        if let Some(p) = hit {
            out.push_str(&s[i..i + p.len()]);
            i += p.len();
            let sep = if p.starts_with('\\') { b'\\' } else { b'/' };
            let end = bytes[i..]
                .iter()
                .position(|&b| b == sep || b == b' ' || b == b'"' || b == b'\'')
                .map_or(s.len(), |n| i + n);
            if end > i {
                out.push_str("<user>");
            }
            i = end;
        } else {
            let ch = s[i..].chars().next().unwrap_or(' ');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn mask_emails(s: &str) -> String {
    s.split_inclusive(char::is_whitespace)
        .map(|chunk| {
            let word = chunk.trim_end();
            let trailing = &chunk[word.len()..];
            match word.find('@') {
                Some(at) if at > 0 && word[at + 1..].contains('.') && !word.contains("://") => {
                    format!("[email]{trailing}")
                }
                _ => chunk.to_string(),
            }
        })
        .collect()
}

fn mask_secrets(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut word = String::new();
    let mut after_bearer = false;
    let flush = |word: &mut String, out: &mut String, after_bearer: &mut bool| {
        if word.is_empty() {
            return;
        }
        let w = std::mem::take(word);
        if *after_bearer || looks_like_secret(&w) {
            out.push_str(REDACTED);
        } else {
            out.push_str(&w);
        }
        *after_bearer = w.eq_ignore_ascii_case("bearer") && !*after_bearer;
    };
    for c in s.chars() {
        if is_token_char(c) {
            word.push(c);
        } else {
            flush(&mut word, &mut out, &mut after_bearer);
            // `Bearer <token>`: the separator keeps the flag, a token resets it.
            out.push(c);
        }
    }
    flush(&mut word, &mut out, &mut after_bearer);
    out
}

/// Sanitize free text: user names in paths, e-mail addresses, tokens and keys; cap the length.
pub fn sanitize_text(input: &str) -> String {
    let s = mask_user_names(input);
    let s = mask_emails(&s);
    let s = mask_secrets(&s);
    if s.chars().count() > MAX_TEXT {
        let cut: String = s.chars().take(MAX_TEXT).collect();
        format!("{cut}...")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_unchanged() {
        let t =
            "RAM is at 96%. Docker/WSL2 and Android Gradle build account for approximately 8.9 GB.";
        assert_eq!(sanitize_text(t), t);
    }

    #[test]
    fn user_names_in_paths_are_masked() {
        assert_eq!(
            sanitize_text("C:\\Users\\alice\\AppData\\Local\\Temp"),
            "C:\\Users\\<user>\\AppData\\Local\\Temp"
        );
        assert_eq!(
            sanitize_text("c:/users/Bob Smith/x"),
            "c:/users/<user> Smith/x"
        );
        assert_eq!(sanitize_text("D:\\USERS\\carol"), "D:\\USERS\\<user>");
        assert!(!sanitize_text("C:\\Users\\alice").contains("alice"));
    }

    #[test]
    fn api_keys_and_tokens_are_masked() {
        for secret in [
            "sk-ant-api03-AbCdEf1234567890abcdef",
            "sk-proj-1234567890abcdefghij",
            "ghp_1234567890abcdefghijklmnop",
            "AKIAIOSFODNN7EXAMPLE",
            "xoxb-123456789012-abcdefghij",
            "eyJhbGciOiJIUzI1NiJ9.payload.sig",
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b",
        ] {
            let out = sanitize_text(&format!("token is {secret} ok"));
            assert!(!out.contains(secret), "{secret} -> {out}");
            assert!(out.contains("[redacted]"), "{out}");
            assert!(out.starts_with("token is ") && out.ends_with(" ok"));
        }
    }

    #[test]
    fn bearer_values_are_masked_even_if_short() {
        let out = sanitize_text("Authorization: Bearer abc123");
        assert!(!out.contains("abc123"), "{out}");
        assert!(out.contains("Bearer"));
    }

    #[test]
    fn emails_are_masked_but_urls_and_versions_survive() {
        assert_eq!(
            sanitize_text("contact dev@example.com now"),
            "contact [email] now"
        );
        assert!(sanitize_text("https://example.com/a@b").contains("https://example.com/a@b"));
        assert!(sanitize_text("node v22.13.1 and 1.2.3").contains("v22.13.1"));
    }

    #[test]
    fn ordinary_long_words_are_not_over_redacted() {
        let t = "supercalifragilisticexpialidocious memory growth";
        assert_eq!(sanitize_text(t), t, "no digits, so not a token");
        assert_eq!(
            sanitize_text("approximately 1234567890 bytes"),
            "approximately 1234567890 bytes"
        );
    }

    #[test]
    fn long_text_is_capped() {
        let out = sanitize_text(&"word ".repeat(500));
        assert!(out.chars().count() <= MAX_TEXT + 3);
        assert!(out.ends_with("..."));
    }

    #[test]
    fn handles_unicode_and_empty() {
        assert_eq!(sanitize_text(""), "");
        assert_eq!(sanitize_text("Größe 5 GB — fine"), "Größe 5 GB — fine");
        assert_eq!(sanitize_text("C:\\Users\\"), "C:\\Users\\");
    }

    #[test]
    fn idempotent() {
        let once = sanitize_text("C:\\Users\\alice\\x token sk-abcdef1234567890abcd a@b.co");
        assert_eq!(sanitize_text(&once), once);
    }
}

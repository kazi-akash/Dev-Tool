//! AI provider interface plus the two wire formats. Request building and response parsing are pure
//! (no network), so they are fully testable; `client.rs` does the sending.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::payload::AiPayload;
use super::sanitize::sanitize_text;

pub const MAX_OUTPUT_TOKENS: u32 = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    /// OpenAI and anything speaking its chat-completions format (also Ollama, LM Studio, vLLM...).
    OpenAi,
    /// Anthropic Messages API and compatible gateways.
    Anthropic,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "code", content = "message", rename_all = "camelCase")]
pub enum AiError {
    NotConfigured(String),
    InvalidConfig(String),
    Network(String),
    Auth(String),
    RateLimited(String),
    Provider(String),
    BadResponse(String),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (AiError::NotConfigured(m)
        | AiError::InvalidConfig(m)
        | AiError::Network(m)
        | AiError::Auth(m)
        | AiError::RateLimited(m)
        | AiError::Provider(m)
        | AiError::BadResponse(m)) = self;
        write!(f, "{m}")
    }
}

pub trait AiProvider {
    fn build_request(&self, cfg: &AiConfig, api_key: &str, system: &str, user: &str)
    -> HttpRequest;
    fn parse_response(&self, body: &str) -> Result<String, AiError>;
}

pub struct OpenAiCompatible;
pub struct AnthropicCompatible;

pub fn provider_for(kind: ProviderKind) -> &'static dyn AiProvider {
    match kind {
        ProviderKind::OpenAi => &OpenAiCompatible,
        ProviderKind::Anthropic => &AnthropicCompatible,
    }
}

fn join_url(base: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

fn error_message(v: &Value) -> Option<String> {
    let e = v.get("error")?;
    let msg = e
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| e.as_str())?;
    // The provider's message may echo input; keep it short and sanitized.
    Some(sanitize_text(msg).chars().take(200).collect())
}

impl AiProvider for OpenAiCompatible {
    fn build_request(
        &self,
        cfg: &AiConfig,
        api_key: &str,
        system: &str,
        user: &str,
    ) -> HttpRequest {
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        // Local servers (Ollama) need no key.
        if !api_key.is_empty() {
            headers.push(("Authorization".into(), format!("Bearer {api_key}")));
        }
        HttpRequest {
            url: join_url(&cfg.base_url, "chat/completions"),
            headers,
            body: json!({
                "model": cfg.model,
                "temperature": 0.2,
                "max_tokens": MAX_OUTPUT_TOKENS,
                "messages": [
                    { "role": "system", "content": system },
                    { "role": "user", "content": user },
                ],
            }),
        }
    }

    fn parse_response(&self, body: &str) -> Result<String, AiError> {
        let v: Value = serde_json::from_str(body)
            .map_err(|_| AiError::BadResponse("The provider did not return JSON.".into()))?;
        if let Some(m) = error_message(&v) {
            return Err(AiError::Provider(m));
        }
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim();
        if text.is_empty() {
            return Err(AiError::BadResponse(
                "The provider returned an empty answer.".into(),
            ));
        }
        Ok(text.to_string())
    }
}

impl AiProvider for AnthropicCompatible {
    fn build_request(
        &self,
        cfg: &AiConfig,
        api_key: &str,
        system: &str,
        user: &str,
    ) -> HttpRequest {
        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("anthropic-version".into(), "2023-06-01".into()),
        ];
        if !api_key.is_empty() {
            headers.push(("x-api-key".into(), api_key.to_string()));
        }
        HttpRequest {
            url: join_url(&cfg.base_url, "v1/messages"),
            headers,
            body: json!({
                "model": cfg.model,
                "max_tokens": MAX_OUTPUT_TOKENS,
                "temperature": 0.2,
                "system": system,
                "messages": [ { "role": "user", "content": user } ],
            }),
        }
    }

    fn parse_response(&self, body: &str) -> Result<String, AiError> {
        let v: Value = serde_json::from_str(body)
            .map_err(|_| AiError::BadResponse("The provider did not return JSON.".into()))?;
        if let Some(m) = error_message(&v) {
            return Err(AiError::Provider(m));
        }
        let text: String = v["content"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter(|p| p["type"] == "text")
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let text = text.trim();
        if text.is_empty() {
            return Err(AiError::BadResponse(
                "The provider returned an empty answer.".into(),
            ));
        }
        Ok(text.to_string())
    }
}

/// Base URL rules: HTTPS for anything remote; plain HTTP only for this machine (Ollama and friends).
/// No credentials in the URL, no query or fragment.
pub fn validate_config(cfg: &AiConfig) -> Result<(), AiError> {
    let bad = |m: &str| Err(AiError::InvalidConfig(m.to_string()));
    let url = cfg.base_url.trim();
    let (scheme, rest) = match url.split_once("://") {
        Some(p) => p,
        None => {
            return bad("The base URL must start with https:// (or http:// for a local server).");
        }
    };
    if rest.is_empty() || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return bad("The base URL is not valid.");
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return bad("The base URL must not contain credentials.");
    }
    if rest.contains('?') || rest.contains('#') {
        return bad("The base URL must not contain a query or fragment.");
    }
    let host = authority
        .rsplit_once(':')
        .filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
        .map_or(authority, |(h, _)| h);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let local = matches!(
        host.to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "::1"
    );
    match scheme.to_ascii_lowercase().as_str() {
        "https" => {}
        "http" if local => {}
        "http" => {
            return bad(
                "Remote providers must use https://. Plain http:// is only allowed for localhost.",
            );
        }
        _ => return bad("Unsupported URL scheme."),
    }
    if host.is_empty() {
        return bad("The base URL has no host.");
    }
    let model = cfg.model.trim();
    if model.is_empty()
        || model.len() > 100
        || !model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '/'))
    {
        return bad("The model name is empty or contains unsupported characters.");
    }
    Ok(())
}

pub const SYSTEM_PROMPT: &str = "You are DevPilot's troubleshooting assistant for Windows developer machines. \
You receive a summarized, sanitized snapshot of one PC: RAM, CPU, disks, developer workloads \
(Docker/WSL2, Android/Gradle, Next.js, Laragon...), top processes and a rule-engine diagnosis. \
Answer these, in this order, in plain language and under 250 words: \
1) What is happening? 2) Why is it happening? 3) Which process or workload is most likely responsible? \
4) What should the user do? 5) What is the safest action? 6) What could happen if they do nothing? \
Rules: rely only on the snapshot; say when you are unsure and never invent processes, paths or numbers; \
do not call a memory growth a leak without evidence; never ask for files, credentials or source code; \
you cannot run commands, so only propose steps. The user decides and DevPilot performs any action after the user approves it.";

/// The user message sent to the model: the payload JSON plus an optional (sanitized) question.
pub fn build_user_message(payload: &AiPayload, question: Option<&str>) -> String {
    let json = serde_json::to_string_pretty(payload).unwrap_or_default();
    let q = question
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(sanitize_text);
    match q {
        Some(q) => format!("Snapshot:\n{json}\n\nUser question: {q}"),
        None => format!("Snapshot:\n{json}\n\nExplain what is happening and what to do."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(kind: ProviderKind, url: &str, model: &str) -> AiConfig {
        AiConfig {
            kind,
            base_url: url.into(),
            model: model.into(),
        }
    }

    #[test]
    fn openai_request_shape() {
        let c = cfg(
            ProviderKind::OpenAi,
            "https://api.openai.com/v1/",
            "gpt-4o-mini",
        );
        let r = provider_for(c.kind).build_request(&c, "sk-test-123456789012", "SYS", "USER");
        assert_eq!(r.url, "https://api.openai.com/v1/chat/completions");
        assert!(
            r.headers
                .contains(&("Authorization".into(), "Bearer sk-test-123456789012".into()))
        );
        assert_eq!(r.body["model"], "gpt-4o-mini");
        assert_eq!(r.body["messages"][0]["role"], "system");
        assert_eq!(r.body["messages"][1]["content"], "USER");
        assert_eq!(r.body["max_tokens"], MAX_OUTPUT_TOKENS);
    }

    #[test]
    fn anthropic_request_shape() {
        let c = cfg(
            ProviderKind::Anthropic,
            "https://api.anthropic.com",
            "claude-sonnet-5-5",
        );
        let r = provider_for(c.kind).build_request(&c, "sk-ant-key-0123456789", "SYS", "USER");
        assert_eq!(r.url, "https://api.anthropic.com/v1/messages");
        assert!(
            r.headers
                .contains(&("x-api-key".into(), "sk-ant-key-0123456789".into()))
        );
        assert!(
            r.headers
                .iter()
                .any(|(k, v)| k == "anthropic-version" && v == "2023-06-01")
        );
        assert_eq!(r.body["system"], "SYS");
        assert_eq!(r.body["messages"][0]["role"], "user");
        assert!(
            r.body["messages"].as_array().unwrap().len() == 1,
            "system prompt is a top-level field"
        );
    }

    #[test]
    fn api_key_only_ever_goes_in_a_header_never_in_the_body_or_url() {
        for kind in [ProviderKind::OpenAi, ProviderKind::Anthropic] {
            let c = cfg(kind, "https://example.com", "m");
            let r = provider_for(kind).build_request(&c, "SUPERSECRETKEY123", "s", "u");
            assert!(!r.url.contains("SUPERSECRETKEY123"));
            assert!(!r.body.to_string().contains("SUPERSECRETKEY123"));
        }
    }

    #[test]
    fn no_key_means_no_auth_header_for_local_servers() {
        let c = cfg(ProviderKind::OpenAi, "http://localhost:11434/v1", "llama3");
        let r = provider_for(c.kind).build_request(&c, "", "s", "u");
        assert!(r.headers.iter().all(|(k, _)| k != "Authorization"));
        assert_eq!(r.url, "http://localhost:11434/v1/chat/completions");
    }

    #[test]
    fn parses_openai_and_anthropic_answers() {
        let o = r#"{"choices":[{"message":{"role":"assistant","content":"  Close Docker.  "}}]}"#;
        assert_eq!(OpenAiCompatible.parse_response(o).unwrap(), "Close Docker.");
        let a = r#"{"content":[{"type":"text","text":"Part one."},{"type":"tool_use","id":"x"},{"type":"text","text":"Part two."}]}"#;
        assert_eq!(
            AnthropicCompatible.parse_response(a).unwrap(),
            "Part one.\nPart two."
        );
    }

    #[test]
    fn provider_errors_and_garbage_are_reported_not_trusted() {
        let e = r#"{"error":{"message":"Invalid API key: sk-live-ABCDEFGHIJKLMNOP12345"}}"#;
        let err = OpenAiCompatible.parse_response(e).unwrap_err();
        assert!(
            matches!(&err, AiError::Provider(m) if !m.contains("ABCDEFGHIJKLMNOP")),
            "{err:?}"
        );
        assert!(matches!(
            AnthropicCompatible
                .parse_response(r#"{"type":"error","error":{"type":"x","message":"overloaded"}}"#),
            Err(AiError::Provider(_))
        ));
        assert!(matches!(
            OpenAiCompatible.parse_response("<html>"),
            Err(AiError::BadResponse(_))
        ));
        assert!(matches!(
            OpenAiCompatible.parse_response(r#"{"choices":[]}"#),
            Err(AiError::BadResponse(_))
        ));
        assert!(matches!(
            AnthropicCompatible.parse_response(r#"{"content":[]}"#),
            Err(AiError::BadResponse(_))
        ));
        assert!(matches!(
            OpenAiCompatible.parse_response(r#"{"choices":[{"message":{"content":"   "}}]}"#),
            Err(AiError::BadResponse(_))
        ));
    }

    #[test]
    fn base_url_validation() {
        let ok = |u: &str| validate_config(&cfg(ProviderKind::OpenAi, u, "gpt-4o"));
        for good in [
            "https://api.openai.com/v1",
            "https://example.com:8443/api",
            "http://localhost:11434/v1",
            "http://127.0.0.1:1234/v1",
            "http://[::1]:11434/v1",
        ] {
            assert!(ok(good).is_ok(), "{good}");
        }
        for bad in [
            "http://api.openai.com/v1",
            "http://192.168.1.5:11434/v1",
            "ftp://x.com",
            "api.openai.com",
            "https://user:pw@api.openai.com",
            "https://api.openai.com/v1?key=1",
            "https://api.openai.com/v1#x",
            "https://",
            "https://a b.com",
            "javascript:alert(1)",
            "",
            "http://localhost.evil.com/v1",
        ] {
            assert!(
                matches!(ok(bad), Err(AiError::InvalidConfig(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn model_validation() {
        let m = |s: &str| {
            validate_config(&cfg(
                ProviderKind::Anthropic,
                "https://api.anthropic.com",
                s,
            ))
        };
        assert!(m("claude-sonnet-5-5").is_ok());
        assert!(m("llama3:8b").is_ok());
        assert!(m("org/model-v1.5").is_ok());
        for bad in ["", "   ", "a b", "a;b", "a\"b", &"x".repeat(101)] {
            assert!(m(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn user_message_sanitizes_the_question() {
        let payload = AiPayload {
            system: crate::ai::payload::SystemSummary {
                ram: "1/2 GB".into(),
                cpu: "1%".into(),
                pressure: "".into(),
                trend: "".into(),
                disks: vec![],
            },
            workloads: vec![],
            top_processes: vec![],
            diagnosis_status: "healthy".into(),
            findings: vec![],
        };
        let m = build_user_message(
            &payload,
            Some("why? my key is sk-ant-api03-ABCDEFGHIJKLMNOPQRS and C:\\Users\\alice\\x"),
        );
        assert!(m.contains("User question: why?"));
        assert!(!m.contains("ABCDEFGH") && !m.contains("alice"));
        assert!(build_user_message(&payload, None).contains("Explain what is happening"));
        assert!(build_user_message(&payload, Some("   ")).contains("Explain what is happening"));
    }

    #[test]
    fn system_prompt_forbids_execution_and_invention() {
        assert!(SYSTEM_PROMPT.contains("cannot run commands"));
        assert!(SYSTEM_PROMPT.contains("never invent"));
        assert!(SYSTEM_PROMPT.contains("never ask for files, credentials"));
    }
}

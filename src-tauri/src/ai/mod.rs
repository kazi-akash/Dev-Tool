//! Optional AI gateway. Off unless the user enables it, sends only a sanitized summary, returns
//! text only. The model can propose; it cannot execute anything.

pub mod client;
pub mod keystore;
pub mod payload;
pub mod provider;
pub mod sanitize;

use payload::AiPayload;
use provider::{
    AiConfig, AiError, SYSTEM_PROMPT, build_user_message, provider_for, validate_config,
};

/// Validate, build the request, send it, and interpret the answer.
pub fn ask(
    cfg: &AiConfig,
    api_key: Option<&str>,
    payload: &AiPayload,
    question: Option<&str>,
    sender: client::Sender,
) -> Result<String, AiError> {
    validate_config(cfg)?;
    let key = api_key.unwrap_or("");
    if key.is_empty() && !client::is_local(cfg) {
        return Err(AiError::NotConfigured(
            "Add an API key in Settings first.".into(),
        ));
    }
    let provider = provider_for(cfg.kind);
    let req = provider.build_request(
        cfg,
        key,
        SYSTEM_PROMPT,
        &build_user_message(payload, question),
    );
    let (status, body) = sender(&req)?;
    client::interpret(provider, status, &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use provider::{HttpRequest, ProviderKind};
    use std::cell::RefCell;

    fn payload() -> AiPayload {
        AiPayload {
            system: payload::SystemSummary {
                ram: "1/2 GB".into(),
                cpu: "1%".into(),
                pressure: "NORMAL".into(),
                trend: "STABLE".into(),
                disks: vec![],
            },
            workloads: vec![],
            top_processes: vec![],
            diagnosis_status: "healthy".into(),
            findings: vec![],
        }
    }

    fn cfg(url: &str) -> AiConfig {
        AiConfig {
            kind: ProviderKind::OpenAi,
            base_url: url.into(),
            model: "gpt-4o".into(),
        }
    }

    const OK: &str = r#"{"choices":[{"message":{"content":"All good."}}]}"#;

    #[test]
    fn happy_path_returns_text_and_sends_exactly_one_request() {
        let seen: RefCell<Vec<HttpRequest>> = RefCell::new(vec![]);
        let sender = |r: &HttpRequest| {
            seen.borrow_mut().push(r.clone());
            Ok((200, OK.to_string()))
        };
        let out = ask(
            &cfg("https://api.openai.com/v1"),
            Some("sk-test-key-0123456789"),
            &payload(),
            Some("why slow?"),
            &sender,
        )
        .unwrap();
        assert_eq!(out, "All good.");
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1);
        let body = seen[0].body.to_string();
        // The snapshot is JSON embedded in a string, so its quotes are escaped in the request body.
        assert!(body.contains("why slow?") && body.contains("diagnosisStatus"));
        assert!(!body.contains("sk-test-key"), "key stays in the header");
    }

    #[test]
    fn remote_provider_without_a_key_never_sends_anything() {
        let called = RefCell::new(false);
        let sender = |_: &HttpRequest| {
            *called.borrow_mut() = true;
            Ok((200, OK.into()))
        };
        for key in [None, Some("")] {
            assert!(matches!(
                ask(
                    &cfg("https://api.openai.com/v1"),
                    key,
                    &payload(),
                    None,
                    &sender
                ),
                Err(AiError::NotConfigured(_))
            ));
        }
        assert!(!*called.borrow());
    }

    #[test]
    fn local_server_works_without_a_key() {
        let sender = |r: &HttpRequest| {
            assert!(r.headers.iter().all(|(k, _)| k != "Authorization"));
            Ok((200, OK.into()))
        };
        assert!(
            ask(
                &cfg("http://localhost:11434/v1"),
                None,
                &payload(),
                None,
                &sender
            )
            .is_ok()
        );
    }

    #[test]
    fn invalid_config_is_rejected_before_any_request() {
        let called = RefCell::new(false);
        let sender = |_: &HttpRequest| {
            *called.borrow_mut() = true;
            Ok((200, OK.into()))
        };
        let r = ask(
            &cfg("http://evil.example.com/v1"),
            Some("k"),
            &payload(),
            None,
            &sender,
        );
        assert!(matches!(r, Err(AiError::InvalidConfig(_))));
        assert!(!*called.borrow());
    }

    #[test]
    fn provider_failures_surface_as_errors() {
        let c = cfg("https://api.openai.com/v1");
        let s = |code: u16| move |_: &HttpRequest| Ok((code, String::new()));
        assert!(matches!(
            ask(&c, Some("k"), &payload(), None, &s(401)),
            Err(AiError::Auth(_))
        ));
        assert!(matches!(
            ask(&c, Some("k"), &payload(), None, &s(429)),
            Err(AiError::RateLimited(_))
        ));
        let net = |_: &HttpRequest| Err(AiError::Network("offline".into()));
        assert!(matches!(
            ask(&c, Some("k"), &payload(), None, &net),
            Err(AiError::Network(_))
        ));
    }

    #[test]
    fn ai_output_is_text_only() {
        // Even if the model answers with something that looks like a command, it is just a string.
        let evil = r#"{"choices":[{"message":{"content":"Run: del /s /q C:\\*  and also {\"action\":\"cleanup.path\"}"}}]}"#;
        let sender = |_: &HttpRequest| Ok((200, evil.to_string()));
        let out = ask(
            &cfg("https://api.openai.com/v1"),
            Some("k"),
            &payload(),
            None,
            &sender,
        )
        .unwrap();
        assert!(out.contains("del /s"));
        // `ask` returns a String; there is no code path from it into the action engine.
    }
}

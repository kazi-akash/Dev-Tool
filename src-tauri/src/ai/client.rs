//! Sends a prepared request. The only network code in DevPilot outside the optional AI feature: none.

use std::time::Duration;

use super::provider::{AiConfig, AiError, AiProvider, HttpRequest};

const TIMEOUT: Duration = Duration::from_secs(60);

pub type Sender<'a> = &'a dyn Fn(&HttpRequest) -> Result<(u16, String), AiError>;

/// Real HTTPS send. Never logs the request (it carries the API key and the payload).
pub fn send(req: &HttpRequest) -> Result<(u16, String), AiError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let mut builder = agent.post(&req.url);
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    let body = req.body.to_string();
    let mut resp = builder.send(body.as_str()).map_err(|e| {
        AiError::Network(format!(
            "Could not reach the provider: {}",
            short(&e.to_string())
        ))
    })?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string().map_err(|e| {
        AiError::Network(format!(
            "Could not read the provider's answer: {}",
            short(&e.to_string())
        ))
    })?;
    Ok((status, text))
}

/// Error text from a library may include the URL; keep it short.
fn short(s: &str) -> String {
    s.chars().take(120).collect()
}

/// Map the HTTP status to a user-facing error, or hand the body to the provider parser.
pub fn interpret(provider: &dyn AiProvider, status: u16, body: &str) -> Result<String, AiError> {
    match status {
        200..=299 => provider.parse_response(body),
        401 | 403 => Err(AiError::Auth("The provider rejected the API key.".into())),
        429 => Err(AiError::RateLimited(
            "The provider is rate limiting requests. Try again in a minute.".into(),
        )),
        500..=599 => Err(AiError::Provider(format!(
            "The provider is having problems (HTTP {status})."
        ))),
        _ => match provider.parse_response(body) {
            Err(AiError::Provider(m)) => Err(AiError::Provider(m)),
            _ => Err(AiError::Provider(format!(
                "The provider answered with HTTP {status}."
            ))),
        },
    }
}

/// Used by the command layer to decide whether an empty key is acceptable (local servers).
pub fn is_local(cfg: &AiConfig) -> bool {
    let rest = cfg.base_url.split_once("://").map_or("", |p| p.1);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit_once(':')
        .filter(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
        .map_or(authority, |(h, _)| h);
    matches!(
        host.trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase()
            .as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::provider::{OpenAiCompatible, ProviderKind};

    #[test]
    fn status_codes_map_to_clear_errors() {
        let p = OpenAiCompatible;
        assert!(matches!(interpret(&p, 401, ""), Err(AiError::Auth(_))));
        assert!(matches!(interpret(&p, 403, ""), Err(AiError::Auth(_))));
        assert!(matches!(
            interpret(&p, 429, ""),
            Err(AiError::RateLimited(_))
        ));
        assert!(matches!(interpret(&p, 503, ""), Err(AiError::Provider(m)) if m.contains("503")));
        assert!(
            matches!(interpret(&p, 404, "not json"), Err(AiError::Provider(m)) if m.contains("404"))
        );
        let msg = interpret(&p, 400, r#"{"error":{"message":"model not found"}}"#).unwrap_err();
        assert!(matches!(msg, AiError::Provider(m) if m == "model not found"));
        let ok = r#"{"choices":[{"message":{"content":"hi"}}]}"#;
        assert_eq!(interpret(&p, 200, ok).unwrap(), "hi");
    }

    #[test]
    fn locality_detection() {
        let c = |u: &str| AiConfig {
            kind: ProviderKind::OpenAi,
            base_url: u.into(),
            model: "m".into(),
        };
        assert!(is_local(&c("http://localhost:11434/v1")));
        assert!(is_local(&c("http://127.0.0.1:1234")));
        assert!(is_local(&c("http://[::1]:8080/v1")));
        assert!(!is_local(&c("https://api.openai.com/v1")));
        assert!(!is_local(&c("http://localhost.evil.com")));
        assert!(!is_local(&c("nonsense")));
    }
}

# AI (optional)

DevPilot is fully useful without AI. When enabled, the AI gives a second opinion on the **local** diagnosis. It cannot act.

## Rules

- **Off by default.** `ask_ai` refuses unless the caller passes the opt-in flag, and the UI only passes the saved setting. Nothing is sent until the user presses "Ask AI".
- **Summaries only.** The payload is built from an explicit allow-list (`ai/payload.rs`), then every string passes the sanitizer (`ai/sanitize.rs`).
- **Text in, text out.** The answer is a plain string rendered as text. It is never parsed, rendered as HTML, or passed to the action engine. Actions only come from the rule engine's recommendations, and still need the user's approval.
- **You can look first.** Settings and the Diagnosis page have "Show what will be sent", which shows the exact payload.

## What is sent

```json
{
  "system": { "ram": "15.5 GB/16.0 GB (97%)", "cpu": "38%", "pressure": "CRITICAL", "trend": "RISING",
              "disks": [ { "drive": "C:", "free": "6.0 GB", "total": "275.0 GB", "state": "CRITICAL" } ] },
  "workloads": [ { "name": "Docker / WSL2", "memory": "4.8 GB", "processes": 3, "members": ["WSL2 VM (vmmem): 4.4 GB"] } ],
  "topProcesses": [ { "name": "vmmemWSL", "category": "DOCKER", "memory": "4.4 GB" } ],
  "diagnosisStatus": "problem",
  "findings": [ { "severity": "critical", "title": "...", "summary": "...", "primaryCause": "...", "confidence": "91%",
                 "contributors": ["..."], "recommendations": [ { "problem": "...", "explanation": "...", "expectedBenefit": "...", "risk": "low" } ] } ]
}
```

Limits: 8 workloads, 8 processes, 4 findings, 4 recommendations each. Drives are drive letters only.

**Never included:** file contents or paths, command lines, environment variables, tokens or keys, source code, PIDs, user names, document names. Tests assert a payload built from a process with a user-profile path and a command line full of secrets contains none of them.

## Sanitizer

Applied to every free-text field and to the optional user question (capped at 600 characters): masks the user name in `...\Users\<name>\...` paths, e-mail addresses, known key/token prefixes (`sk-`, `ghp_`, `AKIA`, `xox`, JWT `eyJ`...), long mixed letter/digit strings (24+ characters), and anything after `Bearer`. It is idempotent and errs on the side of redacting.

## Providers (`ai/provider.rs`)

| Kind | Endpoint | Auth |
|------|----------|------|
| OpenAI-compatible (`openAi`) | `{base}/chat/completions` | `Authorization: Bearer` |
| Anthropic-compatible (`anthropic`) | `{base}/v1/messages` | `x-api-key` + `anthropic-version` |

Ollama, LM Studio and similar local servers speak the OpenAI format: use `http://localhost:11434/v1`, no key. Both providers are behind the `AiProvider` trait (build request, parse response), so a native Ollama provider can be added without touching the rest.

Base URL rules: `https://` for anything remote; `http://` only for `localhost`, `127.0.0.1`, `::1`; no credentials, query or fragment in the URL. `http://192.168...` is rejected; use HTTPS or a local port.

Output is capped at 900 tokens, temperature 0.2. The system prompt asks the six questions (what, why, which process, what to do, safest action, what if nothing) and forbids inventing processes or numbers and asking for files or credentials.

## API key

Stored in the **Windows Credential Manager** (`DevPilot` / `ai-api-key`) via the `keyring` crate. The UI can only ask "is a key stored?"; the key is never returned, never written to a file or `localStorage`, never logged, and sent only in an HTTP header (a test checks it never appears in a URL or body).

## Errors

401/403 -> "provider rejected the API key"; 429 -> rate limited; 5xx -> provider problem; network errors are shortened (library error text can include the URL). Provider error messages are sanitized and truncated to 200 characters before display.

# Security

DevPilot runs with the user's own rights, monitors the machine, and can delete files and end processes when asked. These are the guards, and where each is tested.

## Boundaries

| Boundary | Guard |
|----------|-------|
| Frontend -> Rust | Only commands listed in `build.rs` **and** granted in `capabilities/default.json` can be called; the frontend has no shell, file system, HTTP or process plugins. A Rust test keeps the invoke handler, manifest and capability identical and asserts no extra permissions. |
| WebView content | Release CSP: `default-src 'self'; script-src 'self'` (no inline or eval), `object-src 'none'`, `frame-ancestors 'none'`, `connect-src` limited to IPC. Dev CSP is separate. A test asserts the release CSP stays strict. No `dangerouslySetInnerHTML`/`eval` in the UI (e2e/grep). |
| Actions | Whitelisted ids, parameters validated in Rust, re-validated at execution, risk-based confirmations. See [ACTIONS.md](ACTIONS.md). |
| Paths | `storage/paths.rs`: absolute drive paths only, no `..`, no system or profile-structure folders, deletion only for items from the latest scan, never through links. |
| Processes | No shell; fixed argument lists; program allow-list (`docker`, `wsl`, `cmd`); container names validated; process ends only for non-system processes whose name still matches. |
| AI | Opt-in, summarized and sanitized payload, text-only answers, key in Credential Manager. See [AI.md](AI.md). |
| Secrets | Command lines are redacted at collection; keys are never logged; diagnostic payloads are sanitized. |

## The model for untrusted input

Three things are treated as untrusted: the UI (it can be compromised by a bug), AI output, and recommendation parameters round-tripped through the UI. All three can only *ask*. Rust re-derives what is allowed from live facts (process list, scan results, classification) and never trusts a client-supplied risk, safety class, or path outside the scan.

## `unsafe`

`#![deny(unsafe_code)]` at the crate root. The single exception is one Windows API call (`GetPerformanceInfo`) in `windows/memory.rs` with a `SAFETY` note: a zero-initialized, correctly sized struct is passed and the result checked.

## Data at rest

- `devpilot.db` (history) holds aggregated numbers, workload names, diagnosis headlines and action summaries. No file paths, command lines or secrets. Retention: 30 min raw, 24 h at 1 min, 7 d at 5 min, 180 d at 1 h; events 90 d.
- Settings are in the WebView's `localStorage` (theme, thresholds, AI endpoint and model, automation flag). No secrets.

## Network

None, unless the user enables AI and presses "Ask AI". No telemetry, no update checks, no analytics.

## Known limits

- Deletion is permanent (no Recycle Bin), by design and stated in the preview.
- `process.terminate` cannot ask a program to save; it is MEDIUM risk with a warning.
- Without administrator rights some process paths and command lines are unreadable; those processes classify by name only and are flagged as such.
- The classification of tools with no rule is `OTHER`; DevPilot does not guess.
- Installer is not code-signed. Windows SmartScreen will warn until it is.

## Reporting

Issues: https://github.com/kazi-akash/Dev-Tool/issues

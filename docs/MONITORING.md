# Monitoring

Rust is the single source of truth for system metrics. The UI only renders summaries.

## Components (`src-tauri/src/monitoring/`)

| File | Role |
|------|------|
| `metrics.rs` | Serializable snapshot types and pure math (percent, clamping). No OS calls, fully unit-tested. |
| `config.rs` | `MonitorInterval`: runtime-adjustable interval, validated to 1-60 s (default 2 s). |
| `service.rs` | Background sampler thread (`devpilot-monitor`) built on `sysinfo`. Keeps the latest snapshot in `MonitorState`. |

IPC (`src-tauri/src/commands/system.rs`):
- `get_system_snapshot` -> latest `SystemSnapshot | null`
- `set_monitor_interval(secs)` -> rejects values outside 1-60

Event: `system-metrics` (payload `SystemSnapshot`, camelCase).

## Snapshot

RAM total/used/available/percent, total CPU %, per-drive total/used/free/percent, uptime, process count.

`used = total - available` (available memory, not "free", matches Task Manager semantics).

## Cost control

- Sampler runs independent of the UI; monitoring continues while hidden in the tray.
- Events are pushed to the UI only while the main window is visible; the snapshot is still refreshed.
- Process list is refreshed with `ProcessRefreshKind::nothing()` (count only). Per-process data arrives in Phase 3.
- Disk list refreshes every 5th tick, not every tick.
- Removable and zero-size volumes are skipped.

## Failure handling

- `System::new()` failure: monitor logs and stops; the rest of the app keeps running.
- `Disks` failure: disks list is empty and retried on the disk cadence.
- Poisoned lock: previous snapshot is still served.
- Invalid CPU readings (NaN, above 100) are clamped.

## Measured (dev build, Windows 11, 14 logical cores)

| State | CPU (all DevPilot processes) | RAM |
|-------|------------------------------|-----|
| Window visible | ~0.6% (0.45% with Processes page open, Phase 3) | ~516 MB (7 procs, mostly WebView2) |
| Hidden in tray | ~0.09% | ~512 MB |
| `devpilot.exe` alone | - | ~38 MB |

The 150 MB RAM target is not met while WebView2 stays resident. Planned fix in Phase 17: destroy the webview when the window is hidden and recreate on open.

## Processes (`src-tauri/src/processes/`)

| File | Role |
|------|------|
| `model.rs` | `ProcessInfo`, `Category`, `Evidence`, and the pure `query_processes` (filter, sort, limit). |
| `classify.rs` | Pure `classify(name, exe_path, cmdline)`. Ordered rules; reports what evidence was used. |
| `redact.rs` | Masks secrets in command lines and truncates to 300 chars before anything is stored or sent. |
| `collector.rs` | sysinfo process table -> `ProcessInfo`. Caches path, redacted command line and category per `(pid, start_time)`. |
| `store.rs` | Latest full list behind an `RwLock`; read by IPC and later engines. |

IPC: `get_processes({ limit?, sortBy?, category? })` -> `{ total, matched, processes }`. `limit` is clamped to 1-500 (default 100); sorting and filtering happen in Rust so the UI never receives the full list.

### Categories and evidence

`SYSTEM, BROWSER, DEVELOPMENT, DOCKER, NODE, PHP, DATABASE, JAVA, ANDROID, FLUTTER, IDE, OTHER`.

Each classification also carries `evidence`: `path` > `commandLine` > `name` > `none`. A name-only match is weak (a renamed binary fools it), so later rules should weigh it lower. Examples:

- `node.exe` under `...\Microsoft VS Code\...` is `IDE` (path), not `NODE`.
- `java.exe` is `JAVA` (name). With `org.gradle` in its command line it is `JAVA` (commandLine). With `org.gradle` and `android` it is `ANDROID` (commandLine). Gradle is never called Android without Android evidence.
- `svchost.exe` in `C:\Windows\System32` is `SYSTEM` (path); the same name in a Downloads folder is only name-level.

Rule priority: DOCKER, ANDROID, FLUTTER, IDE, PHP, DATABASE, NODE, JAVA, BROWSER, DEVELOPMENT, SYSTEM, OTHER. Group-level detection (Docker + its containers, Gradle + Android, Laragon stack) is Phase 4.

### Privacy

Command lines routinely carry secrets. They are redacted at collection time (`--token x`, `KEY=value` for sensitive keys, `user:pass@` in URLs) so the raw text never reaches the store, the UI, or a future AI payload. Classification uses the raw text for matching only and does not keep it.

### CPU numbers

sysinfo reports per-process CPU relative to one core. DevPilot divides by logical core count so the value is a share of total capacity, like Task Manager.

### Cost

Path and command line are fetched once per process (`UpdateKind::OnlyIfNotSet`); CPU and memory are refreshed each tick. Protected processes may return no path or command line; they still appear, with name-level evidence only.

## Not yet implemented

- Process memory history and growth detection (Phase 6)
- Commit charge and pagefile (Phase 5)
- Disk trend and prediction (Phase 7)

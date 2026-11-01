# Architecture

DevPilot is a Tauri 2 app: a Rust core that owns all system knowledge and a React UI that renders it.

```
React UI (WebView2)
   |  invoke(command) / listen(event)        <- only commands listed in build.rs + capabilities/default.json
Tauri IPC
   |
Rust application core (src-tauri/src)
   monitoring/   sampler thread: RAM, CPU, disks, uptime; pressure + disk-state + growth engines
   processes/    sysinfo -> ProcessInfo, evidence-based classification, command-line redaction
   developers/   workload grouping (Docker/WSL2, Android/Gradle, Next.js, Laragon...)
   docker/       Docker CLI monitor (on demand, cached, optional)
   storage/      read-only scanner, path safety, cleanup classification
   diagnosis/    deterministic rules -> root cause -> recommendations
   actions/      whitelisted, previewable, risk-gated actions + opt-in safe automation
   ai/           optional gateway: sanitizer, payload builder, providers, key store
   database/     SQLite (SQLx) repository
   history/      background recorder + tiered aggregation
   windows/      tray, window lifecycle, commit query (`memory.rs`), PDH counters (`perf.rs`), background thread priority (`priority.rs`); the only `unsafe`
   commands/     thin Tauri command layer (no logic)
```

## Data flow

1. The sampler thread (`monitoring/service.rs`) wakes every N seconds (default 2). It refreshes memory, CPU and the process table, updates the RAM pressure engine, growth tracker and disk engine, stores the latest `SystemSnapshot`, records a history sample, and emits `system-metrics` only while the window exists and is visible.
2. The UI never polls the OS. It renders the snapshot event and asks for derived views (`get_processes`, `get_workloads`, `run_diagnosis`) when it needs them.
3. `run_diagnosis` gathers snapshot + workloads + growth + Docker + latest storage scan and runs the rules in `diagnosis/rules.rs`. Output is a `DiagnosisReport`.
4. A recommendation may carry an `ActionRef`. The UI opens the action dialog, which calls `preview_action` and then `execute_action`. The engine re-validates everything from fresh facts. See [ACTIONS.md](ACTIONS.md).
5. History: samples are buffered and written in batches, rolled up on a timer. See [MONITORING.md](MONITORING.md#history).

## Lifecycle

- Closing the window **destroys the WebView** (it holds ~450 MB); the process, tray, monitor and history recorder continue (~45 MB). Opening from the tray, or launching DevPilot again, recreates the window (`windows/window.rs`). A route requested while closed is parked in `PendingRoute` and claimed by the new page.
- Only the tray's Quit exits (`RunEvent::ExitRequested` is prevented unless an exit code is present).
- A second launch exits immediately and shows the running instance (single-instance plugin).

## Failure isolation

Every subsystem degrades on its own: no Docker -> Docker panel says why; SQLite fails -> history off, live app unaffected; a scan thread panics -> reported as an error, UI waits for nothing; sysinfo fails -> monitor thread logs and stops, rest of the app runs; AI unavailable -> diagnosis unaffected. Locks recover from poisoning instead of panicking.

## Conventions

- Rust types serialize as camelCase JSON; TypeScript mirrors live in `src/types/`.
- Logic is pure and takes its inputs as parameters (time, environment, process lists), so it is unit-tested without the OS.
- Side effects sit behind small traits (`CommandRunner`, `ProcessKiller`, `SecretStore`) and are faked in tests.

## Tray quick status

The tray menu's "System Status" opens a small undecorated popup (`windows/quick.rs`, window label `status`) in the bottom-right corner. It is the same bundle on route `#/quick-status`, rendered without the app shell or settings provider, and it exists only while open: it is destroyed on close and when it loses focus (a 600 ms grace ignores the initial blur).

It adds no monitoring. `get_quick_status` derives RAM, CPU, the worst drive, a severity and a one-sentence diagnosis from the cached snapshot and the cached process list (no Docker CLI, no scan, no sampling). The sentence is the memory mechanism when RAM pressure is HIGH or worse, otherwise the worst drive, otherwise "No problems detected."

Buttons: Open DevPilot, Run Diagnosis, Scan Storage, Settings, Quit. `quick_status_action` accepts exactly those five ids (anything else is rejected) and reuses the tray menu handler, so the popup can open pages and quit and nothing more. Its window has its own capability (`capabilities/status.json`: `get_quick_status` and `quick_status_action`, no events); a test asserts it shares no command with the main window. The tray tooltip (`DevPilot · RAM 87% · CPU 12%`) is refreshed every fifth sample.

## Start with Windows

Settings has a switch, **off unless the user turns it on**. It writes one value (`DevPilot`) to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`: per user, no administrator rights. The command is `"<path>\devpilot.exe" --minimized`.

- The registry is the source of truth. The switch reads it (`get_start_with_windows`) and re-reads after every change, so it shows what Windows will really do. A value that points at another copy of DevPilot (moved, reinstalled) is shown with a "Use this copy instead" button.
- The only writer is `set_start_with_windows`, called from that switch. Source tests assert that nothing else in the app (startup, tray, monitor, preferences, installer-time code) references the startup entry.
- The main window has `create: false` in `tauri.conf.json`. A normal launch creates it in `setup`; `--minimized` does not, so only the tray, the monitor, incident recording, history and notifications start (no WebView, about 45 MB). A second launch carrying `--minimized` (autostart racing a manual start) does nothing; a manual second launch still shows the window.

### Preferences owned by Rust (`prefs.rs`)

The sampling interval, RAM and disk thresholds, safe automation and the notifications switch are persisted in `%APPDATA%\io.github.kazi-akash.devpilot\prefs.json` whenever they change, and applied in `setup` before monitoring starts. The UI still keeps its copy in localStorage and pushes it when the window opens, but a minimized start has no window, so without this file it would run on defaults (including toasts the user turned off). Values are validated again on load; a corrupt or hand-edited file changes nothing, and unchanged values do not rewrite the file.

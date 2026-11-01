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
   windows/      tray, window lifecycle, commit query (`memory.rs`) and PDH counters (`perf.rs`); the only `unsafe`
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

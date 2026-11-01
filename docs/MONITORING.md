# Monitoring

Rust is the single source of truth for system metrics. The UI only renders summaries.

## Components (`src-tauri/src/monitoring/`)

| File | Role |
|------|------|
| `metrics.rs` | Serializable snapshot types and pure math (percent, clamping). |
| `config.rs` | `MonitorInterval`: runtime-adjustable interval, validated to 1-60 s (default 2 s). |
| `service.rs` | Sampler thread (`devpilot-monitor`) built on `sysinfo`. Holds the latest snapshot in `MonitorState`. |
| `pressure.rs` | RAM pressure state machine. |
| `growth.rs` | Per-process memory growth detection. |
| `disks.rs` | Drive free-space states and trend prediction. |

IPC: `get_system_snapshot`, `set_monitor_interval(secs)`, `get/set_pressure_thresholds`, `get/set_disk_thresholds`, `get_memory_growth`. Event: `system-metrics`.

## Snapshot

RAM total/used/available/percent (`used = total - available`, like Task Manager), total CPU %, per-drive total/used/free, `diskHealth` per drive, `pressure`, uptime, process count.

## RAM pressure

States `NORMAL / MODERATE / HIGH / CRITICAL`, defaults `<70 / 70-85 / 85-92 / >92 %`, configurable (validated: `0 < moderate < high < critical <= 100`, 1 point apart).

Percentage is only the starting point:

- less than 1 GB available forces at least HIGH; less than 512 MB forces CRITICAL (ignored on machines under 4 GB)
- commit charge at 90% of the limit forces at least HIGH; 98% forces CRITICAL (`GetPerformanceInfo`)
- pagefile-backed commit over 60% while already HIGH escalates to CRITICAL. This is an estimate: commit above physical RAM must be backed by the pagefile, so `(commit - RAM) / (limit - RAM)`.
- hysteresis: a state is left only when usage is 2 points clearly below its threshold (no flapping)

Trend comes from a least-squares slope over the last 5 minutes (needs 5 samples spanning 60 s): `FALLING`, `STABLE`, `RISING` (>= 0.5 points/min), `ESCALATING` (>= 2 points/min). Rising reports `minutesToCritical`. So "88% and stable" is `HIGH + STABLE`, and "88% and climbing fast" is `HIGH + ESCALATING`.

## Memory growth

Processes over 100 MB are sampled every 30 s (60 minutes kept, at most 60 tracked). A finding needs: 10+ minutes and 8+ samples, growth of at least 300 MB **and** 25%, at least 75% of steps non-decreasing (2% noise tolerance), a roughly linear fit (R-squared >= 0.6), and no recent release of 10% or more. Confidence blends monotonicity, observation length, relative growth and fit, capped at 95%; below 50% it is dropped.

Wording is always **"Possible memory growth/leak"**, with the confidence. A sawtooth (garbage collection), a flat noisy process, and growth followed by a release are explicitly not flagged (tests).

## Disks

Free-space states: `> 20%` NORMAL, `10-20%` WARNING, `5-10%` HIGH, `< 5%` CRITICAL (configurable). One sample per drive per minute (24 h kept); the trend uses the last 6 hours and is reported only with 6+ samples over 10+ minutes, R-squared >= 0.6 and a drain above 10 MB/h. Output: `hoursToCritical`, `hoursToFull` (capped at 30 days).

## Processes

See the process section below, and [DIAGNOSIS.md](DIAGNOSIS.md) for what consumes this data.

| File (`processes/`) | Role |
|------|------|
| `model.rs` | `ProcessInfo`, `Category`, `Evidence`, pure `query_processes` (filter, sort, limit). |
| `classify.rs` | `classify(name, path, cmdline)` ordered rules + `detect_tags`. |
| `redact.rs` | Masks secrets in command lines, caps at 300 chars. |
| `collector.rs` | sysinfo -> `ProcessInfo`; caches static facts per `(pid, start_time)`. |
| `store.rs` | Latest full list behind an `RwLock`. |

Categories: `SYSTEM, BROWSER, DEVELOPMENT, DOCKER, NODE, PHP, DATABASE, JAVA, ANDROID, FLUTTER, IDE, OTHER`. Each carries the **evidence** it rests on: `path` > `commandLine` > `name` > `none`. A name-only match is the weakest.

- `node.exe` under `...\Microsoft VS Code\...` is `IDE` (path), not `NODE`.
- `java.exe` is `JAVA` (name); with `org.gradle` in its command line it is `JAVA` (commandLine); with `org.gradle` and `android` it is `ANDROID`. Gradle is never called Android without Android evidence.
- `svchost.exe` in `C:\Windows\System32` is `SYSTEM` (path); the same name in Downloads is only name-level.

Rule priority: DOCKER, ANDROID, FLUTTER, IDE, PHP, DATABASE, NODE, JAVA, BROWSER, DEVELOPMENT, SYSTEM, OTHER. Tags (`gradle`, `android`, `nextjs`, `vite`, `artisan`, `laragon`, `package-manager`) are computed from the full command line before redaction and truncation.

CPU is normalized to a share of total capacity (sysinfo reports it per core).

## Workloads (`developers/workloads.rs`)

Each process joins at most one workload by priority (Docker, Android, Flutter, IDE, Laragon, Laravel, Node, Java, Database, Browser). A workload has total memory, CPU, members (for example Docker: WSL2 VM, Docker Desktop, backend), top processes and `signals` (`gradle`, `nextjs`, `vite`, `wsl-vm`) used by the rules. The IDE wins over a stack folder, so VS Code installed under a Laragon path is still an IDE.

## Docker (`docker/`)

On demand and cached for 10 s: `docker ps` and `docker stats --no-stream` with an 8 s hard timeout, no shell, no console window. If Docker is missing or its daemon is stopped, `available: false` with the reason.

## Cost control

- Process path and command line are fetched once per process (`UpdateKind::OnlyIfNotSet`); CPU and memory refresh each tick.
- Disk list refreshes every 5th tick; removable and zero-size volumes are skipped.
- The UI receives events only while the window exists and is visible. Rows sent to the UI are capped (500).
- Storage scans, Docker queries and diagnosis run on worker threads, never on the sampler.

## History

`history/aggregate.rs` (pure) + `database/mod.rs` (SQLx, SQLite WAL, migrations in `src-tauri/migrations`). Samples are buffered in memory and written every 10 s in one transaction.

| Tier | Resolution | Kept |
|------|------------|------|
| raw | sampler interval | 30 minutes |
| 1 minute | averages + peaks | 24 hours |
| 5 minutes | averages + peaks | 7 days |
| 1 hour | averages + peaks | 180 days |

Roll-ups run every minute, only on whole buckets, in one transaction per step, and are idempotent. Averages are weighted by sample count, maxima are kept, and per-drive and top-workload detail comes from the peak-RAM sample. A query picks the resolution from the range (about 100-400 points). Diagnoses (deduplicated: same rule/severity within 30 minutes is not re-recorded) and actions are stored for 90 days.

File: `%APPDATA%\com.niduslab.devpilot\devpilot.db`. If it cannot be opened, history is off and everything else runs.

## Measured (Windows 11, 14 logical cores; release build unless noted)

| State | CPU (all DevPilot processes) | RAM |
|-------|------------------------------|-----|
| Window open | ~0.5% (dev build) | ~461 MB (7 processes, almost all WebView2) |
| Window closed (tray) | ~0.10% | ~41 MB (`devpilot.exe` only) |

WebView2 itself dominates the open-window figure; destroying the WebView on close is what brings the resident footprint under the 150 MB target.

## Memory telemetry and mechanism (`monitoring/telemetry.rs`, `windows/perf.rs`)

RAM percentage cannot say why a PC is slow, so each sample also carries `telemetry` (`SystemSnapshot.telemetry`):

| Field | Source |
|-------|--------|
| physical total / available / used | sysinfo |
| commit bytes, limit, percent | `GetPerformanceInfo` |
| pagefile usage % | PDH `\Paging File(_Total)\% Usage` |
| pagefile estimate % | commit above RAM (fallback; labelled "estimate") |
| hard faults/sec | PDH `\Memory\Pages Input/sec` (pages read from disk), averaged over 5 samples |
| disk active % | 100 - PDH `\PhysicalDisk(_Total)\% Idle Time`, averaged over 5 samples |

Counters use English paths (`PdhAddEnglishCounterW`), so they work on any display language. Rate counters need two collections: the first sample is `None`. **`None` means unknown, never zero**; the UI says "unavailable" and confidence drops 0.1 per missing signal.

Mechanism (only when pressure is HIGH or CRITICAL):

| Kind | Evidence |
|------|----------|
| `HEAVY_PAGING` | hard faults >= 100/s together with disk >= 80% or commit >= 90% (0.7 confidence when paging is the only sign) |
| `COMMIT_EXHAUSTION` | commit >= 95%, no paging |
| `DISK_SATURATION` | disk >= 95% but paging low: the diagnosis says memory is probably not the cause |
| `HIGH_RAM_STABLE` | none of the above: "high but stable" |

The diagnosis `memory-pressure` leads with the mechanism sentence and appends the counter evidence. Wording stays hedged ("appears", "suggests").

## Performance incidents (`monitoring/incident.rs`)

Pure state machine fed once per sample. A rolling 30 s pre-buffer is always kept. An incident starts when a trigger holds for 3 s (RAM above the critical threshold, commit >= 95%, smoothed hard faults >= 500 pages/s, or HIGH pressure that is escalating). **Disk saturation alone (>= 95%) needs 15 s**, because builds and scans saturate disks without any memory problem.

Recording continues until nothing has triggered for 60 s (a recurrence inside that window extends the same incident) or 15 minutes pass. A 30 s cooldown follows. At most 1200 samples are held per incident; the oldest after the pre-incident context are dropped first.

Each sample records RAM, available RAM, commit, pagefile, hard faults, CPU, disk activity, top 5 processes, workload memory, Docker/WSL2 memory and active build sessions (empty until the session tracker exists).

The summary (SQLite `incidents`, 90 days) holds peaks, triggers, contributors (peak memory per workload, never summed across samples), a mechanism classified from the *peak* values, a hedged diagnosis, confidence, evidence and a 120-point timeline. Missing counters lower confidence and the text says the cause could not be determined.

IPC: `get_incidents(limit)`, `get_incident_status`. Events: `incident-started`, `incident-finished` (summary). The History page lists incidents.

## Build sessions (`developers/sessions.rs`)

Detected from the process list each tick (cheap prefilter first): Gradle/Android build (wrapper or launcher client, or a Gradle daemon above 3% total CPU; an idle daemon is not a build; Android evidence makes it an Android build), Flutter build (`flutter_tools` + `build`), Next.js build, npm/pnpm build (suppressed under a Next.js build), Docker compose startup, Android emulator startup. Startup activities are time-boxed (compose 180 s, emulator 120 s) and cannot restart until their process has disappeared once.

A session ends 10 s after the activity vanishes (builds spawn and die in bursts); sessions shorter than 5 s are dropped. It records RAM before (about 15 s earlier), RAM peak, CPU and disk peaks, new processes, per-workload memory growth, Docker/WSL2 change (host-process memory, not container-reported) and a result label from the peak.

Correlation with incidents is computed when listing, never stored:

| Level | Needs |
|-------|-------|
| STRONG | overlaps an incident (30 s slack), RAM grew >= 1 GiB, workload growth explains >= half of it. Text: "Strong correlation detected ... does not by itself prove ..." |
| POSSIBLE | overlap in time only |
| NONE | no overlap, no claim |

Timing alone never produces STRONG, and the wording never says the build "caused" an incident. Active session names are stored in each incident sample. IPC: `get_build_sessions(limit)`.

Limits: command lines are redacted and capped at 300 characters, so a `build` argument beyond that is not seen.

## Docker/WSL2 attribution (`docker/attribution.rs`)

Two measurements are kept apart and **never added**: *host memory* (what Windows charges to `vmmem`/`vmmemWSL` and the Docker Desktop processes; this is what reduces free RAM) and *container-reported memory* (`docker stats`, inside the VM, excluding page cache the VM still holds, sampled at a different moment).

`get_docker_breakdown` returns host total, WSL2 VM, Desktop/backend, running containers (largest first; name, status, image, CPU, memory, explicit `--memory` limit from `docker inspect`, `null` when none was set), container-reported total and the VM remainder. The remainder is only computed when every running container reported memory and the total does not exceed the VM; otherwise the level is `PARTIAL` and a note says why. Docker missing or stopped gives `UNAVAILABLE`: host memory only, no per-container split. Unknown values are `null`, shown as "memory unavailable", never as 0.

## Emergency recovery (`monitoring/emergency.rs`)

Active only while RAM pressure is `CRITICAL`. `get_emergency_status` returns `None` otherwise and does not touch Docker; the UI banner (`EmergencyBanner`, shown on every page) only asks while the snapshot says critical.

The report shows RAM, commit, disk activity and hard faults (unavailable counters say so), the mechanism sentence from telemetry, the largest workloads (at least 1 GiB or 10% of RAM) as likely contributors, whether an incident is recording, and active build sessions.

Recovery steps are derived from measured facts only and are **references to registered actions**, least disruptive first: stop Gradle daemons (only if a daemon is running), stop idle containers (under 1% CPU, at least 50 MB, name validated, top 3), close the Android emulator process (never a system process), shut down WSL (when the VM holds at least 1 GiB). The combined estimate does not double count: WSL shutdown includes the container steps, so the larger of the two is used. Estimates are labelled with their basis ("container-reported", "host process memory") and called upper bounds.

**Nothing executes from here.** A step opens the normal action dialog (preview, confirmation, execute, verify, measure) over the same command ACL. Two source-hygiene tests enforce this: the AI gateway has no reference to the action engine, command runner or process creation, and the emergency assessment contains no execution calls. AI output stays text.

## Notifications (`monitoring/alerts.rs`, `notifier.rs`, `windows/toast.rs`)

Windows toasts, decided by a pure policy and displayed through `tauri-winrt-notification` (no `unsafe` in DevPilot). A toast carries a short summary and one button (and a click on the toast does the same) that opens the app on a fixed route: "View diagnosis" `/diagnosis`, "View results" `/storage`, "View incident" `/history`, "View actions" `/actions`. The activation argument comes from outside the process, so it is checked against the route allow-list again; anything else falls back to the alert's own destination.

| Condition | Level | Text |
|-----------|-------|------|
| memory escalating while HIGH | 1 | "Memory is climbing fast" with the rate and minutes to critical |
| RAM CRITICAL | 2 | "RAM is critical (97%)" with the mechanism sentence or the largest workload |
| RAM CRITICAL with heavy paging or commit exhaustion | 3 | "Memory exhaustion is slowing your PC" |
| a drive CRITICAL | 2 | "Drive S:\ is almost full" with free space and time to full |
| performance incident recorded | one-off | the incident's hedged diagnosis |
| storage scan finished or stopped early | one-off | item count and size (partial result is said) |
| action finished or failed | one-off | the action summary |

De-duplication and cooldown: a condition is announced when **entered**, once. While it persists only a **higher level** interrupts (escalation bypasses the cooldown, once per level). It must be gone for 2 minutes before it counts as a new episode, and a new episode inside the cooldown (10 min memory, 60 min per drive) is suppressed unless it is worse than the last announcement. Results are unique per scan or action and spaced 5 s apart per kind. Scan and action results the user just triggered are skipped while the main window is visible and focused; automated runs and incidents always notify.

The master switch (Settings, on by default) is enforced in Rust (`set_notifications_enabled`). "Send a test notification" shows a toast and reports a failure instead of hiding it. A build run from a build folder is not registered with Windows, so it borrows PowerShell's toast identity; an installed build uses the app identifier. One real toast can be checked by hand: `cargo test --lib real_toast -- --ignored`.

## Process tree and long lists

`get_process_tree` builds a parent/child tree from the cached process list on request (nothing is added to the sampler). It shows the largest subtrees only: at most 40 roots, 400 nodes and 8 levels, with subtrees under 20 MB left out and reported as "+N small". A parent is accepted only when it started no later than its child (Windows reuses PIDs), and cycles are cut. A node's total (Σ) is the sum of each process's own memory; shared pages count once per process. The Processes page has a List / Tree switch; the list stays the default and the tree is fetched only when selected.

Process and storage tables render only a window of their rows once they exceed 100 (`useVirtualWindow`, fixed row heights, spacer rows keep the scrollbar honest), so thousands of rows never become thousands of DOM nodes. The backend caps (500 processes, paged storage results) stay in place as a safety limit.

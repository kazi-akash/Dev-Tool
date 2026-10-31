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

## Measured (dev build, Windows 11, 14 logical cores)

| State | CPU (all DevPilot processes) | RAM |
|-------|------------------------------|-----|
| Window open | ~0.5% | ~480 MB (7 processes, almost all WebView2) |
| Window closed (tray) | ~0.1% | ~44 MB (`devpilot.exe` only) |

WebView2 itself dominates the open-window figure; destroying the WebView on close is what brings the resident footprint under the 150 MB target.

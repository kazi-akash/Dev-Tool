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
| Window visible | ~0.6% | ~516 MB (7 procs, mostly WebView2) |
| Hidden in tray | ~0.09% | ~512 MB |
| `devpilot.exe` alone | - | ~38 MB |

The 150 MB RAM target is not met while WebView2 stays resident. Planned fix in Phase 17: destroy the webview when the window is hidden and recreate on open.

## Not yet implemented

- Per-process CPU/RAM (Phase 3)
- Commit charge and pagefile (Phase 5)
- Disk trend and prediction (Phase 7)

# Actions

Anything DevPilot can change goes through the action engine (`src-tauri/src/actions/`). There is no other write path: not from the UI, the tray, or the AI.

## Contract

Each action has `id`, `name`, `description`, `riskLevel`, `estimatedBenefit`, `automationEligible`, `reversible`, and implements `preview`, `execute`, and optionally `rollback`.

Risk levels: `SAFE, LOW, MEDIUM, HIGH, DESTRUCTIVE`. **MEDIUM and above require confirmation; DESTRUCTIVE requires a second, separate confirmation.** The risk used is the one from the *current preview* of that invocation, not the action's maximum.

`execute` always re-runs `preview` first (validation and risk are recomputed from fresh facts: live process list, latest scan), then checks the confirmations the caller passed, then runs. A caller cannot skip validation or claim a lower risk.

## Registered actions

| Id | Risk | What it does | Undo |
|----|------|--------------|------|
| `docker.stop_container` | LOW | `docker stop <name>` for one container (name validated: `[A-Za-z0-9][A-Za-z0-9_.-]{0,127}`, never a flag) | `docker start` |
| `wsl.shutdown` | MEDIUM | `wsl --shutdown` | no (WSL restarts on demand) |
| `gradle.stop` | LOW | `gradle --stop` (via `cmd /C`, fixed arguments) | no |
| `process.terminate` | MEDIUM | end one process by PID. Refuses: system-category processes, DevPilot itself, a PID whose name no longer matches the recommendation (PID reuse), missing PIDs | no |
| `cleanup.path` | LOW / MEDIUM / DESTRUCTIVE by safety class | permanently delete one scanned item (see below) | no |
| `cleanup.temp` | LOW | delete items in the user Temp folder untouched for 24 h; the only action eligible for automation | no |

### `cleanup.path` rules

1. Path has no `..`, is an absolute drive path, and is not protected (`storage/paths.rs`).
2. It must be **exactly an item from the latest storage scan**. A real folder that was not scanned is refused, as is anything inside a scanned item.
3. Safety is recomputed from the item's kind (not read from the client-visible field). `DO_NOT_TOUCH` is refused. SAFE_TO_CLEAN -> LOW, SAFE_TO_REVIEW -> MEDIUM, REQUIRES_REVIEW -> DESTRUCTIVE.
4. The item must still exist and must not itself be a link or junction.
5. Temp and package caches lose their contents but keep the folder (it may be in use; locked files are skipped). Other items are removed entirely. Links inside are removed as links, never followed.

Deletion is permanent (no Recycle Bin); the preview says so.

## Safe automation (opt-in, off by default)

When enabled in Settings, a background check every minute runs `cleanup.temp` if any drive is HIGH/CRITICAL or has under 10 GB free, at most once an hour. `execute_automated` refuses anything not flagged automation-eligible or above LOW risk. It never deletes arbitrary files, kills processes, edits the registry or Windows files, uninstalls software, or touches documents. Runs appear in history marked `auto`.

## Process and command execution

No shell is used for external programs: `Command::new(program).args(fixed list)`. The real runner only accepts `docker`, `wsl`, `cmd`. Calls have a hard timeout and no console window. Tests replace the runner and the process killer with fakes that panic if called unexpectedly.

## History

Every execution (including refusals after validation, failures and automation) is recorded in memory (200 newest) and persisted to SQLite. The Actions page lists this session with Undo for reversible ones; the History page shows older ones.

## Adding an action

1. Implement `Action` in `builtin.rs` with a precise `preview`.
2. Add it to `builtin::all()`. Actions are not Tauri commands: the generic `preview_action` / `execute_action` commands carry the action id, so no ACL change is needed.
3. Tests: validation failures, confirmation requirements, and that nothing runs when refused.

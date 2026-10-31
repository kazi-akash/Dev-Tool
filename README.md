# DevPilot

Windows developer-system health assistant. Answers: *why is my dev PC slow, out of RAM, or out of disk, and what can I safely do about it?*

Status: **Phase 1 complete** (application shell: tray, sidebar, routing, theme, settings). Pages other than Settings are placeholders. Monitoring starts in Phase 2.

## Stack

Tauri 2, React 19, TypeScript 6, Vite 8, Tailwind CSS 4, shadcn/ui-style components (Radix + lucide-react), react-router (HashRouter), Rust (stable, MSVC). Planned: SQLx/SQLite, sysinfo, windows-rs, Recharts.

## Shell behavior (Phase 1)

- Sidebar: Dashboard, Processes, Storage, Diagnosis, Actions, History, Settings.
- Theme: System / Light / Dark, stored in `localStorage` (`devpilot.settings.v1`). Moves to SQLite in Phase 16.
- Tray menu: Open DevPilot, System Status, Scan Storage, Run Diagnosis, Settings, Quit. Left-click on tray icon opens the window.
- Tray entries show the window and emit a `navigate` event with an allow-listed route (`src-tauri/src/windows/tray.rs`); the frontend never receives arbitrary commands.
- Closing the window hides to tray; monitoring keeps running. Quit only via tray menu.
- Single instance: a second launch exits and re-shows the running window (no duplicate tray icon).

## Prerequisites (Windows 11)

| Tool | Verified version | Install |
|------|------------------|---------|
| Windows | 11 Home 10.0.26200 | - |
| Node.js | 22.13.1 | https://nodejs.org |
| pnpm | 12.16.1 | `npm i -g pnpm` |
| Rust (MSVC toolchain) | rustc/cargo 1.99.1 | `winget install Rustlang.Rustup`, then `rustup default stable-x86_64-pc-windows-msvc` |
| VS 2022 Build Tools | 17.14, workload "Desktop development with C++" | `winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` |
| WebView2 Runtime | 156.0.4314.50 | preinstalled on Windows 11 |

Notes:
- Build Tools need roughly 6-8 GB free on C:. Install fails with `0x80070070` (disk full) otherwise.
- `winget` Rustup install may exit 1 when MSVC is missing; run `rustup default stable-x86_64-pc-windows-msvc` manually.
- On Windows PowerShell 5.1, `Set-Content -Encoding utf8` writes a BOM, which breaks `tauri.conf.json`. Write JSON without BOM.
- Ensure `%USERPROFILE%\.cargo\bin` is on `PATH` (new shell after install).

## Commands

```powershell
pnpm install          # JS deps
pnpm tauri dev        # run app (Vite + Rust, hot reload)
pnpm build            # type-check + frontend bundle
pnpm test             # Vitest (happy-dom)
pnpm tauri build      # release bundle (Phase 20)
cd src-tauri; cargo test
```

Test env note: Vitest uses `happy-dom`, not `jsdom`. jsdom pulls undici 8, which crashes on Node 22.13.1.

## Layout

```
devpilot/
  src/          React frontend
  src-tauri/    Rust core (Tauri 2)
  docs/         architecture + module docs (filled per phase)
  tests/        integration/e2e tests
  scripts/      dev scripts
```

Target structure is in the master spec; modules are added phase by phase.

## Verification

Phase 1:
- `pnpm build`: OK
- `pnpm test`: 4 passed (nav order, routing, theme persistence, corrupt-settings fallback)
- `cargo test`: 2 passed (tray menu route mapping, unknown ids rejected)
- `pnpm tauri dev`: window renders sidebar + settings, Rust process ~36 MB working set
- `cargo clippy -D warnings`, `cargo fmt --check`, `tsc --noEmit`: clean
- Runtime: close hides window and keeps process alive; second launch exits and re-shows window
- Not yet verified: tray icon visuals and menu clicks (manual check needed; Win11 overflow not scriptable)

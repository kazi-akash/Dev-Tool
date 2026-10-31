# DevPilot

Windows developer-system health assistant. Answers: *why is my dev PC slow, out of RAM, or out of disk, and what can I safely do about it?*

Status: **Phase 0 complete** (environment + minimal Tauri 2 shell builds and runs). Later phases not started.

## Stack

Tauri 2, React 19, TypeScript 6, Vite 8, Rust (stable, MSVC). Planned: Tailwind, shadcn/ui, SQLx/SQLite, sysinfo, windows-rs, Recharts.

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
pnpm tauri build      # release bundle (Phase 20)
cd src-tauri; cargo test
```

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

## Phase 0 verification

- `cargo build`: OK
- `pnpm build`: OK
- `cargo test`: OK, 0 tests (no logic yet)
- `pnpm tauri dev`: window "DevPilot" opened, ~38 MB working set for the Rust process

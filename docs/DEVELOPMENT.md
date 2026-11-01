# Development

## Setup

See the prerequisites table in the [README](../README.md). In short: Node 22+, pnpm, Rust stable (MSVC), Visual Studio 2022 Build Tools with the C++ workload, WebView2 (preinstalled on Windows 11).

```powershell
pnpm install
pnpm tauri dev          # app with hot reload (Vite on :1420 + Rust)
```

## Tests

| What | Command | Notes |
|------|---------|-------|
| Rust unit + workflow | `cd src-tauri; cargo test` | ~435 tests (+1 ignored: `cargo test --lib real_toast -- --ignored` shows one real toast); touches the real process table, PDH counters, Windows Credential Manager (unique service name), a throwaway HKCU registry key, temp dirs, in-memory SQLite. The Recycle Bin, Docker, WSL and process killing are always faked |
| Lint | `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` | must stay clean |
| Frontend unit/UI | `pnpm test` | Vitest + happy-dom + Testing Library, Tauri bridge mocked in `tests/ipc-mock.ts` |
| Types | `pnpm exec tsc --noEmit` | |
| End-to-end | `pnpm test:e2e` | Playwright + Chromium against the Vite server with an in-page Tauri bridge (`e2e/bridge.ts`). Writes screenshots to `docs/screenshots/`. First run: `pnpm exec playwright install chromium` |

Vitest uses `happy-dom`, not `jsdom`: jsdom pulls undici 8, which crashes on Node 22.13.1.

## Layout

```
src/                    React app
  app/                  router, navigation
  components/           layout, ActionDialog, ui primitives (shadcn-style)
  features/<page>/      one folder per page
  hooks/ lib/ types/    IPC hooks, helpers, mirrors of Rust JSON
src-tauri/
  src/<module>/         see docs/ARCHITECTURE.md
  migrations/           SQLx migrations (embedded at compile time)
  build.rs              command ACL manifest
  capabilities/         what the window may call
e2e/ tests/ docs/ scripts/
```

## Adding a command

1. Implement it in `src-tauri/src/commands/*.rs` (thin: call a module, no logic) and register it in `lib.rs`'s `generate_handler!`.
2. Add its name to `COMMANDS` in `build.rs` and `allow-<name-with-dashes>` to `capabilities/default.json`. `cargo test` fails until all three agree.
3. Add the TypeScript type in `src/types/` and call it through `call()` from `lib/ipc.ts`.

## Adding a migration

Add `src-tauri/migrations/000N_name.sql`. Never edit an applied migration. Repository functions live in `database/mod.rs` and are tested against in-memory SQLite.

## Release build

```powershell
pnpm tauri build        # NSIS installer under src-tauri/target/release/bundle/nsis
```

Release profile: fat LTO, `opt-level = 3`, `panic = "abort"`, stripped. Per-user install (no admin rights). Not code-signed.

## Conventions

- Keep logic pure and inject time, environment and processes; fake side effects behind traits.
- A new rule, classification or action ships with scenario tests.
- Anything that can leave the machine or change it needs a test that proves the guard.
- No `unwrap`/`expect` on user-influenced data; poisoned locks recover (`into_inner`).

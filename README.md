# DevPilot

A lightweight Windows health assistant for developers. It answers one question:

> **Why is my development PC slow, freezing, running out of RAM or running out of disk, and what can I safely do about it?**

It watches RAM, CPU and disks, recognises developer workloads (Docker/WSL2, Android/Gradle, Next.js, Laragon, IDEs...), names the cause with numbers and confidence, and offers fixes you can preview first. It works fully offline; AI assistance is optional and off by default.

![Dashboard](docs/screenshots/dashboard.png)

## What it is

DevPilot is a small Windows desktop app that sits in your system tray and watches the health of a development PC. Developer machines are hard to read: Docker, WSL2, Android Studio, Gradle, Node, IDEs and browsers all compete for the same RAM and disk, and Task Manager only shows a long list of processes. DevPilot groups those processes into the workloads you actually think in, works out *which one is responsible* and *how* (plain high usage, heavy paging, commit exhaustion, a saturated disk), and tells you what is safe to do about it.

It is **read-only by default**. Anything that changes your system (stopping a container, ending a process, deleting a cache) is a separate, previewed action that you confirm. It works fully offline; AI assistance is optional and off by default.

## Features

- **Dashboard**: system health in one glance. RAM pressure state and trend, per-drive state with predictions such as "full in about 2 h", the memory mechanism (why RAM is high), developer workloads with their members, top processes, alerts and recent diagnoses.
- **Processes**: live table with evidence-based categories (IDE vs Node vs Docker...), redacted command lines, a List / Tree switch for parent/child process trees, and a Docker panel with per-container memory.
- **Storage**: read-only scan for caches, `node_modules`, build output, old APKs and large files, classified `SAFE_TO_CLEAN / SAFE_TO_REVIEW / REQUIRES_REVIEW / DO_NOT_TOUCH`. An optional deep scan reports big AppData folders (report-only).
- **Diagnosis**: a rule engine explains *why* ("RAM is at 96%. Docker/WSL2 and Android Gradle build account for approximately 8.9 GB, making them the primary contributors.") with contributors, confidence, evidence and recommendations (problem, explanation, benefit, risk, action).
- **Actions**: whitelisted, risk-gated, undoable where possible: stop a container, shut down WSL, stop Gradle daemons, close one process, delete one scanned item, clean old temp files. Each result is verified and measured.
- **Emergency recovery**: when RAM is critical, a banner on every page lists the least disruptive steps first with honest estimates. It only opens the normal action dialog; nothing runs by itself.
- **Performance incidents and build sessions**: DevPilot records what happened around a freeze (including the 30 s before it) and detects Gradle, Flutter, Next.js, npm and Docker builds, then says whether a build *strongly correlates* with an incident, never that it "caused" one.
- **History**: RAM, CPU and free-space charts from 15 minutes to 30 days, past diagnoses, actions, incidents and builds. Recorded in the background, also while the window is closed.
- **Notifications**: Windows toasts for critical RAM, a nearly full drive, fast memory growth, recorded incidents and finished scans or actions. Each condition is announced once, not repeatedly. Can be switched off.
- **Tray and quick status**: Open DevPilot, System Status (a small popup with RAM, CPU, worst drive and a one-line diagnosis), Scan Storage, Run Diagnosis, Settings, Quit. Closing the window keeps monitoring running at about 41 MB.
- **Start with Windows**: optional, off by default. Starts straight into the tray, no window, no administrator rights.
- **Optional AI**: bring your own OpenAI-compatible, Anthropic-compatible or local (Ollama) endpoint. Only a sanitized summary is sent, you can inspect it first, and the AI can suggest but never act.

## How it works

### The pipeline

```text
 sample          understand            explain              act (only if you confirm)
 ------          ----------            -------              -------------------------
 RAM, CPU,  -->  classify processes -> rules find the   -->  preview -> confirm ->
 disks,          into workloads,       cause, rank it,       execute -> verify ->
 processes,      track pressure,       give confidence       measure -> history
 Docker          growth, disk trend    and evidence
```

A Rust thread samples the machine every 2 seconds (configurable, 1 to 60 s). Everything else is derived from that data on demand, so the interface never polls Windows itself and the app stays small.

### Feature by feature

**Monitoring and RAM pressure.** The sampler reads RAM, CPU, drives and the process list. RAM is classified `NORMAL / MODERATE / HIGH / CRITICAL`, but not by percentage alone: very little available memory or a nearly full commit charge raises the level, and the state only drops once usage is clearly below its threshold, so it does not flap. A trend over the last 5 minutes says `STABLE`, `RISING` or `ESCALATING` and estimates minutes to critical. You can change the thresholds in Settings.

**Memory mechanism.** High RAM alone does not explain a slow PC. DevPilot also reads commit charge, pagefile use, hard page faults and disk activity, and names the mechanism: high but stable, heavy paging, commit exhaustion, or a saturated disk. A counter Windows cannot provide is shown as "unavailable", never as zero, and lowers confidence.

**Process classification.** Every process gets a category (`IDE`, `NODE`, `DOCKER`, `ANDROID`, `JAVA`, `BROWSER`, `SYSTEM`...) from the strongest evidence available: executable path first, then command line, then name only. `node.exe` inside VS Code is an IDE, and a Java process is only called Android when its command line shows Android evidence. Command lines are redacted (tokens, passwords, keys) and capped before they are shown.

**Workloads.** Processes are grouped into workloads such as Docker/WSL2, Android build, Flutter, IDE, Laragon, Node, Java, Database and Browser. A workload shows its total memory and CPU, its members and its top processes. This is what lets the diagnosis say "Docker accounts for 8.9 GB" instead of listing forty processes.

**Memory growth.** Large processes are tracked over time. A process is reported only if its memory climbs steadily (at least 300 MB and 25%, mostly without drops, close to a straight line). The wording is always "possible memory growth", with a confidence, and normal garbage-collection sawtooth patterns are not flagged.

**Disks.** Each drive has a free-space state and, when the trend is clear, a prediction ("full in about 2 h"). Predictions need several samples over enough time and a real drain rate, so a quiet drive does not get a made-up forecast.

**Docker and WSL2.** If Docker is installed, DevPilot lists running containers with CPU and memory, and splits memory into the WSL2 VM, Docker Desktop and the containers. Host memory (what Windows charges) and container-reported memory are kept apart and never added together. If Docker is missing or stopped, the panel says so and the rest of the app is unaffected.

**Storage scan.** You start a scan; DevPilot never does it by itself. It looks at known developer locations (caches, `node_modules`, build output, Gradle and npm caches, large files) and classifies each item by how safe it is to remove. The scan runs at background priority and pauses or slows down by itself when RAM is critical or a build is running, so it does not add to the problem it is looking at. The optional deep scan of AppData is report-only: it shows which folders are large and offers no delete button.

**Diagnosis.** One click runs the rule engine over the current snapshot, workloads, growth findings, Docker status and the latest scan. The rules cover memory pressure, Docker/WSL2, Android/Gradle, Node development, memory growth, low disk and cleanable caches, and report a healthy system plainly when nothing is wrong. Each finding has a severity, the contributors with their sizes, a confidence, the evidence behind it and recommendations that carry a risk label.

**Actions.** A recommendation can open an action dialog. The flow is always the same:

1. **Preview**: what will happen, the risk level and the expected benefit, checked against fresh facts.
2. **Confirm**: medium risk needs one confirmation, destructive needs a second, separate one. The engine enforces this, not just the dialog.
3. **Execute**: runs only registered actions with fixed arguments and no shell. A deletion must be exactly an item from the latest scan; anything else is refused.
4. **Verify and measure**: checks the expected end state (container gone, process gone, item gone) and measures RAM or disk recovered, which can honestly be zero or negative.
5. **Undo and history**: stopped containers can be restarted, deleted items go to the Recycle Bin when possible, and every run is recorded.

**Safe automation (off by default).** If you enable it, DevPilot may clean old temp files when a drive is nearly full, at most once an hour. It never deletes anything else, closes programs, edits the registry or touches documents. Runs are marked `auto` in history.

**Incidents and build sessions.** While you work, the last 30 seconds of samples are always kept. If RAM goes critical, paging spikes or a disk saturates, DevPilot records an incident with the context before and after, the top processes and a hedged diagnosis. Builds are detected separately and recorded with their RAM, CPU and disk peaks. The History page links the two only when the evidence supports it.

**History.** Samples are stored locally in SQLite and rolled up into coarser steps (raw 30 minutes, 1 minute for 24 hours, 5 minutes for 7 days, 1 hour for 180 days). Diagnoses, actions and incidents are kept for 90 days. Charts pick a sensible resolution for the range you choose.

**Tray and background behaviour.** Closing the window destroys the WebView (the heavy part) while monitoring, history and notifications keep running. Opening the tray icon or launching DevPilot again brings the window back. Only Quit exits.

**Optional AI.** Off by default. If you connect a model, DevPilot builds a small summary from an allow-list of fields (no file contents, environment variables, tokens, source code or full paths), lets you read it first, and sends it only when you click. The reply is shown as text; the AI has no way to run an action. Your API key is kept in Windows Credential Manager.

### Privacy and safety in short

- Offline-first: no telemetry, nothing is uploaded automatically.
- No destructive step without a preview and confirmation; the frontend cannot run commands or touch files directly.
- Every command the UI can call is on an explicit allow-list, with a strict content security policy.
- Details: [Security](docs/SECURITY.md), [Actions](docs/ACTIONS.md), [AI](docs/AI.md).

## Design principles

Windows first. Rust core owns all monitoring; no Node daemon, no Electron. Offline by default. Nothing is deleted or closed without an explicit, previewed confirmation. Unknown things are classified `OTHER` or `REQUIRES_REVIEW`, never guessed. Each subsystem fails on its own (no Docker, no SQLite, no AI) without taking the app down.

## Stack

Tauri 2, Rust (sysinfo, windows-rs, SQLx/SQLite, ureq, keyring), React 19, TypeScript 6, Vite 8, Tailwind CSS 4, shadcn-style components (Radix, lucide-react), Recharts, react-router (HashRouter).

## Install

There is no published release yet. `pnpm tauri build` produces a per-user NSIS installer (3.2 MB, no administrator rights) under `src-tauri/target/release/bundle/nsis`. It is **not code-signed**, so Windows SmartScreen will warn. Windows 10/11 with the WebView2 runtime (preinstalled on Windows 11).

## Build from source

Prerequisites (versions verified on Windows 11 10.0.26200):

| Tool | Version | Install |
|------|---------|---------|
| Node.js | 22.13.1 | https://nodejs.org |
| pnpm | 12.16.1 | `npm i -g pnpm` |
| Rust (MSVC toolchain) | 1.99.1 | `winget install Rustlang.Rustup`, then `rustup default stable-x86_64-pc-windows-msvc` |
| VS 2022 Build Tools | 17.14, "Desktop development with C++" | `winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` |
| WebView2 Runtime | 156 | preinstalled on Windows 11 |

Notes: Build Tools need 6-8 GB free on C: (error `0x80070070` means disk full). The winget Rustup install can exit 1 when MSVC is missing; run `rustup default stable-x86_64-pc-windows-msvc` manually. On Windows PowerShell 5.1, `Set-Content -Encoding utf8` writes a BOM that breaks JSON config; write files without BOM.

```powershell
pnpm install
pnpm tauri dev      # run with hot reload
pnpm tauri build    # NSIS installer: src-tauri/target/release/bundle/nsis
```

## Tests

```powershell
cd src-tauri; cargo test; cargo clippy --all-targets -- -D warnings   # Rust
pnpm test                                                              # Vitest: UI + logic
pnpm test:e2e                                                          # Playwright in Chromium
```

Current counts: see [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md#tests). Vitest uses `happy-dom` because `jsdom` crashes on Node 22.13.1.

## Documentation

[Architecture](docs/ARCHITECTURE.md) · [Monitoring](docs/MONITORING.md) · [Diagnosis](docs/DIAGNOSIS.md) · [Storage](docs/STORAGE.md) · [Actions](docs/ACTIONS.md) · [AI](docs/AI.md) · [Security](docs/SECURITY.md) · [Development](docs/DEVELOPMENT.md)

## Honest limits

- Memory with the window **open** is about 460 MB, nearly all WebView2. With the window closed it is about 41 MB (the WebView is destroyed and recreated on demand). The 150 MB target is met in the resident, tray-only state, not while the window is showing.
- Long process and storage lists render only the visible window of rows; the backend still caps process lists at 500 rows.
- Without administrator rights some process paths and command lines cannot be read; those processes are classified by name only and flagged as such.
- The "pagefile usage" signal is an estimate derived from commit charge, not the pagefile's resident size.
- Memory-growth detection reports a *possible* leak with confidence; caches and builds also grow.
- The installer is unsigned.

## License

[MIT](LICENSE) © 2026 Kazi Sajedul Islam.

## About

DevPilot answers one question for developers on Windows: *why is my PC struggling, and what can I safely do about it?* It is offline-first and private: nothing is uploaded, AI is optional, and nothing destructive runs without your explicit confirmation.

**Built with:** Tauri 2 and Rust (monitoring, diagnosis, actions, SQLite history), React, TypeScript, Tailwind CSS and Recharts (interface), tested with cargo test, Vitest and Playwright. Windows 10/11 only.

**Author and maintainer**

**Kazi Sajedul Islam** — idea, product direction, specification and ownership.

- Email: [quaziakash@gmail.com](mailto:quaziakash@gmail.com)
- GitHub: [kazi-akash](https://github.com/kazi-akash)
- LinkedIn: [kazi-akash](https://www.linkedin.com/in/kazi-akash/)

**Contributors**

| Who | Contribution |
| --- | --- |
| Kazi Sajedul Islam ([@kazi-akash](https://github.com/kazi-akash)) | Creator and maintainer: product definition, requirements, review and decisions. |
| [Claude Code](https://claude.com/claude-code) (Anthropic AI assistant) | Wrote most of the implementation, tests and docs under the author's direction. |

DevPilot is an independent personal project. It is not a product of any company. Issues and pull requests are welcome at [github.com/kazi-akash/Dev-Tool](https://github.com/kazi-akash/Dev-Tool); anyone who contributes will be added to the table above.

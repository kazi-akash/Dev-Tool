# Storage

The scanner (`src-tauri/src/storage/`) **measures and classifies; it never changes anything**. Deleting is a separate, confirmed action (see [ACTIONS.md](ACTIONS.md)).

## What it scans

1. **Known locations** (only those that exist): user Temp, `npm-cache`, `pnpm` store, Yarn cache, Composer, pip cache, Pub cache, Android SDK, Docker data, WSL disks (`ext4.vhdx`), `.gradle\caches` and wrapper dists, `.android\avd`, `.nuget\packages`, `.cargo\registry`, `.npm\_cacache`.
2. **User folders**: `Downloads`, `Desktop`, `Documents`, `Videos`: folder size plus large files (default >= 500 MB).
3. **Development roots**: `source`, `repos`, `projects`, `dev`, `code`, `work`, `workspace`, `Documents`, `Desktop` under your profile, plus the root of every other fixed drive (for example `S:\`). Looks for `node_modules`, build output, old APKs and large files, up to 7 levels deep.

Never entered: protected system areas (Windows, Program Files, ProgramData, System Volume Information, Recycle Bin...), `.git`/`.hg`/`.svn`, and any reparse point (symlinks, junctions, OneDrive placeholders). A `node_modules` is measured as one item; nested ones are not reported separately.

Build output is recognised by a **sibling marker file**, so an unrelated `build` or `dist` folder is not mislabeled: `.next`/`.nuxt` need `package.json`; `dist`/`out` need `package.json` or `tsconfig.json`; `build` needs `package.json`, Gradle, `pubspec.yaml` or CMake; `target` needs `Cargo.toml` or `pom.xml`.

APKs are reported when at least 5 MB and 30 days old. A loose `.exe`/`.msi` in a personal folder is an "installer"; inside a project it is just a large file.

## Result

Per entry: path, name, size, file count, last modified (newest file inside), category, safety class and reason. A scan is bounded (3 minutes, 8 million items), cancellable, reports progress, and says when results are partial or items were unreadable. Only one scan runs at a time, on its own thread.

## Safety classification

Decided by **what the item is and where it lives, never by extension alone**.

| Class | Meaning | Examples |
|-------|---------|----------|
| `SAFE_TO_CLEAN` | known disposable cache; eligible for opt-in automation | user Temp, npm/pnpm/Yarn/pip caches |
| `SAFE_TO_REVIEW` | re-creatable; check first | `.next`, `dist`, `target`, Gradle caches, Pub cache |
| `REQUIRES_REVIEW` | may be the only copy or an active project | `node_modules`, APKs, archives, ISOs, videos, installers, unknown files |
| `DO_NOT_TOUCH` | never offered | Android SDK and AVDs, Docker data, WSL disks, your Documents/Downloads folders themselves |

Extension only gives a label (`.iso` -> disk image); anything found that way is at best `REQUIRES_REVIEW`. A test proves no extension can produce `SAFE_TO_CLEAN`.

## Path safety (`storage/paths.rs`)

Used by both the scanner and the delete action. Rejects relative paths, UNC paths, `..` components, drive roots, system directories, other users' profiles, and the structural folders of your profile (Documents, Desktop, Downloads, Pictures, Music, Videos, OneDrive, AppData, AppData\Local/Roaming/LocalLow) while allowing their contents to be classified. Comparison is case-insensitive and normalizes slashes and `\\?\` prefixes.

## UI

Overview (what could be recovered by class, by category), Largest folders, Largest files, Developer caches, Cleanup candidates. Sort by size, modified date or file count. Lists show 100 rows at a time ("Show more"). Every cleanup goes through the preview dialog.

## Scans stay out of the way (`storage/governor.rs`)

DevPilot must not become the source of pressure it reports, so every scan:

- runs on its own thread in **background mode** (`THREAD_MODE_BACKGROUND_BEGIN`: low CPU, I/O and memory priority; the third audited `unsafe` call, `windows/priority.rs`);
- asks a *governor* every 256 entries whether to continue, and sleeps 1 ms (normal) or 15 ms (throttled) between checks, so it never saturates the disk;
- **pauses** while RAM pressure is CRITICAL, a performance incident is being recorded, or an Android, Gradle or Flutter build is running; it **throttles** while pressure is HIGH or any other build or startup session runs. It resumes by itself, the progress phase reads `Paused: <reason>`, time spent paused does not count against the time budget, and cancelling while paused ends the scan immediately with a partial result;
- is never repeated automatically: a scan runs only when the user starts it.

The decision is a pure function of conditions DevPilot already measures (pressure state, active build sessions, incident state), so checking costs nothing extra.

## Deep scan of AppData (optional)

Normal scans keep looking only at known developer locations. **Deep scan AppData** is a separate button on the Storage page that asks first ("this can take several minutes") and is never started by anything else (`start_deep_appdata_scan` is called from that button only).

It measures the immediate sub-folders of `AppData\Local`, `LocalLow` and `Roaming` and lists those over 100 MB (60 at most). Windows-owned folders (`Microsoft`, `Packages`, `Temp`, `Programs`, `ConnectedDevicesPlatform`, `WindowsApps`, ...) are skipped, as are protected locations, links and junctions. It uses the same governor (plus a 25 ms rest between folders) and a 15 minute budget.

It is **report-only**: its entries have kind `appData`, classified `DO_NOT_TOUCH`, so the cleanup action refuses them (a test asserts it), and the UI shows no delete button. Its result is kept apart from the normal result, which is the whitelist cleanup is checked against, so a deep scan never invalidates cleanup candidates. A deep and a normal scan never run at the same time.

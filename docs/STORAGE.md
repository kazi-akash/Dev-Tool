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

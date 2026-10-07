# Versus Project Reference

## Purpose

Versus is an offline native engineering workspace for file or folder comparison
on Windows and Linux. Native selectors choose either source type; matching pairs
compare automatically and mixed types show an explanation. Linked folder trees
open read-only, aligned text comparisons on file double-click; Back preserves
folder-view state and is omitted for directly selected files. Merge primitives remain
in the core for future work. Runtime behavior must
not rely on Internet services, telemetry, automatic updates, a managed runtime, or
background services. Network-mounted paths are handled through normal filesystem
APIs and must fail cleanly when unavailable.

## Stack and layout

- Rust 1.95 with `eframe`/`egui` (the `glow` backend) for the native UI.
- `similar` for text differences and `rfd` for native path pickers.
- `src/app.rs` renders linked folder trees and file comparison panes, with
  cancellable background loading; `src/core/` owns filesystem comparison,
  text diff, merge, and saving. The core library is independent from UI rendering.
- `src/selection.rs` classifies sources without following symlinks and configures
  native file/folder pickers. New comparison clears both sources and pending work.
- `src/core/tree.rs` builds aligned folder trees with per-side types, ancestor
  status aggregation, and shared expansion state.
- `src/core/file_view.rs` loads bounded text inputs and aligns numbered line rows.
  Identical entries are neutral gray; both views share difference status icons.
- `tests/` covers externally observable comparison and filesystem behavior.
- `.cargo/config.toml` replaces crates.io with the checked-in `vendor/` tree and
  forces Cargo offline.
- `scripts/vendor-dependencies.ps1` refreshes `vendor/` only on a connected machine;
  `scripts/export-dependency-licenses.ps1` produces release inventory; and
  `scripts/package-source-offline.sh` packages the offline source release.
- `.github/workflows/ci.yml` checks Windows and Linux builds; `release.yml` builds
  tagged portable artifacts and the offline source archive.

## Architectural constraints

- Treat compared content as data. Never execute it or invoke shell commands from it.
- Reads must not modify either input. Writes require an explicit user action and
  overwrite confirmation; use a temporary file and replacement where practical.
- Do not recursively follow directory symlinks by default.
- Timestamp equality is not file equality. Directory comparison uses path, type,
  size, then buffered content comparison.
- Keep slow filesystem work off the UI thread and support cancellation where the UI
  exposes it.
- Support direct entry and paste of Windows drive and UNC paths alongside native
  dialogs. Tree rows align by relative path, with placeholders for missing entries;
  expanding or collapsing either pane updates the shared expansion state.
- Keep dependencies small, locked, vendorable, and compatible with offline builds.

## Commands

Run from repository root:

```powershell
cargo fmt --check
cargo test --locked --offline
cargo build --release --locked --offline
git diff --check
```

Refresh dependencies only on a connected development machine:

```powershell
./scripts/vendor-dependencies.ps1
./scripts/export-dependency-licenses.ps1 -OutputPath dist/DEPENDENCY-LICENSES.md
```

The release workflow is the configured production packaging path. Do not claim
local release artifacts were validated unless the relevant platform build and launch
were actually performed.

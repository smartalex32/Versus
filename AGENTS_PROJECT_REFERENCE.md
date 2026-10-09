# Versus Project Reference

## Purpose

Versus is an offline native engineering workspace for file or folder comparison
on Windows and Linux. Header buttons choose Folder Compare or File Compare;
browse opens the corresponding native picker directly. Matching pairs compare
automatically and mixed types show an explanation. Linked folder trees
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
  native pickers for the selected comparison mode. Switching modes clears both
  sources and pending work; New comparison keeps the selected mode and theme.
- `src/cli.rs` parses native command-line paths for Git/IDE launches. Two paths
  infer comparison mode; `--diff`/`--folder` require matching types. Startup uses
  the same background workers as browsing, and the native process stays alive
  until its window closes. Git null-device inputs represent absent file sides
  without opening devices. `--help`/`--version` exit before GUI initialization.
- `src/core/tree.rs` builds aligned folder trees with per-side types, ancestor
  status aggregation, shared expansion state, and full-tree traversal for diff
  navigation that skips folders and reveals changed files' collapsed ancestors.
- `src/core/file_view.rs` loads bounded text inputs and aligns numbered line rows.
  Rows retain original line numbers, terminators, and UTF-8 highlight ranges.
  Changed blocks use bounded similarity alignment to pair edited lines across
  insertions/deletions without discarding exact line anchors or original numbers.
  Identical entries are neutral gray; both views share difference status icons.
- Native accessibility is enabled through eframe AccessKit. Tree rows expose side,
  path, type, status, and size; visible file lines expose side, original number,
  status, and text. Up/Down tree navigation preserves focus side and Tab escapes.
  Focusable file panes support keyboard scrolling. Empty panes show persistent
  drop outlines; toolbar press feedback precedes release activation.
  UI zoom alone is saved through eframe storage; compared content and paths are
  session-local. Row/control geometry follows font metrics.
- Shared UI options filter identical entries and independently ignore inline
  whitespace or line endings. Differences-only filtering and both ignore rules are
  enabled by default. Filtering keeps a top-visible or nearest-surviving anchor;
  diff navigation reports the current change position in the fixed status area.
  Changes to ignore rules restart background workers
  while retaining the displayed result until its replacement is ready. Refreshes
  preserve expansion, selection and source-line scroll anchors; progress uses a
  fixed status area, and folder jumps clamp before painting to avoid transient shifts.
  Folder ignores use bounded UTF-8 normalization and fall back to byte comparison
  for binary, invalid UTF-8, or larger files. Native single-path drops select a
  pane and infer file/folder mode through the existing source classifier.
  Native drag pointer coordinates come from a narrow vendored winit patch, including
  corrected Windows OLE by-value coordinates and storage-medium cleanup,
  documented in `docs/vendor-winit-drop-position.md`; preserve it on vendor
  refresh. Pane targeting retains the drag position when a pointer-leave event
  follows it in the same frame, including drops without an earlier hover frame.
  Windows also reads the live native cursor relative to the app window while
  hovering or dropping, converting physical pixels to UI points and repainting
  during hover. A foreground border identifies the receiving side above pane
  contents. A position forwarded in the current frame takes priority over the
  native sample; older retained pointer state is used only if both are unavailable.
  Linux chooses X11/XWayland when `DISPLAY` exists for native file drops,
  retaining native Wayland browsing when no X11 display is available.
- `src/core/progress.rs` publishes synchronized stage snapshots. Measured stages
  support approximate stage-local remaining time; scanning and line alignment
  show activity without an invented total or estimate.
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

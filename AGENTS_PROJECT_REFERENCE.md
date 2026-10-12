# Versus Project Reference

## Purpose

Versus is an offline native engineering workspace for file or folder comparison
on Windows and Linux. Header buttons choose Folder Compare or File Compare;
browse opens the corresponding native picker directly. Matching pairs compare
automatically and mixed types show an explanation. Linked folder trees
open read-only, aligned text comparisons on file double-click; Back preserves
folder-view state and is omitted for directly selected files. File Merge opens
Left (Incoming) / Base (editable result) / Right (Current), with an optional
common ancestor input defaulting to Current when blank. File Merge is the third
centered header option; outer labels include Incoming and Current.
Runtime behavior must
not rely on Internet services, telemetry, automatic updates, a managed runtime, or
background services. Network-mounted paths are handled through normal filesystem
APIs and must fail cleanly when unavailable.

## Stack and layout

- Rust 1.95 with `eframe`/`egui` (the `glow` backend) for the native UI.
- `similar` for text differences and `rfd` for native path pickers.
- `src/app.rs` renders linked folder trees and file comparison panes, with
  cancellable background loading; `src/core/` owns filesystem comparison,
  text diff, merge, and saving. The core library is independent from UI rendering.
- `src/app/merge_view.rs` owns the GUI-only merge workflow, background input loads
  and saves, source selectors, per-block safe arrows and per-line conflict arrows,
  a full editable result, undo, and discard/overwrite confirmation. Input panes
  use the shared pane styling. The legend centers icon actions for Accept All Left,
  Accept All Right, Auto Merge Non-Conflicts, and Undo, with a conflict legend icon.
  Previous/next difference icons stay at the right with zoom immediately left;
  navigation shares vertical scrolling and advances even when all rows fit.
  File drops target all three panes using shared compare-mode pointer handling,
  dashed empty targets and foreground drag highlights. A background worker checks
  regular-file type before changing a source; invalid drops preserve the merge.
  Save and Finish Merge sit at the bottom right. Save writes the middle buffer to
  the loaded common ancestor path, or Current when Base is blank, retaining native
  path bytes. Edits stay in memory until explicit Save and overwrite confirmation.
  Finish only returns to comparison when the result is clean; it never writes.
  Unsaved Changes is bold highlight text at the bottom left. Dirty state compares
  the buffer against the actual save destination, including the initial Base
  contents. Save As exports a copy without retargeting Save or clearing that dirty
  state. Cancellation/failure preserves unsaved edits.
  Bulk source choices use one undo snapshot, preserving original bytes. Source
  header paths revert to the loaded inputs after a cancelled or failed reload.
  Input panes share diff alignment; the result uses its own live line numbers. All three panes
  share vertical scrolling, with independent horizontal scrolling.
  `src/core/merge_workspace.rs` owns optional-base conflict detection, action
  targets in the live buffer, atomic safe auto-merge and undo snapshots. With no
  base, the core offers conservative manual choices; the GUI uses a single Current
  snapshot as Base when its path is blank. Preserve saves emit the buffer exactly,
  including original endings and final newline. Results and undo stay session-local.
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
- Refresh reloads the active file view, including folder drilldown, and keeps its
  viewport. Folder refresh retains selection, expansion and scroll state for the
  same roots; removed selected entries are cleared. Workers recheck missing sides
  of folder files without following symlinks.
- `src/recent.rs` stores up to 20 recent file/folder source pairs in a bounded,
  versioned per-user local file, preserving native path bytes. The startup/New
  comparison screen offers one-click reopening through background classification.
  Recent rows show only the active file/folder mode as left-aligned path pairs
  with a bold vs separator and alternating neutral backgrounds.
  History IO runs on a serialized worker and queued writes finish on close; paths
  are recorded once both sources are classified, even if the comparison is still
  loading. File contents are never persisted. No new dependencies are required.
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
  Legend-row status text is clipped to one line with full messages on hover.
  The zoom percentage precedes adjacent magnifier −/+ buttons immediately left
  of the differences filter; Ctrl/Command + wheel changes
  UI scale using egui’s smooth zoom delta, without scrolling the comparison.
  Zoom is applied once per frame and bounded to egui’s 20–500% range.
  UI zoom is saved through eframe storage; recent source paths are saved separately
  and compared content remains session-local. Row/control geometry follows font metrics.
- Shared UI options filter identical entries and independently ignore inline
  whitespace or line endings. Differences-only filtering and both ignore rules are
  enabled by default. Filtering keeps a top-visible or nearest-surviving anchor;
  diff navigation reports the current change position in the reserved legend-row
  status area.
  Changes to ignore rules restart background workers
  while retaining the displayed result until its replacement is ready. Refreshes
  preserve expansion, selection and source-line scroll anchors; progress uses a
  reserved legend-row status area, and folder jumps clamp before painting to avoid
  transient shifts.
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
- Both workflows build separate native Linux artifacts for glibc 2.35 (Ubuntu
  22.04) and glibc 2.28 (Rocky 8.10) through `scripts/build-linux.sh`. Artifact and
  release filenames include the baseline so outputs cannot collide.
  `scripts/package-linux.sh` takes the baseline explicitly, bundles dynamically
  loaded window libraries, and checks the finished AppImage/runtime and bundled
  ELF files with `scripts/check-linux-compatibility.py`. Each baseline runs Rust
  tests, release compilation, packaged CLI checks and an Xvfb GUI startup test.
  The 2.28 AppImage adds automatic compatibility mode for X11 servers without
  XInput2, using a small capability probe and a private authenticated Xephyr
  display with software rendering. Modern displays retain the native path;
  private graphics libraries must stay isolated from ordinary launches.
  `--compat-x11` explicitly selects the fallback. The plain executable and 2.35
  AppImage use host graphics. Rocky CI tests automatic selection through nxagent,
  resizing, input, browsing, authentication and cleanup with host runtime files
  hidden. Preserve these tests, runtime license/source inventory, and both glibc
  checks when changing Linux packaging. Original Rocky crypto libraries and their
  vendor HMAC sidecars (including GMP’s `fipscheck/` file) must remain byte-identical.
  Final package verification and forced vendor FIPS self-tests (including negative controls and the nxagent GUI)
  run in CI. FIPS activation hooks are test-only; runtime host policy is preserved.
  Clipboard and file drops across the private-display boundary are not bridged.

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

## Agent template

The shared guide, `.codex/` roles/configuration, and `.agents/skills/` were checked
against [Codex-App](https://github.com/smartalex32/Codex-App) commit
`67cabb07d2345053322491c0308357d2e8ecd104` on 2026-10-12 and match that revision.
Keep this project reference specific to Versus when refreshing the template.

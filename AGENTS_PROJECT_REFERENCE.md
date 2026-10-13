# Versus Project Reference

## Purpose

Versus is an offline native engineering workspace for file or folder comparison
on Windows and Linux. Header buttons choose Folder Compare or File Compare;
browse opens the corresponding native picker directly. The comparison toolbar
switches between Diff and Merge for the same open sources. Diff is read-only and
compares automatically. Merge uses two independent editable working copies; there
is no desktop Base pane or three-way workflow. Its optional Run Diff setting starts
off, leaving editing, chunking, transfers, and protected ignored ranges independent
from comparison. Folder Merge browses directory metadata lazily and loads contents
when a file is opened. Traditional three-way merging is a separate `--merge` CLI
operation. Runtime behavior must
not rely on Internet services, telemetry, automatic updates, a managed runtime, or
background services. Network-mounted paths are handled through normal filesystem
APIs and must fail cleanly when unavailable.

## Stack and layout

- Rust 1.95 with `eframe`/`egui` (the `glow` backend) for the native UI.
- `similar` for text differences and `rfd` for native path pickers.
- `src/app.rs` renders the existing comparison workspace, the Diff/Merge toggle,
  file/folder navigation, and mode-specific comparison options. Diff preferences
  are kept separately from Merge preferences; the three Merge options start off.
  Enabling Run Diff schedules a debounced comparison for the open pair on a
  cancellable worker; disabling it drops pending diff work and removes alignment
  rows/highlights without resetting working copies or history. The alignment panel
  is separate from the editors, so its differences-only filter does not filter text.
- `src/app/interactive_view.rs` renders the two-way editing workspace. Each side
  has its own text buffer, line numbers, dirty state, scroll position, and explicit
  Save Left/Save Right/Save Both action with a pending-content preview. Selection
  transfers can insert at a chosen line or replace an explicitly selected target.
  Accept All Left/Right copy one side into the other as a single transaction, while
  honoring destination protection. Auto Merge previews incoming-only insertions
  with matching boundary context; it does not infer replacements. Chunks are
  heuristic structural blocks for recognized source-like text or blank-line
  paragraphs, not full language parses; users can create, split, combine, and
  adjust manual chunks. Protected ranges are maintained independently and remain
  in the file. They block transfers and search replacement, while direct typing in
  those ranges remains allowed. Undo and Redo preserve transfer, edit, chunk, and
  ignored-range state. Diff rows only assist with alignment and target suggestions
  when enabled; manual transfers work without them. File-pair history uses
  canonical tagged snapshots and survives navigation and alternate pairing.
- `src/app/folder_browser.rs` enumerates one directory level at a time through
  `directory_entries_at`, which reads entry metadata and relative paths without
  comparing file contents. Expanding a directory requests that listing; opening a
  file loads its working copy. The recursive Find and Replace window supports left,
  right, or both roots; plain text/regex, case, and whole-word matching; match
  selection; previews; staged working-copy application; and batch undo/redo. Batch
  undo is atomic across participating files; newer edits or ignored-range changes
  in those files must be undone first. Binary, non-text, and files above the 32 MiB
  text limit are skipped. Saving remains an explicit operation. With Run Diff on,
  cancellable per-file comparisons overlay staged content and protected ranges on
  the folder results; unchanged working-copy results are reused. Difference jumps
  reveal the target using only shallow ancestor listings.
- `src/core/interactive.rs` is the diff-independent two-way text model. Its
  line-number ranges are zero-based and half-open. The chunk detector prioritizes
  recognized syntax blocks and falls back to blank-line paragraphs. Transfers can
  insert or replace; destination protected ranges reject transfers or replacements,
  while manual text edits can modify those lines. Mutations have transactional
  undo/redo, with chunk and ignored-range boundaries remapped after edits.
- `src/core/work_files.rs` owns lazy directory entries and text working copies.
  It records the exact original bytes for external-change checks and supports
  UTF-8 (with or without BOM) and BOM-marked UTF-16 little/big endian, up to the
  32 MiB text limit. Saves retain the source encoding and use safe replacement; a
  changed original causes save to fail rather than overwrite external edits.
  Physical file identity is resolved on a background worker for canonical pair
  history and save deduplication. Original native paths are retained as write
  destinations.
- `src/core/search_replace.rs` owns recursive text search and replacement previews.
  It avoids following symlinks and skips binary/oversized files; selected results
  are applied to working copies and are never written until an explicit save.
- `src/core/merge.rs` owns traditional three-way merge calculation and conflict
  markers. `src/main.rs` executes the CLI request, writes its output, returns 0
  when clean, 1 when conflicts were written, and 2 on argument, IO, or merge error.
  Syntax is `versus --merge BASE OURS THEIRS --output OUT [--overwrite]`; this
  workflow does not start the desktop GUI.
- `src/selection.rs` classifies sources without following symlinks and configures
  native pickers for the selected Folder/File comparison mode. Switching between
  Folder and File starts a fresh session and clears both sources and pending work;
  the Diff/Merge toggle retains the current workspace. New comparison keeps the
  selected Folder/File mode and theme.
- `src/cli.rs` parses native command-line paths for Git/IDE launches and the
  separate conventional three-way merge. Two paths infer comparison mode;
  `--diff`/`--folder` require matching types. Startup uses
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
- Diff UI options filter identical entries and independently ignore inline
  whitespace or line endings. Differences-only filtering and both ignore rules are
  enabled by default in Diff. Merge has a separate set of these three options,
  initially off. Filtering keeps a top-visible or nearest-surviving anchor;
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
- Diff reads must not modify either input. Merge edits stay in working copies until
  explicit save; check original bytes before writing, preserve supported source
  encoding, and use a temporary file and replacement where practical.
- Do not recursively follow directory symlinks by default.
- Timestamp equality is not file equality. Diff directory comparison uses path,
  type, size, then buffered content comparison. Merge directory navigation reads
  only metadata until a file is opened or an explicit recursive search is run.
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
`67cabb07d2345053322491c0308357d2e8ecd104` on 2026-10-13 and match that revision.
Keep this project reference specific to Versus when refreshing the template.

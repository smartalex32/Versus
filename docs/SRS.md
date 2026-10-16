# Versus Software Requirements Specification

Version 1.0 · 2026-10-16

## 1. Purpose and baseline

This SRS defines the current supported behavior of Versus for implementation,
maintenance, and regression review. It is derived from the merged application,
its tests, release workflows, and README at `3ea3e0fa`, with the unused desktop
three-way interfaces removed by the accompanying audit. It does not turn ideas
in the [original PRD](PRD.md) into release commitments. Where that historical
roadmap conflicts with this document, this document describes the current scope.

“Shall” identifies a requirement. Verification references identify relevant
implementation and tests; they do not assert that every requirement has an
automated acceptance test or that every supported platform was tested locally.

## 2. Product and operating environment

Versus is a native, offline desktop workspace for engineers comparing local files
and directories, editing two text working copies, and explicitly saving changes.
It also provides a separate noninteractive three-way merge command for terminal,
Git, and IDE callers. A user needs filesystem access to the chosen sources and
write access to a save destination; no Versus account is required.

Supported release targets are Windows 10/11 x86-64 and Linux x86-64. Linux
distributions provide separate glibc 2.35 and 2.28 builds. macOS can be used for
development checks but is not a specified release target. Network storage must be
mounted or otherwise accessible through operating-system paths, including Windows
drive letters, mapped drives and UNC paths, and Linux mount paths. Authentication,
mounting, and availability of those paths belong to the operating system.

## 3. Terms and state

| Term | Meaning |
| --- | --- |
| Comparison mode | File or Folder; selects source types and navigation. |
| Workspace mode | Diff or Merge; selects read-only comparison or editable working copies. |
| Run Diff | Optional comparison assistance inside Merge, initially disabled. |
| Working copy | In-memory text plus original encoding, disk-byte fingerprint and document identity. |
| Dirty | Working content differs from its saved baseline. |
| Chunk | Heuristically recognized structural block or blank-line paragraph. |
| Search transaction | Validated byte edits applied to working copies as one undoable batch. |
| CLI merge | Base/ours/theirs calculation that writes one output without opening the GUI. |

The initial workspace is Folder Diff. Selecting File/Folder starts a fresh
session; switching Diff/Merge retains the current sources and editable session.
Folder Diff uses aligned paired trees. Folder Merge browses each side
independently. A working-copy edit, a comparison result, and a disk save are
distinct state changes.

## 4. Functional requirements

### 4.1 Sources and sessions

- **SRC-01:** The app shall offer File and Folder modes and a Diff/Merge toggle.
  Selecting a different comparison mode shall cancel pending session work and
  clear sources. Selecting the active mode shall retain the session; Folder may
  return from a drilled-down file to its tree.
- **SRC-02:** Browse shall open the selected mode's native file or folder picker.
  Native dialogs shall allow entering or pasting filesystem paths. Source
  classification shall reject unsupported types without following symlink targets.
- **SRC-03:** A single native file/folder drop shall target the indicated pane and
  infer its comparison mode. Multiple-item drops shall report an error without
  replacing sources. A visible border shall identify the receiving side.
- **SRC-04:** Diff shall compare automatically once compatible sources are ready.
  Merge shall load and permit navigation on either side independently, including
  before the peer is selected. Incompatible Diff types shall show an explanation.
- **SRC-05:** New comparison shall clear the session and pending work while
  retaining comparison mode, theme, and comparison preferences. Leaving an edited
  Merge session or closing its window shall require choosing to discard or keep
  merging, allowing the user to return to the workspace and save first.
- **SRC-06:** The app shall retain at most 20 recent source pairs locally per user,
  newest first, preserving native paths and storing no compared contents. Reopening
  a pair shall reclassify and load it in the background. The list shall show the
  current comparison type. Unavailable sources shall produce an error.

### 4.2 Read-only folder and file comparison

- **DIF-01:** Folder Diff shall recursively align entries by relative path and
  distinguish identical, different, left-only, right-only, type mismatch, and read
  error states. Shared ancestor directories shall aggregate descendant states.
  Empty folders shall be included; timestamps alone shall not establish equality.
- **DIF-02:** Folder comparison shall use filesystem metadata and buffered content
  comparison. Symlinks shall be compared as links without recursive traversal.
  Per-side file sizes and directory totals shall exclude symlink target contents;
  unknown or incomplete totals shall remain visibly unknown.
- **DIF-03:** Folder panes shall share expansion and vertical scrolling, retain
  placeholders for missing peers, and support expanding/collapsing all entries.
  Opening a regular file shall show aligned read-only lines; Back shall restore
  the folder selection, expansion and viewport.
- **DIF-04:** File Diff shall show original line numbers, one-sided placeholders,
  changed-line states, inline changed-text highlights, and line-ending badges
  when endings are compared. Similar edited lines shall be aligned within bounded
  work; exhausted budgets may fall back to positional or whole-line highlighting.
- **DIF-05:** The default Diff options shall enable Show only differences, Ignore
  whitespace, and Ignore line endings. Ignore rules shall be independent. Inline
  Unicode whitespace shall be ignored without discarding line boundaries; strict
  ending comparison shall distinguish LF, CRLF, CR and absent final terminators.
- **DIF-06:** Folder ignore rules shall normalize only bounded valid UTF-8 text;
  binary, invalid UTF-8 and oversized inputs shall use byte comparison. File Diff
  shall explain binary, unsupported encoding, oversized, unavailable or timed-out
  text comparisons. A missing file side shall be represented as empty.
- **DIF-07:** Previous/Next shall navigate changed rows/files, stop at the ends,
  report the current position, and reveal collapsed folder ancestors. Filtering
  shall retain a visible anchor where possible. Diff vertical scrolling shall be
  linked; each pane shall support horizontal scrolling.
- **DIF-08:** Refresh shall reread a direct Diff's sources in the background and
  preserve the applicable viewport, tree expansion and selection. Removed selected
  entries shall be cleared. Ignore changes shall recompute; filtering shall reuse
  the loaded result.

### 4.3 Interactive Merge

- **EDT-01:** Merge shall provide two independent editable text panes with line
  numbers, dirty indicators, independent loading and scrolling, and transactional
  undo/redo. Edits shall remain in memory until an explicit save.
- **EDT-02:** Chunks shall be detected automatically using recognized structural
  blocks with blank-line paragraph fallback. Chunk and expanded line arrows shall
  copy into the destination cursor without modifying the source. Transfers shall
  require both files, preserve source text bytes in the working buffers, and be
  undoable. Accept All Left/Right shall replace the other buffer as one transaction.
- **EDT-03:** Pair history shall survive file navigation and alternate pairing using
  canonical document identity. Aliases shall not create divergent writable copies
  or duplicate saves for the same resolved identity.
- **EDT-04:** Run Diff shall initially be off. Editing, chunk detection and
  transfers shall work without it. Enabling it shall schedule cancellable,
  debounced comparison of working copies. Disabling it shall cancel pending
  comparisons and clear decorations without resetting edits/history.
- **EDT-05:** Merge shall have separate filter and ignore preferences. First
  enabling Run Diff shall enable all three options; subsequent toggles shall retain
  the user's choices. Filtering shall hide lines without changing buffer contents.
- **EDT-06:** Typing and undo/redo shall retain display-only decorations while
  a replacement comparison is pending. Stale results shall not authorize current
  difference navigation or completed folder status. Fresh results shall replace
  the presentation; navigation shall use the affected sides' original line numbers.
- **EDT-07:** Switching an edited pair into Diff shall compare retained working
  copies through the read-only aligned view. Refresh shall compare those buffers
  without replacing them with disk text. Returning to Merge shall restore editing
  and history.
- **EDT-08:** Folder Merge shall enumerate metadata one directory level at a time
  for each side and load contents only when opened, searched, or explicitly
  compared. File selection, tree expansion and scrolling shall remain independent.
  Run Diff folder statuses shall incorporate unsaved working-copy changes.

### 4.4 Find and replace

- **FND-01:** File and Folder Merge shall offer a centered Find and Replace modal
  dismissed by its close control, outside click, or Escape. File search shall use
  selected files only; folder search shall recurse without following symlinks.
- **FND-02:** Search shall support literal/regex, case-sensitive, and whole-word
  matching. Typing shall schedule search after 250 ms without edits; option,
  refresh, reopen and history actions shall start needed searches immediately.
  Empty queries and dismissal shall cancel pending searches.
- **FND-03:** Every search shall collect both sides and stream bounded result
  batches. LEFT/RIGHT/BOTH scope shall constrain replacements; cached results on
  the other side shall remain available. Side counts shall reflect included hits.
  Rows shall show file, line and bounded highlighted previews, with hit exclusion.
- **FND-04:** Search refresh shall repopulate exclusions and reread clean files;
  unsaved copies shall take precedence. Reopening shall search again. Binary,
  unsupported text and files over the text limit shall be skipped.
- **FND-05:** Replace Next/All shall immediately apply included original hits as an
  undoable in-memory transaction, honoring regex captures and literal replacement
  semantics. Remaining hits shall be remapped through exact byte edits without
  searching replacement text. There shall be no separate preview/Apply UI step.
- **FND-06:** Partial, cancelled, failed or stale searches shall not authorize
  replacement. All transaction participants shall validate before publication.
  Refreshed clean editor text shall be reconciled on staged clones; dirty or
  conflicting identities shall fail safely. Batch undo/redo shall be atomic across
  participating files and require undoing newer participating edits first.

### 4.5 Saving and data integrity

- **SAV-01:** Save Left/Right and Save Both shall be explicit actions. Active-file
  saves shall provide pending-content preview. Save Both shall include retained
  dirty files changed by replacement even after the search modal is closed.
- **SAV-02:** Interactive loads/saves shall support UTF-8 with/without BOM and
  BOM-marked UTF-16 little/big endian within the 32 MiB text limit. Saving shall
  retain the source encoding and edited line endings, including mixed endings.
- **SAV-03:** Save shall compare the original byte fingerprint before replacing a
  destination and reject external changes. Native write destinations shall remain
  distinct from canonical history keys. Failed/cancelled saves shall retain edits.
- **SAV-04:** Saves shall use temporary files and safe replacement/recovery paths.
  Successful saved baselines shall reflect the bytes/text actually written. Save
  Both is a collection of per-file saves, not a filesystem-wide atomic commit;
  failures shall be reported without marking failed participants saved.

### 4.6 CLI and external tools

- **CLI-01:** No arguments shall open the empty GUI. Two native paths shall infer
  File/Folder mode; `--diff`/`--folder` shall require the respective types. `--`
  shall allow dash-prefixed paths, relative paths shall use the caller's directory,
  and Git null-device inputs shall represent absent file sides without opening
  devices. `--wait` shall remain accepted for tool compatibility.
- **CLI-02:** GUI launches shall keep the caller's process alive until the window
  closes. Normal close shall return 0 even when files differ; invalid arguments
  shall return 2 and GUI startup failure 1. `--help`/`--version` shall print and
  exit without initializing the GUI.
- **CLI-03:** `versus --merge BASE OURS THEIRS --output OUT [--overwrite]` shall run
  without a GUI, combine disjoint edits and identical concurrent edits, and emit
  `OURS`/`THEIRS` conflict markers for incompatible overlapping changes. Inputs
  shall be bounded UTF-8 text; BOM and line endings shall be normalized for merge.
  The output shall be refused if it exists unless overwrite is explicit. Exit
  codes shall be 0 for clean output, 1 for written conflicts, and 2 for errors.

### 4.7 Presentation and diagnostics

- **UI-01:** The app shall offer light/dark themes, visible status legends,
  keyboard navigation and native accessibility descriptions for visible tree/file
  rows. File panes shall support keyboard scrolling; toolbar actions shall activate
  on release with press feedback and tooltips.
- **UI-02:** Zoom shall support buttons, reset, Ctrl/Command keyboard shortcuts
  and modified wheel input within 20–500%. Zoom shall persist between launches;
  edited contents and undo histories shall remain session-local.
- **UI-03:** Background activity shall occupy reserved footer space. Measured
  stages shall show stage percentage; stages without totals shall show activity.
  Cancellable comparisons shall expose cancellation. Errors shall be deduplicated
  with hover details and dismissible by pointer activity in the panes.
- **LOG-01:** Desktop diagnostics shall remain local beside the executable (outer
  AppImage on Linux), rotate at 5 MiB with one backup, and omit compared contents.
  Setup/write failures shall be reported in the UI. Logging shall use a bounded
  off-thread queue and a shutdown flush bounded to 500 ms.

## 5. Nonfunctional requirements and constraints

- **NFR-01 — Offline:** Runtime shall require no Internet access, accounts,
  telemetry, automatic update service, remote API, or managed language runtime.
  Compared content shall be data and shall never be executed as code or commands.
- **NFR-02 — Responsiveness:** Slow classification, content loading, comparisons,
  search, saves and persistence shall run outside UI painting. Replaced/cancelled
  work shall not publish results into a newer session. Folder Merge shall avoid
  unnecessary recursive work while browsing.
- **NFR-03 — Bounds:** Text processing shall enforce the 32 MiB input limit.
  File display diff shall have a two-second processing budget; similarity and
  inline highlighting shall have bounded work and documented fallbacks. This is
  not a two-second end-to-end filesystem latency guarantee. No general startup,
  memory or large-tree throughput target has been established.
- **NFR-04 — Failure handling:** Missing sources, permission failures, unavailable
  network paths, encoding errors and worker failures shall produce actionable
  errors without silently changing source contents. Directory traversal shall not
  recursively follow symlinks. Identical folder roots shall be valid.
- **NFR-05 — Distribution:** Windows shall ship a portable executable/archive.
  Linux shall ship baseline-specific AppImages and archives. The glibc 2.28
  AppImage shall select its private authenticated X11/software-rendering fallback
  for displays lacking XInput2, and accept `--compat-x11`. Clipboard/file-drop
  bridging into that private display is outside scope; native Wayland without
  X11 shall retain browsing but has no native file drops.
- **NFR-06 — Reproducible source delivery:** Rust 1.95, locked dependency versions,
  vendored dependencies and Cargo offline configuration shall support
  `cargo build --release --locked --offline` with a provisioned toolchain and
  platform build libraries. Vendor updates shall preserve egui and winit patches,
  license notices, and runtime integrity files required by packaging.
- **NFR-07 — Maintainability:** Core comparison/editing/IO behavior shall remain
  independent of egui. Changes shall have proportional behavioral validation;
  Windows and both Linux baselines shall retain CI build/test/package checks.

## 6. Verification and acceptance map

| Requirements | Implementation evidence | Verification |
| --- | --- | --- |
| SRC-01–05 | `selection.rs`, `app/session.rs` transitions, `app.rs` drop/exit handlers | `selection` unit tests; `app::tests`; `app::operating_mode_tests`; native picker/drop/close checks |
| SRC-06 | `recent.rs`, `app.rs::RecentStore` | Recent-history round-trip/error tests; app recent-session tests |
| DIF-01–03 | `core/directory.rs`, `core/tree.rs`, folder UI in `app.rs` | `tests/core.rs`, `tests/folder_tree.rs`; folder UI navigation/refresh tests |
| DIF-04–08 | `core/file_view.rs`, shared `app/file_style.rs`, file UI in `app.rs` | `tests/file_view_options.rs`; file alignment/filter/navigation/refresh UI tests |
| EDT-01–03 | `core/interactive.rs`, `core/chunk_detection.rs`, `app/interactive_view.rs` | `tests/interactive.rs`; document identity, transfer and pair-history unit tests |
| EDT-04–08 | `app/session.rs`, `app/interactive_view.rs`, `app/interactive_view/projection.rs`, `app/folder_browser.rs`, `core/lazy_directory.rs` | Operating-mode tests; decoration, stale-worker, independent-pane and overlay tests |
| FND-01–06 | `core/search_replace.rs`, `app/search_session.rs`, `app/folder_browser.rs`, `app/folder_browser/search_state.rs`, `app/search_result_rows.rs` | Search/session unit tests; modal, streaming, replacement and batch-history tests |
| SAV-01–04 | `core/work_files.rs`, `core/save.rs`, interactive/folder save workers | Core/interactive save tests; UI preview, retained-save and failure tests |
| CLI-01–03 | `cli.rs`, `main.rs`, `core/merge.rs` | Parser tests; `tests/cli.rs`; core merge insertion/overlap/conflict regressions |
| UI-01–03, LOG-01 | Shared UI helpers; `diagnostics.rs` | Headless egui geometry/interaction tests; diagnostic queue/rotation tests; native accessibility checks |
| NFR-01–04 | Core IO and worker owners | Cancellation, error, size-limit and no-follow tests; source review; mounted-share/manual responsiveness checks |
| NFR-05–07 | `.cargo/config.toml`, Cargo manifest, vendor patches, scripts and workflows | Offline format/test/release build; Windows CI; Linux baseline packaging, glibc/FIPS checks and Xvfb/nxagent GUI smoke tests |

Release acceptance requires the applicable automated suites and target-platform
packaging checks to pass. Native file dialogs, screen-reader behavior, drag/drop,
actual mapped/UNC mounts and disconnected-share recovery require verification in
the corresponding OS environment; a local development-host test run does not
substitute for those checks. Specific evidence for this audit belongs in
[the audit report](APP_AUDIT.md), not in these ongoing requirements.

## 7. Scope exclusions

The current desktop has no Base pane, three-way conflict-resolution editor,
automatic conflict merge, manual chunk management, manually ignored line ranges,
or separate replacement preview/Apply workflow. It has no general Save As workflow,
session-content persistence, folder synchronization, remote protocol client,
binary/hex editor, image/office/PDF comparison, plugin execution, or Git repository
management. The CLI three-way merge and external Git/IDE difftool invocation
remain supported. Proposed additions require their own requirements and review.

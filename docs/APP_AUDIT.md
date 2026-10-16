# Application audit: cleanup and maintainability

Date: 2026-10-16. Baseline: merged `main` at `3ea3e0fa` (PR #11).
Branch: `codex/app-audit-cleanup-srs`.

## Scope and method

Reviewed first-party Rust source, integration tests, manifest dependencies, current
project guidance, README, PRD and release workflows. Traced callers from GUI and
CLI entrypoints and distinguished production calls from tests and target-specific
code. Used two independent read-only investigations for core and UI code, then
checked removal candidates against repository-wide references. This is a source
and test evaluation, not a new interactive UX study or dependency security audit.
Third-party vendored implementations were outside the deletion scope; their local
platform fixes and license material are required by the application.

Compiler silence alone is insufficient here: publicly exported Rust library
functions can be unused by the application without triggering dead-code warnings.
Conversely, a test-only call does not prove that a supported primitive is obsolete.
Removal requires both caller evidence and a retired/duplicate purpose.

## Completed cleanup

| Finding | Evidence and change | Behavior retained |
| --- | --- | --- |
| Retired desktop three-way comparison entrypoints | Removed `compare_three_way`, `compare_three_way_text`, and `compare_three_texts` in `core/merge.rs`. The first had no callers; the text wrapper chain was used only by integration tests. Its `FileCompareOptions` argument was ignored. | Active `main.rs::run_merge` still calls `merge_three_way`/`merge_three_way_text`. Algorithm regressions now exercise that interface. |
| Retired desktop hunk resolution | Removed unused `resolve_merge_hunk`, `MergeChoice`, and `render_three_way` (the latter only exercised by the old choice test). Removed retained base/left/right buffers and per-hunk replacement/range metadata used for that UI. | CLI output, hunk classification/base ranges, conventional conflict markers, and exit behavior remain. The former choice test now asserts exact CLI conflict output and identical-change classification. |
| Unused working-copy path mutator | `WorkingFile::set_path` had no callers, including tests; removed it. There is no current Save As workflow. | Existing native destinations and background-resolved identities remain unchanged. |
| Unused duplicate saved-baseline helper | `TwoWayWorkspace::mark_saved` had no callers; removed it. | Save completion uses `mark_saved_text`, recording the actual saved snapshot even if newer edits exist. |
| Obsolete warning suppression | Removed `allow(dead_code)` from the test-only `InteractiveView::editor_id`, which is used by editor interaction tests. | Test helper and tests remain. |
| App implementation obscured by inline tests | Moved the existing `app::tests` body into `src/app/tests.rs` without changing its module name or behavior. | All existing tests and private-state access remain; `operating_mode_tests` remains separate. |
| Conflicting product documentation | Added `SRS.md` with stable requirement IDs, current scope and verification mapping; marked the original PRD historical and linked both documents from README. | Historical product rationale remains available without being mistaken for current acceptance criteria. |

These removals change the exported Rust library surface. No repository production
caller or documented integration uses the removed interfaces; external Rust code
using them would need migration. The native executable's supported workflows and
CLI flags are preserved.

## Evaluated and retained

- **CLI three-way merge:** actively exposed by the parser/help and `run_merge`,
  with integration coverage. Retirement of the desktop three-way workflow does
  not retire this command.
- **Compatibility paths:** `--wait`, Windows native drop/cursor handling, Linux
  X11 selection and packaged glibc 2.28 compatibility mode have active caller or
  release requirements. Host-specific non-use is not dead-code evidence.
- **Search previews:** `SearchPreview` and `preview_replace` are still the
  preparation format for immediate undoable replacements. Removal of the preview
  dialog did not remove transaction preparation. `apply_preview` is also used
  by core integration/unit tests as a working-copy helper; replacing those tests
  with UI transactions is a separate test/interface consolidation decision.
- **Core convenience interfaces:** direct comparisons, collecting search, paired
  directory listing and transfer helpers are exercised by behavioral library tests.
  Unlike the retired desktop resolver, their underlying capabilities remain in
  scope. They were not deleted merely because current GUI callers use streaming,
  single-side or document-based variants.
- **Merge selection flags:** `merge_file_open` and per-side state participate in
  opening one side before the peer is selected. Source review and existing
  operating-mode regressions show they are not interchangeable without redesign.
- **Dependencies:** every direct dependency has source/test or platform references.
  No manifest dependency was confirmed unused. Offline vendoring and patched egui
  remain necessary; no dependency/vendor pruning was performed.

No additional obsolete UI feature path was confirmed. This does not claim that all
possible dead code is mathematically excluded, particularly for exported library
interfaces and conditional platform code.

## Structure improvements

The following recommendations were implemented in the structural follow-up on
the same branch. The table retains the acceptance criteria used for that work.

The core/UI separation is useful and should remain. `search_session.rs` and
`file_style.rs` already demonstrate focused modules: result remapping and file
painting can evolve without owning filesystem work. Prefer small interfaces that
hide state and ordering rules; splitting files alone does not create that depth.

| Priority | Audit-baseline evidence | Change and acceptance criteria |
| --- | --- | --- |
| 1 | `VersusApp` still owns mode transitions, jobs, folder/file rendering and shared painting in `app.rs`, even after test extraction. `set_workspace_merge`, `switch_comparison_mode` and `poll_*` jointly guard retained edits and stale work. | Extract a session module that owns mode transitions and worker publication through a small interface. Keep UI widgets in the app. Preserve independent Merge sides, retained working-copy Diff, cancellation and generation checks; verify with existing operating-mode/refresh tests before moving more rendering. |
| 2 | `FolderBrowser` owns lazy listings, search debounce/results, save ownership and folder overlay caches in one state struct. `new_search` and `set_search_sources` already support file-only search without enumeration. | Extract search-session ownership from directory navigation: query/options, current job, selected hits and results. Keep the app's transaction coordinator responsible for canonical documents and atomic batch history. Preserve both-side collection, cached scope changes, stale-search rejection and Save Both after dismissal. |
| 3 | `InteractiveView` combines canonical documents/pair history, diff jobs, editable projections and painting. `MergeDecoration`, `PendingMergePresentation` and `EditorProjection` encode display remapping. | Move projection/decorations into a focused internal module whose interface maps source edits to displayed rows. Keep history and save identity in one owner. Preserve byte offsets, hidden-line edits, pending diff decorations, cursor restoration and independent scrolling using existing regression tests. |
| 4 | `core/work_files.rs` combines text encoding/fingerprints/saving with `LazyDirectorySide`, `LazyDirectoryItem` and metadata traversal. | Separate lazy listing into a core module, retaining the existing interface initially. No symlink traversal, content reads on expansion or change to root/ancestor validation. Validate lazy listing and no-follow regressions. |
| 5 | `core/interactive.rs` combines history/byte transactions with `detect_chunks`, `StructuralKind`, `LexState` and brace heuristics. | Extract chunk detection as a pure text-to-chunks module. Preserve chunk IDs/ranges and heuristic fallback; use existing chunk and transfer tests. Avoid language-parser dependencies or changing heuristics during the move. |

Keep future extractions in separate behavior-preserving changes with their tests.
Do not introduce a generic worker framework solely because channel
and cancellation code repeats: comparison, search streaming and saves have
different publication and data-integrity rules. A refactor must reduce what callers
need to know, not spread the same flags and invariants across more files.

## Initial audit validation

Checks run with Rust 1.95 on the macOS development host:

- Baseline `cargo check --all-targets --locked --offline`: passed before edits.
- Focused `cargo test --locked --offline --test core --test cli`: 23 passed.
- Final `cargo test --locked --offline`: 348 passed, zero failures or ignored
  tests. The Windows-only drop target has zero tests on this host.
- Final `cargo fmt --check`: passed.
- Final `cargo build --release --locked --offline`: passed.
- `git diff --check`: passed; local links in the new documents resolve.
- Compared the relocated app test suite with the extracted and formatted baseline:
  identical contents, retaining all 121 tests. Reviewed the implementation diff
  against callers and requirements; no active executable workflow was removed.

Windows/Linux packaging, GUI launches, native accessibility, pickers, actual
network mounts and native drops were not exercised in this audit; those require
their CI/runtime environments. No interactive supported-platform evaluation is
implied by headless egui tests on the development host. The initial audit was
subsequently committed as `c0cb31c5` and pushed to the audit branch.

## Structural follow-up

- `app/session.rs` now owns source and mode transitions, retained workspace state,
  comparison workers, cancellation and ordered result publication. The window
  renders that state and calls the session's transition methods. Polling order,
  Diff/Merge preferences, independent Merge sides and retained edits are preserved.
- `app/folder_browser/search_state.rs` owns search query/options, debounce,
  worker cancellation/streaming, selected hits, scope and displayed side.
  `FolderBrowser` retains directory navigation, save ownership and replacement
  coordination with `InteractiveView`. Scope changes still reuse both-side results.
- `app/interactive_view/projection.rs` owns derived pending decorations, rebasing,
  filtered editor/source mappings and cursor restoration. Canonical documents,
  pair history and save identity remain in the editor owner.
- `core/lazy_directory.rs` owns shallow metadata listings and no-follow path
  validation, preserving the existing public exports through `work_files`.
- `core/chunk_detection.rs` owns pure automatic chunk detection with unchanged
  IDs and heuristics. `core/text_lines.rs` shares the exact line segmentation used
  by chunk detection and byte-preserving editing, avoiding duplicated rules.
- Every remaining inline unit/headless UI test module has moved into a dedicated
  `<module>/tests.rs`. Existing integration tests remain in `tests/`. Test-only
  editor fixtures/inspection helpers also live in test files and are excluded
  from production builds. Existing module paths and all test names are retained.

The refactor introduces no dependency or user-visible behavior changes. Session,
search and editor-presentation extraction received independent read-only review;
no actionable behavior regressions were found. Windows-specific tests remain
guarded by their existing platform conditions.

Production entry-file sizes after extraction (lines, excluding the new modules
and separate tests):

| File | Audit baseline | After structural follow-up |
| --- | ---: | ---: |
| `src/app.rs` | 5,248 | 3,935 |
| `src/app/interactive_view.rs` | 6,083 | 2,950 |
| `src/app/folder_browser.rs` | 3,616 | 1,741 |
| `src/core/interactive.rs` | 941 | 597 |
| `src/core/work_files.rs` | 813 | 490 |

Final follow-up validation on the macOS development host:

- `cargo test --locked --offline`: all 348 tests passed with no compiler warnings.
  Compared the executed names with the audit baseline: no added or missing tests.
- `cargo fmt --check`, `git diff --check`, and
  `cargo build --release --locked --offline`: passed.
- Verified all 299 first-party source test functions, including platform-guarded
  tests, remain in dedicated test files. Windows-only tests were not executed here.
- Reviewed module interfaces and the complete diff; updated project-reference and
  SRS implementation pointers. Local documentation links resolve.

Windows/Linux packaging and native interaction checks retain the limitations
listed above; no supported-platform release artifact was produced by this host.

## Capability-oriented follow-up after `48a9f6ba`

The remaining large implementations were separated by capability while retaining
their state owners and existing application/core interfaces. Parent files now
show state and orchestration; child files group complete operations. Shared visual
dependencies name their defining module. Non-painting row anchors/filtering and
result summaries live with Session rather than renderers. No forwarding framework,
dependency or new product behavior was introduced.

| Entry file | Before | After |
| --- | ---: | ---: |
| `src/app.rs` | 3,935 | 144 |
| `src/app/interactive_view.rs` | 2,950 | 152 |
| `src/app/folder_browser.rs` | 1,741 | 222 |
| `src/app/session.rs` | 1,457 | 229 |
| `src/core/file_view.rs` | 921 | 216 |

These counts describe entry-file navigation, not deleted functionality or a measure
of module depth. The largest first-party production Rust file is now 645 lines.
The app/editor/folder test suites are also grouped by capability with shared
fixtures. All 299 source test functions remain; platform conditions are unchanged.
Some fully qualified test paths changed to include the new capability grouping.

The [architecture guide](ARCHITECTURE.md), six focused guides, and
[glossary](../GLOSSARY.md) provide the human route through the application.
`AGENTS.md` and the shorter `AGENTS_PROJECT_REFERENCE.md` route agents by task,
file ownership, invariants and test command. The SRS implementation map was updated
to point at the new owners. Platform and vendor constraints were retained in the
platform guide and linked from the agent reference.

Validation on the macOS development host: all 348 baseline tests passed, with no
compiler warnings; formatter, diff whitespace check, documentation-link validation
and offline release build passed. Independent read-only review found no behavior
regressions. Its capability-ownership and stale-documentation findings were
corrected before delivery. Supported-platform/native checks retain the limitations
already listed above.

## Test directory follow-up

All first-party unit suites and test-only fixtures now live under `tests/unit/`,
grouped by their owning capability. The existing integration targets remain in
`tests/*.rs`. Production modules retain only test-gated path registrations; no
test bodies or fixtures remain in `src/`. This supersedes the earlier physical
test locations recorded above.

The relocation preserves private access, platform conditions, logical Rust module
names, and existing Cargo filters. Test-only imports moved with their suites.
[`tests/README.md`](../tests/README.md) explains the layout, and the architecture
guides and agent routing table point to the new paths.

Validation on the macOS development host: all 299 relocated test functions were
accounted for, and all 348 executed tests passed with exactly the same names as
the preceding baseline and no compiler warnings. Formatter, diff whitespace
check, documentation-link validation, and offline release build passed.
Windows-only tests and supported-platform packaging were not run on this host.

# Core comparison

`src/core` contains UI-independent operations that can be exercised without starting eframe. The public exports in [`src/core/mod.rs`](../../src/core/mod.rs) and [`src/lib.rs`](../../src/lib.rs) define the library surface used by the desktop app and CLI.

## File comparison

[`src/core/file_view.rs`](../../src/core/file_view.rs) is the public orchestration and display model. Its `input.rs` loads bounded text inputs, `text.rs` prepares comparable lines and highlight ranges, and `alignment.rs` aligns changed blocks. The result has original line numbers and inline ranges. It owns comparison output, not egui row painting. The app consumes the result to draw aligned panes and apply viewport/navigation behavior.

When changing line matching, preserve exact anchors where possible and keep the bounded fallback behavior visible in tests. A useful change should state which alignment invariant it affects and exercise that invariant at the core interface.

## Folder comparison and navigation data

[`src/core/directory.rs`](../../src/core/directory.rs) compares folder entries and computes per-entry states. [`src/core/tree.rs`](../../src/core/tree.rs) builds aligned tree structures, aggregates descendant status, and supports changed-file traversal/reveal. [`src/core/lazy_directory.rs`](../../src/core/lazy_directory.rs) lists one directory level with metadata and validates root/relative paths for lazy browsing.

Folder Diff may need a complete comparison snapshot; Folder Merge navigation requests shallow listings and loads content on demand. Keep these use cases distinct. UI code should consume states and trees rather than reimplementing filesystem traversal or status aggregation.

## Editable and save primitives

[`src/core/interactive.rs`](../../src/core/interactive.rs) provides the two-way editable workspace, document operations, transfers, and undo/redo. [`src/core/chunk_detection.rs`](../../src/core/chunk_detection.rs) supplies automatic chunk detection. [`src/core/work_files.rs`](../../src/core/work_files.rs) loads and saves text working copies while retaining the information needed to detect external edits and preserve supported encodings.

A core operation should accept explicit input and return a result or transaction. Avoid passing UI contexts or relying on hidden global state; that keeps the interface a useful test surface for its semantics.

## Search and CLI three-way merge

[`src/core/search_replace.rs`](../../src/core/search_replace.rs) performs recursive text matching and produces replacement-capable hit data. It does not directly persist replacements. The app applies validated edits to its retained working copies; see [Editing, search, and save](editing-search-save.md).

[`src/core/merge.rs`](../../src/core/merge.rs) computes conventional base/ours/theirs merge hunks and conflict markers. The CLI route in [`src/main.rs`](../../src/main.rs) invokes it and writes the requested output. This path is separate from the desktop two-way Merge workspace.

## Error and test ownership

Core errors and shared result types are defined in the relevant modules and [`src/core/error.rs`](../../src/core/error.rs). Prefer returning typed errors at the core seam and translating them to user-facing text in the app or CLI. Module-local tests live beside the implementation under `src/core/<module>/tests.rs` or its test module; consult the module declaration before creating a new test file.

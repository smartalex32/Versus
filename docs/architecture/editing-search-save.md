# Editing, search, and save

This guide follows text from a working copy through edits and replacement transactions to an explicit disk save. See the [SRS](../SRS.md) for supported formats and user-visible guarantees.

## Working copies and document identity

[`src/core/work_files.rs`](../../src/core/work_files.rs) owns text working-file loading and safe persistence metadata, including original bytes used to detect external changes. [`src/core/interactive.rs`](../../src/core/interactive.rs) owns editable document/workspace operations and transactional undo/redo. These core modules have no egui dependency.

[`src/app/interactive_view.rs`](../../src/app/interactive_view.rs) coordinates the active pair and retains documents across navigation. Its `document.rs` and `history.rs` child modules are the first stops for document lifetime, aliases, canonical identity, and history behavior. Editor projection is display state; it must not silently replace canonical document contents.

## Editing and transfers

Editor events are handled through the app's interactive view, which applies operations to the core workspace. Inline transfers and accept-all operations should remain undoable workspace transactions. When changing editor presentation, check whether projection/remapping code translates the visible text position back to canonical content and cursor positions; paint-only comparison decoration should not become document data.

## Search and replacement

Search is split across three owners:

- [`src/core/search_replace.rs`](../../src/core/search_replace.rs) walks files, matches text, and produces search results without writing them.
- [`src/app/folder_browser/search_state.rs`](../../src/app/folder_browser/search_state.rs) owns query/options, debounce, cancellation, streaming results, scope, and included/excluded hits.
- [`src/app/search_session.rs`](../../src/app/search_session.rs) prepares replacement plans and remaps the remaining original hit snapshots. `InteractiveView` in `interactive_view/history.rs` validates and atomically publishes replacement batches to retained working copies.

The folder browser's [`replacement.rs`](../../src/app/folder_browser/replacement.rs) connects search UI intent to plan preparation; [`search_modal.rs`](../../src/app/folder_browser/search_modal.rs) owns modal rendering. Keep matching independent from UI, and keep application of results behind the `InteractiveView` transaction path. Partial, cancelled, or stale result sets must never authorize edits.

Batch replacement coordinates several documents. Stage and validate each participant before publishing the batch so an invalid identity or byte range cannot leave only some files changed. Undo order and newer edits in participating files are part of the transaction contract.

To trace a replacement through the code:

1. `FolderBrowser::replace_matches` in `replacement.rs` asks `replacement_plan`
   to select current, included hits in the chosen scope.
2. `search_session::preview_replacements` delegates text-edit preparation to
   `core::preview_replace`; the resulting plan contains original snapshots and
   exact byte edits, not disk writes.
3. `InteractiveView::apply_search_preview` in `history.rs` stages retained
   document clones, reconciles refreshed clean snapshots, validates every
   participant and publishes one tagged replacement batch.
4. `SearchState::advance_after_replacement` invokes
   `search_session::advance_results` to remap original hits. Replacement text is
   not searched again. Undo invalidates the old hit snapshots before a new search.
5. A later explicit save goes through `editing_actions.rs` or folder `save.rs`;
   `core::save_working_file` performs the byte-fingerprint check and encoded write.

## Saving

A change to a working copy is not a disk write. Explicit save actions are coordinated in `InteractiveView`; folder-wide saves are coordinated through `FolderBrowser`'s `save.rs`. Core writing and external-change protection belong to [`src/core/work_files.rs`](../../src/core/work_files.rs) and its safe-save helpers.

When changing save behavior, preserve the distinction between the path used to write and the identity used to recognize a document. Verify that failed or stale saves leave the working copy dirty, and that successful saves update the baseline from the content actually written. File-format limits and encoding rules live in the SRS and core module documentation.

## Where to change common behaviors

| Change | Start here |
| --- | --- |
| Add a matching option | core search matcher, then `SearchState` option lifecycle and its tests |
| Change result streaming or cancellation | `SearchState` and `core/search_replace.rs` |
| Change Replace Next/All semantics | `search_session.rs` for plan/snapshot handling and `interactive_view/history.rs` for atomic publication |
| Change editor undo/redo or transfer behavior | `core/interactive.rs`, then `interactive_view` event handling |
| Change save conflict detection or encoding | `core/work_files.rs` and its tests |
| Change folder Save Both collection | `folder_browser/save.rs` |

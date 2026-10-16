# Desktop rendering and sessions

The desktop is built with eframe/egui. This guide describes the rendering and state ownership map; exact control behavior is specified in the [SRS](../SRS.md).

## Main composition

[`src/main.rs`](../../src/main.rs) configures the native window and constructs `VersusApp`. The main app module [`src/app.rs`](../../src/app.rs) owns window-level lifecycle/state and composes capabilities. [`workspace.rs`](../../src/app/workspace.rs) lays out the main workspace; [`source_controls.rs`](../../src/app/source_controls.rs) renders comparison/workspace mode and source controls; [`file_diff.rs`](../../src/app/file_diff.rs) and [`folder_diff.rs`](../../src/app/folder_diff.rs) render read-only comparison views. Recent rows, drop targets, leave confirmation, and status are owned by `recent_view.rs`, `drop_targets.rs`, `leave_dialog.rs`, and `workspace_status.rs`. Shared visual code is indexed by [`visuals.rs`](../../src/app/visuals.rs), with theme, panels, status, footer, and icons in its child modules.

`Session` in [`src/app/session.rs`](../../src/app/session.rs) owns shared source/mode state. Its `source.rs` handles classification and source lifecycle, `navigation.rs` handles display-row anchors/filtering, refresh and difference navigation, `comparison.rs` owns file/folder workers, result counts and publication, `merge_workspace.rs` coordinates retained Merge work, and `history.rs` owns recent-pair persistence. Inspect the child module matching the transition before changing the parent state type. Session does not own editor history or the folder search transaction.

## Read-only Diff

Core file comparison produces `FileComparison` and aligned rows through [`src/core/file_view.rs`](../../src/core/file_view.rs), which delegates decoding to `input.rs`, line matching to `alignment.rs`, and comparison text preparation/highlights to `text.rs`. Recursive directory comparison originates in [`src/core/directory.rs`](../../src/core/directory.rs); aligned tree construction/status comes from [`src/core/tree.rs`](../../src/core/tree.rs). [`file_diff.rs`](../../src/app/file_diff.rs) and [`folder_diff.rs`](../../src/app/folder_diff.rs) paint those results and map input events to filtering, navigation, and tree operations. Keep comparison semantics in `src/core`; keep egui layout and hit testing in the owning app module.

## Editable Merge

[`src/app/interactive_view.rs`](../../src/app/interactive_view.rs) is the stable owner and coordination point for the two-sided workspace. Its children divide presentation responsibilities: `document.rs` for retained documents and lifecycle; `history.rs` for undo/redo, replacement batches, and pending decorations; `editor.rs` for projected editor painting and input; `diff.rs` and `readonly.rs` for optional comparison presentation; `projection.rs` for display-only projections; `editing_actions.rs` for save, transfer, cursor, and shortcut actions; `toolbar.rs` for toolbar/footer orchestration; `line_rendering.rs` for arrow and code-line painting.

The core workspace and editing primitives are in [`src/core/interactive.rs`](../../src/core/interactive.rs). The app keeps navigation, egui interaction, and async job coordination around those UI-independent primitives. Editable content and history survive workspace-mode changes according to the session behavior documented in the SRS.

## Folder Merge

[`src/app/folder_browser.rs`](../../src/app/folder_browser.rs) exposes the stable folder-browser interface and coordinates frame work. Child modules separate `navigation.rs`, `overlay.rs`, `search_modal.rs`, `search_state.rs`, `replacement.rs`, and `save.rs`. Navigation owns per-side tree state and lazy listings; overlays reconcile current working copies with cached folder comparison status; the search modules own query and result interaction; replacement and save modules coordinate applied edits and persistence.

When a visible folder status looks wrong, trace from the visible row to the app snapshot and then to the comparison or overlay producer. Avoid deriving a second folder status independently in rendering code.

## Choosing an owner

- A new egui response, paint operation, or layout belongs with the renderer that draws that surface.
- A source or mode transition belongs with `Session`.
- An edit or document identity transition belongs with `InteractiveView` and its document/history modules.
- Per-side folder expansion and path navigation belong with `FolderBrowser` navigation.
- A pure comparison or text operation belongs in `src/core`.

Tests under these modules are the fastest way to learn the expected interaction and state transitions before changing them.

## Why the parent types remain

`VersusApp`, `Session`, `InteractiveView` and `FolderBrowser` each retain one state
owner. Their child files contain capability-specific inherent implementations on
that owner. For example, editor painting and document retention need the same
document identity and cursor state, but they have different reasons to change.
Separating their implementations avoids passing that state through new forwarding
interfaces. Parent files show the state shape, module index and orchestration;
child files show the operation being changed.

Owner-internal imports share that private namespace. Cross-owner visual imports
name their defining `visuals` module explicitly. File row anchors/filtering belong
to session navigation; result counts belong to session comparison. Renderers
consume those models rather than owning their lifecycle rules.

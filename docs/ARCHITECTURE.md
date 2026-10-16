# Architecture guide

Versus is a Rust desktop application for comparing local files and folders. The desktop application presents read-only Diff and editable Merge workspaces; the `versus --merge` command is a separate three-way text merge. The app is offline and keeps edited content in memory until an explicit save.

This page is the newcomer route. The linked guides answer focused questions without requiring a tour through the full product specification.

## Start here

1. Read the [README](../README.md) for purpose, setup, and the normal user workflow. Read the [glossary](../GLOSSARY.md) for project terms.
2. Start at [`src/main.rs`](../src/main.rs): command dispatch, native window setup, and CLI merge execution.
3. Follow [`src/app.rs`](../src/app.rs) for the desktop lifecycle and composition, then enter the relevant capability under `src/app/`.
4. Read the guide matching the change:
   - [Data flow and lifecycle](architecture/dataflow.md) for sources, asynchronous work, and published results.
   - [Desktop rendering and sessions](architecture/desktop.md) for app shell, session transitions, Diff, and Merge presentation.
   - [Editing, search, and save](architecture/editing-search-save.md) for working copies and transactional changes.
   - [Core comparison](architecture/core-comparison.md) for filesystem reads, line alignment, folder trees, and the CLI merge engine.
   - [Development and tests](architecture/development.md) for test ownership, validation, and safe contribution seams.
   - [Platform and packaging](architecture/platform.md) for vendoring, native input, release baselines, and runtime constraints.
5. Agents should also read [`AGENTS.md`](../AGENTS.md) and [`AGENTS_PROJECT_REFERENCE.md`](../AGENTS_PROJECT_REFERENCE.md) for repository instructions and validation commands. Use [`docs/SRS.md`](SRS.md) for externally visible requirements and [`docs/APP_AUDIT.md`](APP_AUDIT.md) for identified follow-up opportunities. This guide explains code ownership and navigation; it does not duplicate the full behavior specification.

## Repository map

| Area | Responsibility |
| --- | --- |
| [`src/main.rs`](../src/main.rs), [`src/cli.rs`](../src/cli.rs) | Process entry, argument parsing, GUI launch, and standalone CLI merge. |
| [`src/app.rs`](../src/app.rs) | Window lifecycle/state and capability composition. |
| [`src/app/workspace.rs`](../src/app/workspace.rs), [`source_controls.rs`](../src/app/source_controls.rs) | Main workspace frame and source/mode controls. |
| [`file_diff.rs`](../src/app/file_diff.rs), [`folder_diff.rs`](../src/app/folder_diff.rs) | Read-only file and folder comparison painting/input. |
| [`recent_view.rs`](../src/app/recent_view.rs), [`drop_targets.rs`](../src/app/drop_targets.rs), [`leave_dialog.rs`](../src/app/leave_dialog.rs), [`workspace_status.rs`](../src/app/workspace_status.rs) | Recent rows, file-drop targets, leave confirmation, and workspace status. |
| [`visuals.rs`](../src/app/visuals.rs) | Shared visual module index; child modules own theme, panels, status, footer, and icons. |
| [`src/app/session.rs`](../src/app/session.rs) | Shared session state and app-facing coordination; child modules separate source lifecycle, navigation, comparison jobs, merge workspace, and recent history. |
| [`src/app/interactive_view.rs`](../src/app/interactive_view.rs) | Retained two-sided editing workspace and its document, history, editor, projection, diff, readonly, editing-actions, line-rendering, and toolbar capabilities. |
| [`src/app/folder_browser.rs`](../src/app/folder_browser.rs) | Folder Merge navigation and coordination; its submodules divide navigation, overlays, search, replacement, and saves. |
| [`src/app/search_session.rs`](../src/app/search_session.rs) | Prepares replacement plans and advances remaining search-hit snapshots; batch publication is owned by `InteractiveView`. |
| [`src/core/`](../src/core/) | UI-independent comparison, editing, filesystem, search, and safe-save primitives; `file_view/` splits input decoding, alignment, and text projection. |
| [`src/selection.rs`](../src/selection.rs), [`src/recent.rs`](../src/recent.rs), [`src/diagnostics.rs`](../src/diagnostics.rs) | Source classification, recent pairs, and local diagnostics. |
| [`src/app/tests.rs`](../src/app/tests.rs), [`src/app/tests/`](../src/app/tests/) | App test index and focused suites for accessibility, navigation, Diff, Merge, session lifecycle, theme, chrome, and zoom. |
| [`src/app/interactive_view/tests/`](../src/app/interactive_view/tests/), `src/app/folder_browser/tests/` | Focused editor/workspace and folder-browser tests. |
| [`tests/`](../tests/) and `src/core/**/tests.rs` | Integration and core module-level behavioral tests. |

## Find a change by capability

| If you are changing… | Begin with… |
| --- | --- |
| startup, CLI arguments, app lifetime | [`src/main.rs`](../src/main.rs), [`src/cli.rs`](../src/cli.rs) |
| choosing or classifying paths | [`src/selection.rs`](../src/selection.rs), then [`source.rs`](../src/app/session/source.rs) or [`source_controls.rs`](../src/app/source_controls.rs) |
| main workspace composition or source controls | [`workspace.rs`](../src/app/workspace.rs), [`source_controls.rs`](../src/app/source_controls.rs) |
| File Diff inputs, line matching, or text ranges | [`src/core/file_view/input.rs`](../src/core/file_view/input.rs), [`alignment.rs`](../src/core/file_view/alignment.rs), or [`text.rs`](../src/core/file_view/text.rs), then [`src/app/file_diff.rs`](../src/app/file_diff.rs) |
| Folder Diff tree/status | [`src/core/directory.rs`](../src/core/directory.rs), [`src/core/tree.rs`](../src/core/tree.rs), then [`src/app/folder_diff.rs`](../src/app/folder_diff.rs) |
| editable documents, transfers, or pair history | [`src/core/interactive.rs`](../src/core/interactive.rs), then [`src/app/interactive_view.rs`](../src/app/interactive_view.rs) and its child module |
| source lifecycle or comparison job publication | [`src/app/session/source.rs`](../src/app/session/source.rs) or [`comparison.rs`](../src/app/session/comparison.rs) |
| folder merge tree navigation | [`src/app/folder_browser/navigation.rs`](../src/app/folder_browser/navigation.rs) and [`src/core/lazy_directory.rs`](../src/core/lazy_directory.rs) |
| Find and Replace | [`src/core/search_replace.rs`](../src/core/search_replace.rs), [`src/app/folder_browser/search_state.rs`](../src/app/folder_browser/search_state.rs), [`src/app/search_session.rs`](../src/app/search_session.rs) for plans, then [`history.rs`](../src/app/interactive_view/history.rs) for atomic batch publication |
| explicit saving and external-change checks | [`src/core/work_files.rs`](../src/core/work_files.rs), then the app save coordinator |
| recent sources or local logs | [`src/recent.rs`](../src/recent.rs), [`src/diagnostics.rs`](../src/diagnostics.rs) |

## Ownership rules

Keep filesystem and text algorithms in `src/core`; app modules should coordinate work and present state. A background worker computes a candidate result, while the owning app state decides whether that result still belongs to the active request before publishing it. Edits update in-memory working copies; disk writes happen only through explicit save paths. These distinctions are central to cancellation, stale-result handling, and data integrity.

The app-facing structs are coordination points. Put behavior in the capability module listed above and keep changes narrow across module seams. If a change crosses app and core, make the core operation usable and testable without egui, then connect it through the owning app state.

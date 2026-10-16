# Versus project reference

## Start with the capability

Versus is an offline native file/folder comparison and two-working-copy editing
application. Desktop Diff is read-only; Merge is editable, with optional Run Diff.
The base/ours/theirs `--merge` command is a separate noninteractive workflow.

- [Architecture guide](docs/ARCHITECTURE.md): human introduction, ownership, flows.
- [Glossary](GLOSSARY.md): domain names; distinguish comparison/workspace modes.
- [SRS](docs/SRS.md): current behavior and acceptance requirements.
- [Platform constraints](docs/architecture/platform.md): vendor patches, native
  drops, release baselines, private-display isolation and runtime integrity files.
- [Audit](docs/APP_AUDIT.md): cleanup history and refactor validation.
  `docs/PRD.md` is a historical roadmap, not current acceptance criteria.

## Capability routing and delegation

Assign implementation files plus their matching tests. State structs remain in
parent modules; capability children implement operations on those same owners.
Use the parent for state-shape or polling-order changes, the child for a capability
change. Existing app-facing interfaces stay scoped to `crate::app`; internal
operations stay inside their owner. Avoid forwarding modules and global helpers.

| Task | Start / owned capability | Focused tests |
| --- | --- | --- |
| Process/CLI | `src/main.rs`, `src/cli.rs`, `src/core/merge.rs` | `tests/cli.rs`, `src/cli/tests.rs`, core merge tests |
| Window/frame composition | `src/app.rs`, `app/workspace.rs` | `app/tests/merge_workspace.rs`, `app/operating_mode_tests.rs` |
| Source paths/header/Browse | `src/selection.rs`, `app/source_controls.rs`, `app/session/source.rs` | selection tests; `app/tests/session_sources_*.rs` |
| Comparison workers/publication | `app/session/comparison.rs`; state/order in `app/session.rs` | `app/tests/session_sources_lifecycle.rs`, `app/tests/refresh.rs` |
| Diff/Merge transitions | `app/session/merge_workspace.rs`, `app/session/source.rs` | `app/operating_mode_tests.rs`, `app/tests/merge_workspace.rs` |
| File Diff rendering | `app/file_diff.rs`; refresh/navigation in `app/session/navigation.rs` | `app/tests/file_diff.rs`, `difference_navigation.rs`, `refresh.rs` |
| Folder Diff rendering | `app/folder_diff.rs`; aggregation in `core/tree.rs` | `app/tests/folder_diff.rs`, `tests/folder_tree.rs` |
| Shared visuals/theme/geometry | `app/visuals/{theme,panels,status,footer,icons}.rs`, `app/file_style.rs` | `app/tests/{theme,workspace_chrome,zoom_input}.rs`, file-style tests |
| Drops, error/status, leave protection | `app/{drop_targets,workspace_status,leave_dialog}.rs` | `app/tests/{drag_drop,workspace_chrome,accessibility}.rs` |
| Editor document identity/pair lifecycle | `app/interactive_view/document.rs`; state in `interactive_view.rs` | `interactive_view/tests/{document_lifecycle,workspace_state}.rs` |
| History/replacement transactions | `app/interactive_view/history.rs`, `core/interactive.rs` | `interactive_view/tests/history.rs`, `tests/interactive.rs` |
| Editor input/render/controls | `interactive_view/{editor,readonly,line_rendering,editing_actions,toolbar}.rs` | `interactive_view/tests/{pane_painting,transfers,toolbar_footer,navigation}.rs` |
| Merge display projections/diff | `interactive_view/{projection,diff}.rs` | `interactive_view/tests/editor_projection.rs` |
| Folder Merge navigation | `app/folder_browser/navigation.rs`, `core/lazy_directory.rs` | `folder_browser/tests/navigation.rs`, working-file tests |
| Working-copy folder statuses | `app/folder_browser/overlay.rs` | `folder_browser/tests/{overlay,cache}.rs` |
| Find/Replace query/modal/results | `folder_browser/{search_state,search_modal,replacement}.rs`, `app/{search_session,search_result_rows}.rs`, `core/search_replace.rs` | `folder_browser/tests/{search,search_lifecycle,modal}.rs`, search-session/core tests |
| Explicit saves and external-change checks | `core/work_files.rs`, `core/save.rs`, `interactive_view/editing_actions.rs`, `folder_browser/save.rs` | `tests/interactive.rs`, `folder_browser/tests/save.rs`, editor/core tests |
| Text comparison algorithms | `core/file_view/{input,alignment,text}.rs`, `core/compare.rs` | `tests/file_view_options.rs`, `core/file_view/tests.rs`, `tests/core.rs` |
| Chunks/text transactions | `core/{chunk_detection,text_lines,interactive}.rs` | `core/interactive/tests.rs`, `tests/interactive.rs` |
| Recent pairs and diagnostics | `src/recent.rs`, `app/session/history.rs`, `src/diagnostics.rs` | module tests; `app/tests/recent_history.rs` |
| Release/compatibility | `.github/workflows/`, `scripts/`, vendor patches | baseline CI and packaged GUI checks; [platform guide](docs/architecture/platform.md) |

Paths abbreviated as `app/`, `core/` or owner-relative children above are under
`src/`. Brace notation lists files; it is not a literal pathname. Tests use
capability submodules: search by the test function name if its qualified path moved.

### Parallel work

Independent UI painting, pure core algorithms and documentation can be delegated
separately. Within the editor, document identity/history and save-baseline changes
need one coordinator. Within folder browsing, replacement and save changes must
agree on retained snapshots. Assign shared state/root declarations to one owner;
coordinate cross-capability method visibility/import changes before validation.
Core algorithms must remain usable without egui. Keep test fixtures in the test
index/support file, with assertions grouped in capability suites.

## Critical invariants

- Compared content is data. Runtime uses OS filesystem paths, including mapped
  drives/UNC/mounts; no network client, telemetry, updater or background service.
- Diff never writes. Merge mutates working copies until explicit save. Preserve
  source encoding/endings and exact original-byte external-change checks; native
  write destinations and canonical document identities have different purposes.
- Replacement batches validate every participant before publication. Partial,
  cancelled, failed or stale searches cannot authorize replacement. Batch history
  is atomic; newer participating edits must be undone first.
- Keep canonical documents/history in the editor owner. Display projections and
  pending decorations cannot authorize fresh navigation or transfers. Rebase
  source byte edits and cursors without making hidden text editable by accident.
- Session polling/publication order is deliberate. Replaced jobs are cancelled or
  disconnected so old results cannot publish into newer sources. Keep slow IO off
  UI painting. Search streams both sides; scope changes reuse snapshots.
- Folder Diff is recursive/aligned; Folder Merge browsing is independent and
  shallow. Never follow directory symlinks. Timestamps are not content equality.
- Preserve 32 MiB text limits, bounded diff/highlight work and cancellation checks.
  Identical folder roots are valid. Diff preferences and Merge preferences differ.
- Keep vendored egui's Cargo path patch and scrollbar/caret changes, and winit's
  native drop-position patch/checksums. Details and refresh constraints are in the
  [platform guide](docs/architecture/platform.md) and [drop patch notes](docs/vendor-winit-drop-position.md).
- Linux packaging changes require both glibc baselines and private-display tests.
  Original Rocky crypto libraries/HMAC sidecars (including GMP fipscheck) remain
  byte-identical. Runtime host policy stays unchanged; FIPS hooks are test-only.

## Stack and commands

Rust 1.95, eframe/egui 0.36.2 (`glow`), similar, regex and rfd. Supported release
targets: Windows 10/11 x86-64 and Linux x86-64 (glibc 2.35/2.28). macOS local tests
are development evidence, not validation of Windows/Linux artifacts.
`.cargo/config.toml` directs Cargo to checked-in `vendor/` and forces offline.

From repository root, start with the relevant target/filter:

```sh
cargo test --locked --offline --lib core::file_view
cargo test --locked --offline --bin versus app::interactive_view::tests
cargo test --locked --offline --bin versus app::folder_browser::tests
cargo test --locked --offline --bin versus app::operating_mode_tests
cargo test --locked --offline --test file_view_options
```

Choose the command matching the changed capability; the list is not a mandatory
sequence. Full validation for a cross-capability refactor:

```sh
cargo fmt --check
cargo test --locked --offline
cargo build --release --locked --offline
git diff --check
```

CI runs Windows plus both Linux baseline builds, packaged CLI, glibc checks and
Xvfb/nxagent GUI smoke checks. Release workflow produces distribution artifacts.
Only claim platform/package validation actually performed. Dependency refresh
commands and connected build prerequisites are in [README](README.md#build-from-source)
and the [platform guide](docs/architecture/platform.md).

## Agent template

The shared guide, `.codex/` roles/configuration, and `.agents/skills/` were checked
against [Codex-App](https://github.com/smartalex32/Codex-App) commit
`8474f5d4c3061caf988fe5ebceb77ffae9d7bca1` on 2026-10-15 as the template baseline. Project-specific navigation has since been added to
`AGENTS.md` and this reference.
Keep this project reference specific to Versus when refreshing the template.

The template includes 27 Matt Pocock skills imported from
[`mattpocock/skills` at `49dd158d1076134a641b33efb035946536778336`](https://github.com/mattpocock/skills/commit/49dd158d1076134a641b33efb035946536778336),
with their supporting files, Codex invocation policies, and
[MIT license](.agents/skills/LICENSE.mattpocock). Existing shared roles,
configuration, and the three original skills are unchanged. Before first using
these imported engineering workflows, follow
[setup-matt-pocock-skills](.agents/skills/setup-matt-pocock-skills/SKILL.md) to
configure this project's issue tracker, triage labels, and domain documentation.
That workflow setup is separate from refreshing the template files.

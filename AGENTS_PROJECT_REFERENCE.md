# Versus project reference

## Start with the capability

Versus is an offline native file/folder comparison and two-working-copy editing
application. Normal desktop Diff is read-only; Merge is editable, with optional
Run Diff. Focused Git tools use separate launch state. The base/ours/theirs
`--merge` command is a separate noninteractive workflow.

- [Architecture guide](docs/ARCHITECTURE.md): human introduction, ownership, flows.
- [Git tool guide](docs/architecture/git-tools.md): Git difftool/mergetool setup, process protocol, focused views, and code/test routing.
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
| Process/CLI | `src/main.rs`, `src/cli.rs`, `src/core/merge.rs` | `tests/cli.rs`, `tests/unit/cli/`, `tests/unit/core/merge/` |
| Git Diff/Merge external tools | `src/cli.rs`, `src/git_launch.rs`, `src/app/git.rs`; merge algorithm in `src/core/git_merge.rs` | `tests/unit/cli/`, `tests/unit/app/git/`, `tests/unit/core/git_merge/`; [Git tool guide](docs/architecture/git-tools.md) |
| Window/frame composition | `src/app.rs`, `src/app/workspace.rs` | `tests/unit/app/`, `tests/unit/app/operating_modes.rs` |
| Source paths/header/Browse | `src/selection.rs`, `src/app/source_controls.rs`, `src/app/session/source.rs` | `tests/unit/selection/`, `tests/unit/app/`, `tests/unit/app/session_sources_interactions.rs` |
| Comparison workers/publication | `src/app/session/comparison.rs`; state/order in `src/app/session.rs` | `tests/unit/app/session_sources_lifecycle.rs`, `tests/unit/app/session_sources_interactions.rs`, `tests/unit/app/refresh.rs` |
| Diff/Merge transitions | `src/app/session/merge_workspace.rs`, `src/app/session/source.rs` | `tests/unit/app/operating_modes.rs`, `tests/unit/app/merge_workspace.rs` |
| File Diff rendering | `src/app/file_diff.rs`; refresh/navigation in `src/app/session/navigation.rs` | `tests/unit/app/file_diff.rs`, `tests/unit/app/difference_navigation.rs`, `tests/unit/app/refresh.rs` |
| Folder Diff rendering | `src/app/folder_diff.rs`; aggregation in `src/core/tree.rs` | `tests/unit/app/folder_diff.rs`, `tests/folder_tree.rs` |
| Shared visuals/theme/geometry | `src/app/visuals/{theme,panels,status,footer,icons}.rs`, `src/app/file_style.rs` | `tests/unit/app/theme.rs`, `workspace_chrome.rs`, `zoom_input.rs`, `tests/unit/app/file_style/`, `tests/unit/app/find_icons/` |
| Drops, error/status, leave protection | `src/app/{drop_targets,workspace_status,leave_dialog}.rs` | `tests/unit/app/{drag_drop,accessibility,workspace_chrome}.rs`, `tests/unit/native_drop/` |
| Editor document identity/pair lifecycle | `src/app/interactive_view/document.rs`; state in `src/app/interactive_view.rs` | `tests/unit/app/interactive_view/{document_lifecycle,workspace_state}.rs` |
| History/replacement transactions | `src/app/interactive_view/history.rs`, `src/core/interactive.rs` | `tests/unit/app/interactive_view/history.rs`, `tests/unit/core/interactive/`, `tests/interactive.rs` |
| Editor input/render/controls | `src/app/interactive_view/{editor,readonly,line_rendering,editing_actions,toolbar}.rs` | `tests/unit/app/interactive_view/{pane_painting,transfers,toolbar_footer,navigation}.rs` |
| Merge display projections/diff | `src/app/interactive_view/{projection,diff}.rs` | `tests/unit/app/interactive_view/{editor_projection,projection_support}.rs` |
| Folder Merge navigation | `src/app/folder_browser/navigation.rs`, `src/core/lazy_directory.rs` | `tests/unit/app/folder_browser/navigation.rs`, `tests/unit/core/work_files/` |
| Working-copy folder statuses | `src/app/folder_browser/overlay.rs` | `tests/unit/app/folder_browser/{overlay,cache}.rs` |
| Find/Replace query/modal/results | `src/app/folder_browser/{search_state,search_modal,replacement}.rs`, `src/app/{search_session,search_result_rows}.rs`, `src/core/search_replace.rs` | `tests/unit/app/folder_browser/{search,search_lifecycle,modal}.rs`, `tests/unit/app/search_session/`, `tests/unit/app/search_result_rows/`, `tests/unit/core/search_replace/` |
| Explicit saves and external-change checks | `src/core/work_files.rs`, `src/core/save.rs`, `src/app/interactive_view/editing_actions.rs`, `src/app/folder_browser/save.rs` | `tests/interactive.rs`, `tests/unit/app/folder_browser/save.rs`, `tests/unit/core/work_files/` |
| Text comparison algorithms | `src/core/file_view/{input,alignment,text}.rs`, `src/core/compare.rs` | `tests/file_view_options.rs`, `tests/unit/core/file_view/`, `tests/core.rs` |
| Chunks/text transactions | `src/core/{chunk_detection,text_lines,interactive}.rs` | `tests/unit/core/interactive/`, `tests/interactive.rs` |
| Recent pairs and diagnostics | `src/recent.rs`, `src/app/session/history.rs`, `src/diagnostics.rs` | `tests/unit/recent/`, `tests/unit/app/recent_history.rs`, `tests/unit/diagnostics/` |
| Release/compatibility | `.github/workflows/`, `scripts/`, vendor patches | baseline CI and packaged GUI checks; [platform guide](docs/architecture/platform.md) |

Production paths are rooted at `src/`. Unit tests and their test-only support live
under `tests/unit/`; integration test targets are individual `tests/*.rs` files.
Brace notation lists paths; it is not a literal pathname. Unit suites retain their
logical Rust module names, so existing Cargo filters still work; their physical
file paths do not appear in those filters. Search by test function name when a
suite's physical path has moved.

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
cargo test --locked --offline --bin versus app::git
cargo test --locked --offline --lib core::git_merge::tests
cargo test --locked --offline --bin versus cli::tests
cargo test --locked --offline --test git_merge
cargo test --locked --offline --test git_protocol
```

Unit suite files live under `tests/unit/`, but are registered into their owning
`lib` or `versus` binary module. The commands above intentionally keep the same
module filters; only integration tests use the `--test <target>` form. See
[`tests/README.md`](tests/README.md) for the physical layout and test-only support
conventions.

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

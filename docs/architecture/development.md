# Development and tests

Use the repository [agent guide](../../AGENTS.md) and [project reference](../../AGENTS_PROJECT_REFERENCE.md) for current toolchain, validation commands, and platform coverage. This guide explains how to orient changes and locate the behavioral checks.

## Test layout

Rust unit tests are kept out of implementation bodies and grouped by capability. App behavior suites are indexed under [`src/app/tests/`](../../src/app/tests/); interactive editor suites live under [`src/app/interactive_view/tests/`](../../src/app/interactive_view/tests/); folder browser suites live under `src/app/folder_browser/tests/`; core module tests live under `src/core/<capability>/tests.rs`. Integration tests live under [`tests/`](../../tests/). Search for the module's test declarations and nearby test names before adding coverage; follow the established sibling layout.

Tests should exercise public or app-facing behavior at the narrowest useful seam. Core comparison and save rules belong with core module tests. Session transitions belong with `Session` tests. Editor behavior belongs in `interactive_view/tests/`. Folder interaction and transactional replacement belong with their owning capability tests; app-level workflows that cross modules belong in the matching `src/app/tests/` suite. Avoid testing the same implementation detail at several layers unless each layer protects a separate contract.

## Safe contribution chunks

A low-risk change usually has one clear owner. For example, a new line-alignment rule can be implemented and tested in `core/file_view`; an editor control can be handled in one `interactive_view` capability module; a new folder-tree interaction can stay within `FolderBrowser` navigation and its tests. Cross-layer changes should keep the core operation independently testable, then connect it through the app-facing owner.

Before editing, inspect the active declarations and the nearest tests. Refactoring may move responsibilities while preserving stable parent interfaces; source links here are navigation aids, so confirm current ownership in the module itself. Keep unrelated UI behavior and user changes intact.

## Validation

Install Rust 1.95 and the host build libraries described in the
[build instructions](../../README.md#build-from-source). Cargo uses the checked-in
vendor tree; ordinary compilation and tests do not require dependency downloads.
Run commands from the repository root.

| Changed capability | Focused command |
| --- | --- |
| Input, line alignment, highlights | `cargo test --locked --offline --lib core::file_view` and `cargo test --locked --offline --test file_view_options` |
| Editing/history/projection | `cargo test --locked --offline --bin versus app::interactive_view::tests` |
| Folder navigation, overlays, search, saves | `cargo test --locked --offline --bin versus app::folder_browser::tests` |
| Cross-workspace transitions | `cargo test --locked --offline --bin versus app::operating_mode_tests` |
| CLI arguments/output/exit codes | `cargo test --locked --offline --test cli` |

For a change spanning capability owners, run:

```sh
cargo fmt --check
cargo test --locked --offline
cargo build --release --locked --offline
git diff --check
```

Report commands that could not run and platform coverage that remains unverified.
Do not infer Windows/Linux behavior from a macOS-only run. Native dialogs,
accessibility, mounted shares and packaged graphics require their target systems;
the [platform guide](platform.md) identifies the owning CI checks.

## Platform-specific paths

Conditional code is visible in [`src/main.rs`](../../src/main.rs), [`src/selection.rs`](../../src/selection.rs), and native integration modules such as [`src/native_drop.rs`](../../src/native_drop.rs). Keep platform-specific assumptions behind the existing conditional modules and validate on the relevant target where possible. The project reference is authoritative for supported release targets and current local validation availability.

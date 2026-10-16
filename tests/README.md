# Test layout

Versus keeps all first-party tests and test-only fixtures outside `src/`. This
keeps production modules readable while retaining the original Rust unit-test
ownership: unit suites are compiled into the `lib` or `versus` binary target and
registered under the owning module. Private implementation details remain
testable without widening production visibility, and existing Cargo module
filters continue to work.

## Where tests live

| Path | Purpose |
| --- | --- |
| `unit/cli/` | CLI parsing and command behavior registered under `cli` |
| `unit/core/` | UI-independent comparison, editing, search, filesystem, and save rules |
| `unit/app/` | App workflows, session behavior, visuals, and operating-mode transitions |
| `unit/app/interactive_view/` | Editor state, rendering, projections, navigation, and editing behavior |
| `unit/app/folder_browser/` | Folder Merge browsing, overlays, search, replacement, and save behavior |
| `unit/app/search_session/`, `unit/app/search_result_rows/`, `unit/app/file_style/`, `unit/app/find_icons/` | Focused suites for app capabilities that have their own module owner |
| `unit/diagnostics/` | Local diagnostics formatting and recording behavior |
| `*.rs` at this directory's root | Cargo integration-test targets; these exercise crate-visible behavior and external workflows |

Each unit owner has a `mod.rs` test index or suite entry. Related suite files and
test-only support live beside that entry. For example, `unit/app/` may contain
an `operating_modes.rs` suite, while `unit/app/interactive_view/` may contain a
`projection_support.rs` helper. The `src/` registration is intentionally small:
it selects the external test file and declares the logical module name. Put
test-only imports and fixtures in the external test suite or its support module,
not back into production source.

## Finding and running tests

Use the test routing table in [`AGENTS_PROJECT_REFERENCE.md`](../AGENTS_PROJECT_REFERENCE.md)
to find the suite for a capability. For cross-capability flows and contribution
guidance, see [Development and tests](../docs/architecture/development.md).

Unit tests run with their established logical module paths, not with their
physical `tests/unit/` paths. For example:

```sh
cargo test --locked --offline --lib core::file_view
cargo test --locked --offline --bin versus app::interactive_view::tests
cargo test --locked --offline --bin versus app::folder_browser::tests
cargo test --locked --offline --bin versus app::operating_mode_tests
```

Integration tests remain Cargo targets selected by filename:

```sh
cargo test --locked --offline --test cli
cargo test --locked --offline --test file_view_options
```

Keep a test with the narrowest owner that can exercise its contract. Use an
integration target when the behavior needs to be verified through the crate's
external interface or spans a user-visible workflow. Put shared test support
beside the suites that use it and keep it narrowly scoped; promote a fixture to a
broader support module only when multiple capability suites share its meaning.

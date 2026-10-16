# Platform and packaging constraints

This guide records platform details that can be easy to break during dependency, native-input, and packaging changes. The authoritative implementation and validation commands remain in [`AGENTS_PROJECT_REFERENCE.md`](../../AGENTS_PROJECT_REFERENCE.md) and the release workflows. Read the referenced scripts and CI checks before changing packaging.

## Offline dependency tree

[`.cargo/config.toml`](../../.cargo/config.toml) redirects crates.io to the checked-in `vendor/` tree and enables Cargo offline mode. Builds must work from that source tree without network access. Dependency refresh happens on a connected development machine through [`scripts/vendor-dependencies.ps1`](../../scripts/vendor-dependencies.ps1); it is not part of normal builds.

`Cargo.toml` patches egui to [`vendor/egui`](../../vendor/egui). This fork contains local behavior changes and must remain a path patch so Cargo tracks its sources and rebuilds them. Do not replace it with a registry copy or treat the modified files as immutable cached dependency sources. When refreshing dependencies, preserve and revalidate both the egui changes and the vendored winit patch.

The winit patch supplies native drag coordinates, including corrected Windows OLE by-value coordinates and storage-medium cleanup. Its rationale is recorded in [`docs/vendor-winit-drop-position.md`](../vendor-winit-drop-position.md). Preserve the patch on vendor refresh. Windows also samples the live native cursor relative to the app window during drag hover/drop, converting physical pixels to UI points; pane hit testing gives a current-frame forwarded position priority over a native sample. Linux selects X11/XWayland when `DISPLAY` is available for native file drops and keeps native Wayland when it is not.

## Release targets and Linux runtimes

Release artifacts are portable Windows x86-64 and Linux x86-64 builds. Linux has separate glibc 2.35 (Ubuntu 22.04) and glibc 2.28 (Rocky 8.10) baselines. CI and release workflows build both through [`scripts/build-linux.sh`](../../scripts/build-linux.sh); artifact names include the baseline. [`scripts/package-linux.sh`](../../scripts/package-linux.sh) packages the selected baseline and [`scripts/check-linux-compatibility.py`](../../scripts/check-linux-compatibility.py) checks the resulting runtime and bundled ELF files.

The 2.28 AppImage can select a compatibility path for X11 servers without XInput2. It uses a private authenticated Xephyr display with software rendering; `--compat-x11` selects it explicitly. The plain executable and 2.35 AppImage use host graphics. Private graphics libraries must stay isolated from ordinary launches. Rocky CI also exercises automatic selection through nxagent, resizing, input, browsing, authentication, cleanup, and hidden host runtime files. Keep these checks with changes to this path.

The private display is a process and graphics compatibility mechanism, not a general desktop bridge: clipboard and file drops do not cross its boundary. Do not add host runtime files to the test environment when verifying isolation.

## Rocky crypto and FIPS checks

Original Rocky crypto libraries and their vendor HMAC sidecars, including GMP's `fipscheck/` file, must remain byte-identical. Packaging and verification must preserve the runtime license/source inventory as well as those bytes. CI runs final package verification and forced vendor FIPS self-tests, including negative controls and the nxagent GUI path. FIPS activation hooks exist only for tests; they must not alter the host runtime policy or be presented as product certification.

## Licenses, diagnostics, and generated release files

The release process exports dependency license information with [`scripts/export-dependency-licenses.ps1`](../../scripts/export-dependency-licenses.ps1) and can create an offline source archive with [`scripts/package-source-offline.sh`](../../scripts/package-source-offline.sh). Preserve both the license and source inventories when adding, removing, or repackaging dependencies. Embedded Codicons carry their CC BY 4.0 attribution and license under `assets/codicons/` and are included in Windows and Linux packages.

Desktop diagnostics are written beside the executable (beside the outer AppImage on Linux). A bounded queue keeps writes off the UI thread. Logs rotate at 5 MiB with one `versus.log.1` backup, exclude compared file contents, and flush at shutdown for up to 500 ms. Keep this behavior local to [`src/diagnostics.rs`](../../src/diagnostics.rs); packaging changes should not move logs into shared configuration or runtime state.

## Validation path

Use the repository project reference for the current commands. Platform claims require the corresponding build and launch checks; a successful macOS development build does not verify Windows or either Linux runtime baseline. For packaging work, inspect the CI job that owns the affected artifact and retain its compatibility, inventory, authentication, and cleanup checks.

# Versus

Versus is an offline engineering workspace for comparing two folders in linked,
side-by-side trees. It uses operating-system filesystem paths, so
mapped drives and UNC paths on Windows and mounted network filesystems on Linux
work through the same local APIs as other paths. Versus does not implement network
authentication or make outbound network requests at runtime.

## Use

Use the folder icon in each pane header, or enter/paste both folder paths and use
the refresh icon at the upper right to compare them. Selecting the second folder
with the picker starts comparison
automatically; pressing Enter in a path field also starts comparison when both
paths are supplied. The full selected path is shown above each tree, wrapping when
needed. Local paths, mapped drives, UNC shares, and mounted Linux paths all use the
same filesystem APIs.

Click a folder in either tree to expand or collapse the corresponding relative
path in both. Scrolling is linked, and a dash marks the empty position opposite
a one-sided item so matching paths stay aligned. **Expand all** and **Collapse
all** are the double-chevron icons at the upper right and control both trees. Hover
over an action icon for its label. Tab focuses tree rows; Enter or Space toggles a
folder, and the left/right arrow keys collapse/expand a focused folder.

- Green checkmark: identical entries.
- Amber unequal sign: changed files or shared folders containing differences.
- Cyan left arrow: files and folders present only on the left.
- Violet right arrow: files and folders present only on the right.
- Red boxed cross: different entry types.
- Red warning triangle: a filesystem read error. Hover for details.

The legend sits above the two panes; the same icons replace status words in each
compact tree row. File sizes and recursive folder totals appear beside the status
icon, independently for each side, using binary units (KiB, MiB, and so on). Sizes
reflect filesystem metadata collected during the scan. Folder
totals count regular-file bytes and exclude symlink targets. Empty folders show
zero bytes; unknown sizes or incomplete totals show a dash. Hover for status and
error details. The cursor becomes a pointer over files and folders.

Comparison runs in the background with refresh and cancel icons in the header.
Files are compared by content, not timestamps. Symlinks are compared by their
targets and are never recursively followed. Empty folders are included. Changing
a path clears the prior result so it cannot be mistaken for the new selection.

This rebuild focuses exclusively on read-only folder comparison. Text comparison,
merge, editing, saving, and other workflows are deferred; their existing Rust core
primitives remain available for later work. Compared folders are never modified.

## Releases

Tagged releases build these artifacts in GitHub Actions:

- `Versus.exe`: the portable Windows executable for Windows 10 and 11.
- `Versus.AppImage`: the primary portable Linux x86-64 distribution. Run `chmod +x
  Versus.AppImage && ./Versus.AppImage`.
- `Versus-windows-x86_64.zip` and `versus-linux-x86_64.tar.gz`: convenience archives.
- `versus-source-offline.tar.gz`: source, lockfile, vendor tree, build scripts, and
  generated dependency/license inventory for disconnected builds.

The release workflow builds these artifacts; this repository does not claim that a
particular artifact has been executed on every supported operating system. Linux
still requires a compatible kernel, display stack, and graphics driver supplied by
the host.

Versus is licensed under [MIT](LICENSE). Each release includes
`DEPENDENCY-LICENSES.md`, generated from Cargo metadata; review third-party
licenses before redistributing.

## Build from source

Install Rust 1.95 on a connected development machine, clone the repository, and run:

```powershell
cargo build --release --locked --offline
```

The checked-in `.cargo/config.toml` directs Cargo to `vendor/` and forces offline
resolution. The command must work without DNS, package repositories, or Internet
access when `Cargo.lock` and `vendor/` are present.

To refresh locked dependencies on a connected machine after intentionally changing
`Cargo.toml` or `Cargo.lock`, run:

```powershell
./scripts/vendor-dependencies.ps1
cargo build --release --locked --offline
./scripts/export-dependency-licenses.ps1 -OutputPath dist/DEPENDENCY-LICENSES.md
```

Review and commit the resulting `vendor/`, `.cargo/config.toml`, lockfile, and
inventory output used for a release. Do not run the vendor refresh in an air-gapped
environment. A separate Rust toolchain bundle is required where Rust is not already
installed; it is intentionally outside this repository.

On Linux, install the host development libraries required by the native windowing
backend, then use the same Cargo command. The release workflow lists the Ubuntu
packages it uses as a reproducible reference.

## Development checks

```powershell
cargo fmt --check
cargo test --locked --offline
cargo build --release --locked --offline
git diff --check
```

Run the narrow relevant test target first while developing. The full commands above
are the release readiness baseline.

# Versus

Versus is an offline engineering workspace for comparing two files or two folders
in side-by-side panes. It uses operating-system filesystem paths, so
mapped drives and UNC paths on Windows and mounted network filesystems on Linux
work through the same local APIs as other paths. Versus does not implement network
authentication or make outbound network requests at runtime.

## Use

Choose **Folder Compare** or **File Compare** in the center of the header. Each
button includes a folder or file icon. The active mode is highlighted, with Folder
Compare selected initially. Clicking a path
area or browse icon in either pane opens that mode's native folder or file picker
directly. Selecting the second source starts comparison automatically. File
Compare opens the line comparison without a Back button.

Changing modes starts a fresh comparison and cancels pending work. Clicking the
active mode preserves the current comparison; clicking Folder Compare while
viewing a file from its tree returns to that tree. The **+** button at the right
of the legend has the tooltip **New comparison** and clears both selections,
results, and pending work while keeping the selected mode and theme.

Both pane headers remain available for browsing in directly selected file views.
Folder paths wrap to show the full path; file paths use a leading ellipsis with
the full path on hover. LEFT/RIGHT and browse stay centered beside the path.
The refresh icon at the upper right compares the same folders again. Local paths,
mapped drives, UNC shares, and mounted Linux paths all use the same filesystem
APIs; paths can be entered through the native dialog.

Click a folder in either tree to expand or collapse the corresponding relative
path in both. Scrolling is linked, and a dash marks the empty position opposite
a one-sided item so matching paths stay aligned. **Expand all** and **Collapse
all** are the stacked plus/minus icons at the upper right and control both trees. Hover
over an action icon for its label. Tab focuses tree rows; Enter or Space toggles a
folder, and the left/right arrow keys collapse/expand a focused folder.

- Amber unequal sign: changed files or shared folders containing differences.
- Cyan left arrow: files and folders present only on the left.
- Violet right arrow: files and folders present only on the right.
- Red boxed cross: different entry types.
- Red warning triangle: a filesystem read error. Hover for details.

Identical files, folders, and lines use ordinary gray text without a status icon
or a legend entry. The legend sits above the two panes; the same icons replace
status words in each compact tree row. File sizes and recursive folder totals appear beside the status
icon, independently for each side, using binary units (KiB, MiB, and so on). Sizes
reflect filesystem metadata collected during the scan. Folder
totals count regular-file bytes and exclude symlink targets. Empty folders show
zero bytes; unknown sizes or incomplete totals show a dash. Hover for status and
error details. The cursor becomes a pointer over files and folders.

Comparison runs in the background with refresh and cancel icons in the header.
Files are compared by content, not timestamps. Symlinks are compared by their
targets and are never recursively followed. Empty folders are included. Changing
a path clears the prior result so it cannot be mistaken for the new selection.

When comparing folders, double-click a regular file on either side to open a
read-only, line-by-line file comparison. The same legend colors and icons identify changed, left-only, and
right-only lines; line numbers and empty placeholders keep both sides aligned.
File headers use the same LEFT/RIGHT styling as folder headers. Long file paths
show a leading ellipsis so the filename remains visible; hover for the full path.
Vertical scrolling stays linked, and each pane can scroll horizontally for long
lines. The **back arrow**, at the left of the legend, returns to the folder view
with its selection, expansion, and scroll position preserved. Enter or Space on a
focused file also opens it.

File contents load in the background. Missing files show an empty side; binary or
non-UTF-8 files, files above the 32 MiB text limit, and comparisons exceeding the
two-second diff processing limit show an explanation. Type mismatches do not open
folders or symlink targets. Read errors are shown in the file view; Back remains
available for files opened from a folder comparison. Line endings are normalized
for text comparison. Merge, editing, saving, and other workflows remain
deferred. Compared files and folders are never modified.

The sun/moon icon at the upper right switches between light and dark themes in
either view. The logo and window icon preserve the blue half and use a white half
in dark mode or the original dark half in light mode.

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

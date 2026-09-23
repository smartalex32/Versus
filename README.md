# Versus

Versus is a fast, offline desktop utility for comparing directories and two files.
It uses operating-system filesystem paths, so
mapped drives and UNC paths on Windows and mounted network filesystems on Linux
work through the same local APIs as other paths. Versus does not implement network
authentication or make outbound network requests at runtime.

## Use

Open Versus and choose **Directory** or **2-Way**. Enter, paste, drop,
or browse for local paths, mapped drives, UNC shares, or mounted Linux paths.
Directory comparison starts with the **Diffs** filter; use **All**, **Diffs**, or
**Same** to change the visible results, and double-click a changed file to inspect
it and return to the directory results afterward. File comparison has the same
view controls and shows aligned line numbers, highlighted changes, and difference
navigation. Choose
**Edit buffers** to edit either side, then **Recalculate edited diff** to update
the highlighting. Copying a selected difference changes only the working buffer
until you choose **Save** or **Save As**. Use the
**Dark mode** or **Light mode** button in the header to switch appearance; the
choice is saved with the other local settings.

Versus never writes compared files during opening or comparison. It requests
confirmation before replacing an existing destination. Settings are stored locally
in `Versus/settings.conf` under the platform's configuration directory and contain
no file contents.

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

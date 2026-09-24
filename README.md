# Versus

Versus is an offline desktop utility for comparing directories and two text files.
The interface uses React and TypeScript in a Tauri 2 WebView. Rust performs all file
comparison and saving. Local, mapped-drive, UNC, and mounted network paths are
handled by the operating system; Versus has no cloud service or telemetry.

## Use

Choose **Folders** or **Files**, then enter, paste, drop, or browse for two paths.
Folder results can be filtered and expanded. Select a changed file to inspect it
without losing the folder results. The file view aligns lines, highlights changes,
and has previous/next navigation. **Edit buffers** lets you change either side or
copy the selected difference. Changes stay in memory until **Save**; replacing an
existing file requires explicit confirmation.

Appearance and comparison options are stored locally. Compared file contents are
not stored in preferences.

## Development

Install Node.js, Rust, and the [Tauri 2 platform prerequisites](https://v2.tauri.app/start/prerequisites/).
Then run:

```powershell
npm ci --offline --cache npm-cache
npm run tauri -- dev
```

The root Rust crate is the UI-independent comparison core. `src-tauri/` contains
the desktop shell and narrow native commands; `src-ui/` contains the TypeScript UI.
The production frontend has no network assets. Rust dependencies are locked in
`Cargo.lock` and checked into `vendor/`; `.cargo/config.toml` forces offline Cargo
resolution. The npm lockfile fixes frontend dependency versions, and `npm-cache/`
contains the Windows and Linux x64 npm packages needed for an offline source build.

```powershell
cargo fmt --check
cargo test --locked --offline
cargo test -p versus-desktop --locked --offline
npm run build
npm run tauri -- build --no-bundle
git diff --check
```

Refresh Rust dependencies on a connected development machine with
`./scripts/vendor-dependencies.ps1`, then check the lockfile and vendor changes.

## Packaging

The release workflow builds a Windows NSIS installer with an offline WebView2
installer, a Windows executable for systems that already have WebView2, and a Linux
AppImage. The Windows executable alone depends on WebView2 being present. Linux
requires the host's compatible display and WebKit stack. No programming runtime or
Internet access is needed when running a packaged app.

Versus is licensed under [MIT](LICENSE). Review the dependency inventory before
redistributing a release.

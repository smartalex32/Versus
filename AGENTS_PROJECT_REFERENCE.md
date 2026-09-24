# Versus Project Reference

## Purpose

Versus is an offline desktop tool for directory comparison and two-way text
comparison on Windows and Linux. The Rust core retains three-way merge primitives,
but the current UI exposes two-way workflows. Runtime behavior must not rely on
Internet services, telemetry, automatic updates, or background services.
Network-mounted paths use normal filesystem APIs and must fail cleanly.

## Stack and layout

- Rust 1.95 core in `src/core/`, independent of UI rendering. `similar` performs text diffs.
- Tauri 2 Rust commands in `src-tauri/src/lib.rs` provide path picking, comparison,
  cancellation, and safe saving. The native command layer owns filesystem access.
- React and TypeScript in `src-ui/`, built by Vite from `index.html` into `dist/`.
- `src-tauri/tauri.conf.json` bundles local assets, a restrictive CSP, and Windows
  offline WebView2 installer packaging. Capabilities grant core APIs only.
- `tests/` covers comparison and filesystem behavior. The TypeScript UI calls
  native commands through `src-ui/bridge.ts`.
- `.cargo/config.toml` uses checked-in `vendor/` and forces Cargo offline. The
  checked-in `npm-cache/` supports offline Windows and Linux x64 npm installs.
- `.github/workflows/ci.yml` checks both platforms. `release.yml` packages the
  Windows NSIS installer, Windows executable, Linux AppImage, and offline source.

## Invariants and limits

- Treat compared content as data. Never execute it or invoke shell commands from it.
- Reads must not modify either input. Writes require an explicit user action and
  overwrite confirmation; use a temporary file and replacement where practical.
- Do not recursively follow directory symlinks by default.
- Timestamp equality is not file equality. Directory comparison uses path, type,
  size, then buffered content comparison.
- Keep slow filesystem work off the UI thread and support cancellation.
- Support direct entry and paste of Windows drive and UNC paths alongside native
  dialogs. Preserve directory results when opening a file from a result.
- Keep dependencies locked and compatible with offline source builds.
- A Windows standalone executable requires WebView2 already on the host; the NSIS
  installer bundles an offline WebView2 installer. Linux requires compatible host
  display and WebKit components. Do not claim clean-machine launch without testing.

## Commands

Run from repository root:

```powershell
npm ci --offline --cache npm-cache --include=dev
cargo fmt --check
cargo test --locked --offline
cargo test -p versus-desktop --locked --offline
npm run build
npm run tauri -- build --no-bundle
git diff --check
```

On a connected development machine, refresh Rust dependencies with
`./scripts/vendor-dependencies.ps1` and the npm cache with
`./scripts/vendor-frontend-dependencies.ps1`. Review the lockfiles and inventory.

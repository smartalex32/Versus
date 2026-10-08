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

## Git and IDE integration

Launch Versus with two paths to open their comparison immediately:

```sh
versus --diff -- "left file.txt" "right file.txt"
versus --folder -- "left folder" "right folder"
```

Without a mode flag, two paths select file or folder comparison automatically.
With no arguments, the normal empty workspace opens. Relative paths resolve from
the caller's working directory; quote paths containing spaces. `--` lets filenames
begin with a dash. `--diff` requires regular files and `--folder` requires folders;
unavailable or incompatible sources show an explanation in the window. Git's
`/dev/null` (and `NUL` on Windows) represents an empty file side for additions and
deletions. Other devices and symlink targets are not opened.

Versus runs in the foreground until its window closes, keeping Git/IDE temporary
inputs available. `--wait` is accepted for callers that supply it and has the same
behavior. Each invocation opens its own window. Closing normally returns zero,
including when files differ; this is a visual comparison, not a command-line
equality check. `--help` and `--version` print without opening a window. Invalid
arguments return 2; failure to start the window returns 1. Read failures appear
in the window. Save editor buffers first: Versus reads files from disk and never
writes back to them.

Use the installed executable on your PATH, or replace `versus` with its quoted
full path (`Versus.exe` on Windows or `Versus.AppImage` on Linux). In PowerShell,
invoke a quoted executable path with `&`:

```powershell
& 'C:\Tools\Versus.exe' --diff -- 'left file.txt' 'right file.txt'
```

### Git

Run these once in a shell (Git Bash on Windows). Omit `--global` to configure only
the current repository:

```sh
git config --global diff.tool versus
git config --global diff.guitool versus
git config --global difftool.versus.cmd 'versus --diff -- "$LOCAL" "$REMOTE"'
```

For an executable outside PATH, set the command with its full path instead:

```sh
git config --global difftool.versus.cmd '"C:/Program Files/Versus/Versus.exe" --diff -- "$LOCAL" "$REMOTE"'
```

Then compare working-tree changes, staged changes, or two revisions:

```sh
git difftool --no-prompt
git difftool --no-prompt --cached
git difftool --no-prompt HEAD~1 HEAD -- path/to/file.txt
```

Close each Versus window to advance to the next changed file. For a single folder
comparison, temporarily select folder mode and ask Git to copy working-tree
files instead of creating symlinks:

```sh
git -c difftool.versus.cmd='versus --folder -- "$LOCAL" "$REMOTE"' difftool --tool=versus --dir-diff --no-symlinks
```

The command receives Git's pre-image on the left and post-image on the right.
See the [Git difftool reference](https://git-scm.com/docs/git-difftool) for revision
and path options. Versus currently provides read-only two-way comparison.

### VS Code and other IDEs

Add the following tasks to your project's `.vscode/tasks.json` (merge them into
any existing tasks). Replace the first task's `command` with your executable path
if Versus is outside PATH. Open a saved file, then use **Tasks: Run Task** to
compare it with another file or view its Git changes in a Versus window. The Git
task uses the configuration above. Tasks require an open workspace folder.

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "Versus: Compare active file",
      "type": "process",
      "command": "versus",
      "args": ["--diff", "--", "${file}", "${input:versusOtherFile}"],
      "options": { "cwd": "${workspaceFolder}" },
      "problemMatcher": []
    },
    {
      "label": "Versus: Git changes for active file",
      "type": "process",
      "command": "git",
      "args": ["difftool", "--tool=versus", "--no-prompt", "--", "${file}"],
      "options": { "cwd": "${fileDirname}" },
      "problemMatcher": []
    }
  ],
  "inputs": [
    {
      "id": "versusOtherFile",
      "type": "promptString",
      "description": "Other file path (absolute or relative to the workspace)"
    }
  ]
}
```

The process tasks pass paths as separate arguments, including spaces. See VS
Code's [external-tool tasks](https://code.visualstudio.com/docs/debugtest/tasks)
and [input variables](https://code.visualstudio.com/docs/reference/variables-reference).
For an IDE with an external diff-tool setting, choose the Versus executable and
set its arguments to `--diff -- <left-file> <right-file>`, substituting that IDE's
two filename placeholders. Enable waiting for the tool to exit if the IDE offers
that option.

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

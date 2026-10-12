# Versus

Versus is an offline engineering workspace for comparing files or folders and
merging text files. It uses operating-system filesystem paths, so
mapped drives and UNC paths on Windows and mounted network filesystems on Linux
work through the same local APIs as other paths. Versus does not implement network
authentication or make outbound network requests at runtime.

## Use

**File Merge** is the third option in the centered header, alongside Folder Compare
and File Compare. It opens **Left** (Incoming), **Base** (the editable merged
result), and **Right** (Current). The outer headers read **LEFT (Incoming)** and
**RIGHT (Current)**. Browse, paste, or drop a file into any of the three panes.
The Base path can specify their common ancestor; leaving it blank uses Current.
Selecting sources starts
the merge; committing a typed path with Enter or leaving its field reloads it. Opening
merge from a file comparison preselects its two files. The result starts with
Current's contents. The source panes remain read-only; typing and merge actions
change only the in-memory result.

The legend row centers four icons with tooltips: **Accept All Left**, **Accept All
Right**, **Auto Merge Non-Conflicts**, and **Undo**. Accept All replaces the result
with that source in one undoable operation. The merge legend adds a conflict icon.
Zoom sits immediately left of the up/down difference-navigation icons at the right
of that row; merge actions stay centered. Use the arrows to jump backward or forward
through the differences, with shared scrolling across the panes.
**Auto Merge Non-Conflicts** combines the safe changes as one
undoable operation. Each consecutive nonconflicting block has one inward arrow
per side. Conflicting lines have **!** indicators and individual arrows. You can also select, paste, and
type freely in the middle pane. Source panes retain their diff alignment and
original line numbers; the result has its own live line numbers. Vertical
scrolling is shared and horizontal scrolling is independent.

**Undo** reverses typing, arrows, and automatic merges; Ctrl/Command+Z works while
editing the result. Manual edits that overlap a pending action disable its arrow
to protect your text; finish that part by typing, or undo the edit. **Save** and
**Finish Merge** sit at the bottom right. **Save** writes the middle pane to the
selected common ancestor file, or to **Current** when the Base path is blank;
it does not open a destination picker. Existing files require explicit overwrite
confirmation. **Unsaved Changes** appears at the bottom left in bold highlight
color until the middle buffer matches the saved destination. **Finish Merge**
returns to comparison only when there are no unsaved changes and never writes
files itself. Right-click Save for **Save As…** to export a copy; exporting does
not change the regular Save destination or mark its outstanding changes saved.
Cancelling or failing a save keeps the merge open. Saves preserve the buffer's line
endings and final newline. Leaving or replacing an unsaved result asks before
discarding it. Merge contents are never stored in comparison history. This merge
workflow is currently GUI-only.

Empty merge panes use the same dashed inner drop borders as comparison modes.
Dragging a file over a pane shows the shared blue foreground border and tint.
Drop one regular file at a time; invalid drops preserve the existing merge, and
replacing an unsaved merge still requires a discard choice.

Choose **Folder Compare** or **File Compare** in the center of the header. Each
button includes a folder or file icon. The active mode is highlighted, with Folder
Compare selected initially. Clicking a path
area or browse icon in either pane opens that mode's native folder or file picker
directly. Selecting the second source starts comparison automatically. File
Compare opens the line comparison without a Back button.

The **Recently compared** list appears when Versus opens and after **New
comparison**. Click a file or folder pair to recompare it without browsing again.
The list shows only the active comparison type, with left-aligned path pairs
separated by a bold **vs** and subtle alternating row backgrounds.
The 20 most recent pairs are saved locally per user, newest first; reopening a
pair moves it to the top. Paths and comparison type are saved, never file contents.
Unavailable or changed sources show an error and can be selected again.

Changing modes starts a fresh comparison and cancels pending work. Clicking the
active mode preserves the current comparison; clicking Folder Compare while
viewing a file from its tree returns to that tree. The **+** button at the right
of the legend has the tooltip **New comparison** and clears both selections,
results, and pending work while keeping the selected mode and theme.

Both pane headers remain available for browsing in directly selected file views.
Folder paths wrap to show the full path; file paths use a leading ellipsis with
the full path on hover. LEFT/RIGHT and browse stay centered beside the path.
The refresh icon at the upper right reloads the file comparison you are viewing,
including files opened from a folder tree. In the folder view it compares the same
folders again while preserving selection, expansion, and scroll position; a
selection is cleared only if the entry no longer exists. Local paths,
mapped drives, UNC shares, and mounted Linux paths all use the same filesystem
APIs; paths can be entered through the native dialog.

Click a folder in either tree to expand or collapse the corresponding relative
path in both. Scrolling is linked, and a dash marks the empty position opposite
a one-sided item so matching paths stay aligned. **Expand all** and **Collapse
all** are the stacked plus/minus icons at the upper right and control both trees. Hover
over an action icon for its label. Tab focuses tree rows; Enter or Space toggles a
folder, and the left/right arrow keys collapse/expand a focused folder. Up/Down
moves through visible rows while keeping the active pane and selected entry in
view. Keyboard focus has a distinct blue outline. Native screen readers receive
side, path, kind, status, and size labels for tree entries and numbered text for
visible file lines; focused file panes also support arrow keys, Page Up/Down,
and Home/End.

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
target paths rather than target contents and are never recursively followed.
This keeps traversal within the selected roots and avoids loops or repeated
comparisons when links point to ancestors or outside folders. Empty folders are
included. Changing a path clears the prior result so it cannot be mistaken for
the new selection.
The status area immediately to the right of the legend shows change position,
loading progress, or errors without adding another row above the panes.
Long status messages are shortened on screen; hover for the complete text.
While loading, it shows the current stage and elapsed time.
Measured stages also show progress and an approximate time remaining for that
stage. Folder discovery and line alignment have no reliable total, so they show
activity without a time estimate. File loading can also be cancelled.

The **+** button stays beside the legend. The other icon controls at the far right
apply to both comparison views:

- **Show only differences** hides identical lines and tree entries. File line
  numbers still refer to the original files; changed folder ancestors stay visible.
  Filtering preserves the top visible entry, or the nearest remaining entry.
  It starts on.
- **Ignore whitespace** ignores inline Unicode whitespace, including spaces and
  tabs, while preserving line boundaries. It starts on.
- **Ignore line endings** ignores CRLF, LF, CR, and final-terminator differences.
  It starts on. Turn it off to compare endings exactly; changed endings appear
  as badges beside their lines.
- **Previous difference** and **Next difference**, the up/down arrows, scroll
  both panes to the previous or next changed line or file. Folder navigation skips
  folders and expands the target file's parents, including when they were
  collapsed. Navigation stops at the first or last difference; the status area
  shows **Change N of M** for the current target.

Selected option icons have a blue outline. Changing an ignore option recomputes
the comparison in the background; filtering does not reread the sources. Options
remain selected when starting a new comparison or changing modes. Folder ignore
options apply to valid UTF-8 text up to 32 MiB; binary, invalid UTF-8, and larger
files retain byte-for-byte comparison.

Drag one file or folder from your file explorer into the large view on the left
or right, below the path bars, to replace that side. Empty panes have persistent
dashed outlines and drop hints. A blue border and drop label
identify LEFT or RIGHT above the pane contents, including the empty chooser view.
Windows follows the live cursor during native drags even when ordinary pointer
events are unavailable. The source type
selects the corresponding view automatically; mixed file/folder pairs show why
they cannot compare. A drop in a file opened from the folder tree keeps the other
displayed file and starts a direct comparison. Multiple-item drops are rejected
without replacing the current selection.
On Linux, Versus uses X11 or XWayland when `DISPLAY` is available so native
file drops work. Pure Wayland sessions retain browse selection; the windowing
backend does not support file drops there.

When comparing folders, double-click a regular file on either side to open a
read-only, line-by-line file comparison. The same legend colors and icons identify changed, left-only, and
right-only lines; line numbers and empty placeholders keep both sides aligned.
File headers use the same LEFT/RIGHT styling as folder headers. Long file paths
show a leading ellipsis so the filename remains visible; hover for the full path.
Changed text within a line has a stronger highlight in that line's status color;
unchanged text remains unhighlighted. Very long lines or exhausted highlight
processing time use a whole-line highlight to keep loading responsive.
Within a changed block, similar lines are aligned even when an added or removed
line shifts their original numbers. For example, an inserted left line appears
opposite an empty placeholder, and following code aligns with its commented-out
version on the right while the comment text remains highlighted as a change.
Similarity alignment uses bounded extra work; very large changed blocks or an
exhausted processing budget retain positional pairing.
Vertical scrolling stays linked, and each pane can scroll horizontally for long
lines. The **back arrow**, at the left of the legend, returns to the folder view
with its selection, expansion, and scroll position preserved. Enter or Space on a
focused file also opens it.

File contents load in the background. Missing files show an empty side; binary or
non-UTF-8 files, files above the 32 MiB text limit, and comparisons exceeding the
two-second diff processing limit show an explanation. Type mismatches do not open
folders or symlink targets. Read errors are shown in the file view; Back remains
available for files opened from a folder comparison. Line endings are normalized
when **Ignore line endings** is selected. Merge, editing, saving, and other workflows remain
deferred. Compared files and folders are never modified.

The sun/moon icon at the upper right switches between light and dark themes in
either view. The logo and window icon preserve the blue half and use a white half
in dark mode or the original dark half in light mode.

The legend row places **Zoom** and its percentage immediately before adjacent
magnifying-glass **−** and **+** buttons, just left of **Show only differences**.
Click them to reduce or enlarge the interface. Hold
**Ctrl** and scroll the mouse wheel up/down to zoom in/out; the wheel alone
scrolls the comparison. Command + wheel also works on macOS. Use
Ctrl + / Ctrl - / Ctrl 0 (Command on macOS) for the keyboard shortcuts,
or click the percentage to reset to 100%. Zoom is restored across launches;
source paths and compared content are not saved. Row and control heights follow font metrics while staying compact at
the default size. Toolbar buttons highlight when pressed and activate on release;
releasing outside cancels the action.

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

GitHub Actions builds two separate native Linux x86-64 distributions in both CI
and tagged releases:

| GitHub artifact | Build environment / glibc baseline | AppImage |
| --- | --- | --- |
| `linux-glibc-2.35-x86_64` | Ubuntu 22.04 / glibc 2.35 | `Versus-linux-x86_64-glibc-2.35.AppImage` |
| `linux-glibc-2.28-x86_64` | Rocky Linux 8.10 / glibc 2.28, automatic legacy X11 fallback | `Versus-linux-x86_64-glibc-2.28.AppImage` |

Each artifact also includes a correspondingly named `.tar.gz` containing the plain
executable, README and license. The AppImage bundles runtime libraries; the plain
executable requires its native libraries on the host. Linux filenames include the
baseline so both variants can coexist in a release without overwriting each other.

Check your system with `getconf GNU_LIBC_VERSION`. Use the 2.28 artifact on Rocky 8.
Download the artifact from a successful CI run, unzip it, and run:

```bash
chmod +x Versus-linux-x86_64-glibc-2.28.AppImage
./Versus-linux-x86_64-glibc-2.28.AppImage
```

For the normal 2.35 build, substitute `2.35` in the filename. If the host cannot
mount AppImages through FUSE, append `--appimage-extract-and-run`; this optional
fallback needs no FUSE installation or administrator access. Both builds run the
same app.
The 2.35 AppImage and plain executables use the host display and graphics stack.

The 2.28 package preserves Rocky’s original crypto libraries and their vendor
integrity files so the display helpers can perform FIPS self-tests. It leaves
host crypto policies unchanged. CI exercises vendor FIPS activation and integrity
checks; this is not a claim of FIPS certification.

The **2.28 AppImage automatically detects X11 displays without XInput2**, including
traditional X2Go/nxagent sessions. On those displays it starts a bundled private
display and software renderer within the existing desktop. Launch it with the
same command above; no additional installation, remote service, or launch option
is needed. Normal X11 and Wayland-only sessions retain the native launch path. The
AppImage also accepts `--compat-x11` to explicitly select the fallback, including
when used as a Git/IDE diff tool. `--help` and `--version` do not open a display.

Compatibility mode retains comparisons, navigation, highlighting, filtering,
browsing and zoom. It increases package size and may use more CPU for rendering.
Clipboard contents and file drops from the surrounding desktop are not bridged
into the private display; use Browse or command-line paths. Automatic selection
checks XInput2 support, not every possible graphics-driver incompatibility.

On a shared X11 host, Xephyr may print `server already running` while its
race-safe display allocator skips occupied display numbers. If the app opens,
these messages do not indicate a second Versus instance. A warning about the
owner of `/tmp/.X11-unix` concerns the host's shared socket directory; Versus
leaves its ownership and permissions unchanged.

The 2.28 artifact also includes `linux-runtime-sources-glibc-2.28.tar.gz` with
matching runtime sources and the helper rebuild recipe; it is not needed to run.

Both build jobs run formatting, unit/integration tests and a release build in their
baseline container. Packaging checks the glibc requirements of the final AppImage
runtime and all bundled ELF files, then tests the packaged CLI and GUI startup
with Xvfb/software rendering in that same environment. The Rocky job additionally
tests automatic fallback through real nxagent, with host graphics/picker resources
hidden to check that the bundle supplies them. It exercises resizing, Ctrl+wheel,
file/folder pickers, private-display authentication and normal window closing.
OpenSSL, libgcrypt and GnuTLS self-tests also run with vendor FIPS activation
enabled, including integrity-failure controls and checks for crypto errors in
the running GUI’s log.
This verifies those configurations, not every user's display, kernel or driver.

Tagged releases additionally publish:

- `Versus.exe` and `Versus-windows-x86_64.zip` for Windows 10 and 11.
- `versus-source-offline.tar.gz`: source, lockfile, vendor tree, build scripts, and
  generated dependency/license inventory for disconnected builds.

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
The vendor tree includes a [native drop-position patch](docs/vendor-winit-drop-position.md).
Reapply it and update its checksums after refreshing dependencies.

On Linux, install the host development libraries required by the native windowing
backend, then use the same Cargo command. `scripts/build-linux.sh` lists the
Ubuntu 22.04 and Rocky 8.10 build packages used by CI and releases. On a machine
with Docker, reproduce either Linux build with:

```bash
docker run --rm --platform linux/amd64 -v "$PWD:/workspace" -w /workspace ubuntu:22.04 bash scripts/build-linux.sh 2.35
docker run --rm --platform linux/amd64 -v "$PWD:/workspace" -w /workspace rockylinux/rockylinux:8.10 bash scripts/build-linux.sh 2.28
```

Use separate clean checkouts for concurrent baseline builds. Container setup and
packaging need network access; Cargo compilation uses the vendored dependencies.

## Development checks

```powershell
cargo fmt --check
cargo test --locked --offline
cargo build --release --locked --offline
git diff --check
```

Run the narrow relevant test target first while developing. The full commands above
are the release readiness baseline.
